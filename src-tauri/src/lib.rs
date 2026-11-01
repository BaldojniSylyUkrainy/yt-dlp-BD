use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use minisign_verify::{PublicKey as MinisignPublicKey, Signature as MinisignSignature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::{OsStr, OsString},
    fs,
    io::{BufRead, BufReader, Read},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;
use uuid::Uuid;
mod conversion;
use conversion::*;
mod browser_proxy;
mod browser_security;
#[cfg(debug_assertions)]
mod browser_smoke;
mod diagnostics;
mod discovery;
mod errors;
mod job_process;
mod media_identity;
#[cfg(test)]
mod media_smoke_tests;
mod probe_cache;
mod process_runner;
mod progress_speed;
mod recovery;
mod runtime;
mod runtime_check;
mod runtime_transaction;
mod thumbnails;
mod vpn;
mod ytdlp_onedir;
mod ytdlp_signature;
use errors::*;
use media_identity::MediaIdentity;
use runtime::*;
use thumbnails::*;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

// The first start of a new binary waits for the system scan: about 30 s on Mac.
const MIN_FREE_SPACE_RESERVE: u64 = 500 * 1024 * 1024;
const DISK_GUARD_TRIGGER: u64 = MIN_FREE_SPACE_RESERVE + 128 * 1024 * 1024;
const THREADS_EXTRACTOR_SOURCE: &str = include_str!("../resources/threads_extractor.py");
const DISCOVERY_EXTRACTOR_SOURCE: &str = include_str!("../resources/discovery_extractor.py");
const INSTAGRAM_PHOTOS_SOURCE: &str = include_str!("../resources/instagram_photos.py");
/// yt-dlp format id that the Instagram plugin gives to a still photo (spelled
/// out in `format_selector`). It only exists on media items without any video.
#[cfg(test)]
const INSTAGRAM_PHOTO_FORMAT_ID: &str = "baldojnyi-photo";
const INSTAGRAM_PHOTO_ONLY_MARKER: &str = "BALDOJNYI_PHOTO_ONLY";
const INSTAGRAM_PHOTOS_SKIPPED_MARKER: &str = "BALDOJNYI_PHOTOS_SKIPPED";
const PHOTOS_SKIPPED_NOTICE: &str =
    "Готово. Фото з поста пропущено: режим «Аудіо» зберігає лише звук.";
const YTDLP_OUTPUT_TEMPLATE: &str = "%(title).140B [%(id).40B].%(ext)s";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_POWERSHELL_UTF8_PREFIX: &str =
    "$OutputEncoding = [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false); ";

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn SetThreadExecutionState(es_flags: u32) -> u32;
}

#[cfg(target_os = "windows")]
const ES_CONTINUOUS: u32 = 0x8000_0000;
#[cfg(target_os = "windows")]
const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreflightResult {
    available_space: u64,
    sufficient: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    id: String,
    discovery_session: Option<String>,
    url: String,
    output_dir: String,
    mode: String,
    quality: String,
    audio_format: String,
    subtitles: bool,
    multi_item: bool,
    cookies_browser: Option<String>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadEvent {
    id: String,
    kind: String,
    percent: Option<f32>,
    speed: Option<String>,
    eta: Option<String>,
    message: Option<String>,
    storage: Option<StorageEstimate>,
    outputs: Option<Vec<DownloadOutput>>,
    title: Option<String>,
    thumbnail: Option<String>,
    uploader: Option<String>,
    extractor: Option<String>,
    error_code: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadOutput {
    path: String,
    size: u64,
    #[serde(flatten)]
    identity: MediaIdentity,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryFileStatus {
    path: String,
    available: bool,
    size: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageEstimate {
    estimated_size: u64,
    required_space: u64,
    available_space: Option<u64>,
    sufficient: Option<bool>,
}

#[derive(Clone)]
struct StorageTracker {
    output_dir: PathBuf,
    estimates: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    video: bool,
    audio_format: String,
}

struct SelectedMediaEstimate {
    media_id: String,
    source_size: u64,
    duration: Option<f64>,
    height: Option<u64>,
    fps: Option<f64>,
    total_bitrate_kbps: Option<f64>,
}

#[derive(Clone)]
struct MediaInfo {
    duration: f64,
    height: u64,
    fps: f64,
}

#[derive(Clone)]
struct DownloadedMedia {
    identity: MediaIdentity,
    path: PathBuf,
    duration: Option<f64>,
    height: Option<u64>,
    fps: Option<f64>,
    video_codec: Option<String>,
    audio_codec: Option<String>,
    /// A still photo saved by the Instagram plugin; never probed or converted.
    photo: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VideoEncoder {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    VideoToolbox,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Nvenc,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    QuickSync,
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Amf,
    Software,
}

type EncoderSelectionCache = Option<(PathBuf, Option<SystemTime>, VideoEncoder)>;

fn encoder_selection_cache() -> &'static Mutex<EncoderSelectionCache> {
    static CACHE: OnceLock<Mutex<EncoderSelectionCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

impl VideoEncoder {
    fn codec(self) -> &'static str {
        match self {
            Self::VideoToolbox => "h264_videotoolbox",
            Self::Nvenc => "h264_nvenc",
            Self::QuickSync => "h264_qsv",
            Self::Amf => "h264_amf",
            Self::Software => "libx264",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::VideoToolbox => "VideoToolbox",
            Self::Nvenc => "NVENC",
            Self::QuickSync => "Quick Sync",
            Self::Amf => "AMF",
            Self::Software => "H.264 software",
        }
    }
}

#[derive(Debug)]
enum ConversionAttemptError {
    Cancelled,
    LowDisk,
    DestinationUnavailable,
    Failed(String),
}

#[derive(Clone, Copy)]
struct AuthRetryPolicy {
    allow_browser_retry: bool,
    youtube_unavailable_may_need_cookies: bool,
    forbidden_may_need_cookies: bool,
}

#[derive(Clone)]
struct OutputReaderState {
    downloaded_files: Option<Arc<Mutex<Vec<DownloadedMedia>>>>,
    storage_tracker: Option<StorageTracker>,
    last_error: Arc<Mutex<Option<String>>>,
    auth_retry: AuthRetryPolicy,
    /// Set when the Instagram plugin dropped photo slides in audio mode.
    photos_skipped: Arc<AtomicBool>,
}

#[derive(Clone, Default)]
struct DownloadManager {
    handoff: Arc<Mutex<()>>,
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    cancelled: Arc<Mutex<HashSet<String>>>,
    auth_required: Arc<Mutex<HashSet<String>>>,
}

#[derive(Default)]
struct SleepPrevention {
    #[cfg(target_os = "macos")]
    process: Mutex<Option<Child>>,
    #[cfg(target_os = "windows")]
    stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl Drop for RuntimeProcessLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
    }
}

#[derive(Default)]
struct AppExitConfirmation {
    approved: AtomicBool,
    pending: AtomicBool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppCloseRequest {
    active_downloads: usize,
}

fn app_close_request(manager: &DownloadManager) -> AppCloseRequest {
    AppCloseRequest {
        active_downloads: manager.jobs.lock().map(|jobs| jobs.len()).unwrap_or(0),
    }
}

fn request_app_close_confirmation(app: &AppHandle) {
    app.state::<AppExitConfirmation>()
        .pending
        .store(true, Ordering::SeqCst);
    let payload = app_close_request(app.state::<DownloadManager>().inner());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
        let _ = window.emit("app-close-requested", payload);
    }
}

#[tauri::command]
fn take_app_close_request(
    manager: State<'_, DownloadManager>,
    confirmation: State<'_, AppExitConfirmation>,
) -> Option<AppCloseRequest> {
    confirmation
        .pending
        .swap(false, Ordering::SeqCst)
        .then(|| app_close_request(manager.inner()))
}

#[tauri::command]
fn dismiss_app_close_request(confirmation: State<'_, AppExitConfirmation>) {
    confirmation.pending.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn allow_app_exit_once(confirmation: State<'_, AppExitConfirmation>) {
    confirmation.pending.store(false, Ordering::SeqCst);
    confirmation.approved.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn cancel_app_exit_approval(confirmation: State<'_, AppExitConfirmation>) {
    confirmation.approved.store(false, Ordering::SeqCst);
}

#[tauri::command]
fn confirm_app_close(app: AppHandle, confirmation: State<'_, AppExitConfirmation>) {
    confirmation.pending.store(false, Ordering::SeqCst);
    confirmation.approved.store(true, Ordering::SeqCst);
    app.exit(0);
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn is_threads_host(host: &str) -> bool {
    host_matches(host, "threads.com") || host_matches(host, "threads.net")
}

fn is_tiktok_host(host: &str) -> bool {
    host_matches(host, "tiktok.com")
}

fn impersonation_target(is_threads: bool, is_tiktok: bool) -> Option<&'static str> {
    if is_tiktok {
        Some("Chrome-99:Android-12")
    } else if is_threads {
        Some("chrome")
    } else {
        None
    }
}

fn normalize_media_url(value: &str) -> Result<Url, String> {
    let mut parsed = Url::parse(value).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    let canonical_tiktok_path = if is_tiktok_host(&host) {
        parsed
            .path_segments()
            .map(|segments| {
                segments
                    .filter(|segment| !segment.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|segments| {
                segments.len() == 3
                    && segments[0].starts_with('@')
                    && segments[1].eq_ignore_ascii_case("video")
                    && !segments[2].is_empty()
                    && segments[2].bytes().all(|value| value.is_ascii_digit())
            })
            .map(|segments| format!("/{}/{}/{}", segments[0], segments[1], segments[2]))
    } else {
        None
    };
    if let Some(path) = canonical_tiktok_path {
        parsed.set_path(&path);
        parsed.set_query(None);
        parsed.set_fragment(None);
    }
    Ok(parsed)
}

fn format_selector(mode: &str) -> &'static str {
    if mode == "audio" {
        "bestaudio/best"
    } else {
        // The trailing photo format id is a last resort that can match only an
        // Instagram media item without video; every video keeps the previous order.
        "bestvideo+bestaudio/best/baldojnyi-photo"
    }
}

/// A quality limit is a preference, not a filter. `res` is the shorter side of
/// the frame, so 1080x1920 counts as 1080p, and a video that exists only in a
/// larger size or without size metadata still downloads instead of failing.
fn quality_sort(mode: &str, quality: &str) -> Option<&'static str> {
    if mode == "audio" {
        return None;
    }
    match quality {
        "2160" => Some("res:2160"),
        "1080" => Some("res:1080"),
        "720" => Some("res:720"),
        "480" => Some("res:480"),
        _ => None,
    }
}

fn is_expected_photo_warning(line: &str) -> bool {
    let value = line.to_ascii_lowercase();
    value.starts_with("warning:")
        && value.contains("only images are available for download")
        && !value.contains("drm")
}

/// Audio mode never saves photos: the Instagram plugin drops photo slides and
/// reports it, or fails with a dedicated marker when the post has no video.
fn photo_extractor_args(mode: &str) -> Option<[&'static str; 2]> {
    (mode == "audio").then_some(["--extractor-args", "instagram:baldojnyi_photos=skip"])
}

fn playlist_flag(multi_item: bool) -> &'static str {
    if multi_item {
        "--yes-playlist"
    } else {
        "--no-playlist"
    }
}

fn yt_dlp_output_encoding_args() -> [&'static str; 2] {
    ["--encoding", "UTF-8"]
}

fn extraction_args(
    app: &AppHandle,
    mode: &str,
    quality: &str,
    multi_item: bool,
    cookies_browser: Option<&str>,
    impersonation: Option<&str>,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![
        OsString::from("--plugin-dirs"),
        threads_plugin_dir(app)?.into_os_string(),
        OsString::from(playlist_flag(multi_item)),
        OsString::from("-f"),
        OsString::from(format_selector(mode)),
    ];
    if let Some(sort) = quality_sort(mode, quality) {
        args.extend([OsString::from("-S"), OsString::from(sort)]);
    }
    if let Some(photo_args) = photo_extractor_args(mode) {
        args.extend(photo_args.map(OsString::from));
    }
    if let Some(target) = impersonation {
        args.extend([OsString::from("--impersonate"), OsString::from(target)]);
    }
    if let Some(browser) = cookies_browser {
        let supported = [
            "brave", "chrome", "chromium", "edge", "firefox", "opera", "vivaldi",
        ];
        if !supported.contains(&browser) {
            return Err("Непідтримуваний браузер для cookies".into());
        }
        args.extend([
            OsString::from("--cookies-from-browser"),
            OsString::from(browser),
        ]);
    }
    if let Some(path) = managed_deno_path(app)? {
        args.extend([
            OsString::from("--js-runtimes"),
            OsString::from(format!("deno:{}", path.display())),
        ]);
    }
    Ok(args)
}

fn replace_option_value(args: &[OsString], option: &str, value: &str) -> Vec<OsString> {
    let mut replaced = args.to_vec();
    for index in 0..replaced.len().saturating_sub(1) {
        if replaced[index] == OsStr::new(option) {
            replaced[index + 1] = OsString::from(value);
            break;
        }
    }
    replaced
}

fn youtube_retry_message(retry_attempt: u8) -> &'static str {
    if retry_attempt == 1 {
        "YouTube перервав потік. Оновлюємо посилання й продовжуємо…"
    } else {
        "YouTube знову перервав потік. Отримуємо ще одне свіже посилання…"
    }
}

const PROGRESS_MARKER: &str = "__YTDLP_PROGRESS__";

#[derive(Debug, Default, PartialEq)]
struct DownloadProgress {
    percent: Option<f32>,
    speed: Option<String>,
    eta: Option<String>,
    message: Option<String>,
}

/// `percent|speed|eta|item|items|downloaded|total|title`. Inside a playlist or
/// carousel the bar covers the whole collection instead of restarting on every
/// file. Speed and time left are averaged over the last seconds by `smoother`.
fn parse_download_progress(
    payload: &str,
    smoother: &mut progress_speed::SpeedSmoother,
    now: Instant,
) -> DownloadProgress {
    let mut values = payload.splitn(8, '|').map(str::trim);
    let mut text = || values.next().filter(|value| !value.is_empty());
    let percent = text().and_then(|value| value.trim_end_matches('%').trim().parse::<f32>().ok());
    let raw_speed = text();
    let raw_eta = text();
    let item = text().and_then(|value| value.parse::<u32>().ok());
    let items = text().and_then(|value| value.parse::<u32>().ok());
    let downloaded = progress_speed::parse_byte_count(text());
    let total = progress_speed::parse_byte_count(text());
    let progress_speed::SpeedReading { speed, eta } =
        smoother.sample(now, downloaded, total, raw_speed, raw_eta);
    let collection = item
        .zip(items)
        .filter(|(item, items)| *items > 1 && item <= items);
    DownloadProgress {
        percent: match collection {
            Some((item, items)) => {
                percent.map(|percent| ((item - 1) as f32 + percent / 100.0) / items as f32 * 100.0)
            }
            None => percent,
        },
        speed,
        eta,
        message: collection.map(|(item, items)| format!("Файл {item} із {items}")),
    }
}

fn saved_files_label(count: usize) -> String {
    let word = match (count % 10, count % 100) {
        (1, remainder) if remainder != 11 => "файл",
        (2..=4, remainder) if !(12..=14).contains(&remainder) => "файли",
        _ => "файлів",
    };
    format!("Готово. Збережено {count} {word}.")
}

fn completion_message(outputs: usize, photos_skipped: bool) -> Option<String> {
    if photos_skipped {
        Some(PHOTOS_SKIPPED_NOTICE.into())
    } else {
        (outputs > 1).then(|| saved_files_label(outputs))
    }
}

fn terminal_kind(
    cancelled: bool,
    auth_required: bool,
    low_disk: bool,
    succeeded: bool,
    conversion_failed: bool,
) -> &'static str {
    if cancelled {
        "cancelled"
    } else if auth_required {
        "auth_required"
    } else if !low_disk && succeeded && !conversion_failed {
        "completed"
    } else {
        "failed"
    }
}

fn emit_line(app: &AppHandle, id: &str, line: &str, smoother: &mut progress_speed::SpeedSmoother) {
    if !line.starts_with(PROGRESS_MARKER) {
        diagnostics::record(id, line);
    }
    const POSTPROCESS_MARKER: &str = "__YTDLP_POSTPROCESS__";
    if let Some(payload) = line.strip_prefix(PROGRESS_MARKER) {
        let progress = parse_download_progress(payload, smoother, Instant::now());
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: "progress".into(),
                percent: progress.percent,
                speed: progress.speed,
                eta: progress.eta,
                message: progress.message,
                ..Default::default()
            },
        );
    } else if let Some(payload) = line.strip_prefix(POSTPROCESS_MARKER) {
        let mut values = payload.splitn(3, '|');
        let status = values.next().map(str::trim).unwrap_or_default();
        let postprocessor = values.next().map(str::trim).unwrap_or_default();
        let _title = values.next().map(str::trim).unwrap_or_default();
        if status == "started" {
            let message = if postprocessor.contains("Merger")
                || postprocessor.contains("Convertor")
                || postprocessor.contains("Remuxer")
            {
                "Об’єднуємо завантажені потоки… Не закривайте застосунок"
            } else {
                "Обробляємо файл… Не закривайте застосунок"
            };
            let _ = app.emit(
                "download-event",
                DownloadEvent {
                    id: id.into(),
                    kind: "postprocess".into(),
                    message: Some(message.into()),
                    ..Default::default()
                },
            );
        }
    } else if !line.trim().is_empty() {
        let normalized = line.to_ascii_lowercase();
        let is_postprocess_log = ["[merger]", "[videoconvertor]", "[videoremuxer]"]
            .iter()
            .any(|marker| normalized.contains(marker));
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: if is_postprocess_log {
                    "postprocess".into()
                } else {
                    "log".into()
                },
                message: Some(if is_postprocess_log {
                    "Об’єднуємо завантажені потоки… Не закривайте застосунок".into()
                } else {
                    line.trim().to_string()
                }),
                ..Default::default()
            },
        );
    }
}

