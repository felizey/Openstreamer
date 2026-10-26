//! Uploaded clips are converted once. Playback only demuxes the saved H.264;
//! the Pi's existing output decoder handles it just like the goggles stream.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::OwnedSemaphorePermit;

pub const MAX_UPLOAD_BYTES: usize = 256 << 20;
const MAX_DURATION_SECONDS: f64 = 300.0;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_COMMAND_OUTPUT: u64 = 64 << 10;
pub const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);
// These demuxers read a single local video file. In particular, do not allow
// playlists to follow paths or URLs supplied inside an uploaded document.
const DEMUXERS: &str =
    "mov,matroska,webm,avi,mpeg,mpegts,ogg,flv,gif,asf,nut,mjpeg,h264,hevc,mxf,dv,ivf";

#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<Stream>,
    format: Option<Format>,
}

#[derive(Deserialize)]
struct Stream {
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
    profile: Option<String>,
    has_b_frames: Option<u32>,
    r_frame_rate: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    format_name: Option<String>,
    duration: Option<String>,
    tags: Option<FormatTags>,
}

#[derive(Deserialize)]
struct FormatTags {
    major_brand: Option<String>,
}

impl Probe {
    fn validate_source(&self, is_mp4: bool) -> anyhow::Result<()> {
        let stream = self
            .streams
            .first()
            .context("The file has no video track")?;
        let codec = stream
            .codec_name
            .as_deref()
            .context("Unknown video codec")?;
        self.format
            .as_ref()
            .and_then(|f| f.format_name.as_deref())
            .context("Unknown video container")?;
        // The ftyp brand distinguishes MP4 from QuickTime MOV: ffprobe uses
        // exactly the same comma-separated format_name for both containers.
        let is_mp4 = is_mp4
            || self
                .format
                .as_ref()
                .and_then(|format| format.tags.as_ref())
                .and_then(|tags| tags.major_brand.as_deref())
                .is_some_and(|brand| iso_mp4_brand(brand.as_bytes()));
        if is_mp4 && codec != "h264" {
            anyhow::bail!("An MP4 source must contain H.264 video (this file uses {codec})");
        }
        let (width, height) = (stream.width.unwrap_or(0), stream.height.unwrap_or(0));
        if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 7680 * 4320 {
            anyhow::bail!("Video dimensions must be valid and no larger than 8K");
        }
        let duration = stream
            .duration
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| {
                self.format
                    .as_ref()?
                    .duration
                    .as_deref()?
                    .parse::<f64>()
                    .ok()
            })
            .context("The video must have a known duration")?;
        if !duration.is_finite() || duration <= 0.0 || duration > MAX_DURATION_SECONDS {
            anyhow::bail!("The video must be between 0 and 300 seconds long");
        }
        Ok(())
    }

    fn validate_output(&self) -> anyhow::Result<()> {
        self.validate_source(true)?;
        let stream = &self.streams[0];
        if stream.codec_name.as_deref() != Some("h264")
            || stream.profile.as_deref() != Some("High")
            || stream.width != Some(1920)
            || stream.height != Some(1080)
            || stream.has_b_frames != Some(0)
            || stream.r_frame_rate.as_deref() != Some("30/1")
        {
            anyhow::bail!("The converted video did not meet the splash playback format");
        }
        Ok(())
    }
}

/// Both files are installed before this tiny manifest is atomically replaced.
/// A failed conversion therefore never replaces a working splash.
#[derive(Deserialize, Serialize)]
struct Saved {
    video: String,
    preview: String,
}

impl Saved {
    fn validate(&self) -> anyhow::Result<()> {
        for (name, suffix) in [(&self.video, ".mp4"), (&self.preview, ".png")] {
            let id = name
                .strip_prefix("splash-video-")
                .and_then(|s| s.strip_suffix(suffix))
                .context("Invalid saved splash filename")?;
            if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
                anyhow::bail!("Invalid saved splash filename");
            }
        }
        Ok(())
    }
}

pub struct Video {
    pub path: PathBuf,
    pub preview: PathBuf,
}

impl Video {
    pub fn load(parent: &Path) -> anyhow::Result<Option<Arc<Self>>> {
        let manifest = match std::fs::read(parent.join("splash-video.json")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let saved: Saved = serde_json::from_slice(&manifest)?;
        saved.validate()?;
        let video = Self {
            path: parent.join(saved.video),
            preview: parent.join(saved.preview),
        };
        if !video.path.is_file() || !video.preview.is_file() {
            anyhow::bail!("Saved splash video files are missing");
        }
        Ok(Some(Arc::new(video)))
    }

    pub fn remove_manifest(parent: &Path) -> std::io::Result<()> {
        match std::fs::remove_file(parent.join("splash-video.json")) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }

    pub fn remove_files(&self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(&self.preview);
    }
}

/// Owns the upload, conversion files and the single media-mutation permit.
/// Cancellation or any error drops all temporary files and releases the slot.
pub struct Upload {
    directory: PathBuf,
    id: String,
    _permit: OwnedSemaphorePermit,
}

impl Upload {
    pub fn new(parent: &Path, permit: OwnedSemaphorePermit) -> anyhow::Result<Self> {
        std::fs::create_dir_all(parent)?;
        let id = format!("{:016x}", rand::random::<u64>());
        let directory = parent.join(format!(".splash-video-upload-{id}"));
        std::fs::create_dir(&directory)?;
        Ok(Self {
            directory,
            id,
            _permit: permit,
        })
    }

