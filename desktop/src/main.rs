#![cfg_attr(windows, windows_subsystem = "windows")]

mod paths;
mod player;
mod room;
mod sender;
mod server;
mod tunnel;
#[cfg(target_os = "linux")]
mod audio_linux;
#[cfg(target_os = "linux")]
mod capture_linux;

use gstreamer as gst;
use serde::{Deserialize, Serialize};
use server::SrtTarget;
use slint::{ComponentHandle, Image, Model, ModelRc, SharedString, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

slint::include_modules!();

#[derive(Serialize, Deserialize, Default, Clone)]
struct Settings {
    name: String,
    with_audio: Option<bool>,
    restore_token: Option<String>,
    /// Kept so the invite stays the same from one day to the next.
    room_secret: Option<String>,
    firewall_open: bool,
}

impl Settings {
    fn path() -> std::path::PathBuf {
        paths::data_dir().join("desktop.json")
    }
    fn load() -> Self {
        std::fs::read(Self::path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }
    fn save(&self) {
        if let Ok(b) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(Self::path(), b);
        }
    }
}

thread_local! {
    /// The room lives on the UI thread; worker threads hand results back to it.
    static ROOM: RefCell<Room> = const { RefCell::new(Room::None) };
}

enum Room {
    None,
    Host { _server: server::Server, _tunnel: tunnel::Host, target: SrtTarget },
    Guest { tunnel: tunnel::Guest, target: SrtTarget },
}

impl Room {
    fn target(&self) -> Option<SrtTarget> {
        match self {
            Room::None => None,
            Room::Host { target, .. } | Room::Guest { target, .. } => Some(target.clone()),
        }
    }
}

/// Over the internet SRT needs room to retransmit lost packets.
const LOCAL_LATENCY_MS: u32 = 40;
const REMOTE_LATENCY_MS: u32 = 120;

/// Stream names become server paths, so only [a-z0-9_-].
fn safe_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        let c = match c {
            'á' | 'à' | 'â' | 'ã' => 'a',
            'é' | 'ê' => 'e',
            'í' => 'i',
            'ó' | 'ô' | 'õ' => 'o',
            'ú' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        };
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').chars().take(32).collect()
}

fn room_secret() -> [u8; 16] {
    let mut settings = Settings::load();
    if let Some(secret) = settings.room_secret.as_deref().and_then(room::unhex) {
        return secret;
    }
    let mut secret = [0u8; 16];
    getrandom::fill(&mut secret).expect("system randomness");
    settings.room_secret = Some(room::hex(&secret));
    settings.save();
    secret
}

fn open_room() -> anyhow::Result<(Room, String)> {
    let secret = room_secret();
    let passphrase = room::hex(&secret);
    if !Settings::load().firewall_open {
        room::open_firewall()?;
        let mut s = Settings::load();
        s.firewall_open = true;
        s.save();
    }
    let server = server::Server::start(&passphrase)?;
    let tunnel = tunnel::Host::start(room::TUNNEL_PORT, secret, server::SRT_PORT, || {
        server::live_streams().unwrap_or_default()
    })?;
    let addrs = room::local_addresses()
        .into_iter()
        .map(|ip| std::net::SocketAddr::new(ip, room::TUNNEL_PORT))
        .collect::<Vec<_>>();
    if addrs.is_empty() {
        anyhow::bail!("este PC não tem nenhum endereço de rede utilizável");
    }
    let invite = room::Invite { secret, addrs }.encode();
    let target = SrtTarget { port: server::SRT_PORT, latency_ms: LOCAL_LATENCY_MS, passphrase };
    Ok((Room::Host { _server: server, _tunnel: tunnel, target }, invite))
}

fn join_room(code: &str) -> anyhow::Result<Room> {
    let invite = room::Invite::decode(code)?;
    let guest = tunnel::Guest::connect(&invite.addrs, invite.secret).map_err(|_| {
        anyhow::anyhow!(
            "não consegui chegar ao PC de quem criou a sala. A sala está aberta? Se estiver, as duas redes não permitem conexão direta: tente que outra pessoa crie a sala."
        )
    })?;
    let target = SrtTarget { port: guest.local_port, latency_ms: REMOTE_LATENCY_MS, passphrase: room::hex(&invite.secret) };
    Ok(Room::Guest { tunnel: guest, target })
}

