use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs,
    io::{BufRead, BufReader, Read},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::process::CommandExt;

const YTDLP_RELEASE_BASE: &str = "https://github.com/yt-dlp/yt-dlp/releases/latest/download";
#[cfg(target_os = "windows")]
const FFMPEG_RELEASE_BASE: &str =
    "https://github.com/yt-dlp/FFmpeg-Builds/releases/download/latest";
const MIN_FREE_SPACE_RESERVE: u64 = 500 * 1024 * 1024;
const DISK_GUARD_TRIGGER: u64 = MIN_FREE_SPACE_RESERVE + 128 * 1024 * 1024;
const MAX_HISTORY_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;
const HISTORY_THUMBNAIL_CACHE_LIMIT: usize = 550;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn SetThreadExecutionState(es_flags: u32) -> u32;
}

#[cfg(target_os = "windows")]
const ES_CONTINUOUS: u32 = 0x8000_0000;
#[cfg(target_os = "windows")]
const ES_SYSTEM_REQUIRED: u32 = 0x0000_0001;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ComponentStatus {
    installed: bool,
    version: Option<String>,
    path: Option<String>,
    managed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    yt_dlp: ComponentStatus,
    ffmpeg: ComponentStatus,
    deno: ComponentStatus,
    runtime_dir: String,
    platform: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInstallProgress {
    component: String,
    downloaded: u64,
    total: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VpnStatus {
    detected: bool,
    interface_name: Option<String>,
    confidence: String,
    detail: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MediaPreview {
    title: String,
    thumbnail: Option<String>,
    duration: Option<f64>,
    duration_is_total: bool,
    uploader: Option<String>,
    extractor: Option<String>,
    webpage_url: Option<String>,
    item_count: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreflightRequest {
    url: String,
    output_dir: String,
    mode: String,
    quality: String,
    audio_format: String,
    multi_item: bool,
    title: Option<String>,
    duration: Option<f64>,
    item_count: Option<u64>,
    duration_is_total: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreflightResult {
    title: String,
    item_count: u64,
    intermediate_size: Option<u64>,
    final_output_size: Option<u64>,
    protected_reserve: u64,
    required_space: Option<u64>,
    available_space: u64,
    confidence: String,
    sufficient: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadRequest {
    id: String,
    url: String,
    output_dir: String,
    mode: String,
    quality: String,
    audio_format: String,
    subtitles: bool,
    multi_item: bool,
    cookies_browser: Option<String>,
    expected_required_space: Option<u64>,
}

#[derive(Clone, Serialize)]
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

struct MediaInfo {
    duration: f64,
    height: u64,
    fps: f64,
}

#[derive(Clone, Default)]
struct DownloadManager {
    jobs: Arc<Mutex<HashMap<String, Arc<Mutex<Child>>>>>,
    cancelled: Arc<Mutex<HashSet<String>>>,
    auth_required: Arc<Mutex<HashSet<String>>>,
}

#[derive(Clone, Default)]
struct ProbeManager {
    active: Arc<Mutex<Option<(String, tokio::task::AbortHandle)>>>,
}

#[derive(Default)]
struct SleepPrevention {
    #[cfg(target_os = "macos")]
    process: Mutex<Option<Child>>,
    #[cfg(target_os = "windows")]
    stop: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

#[derive(Default)]
struct HistoryThumbnailCache {
    operation: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct RuntimeMaintenance {
    operation: tokio::sync::Mutex<()>,
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

fn ensure_runtime_idle(manager: &DownloadManager) -> Result<(), String> {
    let active = manager
        .jobs
        .lock()
        .map_err(|_| "Не вдалося перевірити активні завантаження".to_string())?;
    if active.is_empty() {
        Ok(())
    } else {
        Err("Компоненти оновляться після завершення активного завантаження".into())
    }
}

fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("runtime");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не вдалося створити runtime-папку: {error}"))?;
    Ok(directory)
}

fn yt_dlp_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let filename = "yt-dlp.exe";
    #[cfg(not(target_os = "windows"))]
    let filename = "yt-dlp";
    Ok(runtime_dir(app)?.join(filename))
}

fn configure_command(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

fn configure_tokio_command(command: &mut tokio::process::Command) {
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

fn command_version(path: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new(path);
    configure_command(&mut command);
    let output = command.args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn find_ffmpeg(app: &AppHandle) -> Result<ComponentStatus, String> {
    let directory = runtime_dir(app)?;
    let managed_path = directory.join(if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    let probe_path = directory.join(if cfg!(target_os = "windows") {
        "ffprobe.exe"
    } else {
        "ffprobe"
    });

    if managed_path.is_file() && probe_path.is_file() {
        if let (Some(version), Some(_)) = (
            command_version(&managed_path, &["-version"]),
            command_version(&probe_path, &["-version"]),
        ) {
            let first_line = version.lines().next().unwrap_or(&version).to_string();
            return Ok(ComponentStatus {
                installed: true,
                version: Some(first_line),
                path: Some(managed_path.to_string_lossy().to_string()),
                managed: true,
            });
        }
    }

    Ok(ComponentStatus {
        installed: false,
        version: None,
        path: None,
        managed: true,
    })
}

fn find_deno(app: &AppHandle) -> Result<ComponentStatus, String> {
    let managed_path = runtime_dir(app)?.join(if cfg!(target_os = "windows") {
        "deno.exe"
    } else {
        "deno"
    });
    if managed_path.is_file() {
        if let Some(version) = command_version(&managed_path, &["--version"]) {
            return Ok(ComponentStatus {
                installed: true,
                version: Some(version.lines().next().unwrap_or(&version).to_string()),
                path: Some(managed_path.to_string_lossy().to_string()),
                managed: true,
            });
        }
    }

    Ok(ComponentStatus {
        installed: false,
        version: None,
        path: None,
        managed: true,
    })
}

fn runtime_status_blocking(app: &AppHandle) -> Result<RuntimeStatus, String> {
    let directory = runtime_dir(app)?;
    let path = yt_dlp_path(app)?;
    let installed = path.is_file();
    let version = installed.then(|| {
        fs::read_to_string(directory.join(".yt-dlp.version"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "Встановлено".into())
    });

    Ok(RuntimeStatus {
        yt_dlp: ComponentStatus {
            installed,
            version,
            path: installed.then(|| path.to_string_lossy().to_string()),
            managed: true,
        },
        ffmpeg: find_ffmpeg(app)?,
        deno: find_deno(app)?,
        runtime_dir: directory.to_string_lossy().to_string(),
        platform: std::env::consts::OS.into(),
    })
}

#[tauri::command]
async fn runtime_status(app: AppHandle) -> Result<RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || runtime_status_blocking(&app))
        .await
        .map_err(|error| format!("Не вдалося перевірити локальні компоненти: {error}"))?
}

async fn fetch_bytes_with_final_url(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<u8>, Url), String> {
    let response = client
        .get(url)
        .header("User-Agent", "yt-dlp-desktop")
        .send()
        .await
        .map_err(|error| format!("Помилка мережі: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Сервер повернув помилку: {error}"))?;
    let final_url = response.url().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Не вдалося прочитати відповідь: {error}"))?;
    Ok((bytes.to_vec(), final_url))
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    Ok(fetch_bytes_with_final_url(client, url).await?.0)
}

fn release_version_from_download_url(url: &Url) -> Option<String> {
    if url.scheme() != "https" || url.host_str() != Some("github.com") {
        return None;
    }
    let segments = url.path_segments()?.collect::<Vec<_>>();
    let releases = segments
        .windows(2)
        .position(|pair| pair == ["releases", "download"])?;
    let version = *segments.get(releases + 2)?;
    let file = *segments.get(releases + 3)?;
    if file.is_empty() {
        return None;
    }
    (!version.is_empty() && version != "latest").then(|| version.to_string())
}

fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("Не вдалося підготувати завантаження: {error}"))
}

fn is_public_thumbnail_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_unspecified()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 0 && c == 2)
                || (a == 198 && (b == 18 || b == 19))
                || (a == 198 && b == 51 && c == 100)
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return is_public_thumbnail_ip(IpAddr::V4(mapped));
            }
            !(ip.is_loopback()
                || ip.is_multicast()
                || ip.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80
                || (segments[0] & 0xffc0) == 0xfec0
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

async fn public_thumbnail_client(url: &Url) -> Result<reqwest::Client, String> {
    let host = url
        .host_str()
        .ok_or_else(|| "Посилання на прев’ю не містить адреси сервера".to_string())?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let lookup_host = host.clone();
    let addresses = tauri::async_runtime::spawn_blocking(move || {
        (lookup_host.as_str(), port)
            .to_socket_addrs()
            .map(|addresses| addresses.collect::<Vec<SocketAddr>>())
    })
    .await
    .map_err(|error| format!("Не вдалося перевірити адресу прев’ю: {error}"))?
    .map_err(|error| format!("Не вдалося знайти сервер прев’ю: {error}"))?;
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !is_public_thumbnail_ip(address.ip()))
    {
        return Err("Прев’ю вказує на локальну або службову мережеву адресу".into());
    }
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .resolve_to_addrs(&host, &addresses)
        .build()
        .map_err(|error| format!("Не вдалося підготувати безпечне завантаження прев’ю: {error}"))
}

async fn fetch_to_file(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    component: &str,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut response = client
        .get(url)
        .header("User-Agent", "yt-dlp-desktop")
        .send()
        .await
        .map_err(|error| format!("Помилка мережі: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Сервер повернув помилку: {error}"))?;
    let total = response.content_length();
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| format!("Не вдалося створити тимчасовий файл: {error}"))?;
    let mut downloaded = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Не вдалося прочитати завантаження: {error}"))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Не вдалося записати завантаження: {error}"))?;
        downloaded = downloaded.saturating_add(chunk.len() as u64);
        let _ = app.emit(
            "runtime-install-progress",
            RuntimeInstallProgress {
                component: component.into(),
                downloaded,
                total,
            },
        );
    }
    file.flush()
        .await
        .map_err(|error| format!("Не вдалося завершити запис завантаження: {error}"))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let file = fs::File::open(path)
        .map_err(|error| format!("Не вдалося відкрити файл для перевірки: {error}"))?;
    let mut reader = BufReader::new(file);
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("Не вдалося перевірити завантажений файл: {error}"))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_sha256_file_expected(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual != expected {
        return Err(format!("Контрольна сума {label} не збігається"));
    }
    Ok(())
}

fn verify_sha256_file(path: &Path, checksum_file: &[u8], label: &str) -> Result<(), String> {
    let expected = checksum_value(checksum_file, label)?;
    verify_sha256_file_expected(path, &expected, label)
}

fn replace_runtime_file(temporary: &Path, destination: &Path, label: &str) -> Result<(), String> {
    if !destination.exists() {
        return fs::rename(temporary, destination)
            .map_err(|error| format!("Не вдалося встановити {label}: {error}"));
    }
    let backup = destination.with_extension(format!("previous-{}", Uuid::new_v4()));
    fs::rename(destination, &backup)
        .map_err(|error| format!("Не вдалося підготувати заміну {label}: {error}"))?;
    match fs::rename(temporary, destination) {
        Ok(()) => {
            let _ = fs::remove_file(backup);
            Ok(())
        }
        Err(error) => {
            let restore = fs::rename(&backup, destination);
            if let Err(restore_error) = restore {
                return Err(format!(
                    "Не вдалося встановити {label}: {error}. Також не вдалося відновити попередню версію: {restore_error}"
                ));
            }
            Err(format!("Не вдалося встановити {label}: {error}"))
        }
    }
}

#[cfg(any(target_os = "windows", test))]
fn checksum_for_asset(checksum_file: &[u8], asset_name: &str) -> Result<String, String> {
    let checksums = String::from_utf8(checksum_file.to_vec())
        .map_err(|_| "Файл контрольних сум FFmpeg має неправильний формат".to_string())?;
    checksums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?))
        })
        .find_map(|(hash, filename)| {
            let normalized = filename.trim_start_matches('*').trim_start_matches("./");
            (normalized == asset_name).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| format!("Не знайдено контрольну суму для {asset_name}"))
}