    pub fn source(&self) -> PathBuf {
        self.directory.join("source")
    }

    pub async fn convert(&self) -> anyhow::Result<()> {
        let mut header = [0u8; 4096];
        let mut source = tokio::fs::File::open(self.source()).await?;
        let count = source.read(&mut header).await?;
        probe(&self.source())
            .await?
            .validate_source(mp4_brand(&header[..count]))?;
        let output = self.directory.join("converted.mp4");
        let mut command = limited_command("ffmpeg");
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
        input_options(&mut command);
        command.arg("-i").arg(self.source());
        command.args(encode_options());
        command.arg(&output);
        run(command, COMMAND_TIMEOUT)
            .await
            .context("Video conversion failed")?;
        probe(&output).await?.validate_output()?;

        let mut command = limited_command("ffmpeg");
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"]);
        input_options(&mut command);
        command.arg("-i").arg(&output);
        command.args(["-map", "0:v:0", "-frames:v", "1", "-threads", "1"]);
        command.arg(self.directory.join("preview.png"));
        run(command, PROBE_TIMEOUT)
            .await
            .context("Couldn't create the video preview")?;
        Ok(())
    }

    pub fn install(&self, parent: &Path) -> anyhow::Result<Arc<Video>> {
        let saved = Saved {
            video: format!("splash-video-{}.mp4", self.id),
            preview: format!("splash-video-{}.png", self.id),
        };
        let video = Arc::new(Video {
            path: parent.join(&saved.video),
            preview: parent.join(&saved.preview),
        });
        let result = (|| {
            std::fs::rename(self.directory.join("converted.mp4"), &video.path)?;
            std::fs::rename(self.directory.join("preview.png"), &video.preview)?;
            let manifest = self.directory.join("manifest.json");
            std::fs::write(&manifest, serde_json::to_vec(&saved)?)?;
            std::fs::rename(manifest, parent.join("splash-video.json"))?;
            Ok::<_, anyhow::Error>(())
        })();
        if let Err(error) = result {
            video.remove_files();
            return Err(error);
        }
        Ok(video)
    }
}

fn mp4_brand(mut bytes: &[u8]) -> bool {
    while bytes.len() >= 16 {
        let size = u32::from_be_bytes(bytes[..4].try_into().expect("four bytes")) as usize;
        if &bytes[4..8] == b"ftyp" {
            let brand = &bytes[8..12];
            return iso_mp4_brand(brand);
        }
        if size < 8 || size > bytes.len() {
            break;
        }
        bytes = &bytes[size..];
    }
    false
}

fn iso_mp4_brand(brand: &[u8]) -> bool {
    brand.len() == 4 && brand != b"qt  " && !brand.starts_with(b"3g")
}

impl Drop for Upload {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

fn encode_options() -> &'static [&'static str] {
    &[
        "-map",
        "0:v:0",
        "-an",
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
        "-vf",
        "scale=1920:1080:force_original_aspect_ratio=decrease:force_divisible_by=2,pad=1920:1080:(ow-iw)/2:(oh-ih)/2,setsar=1,fps=30,format=yuv420p",
        "-c:v",
        "libx264",
        "-preset",
        "veryfast",
        "-profile:v",
        "high",
        "-level:v",
        "4.1",
        "-b:v",
        "4M",
        "-maxrate",
        "4M",
        "-bufsize",
        "8M",
        "-bf",
        "0",
        "-g",
        "30",
        "-keyint_min",
        "30",
        "-sc_threshold",
        "0",
        "-x264-params",
        "aud=1:repeat-headers=1:open-gop=0",
        "-threads",
        "2",
        "-t",
        "300",
        "-movflags",
        "+faststart",
        "-f",
        "mp4",
    ]
}

fn input_options(command: &mut Command) {
    command.args([
        "-protocol_whitelist",
        "file,pipe",
        "-format_whitelist",
        DEMUXERS,
        "-threads",
        "2",
        "-filter_threads",
        "1",
        "-filter_complex_threads",
        "1",
    ]);
}

async fn probe(path: &Path) -> anyhow::Result<Probe> {
    let mut command = limited_command("ffprobe");
    // ffprobe doesn't accept ffmpeg's filter thread options.
    command.args([
        "-v", "error", "-protocol_whitelist", "file,pipe", "-format_whitelist", DEMUXERS,
        "-threads", "2", "-select_streams", "v:0", "-show_entries",
        "stream=codec_name,width,height,duration,profile,has_b_frames,r_frame_rate:format=format_name,duration:format_tags=major_brand",
        "-of", "json",
    ]);
    command.arg(path);
    let output = run(command, PROBE_TIMEOUT)
        .await
        .context("Couldn't read the video")?;
    Ok(serde_json::from_slice(&output)?)
}

fn limited_command(program: &str) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Apply limits only to the converter, never to the main streaming process.
    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            for (resource, limit) in [
                (libc::RLIMIT_AS, 1536_u64 << 20),
                (libc::RLIMIT_FSIZE, 512_u64 << 20),
                (libc::RLIMIT_CPU, 30 * 60),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: limit,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            libc::nice(10);
            Ok(())
        });
    }
    command
}

