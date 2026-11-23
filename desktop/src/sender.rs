#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use crate::{audio_linux::AudioRouting, capture_linux::ScreenCapture};

#[cfg(windows)]
#[path = "sender_windows.rs"]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{Sender, monitor_count, start};

#[cfg(target_os = "linux")]
pub struct Sender {
    pipeline: gst::Element,
    stop: Arc<AtomicBool>,
    _audio: Option<AudioRouting>,
    _capture: ScreenCapture,
}

#[cfg(target_os = "linux")]
impl Drop for Sender {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(target_os = "linux")]
fn has(element: &str) -> bool {
    gst::ElementFactory::find(element).is_some()
}

/// Output is always 1080p60. NVENC first, then VA-API, then the CPU (x264, or
/// OpenH264 where x264 isn't packaged, as on Fedora). Each list starts with
/// the full quality settings and falls back to plainer ones: older GStreamer
/// (Debian 12 has 1.22) lacks NVENC's p1-p7 presets and tunes.
/// Fast motion needs room: the VBV buffer holds a tenth of a second of video.
/// Larger (a quarter second) made keyframes of ~600 KB that overflowed the
/// receiving sockets in one burst.
#[cfg(target_os = "linux")]
const CANDIDATES: &[(&str, &str, &str, &[&str])] = &[
    ("placa NVIDIA", "nvh264enc", "", &[
        "bitrate=20000 rc-mode=cbr vbv-buffer-size=2000 gop-size=60 bframes=0 zerolatency=true preset=p5 tune=low-latency spatial-aq=true",
        "bitrate=20000 rc-mode=cbr vbv-buffer-size=2000 gop-size=60 bframes=0 zerolatency=true preset=low-latency-hq",
        "bitrate=20000",
    ]),
    ("placa de vídeo", "vah264enc", "", &[
        "bitrate=20000 cpb-size=2000 rate-control=cbr key-int-max=60 b-frames=0",
        "bitrate=20000 rate-control=cbr key-int-max=60 b-frames=0",
        "bitrate=20000",
    ]),
    ("processador", "x264enc", "", &[
        "bitrate=20000 vbv-buf-capacity=100 tune=zerolatency speed-preset=superfast key-int-max=60 bframes=0",
        "bitrate=20000 tune=zerolatency speed-preset=ultrafast",
    ]),
    // OpenH264 takes only I420 and counts the bitrate in bit/s (adapt_bitrate
    // converts).
    ("processador", "openh264enc", "videoconvert ! video/x-raw,format=I420 ! ", &[
        "bitrate=20000000 rate-control=bitrate gop-size=60 complexity=low multi-thread=0",
        "bitrate=20000000",
    ]),
];