/// The Windows installer ships GStreamer next to the app; use only that copy,
/// never one that may be installed system-wide.
#[cfg(windows)]
fn use_bundled_gstreamer() {
    let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())) else { return };
    let plugins = dir.join("gstreamer-1.0");
    if !plugins.is_dir() {
        return;
    }
    // SAFETY: called before any other thread exists.
    unsafe {
        std::env::set_var("GST_PLUGIN_SYSTEM_PATH_1_0", &plugins);
        std::env::set_var("GST_PLUGIN_PATH_1_0", "");
        std::env::set_var("GST_PLUGIN_SCANNER_1_0", dir.join("gst-plugin-scanner.exe"));
        std::env::set_var("GST_REGISTRY_1_0", paths::data_dir().join("gstreamer-registry.bin"));
    }
}

fn main() -> anyhow::Result<()> {
    #[cfg(windows)]
    use_bundled_gstreamer();
    gst::init()?;
    let ui = AppWindow::new()?;
    let settings = Rc::new(RefCell::new(Settings::load()));
    {
        let s = settings.borrow();
        let fallback = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_default();
        ui.set_name(if s.name.is_empty() { safe_name(&fallback) } else { s.name.clone() }.into());
        ui.set_with_audio(s.with_audio.unwrap_or(true));
    }
    ui.set_room_status("Crie uma sala ou entre na de um amigo com o convite.".into());
    ui.set_stage_message("Entre numa sala para ver quem está ao vivo.".into());

    #[cfg(target_os = "linux")]
    let runtime = Arc::new(tokio::runtime::Runtime::new()?);
    let sender: Arc<Mutex<Option<sender::Sender>>> = Arc::default();
    let player: Rc<RefCell<Option<player::Player>>> = Rc::default();
    let frame_count = Arc::new(AtomicU32::new(0));
    let frame_size = Arc::new(Mutex::new((0u32, 0u32)));
    let live_model = Rc::new(VecModel::<SharedString>::default());
    ui.set_live(ModelRc::from(live_model.clone()));

    {
        let ui_weak = ui.as_weak();
        ui.on_create_room(move || {
            let ui = ui_weak.unwrap();
            ui.set_room_busy(true);
            ui.set_error_text("".into());
            ui.set_room_status("Abrindo a sala. Se o sistema pedir a senha, é para liberar a porta da sala no firewall.".into());
            let ui_weak = ui_weak.clone();
            std::thread::spawn(move || {
                let result = open_room();
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_room_busy(false);
                    match result {
                        Ok((opened, invite)) => {
                            ROOM.with(|r| *r.borrow_mut() = opened);
                            ui.set_invite_code(invite.into());
                            ui.set_room_state(1);
                            ui.set_room_status("Sua sala está aberta. Mande o convite para os amigos; ele vale nos próximos dias também.".into());
                            ui.set_stage_message("Escolha uma transmissão na lista.".into());
                        }
                        Err(e) => {
                            ui.set_room_status("Crie uma sala ou entre na de um amigo com o convite.".into());
                            ui.set_error_text(format!("Não deu para abrir a sala: {e:#}").into());
                        }
                    }
                });
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_join_room(move |code| {
            let ui = ui_weak.unwrap();
            ui.set_room_busy(true);
            ui.set_error_text("".into());
            ui.set_room_status("Procurando a sala…".into());
            let ui_weak = ui_weak.clone();
            let code = code.to_string();
            std::thread::spawn(move || {
                let result = join_room(&code);
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_room_busy(false);
                    match result {
                        Ok(joined) => {
                            if let Room::Guest { tunnel, .. } = &joined {
                                ui.set_room_status(format!("Na sala de {}.", tunnel.host.ip()).into());
                            }
                            ROOM.with(|r| *r.borrow_mut() = joined);
                            ui.set_room_state(2);
                            ui.set_invite_input("".into());
                            ui.set_stage_message("Escolha uma transmissão na lista.".into());
                        }
                        Err(e) => {
                            ui.set_room_status("Crie uma sala ou entre na de um amigo com o convite.".into());
                            ui.set_error_text(format!("{e:#}").into());
                        }
                    }
                });
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_leave_room(move || {
            let ui = ui_weak.unwrap();
            ui.invoke_stop_watch();
            if ui.get_transmitting() {
                ui.invoke_stop_transmit();
            }
            ROOM.with(|r| *r.borrow_mut() = Room::None);
            ui.set_room_state(0);
            ui.set_invite_code("".into());
            ui.set_room_status("Crie uma sala ou entre na de um amigo com o convite.".into());
            ui.set_stage_message("Entre numa sala para ver quem está ao vivo.".into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let sender = sender.clone();
        let settings = settings.clone();
        #[cfg(target_os = "linux")]
        let runtime = runtime.clone();
        ui.on_transmit(move |name, with_audio| {
            let ui = ui_weak.unwrap();
            let Some(target) = ROOM.with(|r| r.borrow().target()) else { return };
            let name = safe_name(&name);
            if name.is_empty() {
                ui.set_error_text("Escreva um nome com letras ou números.".into());
                return;
            }
            ui.set_name(name.clone().into());
            ui.set_error_text("".into());
            ui.set_busy(true);
            ui.set_transmit_status("Escolha a tela na janela do sistema.".into());
            {
                let mut s = settings.borrow_mut();
                s.name = name.clone();
                s.with_audio = Some(with_audio);
                s.save();
            }
            let token = Settings::load().restore_token;
            let ui_weak = ui_weak.clone();
            let sender = sender.clone();
            #[cfg(target_os = "linux")]
            let runtime = runtime.clone();
            std::thread::spawn(move || {
                #[cfg(target_os = "linux")]
                let result = runtime.block_on(capture_linux::request(token)).and_then(|(capture, token)| {
                    let ui_weak = ui_weak.clone();
                    let on_error = move |message: String| {
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_error_text(format!("A transmissão parou: {message}").into());
                            ui.invoke_stop_transmit();
                        });
                    };
                    let (s, summary) = sender::start(capture, &target.publish(&name), with_audio, on_error)?;
                    *sender.lock().unwrap() = Some(s);
                    Ok((summary, token))
                });
                #[cfg(windows)]
                let result = {
                    let _ = token;
                    let ui_weak = ui_weak.clone();
                    let on_error = move |message: String| {
                        let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                            ui.set_error_text(format!("A transmissão parou: {message}").into());
                            ui.invoke_stop_transmit();
                        });
                    };
                    sender::start(&target.publish(&name), with_audio, on_error).map(|(s, summary)| {
                        *sender.lock().unwrap() = Some(s);
                        (summary, None::<String>)
                    })
                };
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_busy(false);
                    match result {
                        Ok((summary, token)) => {
                            ui.set_transmitting(true);
                            ui.set_transmit_status(format!("Ao vivo como \"{}\": {summary}.", ui.get_name()).into());
                            if token.is_some() {
                                let mut s = Settings::load();
                                s.restore_token = token;
                                s.save();
                            }
                        }
                        Err(e) => {
                            ui.set_transmit_status("".into());
                            ui.set_error_text(format!("Não deu para transmitir: {e:#}").into());
                        }
                    }
                });
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        let sender = sender.clone();
        ui.on_stop_transmit(move || {
            sender.lock().unwrap().take();
            let ui = ui_weak.unwrap();
            ui.set_transmitting(false);
            ui.set_transmit_status("".into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let player = player.clone();
        let (frame_count, frame_size) = (frame_count.clone(), frame_size.clone());
        ui.on_watch(move |name| {
            let ui = ui_weak.unwrap();
            let Some(target) = ROOM.with(|r| r.borrow().target()) else { return };
            player.borrow_mut().take();
            ui.set_has_frame(false);
            ui.set_stage_message(format!("Conectando a {name}…").into());
            let frames = ui.as_weak();
            let errors = ui.as_weak();
            let (count, size) = (frame_count.clone(), frame_size.clone());
            let result = player::Player::start(
                &target.read(&name),
                move |pixels, pending| {
                    let (count, size) = (count.clone(), size.clone());
                    let _ = frames.upgrade_in_event_loop(move |ui| {
                        // Counted on the UI thread: this is what the window actually shows.
                        count.fetch_add(1, Ordering::Relaxed);
                        *size.lock().unwrap() = (pixels.width(), pixels.height());
                        ui.set_frame(Image::from_rgba8(pixels));
                        ui.set_has_frame(true);
                        pending.store(false, Ordering::Release);
                    });
                },
                move |message| {
                    let _ = errors.upgrade_in_event_loop(move |ui| {
                        ui.set_has_frame(false);
                        ui.set_stage_message(format!("A transmissão caiu: {message}").into());
                    });
                },
            );
            match result {
                Ok(p) => {
                    p.set_muted(ui.get_muted());
                    *player.borrow_mut() = Some(p);
                    ui.set_watching(name);
                }
                Err(e) => ui.set_stage_message(format!("Não deu para assistir: {e:#}").into()),
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        let player = player.clone();
        ui.on_stop_watch(move || {
            player.borrow_mut().take();
            let ui = ui_weak.unwrap();
            if ui.get_fullscreen() {
                ui.invoke_toggle_fullscreen();
            }
            ui.set_video_info("".into());
            ui.set_watching("".into());
            ui.set_has_frame(false);
            ui.set_stage_message("Escolha uma transmissão na lista.".into());
        });
    }
    {
        let player = player.clone();
        ui.on_mute_changed(move |muted| {
            if let Some(p) = player.borrow().as_ref() {
                p.set_muted(muted);
            }
        });
    }
    {
        let ui_weak = ui.as_weak();
        ui.on_toggle_fullscreen(move || {
            let ui = ui_weak.unwrap();
            let fullscreen = !ui.get_fullscreen();
            ui.window().set_fullscreen(fullscreen);
            ui.set_fullscreen(fullscreen);
        });
    }
    let fps_timer = slint::Timer::default();
    {
        let ui_weak = ui.as_weak();
        let (count, size) = (frame_count.clone(), frame_size.clone());
        fps_timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let frames = count.swap(0, Ordering::Relaxed);
            if ui.get_watching().is_empty() || frames == 0 {
                ui.set_video_info("".into());
                return;
            }
            let (w, h) = *size.lock().unwrap();
            ui.set_video_info(format!("{w}×{h}, {frames} fps").into());
        });
    }

    let timer = slint::Timer::default();
    {
        let ui_weak = ui.as_weak();
        let live_model = live_model.clone();
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let names = ROOM.with(|r| match &*r.borrow() {
                Room::None => Vec::new(),
                Room::Host { .. } => server::live_streams().unwrap_or_default(),
                Room::Guest { tunnel, .. } => {
                    if !tunnel.connected() {
                        ui.set_room_status("Sem resposta da sala. O PC de quem criou a sala continua ligado?".into());
                    } else {
                        ui.set_room_status(format!("Na sala de {}.", tunnel.host.ip()).into());
                    }
                    tunnel.live()
                }
            });
            let current: Vec<String> = live_model.iter().map(|s| s.to_string()).collect();
            if current != names {
                live_model.set_vec(names.iter().map(SharedString::from).collect::<Vec<_>>());
            }
            let watching = ui.get_watching().to_string();
            if !watching.is_empty() && !names.contains(&watching) {
                ui.invoke_stop_watch();
                ui.set_stage_message("A transmissão terminou.".into());
            }
        });
    }

    ui.run()?;
    sender.lock().unwrap().take();
    player.borrow_mut().take();
    ROOM.with(|r| *r.borrow_mut() = Room::None);
    Ok(())
}


