use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSrc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

mod hotplug;
mod ingress;
mod mirror;
mod startup;
use ingress::{Feed, Ingress};

use super::display::{Display, ModeInfo, OutputInfo};
use crate::input::h264;
use crate::output::overlay::{self, Overlay, Rect};
use crate::settings::{DisplaySettings, Scaling};

type DisplayModes = (Vec<ModeInfo>, Option<ModeInfo>, Vec<OutputInfo>);

// kmssink takes the decoder's YUV dmabufs straight onto a display plane. A
// v4l2convert here turned every frame into 32-bit RGB first, which capped
// HDMI at ~20 fps once the goggles went to 60 fps. vc4 is an atomic driver
// whose commit already waits for vblank; without skip-vsync kmssink waited a
// second time, 30 fps at most
const PIPELINE: &str = "appsrc name=src is-live=true block=false max-bytes=2000000 format=time ! \
    h264parse config-interval=-1 ! \
    v4l2h264dec ! tee name=video ";

// DRM plane rotation bits
const ROTATE_0: i32 = 0x1;
const ROTATE_180: i32 = 0x4;
const REFLECT_X: i32 = 0x10;

struct Running {
    pipeline: gst::Pipeline,
    appsrc: AppSrc,
    sink: gst::Element,
    sinks: Vec<gst::Element>,
    failed: Arc<AtomicBool>,
    startup: Mutex<startup::DecoderStartup>,
}

struct State {
    cfg: DisplaySettings,
    displays: Vec<Display>,
    running: Option<Running>,
    retry_after: Option<Instant>,
    topology: Option<hotplug::Topology>,
    last_error: Option<String>,
}

pub struct GstPlayer {
    overlay: Arc<Overlay>,
    state: Mutex<State>,
    enabled: AtomicBool,
    // Serializes input changes, AU assembly, pushes and pipeline replacement.
    // Always acquire ingress before state; bus handlers never acquire either.
    ingress: Mutex<Ingress>,
    live_generation: AtomicU64,
}

fn configure(st: &mut State) -> Result<(), String> {
    if let Some(old) = st.running.take() {
        old.pipeline
            .set_state(gst::State::Null)
            .map_err(|error| format!("Couldn't stop HDMI output: {error}"))?;
    }
    // Release old framebuffers and the DRM master before opening a new route.
    st.displays.clear();
    if !st.cfg.hdmi_output {
        eprintln!("[hdmi] output off");
        return Ok(());
    }
    let mut last_error = None;
    let displays = Display::open(st.cfg.ports)?;
    let found = !displays.is_empty();
    for mut display in displays {
        match display.set_mode(&st.cfg.mode) {
            Ok(_) => st.displays.push(display),
            Err(error) => {
                eprintln!("[hdmi] {} unavailable: {error}", display.port());
                last_error = Some(error);
            }
        }
    }
    if st.displays.is_empty() {
        if found {
            return Err(last_error.unwrap_or_else(|| "No usable HDMI modes".into()));
        }
        eprintln!("[hdmi] waiting for a connected screen");
        return Ok(());
    }
    let running = build(&st.cfg, &st.displays)?;
    running
        .pipeline
        .set_state(gst::State::Playing)
        .map_err(|error| format!("Couldn't start HDMI output: {error}"))?;
    eprintln!(
        "[hdmi] output on: {}",
        st.displays
            .iter()
            .map(|display| display.port())
            .collect::<Vec<_>>()
            .join(", ")
    );
    st.running = Some(running);
    Ok(())
}

fn record_result(st: &mut State, result: &Result<(), String>) {
    match result {
        Ok(()) => {
            st.retry_after = st
                .topology
                .as_ref()
                .filter(|topology| topology.connected_count() > st.displays.len())
                .map(|_| Instant::now() + Duration::from_secs(3));
            st.last_error = None;
        }
        Err(error) => {
            if st.last_error.as_ref() != Some(error) {
                eprintln!("[hdmi] {error}; retrying connected outputs");
            }
            st.last_error = Some(error.clone());
            st.retry_after = Some(Instant::now() + Duration::from_secs(2));
        }
    }
}

