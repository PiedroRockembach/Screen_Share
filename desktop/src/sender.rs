#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;

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
    #[cfg(target_os = "linux")]
    _audio: Option<AudioRouting>,
    #[cfg(target_os = "linux")]
    _capture: ScreenCapture,
}

#[cfg(target_os = "linux")]
impl Drop for Sender {
    fn drop(&mut self) {
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
            "nvh264enc bitrate=9000 rc-mode=cbr gop-size=60 bframes=0 zerolatency=true preset=p4 tune=ultra-low-latency ! video/x-h264,profile=high",
        )
    } else if has("vah264enc") {
        ("VA-API", "vah264enc bitrate=9000 rate-control=cbr key-int-max=60 b-frames=0")
    } else {
        ("processador", "x264enc bitrate=9000 tune=zerolatency speed-preset=ultrafast key-int-max=60 bframes=0")
    }
}

/// The server only announces the tracks it sees first and the encoder is slow
/// to start: the mux latency waits for video instead of announcing audio only.
/// With sync=false that wait doesn't become delay once both are flowing.
fn tail(uri: &str) -> String {
    format!(
        " ! h264parse config-interval=-1 ! queue ! mux. mpegtsmux name=mux alignment=6 latency=5000000000 ! srtsink sync=false uri=\"{uri}\""
    )
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
    Ok((Sender { pipeline, _audio: audio, _capture: capture }, summary))
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