async fn run(mut command: Command, timeout: Duration) -> anyhow::Result<Vec<u8>> {
    let mut child = command
        .spawn()
        .context("ffmpeg/ffprobe is not installed or could not start")?;
    let stdout = child.stdout.take().context("No converter stdout")?;
    let stderr = child.stderr.take().context("No converter stderr")?;
    let result = tokio::time::timeout(timeout, async {
        let read = |pipe: Box<dyn tokio::io::AsyncRead + Unpin + Send>| async move {
            let mut bytes = Vec::new();
            pipe.take(MAX_COMMAND_OUTPUT + 1)
                .read_to_end(&mut bytes)
                .await?;
            if bytes.len() as u64 > MAX_COMMAND_OUTPUT {
                anyhow::bail!("Converter output exceeded its limit");
            }
            Ok::<_, anyhow::Error>(bytes)
        };
        let (stdout, stderr, status) =
            tokio::try_join!(read(Box::new(stdout)), read(Box::new(stderr)), async {
                Ok::<_, anyhow::Error>(child.wait().await?)
            },)?;
        if !status.success() {
            anyhow::bail!("{}", String::from_utf8_lossy(&stderr).trim());
        }
        Ok(stdout)
    })
    .await;
    match result {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => {
            let _ = child.kill().await;
            Err(error)
        }
        Err(_) => {
            let _ = child.kill().await;
            anyhow::bail!("Video processing timed out");
        }
    }
}

pub struct Player {
    pub video: Arc<Video>,
    pipeline: gst::Pipeline,
    sink: AppSink,
    next_frame_at: std::time::Instant,
    last_sample_at: std::time::Instant,
}

impl Player {
    pub fn new(video: Arc<Video>) -> anyhow::Result<Self> {
        gst::init()?;
        let pipeline = gst::parse::launch(
            "filesrc name=src ! qtdemux ! h264parse config-interval=-1 ! \
             video/x-h264,stream-format=byte-stream,alignment=au ! \
             appsink name=sink sync=false max-buffers=2 drop=false",
        )?
        .dynamic_cast::<gst::Pipeline>()
        .map_err(|_| anyhow::anyhow!("Not a video pipeline"))?;
        pipeline
            .by_name("src")
            .context("No splash video source")?
            .set_property(
                "location",
                video.path.to_str().context("Invalid video path")?,
            );
        let sink = pipeline
            .by_name("sink")
            .context("No splash video sink")?
            .dynamic_cast::<AppSink>()
            .map_err(|_| anyhow::anyhow!("Not an appsink"))?;
        pipeline.set_state(gst::State::Playing)?;
        Ok(Self {
            video,
            pipeline,
            sink,
            next_frame_at: std::time::Instant::now(),
            last_sample_at: std::time::Instant::now(),
        })
    }

    pub fn next_frame(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let now = std::time::Instant::now();
        if now < self.next_frame_at {
            return Ok(None);
        }
        if let Some(bus) = self.pipeline.bus()
            && let Some(message) = bus.pop_filtered(&[gst::MessageType::Error])
            && let gst::MessageView::Error(error) = message.view()
        {
            anyhow::bail!("{}", error.error());
        }
        if let Some(sample) = self.sink.try_pull_sample(gst::ClockTime::ZERO) {
            let buffer = sample.buffer().context("Video frame has no buffer")?;
            let map = buffer.map_readable()?;
            // Keep a 30 Hz schedule without accumulating the polling delay.
            // A slow status redraw must not release a burst of overdue frames.
            let scheduled = self.next_frame_at + FRAME_INTERVAL;
            self.next_frame_at = if scheduled <= now {
                now + FRAME_INTERVAL
            } else {
                scheduled
            };
            self.last_sample_at = now;
            return Ok(Some(map.to_vec()));
        }
        if self.sink.is_eos() {
            self.pipeline.seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT,
                gst::ClockTime::ZERO,
            )?;
        } else if self.last_sample_at.elapsed() > Duration::from_secs(10) {
            anyhow::bail!("Splash video playback timed out");
        }
        Ok(None)
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
#[path = "video.test.rs"]
mod tests;