/// Encodes a few test frames with each candidate; the first that works wins.
#[cfg(target_os = "linux")]
fn encoder() -> Result<(&'static str, String)> {
    use std::sync::Mutex;
    static CHOSEN: Mutex<Option<(&'static str, String)>> = Mutex::new(None);
    if let Some(chosen) = CHOSEN.lock().unwrap().clone() {
        return Ok(chosen);
    }
    for (name, factory, input, props) in CANDIDATES {
        if !has(factory) {
            continue;
        }
        for p in *props {
            let element = format!("{input}{factory} name=enc {p}");
            let test = format!(
                "videotestsrc num-buffers=30 ! video/x-raw,format=NV12,width=1920,height=1080,framerate=60/1 ! {element} ! h264parse ! fakesink"
            );
            let Ok(pipeline) = gst::parse::launch(&test) else { continue };
            let ok = pipeline.set_state(gst::State::Playing).is_ok()
                && pipeline
                    .bus()
                    .and_then(|bus| {
                        bus.timed_pop_filtered(gst::ClockTime::from_seconds(20), &[gst::MessageType::Eos, gst::MessageType::Error])
                    })
                    .is_some_and(|m| matches!(m.view(), gst::MessageView::Eos(_)));
            let _ = pipeline.set_state(gst::State::Null);
            if ok {
                *CHOSEN.lock().unwrap() = Some((name, element.clone()));
                return Ok((name, element));
            }
        }
    }
    anyhow::bail!("nenhum codificador de vídeo funcionou neste PC: instale os plugins do GStreamer (x264 ou OpenH264)")
}

/// What the window shows about a running transmission.
pub struct Summary {
    /// Where the video is encoded, as a person would say it ("placa NVIDIA").
    pub encoder: &'static str,
    /// Whether the PC's sound goes along, as a full sentence.
    pub sound: &'static str,
}

/// The server only announces the tracks it sees first and the encoder is slow
/// to start: the mux latency waits for video instead of announcing audio only.
/// With sync=false that wait doesn't become delay once both are flowing.
/// The encoder must be named `enc` for [`adapt_bitrate`].
fn tail(uri: &str) -> String {
    format!(
        " ! h264parse config-interval=-1 ! queue ! mux. mpegtsmux name=mux alignment=6 latency=5000000000 ! srtsink name=out sync=false uri=\"{uri}\""
    )
}

/// Bitrate range in kbit/s. 1080p60 with fast motion needs ~20 Mbps to stay
/// clean; a weak upload gets a softer image instead of stalls and smears.
/// The encoder lists (CANDIDATES here and in sender_windows.rs) repeat it.
pub const MAX_KBPS: u32 = 20000;
const MIN_KBPS: u32 = 2000;

/// Follows the SRT connection end to end (through the tunnel too). An RTT well
/// above its recent minimum means a queue is building somewhere on the path,
/// typical of a mobile upload: the bitrate drops right away. After a calm
/// stretch it climbs back slowly. `current` gives the pipeline in use, which
/// may be rebuilt meanwhile (Windows does when Discord restarts).
pub fn adapt_bitrate(current: impl Fn() -> Option<gst::Element> + Send + 'static, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut kbps = MAX_KBPS;
        let mut rtts: VecDeque<f64> = VecDeque::new();
        let mut last: Option<(i64, i64)> = None;
        let mut calm_since = Instant::now();
        let mut hold_until = Instant::now();
        let mut queued_for = 0u32;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            let Some(bin) = current().and_then(|p| p.downcast::<gst::Bin>().ok()) else { continue };
            // Every encoder we use applies a new bitrate while playing, even
            // those whose property doesn't say so: qsvh264enc resets its
            // session and mfh264enc reopens itself (GStreamer 1.28 sources).
            let (Some(enc), Some(out)) = (bin.by_name("enc"), bin.by_name("out")) else { continue };
            let Ok(Some(stats)) = out.property_value("stats").get::<Option<gst::Structure>>() else { continue };
            let (Ok(rtt), Ok(sent)) = (stats.get::<f64>("rtt-ms"), stats.get::<i64>("packets-sent")) else { continue };
            // Lost packets that SRT resends in time are harmless (Wi-Fi has
            // them); dropped ones came too late and show up as smears.
            let dropped = stats.get::<i32>("packets-sent-dropped").map(i64::from).unwrap_or(0);
            if sent == 0 {
                continue;
            }
            // A rebuilt pipeline starts its counters (and its bitrate) over.
            let (sent_delta, dropped_delta) = match last {
                Some((s, d)) if sent >= s => (sent - s, dropped - d),
                _ => {
                    rtts.clear();
                    (0, 0)
                }
            };
            last = Some((sent, dropped));
            rtts.push_back(rtt);
            if rtts.len() > 60 {
                rtts.pop_front();
            }
            let base = rtts.iter().copied().fold(f64::MAX, f64::min);
            let drops = if sent_delta > 0 { dropped_delta as f64 / sent_delta as f64 } else { 0.0 };
            // A keyframe or Wi-Fi jitter lifts the RTT for an instant; a real
            // queue keeps it well up for 1.5 s.
            queued_for = if rtt > (base + 60.0).max(base * 2.0) { queued_for + 1 } else { 0 };
            let congested = queued_for >= 3 || drops > 0.01;

            let now = Instant::now();
            let before = kbps;
            if congested {
                calm_since = now;
                if now >= hold_until {
                    kbps = (kbps * 7 / 10).max(MIN_KBPS);
                    // Give the queue time to drain before judging again.
                    hold_until = now + Duration::from_secs(2);
                }
            } else if calm_since.elapsed() >= Duration::from_secs(3) && kbps < MAX_KBPS {
                kbps = (kbps * 115 / 100).min(MAX_KBPS);
                calm_since = now;
            }
            if kbps != before {
                eprintln!("taxa: {before} -> {kbps} kbit/s (rtt {rtt:.0} ms, mínimo {base:.0} ms, descartes {:.1}%)", drops * 100.0);
            }
            // OpenH264 counts bit/s, every other encoder kbit/s.
            let unit = if enc.factory().is_some_and(|f| f.name() == "openh264enc") { 1000 } else { 1 };
            let applied = enc.property_value("bitrate").get::<u32>().ok();
            if applied != Some(kbps * unit) {
                enc.set_property_from_str("bitrate", &(kbps * unit).to_string());
                // The burst allowance follows the rate: a tenth of a second.
                if enc.find_property("vbv-buffer-size").is_some() {
                    enc.set_property_from_str("vbv-buffer-size", &(kbps / 10).to_string());
                }
            }
        }
    });
}