fn looks_like_auth_error(
    line: &str,
    youtube_unavailable_may_need_cookies: bool,
    forbidden_may_need_cookies: bool,
) -> bool {
    let value = line.to_ascii_lowercase();
    let explicit_auth_error = [
        "sign in to confirm",
        "login required",
        "log in to",
        "use --cookies-from-browser",
        "cookies are required",
        "confirm your age",
        "age-restricted",
        "members-only",
        "private video",
        "this video is private",
    ]
    .iter()
    .any(|pattern| value.contains(pattern));
    let ambiguous_youtube_unavailable = youtube_unavailable_may_need_cookies
        && ["video unavailable", "video is unavailable"]
            .iter()
            .any(|pattern| value.contains(pattern));
    let ambiguous_forbidden = forbidden_may_need_cookies && is_forbidden_error(&value);
    explicit_auth_error || ambiguous_youtube_unavailable || ambiguous_forbidden
}

/// Only yt-dlp's own `ERROR:` lines count: progress and warnings carry titles,
/// speeds and IDs that can contain the same words and digits by coincidence.
fn should_request_browser_auth(line: &str, policy: AuthRetryPolicy) -> bool {
    policy.allow_browser_retry
        && line.trim_start().to_ascii_lowercase().starts_with("error:")
        && looks_like_auth_error(
            line,
            policy.youtube_unavailable_may_need_cookies,
            policy.forbidden_may_need_cookies,
        )
}

fn json_number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    })
}

fn positive_finite_number(value: Option<&serde_json::Value>) -> Option<f64> {
    json_number(value).filter(|value| value.is_finite() && *value > 0.0)
}

fn downloaded_media_from_print(payload: &str) -> Option<DownloadedMedia> {
    let metadata = serde_json::from_str::<serde_json::Value>(payload).ok();
    if let Some(metadata) = metadata {
        let path = metadata
            .get("filepath")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        return Some(DownloadedMedia {
            identity: MediaIdentity::from_metadata(&metadata),
            path: PathBuf::from(path),
            duration: positive_finite_number(metadata.get("duration")),
            height: positive_finite_number(metadata.get("height"))
                .map(|value| value.round() as u64),
            fps: positive_finite_number(metadata.get("fps")),
            video_codec: metadata
                .get("vcodec")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase),
            audio_codec: metadata
                .get("acodec")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase),
            photo: metadata
                .get("baldojnyi_photo")
                .and_then(|value| value.as_bool())
                == Some(true),
        });
    }

    let path = payload.trim();
    (!path.is_empty()).then(|| DownloadedMedia {
        identity: MediaIdentity::default(),
        path: PathBuf::from(path),
        duration: None,
        height: None,
        fps: None,
        video_codec: None,
        audio_codec: None,
        photo: false,
    })
}

fn parse_size_estimate(metadata: &serde_json::Value) -> Option<SelectedMediaEstimate> {
    let media_id = metadata
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let duration = json_number(metadata.get("duration"));
    let total_bitrate_kbps = json_number(metadata.get("tbr"));
    let bytes = json_number(metadata.get("filesize"))
        .or_else(|| json_number(metadata.get("filesize_approx")))
        .or_else(|| {
            let duration = duration?;
            let bitrate_kbps = total_bitrate_kbps?;
            Some(duration * bitrate_kbps * 1_000.0 / 8.0)
        })?;
    (bytes.is_finite() && bytes > 0.0 && bytes <= u64::MAX as f64).then_some(
        SelectedMediaEstimate {
            media_id,
            source_size: bytes.round() as u64,
            duration,
            height: json_number(metadata.get("height")).map(|value| value.round() as u64),
            fps: json_number(metadata.get("fps")),
            total_bitrate_kbps,
        },
    )
}

fn metadata_string(metadata: &serde_json::Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn emit_download_metadata(app: &AppHandle, id: &str, metadata: &serde_json::Value) {
    let title = metadata_string(metadata, "title");
    let thumbnail = metadata_string(metadata, "thumbnail");
    let uploader =
        metadata_string(metadata, "uploader").or_else(|| metadata_string(metadata, "channel"));
    let extractor = metadata_string(metadata, "extractor_key")
        .or_else(|| metadata_string(metadata, "extractor"));
    if title.is_none() && thumbnail.is_none() && uploader.is_none() && extractor.is_none() {
        return;
    }
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            id: id.into(),
            kind: "metadata".into(),
            title,
            thumbnail,
            uploader,
            extractor,
            ..Default::default()
        },
    );
}

fn target_video_bitrate(height: Option<u64>, fps: Option<f64>, fallback_kbps: Option<f64>) -> u64 {
    let base = match height {
        Some(value) if value >= 2160 => 45_000_000,
        Some(value) if value >= 1440 => 24_000_000,
        Some(value) if value >= 1080 => 14_000_000,
        Some(value) if value >= 720 => 7_500_000,
        Some(value) if value >= 480 => 4_000_000,
        Some(_) => 2_000_000,
        None => {
            (fallback_kbps.unwrap_or(4_000.0) * 2_500.0).clamp(2_000_000.0, 45_000_000.0) as u64
        }
    };
    if fps.unwrap_or(30.0) > 45.0 {
        base.saturating_mul(3) / 2
    } else {
        base
    }
}

fn estimated_output_size(
    estimate: &SelectedMediaEstimate,
    video: bool,
    audio_format: &str,
) -> Option<u64> {
    let duration = estimate
        .duration
        .filter(|value| value.is_finite() && *value > 0.0)?;
    let bitrate = if video {
        let video_maxrate =
            target_video_bitrate(estimate.height, estimate.fps, estimate.total_bitrate_kbps)
                .saturating_mul(11)
                / 10;
        video_maxrate.saturating_add(192_000)
    } else {
        match audio_format {
            "wav" => 1_536_000,
            "opus" => 160_000,
            _ => 192_000,
        }
    };
    let bytes = duration * bitrate as f64 / 8.0 * 1.02;
    if bytes.is_finite() && bytes > 0.0 && bytes <= u64::MAX as f64 {
        Some(bytes.ceil() as u64)
    } else {
        None
    }
}

fn estimated_final_size(estimate: &SelectedMediaEstimate, tracker: &StorageTracker) -> u64 {
    estimated_output_size(estimate, tracker.video, &tracker.audio_format)
        .unwrap_or(estimate.source_size)
}

fn low_disk_guard_triggered(available: u64) -> bool {
    available <= DISK_GUARD_TRIGGER
}