#[cfg(target_os = "macos")]
fn macos_ffmpeg_release_asset(page: &str, filename: &str) -> Result<String, String> {
    let release_section = page
        .split_once("<h2>Download Release Build</h2>")
        .map(|(_, section)| section)
        .ok_or_else(|| "Сервер ffmpeg не повідомив про стабільний реліз".to_string())?;
    let architecture_prefix = "/download/macos/arm64/";

    for candidate in release_section.split("href=\"").skip(1) {
        let Some(path) = candidate.split('"').next() else {
            continue;
        };
        if path.starts_with(architecture_prefix) && path.ends_with(filename) {
            return Ok(format!("https://ffmpeg.martin-riedl.de{path}"));
        }
    }

    Err(format!(
        "Сервер ffmpeg не надав {filename} для macOS Apple Silicon"
    ))
}

fn checksum_value(checksum_file: &[u8], label: &str) -> Result<String, String> {
    let checksum = String::from_utf8(checksum_file.to_vec())
        .map_err(|_| format!("Контрольна сума {label} має неправильний формат"))?;
    let expected = checksum
        .split_whitespace()
        .next()
        .ok_or_else(|| format!("Контрольна сума {label} порожня"))?
        .to_ascii_lowercase();
    Ok(expected)
}

fn extract_binary_from_file(
    archive_path: &Path,
    binary_name: &str,
    destination: &Path,
) -> Result<(), String> {
    let archive_file = fs::File::open(archive_path)
        .map_err(|error| format!("Не вдалося відкрити архів {binary_name}: {error}"))?;
    let mut archive = zip::ZipArchive::new(archive_file)
        .map_err(|error| format!("Не вдалося відкрити архів {binary_name}: {error}"))?;
    let index = (0..archive.len())
        .find(|index| {
            archive
                .by_index(*index)
                .ok()
                .and_then(|file| {
                    Path::new(file.name())
                        .file_name()
                        .map(|name| name == binary_name)
                })
                .unwrap_or(false)
        })
        .ok_or_else(|| format!("В архіві не знайдено {binary_name}"))?;
    let mut archived_binary = archive
        .by_index(index)
        .map_err(|error| format!("Не вдалося прочитати {binary_name}: {error}"))?;
    let temporary = destination.with_extension("download");
    let mut output = fs::File::create(&temporary)
        .map_err(|error| format!("Не вдалося створити {binary_name}: {error}"))?;
    std::io::copy(&mut archived_binary, &mut output)
        .map_err(|error| format!("Не вдалося розпакувати {binary_name}: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("Не вдалося завершити запис {binary_name}: {error}"))?;
    drop(output);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Не вдалося дозволити запуск {binary_name}: {error}"))?;
    }
    replace_runtime_file(&temporary, destination, binary_name)
}

async fn install_ffmpeg_inner(app: AppHandle) -> Result<(), String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        const RELEASES_PAGE: &str = "https://ffmpeg.martin-riedl.de/";
        let client = download_client()?;
        let page = fetch_bytes(&client, RELEASES_PAGE).await?;
        let page = String::from_utf8(page)
            .map_err(|_| "Сторінка релізів ffmpeg має неправильний формат".to_string())?;
        let ffmpeg_url = macos_ffmpeg_release_asset(&page, "ffmpeg.zip")?;
        let ffprobe_url = macos_ffmpeg_release_asset(&page, "ffprobe.zip")?;
        eprintln!("Перевіряємо стабільний ffmpeg: {ffmpeg_url}");
        let ffmpeg_checksum_url = format!("{ffmpeg_url}.sha256");
        let ffprobe_checksum_url = format!("{ffprobe_url}.sha256");
        let (ffmpeg_checksum, ffprobe_checksum) = tokio::try_join!(
            fetch_bytes(&client, &ffmpeg_checksum_url),
            fetch_bytes(&client, &ffprobe_checksum_url)
        )?;
        let ffmpeg_expected = checksum_value(&ffmpeg_checksum, "ffmpeg")?;
        let ffprobe_expected = checksum_value(&ffprobe_checksum, "ffprobe")?;
        let directory = runtime_dir(&app)?;
        let ffmpeg_path = directory.join("ffmpeg");
        let ffprobe_path = directory.join("ffprobe");
        let ffmpeg_stamp = directory.join(".ffmpeg.sha256");
        let ffprobe_stamp = directory.join(".ffprobe.sha256");
        let current = command_version(&ffmpeg_path, &["-version"]).is_some()
            && command_version(&ffprobe_path, &["-version"]).is_some()
            && fs::read_to_string(&ffmpeg_stamp)
                .map(|value| value.trim() == ffmpeg_expected)
                .unwrap_or(false)
            && fs::read_to_string(&ffprobe_stamp)
                .map(|value| value.trim() == ffprobe_expected)
                .unwrap_or(false);
        if current {
            eprintln!("Керований ffmpeg вже актуальний");
            return Ok(());
        }

        eprintln!("Завантажуємо керовані ffmpeg та ffprobe");
        let ffmpeg_archive = directory.join(".ffmpeg.zip.download");
        let ffprobe_archive = directory.join(".ffprobe.zip.download");
        let download_result = tokio::try_join!(
            fetch_to_file(&app, &client, &ffmpeg_url, &ffmpeg_archive, "ffmpeg"),
            fetch_to_file(&app, &client, &ffprobe_url, &ffprobe_archive, "ffmpeg")
        );
        if let Err(error) = download_result {
            let _ = fs::remove_file(&ffmpeg_archive);
            let _ = fs::remove_file(&ffprobe_archive);
            return Err(error);
        }
        let install_result = (|| {
            verify_sha256_file(&ffmpeg_archive, &ffmpeg_checksum, "ffmpeg")?;
            verify_sha256_file(&ffprobe_archive, &ffprobe_checksum, "ffprobe")?;
            extract_binary_from_file(&ffmpeg_archive, "ffmpeg", &ffmpeg_path)?;
            extract_binary_from_file(&ffprobe_archive, "ffprobe", &ffprobe_path)
        })();
        let _ = fs::remove_file(&ffmpeg_archive);
        let _ = fs::remove_file(&ffprobe_archive);
        install_result?;
        fs::write(ffmpeg_stamp, ffmpeg_expected)
            .map_err(|error| format!("Не вдалося зберегти версію ffmpeg: {error}"))?;
        fs::write(ffprobe_stamp, ffprobe_expected)
            .map_err(|error| format!("Не вдалося зберегти версію ffprobe: {error}"))?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        #[cfg(target_arch = "x86_64")]
        let asset_name = "ffmpeg-master-latest-win64-gpl.zip";
        #[cfg(target_arch = "aarch64")]
        let asset_name = "ffmpeg-master-latest-winarm64-gpl.zip";
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        return Err("Portable FFmpeg підтримує лише Windows x64 та ARM64".into());

        let client = download_client()?;
        let archive_url = format!("{FFMPEG_RELEASE_BASE}/{asset_name}");
        let checksums_url = format!("{FFMPEG_RELEASE_BASE}/checksums.sha256");
        let checksums = fetch_bytes(&client, &checksums_url).await?;
        let expected = checksum_for_asset(&checksums, asset_name)?;
        let directory = runtime_dir(&app)?;
        let ffmpeg_path = directory.join("ffmpeg.exe");
        let ffprobe_path = directory.join("ffprobe.exe");
        let stamp = directory.join(".ffmpeg.sha256");
        let current = command_version(&ffmpeg_path, &["-version"]).is_some()
            && command_version(&ffprobe_path, &["-version"]).is_some()
            && fs::read_to_string(&stamp)
                .map(|value| value.trim() == expected)
                .unwrap_or(false);
        if current {
            return Ok(());
        }

        eprintln!("Завантажуємо керовані FFmpeg та FFprobe для Windows");
        let archive = directory.join(".ffmpeg.zip.download");
        if let Err(error) = fetch_to_file(&app, &client, &archive_url, &archive, "ffmpeg").await {
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
        let install_result = (|| {
            verify_sha256_file_expected(&archive, &expected, "FFmpeg")?;
            extract_binary_from_file(&archive, "ffmpeg.exe", &ffmpeg_path)?;
            extract_binary_from_file(&archive, "ffprobe.exe", &ffprobe_path)
        })();
        let _ = fs::remove_file(&archive);
        install_result?;
        fs::write(stamp, expected)
            .map_err(|error| format!("Не вдалося зберегти версію FFmpeg: {error}"))?;
        Ok(())
    }

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        target_os = "windows"
    )))]
    Err("Portable FFmpeg поки підтримує лише macOS Apple Silicon та Windows".into())
}