#[cfg(target_os = "linux")]
pub fn start(capture: ScreenCapture, uri: &str, with_audio: bool, on_error: impl Fn(String) + Send + 'static) -> Result<(Sender, Summary)> {
    use std::os::fd::AsRawFd;

    let audio = if with_audio { AudioRouting::start() } else { None };
    let (encoder_name, encoder) = encoder()?;
    let mut description = format!(
        "pipewiresrc fd={} path={} do-timestamp=true keepalive-time=100 ! videorate ! videoconvertscale n-threads=4 \
         ! video/x-raw,format=NV12,width=1920,height=1080,framerate=60/1,pixel-aspect-ratio=1/1 \
         ! queue max-size-buffers=2 leaky=downstream ! {encoder}{}",
        capture.fd.as_raw_fd(),
        capture.node,
        tail(uri)
    );
    if audio.is_some() {
        description += &format!(
            " pulsesrc device={}.monitor do-timestamp=true ! queue ! audioconvert ! audioresample \
             ! audio/x-raw,rate=48000,channels=2 ! opusenc bitrate=128000 \
             ! queue max-size-time=6000000000 max-size-buffers=0 max-size-bytes=0 ! mux.",
            crate::audio_linux::SINK
        );
    }
    let pipeline = gst::parse::launch(&description).context("montar a transmissão")?;
    watch_errors(&pipeline, on_error);
    pipeline.set_state(gst::State::Playing).context("iniciar a transmissão")?;
    let sound = match (with_audio, audio.is_some()) {
        (false, _) => "Sem som.",
        (true, true) => "Com o som do PC, sem o Discord.",
        (true, false) => "Sem som: não deu para criar a saída de áudio da sala.",
    };
    let summary = Summary { encoder: encoder_name, sound };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let pipeline = pipeline.clone();
        adapt_bitrate(move || Some(pipeline.clone()), stop.clone());
    }
    Ok((Sender { pipeline, stop, _audio: audio, _capture: capture }, summary))
}

pub fn watch_errors(pipeline: &gst::Element, on_error: impl Fn(String) + Send + 'static) {
    let bus = pipeline.bus().expect("pipeline has a bus");
    std::thread::spawn(move || {
        for message in bus.iter_timed(gst::ClockTime::NONE) {
            match message.view() {
                gst::MessageView::Error(err) => {
                    on_error(format!("{} ({})", err.error(), err.debug().unwrap_or_default()));
                    break;
                }
                gst::MessageView::Eos(_) => break,
                _ => {}
            }
        }
    });
}