fn available_disk_space(path: &Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

/// Shown when the save folder disappears mid-job, e.g. an external drive was
/// unplugged. `errors.rs` recognises this text to pick the error code.
pub(crate) const DESTINATION_UNAVAILABLE_MESSAGE: &str =
    "Папка збереження стала недоступною: диск від’єднано або папку видалено. Під’єднайте диск і повторіть.";

#[derive(Debug, PartialEq)]
enum DiskGuard {
    Ok,
    Low,
    Unavailable,
}

/// Free space alone cannot tell a full disk from a missing one: an unplugged
/// drive fails the space query, while on Windows a deleted folder on a present
/// drive still reports the drive's space. The folder is checked first (jobs only
/// start in an existing folder); any other unknown state still stops the job to
/// protect the disk.
fn disk_guard(path: &Path) -> DiskGuard {
    if !destination_present(path) {
        return DiskGuard::Unavailable;
    }
    match available_disk_space(path) {
        Some(available) if !low_disk_guard_triggered(available) => DiskGuard::Ok,
        _ => DiskGuard::Low,
    }
}

fn destination_present(path: &Path) -> bool {
    path.try_exists().unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn macos_important_usage_space(path: &Path) -> Option<u64> {
    use objc2_foundation::{
        NSNumber, NSString, NSURLVolumeAvailableCapacityForImportantUsageKey, NSURL,
    };

    let path = NSString::from_str(&path.to_string_lossy());
    let url = NSURL::fileURLWithPath_isDirectory(&path, true);
    let mut value = None;
    unsafe {
        url.getResourceValue_forKey_error(
            &mut value,
            NSURLVolumeAvailableCapacityForImportantUsageKey,
        )
        .ok()?;
    }
    value?
        .downcast::<NSNumber>()
        .ok()
        .map(|value| value.as_u64())
}

/// Capacity suitable for a user-requested download. On APFS this includes
/// purgeable space that macOS can reclaim, matching the system storage UI.
/// The live guard deliberately continues to use `available_disk_space`, so it
/// still reacts to immediately writable blocks while a process is active.
#[cfg(target_os = "macos")]
fn select_macos_planning_capacity(
    physical: Option<u64>,
    important_usage: Option<u64>,
) -> Option<u64> {
    match (physical, important_usage) {
        (Some(physical), Some(important_usage)) => Some(physical.max(important_usage)),
        (physical, important_usage) => physical.or(important_usage),
    }
}

fn planning_available_disk_space(path: &Path) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        select_macos_planning_capacity(
            available_disk_space(path),
            macos_important_usage_space(path),
        )
    }

    #[cfg(not(target_os = "macos"))]
    available_disk_space(path)
}

#[tauri::command]
async fn folder_free_space(path: String) -> Result<u64, String> {
    tauri::async_runtime::spawn_blocking(move || folder_free_space_inner(path))
        .await
        .map_err(|e| e.to_string())?
}
fn folder_free_space_inner(path: String) -> Result<u64, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err("Обрана папка недоступна".into());
    }
    planning_available_disk_space(&path)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".into())
}

#[tauri::command]
async fn preflight_download(url: String, output_dir: String) -> Result<PreflightResult, String> {
    tauri::async_runtime::spawn_blocking(move || preflight_download_inner(&url, &output_dir))
        .await
        .map_err(|e| e.to_string())?
}
fn preflight_download_inner(url: &str, output_dir: &str) -> Result<PreflightResult, String> {
    normalize_media_url(url)?;
    let output_dir = PathBuf::from(output_dir);
    if !output_dir.is_dir() {
        return Err("Обрана папка для завантаження недоступна".into());
    }
    let available_space = planning_available_disk_space(&output_dir)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".to_string())?;
    Ok(PreflightResult {
        available_space,
        sufficient: available_space > DISK_GUARD_TRIGGER,
    })
}

fn emit_storage_estimate(
    app: &AppHandle,
    id: &str,
    tracker: &StorageTracker,
    estimate: SelectedMediaEstimate,
) {
    let source_size = estimate.source_size;
    let final_size = estimated_final_size(&estimate, tracker);
    let (estimated_size, required_space) = tracker
        .estimates
        .lock()
        .ok()
        .map(|mut estimates| {
            estimates.insert(estimate.media_id, (source_size, final_size));
            estimates.values().copied().fold(
                (0_u64, 0_u64),
                |(final_total, working_total), (source, final_output)| {
                    (
                        final_total.saturating_add(final_output),
                        working_total.saturating_add(source.saturating_add(final_output)),
                    )
                },
            )
        })
        .unwrap_or((final_size, source_size.saturating_add(final_size)));
    let required_space = required_space.saturating_add(MIN_FREE_SPACE_RESERVE);
    let available_space = planning_available_disk_space(&tracker.output_dir);
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            id: id.into(),
            kind: "storage_estimate".into(),
            storage: Some(StorageEstimate {
                estimated_size,
                required_space,
                available_space,
                sufficient: available_space.map(|available| available >= required_space),
            }),
            ..Default::default()
        },
    );
}

fn read_lossy_line<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
) -> std::io::Result<Option<String>> {
    buffer.clear();
    if reader.read_until(b'\n', buffer)? == 0 {
        return Ok(None);
    }
    while matches!(buffer.last(), Some(b'\n' | b'\r')) {
        buffer.pop();
    }
    Ok(Some(String::from_utf8_lossy(buffer).into_owned()))
}

fn read_lossy_to_string<R: Read>(mut reader: R) -> std::io::Result<String> {
    let mut buffer = Vec::new();
    let mut chunk = [0; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
        if buffer.len() > 64 * 1024 {
            buffer.drain(..buffer.len() - 64 * 1024);
        }
    }
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn read_output<R: Read + Send + 'static>(
    app: AppHandle,
    manager: DownloadManager,
    id: String,
    reader: R,
    state: OutputReaderState,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut buffer = Vec::new();
        let mut smoother = progress_speed::SpeedSmoother::default();
        while let Ok(Some(line)) = read_lossy_line(&mut reader, &mut buffer) {
            if line.starts_with(PROGRESS_MARKER) || line.starts_with("__YTDLP_POSTPROCESS__") {
                emit_line(&app, &id, &line, &mut smoother);
                continue;
            }
            if let Some((_, payload)) = line.split_once("__YTDLP_CANDIDATES__") {
                if payload.len() < 300_000 {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) {
                        if let (Some(page), Some(urls)) =
                            (value["pageUrl"].as_str(), value["urls"].as_array())
                        {
                            discovery::publish(
                                &app,
                                &id,
                                page,
                                urls.iter()
                                    .take(30)
                                    .filter_map(|v| v.as_str().map(str::to_string))
                                    .collect(),
                            );
                        }
                    }
                }
                continue;
            }
            if let Some(payload) = line.strip_prefix("__YTDLP_SIZE__") {
                if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(payload) {
                    emit_download_metadata(&app, &id, &metadata);
                    if let (Some(tracker), Some(estimate)) = (
                        state.storage_tracker.as_ref(),
                        parse_size_estimate(&metadata),
                    ) {
                        emit_storage_estimate(&app, &id, tracker, estimate);
                    }
                }
                continue;
            } else if line.contains(INSTAGRAM_PHOTOS_SKIPPED_MARKER) {
                state.photos_skipped.store(true, Ordering::Relaxed);
                continue;
            } else if is_expected_photo_warning(&line) {
                // yt-dlp warns about image-only formats; for a plugin photo this
                // is the expected result, so keep it out of the progress popup.
                diagnostics::record(&id, &line);
                continue;
            } else if let Some(path) = line.strip_prefix("__YTDLP_FILE__") {
                if let Some(files) = state.downloaded_files.as_ref() {
                    if let (Some(media), Ok(mut files)) =
                        (downloaded_media_from_print(path), files.lock())
                    {
                        if !files.iter().any(|file| file.path == media.path) {
                            files.push(media);
                        }
                    }
                }
                continue;
            }
            if should_request_browser_auth(&line, state.auth_retry) {
                if let Ok(mut jobs) = manager.auth_required.lock() {
                    jobs.insert(id.clone());
                }
            }
            let normalized = line.to_ascii_lowercase();
            if normalized.contains("error:")
                || normalized.contains("http error")
                || normalized.contains("unable to download")
            {
                if let Ok(mut error) = state.last_error.lock() {
                    *error = Some(line.trim().trim_start_matches("ERROR:").trim().to_string());
                }
            }
            emit_line(&app, &id, &line, &mut smoother);
        }
    })
}

fn download_was_cancelled(manager: &DownloadManager, id: &str) -> bool {
    manager
        .cancelled
        .lock()
        .ok()
        .map(|jobs| jobs.contains(id))
        .unwrap_or(false)
}

fn stop_child_process_group(child: &Arc<Mutex<Child>>) -> Result<(), String> {
    // Hold the child lock through termination. Never signal a PID already reaped
    // by the monitor: it may now identify an unrelated process.
    let mut child = child
        .lock()
        .map_err(|_| "Не вдалося зупинити процес".to_string())?;
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        let process_group = child.id() as i32;
        let result = unsafe { libc::kill(-process_group, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(format!("Не вдалося зупинити процес: {error}"))
    }

    #[cfg(not(unix))]
    {
        let process_id = child.id();
        #[cfg(target_os = "windows")]
        {
            let mut command = Command::new("taskkill");
            configure_command(&mut command);
            let mut killer = command
                .args(["/PID", &process_id.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| format!("Не вдалося запустити taskkill: {error}"))?;
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match killer.try_wait() {
                    Ok(Some(status)) if status.success() => return Ok(()),
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if Instant::now() < deadline => {
                        thread::sleep(Duration::from_millis(25))
                    }
                    _ => {
                        let _ = killer.kill();
                        let _ = killer.wait();
                        break;
                    }
                }
            }
        }
        child
            .kill()
            .map_err(|error| format!("Не вдалося зупинити процес: {error}"))
    }
}

fn ffprobe_path(ffmpeg_path: &Path) -> PathBuf {
    ffmpeg_path.with_file_name(managed_binary_name("ffprobe"))
}

fn parse_frame_rate(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

fn media_info(ffmpeg_path: &Path, input: &Path) -> Result<MediaInfo, String> {
    let mut command = Command::new(ffprobe_path(ffmpeg_path));
    command.args(LOCAL_INPUT_PROTOCOLS);
    configure_command(&mut command);
    let output = process_runner::capture(
        command
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "format=duration:stream=height,avg_frame_rate",
                "-of",
                "json",
            ])
            .arg(input),
        Duration::from_secs(15),
    )
    .map_err(|error| format!("Не вдалося запустити ffprobe: {error}"))?;
    if !output.status.success() {
        return Err("ffprobe не зміг визначити тривалість відео".into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "ffprobe повернув некоректні дані про відео".to_string())?;
    let duration = json_number(metadata.pointer("/format/duration"))
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "ffprobe повернув некоректну тривалість відео".to_string())?;
    let stream = metadata
        .get("streams")
        .and_then(|value| value.as_array())
        .and_then(|streams| streams.first());
    let height = stream
        .and_then(|stream| json_number(stream.get("height")))
        .map(|value| value.round() as u64)
        .unwrap_or(1080);
    let fps = stream
        .and_then(|stream| stream.get("avg_frame_rate"))
        .and_then(|value| value.as_str())
        .and_then(parse_frame_rate)
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(30.0);
    Ok(MediaInfo {
        duration,
        height,
        fps,
    })
}

