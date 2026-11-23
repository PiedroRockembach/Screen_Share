//! Windows: D3D11 screen capture, hardware encoder, and the PC's sound captured
//! WITHOUT Discord's process tree (wasapi2src exclude-process-tree). The
//! excluded PID must be Discord's current one: when Discord opens, closes or
//! restarts, the pipeline is rebuilt right away so its voice can never leak.

use super::{tail, watch_errors};
use anyhow::{Context, Result, bail};
use gstreamer as gst;
use gstreamer::prelude::*;
use regex::Regex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

static DISCORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(discord|discordptb|discordcanary|discorddevelopment|vesktop|legcord)\.exe$").unwrap());

struct Encoder {
    name: &'static str,
    element: String,
    /// Accepts GPU (D3D11) frames directly; otherwise frames are downloaded first.
    d3d11: bool,
}

/// Bitrate is super::MAX_KBPS (20000) and the VBV a tenth of a second of it.
/// Each list tries the full quality settings first, then plainer ones.
const CANDIDATES: &[(&str, &str, bool, &[&str])] = &[
    ("placa NVIDIA", "nvd3d11h264enc", true, &[
        "bitrate=20000 vbv-buffer-size=2000 gop-size=60 bframes=0 rc-mode=cbr zero-reorder-delay=true preset=p5 tune=low-latency spatial-aq=true",
        "bitrate=20000 gop-size=60 bframes=0 rc-mode=cbr zero-reorder-delay=true",
        "bitrate=20000",
    ]),
    ("placa AMD", "amfh264enc", true, &[
        "bitrate=20000 gop-size=60 rate-control=cbr usage=low-latency preset=quality",
        "bitrate=20000 gop-size=60 rate-control=cbr usage=low-latency",
        "bitrate=20000",
    ]),
    ("placa Intel", "qsvh264enc", true, &[
        "bitrate=20000 gop-size=60 b-frames=0 rate-control=cbr target-usage=3",
        "bitrate=20000 gop-size=60 b-frames=0 rate-control=cbr",
        "bitrate=20000",
    ]),
    ("codificador do Windows", "mfh264enc", false, &[
        "bitrate=20000 gop-size=60 bframes=0 rc-mode=cbr low-latency=true quality-vs-speed=70",
        "bitrate=20000 gop-size=60 bframes=0 rc-mode=cbr low-latency=true",
        "bitrate=20000",
    ]),
];

fn conversion(d3d11: bool) -> &'static str {
    if d3d11 { "" } else { " ! d3d11download ! videoconvert ! video/x-raw,format=NV12" }
}

/// Encodes a few test frames with each candidate; the first that works wins.
fn choose_encoder() -> Result<Encoder> {
    static CHOSEN: Mutex<Option<(&'static str, String, bool)>> = Mutex::new(None);
    if let Some((name, element, d3d11)) = CHOSEN.lock().unwrap().clone() {
        return Ok(Encoder { name, element, d3d11 });
    }
    for (name, factory, d3d11, props) in CANDIDATES {
        if gst::ElementFactory::find(factory).is_none() {
            continue;
        }
        for p in *props {
            let element = format!("{factory} name=enc {p}");
            let test = format!(
                "d3d11testsrc num-buffers=30 ! video/x-raw(memory:D3D11Memory),width=1920,height=1080,framerate=60/1{} ! {element} ! h264parse ! fakesink",
                conversion(*d3d11)
            );
            let Ok(pipeline) = gst::parse::launch(&test) else { continue };
            let ok = pipeline.set_state(gst::State::Playing).is_ok()
                && pipeline.bus().and_then(|bus| {
                    bus.timed_pop_filtered(gst::ClockTime::from_seconds(20), &[gst::MessageType::Eos, gst::MessageType::Error])
                }).is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
            let _ = pipeline.set_state(gst::State::Null);
            if ok {
                *CHOSEN.lock().unwrap() = Some((name, element.clone(), *d3d11));
                return Ok(Encoder { name, element, d3d11: *d3d11 });
            }
        }
    }
    bail!("nenhum codificador de vídeo funcionou neste PC")
}

/// Discord processes whose parent isn't another Discord: excluding that tree
/// removes every Discord sound from the capture.
fn discord_roots() -> Vec<u32> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    let mut processes = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
        while ok {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            processes.push((entry.th32ProcessID, entry.th32ParentProcessID, DISCORD.is_match(&name)));
            ok = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    let discord: std::collections::HashSet<u32> = processes.iter().filter(|p| p.2).map(|p| p.0).collect();
    let mut roots: Vec<u32> = processes.iter().filter(|p| p.2 && !discord.contains(&p.1)).map(|p| p.0).collect();
    roots.sort();
    roots
}

fn excludes_processes() -> bool {
    gst::ElementFactory::make("wasapi2src").build().is_ok_and(|e| e.has_property("loopback-mode"))
}

/// How many monitors this PC has; "Trocar tela" walks through them.
pub fn monitor_count() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CMONITORS};
    // SAFETY: plain query with no pointers.
    unsafe { GetSystemMetrics(SM_CMONITORS) }.max(1) as u32
}