impl GstPlayer {
    pub fn new(cfg: &DisplaySettings, overlay: Arc<Overlay>) -> anyhow::Result<Arc<Self>> {
        gst::init()?;
        let this = Arc::new(Self {
            overlay,
            state: Mutex::new(State {
                cfg: cfg.clone(),
                displays: Vec::new(),
                running: None,
                retry_after: None,
                topology: None,
                last_error: None,
            }),
            enabled: AtomicBool::new(false),
            ingress: Mutex::new(Ingress::new()),
            live_generation: AtomicU64::new(0),
        });
        // No screen at boot isn't fatal; discovery continues in the monitor.
        if let Err(e) = this.apply(cfg) {
            eprintln!("[hdmi] not started: {e}");
        }

        let stats = Arc::clone(&this);
        thread::spawn(move || {
            let mut last_rendered: u64 = 0;
            loop {
                thread::sleep(Duration::from_secs(1));
                stats.refresh_outputs();
                // Also catches the video changing size, e.g. 16:9 to 4:3
                stats.refresh_logo();
                let Some(sink) = stats.sink() else {
                    continue;
                };
                let s = sink.property::<gst::Structure>("stats");
                let rendered = s.get::<u64>("rendered").unwrap_or(0);
                let dropped = s.get::<u64>("dropped").unwrap_or(0);
                if rendered != last_rendered && crate::settings::verbose() {
                    println!(
                        "[hdmi] {} fps (dropped={})",
                        rendered.saturating_sub(last_rendered),
                        dropped
                    );
                }
                last_rendered = rendered;
            }
        });
        Ok(this)
    }

    /// Everything but a mode change is instant; a mode change also rebuilds
    /// the pipeline, so the picture returns with the next keyframe
    pub fn apply(&self, cfg: &DisplaySettings) -> Result<(), String> {
        let mut ingress = self.ingress.lock().unwrap();
        let mut st = self.state.lock().unwrap();
        let topology = if cfg.hdmi_output {
            Some(hotplug::Topology::scan(cfg.ports)?)
        } else {
            None
        };
        let unchanged = st
            .running
            .as_ref()
            .is_some_and(|running| !running.failed.load(Ordering::Acquire))
            && st.cfg == *cfg
            && st.topology == topology
            && st.displays.iter().all(Display::route_active);
        if unchanged {
            return Ok(());
        }
        self.enabled.store(false, Ordering::SeqCst);
        ingress.reset();
        st.cfg = cfg.clone();
        st.topology = topology;
        let result = configure(&mut st);
        record_result(&mut st, &result);
        self.enabled.store(st.running.is_some(), Ordering::SeqCst);
        result
    }

    fn refresh_outputs(&self) {
        let cfg = self.state.lock().unwrap().cfg.clone();
        if !cfg.hdmi_output {
            return;
        }
        let observed = match hotplug::Topology::scan(cfg.ports) {
            Ok(topology) => topology,
            Err(error) => {
                let mut st = self.state.lock().unwrap();
                if st.last_error.as_ref() != Some(&error) {
                    eprintln!("[hdmi] {error}");
                    st.last_error = Some(error);
                }
                return;
            }
        };
        {
            let st = self.state.lock().unwrap();
            if st.cfg != cfg {
                return;
            }
            let changed = st.topology.as_ref() != Some(&observed);
            let failed = st
                .running
                .as_ref()
                .is_some_and(|running| running.failed.load(Ordering::Acquire));
            let route_lost = st.displays.iter().any(|display| !display.route_active());
            let absent = st.running.is_none() && observed.has_screen();
            let missing = observed.connected_count() > st.displays.len();
            if !changed && !failed && !route_lost && !absent && !missing {
                return;
            }
            if !changed && st.retry_after.is_some_and(|at| Instant::now() < at) {
                return;
            }
        }
        // Input producers skip HDMI while mode-setting rather than blocking
        // the shared UDP receiver, recordings, RTSP and browser outputs.
        self.enabled.store(false, Ordering::SeqCst);
        let mut ingress = self.ingress.lock().unwrap();
        let mut st = self.state.lock().unwrap();
        if st.cfg != cfg {
            self.enabled.store(st.running.is_some(), Ordering::SeqCst);
            return;
        }
        eprintln!("[hdmi] refreshing connected outputs");
        self.enabled.store(false, Ordering::SeqCst);
        ingress.reset();
        st.topology = Some(observed);
        let result = configure(&mut st);
        record_result(&mut st, &result);
        self.enabled.store(st.running.is_some(), Ordering::SeqCst);
    }