#[tauri::command]
async fn install_ffmpeg(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    ensure_runtime_idle(manager.inner())?;
    install_ffmpeg_inner(app).await
}

async fn install_deno_inner(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let asset_name = if cfg!(target_arch = "aarch64") {
        "deno-aarch64-apple-darwin.zip"
    } else {
        "deno-x86_64-apple-darwin.zip"
    };
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "deno-x86_64-pc-windows-msvc.zip";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    let asset_name = "deno-aarch64-pc-windows-msvc.zip";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("Ця платформа поки що не підтримується".into());

    let base = "https://github.com/denoland/deno/releases/latest/download";
    let archive_url = format!("{base}/{asset_name}");
    let checksum_url = format!("{base}/{asset_name}.sha256sum");
    let client = download_client()
        .map_err(|error| format!("Не вдалося підготувати завантаження Deno: {error}"))?;
    let checksum = fetch_bytes(&client, &checksum_url).await?;
    let expected = checksum_value(&checksum, "Deno")?;
    let filename = if cfg!(target_os = "windows") {
        "deno.exe"
    } else {
        "deno"
    };
    let directory = runtime_dir(&app)?;
    let destination = directory.join(filename);
    let stamp = directory.join(".deno.sha256");
    let current = command_version(&destination, &["--version"]).is_some()
        && fs::read_to_string(&stamp)
            .map(|value| value.trim() == expected)
            .unwrap_or(false);
    if current {
        return Ok(());
    }

    let archive = directory.join(".deno.zip.download");
    if let Err(error) = fetch_to_file(&app, &client, &archive_url, &archive, "deno").await {
        let _ = fs::remove_file(&archive);
        return Err(error);
    }
    let install_result = (|| {
        verify_sha256_file(&archive, &checksum, "Deno")?;
        extract_binary_from_file(&archive, filename, &destination)
    })();
    let _ = fs::remove_file(&archive);
    install_result?;
    fs::write(stamp, expected)
        .map_err(|error| format!("Не вдалося зберегти версію Deno: {error}"))?;
    Ok(())
}

#[tauri::command]
async fn install_deno(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    ensure_runtime_idle(manager.inner())?;
    install_deno_inner(app).await
}

async fn install_ytdlp_inner(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let asset_name = "yt-dlp_macos";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let asset_name = "yt-dlp.exe";
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    let asset_name = "yt-dlp_arm64.exe";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Err("Ця платформа поки що не підтримується".into());

    let client = download_client()?;
    let binary_url = format!("{YTDLP_RELEASE_BASE}/{asset_name}");
    let checksums_url = format!("{YTDLP_RELEASE_BASE}/SHA2-256SUMS");

    let (checksums, checksums_final_url) =
        fetch_bytes_with_final_url(&client, &checksums_url).await?;
    let release_version = release_version_from_download_url(&checksums_final_url);
    let checksums = String::from_utf8(checksums)
        .map_err(|_| "Файл контрольних сум має неправильний формат".to_string())?;
    let expected = checksums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?))
        })
        .find_map(|(hash, filename)| {
            (filename.trim_start_matches('*') == asset_name).then(|| hash.to_lowercase())
        })
        .ok_or_else(|| "Не вдалося знайти контрольну суму yt-dlp".to_string())?;

    let destination = yt_dlp_path(&app)?;
    let version_stamp = runtime_dir(&app)?.join(".yt-dlp.version");
    let current = fs::read(&destination)
        .map(|binary| format!("{:x}", Sha256::digest(binary)) == expected)
        .unwrap_or(false);
    if current {
        if let Some(version) = release_version {
            let _ = fs::write(version_stamp, version);
        }
        return Ok(());
    }

    eprintln!("Завантажуємо стабільний yt-dlp");
    let temporary = destination.with_extension("download");
    if let Err(error) = fetch_to_file(&app, &client, &binary_url, &temporary, "ytDlp").await {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let actual = sha256_file(&temporary)?;
    if actual != expected {
        let _ = fs::remove_file(&temporary);
        return Err("Контрольна сума yt-dlp не збігається. Файл не встановлено".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Не вдалося дозволити запуск yt-dlp: {error}"))?;
    }

    replace_runtime_file(&temporary, &destination, "yt-dlp")?;
    if let Some(version) = release_version {
        let _ = fs::write(version_stamp, version);
    }

    Ok(())
}

#[tauri::command]
async fn install_ytdlp(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    ensure_runtime_idle(manager.inner())?;
    install_ytdlp_inner(app).await
}

#[tauri::command]
async fn update_ytdlp(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    ensure_runtime_idle(manager.inner())?;
    let result = install_ytdlp_inner(app).await;
    if let Err(error) = &result {
        eprintln!("Не вдалося оновити yt-dlp: {error}");
    }
    result
}

fn clean_probe_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let raw = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("yt-dlp не зміг перевірити це посилання")
        .trim()
        .trim_start_matches("ERROR:")
        .trim()
        .to_string();
    friendly_download_error(&raw)
}

fn friendly_download_error(message: &str) -> String {
    let value = message.to_ascii_lowercase();
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "Недостатньо вільного місця. Процес зупинено, щоб захистити диск.".into()
    } else if ["truncated_id", "incomplete youtube id", "looks truncated"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Посилання YouTube неповне. Скопіюйте його ще раз з адресного рядка або через «Поділитися»."
            .into()
    } else if ["private video", "this video is private"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео приватне. Для завантаження потрібен доступ через ваш браузер.".into()
    } else if [
        "sign in",
        "login required",
        "log in",
        "cookies are required",
        "confirm your age",
        "age-restricted",
        "members-only",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "Сервіс просить увійти в обліковий запис. Спробуйте завантаження з cookies браузера.".into()
    } else if ["unsupported url", "no suitable extractor"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Цей сайт або тип посилання поки не підтримується.".into()
    } else if value.contains("drm") {
        "Відео захищене DRM і не може бути завантажене.".into()
    } else if ["geo", "country", "region"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне у вашому регіоні.".into()
    } else if ["video unavailable", "not available", "removed"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Відео недоступне або було видалене.".into()
    } else if ["404", "not found"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "За цим посиланням нічого не знайдено. Перевірте, чи воно скопійоване повністю.".into()
    } else if ["403", "forbidden"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "Сайт відхилив доступ до медіафайла. Спробуйте ще раз або скористайтеся іншим підключенням."
            .into()
    } else {
        "Не вдалося прочитати це посилання. Перевірте його та спробуйте ще раз.".into()
    }
}

fn friendly_conversion_error(message: &str) -> String {
    let value = message.to_ascii_lowercase();
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "Недостатньо вільного місця. Готові файли збережено, решту обробки зупинено.".into()
    } else if value.contains("скасовано") {
        "Обробку скасовано. Уже готові файли залишилися у вибраній папці.".into()
    } else if value.contains("ffprobe") || value.contains("invalid data found") {
        "Завантажений медіафайл пошкоджений або має формат, який не вдалося прочитати. Уже готові файли збережено.".into()
    } else if value.contains("encoder")
        || value.contains("videotoolbox")
        || value.contains("libx264")
    {
        "Не вдалося запустити відеокодування. Уже готові файли збережено; спробуйте повторити проблемний матеріал.".into()
    } else {
        "Не вдалося завершити конвертацію одного з файлів. Уже готові результати збережено у вибраній папці.".into()
    }
}

fn download_error_code(message: &str) -> &'static str {
    let value = message.to_ascii_lowercase();
    if value.contains("недостатньо вільного місця") || value.contains("no space left")
    {
        "low_disk"
    } else if value.contains("429")
        || value.contains("too many requests")
        || value.contains("rate limit")
    {
        "rate_limited"
    } else if [
        "sign in",
        "login required",
        "cookies are required",
        "private video",
    ]
    .iter()
    .any(|pattern| value.contains(pattern))
    {
        "auth_required"
    } else if ["unsupported url", "no suitable extractor"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "unsupported"
    } else if ["404", "not found", "removed", "unavailable"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "unavailable"
    } else if ["network", "timed out", "connection", "temporary failure"]
        .iter()
        .any(|pattern| value.contains(pattern))
    {
        "network"
    } else {
        "unknown"
    }
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn format_selector(mode: &str, quality: &str) -> &'static str {
    if mode == "audio" {
        return "bestaudio/best";
    }
    match quality {
        "2160" => "bestvideo[height<=2160]+bestaudio/best[height<=2160]",
        "1080" => "bestvideo[height<=1080]+bestaudio/best[height<=1080]",
        "720" => "bestvideo[height<=720]+bestaudio/best[height<=720]",
        "480" => "bestvideo[height<=480]+bestaudio/best[height<=480]",
        _ => "bestvideo+bestaudio/best",
    }
}

fn playlist_flag(multi_item: bool) -> &'static str {
    if multi_item {
        "--yes-playlist"
    } else {
        "--no-playlist"
    }
}

fn extraction_args(
    app: &AppHandle,
    mode: &str,
    quality: &str,
    multi_item: bool,
    cookies_browser: Option<&str>,
) -> Result<Vec<OsString>, String> {
    let mut args = vec![
        OsString::from(playlist_flag(multi_item)),
        OsString::from("-f"),
        OsString::from(format_selector(mode, quality)),
    ];
    if let Some(browser) = cookies_browser {
        let supported = [
            "brave", "chrome", "chromium", "edge", "firefox", "opera", "safari", "vivaldi",
        ];
        if !supported.contains(&browser) {
            return Err("Непідтримуваний браузер для cookies".into());
        }
        args.extend([
            OsString::from("--cookies-from-browser"),
            OsString::from(browser),
        ]);
    }
    if let Some(path) = find_deno(app)?.path {
        args.extend([
            OsString::from("--js-runtimes"),
            OsString::from(if path == "deno" {
                "deno".to_string()
            } else {
                format!("deno:{path}")
            }),
        ]);
    }
    Ok(args)
}

