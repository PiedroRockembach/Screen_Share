use anyhow::{Context, Result, anyhow};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use gstreamer_video::prelude::*;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Player {
    pipeline: gst::Element,
    volume: gst::Element,
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn decoder() -> &'static str {
    // OpenH264 last: on Fedora without RPM Fusion it may be the only one.
    ["nvh264dec", "vah264dec", "d3d11h264dec", "avdec_h264", "openh264dec"]
        .into_iter()
        .find(|d| gst::ElementFactory::find(d).is_some())
        .unwrap_or("avdec_h264")
}

impl Player {
    /// Decodes the stream and hands each frame to `on_frame` as RGBA. A frame is
    /// skipped while the window hasn't drawn the previous one, so a slow repaint
    /// never builds up delay. The demuxer's default 700 ms "smoothing" is off.
    pub fn start(
        uri: &str,
        on_frame: impl Fn(SharedPixelBuffer<Rgba8Pixel>, Arc<AtomicBool>) + Send + Sync + 'static,
        on_error: impl Fn(String) + Send + 'static,
    ) -> Result<Self> {
        let pipeline = gst::Pipeline::new();
        // GStreamer's own estimate adds ~250 ms of waiting; audio and video stay
        // in sync with this fixed margin instead.
        pipeline.set_latency(Some(gst::ClockTime::from_mseconds(80)));
        let source = gst::ElementFactory::make("srtsrc").property("uri", uri).build()?;
        let demux = gst::ElementFactory::make("tsdemux").property("latency", 0i32).build()?;
        pipeline.add_many([&source, &demux])?;
        source.link(&demux)?;

        let video = gst::parse::bin_from_description(
            &format!(
                "queue ! h264parse ! {} ! videoconvertscale n-threads=4 ! video/x-raw,format=RGBA ! appsink name=frames max-buffers=1 drop=true",
                decoder()
            ),
            true,
        )?;
        let audio = gst::parse::bin_from_description(
            "queue ! opusparse ! opusdec ! audioconvert ! audioresample ! volume name=volume ! autoaudiosink",
            true,
        )?;
        let appsink = video
            .by_name("frames")
            .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
            .ok_or_else(|| anyhow!("appsink ausente"))?;
        let pending = Arc::new(AtomicBool::new(false));
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    if pending.load(Ordering::Acquire) {
                        return Ok(gst::FlowSuccess::Ok);
                    }
                    let info = sample
                        .caps()
                        .and_then(|c| gst_video::VideoInfo::from_caps(c).ok())
                        .ok_or(gst::FlowError::NotNegotiated)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
                        .map_err(|_| gst::FlowError::Error)?;
                    let (width, height) = (info.width(), info.height());
                    let stride = frame.plane_stride()[0] as usize;
                    let data = frame.plane_data(0).map_err(|_| gst::FlowError::Error)?;
                    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
                    let row = width as usize * 4;
                    let target = pixels.make_mut_bytes();
                    for y in 0..height as usize {
                        target[y * row..(y + 1) * row].copy_from_slice(&data[y * stride..y * stride + row]);
                    }
                    pending.store(true, Ordering::Release);
                    on_frame(pixels, pending.clone());
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let volume = audio.by_name("volume").ok_or_else(|| anyhow!("volume ausente"))?;
        let weak = pipeline.downgrade();
        demux.connect_pad_added(move |_, pad| {
            let Some(pipeline) = weak.upgrade() else { return };
            let caps = pad.current_caps().or_else(|| Some(pad.query_caps(None)));
            let kind = caps.and_then(|c| c.structure(0).map(|s| s.name().to_string())).unwrap_or_default();
            let branch = if kind.starts_with("video/x-h264") {
                &video
            } else if kind.starts_with("audio/x-opus") {
                &audio
            } else {
                return;
            };
            if branch.parent().is_some() || pipeline.add(branch).is_err() {
                return;
            }
            let _ = branch.sync_state_with_parent();
            if let Some(sink) = branch.static_pad("sink") {
                let _ = pad.link(&sink);
            }
        });
        let pipeline: gst::Element = pipeline.upcast();
        crate::sender::watch_errors(&pipeline, on_error);
        pipeline.set_state(gst::State::Playing).context("abrir a transmissão")?;
        Ok(Self { pipeline, volume })
    }

    pub fn set_muted(&self, muted: bool) {
        self.volume.set_property("mute", muted);
    }
}