    /// The connected screen's modes, and the one in use
    pub fn modes(&self) -> Result<DisplayModes, String> {
        let st = self.state.lock().unwrap();
        if !st.displays.is_empty() {
            return Ok(describe_outputs(&st.displays, st.running.is_some()));
        }
        let ports = st.cfg.ports;
        if st.cfg.hdmi_output {
            return Err(st
                .last_error
                .clone()
                .unwrap_or_else(|| "Waiting for a screen on the selected HDMI ports".into()));
        }
        // Output is off: look without taking the screen from the console
        let displays = Display::open(ports)?;
        if displays.is_empty() {
            return Err("No screen connected to the selected HDMI ports".into());
        }
        Ok(describe_outputs(&displays, false))
    }

    pub fn refresh_logo(&self) {
        let mut st = self.state.lock().unwrap();
        let State {
            cfg,
            displays,
            running,
            ..
        } = &mut *st;
        for (index, display) in displays.iter_mut().enumerate() {
            let target = (|| {
                // Already in the picture: drawing it again would double it
                if self.overlay.burned() {
                    return None;
                }
                let (logo, overlay_cfg) = self.overlay.active()?;
                let screen = display.current()?;
                let caps = running
                    .as_ref()?
                    .sinks
                    .get(index)?
                    .static_pad("sink")?
                    .current_caps()?;
                let s = caps.structure(0)?;
                let video = (s.get::<i32>("width").ok()?, s.get::<i32>("height").ok()?);
                let screen = (screen.width as u32, screen.height as u32);
                let area = video_area(screen, (video.0 as u32, video.1 as u32), cfg.scaling);
                let mut at = overlay::place(area, (logo.width, logo.height), &overlay_cfg);
                // The video plane is turned in hardware; the logo goes with it
                if cfg.rotate_180 {
                    at.x = screen.0 as i32 - at.x - at.w as i32;
                    at.y = screen.1 as i32 - at.y - at.h as i32;
                }
                if cfg.mirror {
                    at.x = screen.0 as i32 - at.x - at.w as i32;
                }
                Some((logo, overlay_cfg.opacity_percent, at))
            })();
            match target {
                Some((logo, opacity, at)) => {
                    let rotation = plane_rotation(cfg) as u64;
                    if let Err(e) =
                        display.show_logo(&logo, self.overlay.version(), at, opacity, rotation)
                    {
                        eprintln!("[hdmi] {e}");
                    }
                }
                None => display.hide_logo(),
            }
        }
    }

    /// Pictures shown and dropped since the output started
    pub fn frames(&self) -> Option<(u64, u64)> {
        let stats = self.sink()?.property::<gst::Structure>("stats");
        Some((
            stats.get::<u64>("rendered").unwrap_or(0),
            stats.get::<u64>("dropped").unwrap_or(0),
        ))
    }

    fn sink(&self) -> Option<gst::Element> {
        let st = self.state.lock().unwrap();
        st.running.as_ref().map(|r| r.sink.clone())
    }

    /// A new input connection has a new H.264 reference history, even when its
    /// source and SPS are identical and the cable was out for less than a second.
    pub fn begin_stream(&self) {
        self.live_generation.fetch_add(1, Ordering::Release);
    }