async fn probe_oembed(url: &str, endpoint: &str, extractor: &str) -> Result<MediaPreview, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|error| format!("Не вдалося підготувати швидку перевірку: {error}"))?;
    let response = client
        .get(endpoint)
        .query(&[("url", url), ("format", "json")])
        .header("User-Agent", "yt-dlp-desktop")
        .send()
        .await
        .map_err(|_| "Сервіс не відповів під час швидкої перевірки".to_string())?;
    if !response.status().is_success() {
        return Err("Відео не існує, закрите або недоступне".into());
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|_| "Не вдалося прочитати дані про відео".to_string())?;
    let metadata: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Сервіс повернув пошкоджені дані про відео".to_string())?;
    let title = metadata
        .get("title")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "У посиланні не знайдено доступного відео".to_string())?;
    Ok(MediaPreview {
        title: title.to_string(),
        thumbnail: metadata
            .get("thumbnail_url")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        duration: metadata.get("duration").and_then(|value| value.as_f64()),
        duration_is_total: true,
        uploader: metadata
            .get("author_name")
            .and_then(|value| value.as_str())
            .map(str::to_string),
        extractor: Some(extractor.to_string()),
        webpage_url: Some(url.to_string()),
        item_count: Some(1),
    })
}

async fn probe_url_inner(
    app: AppHandle,
    url: String,
    playlist: bool,
) -> Result<MediaPreview, String> {
    let parsed = Url::parse(&url).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if !playlist && (host_matches(&host, "youtube.com") || host_matches(&host, "youtu.be")) {
        return probe_oembed(&url, "https://www.youtube.com/oembed", "YouTube").await;
    }
    if !playlist && host_matches(&host, "vimeo.com") {
        return probe_oembed(&url, "https://vimeo.com/api/oembed.json", "Vimeo").await;
    }

    let yt_dlp = yt_dlp_path(&app)?;
    if !yt_dlp.is_file() {
        return Err("yt-dlp ще не встановлено".into());
    }

    let mut command = tokio::process::Command::new(yt_dlp);
    configure_tokio_command(&mut command);
    command.args([
        "--ignore-config",
        "--simulate",
        "--no-warnings",
        "--no-ignore-no-formats-error",
        "--socket-timeout",
        "6",
        "--extractor-retries",
        "1",
    ]);
    if playlist {
        command.args(["--yes-playlist", "--flat-playlist", "--dump-single-json"]);
    } else {
        command.args([
            "--no-playlist",
            "--print",
            "%(.{title,thumbnail,duration,uploader,channel,extractor,extractor_key,webpage_url})#j",
        ]);
    }
    if let Some(path) = find_deno(&app)?.path {
        if path == "deno" {
            command.args(["--js-runtimes", "deno"]);
        } else {
            command.args(["--js-runtimes", &format!("deno:{path}")]);
        }
    }
    command.arg(&url).kill_on_drop(true);

    let output = tokio::time::timeout(Duration::from_secs(12), command.output())
        .await
        .map_err(|_| "Перевірка тривала надто довго. Спробуйте ще раз".to_string())?
        .map_err(|error| format!("Не вдалося запустити yt-dlp: {error}"))?;
    if !output.status.success() {
        return Err(clean_probe_error(&output.stderr));
    }

    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "yt-dlp повернув пошкоджені дані про відео".to_string())?;
    let title = metadata
        .get("title")
        .and_then(|value| value.as_str())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "У посиланні не знайдено доступного відео".to_string())?;
    let string_field = |name: &str| {
        metadata
            .get(name)
            .and_then(|value| value.as_str())
            .filter(|value| !value.trim().is_empty())
            .map(str::to_string)
    };
    let entries = metadata.get("entries").and_then(|value| value.as_array());
    let first_entry = entries.and_then(|entries| entries.first());
    let playlist_string = |name: &str| {
        string_field(name).or_else(|| {
            first_entry
                .and_then(|entry| entry.get(name))
                .and_then(|value| value.as_str())
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
    };
    let (duration, duration_is_total) = if playlist {
        if let Some(entries) = entries.filter(|entries| !entries.is_empty()) {
            let durations = entries
                .iter()
                .filter_map(|entry| json_number(entry.get("duration")))
                .collect::<Vec<_>>();
            if durations.len() == entries.len() {
                (Some(durations.into_iter().sum()), true)
            } else {
                (durations.first().copied(), false)
            }
        } else {
            (json_number(metadata.get("duration")), true)
        }
    } else {
        (json_number(metadata.get("duration")), true)
    };
    Ok(MediaPreview {
        title: title.to_string(),
        thumbnail: playlist_string("thumbnail"),
        duration,
        duration_is_total,
        uploader: playlist_string("uploader").or_else(|| playlist_string("channel")),
        extractor: playlist_string("extractor_key").or_else(|| playlist_string("extractor")),
        webpage_url: string_field("webpage_url"),
        item_count: if playlist {
            json_number(metadata.get("playlist_count"))
                .map(|value| value.round() as u64)
                .or_else(|| entries.map(|entries| entries.len() as u64))
        } else {
            Some(1)
        },
    })
}

#[tauri::command]
async fn probe_url(
    app: AppHandle,
    manager: State<'_, ProbeManager>,
    probe_id: String,
    url: String,
    playlist: bool,
) -> Result<MediaPreview, String> {
    let task = tokio::spawn(probe_url_inner(app, url, playlist));
    let abort_handle = task.abort_handle();
    {
        let mut active = manager
            .active
            .lock()
            .map_err(|_| "Внутрішня помилка перевірки".to_string())?;
        if let Some((_, previous)) = active.replace((probe_id.clone(), abort_handle)) {
            previous.abort();
        }
    }
    let result = task.await.map_err(|error| {
        if error.is_cancelled() {
            "Перевірку скасовано через нове посилання".to_string()
        } else {
            format!("Не вдалося завершити перевірку: {error}")
        }
    })?;
    if let Ok(mut active) = manager.active.lock() {
        if active
            .as_ref()
            .map(|(active_id, _)| active_id == &probe_id)
            .unwrap_or(false)
        {
            *active = None;
        }
    }
    result
}

fn is_tunnel_interface(interface_name: &str) -> bool {
    let value = interface_name.to_ascii_lowercase();
    ["utun", "tun", "tap", "wg", "wireguard", "ppp"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        || ["vpn", "tailscale", "mullvad", "nordlynx", "proton"]
            .iter()
            .any(|marker| value.contains(marker))
}

#[tauri::command]
fn check_vpn(host: String) -> Result<VpnStatus, String> {
    if host.is_empty()
        || host.len() > 253
        || !host
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'))
    {
        return Err("Некоректне доменне ім’я".into());
    }

    #[cfg(target_os = "macos")]
    {
        let ip = (host.as_str(), 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next())
            .map(|address| address.ip().to_string());
        let interface_name = ip.as_ref().and_then(|ip| {
            Command::new("/sbin/route")
                .args(["-n", "get", ip])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .find_map(|line| line.trim().strip_prefix("interface:").map(str::trim))
                        .map(str::to_string)
                })
        });
        let routed_through_tunnel = interface_name
            .as_deref()
            .map(is_tunnel_interface)
            .unwrap_or(false);
        let detected = routed_through_tunnel;
        let detail = if routed_through_tunnel {
            "Маршрут саме до цього ресурсу проходить через VPN-тунель"
        } else {
            "Маршрут до цього ресурсу не проходить через VPN-тунель"
        };
        Ok(VpnStatus {
            detected,
            interface_name,
            confidence: "high".into(),
            detail: detail.into(),
        })
    }

    #[cfg(target_os = "windows")]
    {
        let ip = (host.as_str(), 443)
            .to_socket_addrs()
            .ok()
            .and_then(|mut addresses| addresses.next())
            .map(|address| address.ip().to_string());
        let interface_name = ip.as_ref().and_then(|ip| {
            let mut command = Command::new("powershell.exe");
            configure_command(&mut command);
            command
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-Command",
                    "$route = Find-NetRoute -RemoteIPAddress $env:YTDLP_ROUTE_IP -ErrorAction Stop | Select-Object -First 1; $route.InterfaceAlias",
                ])
                .env("YTDLP_ROUTE_IP", ip)
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| {
                    String::from_utf8_lossy(&output.stdout)
                        .lines()
                        .map(str::trim)
                        .find(|line| !line.is_empty())
                        .map(str::to_string)
                })
        });
        let routed_through_tunnel = interface_name
            .as_deref()
            .map(is_tunnel_interface)
            .unwrap_or(false);
        let route_known = interface_name.is_some();
        Ok(VpnStatus {
            detected: routed_through_tunnel,
            interface_name,
            confidence: if route_known { "high" } else { "unknown" }.into(),
            detail: if routed_through_tunnel {
                "Маршрут саме до цього ресурсу проходить через VPN-тунель"
            } else if route_known {
                "Маршрут до цього ресурсу не проходить через відомий VPN-інтерфейс"
            } else {
                "Не вдалося визначити Windows-інтерфейс для маршруту до цього ресурсу"
            }
            .into(),
        })
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(VpnStatus {
        detected: false,
        interface_name: None,
        confidence: "unknown".into(),
        detail: "Перевірку VPN для цієї платформи ще не реалізовано".into(),
    })
}

