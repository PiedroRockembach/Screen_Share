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
use std::cell::{Cell, RefCell};
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
    /// Label of the screen the restore token points to ("Tela de 2560×1440").
    screen: Option<String>,
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

/// Before going live on Linux: which screen the saved choice will use.
fn next_screen_text(screen: Option<&str>) -> String {
    match screen {
        Some(screen) => format!("{screen}, a mesma da última vez"),
        None => "A mesma tela da última vez".to_owned(),
    }
}

/// First letter in upper case: error chains start in lower case.
fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The light a frame throws on the wall around the TV: its average color,
/// kept dim and not too saturated so it never competes with the picture.
fn ambient_light(pixels: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> slint::Color {
    let data = pixels.as_slice();
    // An odd stride so the samples don't line up into a few columns.
    let step = (data.len() / 4096).max(1) | 1;
    let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
    for p in data.iter().step_by(step) {
        r += u64::from(p.r);
        g += u64::from(p.g);
        b += u64::from(p.b);
        n += 1;
    }
    if n == 0 {
        return slint::Color::from_rgb_u8(0x15, 0x11, 0x0e);
    }
    let (r, g, b) = (r as f32 / n as f32 / 255.0, g as f32 / n as f32 / 255.0, b as f32 / n as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, s) = if d < 1e-4 {
        (0.0, 0.0)
    } else {
        let s = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == r {
            ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        (h * 60.0, s)
    };
    let (s, l) = (s.min(0.6), 0.08 + 0.16 * l);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    slint::Color::from_rgb_u8(to(r), to(g), to(b))
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

    #[cfg(target_os = "linux")]
    let runtime = Arc::new(tokio::runtime::Runtime::new()?);
    let sender: Arc<Mutex<Option<sender::Sender>>> = Arc::default();
    let player: Rc<RefCell<Option<player::Player>>> = Rc::default();
    let frame_count = Arc::new(AtomicU32::new(0));
    let frame_size = Arc::new(Mutex::new((0u32, 0u32)));
    // Failed reconnections in a row while watching; a frame resets it.
    let reconnects = Arc::new(AtomicU32::new(0));
    // Linux: the next transmission opens the system's screen picker again
    // instead of reusing the saved choice.
    #[cfg(target_os = "linux")]
    let choose_screen = Rc::new(Cell::new(false));
    // Windows: which monitor is captured.
    #[cfg(windows)]
    let monitor = Rc::new(Cell::new(0u32));
    #[cfg(windows)]
    {
        let screens = sender::monitor_count();
        ui.set_can_switch_screen(screens > 1);
        ui.set_other_screen_label("Trocar tela".into());
        if screens > 1 {
            ui.set_next_screen(format!("Tela 1 de {screens}").into());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let s = settings.borrow();
        if s.restore_token.is_some() {
            ui.set_next_screen(next_screen_text(s.screen.as_deref()).into());
        }
    }
    let live_model = Rc::new(VecModel::<SharedString>::default());
    ui.set_live(ModelRc::from(live_model.clone()));

    {
        let ui_weak = ui.as_weak();
        ui.on_create_room(move || {
            let ui = ui_weak.unwrap();
            ui.set_room_busy(true);
            ui.set_error_text("".into());
            ui.set_joining(false);
            ui.set_room_status("Se o sistema pedir a senha, é para liberar a sala no firewall.".into());
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
                            ui.set_room_status("".into());
                            ui.set_stage_message("".into());
                        }
                        Err(e) => {
                            ui.set_room_status("".into());
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
            ui.set_joining(true);
            ui.set_room_status("Procurando a sala do seu amigo pela rede de casa e pela internet.".into());
            let ui_weak = ui_weak.clone();
            let code = code.to_string();
            std::thread::spawn(move || {
                let result = join_room(&code);
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_room_busy(false);
                    ui.set_joining(false);
                    ui.set_room_status("".into());
                    match result {
                        Ok(joined) => {
                            ROOM.with(|r| *r.borrow_mut() = joined);
                            ui.set_room_connected(true);
                            ui.set_room_state(2);
                            ui.set_invite_input("".into());
                            ui.set_stage_message("".into());
                        }
                        Err(e) => {
                            ui.set_error_text(capitalize(&format!("{e:#}")).into());
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
            ui.set_room_status("".into());
            ui.set_error_text("".into());
            ui.set_stage_message("".into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let sender = sender.clone();
        let settings = settings.clone();
        #[cfg(target_os = "linux")]
        let runtime = runtime.clone();
        #[cfg(target_os = "linux")]
        let choose_screen = choose_screen.clone();
        #[cfg(windows)]
        let monitor = monitor.clone();
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
            // Restored if switching screens fails while the old one keeps going.
            let previous_status = ui.get_transmit_status().to_string();
            ui.set_transmit_status(
                if cfg!(target_os = "linux") { "Escolha a tela na janela do sistema." } else { "Preparando a captura da tela…" }.into(),
            );
            {
                // Saved from a fresh load: the copy taken at startup would wipe
                // the room secret and firewall flag written since then.
                let mut s = Settings::load();
                s.name = name.clone();
                s.with_audio = Some(with_audio);
                s.save();
                *settings.borrow_mut() = s;
            }
            #[cfg(target_os = "linux")]
            let token = if choose_screen.replace(false) { None } else { Settings::load().restore_token };
            #[cfg(windows)]
            let token = None::<String>;
            #[cfg(windows)]
            let (screen, screens) = (monitor.get(), sender::monitor_count());
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
                    // A new screen was picked: the old transmission stops only now,
                    // so cancelling the picker leaves it running.
                    drop(sender.lock().unwrap().take());
                    let screen = match capture.size {
                        Some((w, h)) => format!("Tela de {w}×{h}"),
                        None => "Tela escolhida no sistema".to_owned(),
                    };
                    let (s, summary) = sender::start(capture, &target.publish(&name), with_audio, on_error)?;
                    *sender.lock().unwrap() = Some(s);
                    Ok((summary, token, screen))
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
                    drop(sender.lock().unwrap().take());
                    sender::start(&target.publish(&name), with_audio, screen, on_error).map(|(s, summary)| {
                        *sender.lock().unwrap() = Some(s);
                        let label = if screens > 1 { format!("Tela {} de {screens}", screen + 1) } else { "Tela principal".to_owned() };
                        (summary, token, label)
                    })
                };
                let _ = ui_weak.upgrade_in_event_loop(move |ui| {
                    ui.set_busy(false);
                    match result {
                        Ok((summary, token, screen)) => {
                            ui.set_transmitting(true);
                            ui.set_transmit_status(summary.sound.into());
                            ui.set_transmit_detail(format!("1080p60, {}", summary.encoder).into());
                            ui.set_transmit_screen(screen.clone().into());
                            if token.is_some() {
                                ui.set_next_screen(next_screen_text(Some(&screen)).into());
                                let mut s = Settings::load();
                                s.restore_token = token;
                                s.screen = Some(screen);
                                s.save();
                            }
                        }
                        Err(e) if sender.lock().unwrap().is_some() => {
                            ui.set_transmit_status(previous_status.into());
                            ui.set_error_text(format!("A tela continua a mesma: {e:#}").into());
                        }
                        Err(e) => {
                            ui.set_transmitting(false);
                            ui.set_transmit_status("".into());
                            ui.set_transmit_detail("".into());
                            ui.set_error_text(format!("Não deu para transmitir: {e:#}").into());
                        }
                    }
                });
            });
        });
    }
    {
        let ui_weak = ui.as_weak();
        #[cfg(target_os = "linux")]
        let choose_screen = choose_screen.clone();
        #[cfg(windows)]
        let monitor = monitor.clone();
        ui.on_switch_screen(move || {
            let ui = ui_weak.unwrap();
            // Linux: the system picker opens and the chosen screen goes on air.
            #[cfg(target_os = "linux")]
            {
                choose_screen.set(true);
                ui.invoke_transmit(ui.get_name(), ui.get_with_audio());
            }
            // Windows: the next monitor; before going live it only changes
            // which one will be used.
            #[cfg(windows)]
            {
                let screens = sender::monitor_count();
                monitor.set((monitor.get() + 1) % screens);
                ui.set_next_screen(format!("Tela {} de {screens}", monitor.get() + 1).into());
                if ui.get_transmitting() {
                    ui.invoke_transmit(ui.get_name(), ui.get_with_audio());
                }
            }
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
            ui.set_transmit_detail("".into());
            ui.set_transmit_screen("".into());
        });
    }
    {
        let ui_weak = ui.as_weak();
        let player = player.clone();
        let (frame_count, frame_size) = (frame_count.clone(), frame_size.clone());
        let reconnects = reconnects.clone();
        ui.on_watch(move |name| {
            let ui = ui_weak.unwrap();
            let Some(target) = ROOM.with(|r| r.borrow().target()) else { return };
            player.borrow_mut().take();
            ui.set_has_frame(false);
            ui.set_stage_message(format!("Conectando a {name}…").into());
            let frames = ui.as_weak();
            let errors = ui.as_weak();
            let (count, size) = (frame_count.clone(), frame_size.clone());
            let sampled = Arc::new(AtomicU32::new(0));
            let stream = name.to_string();
            let (got_frame, failures) = (reconnects.clone(), reconnects.clone());
            let result = player::Player::start(
                &target.read(&name),
                move |pixels, pending| {
                    let (count, size) = (count.clone(), size.clone());
                    got_frame.store(0, Ordering::Relaxed);
                    // About ten times a second is enough for light on a wall.
                    let ambient = (sampled.fetch_add(1, Ordering::Relaxed) % 6 == 0).then(|| ambient_light(&pixels));
                    let _ = frames.upgrade_in_event_loop(move |ui| {
                        if let Some(color) = ambient {
                            ui.set_ambient(color);
                        }
                        // Counted on the UI thread: this is what the window actually shows.
                        count.fetch_add(1, Ordering::Relaxed);
                        *size.lock().unwrap() = (pixels.width(), pixels.height());
                        ui.set_frame(Image::from_rgba8(pixels));
                        ui.set_has_frame(true);
                        pending.store(false, Ordering::Release);
                    });
                },
                move |message| {
                    eprintln!("player: {message}");
                    let (stream, failures) = (stream.clone(), failures.clone());
                    let _ = errors.upgrade_in_event_loop(move |ui| {
                        if ui.get_watching() != stream.as_str() {
                            return;
                        }
                        ui.set_has_frame(false);
                        // The sender restarts when switching screens (or, on
                        // Windows, when Discord restarts): try again on our own.
                        if failures.fetch_add(1, Ordering::Relaxed) < 5 {
                            ui.set_stage_message(format!("Reconectando a {stream}…").into());
                            let ui_weak = ui.as_weak();
                            slint::Timer::single_shot(Duration::from_millis(1500), move || {
                                if let Some(ui) = ui_weak.upgrade() {
                                    if ui.get_watching() == stream.as_str() {
                                        ui.invoke_watch(stream.as_str().into());
                                    }
                                }
                            });
                        } else {
                            ui.set_stage_message(
                                format!("A transmissão de {stream} caiu. Escolha de novo em Ao vivo agora para tentar outra vez.").into(),
                            );
                        }
                    });
                },
            );
            match result {
                Ok(p) => {
                    p.set_muted(ui.get_muted());
                    *player.borrow_mut() = Some(p);
                    ui.set_watching(name);
                }
                Err(e) => {
                    eprintln!("player: {e:#}");
                    ui.set_stage_message(format!("Não deu para assistir a {name}. Tente de novo em instantes.").into());
                }
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
            ui.set_stage_message("".into());
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
        // Ticks the watched stream has been missing from the list.
        let missing = Cell::new(0u32);
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let names = ROOM.with(|r| match &*r.borrow() {
                Room::None => Vec::new(),
                Room::Host { .. } => server::live_streams().unwrap_or_default(),
                Room::Guest { tunnel, .. } => {
                    ui.set_room_connected(tunnel.connected());
                    tunnel.live()
                }
            });
            let current: Vec<String> = live_model.iter().map(|s| s.to_string()).collect();
            if current != names {
                live_model.set_vec(names.iter().map(SharedString::from).collect::<Vec<_>>());
            }
            let watching = ui.get_watching().to_string();
            let gone = !watching.is_empty() && !names.contains(&watching);
            missing.set(if gone { missing.get() + 1 } else { 0 });
            // Two ticks (4 s): a screen switch briefly takes it off the list.
            if missing.get() >= 2 {
                missing.set(0);
                ui.invoke_stop_watch();
                ui.set_stage_message(format!("A transmissão de {watching} terminou.").into());
            }
        });
    }

    ui.run()?;
    sender.lock().unwrap().take();
    player.borrow_mut().take();
    ROOM.with(|r| *r.borrow_mut() = Room::None);
    Ok(())
}