    fn input_lock(&self) -> anyhow::Result<Option<std::sync::MutexGuard<'_, Ingress>>> {
        loop {
            if !self.enabled.load(Ordering::Relaxed) {
                return Ok(None);
            }
            match self.ingress.try_lock() {
                Ok(ingress) => return Ok(Some(ingress)),
                Err(std::sync::TryLockError::WouldBlock) => {
                    thread::sleep(Duration::from_micros(100))
                }
                Err(error) => return Err(anyhow::anyhow!("HDMI ingress: {error}")),
            }
        }
    }

    /// Raw chunks from the selected source, cut into pictures here.
    pub fn push(&self, source: crate::settings::InputMode, data: &[u8]) -> anyhow::Result<()> {
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(mut ingress) = self.input_lock()? else {
            return Ok(());
        };
        let feed = Feed::Live(source, self.live_generation.load(Ordering::Acquire));
        if !self.prepare(&mut ingress, feed, data)? {
            return Ok(());
        }
        ingress.assembler.push(data);
        while let Some(au) = ingress.assembler.next_au() {
            self.push_buffer(au, ingress.take_discont())?;
        }
        Ok(())
    }

    /// One complete picture, e.g. from the splash screen
    pub fn push_au(&self, owned: Vec<u8>) -> anyhow::Result<()> {
        if owned.len() < 10 || !self.enabled.load(Ordering::Relaxed) {
            return Ok(());
        }
        let Some(mut ingress) = self.input_lock()? else {
            return Ok(());
        };
        if !self.prepare(&mut ingress, Feed::Splash, &owned)? {
            return Ok(());
        }
        self.push_buffer(owned, ingress.take_discont())
    }

    fn prepare(&self, ingress: &mut Ingress, feed: Feed, _data: &[u8]) -> anyhow::Result<bool> {
        let boundary = ingress.select(feed);
        let mut st = self.state.lock().unwrap();
        if !self.enabled.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let failed = st
            .running
            .as_ref()
            .is_none_or(|r| r.failed.load(Ordering::Acquire));
        if failed {
            // Reopen/re-mode-set DRM in the monitor, even when no new input
            // headers arrive. Restarting a decoder on a dead route cannot help.
            return Ok(false);
        }
        if !boundary {
            return Ok(true);
        }
        // A new live/splash feed needs its own parser and reference pictures.
        ingress.discard_partial();
        eprintln!("[hdmi] restarting decoder for {feed:?}");
        if let Err(error) = restart(&mut st) {
            st.retry_after = Some(Instant::now() + Duration::from_secs(1));
            return Err(anyhow::anyhow!(error));
        }
        st.retry_after = None;
        Ok(true)
    }

    // The caller holds ingress, so apply/restart cannot replace this appsrc
    // between selecting it and pushing the buffer.
    fn push_buffer(&self, owned: Vec<u8>, discont: bool) -> anyhow::Result<()> {
        let st = self.state.lock().unwrap();
        st.running
            .as_ref()
            .map_or(Ok(()), |running| running.push_buffer(owned, discont))
    }
}

/// Closing the old decoder discards its DPB and driver queues. A flush alone
/// leaves the old parser/codec instance negotiating an unrelated H.264 stream.
fn restart(st: &mut State) -> Result<(), String> {
    replace_running(&mut st.running, || build(&st.cfg, &st.displays))
}

fn replace_running(
    running: &mut Option<Running>,
    create: impl FnOnce() -> Result<Running, String>,
) -> Result<(), String> {
    if let Some(old) = running.take() {
        old.pipeline
            .set_state(gst::State::Null)
            .map_err(|e| format!("Couldn't stop HDMI decoder: {e}"))?;
    }
    let next = create()?;
    next.pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| format!("Couldn't restart HDMI decoder: {e}"))?;
    *running = Some(next);
    Ok(())
}

impl Running {
    fn push_buffer(&self, owned: Vec<u8>, discont: bool) -> anyhow::Result<()> {
        let mut startup = self.startup.lock().unwrap();
        let primer = startup.before(&owned);
        if !startup.ready {
            // Retain headers in h264parse while waiting for a source IDR,
            // including formats that cannot use the generated primer.
            let headers = startup::parameter_sets(&owned);
            if !headers.is_empty() {
                self.push_frame(headers, discont)?;
            }
            return Ok(());
        }
        let primed = primer.is_some();
        if let Some(primer) = primer {
            self.push_frame(primer, true)?;
        }
        self.push_frame(owned, discont && !primed)
    }

