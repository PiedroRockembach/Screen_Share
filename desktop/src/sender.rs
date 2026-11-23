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
pub use windows_impl::{Sender, start};

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

/// Output is always 1080p60. NVENC first, then VA-API, then the CPU.
#[cfg(target_os = "linux")]
fn encoder() -> (&'static str, &'static str) {
    if has("nvh264enc") {
        (
            "NVIDIA",
            "nvh264enc name=enc bitrate=9000 rc-mode=cbr gop-size=60 bframes=0 zerolatency=true preset=p4 tune=ultra-low-latency ! video/x-h264,profile=high",
        )
    } else if has("vah264enc") {
        ("VA-API", "vah264enc name=enc bitrate=9000 rate-control=cbr key-int-max=60 b-frames=0")
    } else {
        ("processador", "x264enc name=enc bitrate=9000 tune=zerolatency speed-preset=ultrafast key-int-max=60 bframes=0")
    }
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

/// Bitrate range in kbit/s. The picture stays 1080p60: a weak upload gets a
/// softer image instead of stalls and smears.
const MAX_KBPS: u32 = 9000;
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
        let mut was_queued = false;
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(500));
            let Some(bin) = current().and_then(|p| p.downcast::<gst::Bin>().ok()) else { continue };
            let (Some(enc), Some(out)) = (bin.by_name("enc"), bin.by_name("out")) else { continue };
            let mutable = enc
                .find_property("bitrate")
                .is_some_and(|p| p.flags().contains(gst::PARAM_FLAG_MUTABLE_PLAYING));
            if !mutable {
                continue;
            }
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
            // A keyframe alone lifts the RTT for an instant; a real queue
            // keeps it up for at least two samples in a row.
            let queued = rtt > base * 2.0 + 40.0;
            let congested = (queued && was_queued) || drops > 0.01;
            was_queued = queued;

            let now = Instant::now();
            if congested {
                calm_since = now;
                if now >= hold_until {
                    kbps = (kbps * 7 / 10).max(MIN_KBPS);
                    // Give the queue time to drain before judging again.
                    hold_until = now + Duration::from_secs(2);
                }
            } else if calm_since.elapsed() >= Duration::from_secs(4) && kbps < MAX_KBPS {
                kbps = (kbps * 11 / 10).min(MAX_KBPS);
                calm_since = now;
            }
            let applied = enc.property_value("bitrate").get::<u32>().ok();
            if applied != Some(kbps) {
                enc.set_property_from_str("bitrate", &kbps.to_string());
            }
        }
    });
}

#[cfg(target_os = "linux")]
pub fn start(capture: ScreenCapture, uri: &str, with_audio: bool, on_error: impl Fn(String) + Send + 'static) -> Result<(Sender, String)> {
    use std::os::fd::AsRawFd;

    let audio = if with_audio { AudioRouting::start() } else { None };
    let (encoder_name, encoder) = encoder();
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
        (false, _) => "sem som",
        (true, true) => "com o som do PC, sem o Discord",
        (true, false) => "sem som: não consegui criar a saída de áudio da Sala",
    };
    let summary = format!("1080p60, {encoder_name}, {sound}");
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
