//! Video through the FFmpeg command-line tools: metadata from `ffprobe`,
//! still frames for thumbnails, and streamed decoding for playback. Frames
//! and audio arrive as raw pixels and samples on pipes, so no FFmpeg
//! libraries are linked into Picasso.

use std::io::Read;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use image::RgbaImage;
use serde_json::Value;

use crate::i18n::{tr, trf};
use crate::image_io::Probe;
use crate::types::format_duration;

/// Longest edge of frames decoded for playback.
pub const PLAY_EDGE: u32 = 1920;
const AUDIO_RATE: u32 = 48_000;
const AUDIO_CHANNELS: u16 = 2;
/// Audio samples per pipe read: 2048 stereo frames, about 43 ms.
const AUDIO_CHUNK: usize = 4096;
/// How long playback waits for audio before starting without it.
const AUDIO_WAIT: Duration = Duration::from_millis(1500);

struct Tools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

fn tools() -> Option<&'static Tools> {
    static TOOLS: OnceLock<Option<Tools>> = OnceLock::new();
    TOOLS.get_or_init(find_tools).as_ref()
}

/// Whether `ffmpeg` and `ffprobe` were found.
pub fn available() -> bool {
    tools().is_some()
}

/// Whether [`web_video`] can make videos: FFmpeg with an H.264 encoder.
pub fn makes_web_video() -> bool {
    available() && h264_encoder().is_some()
}

fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

/// Looks in `PICASSO_FFMPEG`, beside the Picasso executable, on `PATH`, and
/// where winget and Homebrew install FFmpeg. Apps started from the desktop
/// on macOS do not see the shell `PATH`, and a winget install is not on the
/// `PATH` of programs that were already running.
fn find_tools() -> Option<Tools> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(custom) = std::env::var_os("PICASSO_FFMPEG") {
        let custom = PathBuf::from(custom);
        if custom.is_file() {
            dirs.extend(custom.parent().map(Path::to_path_buf));
        } else {
            dirs.push(custom);
        }
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        dirs.push(dir.join("ffmpeg"));
        dirs.push(dir);
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let winget = PathBuf::from(local).join("Microsoft").join("WinGet");
            dirs.push(winget.join("Links"));
            dirs.extend(winget_package_bins(&winget.join("Packages")));
        }
    } else {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
    }
    dirs.into_iter().find_map(|dir| {
        let ffmpeg = dir.join(exe("ffmpeg"));
        let ffprobe = dir.join(exe("ffprobe"));
        (ffmpeg.is_file() && ffprobe.is_file()).then(|| Tools {
            ffmpeg: ffmpeg.canonicalize().unwrap_or(ffmpeg),
            ffprobe: ffprobe.canonicalize().unwrap_or(ffprobe),
        })
    })
}

/// `bin` folders of FFmpeg packages installed by winget.
fn winget_package_bins(packages: &Path) -> Vec<PathBuf> {
    let mut bins = Vec::new();
    let Ok(entries) = std::fs::read_dir(packages) else {
        return bins;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().to_lowercase().contains("ffmpeg") {
            continue;
        }
        if let Ok(builds) = std::fs::read_dir(entry.path()) {
            bins.extend(builds.flatten().map(|build| build.path().join("bin")));
        }
    }
    bins
}

fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[derive(Clone, Debug, Default)]
pub struct VideoInfo {
    /// Frame size as displayed, after any rotation stored in the file.
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub fps: f64,
    pub has_audio: bool,
    pub taken_at: Option<i64>,
    pub camera: String,
    pub gps: Option<(f64, f64)>,
    pub video_codec: String,
    pub audio: String,
    pub bit_rate: Option<u64>,
}

pub fn info(path: &Path) -> Result<VideoInfo, String> {
    let tools = tools().ok_or(tr("Install FFmpeg to read and play videos"))?;
    let output = command(&tools.ffprobe)
        .args(["-v", "error", "-print_format", "json", "-show_format", "-show_streams"])
        .arg(path)
        .output()
        .map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(tr("FFmpeg could not read this video").into());
    }
    let json: Value = serde_json::from_slice(&output.stdout).map_err(|err| err.to_string())?;
    parse_info(&json)
}