fn build(encoder: &Encoder, uri: &str, monitor: u32, discord_pid: Option<u32>) -> Result<gst::Element> {
    let mut description = format!(
        "d3d11screencapturesrc show-cursor=true monitor-index={monitor} ! video/x-raw(memory:D3D11Memory),framerate=60/1 \
         ! d3d11convert ! video/x-raw(memory:D3D11Memory),width=1920,height=1080,pixel-aspect-ratio=1/1{} \
         ! queue max-size-buffers=2 leaky=downstream ! {}{}",
        conversion(encoder.d3d11),
        encoder.element,
        tail(uri)
    );
    if let Some(pid) = discord_pid {
        description += &format!(
            " wasapi2src loopback=true low-latency=true loopback-mode=exclude-process-tree loopback-target-pid={pid} \
             ! queue max-size-time=200000000 max-size-buffers=0 max-size-bytes=0 leaky=downstream \
             ! audioconvert ! audioresample ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 \
             ! queue max-size-time=6000000000 max-size-buffers=0 max-size-bytes=0 ! mux."
        );
    }
    Ok(gst::parse::launch(&description).context("montar a transmissão")?)
}

fn sound_note(with_audio: bool, pid: Option<u32>, roots: usize, supported: bool) -> &'static str {
    match (with_audio, pid, roots, supported) {
        (false, ..) => "Sem som.",
        (true, _, _, false) => "Sem som: este Windows não permite tirar o Discord do áudio.",
        (true, Some(_), ..) => "Com o som do PC, sem o Discord.",
        (true, None, 0, _) => "Sem som por enquanto: abra o Discord e o som liga sozinho.",
        (true, None, _, _) => "Sem som: há mais de um Discord aberto.",
    }
}

pub struct Sender {
    stop: Arc<AtomicBool>,
    current: Arc<Mutex<Option<gst::Element>>>,
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(p) = self.current.lock().unwrap().take() {
            let _ = p.set_state(gst::State::Null);
        }
    }
}

pub fn start(uri: &str, with_audio: bool, monitor: u32, on_error: impl Fn(String) + Send + Sync + 'static) -> Result<(Sender, super::Summary)> {
    let encoder = choose_encoder()?;
    let supported = excludes_processes();
    let roots = discord_roots();
    // No Discord found means no audio at all: never risk the call's voice.
    let pid = (with_audio && supported && roots.len() == 1).then(|| roots[0]);
    let on_error = Arc::new(on_error);
    let pipeline = build(&encoder, uri, monitor, pid)?;
    {
        let on_error = on_error.clone();
        watch_errors(&pipeline, move |m| on_error(m));
    }
    pipeline.set_state(gst::State::Playing).context("iniciar a transmissão")?;
    let summary = super::Summary { encoder: encoder.name, sound: sound_note(with_audio, pid, roots.len(), supported) };

    let stop = Arc::new(AtomicBool::new(false));
    let current = Arc::new(Mutex::new(Some(pipeline)));
    {
        let current = current.clone();
        super::adapt_bitrate(move || current.lock().unwrap().clone(), stop.clone());
    }
    if with_audio && supported {
        let (stop, current, uri) = (stop.clone(), current.clone(), uri.to_owned());
        std::thread::spawn(move || {
            let mut excluded = pid;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_secs(1));
                let roots = discord_roots();
                let wanted = (roots.len() == 1).then(|| roots[0]);
                if wanted == excluded {
                    continue;
                }
                // Stop first: with a new Discord PID the old exclusion no longer applies.
                let mut slot = current.lock().unwrap();
                if let Some(old) = slot.take() {
                    let _ = old.set_state(gst::State::Null);
                }
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                excluded = wanted;
                match build(&encoder, &uri, monitor, wanted) {
                    Ok(p) => {
                        let on_error = on_error.clone();
                        watch_errors(&p, move |m| on_error(m));
                        if p.set_state(gst::State::Playing).is_ok() {
                            *slot = Some(p);
                        }
                    }
                    Err(e) => {
                        on_error(format!("{e:#}"));
                        break;
                    }
                }
            }
        });
    }
    Ok((Sender { stop, current }, summary))
}