    fn push_frame(&self, mut owned: Vec<u8>, discont: bool) -> anyhow::Result<()> {
        patch_sps_level(&mut owned);
        let mut buffer = gst::Buffer::from_mut_slice(owned);
        if discont {
            buffer
                .get_mut()
                .expect("new buffer")
                .set_flags(gst::BufferFlags::DISCONT);
        }
        if let Err(error) = self.appsrc.push_buffer(buffer) {
            self.failed.store(true, Ordering::Release);
            return Err(anyhow::anyhow!("push_buffer failed: {error:?}"));
        }
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn plane_rotation(cfg: &DisplaySettings) -> i32 {
    let mut rotation = if cfg.rotate_180 { ROTATE_180 } else { ROTATE_0 };
    if cfg.mirror {
        rotation |= REFLECT_X;
    }
    rotation
}

/// Where kmssink puts the picture: centred and fitted, or across the whole
/// screen when filling (cropped edges fall outside it)
fn video_area(screen: (u32, u32), video: (u32, u32), scaling: Scaling) -> Rect {
    let full = Rect {
        x: 0,
        y: 0,
        w: screen.0,
        h: screen.1,
    };
    if scaling == Scaling::Fill || video.0 == 0 || video.1 == 0 {
        return full;
    }
    // Compare shapes without floats: video wider than the screen?
    if video.0 as u64 * screen.1 as u64 >= video.1 as u64 * screen.0 as u64 {
        let h = (screen.0 as u64 * video.1 as u64 / video.0 as u64) as u32;
        Rect {
            x: 0,
            y: ((screen.1 - h.min(screen.1)) / 2) as i32,
            w: screen.0,
            h,
        }
    } else {
        let w = (screen.1 as u64 * video.0 as u64 / video.1 as u64) as u32;
        Rect {
            x: ((screen.0 - w.min(screen.0)) / 2) as i32,
            y: 0,
            w,
            h: screen.1,
        }
    }
}

fn branch(index: usize, scaling: Scaling, width: u16, height: u16, sink: &str) -> String {
    // Fill crops in the display hardware: aspectratiocrop only attaches crop
    // metadata, which kmssink turns into the plane's source rectangle
    let crop = match scaling {
        Scaling::Fit => String::new(),
        Scaling::Fill => format!("aspectratiocrop aspect-ratio={}/{} ! ", width, height),
    };
    let name = if index == 0 {
        "sink".into()
    } else {
        format!("sink{index}")
    };
    let vsync = if sink == "kmssink" {
        "skip-vsync=true"
    } else {
        ""
    };
    format!(
        " video. ! queue max-size-buffers=1 max-size-bytes=0 max-size-time=0 leaky=downstream ! \
        identity name=mirror{index} ! {crop}{sink} name={name} sync=false async=false {vsync} "
    )
}

fn build(cfg: &DisplaySettings, displays: &[Display]) -> Result<Running, String> {
    if displays.is_empty() {
        return Err("HDMI display is unavailable".into());
    }
    let mut description = PIPELINE.to_string();
    let render = Arc::new(Mutex::new(()));
    for (index, display) in displays.iter().enumerate() {
        let mode = display.current().ok_or("The screen has no active mode")?;
        description.push_str(&branch(
            index,
            cfg.scaling,
            mode.width,
            mode.height,
            "kmssink",
        ));
    }
    let pipeline = gst::parse::launch(&description)
        .map_err(|e| format!("HDMI pipeline: {e}"))?
        .dynamic_cast::<gst::Pipeline>()
        .expect("Failed to cast to Pipeline");

    for (index, display) in displays.iter().enumerate() {
        let name = if index == 0 {
            "sink".into()
        } else {
            format!("sink{index}")
        };
        let sink = pipeline.by_name(&name).expect("Failed to find kmssink");
        sink.set_property("fd", display.fd());
        sink.set_property("connector-id", display.connector_id());
        sink.set_property("plane-id", display.plane_id());
        // Always set, or the plane keeps the last pipeline's rotation
        sink.set_property(
            "plane-properties",
            gst::Structure::builder("s")
                .field("rotation", plane_rotation(cfg))
                .build(),
        );
        if displays.len() > 1 {
            mirror::isolate(&pipeline, index, Arc::clone(&render))?;
        }
    }

    Ok(finish_build(pipeline))
}

fn finish_build(pipeline: gst::Pipeline) -> Running {
    let appsrc = pipeline
        .by_name("src")
        .expect("Failed to find appsrc")
        .dynamic_cast::<AppSrc>()
        .expect("Failed to cast to AppSrc");
    // Whole pictures: with a bare byte stream h264parse only knows a
    // picture is complete once the next one starts, a frame of latency
    let caps = gst::Caps::builder("video/x-h264")
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
    appsrc.set_caps(Some(&caps));
    appsrc.set_stream_type(gstreamer_app::AppStreamType::Stream);
    appsrc.set_max_bytes(4_000_000);
    appsrc.set_block(false);
    // Stamp new buffers against this pipeline's running-time clock. The input
    // H.264 chunks carry no GStreamer timestamps, including after reconnect.
    appsrc.set_property("do-timestamp", true);
    appsrc.set_latency(gst::ClockTime::ZERO, gst::ClockTime::NONE);

    let sink = pipeline.by_name("sink").expect("Failed to find HDMI sink");
    let mut sinks = vec![sink.clone()];
    for index in 1.. {
        let Some(sink) = pipeline.by_name(&format!("sink{index}")) else {
            break;
        };
        sinks.push(sink);
    }

    let failed = Arc::new(AtomicBool::new(false));
    if let Some(bus) = pipeline.bus() {
        let failed = Arc::clone(&failed);
        bus.set_sync_handler(move |_, msg| {
            use gst::MessageView;
            match msg.view() {
                MessageView::Error(err) => {
                    failed.store(true, Ordering::Release);
                    eprintln!("[hdmi] error: {} ({:?})", err.error(), err.debug());
                }
                MessageView::Warning(w) => {
                    eprintln!("[hdmi] warning: {} ({:?})", w.error(), w.debug());
                }
                _ => (),
            }
            gst::BusSyncReply::Drop
        });
    }

    Running {
        pipeline,
        appsrc,
        sink,
        sinks,
        failed,
        startup: Mutex::new(startup::DecoderStartup::default()),
    }
}

fn describe_outputs(displays: &[Display], active: bool) -> DisplayModes {
    let outputs: Vec<_> = displays
        .iter()
        .map(|display| OutputInfo {
            port: display.port().to_string(),
            modes: display.modes(),
            current: active.then(|| display.current().cloned()).flatten(),
        })
        .collect();
    let mut modes: Vec<ModeInfo> = Vec::new();
    for output in &outputs {
        for mode in &output.modes {
            if !modes.iter().any(|listed| listed.id == mode.id) {
                modes.push(mode.clone());
            }
        }
    }
    let current = outputs.first().and_then(|output| output.current.clone());
    (modes, current, outputs)
}

/// Patch SPS level for the Raspberry Pi 4's H.264 decoder, which is limited to level 4.2
fn patch_sps_level(data: &mut [u8]) {
    let Some(sps) = h264::nals(data).find(|nal| nal.kind == h264::SPS) else {
        return;
    };
    // Raising this to e.g. 5.2 to lift 4.2's ~16fps/4K macroblock-rate cap was
    // tried and made the decoder fail outright - no picture at all
    if let Some(level) = data.get_mut(sps.header + 3) {
        *level = 42; // H.264 level 4.2
    }
}

impl Drop for GstPlayer {
    fn drop(&mut self) {
        if let Some(r) = self.state.lock().unwrap().running.take() {
            let _ = r.pipeline.set_state(gst::State::Null);
        }
    }
}

#[cfg(test)]
#[path = "hdmi/test.rs"]
mod tests;