fn parse_info(json: &Value) -> Result<VideoInfo, String> {
    let streams = json["streams"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video" && stream["disposition"]["attached_pic"].as_i64() != Some(1))
        .ok_or(tr("The file has no video track"))?;
    let audio = streams.iter().find(|stream| stream["codec_type"] == "audio");
    let format = &json["format"];

    let mut width = video["width"].as_u64().unwrap_or(0) as u32;
    let mut height = video["height"].as_u64().unwrap_or(0) as u32;
    if rotation(video).rem_euclid(180) == 90 {
        std::mem::swap(&mut width, &mut height);
    }
    let fps = [&video["avg_frame_rate"], &video["r_frame_rate"]]
        .into_iter()
        .find_map(|rate| frame_rate(rate.as_str()?))
        .unwrap_or(30.0);
    let duration = number(&format["duration"]).or_else(|| number(&video["duration"])).unwrap_or(0.0);
    let tag = |keys: &[&str]| tag(format, video, keys);

    let taken_at = tag(&["com.apple.quicktime.creationdate"])
        .and_then(parse_time)
        .or_else(|| tag(&["creation_time", "date"]).and_then(parse_time));
    let make = tag(&["com.apple.quicktime.make", "make", "com.android.manufacturer"]).unwrap_or("");
    let model = tag(&["com.apple.quicktime.model", "model", "com.android.model"]).unwrap_or("");
    let gps = tag(&["location", "com.apple.quicktime.location.iso6709", "location-eng"]).and_then(parse_iso6709);

    let audio_text = audio.map(|audio| {
        let mut parts = vec![audio["codec_name"].as_str().unwrap_or("audio").to_string()];
        if let Some(rate) = number(&audio["sample_rate"]) {
            parts.push(format!("{:.1} kHz", rate / 1000.0).replace(".0 ", " "));
        }
        match audio["channel_layout"].as_str() {
            Some(layout) => parts.push(layout.to_string()),
            None => parts.extend(audio["channels"].as_u64().map(|n| format!("{n} ch"))),
        }
        parts.join(", ")
    });

    Ok(VideoInfo {
        width,
        height,
        duration,
        fps: fps.clamp(1.0, 120.0),
        has_audio: audio.is_some(),
        taken_at,
        camera: camera_label(make, model),
        gps,
        video_codec: video["codec_name"].as_str().unwrap_or("video").to_string(),
        audio: audio_text.unwrap_or_default(),
        bit_rate: number(&format["bit_rate"]).map(|bits| bits as u64),
    })
}

fn rotation(stream: &Value) -> i64 {
    let side = stream["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|item| number(&item["rotation"]));
    side.or_else(|| number(&stream["tags"]["rotate"])).unwrap_or(0.0).round() as i64
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().or_else(|| value.as_str()?.trim().parse().ok())
}

fn frame_rate(text: &str) -> Option<f64> {
    let (num, den) = text.split_once('/').unwrap_or((text, "1"));
    let rate = num.trim().parse::<f64>().ok()? / den.trim().parse::<f64>().ok()?;
    (rate.is_finite() && rate > 0.0 && rate < 1000.0).then_some(rate)
}

/// First non-empty tag among `keys`, from the container and then the video track.
fn tag<'a>(format: &'a Value, video: &'a Value, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        for tags in [&format["tags"], &video["tags"]] {
            let found = tags
                .as_object()
                .and_then(|map| map.iter().find(|(name, _)| name.eq_ignore_ascii_case(key)))
                .and_then(|(_, value)| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if found.is_some() {
                return found;
            }
        }
    }
    None
}

fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let stamp = chrono::DateTime::parse_from_rfc3339(text)
        .or_else(|_| chrono::DateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S%z"))
        .map(|time| time.timestamp())
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|time| time.and_utc().timestamp())
        })?;
    // Cameras without a clock write 1904 or 1970.
    (stamp > 315_532_800).then_some(stamp)
}

/// Reads the latitude and longitude of an ISO 6709 string such as
/// `+25.0330+121.5654+012.000/`.
fn parse_iso6709(text: &str) -> Option<(f64, f64)> {
    let mut numbers = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if matches!(ch, '+' | '-' | '/') {
            if !current.is_empty() {
                numbers.push(current.parse::<f64>().ok()?);
            }
            current.clear();
            if ch == '/' {
                break;
            }
        }
        current.push(ch);
    }
    if !current.is_empty() {
        numbers.push(current.parse::<f64>().ok()?);
    }
    let (lat, lon) = (*numbers.first()?, *numbers.get(1)?);
    let valid = (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) && (lat, lon) != (0.0, 0.0);
    valid.then_some((lat, lon))
}

fn camera_label(make: &str, model: &str) -> String {
    crate::image_io::camera_name(make, model)
}

/// Labeled details for the info panels, in display order.
pub fn rows(info: &VideoInfo) -> Vec<(&'static str, String)> {
    let mut rows = vec![("Duration", format_duration(info.duration))];
    let fps = if (info.fps - info.fps.round()).abs() < 0.01 {
        format!("{:.0}", info.fps)
    } else {
        format!("{:.2}", info.fps)
    };
    rows.push((
        "Video",
        format!("{}, {} × {}, {fps} fps", info.video_codec, info.width, info.height),
    ));
    rows.push(("Audio", if info.audio.is_empty() { tr("None").into() } else { info.audio.clone() }));
    if let Some(bits) = info.bit_rate {
        rows.push(("Bit rate", format!("{:.1} Mbps", bits as f64 / 1_000_000.0)));
    }
    if !info.camera.is_empty() {
        rows.push(("Camera", info.camera.clone()));
    }
    rows
}