fn emit_line(app: &AppHandle, id: &str, line: &str) {
    const MARKER: &str = "__YTDLP_PROGRESS__";
    const POSTPROCESS_MARKER: &str = "__YTDLP_POSTPROCESS__";
    if let Some(payload) = line.strip_prefix(MARKER) {
        let mut values = payload.splitn(4, '|');
        let percent = values
            .next()
            .map(str::trim)
            .and_then(|value| value.trim_end_matches('%').trim().parse::<f32>().ok());
        let speed = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let eta = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let message = values
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let _ = app.emit(
            "download-event",
            DownloadEvent {
                id: id.into(),
                kind: "progress".into(),
                percent,
                speed: speed.map(str::to_string),
                eta: eta.map(str::to_string),
                message: message.map(str::to_string),
                storage: None,
                outputs: None,
                title: message.map(str::to_string),
                thumbnail: None,
                uploader: None,
                extractor: None,
                error_code: None,
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
                    percent: None,
                    speed: None,
                    eta: None,
                    message: Some(message.into()),
                    storage: None,
                    outputs: None,
                    title: None,
                    thumbnail: None,
                    uploader: None,
                    extractor: None,
                    error_code: None,
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
                percent: None,
                speed: is_postprocess_log.then(|| "VideoToolbox".into()),
                eta: None,
                message: Some(if is_postprocess_log {
                    "Об’єднуємо завантажені потоки… Не закривайте застосунок".into()
                } else {
                    line.trim().to_string()
                }),
                storage: None,
                outputs: None,
                title: None,
                thumbnail: None,
                uploader: None,
                extractor: None,
                error_code: None,
            },
        );
    }
}