fn media_duration(ffmpeg_path: &Path, input: &Path) -> Result<f64, String> {
    let mut command = Command::new(ffprobe_path(ffmpeg_path));
    command.args(LOCAL_INPUT_PROTOCOLS);
    configure_command(&mut command);
    let output = process_runner::capture(
        command
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(input),
        Duration::from_secs(15),
    )
    .map_err(|error| format!("Не вдалося запустити ffprobe: {error}"))?;
    if !output.status.success() {
        return Err("ffprobe не зміг визначити тривалість медіафайла".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .ok_or_else(|| "ffprobe повернув некоректну тривалість медіафайла".into())
}

fn available_output_path(input: &Path, extension: &str) -> PathBuf {
    let preferred = input.with_extension(extension);
    if !preferred.exists() {
        return preferred;
    }
    let parent = input.parent().unwrap_or_else(|| Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("video");
    for index in 1..10_000 {
        let candidate = parent.join(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    parent.join(format!("{stem}-{}.{extension}", Uuid::new_v4()))
}

fn format_eta(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "—".into();
    }
    let total = seconds.round() as u64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn conversion_percent(elapsed: f64, duration: f64, file_index: usize, file_count: usize) -> f32 {
    let file_percent = (elapsed / duration * 100.0).clamp(0.0, 100.0);
    ((file_index as f64 + file_percent / 100.0) / file_count.max(1) as f64 * 100.0)
        .clamp(0.0, 100.0) as f32
}

#[tauri::command]
async fn inspect_history_files(paths: Vec<String>) -> Result<Vec<HistoryFileStatus>, String> {
    tauri::async_runtime::spawn_blocking(move || inspect_history_files_inner(paths))
        .await
        .map_err(|e| e.to_string())
}
fn inspect_history_files_inner(paths: Vec<String>) -> Vec<HistoryFileStatus> {
    paths
        .into_iter()
        .take(500)
        .map(|path| {
            let metadata = fs::metadata(&path)
                .ok()
                .filter(|metadata| metadata.is_file());
            HistoryFileStatus {
                path,
                available: metadata.is_some(),
                size: metadata.map(|metadata| metadata.len()),
            }
        })
        .collect()
}

type JobAttempt = (
    Arc<Mutex<Child>>,
    Option<thread::JoinHandle<()>>,
    Option<thread::JoinHandle<()>>,
);

/// Start one yt-dlp attempt with piped output and both pipes being read.
fn spawn_job_attempt(
    app: &AppHandle,
    manager: &DownloadManager,
    id: &str,
    program: &OsStr,
    args: &[OsString],
    state: &OutputReaderState,
) -> Result<JobAttempt, String> {
    let mut command = Command::new(program);
    configure_command(&mut command);
    command
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    command.process_group(0);
    let (child, stdout, stderr) = job_process::spawn(manager, id, &mut command)?;
    let reader = |pipe: Box<dyn Read + Send>| {
        read_output(
            app.clone(),
            manager.clone(),
            id.to_string(),
            pipe,
            state.clone(),
        )
    };
    let stdout_reader = stdout.map(|pipe| reader(Box::new(pipe)));
    let stderr_reader = stderr.map(|pipe| reader(Box::new(pipe)));
    Ok((child, stdout_reader, stderr_reader))
}

fn emit_retrying(app: &AppHandle, id: &str, message: &str) {
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            id: id.into(),
            kind: "retrying".into(),
            message: Some(message.into()),
            ..Default::default()
        },
    );
}

fn prepare_retry_dir(directory: &Path, last_error: &Mutex<Option<String>>) -> bool {
    match reset_download_temp_dir(directory) {
        Ok(()) => true,
        Err(error) => {
            if let Ok(mut last_error) = last_error.lock() {
                *last_error = Some(error);
            }
            false
        }
    }
}

/// yt-dlp reports an existing file with the same name as if it had just saved
/// it. Only a file created after this job began is ours to replace; if the time
/// cannot be read, the file stays.
fn source_created_by_job(path: &Path, job_started: SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.created())
        .is_ok_and(|created| created >= job_started)
}

#[tauri::command]
async fn start_download(
    app: AppHandle,
    manager: State<'_, DownloadManager>,
    maintenance: State<'_, RuntimeMaintenance>,
    request: DownloadRequest,
) -> Result<(), String> {
    let _maintenance_available = maintenance
        .operation
        .try_lock()
        .map_err(|_| "Компоненти зараз оновлюються. Зачекайте завершення перевірки".to_string())?;
    let id = Uuid::parse_str(&request.id)
        .map_err(|_| "Некоректний ідентифікатор завантаження".to_string())?
        .to_string();
    {
        let jobs = manager
            .jobs
            .lock()
            .map_err(|_| "Внутрішня помилка черги".to_string())?;
        if jobs.contains_key(&id) {
            return Err("Це завантаження вже запущено".into());
        }
        if !jobs.is_empty() {
            return Err("Інше завантаження ще не завершено".into());
        }
    }
    let parsed = normalize_media_url(&request.url)?;
    let selected_source =
        discovery::take_selection(&app, request.discovery_session.as_deref(), parsed.as_str())?;
    let is_youtube = parsed
        .host_str()
        .map(|host| {
            host_matches(&host.to_ascii_lowercase(), "youtube.com")
                || host_matches(&host.to_ascii_lowercase(), "youtu.be")
        })
        .unwrap_or(false);
    let is_threads = parsed
        .host_str()
        .map(|host| is_threads_host(&host.to_ascii_lowercase()))
        .unwrap_or(false);
    let is_tiktok = parsed
        .host_str()
        .map(|host| is_tiktok_host(&host.to_ascii_lowercase()))
        .unwrap_or(false);
    let auth_retry = AuthRetryPolicy {
        allow_browser_retry: request.cookies_browser.is_none(),
        youtube_unavailable_may_need_cookies: request.cookies_browser.is_none() && is_youtube,
        forbidden_may_need_cookies: request.cookies_browser.is_none() && is_tiktok,
    };
    let output_dir = PathBuf::from(&request.output_dir);
    if !output_dir.is_dir() {
        return Err(
            "Обрана папка завантажень недоступна. Підключіть диск або виберіть іншу папку".into(),
        );
    }
    let available_space = planning_available_disk_space(&output_dir)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".to_string())?;
    if available_space <= DISK_GUARD_TRIGGER {
        return Err("Недостатньо вільного місця для безпечного завантаження".into());
    }
    let yt_dlp = yt_dlp_path(&app)?;
    if !yt_dlp.is_file() {
        return Err("Спочатку встановіть yt-dlp".into());
    }
    let runtime = runtime_dir(&app)?;
    let mut command = Command::new(yt_dlp);
    configure_command(&mut command);
    command.args([
        "--ignore-config",
        yt_dlp_output_encoding_args()[0],
        yt_dlp_output_encoding_args()[1],
        "--no-simulate",
        // File dates mean "saved here"; a server date could also rewrite the
        // creation time that proves a file belongs to this job.
        "--no-mtime",
        "--newline",
        "--no-colors",
        "--progress",
        "--print",
        "video:__YTDLP_SIZE__%(.{id,title,thumbnail,uploader,channel,extractor,extractor_key,webpage_url,filesize,filesize_approx,duration,tbr,height,fps})j",
        "--progress-delta",
        "0.25",
        "--progress-template",
        "__YTDLP_PROGRESS__%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(info.playlist_autonumber)s|%(info.n_entries)s|%(progress.downloaded_bytes)s|%(progress.total_bytes,progress.total_bytes_estimate)s|%(info.title)s",
        "--progress-template",
        "postprocess:__YTDLP_POSTPROCESS__%(progress.status)s|%(progress.postprocessor)s|%(info.title)s",
        "-o",
        YTDLP_OUTPUT_TEMPLATE,
        "-P",
        &request.output_dir,
    ]);
    command.args(extraction_args(
        &app,
        &request.mode,
        &request.quality,
        request.multi_item,
        request.cookies_browser.as_deref(),
        impersonation_target(is_threads, is_tiktok),
    )?);
    if request.multi_item {
        command.args([
            "--sleep-requests",
            "0.5",
            "--sleep-subtitles",
            "2",
            "--sleep-interval",
            "3",
            "--max-sleep-interval",
            "7",
        ]);
    }
    let ffmpeg_path = managed_ffmpeg_path(&app)?;
    command.args(["--ffmpeg-location", runtime.to_string_lossy().as_ref()]);

    command.args([
        "--print",
        "after_move:__YTDLP_FILE__%(.{filepath,duration,height,fps,vcodec,acodec,webpage_url,title,thumbnail,uploader,extractor,baldojnyi_photo})j",
    ]);
    if request.mode != "audio" {
        command.args(["--merge-output-format", "mkv"]);
    }
    if request.subtitles {
        command.args(["--write-subs", "--write-auto-subs", "--sub-langs", "uk,en"]);
    }
    let job_started = SystemTime::now();
    let job_temp_dir = create_download_temp_dir(&output_dir, &id)?;
    if let Err(error) = recovery::record(&app, &id, &job_temp_dir) {
        cleanup_download_temp_dir(&job_temp_dir);
        return Err(format!(
            "Не вдалося підготувати аварійне відновлення: {error}"
        ));
    }
    command.args(["-P", &format!("temp:{}", job_temp_dir.to_string_lossy())]);
    if selected_source.is_some() {
        command.args(["--referer", parsed.as_str()]);
    }
    command.arg(
        selected_source
            .as_ref()
            .map(|url| format!("baldoj-source:{url}"))
            .unwrap_or_else(|| parsed.to_string()),
    );

    let retry_program: OsString = command.get_program().to_os_string();
    let retry_args: Vec<OsString> = command.get_args().map(OsString::from).collect();
    let tiktok_retry_args = replace_option_value(&retry_args, "--impersonate", "chrome");
    let downloaded_files = Arc::new(Mutex::new(Vec::<DownloadedMedia>::new()));
    let storage_tracker = StorageTracker {
        output_dir,
        estimates: Arc::new(Mutex::new(HashMap::new())),
        video: request.mode != "audio",
        audio_format: request.audio_format.clone(),
    };
    let last_error = Arc::new(Mutex::new(None::<String>));
    let photos_skipped = Arc::new(AtomicBool::new(false));
    let output_reader_state = OutputReaderState {
        downloaded_files: Some(downloaded_files.clone()),
        storage_tracker: Some(storage_tracker.clone()),
        last_error: last_error.clone(),
        auth_retry,
        photos_skipped: photos_skipped.clone(),
    };
    let first_attempt = spawn_job_attempt(
        &app,
        manager.inner(),
        &id,
        &retry_program,
        &retry_args,
        &output_reader_state,
    );
    let (child, stdout_reader, stderr_reader) = match first_attempt {
        Ok(attempt) => attempt,
        Err(error) => {
            cleanup_download_temp_dir(&job_temp_dir);
            recovery::finish(&app, &id, &job_temp_dir);
            return Err(format!("Не вдалося запустити yt-dlp: {error}"));
        }
    };

    let manager = manager.inner().clone();
    let monitor_app = app.clone();
    let monitor_id = id.clone();
    let monitored_output_dir = storage_tracker.output_dir.clone();
    let output_format = if request.mode == "audio" {
        request.audio_format.clone()
    } else {
        "mp4".into()
    };
    thread::spawn(move || {
        let mut active_child = child;
        let mut stdout_reader = stdout_reader;
        let mut stderr_reader = stderr_reader;
        let mut retry_attempt = 0_u8;
        let mut tiktok_retry_used = false;
        let mut discovery_retry_used = selected_source.is_some();
        let mut discovery_started: Option<Instant> = None;
        let mut last_disk_check = Instant::now();
        let mut low_disk = false;
        let mut destination_unavailable = false;
        loop {
            if discovery_started.is_some_and(|start| start.elapsed() > Duration::from_secs(25)) {
                discovery_started = None;
                if let Ok(mut error) = last_error.lock() {
                    *error = Some("Automatic discovery found no media before timeout".into());
                }
                let _ = stop_child_process_group(&active_child);
            }
            if !low_disk && last_disk_check.elapsed() >= Duration::from_secs(1) {
                last_disk_check = Instant::now();
                let guard = disk_guard(&monitored_output_dir);
                if guard != DiskGuard::Ok {
                    // Both stop the job the same way; only the explanation differs.
                    low_disk = true;
                    destination_unavailable = guard == DiskGuard::Unavailable;
                    if let Ok(mut error) = last_error.lock() {
                        *error = Some(if destination_unavailable {
                            DESTINATION_UNAVAILABLE_MESSAGE.into()
                        } else {
                            "Недостатньо вільного місця. Процес зупинено, щоб зберегти резерв 500 МіБ.".into()
                        });
                    }
                    let _ = stop_child_process_group(&active_child);
                }
            }
            let result = active_child
                .lock()
                .ok()
                .and_then(|mut child| child.try_wait().ok())
                .flatten();
            if let Some(status) = result {
                if let Some(reader) = stdout_reader.take() {
                    let _ = reader.join();
                }
                if let Some(reader) = stderr_reader.take() {
                    let _ = reader.join();
                }
                let mut cancelled = download_was_cancelled(&manager, &monitor_id);
                let failure_message = last_error.lock().ok().and_then(|error| error.clone());
                let failed_with_403 = !status.success()
                    && !low_disk
                    && failure_message.as_deref().is_some_and(is_forbidden_error);
                let tiktok_retry_ready = is_tiktok
                    && failed_with_403
                    && !cancelled
                    && !tiktok_retry_used
                    && prepare_retry_dir(&job_temp_dir, &last_error);
                if tiktok_retry_ready {
                    tiktok_retry_used = true;
                    manager
                        .auth_required
                        .lock()
                        .ok()
                        .map(|mut jobs| jobs.remove(&monitor_id));
                    emit_retrying(
                        &monitor_app,
                        &monitor_id,
                        "TikTok відхилив запит. Перемикаємо профіль з’єднання й повторюємо…",
                    );
                    if let Ok(mut error) = last_error.lock() {
                        *error = None;
                    }
                    match spawn_job_attempt(
                        &monitor_app,
                        &manager,
                        &monitor_id,
                        &retry_program,
                        &tiktok_retry_args,
                        &output_reader_state,
                    ) {
                        Ok(attempt) => {
                            (active_child, stdout_reader, stderr_reader) = attempt;
                            continue;
                        }
                        Err(error) => {
                            if let Ok(mut last_error) = last_error.lock() {
                                *last_error =
                                    Some(format!("Не вдалося повторно запустити yt-dlp: {error}"));
                            }
                        }
                    }
                }
                let youtube_retry_ready = is_youtube
                    && failed_with_403
                    && !cancelled
                    && retry_attempt < 2
                    && prepare_retry_dir(&job_temp_dir, &last_error);
                if youtube_retry_ready {
                    retry_attempt += 1;
                    emit_retrying(
                        &monitor_app,
                        &monitor_id,
                        youtube_retry_message(retry_attempt),
                    );
                    if let Ok(mut error) = last_error.lock() {
                        *error = None;
                    }
                    // Re-run extraction from scratch and let the installed yt-dlp
                    // choose its current YouTube clients: hard-coded ones age and
                    // can turn a recoverable expired stream URL into a lasting 403.
                    match spawn_job_attempt(
                        &monitor_app,
                        &manager,
                        &monitor_id,
                        &retry_program,
                        &retry_args,
                        &output_reader_state,
                    ) {
                        Ok(attempt) => {
                            (active_child, stdout_reader, stderr_reader) = attempt;
                            continue;
                        }
                        Err(error) => {
                            if let Ok(mut last_error) = last_error.lock() {
                                *last_error =
                                    Some(format!("Не вдалося повторно запустити yt-dlp: {error}"));
                            }
                        }
                    }
                }
                if !status.success()
                    && !cancelled
                    && !low_disk
                    && !discovery_retry_used
                    && downloaded_files.lock().is_ok_and(|files| files.is_empty())
                    && failure_message
                        .as_deref()
                        .is_some_and(discovery::should_retry)
                {
                    discovery_retry_used = true;
                    discovery_started = Some(Instant::now());
                    let notice =
                        "Стандартний спосіб не знайшов відео. Шукаємо джерело на сторінці…";
                    emit_retrying(&monitor_app, &monitor_id, notice);
                    diagnostics::record(&monitor_id, notice);
                    let mut args = retry_args.clone();
                    if let Some(url) = args.last_mut() {
                        *url = format!("baldoj-discover:{}", parsed.as_str()).into();
                    }
                    args.extend(
                        ["--socket-timeout", "10", "--extractor-retries", "0"].map(OsString::from),
                    );
                    if let Ok(attempt) = spawn_job_attempt(
                        &monitor_app,
                        &manager,
                        &monitor_id,
                        &retry_program,
                        &args,
                        &output_reader_state,
                    ) {
                        (active_child, stdout_reader, stderr_reader) = attempt;
                        continue;
                    }
                }
                // A finished process proves that no sign-in was needed after all.
                let auth_required = manager
                    .auth_required
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id))
                    .unwrap_or(false)
                    && !status.success();
                let mut conversion_error = None;
                let mut completed_outputs = Vec::new();
                {
                    let mut files = downloaded_files
                        .lock()
                        .ok()
                        .map(|files| files.clone())
                        .unwrap_or_default();
                    if selected_source.is_some() {
                        for file in &mut files {
                            file.identity.source_url = Some(parsed.to_string());
                        }
                    }
                    if files.is_empty()
                        && status.success()
                        && !cancelled
                        && !low_disk
                        && !auth_required
                    {
                        conversion_error = Some(
                            "yt-dlp завершився, але не повідомив шлях до завантаженого файла"
                                .into(),
                        );
                    } else if !files.is_empty() {
                        for (index, file) in files.iter().enumerate() {
                            let requires_conversion =
                                match requires_app_conversion(file, &output_format, &ffmpeg_path) {
                                    Ok(required) => required,
                                    Err(error) => {
                                        diagnostics::record(&monitor_id, &error);
                                        conversion_error.get_or_insert(error);
                                        continue;
                                    }
                                };
                            if !requires_conversion {
                                let size = fs::metadata(&file.path)
                                    .map(|metadata| metadata.len())
                                    .unwrap_or(0);
                                completed_outputs.push(DownloadOutput {
                                    identity: file.identity.clone(),
                                    path: file.path.to_string_lossy().into_owned(),
                                    size,
                                });
                                continue;
                            }
                            // Keep already-ready playlist outputs, but never start new
                            // processing after an interrupted/auth/storage failure.
                            if cancelled
                                || low_disk
                                || auth_required
                                || download_was_cancelled(&manager, &monitor_id)
                            {
                                continue;
                            }
                            match convert_downloaded_media(
                                &monitor_app,
                                &manager,
                                &monitor_id,
                                &ffmpeg_path,
                                file,
                                (index, files.len()),
                                &output_format,
                                source_created_by_job(&file.path, job_started),
                            ) {
                                Ok(output) => {
                                    let size = fs::metadata(&output)
                                        .map(|metadata| metadata.len())
                                        .unwrap_or(0);
                                    completed_outputs.push(DownloadOutput {
                                        identity: file.identity.clone(),
                                        path: output.to_string_lossy().into_owned(),
                                        size,
                                    });
                                }
                                Err(error) => {
                                    eprintln!(
                                        "Не вдалося конвертувати {}: {error}",
                                        file.path.display()
                                    );
                                    if conversion_error.is_none() {
                                        conversion_error = Some(error);
                                    }
                                }
                            }
                        }
                    }
                    cancelled = download_was_cancelled(&manager, &monitor_id);
                }
                cleanup_download_temp_dir(&job_temp_dir);
                recovery::finish(&monitor_app, &monitor_id, &job_temp_dir);
                cancelled |= job_process::finish(&manager, &monitor_id);
                // A write error from yt-dlp or FFmpeg can arrive before the
                // next disk check; a vanished folder explains it either way.
                if !cancelled
                    && (!status.success() || conversion_error.is_some())
                    && !destination_present(&monitored_output_dir)
                {
                    low_disk = true;
                    destination_unavailable = true;
                }
                let kind = terminal_kind(
                    cancelled,
                    auth_required,
                    low_disk,
                    status.success(),
                    conversion_error.is_some(),
                );
                let raw_error = if destination_unavailable {
                    Some(DESTINATION_UNAVAILABLE_MESSAGE.to_string())
                } else {
                    conversion_error.clone()
                }
                .or_else(|| {
                    if low_disk {
                        return Some(
                            "Недостатньо вільного місця. Процес зупинено, щоб захистити диск."
                                .into(),
                        );
                    }
                    (!status.success())
                        .then(|| last_error.lock().ok().and_then(|error| error.clone()))
                        .flatten()
                });
                if let Some(error) = raw_error.as_deref() {
                    diagnostics::record(&monitor_id, error);
                }
                let message = if destination_unavailable {
                    raw_error.clone()
                } else if conversion_error.is_some() {
                    raw_error.as_deref().map(friendly_conversion_error)
                } else if low_disk {
                    raw_error.clone()
                } else if kind == "completed" {
                    completion_message(
                        completed_outputs.len(),
                        photos_skipped.load(Ordering::Relaxed),
                    )
                } else {
                    raw_error.as_deref().map(friendly_download_error)
                };
                let error_code = match kind {
                    "completed" => None,
                    "cancelled" => Some("cancelled".into()),
                    "auth_required" => Some("auth_required".into()),
                    _ if destination_unavailable => Some("destination_unavailable".into()),
                    _ if low_disk => Some("low_disk".into()),
                    _ if conversion_error.is_some() => {
                        Some(conversion_error_code(raw_error.as_deref().unwrap_or_default()).into())
                    }
                    _ => Some(download_error_code(raw_error.as_deref().unwrap_or_default()).into()),
                };
                let _ = monitor_app.emit(
                    "download-event",
                    DownloadEvent {
                        id: monitor_id,
                        kind: kind.into(),
                        percent: (kind == "completed").then_some(100.0),
                        message,
                        outputs: (!completed_outputs.is_empty()).then_some(completed_outputs),
                        error_code,
                        ..Default::default()
                    },
                );
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
    });

    Ok(())
}

#[tauri::command]
async fn cancel_download(manager: State<'_, DownloadManager>, id: String) -> Result<(), String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || cancel_download_inner(manager, id))
        .await
        .map_err(|e| e.to_string())?
}
fn cancel_download_inner(manager: DownloadManager, id: String) -> Result<(), String> {
    let result = job_process::cancel(&manager, &id);
    process_runner::stop_all();
    result
}

#[tauri::command]
fn play_completion_sound() {
    thread::spawn(|| {
        #[cfg(target_os = "macos")]
        {
            let _ = Command::new("/usr/bin/afplay")
                .args(["-v", "0.28", "/System/Library/Sounds/Ping.aiff"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }

        #[cfg(target_os = "windows")]
        {
            let mut command = Command::new("powershell.exe");
            configure_command(&mut command);
            let _ = command
                .args([
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "[System.Media.SystemSounds]::Asterisk.Play()",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    });
}

#[tauri::command]
fn request_user_attention(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Не вдалося знайти головне вікно".to_string())?;
    window
        .show()
        .map_err(|error| format!("Не вдалося показати вікно: {error}"))?;
    if window.is_minimized().unwrap_or(false) {
        let _ = window.unminimize();
    }
    let _ = window.request_user_attention(Some(tauri::UserAttentionType::Critical));
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(900));
        let _ = window.set_always_on_top(false);
    });
    Ok(())
}

fn update_sleep_prevention(state: &SleepPrevention, enabled: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let mut process = state
            .process
            .lock()
            .map_err(|_| "Не вдалося змінити захист від сну".to_string())?;
        if enabled {
            if process.is_none() {
                let child = Command::new("/usr/bin/caffeinate")
                    .args(["-i", "-w", &std::process::id().to_string()])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|error| format!("Не вдалося ввімкнути захист від сну: {error}"))?;
                *process = Some(child);
            }
        } else if let Some(mut child) = process.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        let mut stop = state
            .stop
            .lock()
            .map_err(|_| "Не вдалося змінити захист від сну".to_string())?;
        if enabled {
            if stop.is_none() {
                let (sender, receiver) = std::sync::mpsc::channel();
                thread::spawn(move || {
                    // SAFETY: SetThreadExecutionState is process-local, takes only documented
                    // flag bits, and is cleared on the same dedicated thread before it exits.
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED);
                    }
                    let _ = receiver.recv();
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                });
                *stop = Some(sender);
            }
        } else if let Some(sender) = stop.take() {
            let _ = sender.send(());
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = state;
        let _ = enabled;
        Err("Захист від сну для цієї платформи не підтримується".into())
    }
}

#[tauri::command]
fn set_queue_sleep_prevention(
    state: State<'_, SleepPrevention>,
    enabled: bool,
) -> Result<(), String> {
    update_sleep_prevention(state.inner(), enabled)
}

fn stop_all_downloads(manager: &DownloadManager) {
    let _handoff = manager.handoff.lock().unwrap_or_else(|e| e.into_inner());
    let active = manager
        .jobs
        .lock()
        .ok()
        .map(|mut jobs| jobs.drain().collect::<Vec<_>>())
        .unwrap_or_default();
    if let Ok(mut cancelled) = manager.cancelled.lock() {
        cancelled.extend(active.iter().map(|(id, _)| id.clone()));
    }
    for (_, child) in active {
        let _ = stop_child_process_group(&child);
    }
}

fn cleanup_background_work(app: &AppHandle) {
    process_runner::stop_all();
    stop_all_downloads(app.state::<DownloadManager>().inner());
    let _ = update_sleep_prevention(app.state::<SleepPrevention>().inner(), false);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    #[cfg(debug_assertions)]
    if browser_smoke::requested() {
        browser_smoke::run(context);
        return;
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(DownloadManager::default())
        .manage(HistoryThumbnailCache::default())
        .manage(RuntimeMaintenance::default())
        .manage(discovery::DiscoveryState::default())
        .manage(SleepPrevention::default())
        .manage(AppExitConfirmation::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            runtime_check::runtime_update_available,
            recovery::interrupted_work,
            recovery::remove_interrupted_work,
            diagnostics::download_diagnostics,
            discovery::open_media_browser,
            discovery::select_media_candidate,
            discovery::dismiss_media_candidates,
            runtime::runtime_status,
            runtime::install_ytdlp,
            runtime::install_ffmpeg,
            runtime::install_deno,
            runtime::update_ytdlp,
            vpn::check_vpn,
            folder_free_space,
            preflight_download,
            start_download,
            cancel_download,
            play_completion_sound,
            request_user_attention,
            set_queue_sleep_prevention,
            take_app_close_request,
            dismiss_app_close_request,
            allow_app_exit_once,
            cancel_app_exit_approval,
            confirm_app_close,
            inspect_history_files,
            thumbnails::cache_history_thumbnail,
            thumbnails::clear_history_thumbnail_cache,
            thumbnails::delete_history_thumbnail
        ])
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                request_app_close_confirmation(window.app_handle());
            }
        })
        .build(context)
        .expect("error while running tauri application");
    app.run(|app_handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            let confirmation = app_handle.state::<AppExitConfirmation>();
            if confirmation.approved.swap(false, Ordering::SeqCst) {
                cleanup_background_work(app_handle);
            } else {
                api.prevent_exit();
                request_app_close_confirmation(app_handle);
            }
        }
        tauri::RunEvent::Exit => cleanup_background_work(app_handle),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_retry_messages_follow_the_attempt() {
        assert_eq!(
            youtube_retry_message(1),
            "YouTube перервав потік. Оновлюємо посилання й продовжуємо…"
        );
        assert_eq!(
            youtube_retry_message(2),
            "YouTube знову перервав потік. Отримуємо ще одне свіже посилання…"
        );
    }

    #[test]
    fn verifies_runtime_manifest_with_the_real_updater_public_key() {
        let signature = b"dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTdHFlblU0elMzUEZzRFNkS21tNGJOMWp4bDRxQ0VwUTFRYjVwZmQ2ZjJUQlVyUnVPQUFkWHVQMFNBMmJzOUN5ZUNNaUl4SmErdXllUXNPTkFXOUdWQThBUVFWSk9HZ2djPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg2MjU3NTQ1CWZpbGU6eXQtZGxwLWJkLXJ1bnRpbWUtc2lnbmF0dXJlLWZpeHR1cmUKbFFOTGRNKy9FUWJzUGJ5WlhOR29BbE13OW5mOEd6Tm5DZUE4My83WWtmdlM0a1RJbGJ6SmpicjZWMU42WTR6cDdQelM3enRMVms3cytOZDNKVXRvREE9PQo=";
        assert!(verify_runtime_manifest_signature(b"runtime-manifest-test-v1", signature).is_ok());
        assert!(verify_runtime_manifest_signature(b"runtime-manifest-test-v2", signature).is_err());
    }

    #[test]
    fn runtime_manifest_rollback_protection_is_monotonic_and_allows_clean_install() {
        let accepted = RuntimeManifestState {
            release_version: "0.6.1.0".into(),
            generated_at: "2026-08-09T08:00:00.000Z".into(),
        };
        assert!(ensure_runtime_manifest_is_current(&accepted, None).is_ok());
        assert!(ensure_runtime_manifest_is_current(&accepted, Some(&accepted)).is_ok());

        let newer_generation = RuntimeManifestState {
            generated_at: "2026-08-09T09:00:00.000Z".into(),
            ..accepted.clone()
        };
        assert!(ensure_runtime_manifest_is_current(&newer_generation, Some(&accepted)).is_ok());

        let newer_release = RuntimeManifestState {
            release_version: "0.6.2.0".into(),
            generated_at: "2026-08-08T00:00:00.000Z".into(),
        };
        assert!(ensure_runtime_manifest_is_current(&newer_release, Some(&accepted)).is_ok());

        let older_generation = RuntimeManifestState {
            generated_at: "2026-08-09T07:59:59.999Z".into(),
            ..accepted.clone()
        };
        assert!(ensure_runtime_manifest_is_current(&older_generation, Some(&accepted)).is_err());

        let older_release = RuntimeManifestState {
            release_version: "0.6.0.0".into(),
            generated_at: "2026-08-10T00:00:00.000Z".into(),
        };
        assert!(ensure_runtime_manifest_is_current(&older_release, Some(&accepted)).is_err());

        let malformed = RuntimeManifestState {
            release_version: "0.6.2".into(),
            generated_at: "tomorrow".into(),
        };
        assert!(ensure_runtime_manifest_is_current(&malformed, Some(&accepted)).is_err());
    }

    #[test]
    fn parses_structured_windows_download_handoff() {
        let payload = serde_json::json!({
            "filepath": r"C:\Users\Влад\Downloads\AGE OF KINGDOMS [x-2wmD5Pc1E].mkv",
            "duration": 5470.0,
            "height": 2160,
            "fps": 24.0
        })
        .to_string();
        let media = downloaded_media_from_print(&payload).unwrap();

        assert_eq!(
            media.path,
            PathBuf::from(r"C:\Users\Влад\Downloads\AGE OF KINGDOMS [x-2wmD5Pc1E].mkv")
        );
        assert_eq!(media.duration, Some(5470.0));
        assert_eq!(media.height, Some(2160));
        assert_eq!(media.fps, Some(24.0));
        assert_eq!(media.video_codec, None);
        assert_eq!(media.audio_codec, None);
    }

    #[test]
    fn download_handoff_keeps_raw_path_fallback_and_rejects_bad_duration() {
        let raw = downloaded_media_from_print("/tmp/video.mkv").unwrap();
        assert_eq!(raw.path, PathBuf::from("/tmp/video.mkv"));
        assert_eq!(raw.duration, None);

        let payload = r#"{"filepath":"video.mkv","duration":"N/A","height":0,"fps":null}"#;
        let media = downloaded_media_from_print(payload).unwrap();
        assert_eq!(media.path, PathBuf::from("video.mkv"));
        assert_eq!(media.duration, None);
        assert_eq!(media.height, None);
        assert_eq!(media.fps, None);
        assert_eq!(media.video_codec, None);
        assert_eq!(media.audio_codec, None);
    }

    #[test]
    fn instagram_photo_is_last_resort_in_every_video_selector_only() {
        assert!(format_selector("video").ends_with(&format!("/{INSTAGRAM_PHOTO_FORMAT_ID}")));
        assert!(!format_selector("audio").contains(INSTAGRAM_PHOTO_FORMAT_ID));
        assert_eq!(
            photo_extractor_args("audio"),
            Some(["--extractor-args", "instagram:baldojnyi_photos=skip"])
        );
        assert_eq!(photo_extractor_args("video"), None);
    }

    #[test]
    fn instagram_plugin_markers_match_app_contract() {
        for marker in [
            INSTAGRAM_PHOTO_FORMAT_ID,
            INSTAGRAM_PHOTO_ONLY_MARKER,
            INSTAGRAM_PHOTOS_SKIPPED_MARKER,
            "baldojnyi_photos",
            "PHOTO_FIELD = 'baldojnyi_photo'",
            "plugin_name='baldojnyi'",
        ] {
            assert!(
                INSTAGRAM_PHOTOS_SOURCE.contains(marker),
                "plugin is missing {marker}"
            );
        }
        let error = format!(
            "[Instagram] DeRf1RKjLc7: {INSTAGRAM_PHOTO_ONLY_MARKER}: this Instagram post contains only photos"
        );
        assert_eq!(download_error_code(&error), "photo_only");
        assert!(friendly_download_error(&error).contains("лише фото"));
        assert!(!friendly_download_error(&error).contains(INSTAGRAM_PHOTO_ONLY_MARKER));
        assert!(!discovery::should_retry(&error));
        assert!(is_expected_photo_warning(
            "WARNING: Only images are available for download. use --list-formats to see them"
        ));
        assert!(!is_expected_photo_warning(
            "WARNING: This video is drm protected and only images are available for download. use --list-formats to see them"
        ));
        assert!(!is_expected_photo_warning(
            "ERROR: only images are available for download"
        ));
    }

    #[test]
    fn download_handoff_marks_only_plugin_photos() {
        let photo = downloaded_media_from_print(
            r#"{"filepath":"Photo by a [1].jpg","vcodec":"none","acodec":"none","baldojnyi_photo":true}"#,
        )
        .unwrap();
        assert!(photo.photo);
        for payload in [
            r#"{"filepath":"clip.mp4"}"#,
            r#"{"filepath":"clip.mp4","baldojnyi_photo":false}"#,
            r#"{"filepath":"clip.mp4","baldojnyi_photo":"skipped"}"#,
            r#"{"filepath":"clip.mp4","baldojnyi_photo":"NA"}"#,
        ] {
            assert!(
                !downloaded_media_from_print(payload).unwrap().photo,
                "{payload}"
            );
        }
        assert!(!downloaded_media_from_print("/tmp/raw.jpg").unwrap().photo);
    }

    #[test]
    fn photos_are_kept_as_downloaded_but_flag_never_bypasses_video() {
        let ffmpeg_path = Path::new("/definitely/missing/ffmpeg");
        let photo = DownloadedMedia {
            identity: MediaIdentity::default(),
            path: PathBuf::from("Photo by author [1].JPG"),
            duration: None,
            height: Some(1350),
            fps: None,
            video_codec: Some("none".into()),
            audio_codec: Some("none".into()),
            photo: true,
        };
        for output_format in ["mp4", "mp3", "m4a", "opus", "wav"] {
            assert!(!requires_app_conversion(&photo, output_format, ffmpeg_path).unwrap());
        }
        let disguised = DownloadedMedia {
            path: PathBuf::from("clip.mkv"),
            ..photo.clone()
        };
        assert!(!is_downloaded_photo(&disguised));
        assert!(requires_app_conversion(&disguised, "mp4", ffmpeg_path).unwrap());
        assert!(requires_app_conversion(&disguised, "mp3", ffmpeg_path).unwrap());
        let unflagged = DownloadedMedia {
            photo: false,
            ..photo
        };
        assert!(!is_downloaded_photo(&unflagged));
    }

    #[test]
    fn download_handoff_reports_selected_codecs() {
        let direct = downloaded_media_from_print(
            r#"{"filepath":"clip.mp4","duration":12,"vcodec":"avc1.640028","acodec":"mp4a.40.2"}"#,
        )
        .unwrap();
        assert_eq!(direct.video_codec.as_deref(), Some("avc1.640028"));
        assert_eq!(direct.audio_codec.as_deref(), Some("mp4a.40.2"));
    }

    #[test]
    fn reports_available_space_for_existing_directory() {
        assert!(available_disk_space(&std::env::temp_dir()).is_some());
        assert!(planning_available_disk_space(&std::env::temp_dir()).is_some());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_planning_capacity_selects_the_larger_available_source() {
        assert_eq!(
            select_macos_planning_capacity(Some(200), Some(100)),
            Some(200)
        );
        assert_eq!(
            select_macos_planning_capacity(Some(100), Some(200)),
            Some(200)
        );
        assert_eq!(select_macos_planning_capacity(Some(100), None), Some(100));
        assert_eq!(select_macos_planning_capacity(None, Some(200)), Some(200));
        assert_eq!(select_macos_planning_capacity(None, None), None);
    }

    #[test]
    fn history_file_status_tracks_existing_and_missing_files() {
        let fixture_id = Uuid::new_v4();
        let existing = std::env::temp_dir().join(format!("yt-dlp-bd-{fixture_id}.mp4"));
        fs::write(&existing, b"video").expect("history fixture");
        let missing = std::env::temp_dir().join(format!("yt-dlp-bd-{fixture_id}-missing.mp4"));
        let statuses = inspect_history_files_inner(vec![
            existing.to_string_lossy().into_owned(),
            missing.to_string_lossy().into_owned(),
        ]);
        assert!(statuses[0].available);
        assert_eq!(statuses[0].size, Some(5));
        assert!(!statuses[1].available);
        assert_eq!(statuses[1].size, None);
        fs::remove_file(existing).expect("remove history fixture");
    }

    #[test]
    fn thumbnail_cache_accepts_only_safe_raster_formats() {
        assert_eq!(
            thumbnail_extension("image/jpeg; charset=binary"),
            Some("jpg")
        );
        assert_eq!(thumbnail_extension("IMAGE/WEBP"), Some("webp"));
        assert_eq!(thumbnail_extension("image/svg+xml"), None);
        assert_eq!(thumbnail_extension("text/html"), None);
    }

    #[test]
    fn thumbnail_cache_rejects_private_and_special_network_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.4",
            "169.254.1.1",
            "192.168.1.4",
            "100.64.0.1",
            "::1",
            "fe80::1",
            "fd00::1",
            "2001:db8::1",
        ] {
            assert!(
                !is_public_thumbnail_ip(address.parse().unwrap()),
                "{address}"
            );
        }
        assert!(is_public_thumbnail_ip("1.1.1.1".parse().unwrap()));
        assert!(is_public_thumbnail_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));
    }

    #[test]
    fn accepts_only_official_stable_or_nightly_yt_dlp_runtime_assets() {
        let asset = |url: &str| RuntimeAssetManifest {
            version: "2026.08.17.073947".into(),
            url: url.into(),
            sha256: "a".repeat(64),
        };
        assert!(require_yt_dlp_asset_source(&asset(
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.08.17/yt-dlp_macos"
        ))
        .is_ok());
        assert!(require_yt_dlp_asset_source(&asset(
            "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/download/2026.08.17.073947/yt-dlp_macos"
        ))
        .is_ok());
        assert!(require_yt_dlp_asset_source(&asset(
            "https://github.com/example/yt-dlp-nightly-builds/releases/download/2026.08.17/yt-dlp_macos"
        ))
        .is_err());
    }

    #[test]
    fn nightly_updater_accepts_only_exact_official_release_assets() {
        let release = GithubRelease {
            tag_name: "2026.08.17.073947".into(),
            assets: vec![GithubReleaseAsset {
                name: "yt-dlp_macos".into(),
                browser_download_url: "https://github.com/yt-dlp/yt-dlp-nightly-builds/releases/download/2026.08.17.073947/yt-dlp_macos".into(),
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                size: 10_000_000,
            }],
        };
        assert!(select_nightly_asset(&release, "yt-dlp_macos").is_ok());

        let mut wrong_host = release;
        wrong_host.assets[0].browser_download_url = "https://example.com/yt-dlp_macos".into();
        assert!(select_nightly_asset(&wrong_host, "yt-dlp_macos").is_err());
    }

    #[test]
    fn nightly_updater_requires_one_strict_matching_sha256() {
        let expected = "a".repeat(64);
        let checksums = format!(
            "{}  yt-dlp_macos\n{}  yt-dlp.exe\n",
            expected,
            "b".repeat(64)
        );
        assert_eq!(
            nightly_checksum_for_asset(checksums.as_bytes(), "yt-dlp_macos").unwrap(),
            expected
        );
        assert!(nightly_checksum_for_asset(checksums.as_bytes(), "missing").is_err());
        let duplicate = format!(
            "{}  yt-dlp_macos\n{}  yt-dlp_macos\n",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert!(nightly_checksum_for_asset(duplicate.as_bytes(), "yt-dlp_macos").is_err());
    }

    #[test]
    fn nightly_versions_are_numeric_and_comparable() {
        assert_eq!(
            parsed_yt_dlp_version("2026.08.17.073947"),
            Some(vec![2026, 8, 17, 73947])
        );
        assert!(parsed_yt_dlp_version("nightly").is_none());
        assert!(
            parsed_yt_dlp_version("2026.08.18.000001") > parsed_yt_dlp_version("2026.08.17.235959")
        );
        assert!(
            ensure_ytdlp_nightly_not_rollback("2026.08.18.000001", Some("2026.08.17.235959"))
                .is_ok()
        );
        assert!(
            ensure_ytdlp_nightly_not_rollback("2026.08.16.000001", Some("2026.08.17.235959"))
                .is_err()
        );
        assert!(ensure_ytdlp_nightly_not_rollback("2026.08.18.000001", None).is_ok());
        assert!(ensure_ytdlp_bootstrap_not_rollback(
            "2026.08.17.000001",
            Some("2026.08.18.000001"),
            true,
        )
        .is_err());
        assert!(ensure_ytdlp_bootstrap_not_rollback(
            "2026.08.17.000001",
            Some("2026.08.18.000001"),
            false,
        )
        .is_ok());
        assert!(verified_nightly_is_current(
            "2026.08.18.000001",
            Some("2026.08.18.000001\n"),
            "abc123",
            Some("abc123\n"),
            Some("abc123"),
        ));
        assert!(!verified_nightly_is_current(
            "2026.08.18.000002",
            Some("2026.08.18.000001"),
            "abc123",
            Some("abc123"),
            Some("abc123"),
        ));
        assert!(!verified_nightly_is_current(
            "2026.08.18.000001",
            Some("2026.08.18.000001"),
            "abc123",
            Some("abc123"),
            Some("changed"),
        ));
    }

    #[test]
    fn runtime_download_limits_reject_oversize_and_overflow() {
        assert_eq!(checked_runtime_download_size(10, 5, Some(20), 30), Ok(15));
        assert!(checked_runtime_download_size(10, 11, Some(20), 30).is_err());
        assert!(checked_runtime_download_size(25, 6, None, 30).is_err());
        assert!(checked_runtime_download_size(u64::MAX, 1, None, u64::MAX).is_err());
    }

    #[test]
    fn runtime_file_lock_excludes_a_second_process_handle() {
        let directory =
            std::env::temp_dir().join(format!("yt-dlp-bd-runtime-lock-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("maintenance.lock");
        let first = try_lock_runtime_file(&path).unwrap();
        assert!(try_lock_runtime_file(&path).is_err());
        drop(first);
        let second = try_lock_runtime_file(&path).unwrap();
        drop(second);
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn failed_ytdlp_binary_replacement_keeps_old_binary_and_stamps_consistent() {
        let directory =
            std::env::temp_dir().join(format!("yt-dlp-bd-runtime-commit-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("yt-dlp");
        let missing_temporary = directory.join("missing-download");
        let version_stamp = directory.join(".yt-dlp.version");
        let hash_stamp = directory.join(".yt-dlp.sha256");
        fs::write(&destination, b"old-binary").unwrap();
        fs::write(&version_stamp, "2026.08.17.000001").unwrap();
        fs::write(&hash_stamp, "old-hash").unwrap();

        assert!(commit_ytdlp_runtime_update(
            &missing_temporary,
            &destination,
            &version_stamp,
            &hash_stamp,
            "2026.08.18.000001",
            "new-hash",
            "yt-dlp nightly",
        )
        .is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"old-binary");
        assert_eq!(
            fs::read_to_string(&version_stamp).unwrap(),
            "2026.08.17.000001"
        );
        assert_eq!(fs::read_to_string(&hash_stamp).unwrap(), "old-hash");

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn runtime_disk_guard_triggers_before_protected_reserve() {
        assert!(low_disk_guard_triggered(DISK_GUARD_TRIGGER));
        assert!(low_disk_guard_triggered(MIN_FREE_SPACE_RESERVE));
        assert!(!low_disk_guard_triggered(DISK_GUARD_TRIGGER + 1));
    }

    #[test]
    fn an_unplugged_drive_is_not_reported_as_a_full_disk() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("unplugged drive").join("Downloads");
        assert_eq!(disk_guard(&missing), DiskGuard::Unavailable);
        assert_ne!(disk_guard(directory.path()), DiskGuard::Unavailable);
        assert_eq!(
            download_error_code(DESTINATION_UNAVAILABLE_MESSAGE),
            "destination_unavailable"
        );
        assert_eq!(
            conversion_error_code(DESTINATION_UNAVAILABLE_MESSAGE),
            "destination_unavailable"
        );
        assert_eq!(
            friendly_conversion_error(DESTINATION_UNAVAILABLE_MESSAGE),
            DESTINATION_UNAVAILABLE_MESSAGE
        );
    }

    #[test]
    fn conversion_progress_and_eta_are_clamped() {
        assert_eq!(conversion_percent(-5.0, 100.0, 0, 1), 0.0);
        assert_eq!(conversion_percent(150.0, 100.0, 0, 1), 100.0);
        assert_eq!(conversion_percent(50.0, 100.0, 1, 2), 75.0);
        assert_eq!(format_eta(65.0), "01:05");
        assert_eq!(format_eta(f64::NAN), "—");
    }

    #[test]
    fn windows_hardware_encoder_selection_is_deterministic() {
        assert_eq!(
            windows_encoder_candidates(),
            [
                VideoEncoder::Nvenc,
                VideoEncoder::QuickSync,
                VideoEncoder::Amf
            ]
        );
        let mut probed = Vec::new();
        let selected = first_available_encoder(&windows_encoder_candidates(), |encoder| {
            probed.push(encoder);
            encoder == VideoEncoder::QuickSync
        });
        assert_eq!(selected, VideoEncoder::QuickSync);
        assert_eq!(probed, vec![VideoEncoder::Nvenc, VideoEncoder::QuickSync]);
    }

    #[test]
    fn unavailable_hardware_selects_software_and_retry_never_loops() {
        let selected = first_available_encoder(&windows_encoder_candidates(), |_| false);
        assert_eq!(selected, VideoEncoder::Software);
        assert_eq!(
            conversion_attempt_encoders(VideoEncoder::Nvenc),
            vec![VideoEncoder::Nvenc, VideoEncoder::Software]
        );
        assert_eq!(
            conversion_attempt_encoders(VideoEncoder::Software),
            vec![VideoEncoder::Software]
        );
        assert_eq!(
            conversion_attempt_encoders(VideoEncoder::VideoToolbox),
            vec![VideoEncoder::VideoToolbox, VideoEncoder::Software]
        );
        assert_eq!(
            windows_conversion_attempt_encoders(None),
            vec![
                VideoEncoder::Nvenc,
                VideoEncoder::QuickSync,
                VideoEncoder::Amf,
                VideoEncoder::Software
            ]
        );
        assert_eq!(
            windows_conversion_attempt_encoders(Some(VideoEncoder::QuickSync)),
            vec![
                VideoEncoder::QuickSync,
                VideoEncoder::Nvenc,
                VideoEncoder::Amf,
                VideoEncoder::Software
            ]
        );
        assert_eq!(
            windows_conversion_attempt_encoders(Some(VideoEncoder::Software)),
            windows_conversion_attempt_encoders(None)
        );
    }

    #[test]
    fn encoder_selection_is_cached_until_the_ffmpeg_binary_changes() {
        use std::cell::Cell;

        let cache = Mutex::new(None);
        let path = Path::new("managed-ffmpeg");
        let first_version = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let second_version = SystemTime::UNIX_EPOCH + Duration::from_secs(2);
        let probes = Cell::new(0_u8);
        let select = || {
            probes.set(probes.get() + 1);
            VideoEncoder::Nvenc
        };

        assert_eq!(
            cached_video_encoder_selection(&cache, path, Some(first_version), select),
            VideoEncoder::Nvenc
        );
        assert_eq!(
            cached_video_encoder_selection(&cache, path, Some(first_version), select),
            VideoEncoder::Nvenc
        );
        assert_eq!(probes.get(), 1);
        assert_eq!(
            cached_video_encoder_selection(&cache, path, Some(second_version), select),
            VideoEncoder::Nvenc
        );
        assert_eq!(probes.get(), 2);
    }

    #[test]
    fn compatible_single_mp4_bypasses_conversion_independent_of_site() {
        let ffmpeg_path = Path::new("ffmpeg");
        let compatible = DownloadedMedia {
            identity: MediaIdentity::default(),
            path: PathBuf::from("ready.mp4"),
            duration: Some(10.0),
            height: Some(1080),
            fps: Some(30.0),
            video_codec: Some("avc1.640028".into()),
            audio_codec: Some("mp4a.40.2".into()),
            photo: false,
        };
        // Metadata alone cannot prove bit depth/HDR; a nonexistent file is not ready.
        assert!(requires_app_conversion(&compatible, "mp4", ffmpeg_path).is_err());
        assert!(requires_app_conversion(&compatible, "mp3", ffmpeg_path).unwrap());

        let incompatible_video = DownloadedMedia {
            video_codec: Some("vp9".into()),
            ..compatible.clone()
        };
        assert!(requires_app_conversion(&incompatible_video, "mp4", ffmpeg_path).unwrap());

        let merged_streams = DownloadedMedia {
            path: PathBuf::from("merged.mkv"),
            ..compatible.clone()
        };
        assert!(requires_app_conversion(&merged_streams, "mp4", ffmpeg_path).unwrap());
    }

    #[test]
    fn failed_hardware_cleanup_preserves_source_and_removes_only_temporary_mp4() {
        let directory =
            std::env::temp_dir().join(format!("yt-dlp-bd-encoder-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("merged-source.mkv");
        let temporary = directory.join(".converted.part.mp4");
        fs::write(&source, b"source").unwrap();
        fs::write(&temporary, b"partial output").unwrap();

        cleanup_failed_conversion(&temporary);

        assert!(source.exists());
        assert!(!temporary.exists());
        fs::remove_file(source).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn download_temp_cleanup_removes_only_the_job_owned_directory() {
        let root = std::env::temp_dir().join(format!("yt-dlp-bd-temp-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let kept = root.join("finished-video.mp4");
        fs::write(&kept, b"finished").unwrap();
        let id = Uuid::new_v4().to_string();
        let job_temp = create_download_temp_dir(&root, &id).unwrap();
        fs::write(job_temp.join("video.f137.part"), b"partial").unwrap();

        reset_download_temp_dir(&job_temp).unwrap();
        assert!(job_temp.is_dir());
        assert_eq!(fs::read_dir(&job_temp).unwrap().count(), 0);
        fs::write(job_temp.join("audio.f140.m4a"), b"partial").unwrap();
        cleanup_download_temp_dir(&job_temp);

        assert!(!job_temp.exists());
        assert!(kept.exists());
        fs::remove_file(kept).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn threads_plugin_install_is_scoped_and_idempotent() {
        let root = std::env::temp_dir().join(format!("yt-dlp-bd-plugin-test-{}", Uuid::new_v4()));
        let unrelated = root.join("keep.txt");
        fs::create_dir_all(&root).unwrap();
        fs::write(&unrelated, b"keep").unwrap();

        let installed_root = install_threads_plugin_at(&root).unwrap();
        let extractors = installed_root
            .join("baldojnyi")
            .join("yt_dlp_plugins")
            .join("extractor");
        let installed = extractors.join("threads_baldojnyi.py");
        assert_eq!(
            fs::read(&installed).unwrap(),
            THREADS_EXTRACTOR_SOURCE.as_bytes()
        );
        for (filename, source, _) in YT_DLP_PLUGIN_MODULES {
            assert_eq!(
                fs::read(extractors.join(filename)).unwrap(),
                source.as_bytes()
            );
        }

        fs::write(&installed, b"outdated").unwrap();
        let instagram = extractors.join("instagram_baldojnyi.py");
        fs::write(&instagram, b"outdated").unwrap();
        install_threads_plugin_at(&root).unwrap();
        assert_eq!(
            fs::read(&installed).unwrap(),
            THREADS_EXTRACTOR_SOURCE.as_bytes()
        );
        assert_eq!(
            fs::read(&instagram).unwrap(),
            INSTAGRAM_PHOTOS_SOURCE.as_bytes()
        );
        assert_eq!(fs::read(&unrelated).unwrap(), b"keep");
        let leftovers: Vec<_> = fs::read_dir(&extractors)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hardware_and_software_video_args_preserve_output_contract() {
        for encoder in [
            VideoEncoder::Nvenc,
            VideoEncoder::QuickSync,
            VideoEncoder::Amf,
            VideoEncoder::Software,
        ] {
            let args = video_encoding_args(encoder, 8_000_000, 8_800_000, 16_000_000);
            let joined = args.join(" ");
            assert!(joined.contains(&format!("-c:v {}", encoder.codec())));
            assert!(joined.contains("-map 0:v:0 -map 0:a:0? -map_metadata 0"));
            assert!(joined.contains("-b:v 8000000 -maxrate 8800000 -bufsize 16000000"));
            assert!(joined.contains("-pix_fmt yuv420p"));
            assert!(joined.contains("-c:a aac -profile:a aac_low -b:a 192k"));
            assert!(joined.contains("-tag:v avc1 -movflags +faststart"));
        }
    }

    #[test]
    fn encoder_probe_creates_a_real_frame() {
        for encoder in windows_encoder_candidates() {
            let args = encoder_probe_args(encoder).join(" ");
            assert!(args.contains("-f lavfi -i color=size=128x128:rate=30"));
            assert!(args.contains("-frames:v 1"));
            assert!(args.contains(&format!("-c:v {}", encoder.codec())));
            assert!(args.ends_with("-f null -"));
        }
    }

    #[test]
    fn selector_and_playlist_intent_are_explicit() {
        assert_eq!(
            format_selector("video"),
            "bestvideo+bestaudio/best/baldojnyi-photo"
        );
        assert_eq!(format_selector("audio"), "bestaudio/best");
        assert_eq!(quality_sort("video", "720"), Some("res:720"));
        assert_eq!(quality_sort("video", "best"), None);
        assert_eq!(quality_sort("audio", "720"), None);
        assert_eq!(
            impersonation_target(false, true),
            Some("Chrome-99:Android-12")
        );
        assert_eq!(impersonation_target(true, false), Some("chrome"));
        assert_eq!(impersonation_target(false, false), None);
        assert_eq!(playlist_flag(false), "--no-playlist");
        assert_eq!(playlist_flag(true), "--yes-playlist");
        assert_eq!(YTDLP_OUTPUT_TEMPLATE, "%(title).140B [%(id).40B].%(ext)s");
    }

    #[test]
    fn impersonation_can_be_replaced_for_tiktok_fallback() {
        let args = vec![
            OsString::from("--no-playlist"),
            OsString::from("--impersonate"),
            OsString::from("Chrome-99:Android-12"),
            OsString::from("https://www.tiktok.com/@user/video/123"),
        ];
        assert_eq!(
            replace_option_value(&args, "--impersonate", "chrome"),
            vec![
                OsString::from("--no-playlist"),
                OsString::from("--impersonate"),
                OsString::from("chrome"),
                OsString::from("https://www.tiktok.com/@user/video/123")
            ]
        );
    }

    #[test]
    fn canonical_tiktok_video_urls_drop_tracking_parameters() {
        let normalized = normalize_media_url(
            "https://www.tiktok.com/@the_best_president_ua/video/7667686431778688274/?is_from_webapp=1&sender_device=pc#share",
        )
        .unwrap();
        assert_eq!(
            normalized.as_str(),
            "https://www.tiktok.com/@the_best_president_ua/video/7667686431778688274"
        );
        assert_eq!(
            normalize_media_url("https://vm.tiktok.com/ZMexample/?share_app_id=123")
                .unwrap()
                .as_str(),
            "https://vm.tiktok.com/ZMexample/?share_app_id=123"
        );
    }

    #[test]
    fn friendly_errors_hide_raw_extractor_diagnostics() {
        assert!(
            friendly_download_error("[youtube:truncated_id] incomplete youtube id")
                .contains("неповне")
        );
        assert!(friendly_download_error("HTTP Error 403: Forbidden").contains("відхилив"));
        assert!(
            friendly_download_error("Operation not permitted: Safari/Cookies.binarycookies")
                .contains("macOS")
        );
        assert!(friendly_download_error("No space left on device").contains("вільного місця"));
        assert!(
            friendly_download_error("WARNING: failed to decrypt with DPAPI").contains("Firefox")
        );
        assert!(
            friendly_download_error("ERROR: Could not copy Chrome cookie database")
                .contains("закрийте")
        );
        assert!(
            friendly_download_error("ERROR: could not find chrome cookies database")
                .contains("не знайдено")
        );
        assert!(
            friendly_download_error("[youtube] example: This video is unavailable")
                .contains("недоступне")
        );
        let conversion = friendly_conversion_error(
            "/Users/name/Downloads/private.mkv: Invalid data found when processing input",
        );
        assert!(conversion.contains("пошкоджений"));
        assert!(!conversion.contains("/Users/name"));
    }

    #[test]
    fn retries_ambiguous_youtube_unavailable_with_browser_cookies_only_when_allowed() {
        assert_eq!(yt_dlp_output_encoding_args(), ["--encoding", "UTF-8"]);
        let unavailable = "ERROR: [youtube] example: This video is unavailable";

        assert!(looks_like_auth_error(unavailable, true, false));
        assert!(!looks_like_auth_error(unavailable, false, false));
        assert!(looks_like_auth_error(
            "ERROR: Sign in to confirm your age",
            false,
            false
        ));
        assert!(!should_request_browser_auth(
            "ERROR: Sign in to confirm your age",
            AuthRetryPolicy {
                allow_browser_retry: false,
                youtube_unavailable_may_need_cookies: false,
                forbidden_may_need_cookies: false,
            }
        ));
    }

    #[test]
    fn retries_tiktok_forbidden_with_browser_cookies_only_when_allowed() {
        let forbidden = "ERROR: [TikTok] HTTP Error 403: Forbidden";

        assert!(looks_like_auth_error(forbidden, false, true));
        assert!(!looks_like_auth_error(forbidden, false, false));
        assert!(should_request_browser_auth(
            forbidden,
            AuthRetryPolicy {
                allow_browser_retry: true,
                youtube_unavailable_may_need_cookies: false,
                forbidden_may_need_cookies: true,
            }
        ));
        assert!(!should_request_browser_auth(
            forbidden,
            AuthRetryPolicy {
                allow_browser_retry: false,
                youtube_unavailable_may_need_cookies: false,
                forbidden_may_need_cookies: true,
            }
        ));
    }

    #[test]
    fn titles_and_speeds_in_progress_never_look_like_a_sign_in_request() {
        let policy = AuthRetryPolicy {
            allow_browser_retry: true,
            youtube_unavailable_may_need_cookies: true,
            forbidden_may_need_cookies: true,
        };
        for line in [
            "__YTDLP_PROGRESS__ 52.0%|1.00MiB/s|00:03|NA|NA|How to log in to Gmail",
            "__YTDLP_PROGRESS__ 52.0%|403.21KiB/s|00:03|NA|NA|Cooking",
            "__YTDLP_PROGRESS__ 52.0%|1.00MiB/s|00:03|NA|NA|The forbidden city",
            "WARNING: [youtube] Private video list could not be read",
        ] {
            assert!(!should_request_browser_auth(line, policy), "{line}");
        }
        assert!(should_request_browser_auth(
            "ERROR: [youtube] abc: Sign in to confirm you’re not a bot",
            policy
        ));
    }

    #[test]
    fn a_successful_process_is_never_reported_as_failed_or_needing_sign_in() {
        assert_eq!(terminal_kind(false, false, false, true, false), "completed");
        assert_eq!(
            terminal_kind(false, true, false, false, false),
            "auth_required"
        );
        assert_eq!(terminal_kind(false, false, false, true, true), "failed");
        assert_eq!(terminal_kind(false, false, true, true, false), "failed");
        assert_eq!(terminal_kind(true, true, true, true, true), "cancelled");
    }

    #[test]
    fn collection_progress_covers_the_whole_carousel() {
        let now = Instant::now();
        let progress = parse_download_progress(
            " 50.0%|1.00MiB/s|00:03|3|10|1048576|2097152|Назва | із рискою",
            &mut progress_speed::SpeedSmoother::default(),
            now,
        );
        assert_eq!(progress.percent, Some(25.0));
        assert_eq!(progress.message.as_deref(), Some("Файл 3 із 10"));
        assert_eq!(progress.speed.as_deref(), Some("1.00MiB/s"));
        let single = parse_download_progress(
            " 50.0%|1.00MiB/s|00:03|NA|NA|NA|NA|Назва",
            &mut progress_speed::SpeedSmoother::default(),
            now,
        );
        assert_eq!(single.percent, Some(50.0));
        assert_eq!(single.message, None);
    }

    #[test]
    fn completion_message_counts_files_in_ukrainian() {
        for (count, expected) in [
            (2, "2 файли"),
            (5, "5 файлів"),
            (11, "11 файлів"),
            (21, "21 файл"),
            (22, "22 файли"),
        ] {
            assert!(saved_files_label(count).contains(expected), "{count}");
        }
        assert_eq!(completion_message(1, false), None);
        assert_eq!(
            completion_message(3, true).as_deref(),
            Some(PHOTOS_SKIPPED_NOTICE)
        );
    }

    #[test]
    fn only_a_file_created_during_the_job_counts_as_its_own() {
        let directory = tempfile::tempdir().unwrap();
        let earlier = directory.path().join("kept from before.mp4");
        fs::write(&earlier, b"user file").unwrap();
        thread::sleep(Duration::from_millis(20));
        let job_started = SystemTime::now();
        thread::sleep(Duration::from_millis(20));
        let created = directory.path().join("from this job.mp4");
        fs::write(&created, b"new file").unwrap();

        assert!(!source_created_by_job(&earlier, job_started));
        assert!(source_created_by_job(&created, job_started));
        assert!(!source_created_by_job(
            &directory.path().join("missing"),
            job_started
        ));
    }

    #[test]
    fn missing_managed_ffmpeg_is_not_replaced_by_a_bare_command() {
        assert_eq!(
            ffprobe_path(Path::new("/runtime/ffmpeg")),
            Path::new("/runtime").join(managed_binary_name("ffprobe"))
        );
    }

    #[test]
    fn lossy_pipe_decoding_keeps_reading_after_invalid_windows_bytes() {
        let mut reader = BufReader::new(&b"first\nsecond \x96 \x92\r\nthird\n"[..]);
        let mut buffer = Vec::new();
        let mut lines = Vec::new();

        while let Some(line) = read_lossy_line(&mut reader, &mut buffer).unwrap() {
            lines.push(line);
        }

        assert_eq!(lines, ["first", "second \u{fffd} \u{fffd}", "third"]);
        let mut ffmpeg_error = "Помилка ".as_bytes().to_vec();
        ffmpeg_error.extend_from_slice(&[0x96]);
        ffmpeg_error.extend_from_slice(" шлях".as_bytes());
        assert_eq!(
            read_lossy_to_string(ffmpeg_error.as_slice()).unwrap(),
            "Помилка \u{fffd} шлях"
        );
    }

    #[test]
    fn windows_powershell_commands_force_utf8_output() {
        let script = windows_powershell_script("Write-Output 'Мережа'");
        assert!(script.starts_with(WINDOWS_POWERSHELL_UTF8_PREFIX));
        assert!(script.ends_with("Write-Output 'Мережа'"));
    }

    #[test]
    fn output_collision_uses_numbered_filename() {
        let directory = std::env::temp_dir().join(format!("yt-dlp-bd-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        let input = directory.join("clip.webm");
        let existing = directory.join("clip.mp4");
        fs::write(&input, b"source").unwrap();
        fs::write(&existing, b"existing").unwrap();
        assert_eq!(
            available_output_path(&input, "mp4"),
            directory.join("clip (1).mp4")
        );
        fs::remove_file(input).unwrap();
        fs::remove_file(existing).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