/// Catalog record for a video. Without FFmpeg the file is still listed, and
/// marked incomplete so a later scan reads it again.
pub fn probe_file(path: &Path, modified_at: i64) -> Probe {
    match info(path) {
        Ok(info) => Probe {
            width: info.width,
            height: info.height,
            taken_at: info.taken_at.unwrap_or(modified_at),
            camera: info.camera,
            lens: String::new(),
            focal_mm: 0.0,
            gps_lat: info.gps.map(|(lat, _)| lat),
            gps_lon: info.gps.map(|(_, lon)| lon),
            zone: None,
            duration: info.duration,
            complete: true,
        },
        Err(_) => Probe {
            width: 0,
            height: 0,
            taken_at: modified_at,
            camera: String::new(),
            lens: String::new(),
            focal_mm: 0.0,
            gps_lat: None,
            gps_lon: None,
            zone: None,
            duration: 0.0,
            complete: available(),
        },
    }
}

/// Frame size that fits within `edge`, never enlarged, in even pixels as
/// most video filters expect.
fn fit(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let (width, height) = if width == 0 || height == 0 { (1280, 720) } else { (width, height) };
    let scale = (edge as f64 / width.max(height) as f64).min(1.0);
    let even = |n: u32| ((n as f64 * scale).round() as u32 / 2 * 2).max(2);
    (even(width), even(height))
}

/// A poster frame a little way into the video, scaled to fit `edge`.
pub fn poster(path: &Path, edge: u32) -> Result<(RgbaImage, VideoInfo), String> {
    let info = info(path)?;
    let (width, height) = fit(info.width, info.height, edge);
    let at = (info.duration * 0.1).min(2.0);
    let image = grab(path, at, width, height).or_else(|_| grab(path, 0.0, width, height))?;
    Ok((image, info))
}

fn grab(path: &Path, at: f64, width: u32, height: u32) -> Result<RgbaImage, String> {
    let tools = tools().ok_or(tr("FFmpeg is not installed"))?;
    let output = command(&tools.ffmpeg)
        .args(["-nostdin", "-v", "error", "-ss", &format!("{at:.3}"), "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1", "-an", "-sn", "-dn"])
        .args(["-vf", &format!("scale={width}:{height}"), "-pix_fmt", "rgba", "-f", "rawvideo", "-"])
        .stdout(Stdio::piped())
        .output()
        .map_err(|err| err.to_string())?;
    let len = width as usize * height as usize * 4;
    if output.stdout.len() < len {
        return Err(tr("No frame could be decoded").into());
    }
    RgbaImage::from_raw(width, height, output.stdout[..len].to_vec()).ok_or_else(|| "Bad frame".into())
}

/// The frame `at` seconds into the video, scaled to fit `edge`.
pub fn frame_at(path: &Path, at: f64, edge: u32) -> Result<RgbaImage, String> {
    let info = info(path)?;
    let (width, height) = fit(info.width, info.height, edge);
    grab(path, at.max(0.0), width, height).or_else(|_| grab(path, 0.0, width, height))
}

/// Raised when [`web_video`] would make a different file of the same part
/// of a video, so files made before aren't taken for the new ones.
pub const WEB_VIDEO_VERSION: u32 = 2;

/// Frame size of a video for the web: within 1080p either way up, never
/// enlarged, in even pixels.
pub fn web_size(width: u32, height: u32) -> (u32, u32) {
    let (width, height) = if width == 0 || height == 0 { (1280, 720) } else { (width, height) };
    let scale = (1920.0 / f64::from(width.max(height))).min(1080.0 / f64::from(width.min(height))).min(1.0);
    let even = |n: u32| ((f64::from(n) * scale).round() as u32 / 2 * 2).max(2);
    (even(width), even(height))
}

/// H.264 encoders, best first: x264 where FFmpeg has it, then the
/// system's own, then Cisco's.
const H264_ENCODERS: [&str; 4] = ["libx264", "h264_videotoolbox", "h264_mf", "libopenh264"];

/// The best H.264 encoder this FFmpeg has.
fn h264_encoder() -> Option<&'static str> {
    static FOUND: OnceLock<Option<&'static str>> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let output = command(&tools()?.ffmpeg).args(["-hide_banner", "-encoders"]).stdout(Stdio::piped()).output().ok()?;
        let listed = String::from_utf8_lossy(&output.stdout);
        H264_ENCODERS.into_iter().find(|name| listed.split_whitespace().any(|word| word == *name))
    })
}