fn looks_like_auth_error(line: &str) -> bool {
    let value = line.to_ascii_lowercase();
    [
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
    .any(|pattern| value.contains(pattern))
}

fn json_number(value: Option<&serde_json::Value>) -> Option<f64> {
    value.and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
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
            percent: None,
            speed: None,
            eta: None,
            message: None,
            storage: None,
            outputs: None,
            title,
            thumbnail,
            uploader,
            extractor,
            error_code: None,
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

fn required_space_with_reserve(intermediate: u64, output: u64) -> Option<u64> {
    intermediate
        .checked_add(output)
        .and_then(|total| total.checked_add(MIN_FREE_SPACE_RESERVE))
}

fn low_disk_guard_triggered(available: u64) -> bool {
    available <= DISK_GUARD_TRIGGER
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SizeConfidence {
    #[cfg(test)]
    Exact,
    Approximate,
    Unknown,
}

#[cfg(test)]
fn selected_media_from_value(
    metadata: &serde_json::Value,
) -> (Option<SelectedMediaEstimate>, SizeConfidence) {
    let media_id = metadata
        .get("id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let duration = json_number(metadata.get("duration"));
    let requested = metadata
        .get("requested_formats")
        .and_then(|value| value.as_array())
        .filter(|formats| !formats.is_empty())
        .or_else(|| {
            metadata
                .get("requested_downloads")
                .and_then(|value| value.as_array())
                .filter(|formats| !formats.is_empty())
        });
    let formats: Vec<&serde_json::Value> = requested
        .map(|formats| formats.iter().collect())
        .unwrap_or_else(|| vec![metadata]);
    let mut source_size = 0_u64;
    let mut confidence = SizeConfidence::Exact;
    let mut height = json_number(metadata.get("height")).map(|value| value.round() as u64);
    let mut fps = json_number(metadata.get("fps"));
    let mut total_bitrate_kbps = json_number(metadata.get("tbr"));
    for format in formats {
        height =
            height.or_else(|| json_number(format.get("height")).map(|value| value.round() as u64));
        fps = fps.or_else(|| json_number(format.get("fps")));
        let bitrate = json_number(format.get("tbr"))
            .or_else(|| json_number(format.get("vbr")))
            .or_else(|| json_number(format.get("abr")));
        total_bitrate_kbps = total_bitrate_kbps.or(bitrate);
        let (size, format_confidence) = if let Some(size) = json_number(format.get("filesize")) {
            (Some(size), SizeConfidence::Exact)
        } else if let Some(size) = json_number(format.get("filesize_approx")) {
            (Some(size), SizeConfidence::Approximate)
        } else if let (Some(duration), Some(bitrate)) = (duration, bitrate) {
            (
                Some(duration * bitrate * 1_000.0 / 8.0),
                SizeConfidence::Approximate,
            )
        } else {
            (None, SizeConfidence::Unknown)
        };
        let Some(size) = size.filter(|size| size.is_finite() && *size > 0.0) else {
            return (None, SizeConfidence::Unknown);
        };
        if format_confidence == SizeConfidence::Approximate {
            confidence = SizeConfidence::Approximate;
        }
        let Some(size) = (size <= u64::MAX as f64).then_some(size.ceil() as u64) else {
            return (None, SizeConfidence::Unknown);
        };
        let Some(total) = source_size.checked_add(size) else {
            return (None, SizeConfidence::Unknown);
        };
        source_size = total;
    }
    (
        Some(SelectedMediaEstimate {
            media_id,
            source_size,
            duration,
            height,
            fps,
            total_bitrate_kbps,
        }),
        confidence,
    )
}

fn available_disk_space(path: &Path) -> Option<u64> {
    fs2::available_space(path).ok()
}

fn scale_playlist_estimate(
    estimate: Option<u64>,
    item_count: u64,
    duration_is_total: bool,
) -> Option<u64> {
    if duration_is_total {
        estimate
    } else {
        estimate.and_then(|value| value.checked_mul(item_count))
    }
}

#[tauri::command]
fn folder_free_space(path: String) -> Result<u64, String> {
    let path = PathBuf::from(path);
    if !path.is_dir() {
        return Err("Обрана папка недоступна".into());
    }
    available_disk_space(&path)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".into())
}

#[tauri::command]
fn preflight_download(request: PreflightRequest) -> Result<PreflightResult, String> {
    let parsed = Url::parse(&request.url).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let output_dir = PathBuf::from(&request.output_dir);
    if !output_dir.is_dir() {
        return Err("Обрана папка для завантаження недоступна".into());
    }
    let available_space = available_disk_space(&output_dir)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".to_string())?;
    let title = request
        .title
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Медіафайл".into());
    let item_count = if request.multi_item {
        request.item_count.unwrap_or(1).max(1)
    } else {
        1
    };
    let duration = request
        .duration
        .filter(|value| value.is_finite() && *value > 0.0);
    let height = match request.quality.as_str() {
        "2160" => Some(2160),
        "1080" => Some(1080),
        "720" => Some(720),
        "480" => Some(480),
        _ => None,
    };
    let estimate = duration.map(|duration| SelectedMediaEstimate {
        media_id: "quick-preflight".into(),
        source_size: 0,
        duration: Some(duration),
        height,
        fps: Some(30.0),
        total_bitrate_kbps: None,
    });
    let per_duration_estimate = estimate.as_ref().and_then(|estimate| {
        estimated_output_size(estimate, request.mode != "audio", &request.audio_format)
    });
    let final_total = scale_playlist_estimate(
        per_duration_estimate,
        item_count,
        !request.multi_item || request.duration_is_total.unwrap_or(false),
    );
    // A fast preflight intentionally reuses already-fetched preview metadata instead of
    // asking yt-dlp to resolve every selected format again. At peak disk usage the source
    // and converted output coexist, so using the output estimate for both is conservative.
    let intermediate_total = final_total;
    let confidence = if final_total.is_some() {
        SizeConfidence::Approximate
    } else {
        SizeConfidence::Unknown
    };
    let required_space = intermediate_total
        .zip(final_total)
        .and_then(|(intermediate, output)| required_space_with_reserve(intermediate, output));
    let sufficient = required_space
        .map(|required| available_space >= required)
        .unwrap_or(available_space > DISK_GUARD_TRIGGER);
    Ok(PreflightResult {
        title,
        item_count,
        intermediate_size: intermediate_total,
        final_output_size: final_total,
        protected_reserve: MIN_FREE_SPACE_RESERVE,
        required_space,
        available_space,
        confidence: match confidence {
            SizeConfidence::Approximate => "approximate",
            SizeConfidence::Unknown => "unknown",
            #[cfg(test)]
            SizeConfidence::Exact => "exact",
        }
        .into(),
        sufficient,
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
    let available_space = available_disk_space(&tracker.output_dir);
    let _ = app.emit(
        "download-event",
        DownloadEvent {
            id: id.into(),
            kind: "storage_estimate".into(),
            percent: None,
            speed: None,
            eta: None,
            message: None,
            storage: Some(StorageEstimate {
                estimated_size,
                required_space,
                available_space,
                sufficient: available_space.map(|available| available >= required_space),
            }),
            outputs: None,
            title: None,
            thumbnail: None,
            uploader: None,
            extractor: None,
            error_code: None,
        },
    );
}

fn read_output<R: Read + Send + 'static>(
    app: AppHandle,
    manager: DownloadManager,
    id: String,
    reader: R,
    downloaded_files: Option<Arc<Mutex<Vec<PathBuf>>>>,
    storage_tracker: Option<StorageTracker>,
    last_error: Arc<Mutex<Option<String>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines().map_while(Result::ok) {
            if let Some(payload) = line.strip_prefix("__YTDLP_SIZE__") {
                if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(payload) {
                    emit_download_metadata(&app, &id, &metadata);
                    if let (Some(tracker), Some(estimate)) =
                        (storage_tracker.as_ref(), parse_size_estimate(&metadata))
                    {
                        emit_storage_estimate(&app, &id, tracker, estimate);
                    }
                }
                continue;
            } else if let Some(path) = line.strip_prefix("__YTDLP_FILE__") {
                if let Some(files) = downloaded_files.as_ref() {
                    if let Ok(mut files) = files.lock() {
                        let path = PathBuf::from(path.trim());
                        if !files.contains(&path) {
                            files.push(path);
                        }
                    }
                }
                continue;
            }
            if looks_like_auth_error(&line) {
                if let Ok(mut jobs) = manager.auth_required.lock() {
                    jobs.insert(id.clone());
                }
            }
            let normalized = line.to_ascii_lowercase();
            if normalized.contains("error:")
                || normalized.contains("http error")
                || normalized.contains("unable to download")
            {
                if let Ok(mut error) = last_error.lock() {
                    *error = Some(line.trim().trim_start_matches("ERROR:").trim().to_string());
                }
            }
            emit_line(&app, &id, &line);
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
    #[cfg(unix)]
    {
        let process_group = child
            .lock()
            .map_err(|_| "Не вдалося зупинити процес".to_string())?
            .id() as i32;
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
        let process_id = child
            .lock()
            .map_err(|_| "Не вдалося зупинити процес".to_string())?
            .id();
        #[cfg(target_os = "windows")]
        {
            let mut command = Command::new("taskkill");
            configure_command(&mut command);
            let status = command
                .args(["/PID", &process_id.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|error| format!("Не вдалося запустити taskkill: {error}"))?;
            if status.success() {
                return Ok(());
            }
        }
        child
            .lock()
            .map_err(|_| "Не вдалося зупинити процес".to_string())?
            .kill()
            .map_err(|error| format!("Не вдалося зупинити процес: {error}"))
    }
}

fn ffprobe_path(ffmpeg_path: &Path) -> PathBuf {
    if ffmpeg_path.components().count() > 1 {
        ffmpeg_path.with_file_name(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        })
    } else {
        PathBuf::from(if cfg!(target_os = "windows") {
            "ffprobe.exe"
        } else {
            "ffprobe"
        })
    }
}

fn parse_frame_rate(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    (denominator > 0.0).then_some(numerator / denominator)
}

fn media_info(ffmpeg_path: &Path, input: &Path) -> Result<MediaInfo, String> {
    let mut command = Command::new(ffprobe_path(ffmpeg_path));
    configure_command(&mut command);
    let output = command
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
        .arg(input)
        .output()
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
    configure_command(&mut command);
    let output = command
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .output()
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

fn videotoolbox_encoder_available(ffmpeg_path: &Path) -> bool {
    let mut command = Command::new(ffmpeg_path);
    configure_command(&mut command);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-f",
            "lavfi",
            "-i",
            "color=size=64x64:rate=1",
            "-frames:v",
            "1",
            "-an",
            "-c:v",
            "h264_videotoolbox",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn convert_downloaded_media(
    app: &AppHandle,
    manager: &DownloadManager,
    id: &str,
    ffmpeg_path: &Path,
    input: &Path,
    file_position: (usize, usize),
    output_format: &str,
) -> Result<PathBuf, String> {
    let (file_index, file_count) = file_position;
    if download_was_cancelled(manager, id) {
        return Err("Завантаження скасовано".into());
    }

    let is_video = output_format == "mp4";
    let use_videotoolbox = is_video && videotoolbox_encoder_available(ffmpeg_path);
    let media = is_video
        .then(|| media_info(ffmpeg_path, input))
        .transpose()?;
    let duration = if let Some(media) = media.as_ref() {
        media.duration
    } else {
        media_duration(ffmpeg_path, input)?
    };
    let output = available_output_path(input, output_format);
    let output_dir = output
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if available_disk_space(&output_dir)
        .map(low_disk_guard_triggered)
        .unwrap_or(true)
    {
        return Err(
            "Недостатньо вільного місця. Конвертацію не розпочато, щоб захистити диск.".into(),
        );
    }
    let temporary = output.with_file_name(format!(
        ".{}.{}.part.{}",
        output
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("video"),
        Uuid::new_v4(),
        output_format
    ));
    let mut command = Command::new(ffmpeg_path);
    configure_command(&mut command);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-stats_period",
            "0.25",
            "-progress",
            "pipe:1",
            "-i",
        ])
        .arg(input);
    if let Some(media) = media.as_ref() {
        let bitrate = target_video_bitrate(Some(media.height), Some(media.fps), None);
        let maxrate = bitrate.saturating_mul(11) / 10;
        let bufsize = bitrate.saturating_mul(2);
        command.args([
            "-map",
            "0:v:0",
            "-map",
            "0:a:0?",
            "-map_metadata",
            "0",
            "-c:v",
            if use_videotoolbox {
                "h264_videotoolbox"
            } else {
                "libx264"
            },
            "-b:v",
            &bitrate.to_string(),
            "-maxrate",
            &maxrate.to_string(),
            "-bufsize",
            &bufsize.to_string(),
            "-c:a",
            "aac",
            "-b:a",
            "192k",
            "-tag:v",
            "avc1",
            "-movflags",
            "+faststart",
        ]);
    } else {
        command.args(["-map", "0:a:0", "-vn", "-map_metadata", "0"]);
        match output_format {
            "m4a" => {
                command.args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"]);
            }
            "opus" => {
                command.args(["-c:a", "libopus", "-b:a", "160k"]);
            }
            "wav" => {
                command.args(["-c:a", "pcm_s16le", "-ar", "48000"]);
            }
            _ => {
                command.args(["-c:a", "libmp3lame", "-b:a", "192k"]);
            }
        }
    }
    command
        .arg(&temporary)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .map_err(|error| format!("Не вдалося запустити ffmpeg: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id.to_string(), child.clone());

    let errors = Arc::new(Mutex::new(String::new()));
    let error_reader = stderr.map(|stderr| {
        let errors = errors.clone();
        thread::spawn(move || {
            let mut text = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut text);
            if let Ok(mut target) = errors.lock() {
                *target = text;
            }
        })
    });

    let mut out_time_us = 0.0;
    let mut speed_value = 0.0;
    let format_label = output_format.to_ascii_uppercase();
    let encoder_label = if use_videotoolbox {
        "VideoToolbox"
    } else if is_video {
        "H.264"
    } else {
        "FFmpeg"
    };
    let mut speed_label = encoder_label.to_string();
    let mut low_disk = false;
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = line.strip_prefix("out_time_us=") {
                out_time_us = value.trim().parse::<f64>().unwrap_or(out_time_us);
            } else if let Some(value) = line.strip_prefix("speed=") {
                let value = value.trim();
                speed_label = if value.is_empty() || value == "N/A" {
                    encoder_label.into()
                } else {
                    format!("Кодування {value}")
                };
                speed_value = value
                    .trim_end_matches('x')
                    .parse::<f64>()
                    .unwrap_or(speed_value);
            } else if line.starts_with("progress=") {
                if available_disk_space(&output_dir)
                    .map(low_disk_guard_triggered)
                    .unwrap_or(true)
                {
                    low_disk = true;
                    let _ = stop_child_process_group(&child);
                    break;
                }
                let elapsed = out_time_us / 1_000_000.0;
                let overall_percent = conversion_percent(elapsed, duration, file_index, file_count);
                let remaining = if speed_value > 0.0 {
                    format_eta((duration - elapsed).max(0.0) / speed_value)
                } else {
                    "—".into()
                };
                let _ = app.emit(
                    "download-event",
                    DownloadEvent {
                        id: id.into(),
                        kind: "conversion_progress".into(),
                        percent: Some(overall_percent),
                        speed: Some(speed_label.clone()),
                        eta: Some(remaining),
                        message: Some(if file_count > 1 {
                            format!(
                                "Створюємо {} — файл {} із {}… Не закривайте застосунок",
                                format_label,
                                file_index + 1,
                                file_count
                            )
                        } else {
                            format!("Створюємо {format_label}… Не закривайте застосунок")
                        }),
                        storage: None,
                        outputs: None,
                        title: None,
                        thumbnail: None,
                        uploader: None,
                        extractor: None,
                        error_code: None,
                    },
                );
            }
        }
    }

    let status = loop {
        if let Some(status) = child
            .lock()
            .ok()
            .and_then(|mut child| child.try_wait().ok())
            .flatten()
        {
            break status;
        }
        thread::sleep(Duration::from_millis(100));
    };
    if let Some(reader) = error_reader {
        let _ = reader.join();
    }

    if download_was_cancelled(manager, id) {
        let _ = fs::remove_file(&temporary);
        return Err("Завантаження скасовано".into());
    }
    if low_disk {
        let _ = fs::remove_file(&temporary);
        return Err("Недостатньо вільного місця. Конвертацію зупинено, щоб захистити диск.".into());
    }
    if !status.success() {
        let _ = fs::remove_file(&temporary);
        let detail = errors
            .lock()
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "ffmpeg завершився з помилкою".into());
        return Err(format!("Не вдалося створити {format_label}: {detail}"));
    }

    fs::rename(&temporary, &output)
        .map_err(|error| format!("Не вдалося зберегти готовий {format_label}: {error}"))?;
    let _ = fs::remove_file(input);
    Ok(output)
}

#[tauri::command]
fn inspect_history_files(paths: Vec<String>) -> Vec<HistoryFileStatus> {
    paths
        .into_iter()
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

fn history_thumbnail_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("history-thumbnails");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не вдалося створити кеш прев’ю: {error}"))?;
    Ok(directory)
}

fn thumbnail_extension(content_type: &str) -> Option<&'static str> {
    match content_type
        .split(';')
        .next()?
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "image/jpeg" => Some("jpg"),
        "image/png" => Some("png"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

fn prune_history_thumbnail_cache(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut files = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            metadata.is_file().then(|| {
                (
                    metadata
                        .modified()
                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect::<Vec<_>>();
    if files.len() <= HISTORY_THUMBNAIL_CACHE_LIMIT {
        return;
    }
    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len() - HISTORY_THUMBNAIL_CACHE_LIMIT;
    for (_, path) in files.into_iter().take(remove_count) {
        let _ = fs::remove_file(path);
    }
}

#[tauri::command]
async fn cache_history_thumbnail(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
    url: String,
) -> Result<String, String> {
    let _operation = cache.operation.lock().await;
    let parsed = Url::parse(&url).map_err(|_| "Некоректне посилання на прев’ю".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Прев’ю має використовувати HTTP або HTTPS".into());
    }
    let directory = history_thumbnail_dir(&app)?;
    let digest = format!("{:x}", Sha256::digest(url.as_bytes()));
    for extension in ["jpg", "png", "webp", "gif", "avif"] {
        let cached = directory.join(format!("{digest}.{extension}"));
        if cached.is_file() {
            return Ok(cached.to_string_lossy().into_owned());
        }
    }

    let mut current = parsed;
    let mut redirect_count = 0_u8;
    let mut response = loop {
        let client = public_thumbnail_client(&current).await?;
        let response = client
            .get(current.clone())
            .header("User-Agent", "yt-dlp-desktop")
            .send()
            .await
            .map_err(|error| format!("Не вдалося завантажити прев’ю: {error}"))?;
        if response.status().is_redirection() {
            if redirect_count >= 10 {
                return Err("Сервер прев’ю виконав забагато перенаправлень".into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| "Сервер прев’ю повернув некоректне перенаправлення".to_string())?;
            current = current
                .join(location)
                .map_err(|_| "Сервер прев’ю повернув некоректну нову адресу".to_string())?;
            if !matches!(current.scheme(), "http" | "https") || current.host_str().is_none() {
                return Err("Перенаправлення прев’ю використовує непідтримувану адресу".into());
            }
            redirect_count += 1;
            continue;
        }
        break response
            .error_for_status()
            .map_err(|error| format!("Сервер прев’ю повернув помилку: {error}"))?;
    };
    if response
        .content_length()
        .is_some_and(|size| size > MAX_HISTORY_THUMBNAIL_BYTES)
    {
        return Err("Прев’ю завелике для локального кешу".into());
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let extension = thumbnail_extension(content_type)
        .ok_or_else(|| "Сервер повернув непідтримуваний формат прев’ю".to_string())?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Не вдалося прочитати прев’ю: {error}"))?
    {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > MAX_HISTORY_THUMBNAIL_BYTES {
            return Err("Прев’ю завелике для локального кешу".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Err("Сервер повернув порожнє прев’ю".into());
    }
    let destination = directory.join(format!("{digest}.{extension}"));
    let temporary = directory.join(format!(".{digest}.{}.part", Uuid::new_v4()));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(|error| format!("Не вдалося записати кеш прев’ю: {error}"))?;
    if let Err(error) = tokio::fs::rename(&temporary, &destination).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(format!("Не вдалося завершити кешування прев’ю: {error}"));
    }
    prune_history_thumbnail_cache(&directory);
    Ok(destination.to_string_lossy().into_owned())
}

#[tauri::command]
async fn clear_history_thumbnail_cache(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
) -> Result<(), String> {
    let _operation = cache.operation.lock().await;
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("history-thumbnails");
    if directory.exists() {
        fs::remove_dir_all(&directory)
            .map_err(|error| format!("Не вдалося очистити кеш прев’ю: {error}"))?;
    }
    fs::create_dir_all(directory)
        .map_err(|error| format!("Не вдалося відновити кеш прев’ю: {error}"))
}

#[tauri::command]
async fn delete_history_thumbnail(
    app: AppHandle,
    cache: State<'_, HistoryThumbnailCache>,
    path: String,
) -> Result<(), String> {
    let _operation = cache.operation.lock().await;
    let directory = history_thumbnail_dir(&app)?;
    let candidate = PathBuf::from(path);
    if candidate.parent() != Some(directory.as_path()) {
        return Err("Можна видаляти лише файли кешу прев’ю".into());
    }
    if candidate.exists() {
        fs::remove_file(candidate)
            .map_err(|error| format!("Не вдалося видалити прев’ю з кешу: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
fn start_download(
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
    let parsed = Url::parse(&request.url).map_err(|_| "Вставте повне посилання".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err("Підтримуються лише HTTP та HTTPS посилання".into());
    }
    let is_youtube = parsed
        .host_str()
        .map(|host| {
            host_matches(&host.to_ascii_lowercase(), "youtube.com")
                || host_matches(&host.to_ascii_lowercase(), "youtu.be")
        })
        .unwrap_or(false);
    let output_dir = PathBuf::from(&request.output_dir);
    if !output_dir.is_dir() {
        return Err(
            "Обрана папка завантажень недоступна. Підключіть диск або виберіть іншу папку".into(),
        );
    }
    let available_space = available_disk_space(&output_dir)
        .ok_or_else(|| "Не вдалося визначити вільне місце в обраній папці".to_string())?;
    let enough_space = request
        .expected_required_space
        .map(|required| available_space >= required)
        .unwrap_or(available_space > DISK_GUARD_TRIGGER);
    if !enough_space {
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
        "--no-simulate",
        "--newline",
        "--no-colors",
        "--progress",
        "--print",
        "video:__YTDLP_SIZE__%(.{id,title,thumbnail,uploader,channel,extractor,extractor_key,webpage_url,filesize,filesize_approx,duration,tbr,height,fps})j",
        "--progress-delta",
        "0.25",
        "--progress-template",
        "__YTDLP_PROGRESS__%(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s|%(info.title)s",
        "--progress-template",
        "postprocess:__YTDLP_POSTPROCESS__%(progress.status)s|%(progress.postprocessor)s|%(info.title)s",
        "-P",
        &request.output_dir,
    ]);
    command.args(extraction_args(
        &app,
        &request.mode,
        &request.quality,
        request.multi_item,
        request.cookies_browser.as_deref(),
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
    let ffmpeg = find_ffmpeg(&app)?;
    if let Some(path) = ffmpeg
        .path
        .as_ref()
        .filter(|path| path.as_str() != "ffmpeg")
    {
        let ffmpeg_directory = Path::new(&path).parent().unwrap_or(&runtime);
        command.args([
            "--ffmpeg-location",
            ffmpeg_directory.to_string_lossy().as_ref(),
        ]);
    }

    command.args(["--print", "after_move:__YTDLP_FILE__%(filepath)s"]);
    if request.mode != "audio" {
        command.args(["--merge-output-format", "mkv", "--remux-video", "mkv"]);
    }
    if request.subtitles {
        command.args(["--write-subs", "--write-auto-subs", "--sub-langs", "uk,en"]);
    }
    command
        .arg(&request.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    command.process_group(0);

    let retry_program: OsString = command.get_program().to_os_string();
    let retry_args: Vec<OsString> = command.get_args().map(OsString::from).collect();
    let mut child = command
        .spawn()
        .map_err(|error| format!("Не вдалося запустити yt-dlp: {error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let child = Arc::new(Mutex::new(child));
    let downloaded_files = Arc::new(Mutex::new(Vec::<PathBuf>::new()));
    let storage_tracker = StorageTracker {
        output_dir,
        estimates: Arc::new(Mutex::new(HashMap::new())),
        video: request.mode != "audio",
        audio_format: request.audio_format.clone(),
    };
    let last_error = Arc::new(Mutex::new(None::<String>));
    manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id.clone(), child.clone());

    let stdout_reader = stdout.map(|stdout| {
        read_output(
            app.clone(),
            manager.inner().clone(),
            id.clone(),
            stdout,
            Some(downloaded_files.clone()),
            Some(storage_tracker.clone()),
            last_error.clone(),
        )
    });
    let stderr_reader = stderr.map(|stderr| {
        read_output(
            app.clone(),
            manager.inner().clone(),
            id.clone(),
            stderr,
            Some(downloaded_files.clone()),
            Some(storage_tracker.clone()),
            last_error.clone(),
        )
    });

    let manager = manager.inner().clone();
    let monitor_app = app.clone();
    let monitor_id = id.clone();
    let monitored_output_dir = storage_tracker.output_dir.clone();
    let output_format = if request.mode == "audio" {
        request.audio_format.clone()
    } else {
        "mp4".into()
    };
    let ffmpeg_path = ffmpeg
        .path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("ffmpeg"));
    thread::spawn(move || {
        let mut active_child = child;
        let mut stdout_reader = stdout_reader;
        let mut stderr_reader = stderr_reader;
        let mut retry_attempt = 0_u8;
        let mut last_disk_check = Instant::now();
        let mut low_disk = false;
        loop {
            if !low_disk && last_disk_check.elapsed() >= Duration::from_secs(1) {
                last_disk_check = Instant::now();
                if available_disk_space(&monitored_output_dir)
                    .map(low_disk_guard_triggered)
                    .unwrap_or(true)
                {
                    low_disk = true;
                    if let Ok(mut error) = last_error.lock() {
                        *error = Some("Недостатньо вільного місця. Процес зупинено, щоб зберегти резерв 500 МіБ.".into());
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
                let youtube_403 = is_youtube
                    && !status.success()
                    && !low_disk
                    && failure_message
                        .as_deref()
                        .map(|message| message.contains("403"))
                        .unwrap_or(false);
                if youtube_403 && !cancelled && retry_attempt < 2 {
                    retry_attempt += 1;
                    let retry_message = if retry_attempt == 1 {
                        "YouTube перервав потік. Оновлюємо посилання й продовжуємо…"
                    } else {
                        "Перемикаємо спосіб отримання відео й продовжуємо…"
                    };
                    let _ = monitor_app.emit(
                        "download-event",
                        DownloadEvent {
                            id: monitor_id.clone(),
                            kind: "retrying".into(),
                            percent: None,
                            speed: None,
                            eta: None,
                            message: Some(retry_message.into()),
                            storage: None,
                            outputs: None,
                            title: None,
                            thumbnail: None,
                            uploader: None,
                            extractor: None,
                            error_code: None,
                        },
                    );
                    if let Ok(mut error) = last_error.lock() {
                        *error = None;
                    }
                    let mut retry_command = Command::new(&retry_program);
                    configure_command(&mut retry_command);
                    retry_command
                        .args(&retry_args)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped());
                    if retry_attempt == 2 {
                        retry_command.args([
                            "--extractor-args",
                            "youtube:player_client=web_embedded,android_vr",
                        ]);
                    }
                    #[cfg(unix)]
                    retry_command.process_group(0);

                    match retry_command.spawn() {
                        Ok(mut retry_child) => {
                            let retry_stdout = retry_child.stdout.take();
                            let retry_stderr = retry_child.stderr.take();
                            active_child = Arc::new(Mutex::new(retry_child));
                            manager.jobs.lock().ok().map(|mut jobs| {
                                jobs.insert(monitor_id.clone(), active_child.clone())
                            });
                            stdout_reader = retry_stdout.map(|stdout| {
                                read_output(
                                    monitor_app.clone(),
                                    manager.clone(),
                                    monitor_id.clone(),
                                    stdout,
                                    Some(downloaded_files.clone()),
                                    Some(storage_tracker.clone()),
                                    last_error.clone(),
                                )
                            });
                            stderr_reader = retry_stderr.map(|stderr| {
                                read_output(
                                    monitor_app.clone(),
                                    manager.clone(),
                                    monitor_id.clone(),
                                    stderr,
                                    Some(downloaded_files.clone()),
                                    Some(storage_tracker.clone()),
                                    last_error.clone(),
                                )
                            });
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
                let auth_required = manager
                    .auth_required
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id))
                    .unwrap_or(false);
                let mut conversion_error = None;
                let mut completed_outputs = Vec::new();
                if !cancelled && !low_disk && !auth_required {
                    let files = downloaded_files
                        .lock()
                        .ok()
                        .map(|files| files.clone())
                        .unwrap_or_default();
                    if files.is_empty() && status.success() {
                        conversion_error = Some(
                            "yt-dlp завершився, але не повідомив шлях до завантаженого файла"
                                .into(),
                        );
                    } else if !files.is_empty() {
                        for (index, file) in files.iter().enumerate() {
                            match convert_downloaded_media(
                                &monitor_app,
                                &manager,
                                &monitor_id,
                                &ffmpeg_path,
                                file,
                                (index, files.len()),
                                &output_format,
                            ) {
                                Ok(output) => {
                                    let size = fs::metadata(&output)
                                        .map(|metadata| metadata.len())
                                        .unwrap_or(0);
                                    completed_outputs.push(DownloadOutput {
                                        path: output.to_string_lossy().into_owned(),
                                        size,
                                    });
                                }
                                Err(error) => {
                                    eprintln!(
                                        "Не вдалося конвертувати {}: {error}",
                                        file.display()
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
                manager
                    .jobs
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id));
                manager
                    .cancelled
                    .lock()
                    .ok()
                    .map(|mut jobs| jobs.remove(&monitor_id));
                let kind = if cancelled {
                    "cancelled"
                } else if auth_required && !status.success() {
                    "auth_required"
                } else if status.success() && conversion_error.is_none() {
                    "completed"
                } else {
                    "failed"
                };
                let raw_error = conversion_error.clone().or_else(|| {
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
                let message = if conversion_error.is_some() {
                    raw_error.as_deref().map(friendly_conversion_error)
                } else if low_disk {
                    raw_error.clone()
                } else {
                    raw_error.as_deref().map(friendly_download_error)
                };
                let error_code = match kind {
                    "completed" => None,
                    "cancelled" => Some("cancelled".into()),
                    "auth_required" => Some("auth_required".into()),
                    _ => Some(download_error_code(raw_error.as_deref().unwrap_or_default()).into()),
                };
                let _ = monitor_app.emit(
                    "download-event",
                    DownloadEvent {
                        id: monitor_id,
                        kind: kind.into(),
                        percent: (status.success() && conversion_error.is_none()).then_some(100.0),
                        speed: None,
                        eta: None,
                        message,
                        storage: None,
                        outputs: (!completed_outputs.is_empty()).then_some(completed_outputs),
                        title: None,
                        thumbnail: None,
                        uploader: None,
                        extractor: None,
                        error_code,
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
fn cancel_download(manager: State<'_, DownloadManager>, id: String) -> Result<(), String> {
    let child = manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| "Активне завантаження не знайдено".to_string())?;
    manager
        .cancelled
        .lock()
        .map_err(|_| "Внутрішня помилка черги".to_string())?
        .insert(id);
    stop_child_process_group(&child)
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
    stop_all_downloads(app.state::<DownloadManager>().inner());
    let _ = update_sleep_prevention(app.state::<SleepPrevention>().inner(), false);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .manage(DownloadManager::default())
        .manage(ProbeManager::default())
        .manage(HistoryThumbnailCache::default())
        .manage(RuntimeMaintenance::default())
        .manage(SleepPrevention::default())
        .manage(AppExitConfirmation::default())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            runtime_status,
            install_ytdlp,
            install_ffmpeg,
            install_deno,
            update_ytdlp,
            probe_url,
            check_vpn,
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
            cache_history_thumbnail,
            clear_history_thumbnail_cache,
            delete_history_thumbnail
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                request_app_close_confirmation(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
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
    fn parses_exact_selected_format_size() {
        let metadata = serde_json::json!({
            "id": "video",
            "duration": 100.0,
            "height": 1080,
            "fps": 30,
            "requested_formats": [
                {"filesize": 1_000_000, "height": 1080, "fps": 30},
                {"filesize": 200_000, "abr": 128}
            ]
        });
        let (estimate, confidence) = selected_media_from_value(&metadata);
        assert_eq!(confidence, SizeConfidence::Exact);
        assert_eq!(estimate.unwrap().source_size, 1_200_000);
    }

    #[test]
    fn marks_approximate_and_unknown_sizes() {
        let approximate = serde_json::json!({
            "id": "approx",
            "duration": 10.0,
            "requested_formats": [{"filesize_approx": 500_000}]
        });
        let unknown = serde_json::json!({
            "id": "unknown",
            "duration": 10.0,
            "requested_formats": [{"format_id": "mystery"}]
        });
        assert_eq!(
            selected_media_from_value(&approximate).1,
            SizeConfidence::Approximate
        );
        assert_eq!(
            selected_media_from_value(&unknown).1,
            SizeConfidence::Unknown
        );
        assert!(selected_media_from_value(&unknown).0.is_none());
    }

    #[test]
    fn finds_named_asset_in_multi_file_checksum_list() {
        let checksums = b"aaaaaaaa  other.zip\nbbbbbbbb *ffmpeg-master-latest-win64-gpl.zip\n";
        assert_eq!(
            checksum_for_asset(checksums, "ffmpeg-master-latest-win64-gpl.zip").unwrap(),
            "bbbbbbbb"
        );
        assert!(checksum_for_asset(checksums, "missing.zip").is_err());
    }

    #[test]
    fn reports_available_space_for_existing_directory() {
        assert!(available_disk_space(&std::env::temp_dir()).is_some());
    }

    #[test]
    fn history_file_status_tracks_existing_and_missing_files() {
        let fixture_id = Uuid::new_v4();
        let existing = std::env::temp_dir().join(format!("yt-dlp-bd-{fixture_id}.mp4"));
        fs::write(&existing, b"video").expect("history fixture");
        let missing = std::env::temp_dir().join(format!("yt-dlp-bd-{fixture_id}-missing.mp4"));
        let statuses = inspect_history_files(vec![
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
    fn playlist_estimate_scales_only_sample_duration() {
        assert_eq!(scale_playlist_estimate(Some(100), 12, false), Some(1200));
        assert_eq!(scale_playlist_estimate(Some(1200), 12, true), Some(1200));
        assert_eq!(scale_playlist_estimate(Some(u64::MAX), 2, false), None);
    }

    #[test]
    fn recognizes_common_macos_and_windows_vpn_interfaces() {
        assert!(is_tunnel_interface("utun4"));
        assert!(is_tunnel_interface("WireGuard Tunnel"));
        assert!(is_tunnel_interface("NordLynx"));
        assert!(is_tunnel_interface("ProtonVPN"));
        assert!(!is_tunnel_interface("Ethernet"));
    }

    #[test]
    fn reads_release_version_from_redirected_download_url() {
        let release = Url::parse(
            "https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/SHA2-256SUMS",
        )
        .unwrap();
        let unresolved =
            Url::parse("https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS")
                .unwrap();
        let malformed =
            Url::parse("https://github.com/yt-dlp/yt-dlp/download/2026.07.04/SHA2-256SUMS")
                .unwrap();
        let missing_file =
            Url::parse("https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/").unwrap();
        let non_github = Url::parse(
            "https://example.com/yt-dlp/yt-dlp/releases/download/2026.07.04/SHA2-256SUMS",
        )
        .unwrap();
        assert_eq!(
            release_version_from_download_url(&release).as_deref(),
            Some("2026.07.04")
        );
        assert_eq!(release_version_from_download_url(&unresolved), None);
        assert_eq!(release_version_from_download_url(&malformed), None);
        assert_eq!(release_version_from_download_url(&missing_file), None);
        assert_eq!(release_version_from_download_url(&non_github), None);
    }

    #[test]
    fn required_space_includes_protected_reserve_and_handles_overflow() {
        assert_eq!(
            required_space_with_reserve(2_000, 13_000),
            Some(15_000 + MIN_FREE_SPACE_RESERVE)
        );
        assert_eq!(required_space_with_reserve(u64::MAX, 1), None);
    }

    #[test]
    fn runtime_disk_guard_triggers_before_protected_reserve() {
        assert!(low_disk_guard_triggered(DISK_GUARD_TRIGGER));
        assert!(low_disk_guard_triggered(MIN_FREE_SPACE_RESERVE));
        assert!(!low_disk_guard_triggered(DISK_GUARD_TRIGGER + 1));
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
    fn selector_and_playlist_intent_are_explicit() {
        assert_eq!(
            format_selector("video", "1080"),
            "bestvideo[height<=1080]+bestaudio/best[height<=1080]"
        );
        assert_eq!(format_selector("audio", "best"), "bestaudio/best");
        assert_eq!(playlist_flag(false), "--no-playlist");
        assert_eq!(playlist_flag(true), "--yes-playlist");
    }

    #[test]
    fn friendly_errors_hide_raw_extractor_diagnostics() {
        assert!(
            friendly_download_error("[youtube:truncated_id] incomplete youtube id")
                .contains("неповне")
        );
        assert!(friendly_download_error("HTTP Error 403: Forbidden").contains("відхилив"));
        assert!(friendly_download_error("No space left on device").contains("вільного місця"));
        let conversion = friendly_conversion_error(
            "/Users/name/Downloads/private.mkv: Invalid data found when processing input",
        );
        assert!(conversion.contains("пошкоджений"));
        assert!(!conversion.contains("/Users/name"));
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