/// How `encoder` is asked for a frame `width` × `height` at `fps`: x264 by
/// quality, the others by a bit rate that suits the size.
fn encoder_args(encoder: &'static str, width: u32, height: u32, fps: f64) -> Vec<String> {
    let mut args: Vec<String> = ["-c:v", encoder].map(String::from).to_vec();
    if encoder == "libx264" {
        args.extend(["-preset", "medium", "-crf", "23", "-profile:v", "high"].map(String::from));
    } else {
        let bits = f64::from(width) * f64::from(height) * fps.min(60.0) * 0.1;
        args.extend(["-b:v".to_string(), format!("{}", bits.round() as u64)]);
        if encoder == "h264_videotoolbox" {
            args.extend(["-allow_sw", "1", "-profile:v", "high"].map(String::from));
        }
    }
    args
}

/// Writes the part from `start` to `end` seconds of the video at `path` as
/// an MP4 at `out` that plays in every browser: H.264 within 1080p and 60
/// frames a second, AAC stereo sound, its index first so it plays while it
/// loads, and none of the original's metadata, such as where it was taken.
/// An `end` no later than `start` is the video's end. `done` hears how far
/// it is, from 0 to 1. Setting `cancel` stops it; nothing is left at `out`
/// unless it was made whole.
pub fn web_video(path: &Path, out: &Path, start: f64, end: f64, cancel: &AtomicBool, done: &dyn Fn(f64)) -> Result<(), String> {
    let tools = tools().ok_or(tr("Install FFmpeg to read and play videos"))?;
    let encoder = h264_encoder().ok_or(tr("This FFmpeg can't make H.264 video"))?;
    let source = info(path)?;
    let (width, height) = web_size(source.width, source.height);
    let until = (end > start).then(|| format!("{:.3}", end - start));
    let length = if end > start { end - start } else { (source.duration - start).max(0.1) };
    let mut filters = format!("scale={width}:{height},format=yuv420p");
    if source.fps > 60.5 {
        filters.push_str(",fps=60");
    }
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let partial = out.with_extension(format!("partial-{}-{}", std::process::id(), RUN.fetch_add(1, Ordering::Relaxed)));
    let mut child = command(&tools.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y", "-ss", &format!("{start:.3}"), "-i"])
        .arg(path)
        .args(until.iter().flat_map(|until| ["-t", until.as_str()]))
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn", "-map_metadata", "-1", "-map_chapters", "-1"])
        .args(["-vf", &filters])
        .args(encoder_args(encoder, width, height, source.fps))
        .args(["-c:a", "aac", "-b:a", "128k", "-ac", "2", "-movflags", "+faststart", "-progress", "pipe:1", "-nostats", "-f", "mp4"])
        .arg(&partial)
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|err| trf("Can't start FFmpeg: {}", &[&err]))?;
    let reached = Arc::new(std::sync::atomic::AtomicU64::new(0));
    if let Some(stdout) = child.stdout.take() {
        let reached = Arc::clone(&reached);
        let _ = std::thread::Builder::new().name("picasso-video-progress".into()).spawn(move || {
            use std::io::BufRead;
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(micros) = line.strip_prefix("out_time_us=").and_then(|micros| micros.trim().parse::<u64>().ok()) {
                    reached.store(micros, Ordering::Relaxed);
                }
            }
        });
    }
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&partial);
            return Err("cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&partial);
                return Err(err.to_string());
            }
        }
        done((reached.load(Ordering::Relaxed) as f64 / 1e6 / length).clamp(0.0, 1.0));
    };
    // FFmpeg can successfully write an audio-only or empty container when
    // the selected range has no video frames.
    if !status.success() || info(&partial).is_err() {
        let _ = std::fs::remove_file(&partial);
        return Err(tr("FFmpeg couldn't convert this video").into());
    }
    std::fs::rename(&partial, out).map_err(|err| {
        let _ = std::fs::remove_file(&partial);
        err.to_string()
    })?;
    done(1.0);
    Ok(())
}

/// Writes a 320×240 test picture with a tone, `seconds` long, at `path`;
/// false without FFmpeg.
#[cfg(test)]
pub fn test_clip(path: &Path, seconds: u32) -> bool {
    let Some(tools) = tools() else { return false };
    command(&tools.ffmpeg)
        .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i", &format!("testsrc=duration={seconds}:size=320x240:rate=25")])
        .args(["-f", "lavfi", "-i", &format!("sine=frequency=440:duration={seconds}")])
        .args(["-c:v", "mpeg4", "-c:a", "aac", "-shortest", "-metadata", "location=+25.0330+121.5654/", "-metadata", "creation_time=2024-05-01T10:00:00Z"])
        .arg(path)
        .status()
        .is_ok_and(|status| status.success())
}

/// A decoded frame ready to upload.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Plays one video: reads its details in the background, then streams
/// frames and sound from FFmpeg, restarting the stream on every seek.
pub struct Player {
    path: PathBuf,
    info: Option<VideoInfo>,
    info_rx: Option<Receiver<Result<VideoInfo, String>>>,
    error: Option<String>,
    stream: Option<Stream>,
    /// Longest edge of the frames it decodes.
    edge: u32,
    want_play: bool,
    /// Show a frame at `position` even while paused, after a seek.
    want_frame: bool,
    position: f64,
    volume: f32,
    muted: bool,
    ended: bool,
}

impl Player {
    /// Opens `path` and starts playing as soon as it can.
    pub fn open(path: &Path) -> Self {
        Self::open_within(path, PLAY_EDGE)
    }

    /// [`Player::open`] with frames that fit within `edge`, for a video
    /// shown smaller than the screen.
    pub fn open_within(path: &Path, edge: u32) -> Self {
        let (tx, rx) = bounded(1);
        let owned = path.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("picasso-video-info".into())
            .spawn(move || {
                let _ = tx.send(info(&owned));
            });
        Self {
            path: path.to_path_buf(),
            info: None,
            info_rx: Some(rx),
            error: None,
            stream: None,
            edge: edge.clamp(2, PLAY_EDGE),
            want_play: true,
            want_frame: false,
            position: 0.0,
            volume: 1.0,
            muted: false,
            ended: false,
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn has_audio(&self) -> bool {
        self.info.as_ref().is_some_and(|info| info.has_audio)
    }

    pub fn duration(&self) -> f64 {
        self.info.as_ref().map_or(0.0, |info| info.duration)
    }

    pub fn playing(&self) -> bool {
        self.want_play && !self.ended && self.error.is_none()
    }

    /// True until the first frame of a new stream is due.
    pub fn loading(&self) -> bool {
        self.want_play && self.error.is_none() && self.stream.as_ref().is_none_or(|stream| stream.begun.is_none())
    }

    pub fn position(&self) -> f64 {
        let position = self.stream.as_ref().map_or(self.position, Stream::position);
        let duration = self.duration();
        if duration > 0.0 {
            position.min(duration)
        } else {
            position
        }
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn muted(&self) -> bool {
        self.muted
    }

    pub fn play(&mut self) {
        if self.ended {
            self.ended = false;
            self.position = 0.0;
            self.stream = None;
        }
        self.want_play = true;
        if let Some(stream) = self.stream.as_mut() {
            stream.resume();
        }
    }

    pub fn pause(&mut self) {
        self.want_play = false;
        if let Some(stream) = self.stream.as_mut() {
            stream.pause();
        }
    }

    pub fn toggle(&mut self) {
        if self.playing() {
            self.pause();
        } else {
            self.play();
        }
    }

    pub fn seek(&mut self, to: f64) {
        let duration = self.duration();
        self.position = if duration > 0.0 { to.clamp(0.0, duration) } else { to.max(0.0) };
        self.ended = false;
        self.stream = None;
        self.want_frame = true;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = false;
        self.apply_volume();
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        self.apply_volume();
    }

    fn apply_volume(&self) {
        if let Some(audio) = self.stream.as_ref().and_then(|stream| stream.audio.as_ref()) {
            audio.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    /// Advances playback and returns the frame to show when it changed.
    pub fn tick(&mut self, mixer: Option<&rodio::mixer::Mixer>) -> Option<Frame> {
        if let Some(rx) = &self.info_rx {
            match rx.try_recv() {
                Ok(Ok(info)) => {
                    self.info = Some(info);
                    self.info_rx = None;
                }
                Ok(Err(err)) => {
                    self.error = Some(err);
                    self.info_rx = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.info_rx = None,
            }
        }
        let info = self.info.as_ref()?;
        if self.stream.is_none() && !self.ended && (self.want_play || self.want_frame) {
            let volume = if self.muted { 0.0 } else { self.volume };
            match Stream::start(&self.path, info, (self.position, self.edge), !self.want_play, mixer, volume) {
                Ok(stream) => self.stream = Some(stream),
                Err(err) => self.error = Some(err),
            }
            self.want_frame = false;
        }
        let duration = info.duration;
        let stream = self.stream.as_mut()?;
        let frame = stream.tick();
        if stream.finished() {
            self.position = duration.max(stream.position());
            self.stream = None;
            self.ended = true;
            self.want_play = false;
        }
        frame
    }

    /// Moves to `to` seconds without showing it, for a frame shown from
    /// elsewhere; playing goes on from there.
    pub fn set_position(&mut self, to: f64) {
        self.seek(to);
        self.want_frame = false;
    }
}

/// Single frames of one video, each as soon as FFmpeg finds it, and only
/// the one asked for last: frames asked for meanwhile are skipped, so the
/// frame shown keeps up with a handle being dragged.
pub struct Scrubber {
    asks: Sender<f64>,
    frames: Receiver<Frame>,
}

impl Scrubber {
    /// Opens `path` for frames that fit within `edge`, calling `wake`
    /// whenever one is ready.
    pub fn open(path: &Path, edge: u32, wake: impl Fn() + Send + 'static) -> Self {
        let edge = edge.clamp(2, PLAY_EDGE);
        let (asks, asked) = crossbeam_channel::unbounded::<f64>();
        let (found, frames) = bounded(2);
        let path = path.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("picasso-video-scrub".into())
            .spawn(move || {
                let Ok(info) = info(&path) else { return };
                let (width, height) = fit(info.width, info.height, edge);
                while let Ok(first) = asked.recv() {
                    let at = asked.try_iter().last().unwrap_or(first);
                    // The very end has no frame to decode.
                    let at = if info.duration > 0.0 { at.min(info.duration - 0.1) } else { at };
                    let Ok(image) = grab(&path, at.max(0.0), width, height) else { continue };
                    let frame = Frame { width: width as usize, height: height as usize, rgba: image.into_raw() };
                    if found.send(frame).is_err() {
                        return;
                    }
                    wake();
                }
            });
        Self { asks, frames }
    }

    /// Asks for the frame `at` seconds in.
    pub fn show(&self, at: f64) {
        let _ = self.asks.send(at);
    }

    /// The frame found last, once, if one was found since.
    pub fn frame(&self) -> Option<Frame> {
        self.frames.try_iter().last()
    }
}

struct Stream {
    start: f64,
    fps: f64,
    width: usize,
    height: usize,
    frames: Receiver<Vec<u8>>,
    pending: Option<Vec<u8>>,
    shown: u64,
    video_done: bool,
    audio: Option<rodio::Player>,
    audio_ready: Arc<AtomicBool>,
    children: Vec<Child>,
    opened: Instant,
    /// Wall-clock origin of `start`, moved forward by every pause.
    begun: Option<Instant>,
    paused_at: Option<Instant>,
    paused: bool,
}

impl Stream {
    fn start(
        path: &Path,
        info: &VideoInfo,
        (at, edge): (f64, u32),
        paused: bool,
        mixer: Option<&rodio::mixer::Mixer>,
        volume: f32,
    ) -> Result<Self, String> {
        let tools = tools().ok_or(tr("FFmpeg is not installed"))?;
        let (width, height) = fit(info.width, info.height, edge);
        let start = format!("{at:.3}");
        let mut video = command(&tools.ffmpeg)
            .args(["-nostdin", "-v", "error", "-ss", &start, "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-an", "-sn", "-dn"])
            .args(["-vf", &format!("scale={width}:{height},fps={:.6}", info.fps)])
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "-"])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|err| trf("Can't start FFmpeg: {}", &[&err]))?;
        let (frame_tx, frames) = bounded(4);
        let frame_len = width as usize * height as usize * 4;
        if let Some(stdout) = video.stdout.take() {
            spawn_frames(stdout, frame_len, frame_tx);
        }
        let mut children = vec![video];

        let audio_ready = Arc::new(AtomicBool::new(false));
        let mut audio = None;
        if let (true, Some(mixer)) = (info.has_audio, mixer) {
            let child = command(&tools.ffmpeg)
                .args(["-nostdin", "-v", "error", "-ss", &start, "-i"])
                .arg(path)
                .args(["-map", "0:a:0", "-vn", "-sn", "-dn", "-ac", "2", "-ar", "48000", "-f", "f32le", "-"])
                .stdout(Stdio::piped())
                .spawn();
            if let Ok(mut child) = child {
                let (sample_tx, samples) = bounded(16);
                if let Some(stdout) = child.stdout.take() {
                    spawn_samples(stdout, sample_tx, audio_ready.clone());
                }
                children.push(child);
                let player = rodio::Player::connect_new(mixer);
                player.pause();
                player.set_volume(volume);
                player.append(PipeSource::new(samples));
                audio = Some(player);
            }
        }

        Ok(Self {
            start: at,
            fps: info.fps,
            width: width as usize,
            height: height as usize,
            frames,
            pending: None,
            shown: 0,
            video_done: false,
            audio,
            audio_ready,
            children,
            opened: Instant::now(),
            begun: None,
            paused_at: None,
            paused,
        })
    }

    fn position(&self) -> f64 {
        let elapsed = match self.begun {
            Some(begun) => self
                .paused_at
                .unwrap_or_else(Instant::now)
                .saturating_duration_since(begun)
                .as_secs_f64(),
            None => 0.0,
        };
        self.start + elapsed
    }

    fn pause(&mut self) {
        self.paused = true;
        if self.begun.is_some() && self.paused_at.is_none() {
            self.paused_at = Some(Instant::now());
        }
        if let Some(audio) = &self.audio {
            audio.pause();
        }
    }

    fn resume(&mut self) {
        self.paused = false;
        if let (Some(begun), Some(paused_at)) = (self.begun, self.paused_at.take()) {
            self.begun = Some(begun + paused_at.elapsed());
        }
        if self.begun.is_some() {
            if let Some(audio) = &self.audio {
                audio.play();
            }
        }
    }

    fn fill(&mut self) {
        if self.pending.is_some() || self.video_done {
            return;
        }
        match self.frames.try_recv() {
            Ok(frame) => self.pending = Some(frame),
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.video_done = true,
        }
    }

    fn tick(&mut self) -> Option<Frame> {
        self.fill();
        if self.begun.is_none() {
            let audio_ok = self.audio.is_none()
                || self.audio_ready.load(Ordering::Relaxed)
                || self.opened.elapsed() > AUDIO_WAIT;
            if self.pending.is_none() || !audio_ok {
                return None;
            }
            let now = Instant::now();
            self.begun = Some(now);
            if self.paused {
                self.paused_at = Some(now);
            } else if let Some(audio) = &self.audio {
                audio.play();
            }
        }
        let position = self.position();
        let mut shown = None;
        loop {
            self.fill();
            if self.pending.is_none() {
                break;
            }
            let due = self.start + self.shown as f64 / self.fps;
            if self.shown > 0 && due > position {
                break;
            }
            shown = self.pending.take();
            self.shown += 1;
        }
        shown.map(|rgba| Frame {
            width: self.width,
            height: self.height,
            rgba,
        })
    }

    fn finished(&self) -> bool {
        self.video_done && self.pending.is_none()
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn spawn_frames(mut stdout: ChildStdout, frame_len: usize, frames: Sender<Vec<u8>>) {
    let _ = std::thread::Builder::new()
        .name("picasso-video-frames".into())
        .spawn(move || loop {
            let mut frame = vec![0u8; frame_len];
            if stdout.read_exact(&mut frame).is_err() || frames.send(frame).is_err() {
                return;
            }
        });
}

fn spawn_samples(mut stdout: ChildStdout, samples: Sender<Vec<f32>>, ready: Arc<AtomicBool>) {
    let _ = std::thread::Builder::new()
        .name("picasso-video-audio".into())
        .spawn(move || {
            let mut bytes = vec![0u8; AUDIO_CHUNK * 4];
            loop {
                if stdout.read_exact(&mut bytes).is_err() {
                    return;
                }
                let chunk = bytes
                    .chunks_exact(4)
                    .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                    .collect();
                if samples.send(chunk).is_err() {
                    return;
                }
                ready.store(true, Ordering::Relaxed);
            }
        });
}

/// Stereo samples from the audio pipe. When the pipe falls behind it plays
/// silence, one whole frame at a time so left and right never swap.
struct PipeSource {
    samples: Receiver<Vec<f32>>,
    chunk: Vec<f32>,
    at: usize,
    emitted: u64,
    done: bool,
}

impl PipeSource {
    fn new(samples: Receiver<Vec<f32>>) -> Self {
        Self {
            samples,
            chunk: Vec::new(),
            at: 0,
            emitted: 0,
            done: false,
        }
    }
}

impl Iterator for PipeSource {
    type Item = rodio::Sample;

    fn next(&mut self) -> Option<rodio::Sample> {
        let frame_start = self.emitted.is_multiple_of(AUDIO_CHANNELS as u64);
        if self.at >= self.chunk.len() && frame_start && !self.done {
            match self.samples.try_recv() {
                Ok(chunk) => {
                    self.chunk = chunk;
                    self.at = 0;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.done = true,
            }
        }
        let sample = match self.chunk.get(self.at) {
            Some(sample) => {
                self.at += 1;
                *sample
            }
            None if self.done && frame_start => return None,
            None => 0.0,
        };
        self.emitted += 1;
        Some(sample as rodio::Sample)
    }
}

impl rodio::Source for PipeSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> rodio::ChannelCount {
        NonZero::new(AUDIO_CHANNELS).expect("stereo")
    }

    fn sample_rate(&self) -> rodio::SampleRate {
        NonZero::new(AUDIO_RATE).expect("48 kHz")
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_iso6709_locations() {
        assert_eq!(parse_iso6709("+25.0330+121.5654+012.000/"), Some((25.033, 121.5654)));
        assert_eq!(parse_iso6709("-33.8688+151.2093/"), Some((-33.8688, 151.2093)));
        assert_eq!(parse_iso6709("+00.0000+000.0000/"), None);
        assert_eq!(parse_iso6709("garbage"), None);
    }

    #[test]
    fn fits_even_sizes_without_enlarging() {
        assert_eq!(fit(3840, 2160, 1920), (1920, 1080));
        assert_eq!(fit(641, 481, 1920), (640, 480));
        assert_eq!(fit(1080, 1920, 256), (144, 256));
    }

    #[test]
    fn a_video_for_the_web_fits_1080p_either_way_up() {
        assert_eq!(web_size(3840, 2160), (1920, 1080));
        assert_eq!(web_size(2160, 3840), (1080, 1920));
        assert_eq!(web_size(1440, 1440), (1080, 1080));
        assert_eq!(web_size(4096, 1716), (1920, 804), "wider than 16:9 fits its width");
        assert_eq!(web_size(641, 481), (640, 480), "never enlarged");
        let x264 = encoder_args("libx264", 1920, 1080, 30.0);
        assert!(x264.windows(2).any(|pair| pair == ["-crf", "23"]));
        let system = encoder_args("h264_mf", 1920, 1080, 120.0);
        assert!(system.windows(2).any(|pair| pair == ["-b:v", "12441600"]), "{system:?}: at most 60 frames a second");
    }

    #[test]
    fn scrubbing_shows_the_frame_asked_for_last() {
        let dir = std::env::temp_dir().join(format!("picasso-scrub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.mp4");
        if !test_clip(&source, 3) {
            return;
        }
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = woken.clone();
        let scrubber = Scrubber::open(&source, 160, move || {
            counted.fetch_add(1, Ordering::Relaxed);
        });
        for at in [0.2, 0.9, 2.5] {
            scrubber.show(at);
        }
        let wanted = grab(&source, 2.5, 160, 120).unwrap().into_raw();
        let mut last = None;
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline && last.as_ref().is_none_or(|frame: &Frame| frame.rgba != wanted) {
            if let Some(frame) = scrubber.frame() {
                last = Some(frame);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let last = last.expect("a frame was shown");
        assert_eq!((last.width, last.height), (160, 120), "made to fit the edge asked for");
        assert!(last.rgba == wanted, "the frame asked for last is shown");
        assert!(woken.load(Ordering::Relaxed) >= 1, "the screen is woken for it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_part_of_a_video_is_made_for_the_web_without_where_it_was_taken() {
        let dir = std::env::temp_dir().join(format!("picasso-web-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.mp4");
        if !makes_web_video() || !test_clip(&source, 3) {
            return;
        }
        let out = dir.join("web.mp4");
        let last = std::sync::Mutex::new(0.0);
        web_video(&source, &out, 1.0, 2.0, &AtomicBool::new(false), &|part| *last.lock().unwrap() = part).unwrap();
        let made = info(&out).unwrap();
        assert_eq!((made.video_codec.as_str(), made.width, made.height), ("h264", 320, 240));
        assert!((made.duration - 1.0).abs() < 0.3, "{}", made.duration);
        assert!(made.has_audio && made.gps.is_none() && made.taken_at.is_none(), "{made:?}");
        assert_eq!(*last.lock().unwrap(), 1.0, "progress was reported to the end");
        let original = info(&source).unwrap();
        assert!(original.gps.is_some() && original.taken_at.is_some(), "the test's own clip has both: {original:?}");
        let empty = dir.join("empty.mp4");
        assert!(web_video(&source, &empty, original.duration, 0.0, &AtomicBool::new(false), &|_| {}).is_err());
        assert!(!empty.exists());
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|entry| !entry.file_name().to_string_lossy().contains("partial")), "nothing half made is left");

        // Cancelled before it starts: nothing is made.
        let cancelled = dir.join("cancelled.mp4");
        assert_eq!(web_video(&source, &cancelled, 0.0, 0.0, &AtomicBool::new(true), &|_| {}), Err("cancelled".into()));
        assert!(!cancelled.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn parses_ffprobe_json() {
        let json: Value = serde_json::from_str(
            r#"{
                "streams": [
                    {"codec_type": "video", "codec_name": "hevc", "width": 1920, "height": 1080,
                     "avg_frame_rate": "30000/1001",
                     "side_data_list": [{"rotation": -90}]},
                    {"codec_type": "audio", "codec_name": "aac", "sample_rate": "48000", "channel_layout": "stereo"}
                ],
                "format": {"duration": "12.5", "bit_rate": "8000000",
                    "tags": {"com.apple.quicktime.make": "Apple", "com.apple.quicktime.model": "iPhone 15",
                             "com.apple.quicktime.location.ISO6709": "+25.0330+121.5654+012.000/",
                             "com.apple.quicktime.creationdate": "2024-05-01T18:20:30+0800"}}
            }"#,
        )
        .unwrap();
        let info = parse_info(&json).unwrap();
        assert_eq!((info.width, info.height), (1080, 1920));
        assert!((info.fps - 29.97).abs() < 0.01);
        assert_eq!(info.duration, 12.5);
        assert!(info.has_audio);
        assert_eq!(info.camera, "Apple iPhone 15");
        assert_eq!(info.gps, Some((25.033, 121.5654)));
        assert_eq!(info.taken_at, Some(1_714_558_830));
        assert_eq!(info.audio, "aac, 48 kHz, stereo");
    }
}
