//! Managed yt-dlp, FFmpeg and Deno: signed manifest, downloads, verification and
//! replacement. Working components survive every failure on this path.
use super::*;

pub(crate) const RUNTIME_MANIFEST_URL: &str = "https://github.com/BaldojniSylyUkrainy/yt-dlp-BD/releases/latest/download/runtime-components.json";

pub(crate) const RUNTIME_MANIFEST_SIGNATURE_URL: &str = "https://github.com/BaldojniSylyUkrainy/yt-dlp-BD/releases/latest/download/runtime-components.json.sig";

pub(crate) const YT_DLP_NIGHTLY_RELEASE_API: &str =
    "https://api.github.com/repos/yt-dlp/yt-dlp-nightly-builds/releases/latest";

pub(crate) const MAX_RUNTIME_METADATA_BYTES: usize = 1024 * 1024;

pub(crate) const MAX_YT_DLP_BINARY_BYTES: u64 = 128 * 1024 * 1024;

pub(crate) const MAX_DENO_ARCHIVE_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) const MAX_FFMPEG_ARCHIVE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub(crate) const RUNTIME_RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);

pub(crate) const RUNTIME_DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const YT_DLP_VERSION_PROBE_TIMEOUT: Duration = Duration::from_secs(180);

pub(crate) const TAURI_CONFIG_SOURCE: &str = include_str!("../tauri.conf.json");

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentStatus {
    pub(crate) installed: bool,
    pub(crate) version: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) managed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeStatus {
    pub(crate) yt_dlp: ComponentStatus,
    pub(crate) ffmpeg: ComponentStatus,
    pub(crate) deno: ComponentStatus,
    pub(crate) runtime_dir: String,
    pub(crate) platform: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeInstallProgress {
    pub(crate) component: String,
    pub(crate) downloaded: u64,
    pub(crate) total: Option<u64>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeAssetManifest {
    pub(crate) version: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeFfmpegManifest {
    pub(crate) version: String,
    pub(crate) archive: Option<RuntimeAssetManifest>,
    pub(crate) ffmpeg: Option<RuntimeAssetManifest>,
    pub(crate) ffprobe: Option<RuntimeAssetManifest>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimePlatformManifest {
    pub(crate) yt_dlp: RuntimeAssetManifest,
    pub(crate) deno: RuntimeAssetManifest,
    pub(crate) ffmpeg: RuntimeFfmpegManifest,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeManifest {
    pub(crate) schema_version: u32,
    pub(crate) release_version: String,
    pub(crate) generated_at: String,
    pub(crate) platforms: HashMap<String, RuntimePlatformManifest>,
}

#[derive(Deserialize)]
pub(crate) struct GithubRelease {
    pub(crate) tag_name: String,
    pub(crate) assets: Vec<GithubReleaseAsset>,
}

#[derive(Clone, Deserialize)]
pub(crate) struct GithubReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) digest: Option<String>,
    pub(crate) size: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeManifestState {
    pub(crate) release_version: String,
    pub(crate) generated_at: String,
}

#[derive(Default)]
pub(crate) struct RuntimeMaintenance {
    pub(crate) operation: tokio::sync::Mutex<()>,
    pub(crate) manifest: tokio::sync::Mutex<Option<(Instant, RuntimePlatformManifest)>>,
}

pub(crate) struct RuntimeProcessLock {
    pub(crate) file: fs::File,
}

pub(crate) fn ensure_runtime_idle(manager: &DownloadManager) -> Result<(), String> {
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

pub(crate) fn acquire_runtime_process_lock(app: &AppHandle) -> Result<RuntimeProcessLock, String> {
    let path = runtime_dir(app)?.join(".runtime-maintenance.lock");
    try_lock_runtime_file(&path)
}

pub(crate) fn try_lock_runtime_file(path: &Path) -> Result<RuntimeProcessLock, String> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("Не вдалося відкрити блокування runtime: {error}"))?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            "Інша копія застосунку вже обслуговує runtime-компоненти".to_string()
        } else {
            format!("Не вдалося заблокувати runtime-компоненти: {error}")
        }
    })?;
    Ok(RuntimeProcessLock { file })
}

pub(crate) fn runtime_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_local_data_dir()
        .map_err(|error| format!("Не вдалося знайти папку застосунку: {error}"))?
        .join("runtime");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Не вдалося створити runtime-папку: {error}"))?;
    Ok(directory)
}

pub(crate) fn yt_dlp_single_file_path(app: &AppHandle) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    let filename = "yt-dlp.exe";
    #[cfg(not(target_os = "windows"))]
    let filename = "yt-dlp";
    Ok(runtime_dir(app)?.join(filename))
}

pub(crate) fn yt_dlp_onedir_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_dir(app)?.join(ytdlp_onedir::ONEDIR_DIRECTORY))
}

/// The installed folder when there is one, otherwise the single-file bootstrap.
pub(crate) fn yt_dlp_path(app: &AppHandle) -> Result<PathBuf, String> {
    if cfg!(target_os = "macos") {
        let executable = yt_dlp_onedir_path(app)?.join(ytdlp_onedir::ONEDIR_EXECUTABLE);
        if executable.is_file() {
            return Ok(executable);
        }
    }
    yt_dlp_single_file_path(app)
}

/// Digest of the release asset behind the installed yt-dlp, if the files agree.
/// A folder has no single asset file, so its launcher hash vouches for it.
pub(crate) fn installed_ytdlp_asset_digest(app: &AppHandle) -> Result<Option<String>, String> {
    let directory = runtime_dir(app)?;
    if cfg!(target_os = "macos") {
        let executable = yt_dlp_onedir_path(app)?.join(ytdlp_onedir::ONEDIR_EXECUTABLE);
        if executable.is_file() {
            let vouched = fs::read_to_string(directory.join(ytdlp_onedir::EXECUTABLE_STAMP))
                .is_ok_and(|stamp| sha256_file(&executable).is_ok_and(|hash| hash == stamp.trim()));
            return Ok(vouched
                .then(|| fs::read_to_string(directory.join(".yt-dlp.sha256")).ok())
                .flatten()
                .map(|digest| digest.trim().to_string()));
        }
    }
    Ok(sha256_file(&yt_dlp_single_file_path(app)?).ok())
}

pub(crate) fn managed_binary_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.into()
    }
}

/// The app's own FFmpeg pair. Status probes may time out under load; existing
/// files are still usable, and a system-wide `ffmpeg` is never a substitute.
pub(crate) fn managed_ffmpeg_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = runtime_dir(app)?;
    let ffmpeg = directory.join(managed_binary_name("ffmpeg"));
    if ffmpeg.is_file() && directory.join(managed_binary_name("ffprobe")).is_file() {
        Ok(ffmpeg)
    } else {
        Err(
            "Компонент FFmpeg недоступний. Натисніть «Повторити» біля нього в боковій панелі"
                .into(),
        )
    }
}

pub(crate) fn managed_deno_path(app: &AppHandle) -> Result<Option<PathBuf>, String> {
    let deno = runtime_dir(app)?.join(managed_binary_name("deno"));
    Ok(deno.is_file().then_some(deno))
}

/// yt-dlp extractor modules bundled with the app: (installed filename, source, label).
pub(crate) const YT_DLP_PLUGIN_MODULES: [(&str, &str, &str); 3] = [
    (
        "discovery_baldojnyi.py",
        DISCOVERY_EXTRACTOR_SOURCE,
        "модуль пошуку",
    ),
    (
        "threads_baldojnyi.py",
        THREADS_EXTRACTOR_SOURCE,
        "Threads-модуль",
    ),
    (
        "instagram_baldojnyi.py",
        INSTAGRAM_PHOTOS_SOURCE,
        "Instagram-модуль",
    ),
];

pub(crate) fn install_plugin_module(
    extractor_directory: &Path,
    filename: &str,
    source: &str,
    label: &str,
) -> Result<(), String> {
    let destination = extractor_directory.join(filename);
    if fs::read(&destination).ok().as_deref() == Some(source.as_bytes()) {
        return Ok(());
    }
    let temporary = extractor_directory.join(format!(".{filename}-{}.tmp", Uuid::new_v4()));
    fs::write(&temporary, source)
        .map_err(|error| format!("Не вдалося записати {label}: {error}"))?;
    if let Err(error) = replace_runtime_file(&temporary, &destination, label) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn install_threads_plugin_at(plugin_root: &Path) -> Result<PathBuf, String> {
    let extractor_directory = plugin_root
        .join("baldojnyi")
        .join("yt_dlp_plugins")
        .join("extractor");
    fs::create_dir_all(&extractor_directory)
        .map_err(|error| format!("Не вдалося підготувати модулі yt-dlp: {error}"))?;
    for (filename, source, label) in YT_DLP_PLUGIN_MODULES {
        install_plugin_module(&extractor_directory, filename, source, label)?;
    }
    Ok(plugin_root.to_path_buf())
}

pub(crate) fn threads_plugin_dir(app: &AppHandle) -> Result<PathBuf, String> {
    static INSTALL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = INSTALL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "Не вдалося підготувати Threads-модуль".to_string())?;
    install_threads_plugin_at(&runtime_dir(app)?.join("plugins"))
}

pub(crate) fn configure_command(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

pub(crate) fn configure_tokio_command(command: &mut tokio::process::Command) {
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    #[cfg(not(target_os = "windows"))]
    let _ = command;
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_powershell_script(script: &str) -> String {
    format!("{WINDOWS_POWERSHELL_UTF8_PREFIX}{script}")
}

pub(crate) fn command_version(path: &Path, args: &[&str]) -> Option<String> {
    probe_cache::version(path, args, || command_version_uncached(path, args))
}

pub(crate) fn command_version_uncached(path: &Path, args: &[&str]) -> Option<String> {
    let mut command = Command::new(path);
    configure_command(&mut command);
    let output = process_runner::capture(command.args(args), Duration::from_secs(5)).ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

pub(crate) async fn command_version_bounded(
    path: &Path,
    args: &[&str],
    timeout_duration: Duration,
) -> Result<Option<String>, String> {
    let mut command = tokio::process::Command::new(path);
    configure_tokio_command(&mut command);
    command.args(args).kill_on_drop(true);
    let output = tokio::time::timeout(timeout_duration, command.output())
        .await
        .map_err(|_| "Перевірка нового yt-dlp тривала надто довго".to_string())?
        .map_err(|error| format!("Не вдалося запустити новий yt-dlp: {error}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!text.is_empty()).then_some(text))
}

pub(crate) fn parsed_yt_dlp_version(value: &str) -> Option<Vec<u64>> {
    let parts = value
        .trim()
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    (parts.len() == 3 || parts.len() == 4).then_some(parts)
}

pub(crate) fn ensure_ytdlp_nightly_not_rollback(
    incoming: &str,
    accepted: Option<&str>,
) -> Result<(), String> {
    let incoming = parsed_yt_dlp_version(incoming)
        .filter(|parts| parts.len() == 4)
        .ok_or_else(|| "Офіційний nightly має неправильну версію".to_string())?;
    let Some(accepted) = accepted.and_then(parsed_yt_dlp_version) else {
        return Ok(());
    };
    if incoming < accepted {
        return Err("Офіційний nightly старіший за вже прийняту версію".into());
    }
    Ok(())
}

pub(crate) fn ensure_ytdlp_bootstrap_not_rollback(
    incoming: &str,
    accepted: Option<&str>,
    installed: bool,
) -> Result<(), String> {
    if !installed {
        return Ok(());
    }
    let Some(incoming) = parsed_yt_dlp_version(incoming) else {
        return Err("Підписаний yt-dlp має неправильну версію".into());
    };
    let Some(accepted) = accepted.and_then(parsed_yt_dlp_version) else {
        return Ok(());
    };
    if incoming < accepted {
        return Err("Підписаний yt-dlp старіший за вже прийняту версію".into());
    }
    Ok(())
}

pub(crate) fn verified_nightly_is_current(
    release_tag: &str,
    accepted_version: Option<&str>,
    github_digest: &str,
    accepted_digest: Option<&str>,
    installed_digest: Option<&str>,
) -> bool {
    accepted_version.is_some_and(|version| version.trim() == release_tag)
        && accepted_digest.is_some_and(|digest| digest.trim() == github_digest)
        && installed_digest == Some(github_digest)
}

pub(crate) fn github_asset_sha256(asset: &GithubReleaseAsset) -> Result<String, String> {
    let digest = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .ok_or_else(|| format!("GitHub не надав SHA-256 для {}", asset.name))?;
    if digest.len() != 64 || !digest.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(format!(
            "GitHub надав неправильний SHA-256 для {}",
            asset.name
        ));
    }
    Ok(digest.to_ascii_lowercase())
}

pub(crate) fn validate_nightly_asset_url(
    asset: &GithubReleaseAsset,
    tag: &str,
) -> Result<(), String> {
    let url = Url::parse(&asset.browser_download_url)
        .map_err(|_| format!("Nightly містить неправильний URL для {}", asset.name))?;
    let expected_path = format!(
        "/yt-dlp/yt-dlp-nightly-builds/releases/download/{tag}/{}",
        asset.name
    );
    if url.scheme() != "https"
        || url.host_str() != Some("github.com")
        || url.path() != expected_path
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "Nightly містить небезпечний URL для {}",
            asset.name
        ));
    }
    Ok(())
}

pub(crate) fn select_nightly_asset(
    release: &GithubRelease,
    name: &str,
) -> Result<GithubReleaseAsset, String> {
    if parsed_yt_dlp_version(&release.tag_name)
        .filter(|parts| parts.len() == 4)
        .is_none()
    {
        return Err("Офіційний nightly має неправильну версію".into());
    }
    let mut matches = release.assets.iter().filter(|asset| asset.name == name);
    let asset = matches
        .next()
        .cloned()
        .ok_or_else(|| format!("Офіційний nightly не містить {name}"))?;
    if matches.next().is_some() {
        return Err(format!("Офіційний nightly містить кілька {name}"));
    }
    validate_nightly_asset_url(&asset, &release.tag_name)?;
    github_asset_sha256(&asset)?;
    Ok(asset)
}

pub(crate) fn nightly_checksum_for_asset(
    checksum_file: &[u8],
    asset_name: &str,
) -> Result<String, String> {
    let checksums = std::str::from_utf8(checksum_file)
        .map_err(|_| "Офіційний файл SHA-256 має неправильний формат".to_string())?;
    let matches = checksums
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            Some((parts.next()?, parts.next()?))
        })
        .filter_map(|(hash, filename)| {
            let normalized = filename.trim_start_matches('*').trim_start_matches("./");
            (normalized == asset_name
                && hash.len() == 64
                && hash.chars().all(|value| value.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(format!(
            "Офіційний файл SHA-256 містить неправильний запис для {asset_name}"
        ));
    }
    Ok(matches[0].clone())
}

pub(crate) fn nightly_asset_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "yt-dlp.exe"
    } else {
        "yt-dlp_macos.zip"
    }
}

pub(crate) fn yt_dlp_nightly_temporary_path(app: &AppHandle) -> Result<PathBuf, String> {
    let filename = if cfg!(target_os = "windows") {
        ".yt-dlp-nightly-download.exe"
    } else {
        ".yt-dlp-nightly-download.zip"
    };
    Ok(runtime_dir(app)?.join(filename))
}

pub(crate) fn find_ffmpeg(app: &AppHandle) -> Result<ComponentStatus, String> {
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

pub(crate) fn find_deno(app: &AppHandle) -> Result<ComponentStatus, String> {
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

pub(crate) fn runtime_status_blocking(app: &AppHandle) -> Result<RuntimeStatus, String> {
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
pub(crate) async fn runtime_status(app: AppHandle) -> Result<RuntimeStatus, String> {
    tauri::async_runtime::spawn_blocking(move || runtime_status_blocking(&app))
        .await
        .map_err(|error| format!("Не вдалося перевірити локальні компоненти: {error}"))?
}

pub(crate) async fn fetch_bytes_with_final_url(
    client: &reqwest::Client,
    url: &str,
) -> Result<(Vec<u8>, Url), String> {
    let mut response = tokio::time::timeout(
        RUNTIME_RESPONSE_TIMEOUT,
        client
            .get(url)
            .header("User-Agent", "yt-dlp-desktop")
            .send(),
    )
    .await
    .map_err(|_| "Сервер runtime-компонентів не відповів вчасно".to_string())?
    .map_err(|error| format!("Помилка мережі: {error}"))?;
    if response.status().as_u16() == 429
        || (response.status().as_u16() == 403
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .is_some_and(|v| v == "0"))
    {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let header_number = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        };
        let retry_at = header_number("x-ratelimit-reset")
            .unwrap_or(now + header_number("retry-after").unwrap_or(3600))
            .clamp(now + 60, now + 86400);
        return Err(format!("GitHub тимчасово обмежив перевірку оновлень. Встановлений компонент залишається доступним. [retry_at={retry_at}]"));
    }
    response = response
        .error_for_status()
        .map_err(|error| format!("Сервер повернув помилку: {error}"))?;
    let final_url = response.url().clone();
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RUNTIME_METADATA_BYTES as u64)
    {
        return Err("Службовий runtime-файл перевищує дозволений розмір".into());
    }
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::time::timeout(RUNTIME_DOWNLOAD_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| "Службовий runtime-файл перестав завантажуватися".to_string())?
            .map_err(|error| format!("Не вдалося прочитати відповідь: {error}"))?;
        let Some(chunk) = chunk else { break };
        if bytes.len().saturating_add(chunk.len()) > MAX_RUNTIME_METADATA_BYTES {
            return Err("Службовий runtime-файл перевищує дозволений розмір".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, final_url))
}

pub(crate) async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    Ok(fetch_bytes_with_final_url(client, url).await?.0)
}

pub(crate) fn download_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| format!("Не вдалося підготувати завантаження: {error}"))
}

pub(crate) fn verify_runtime_manifest_signature(
    manifest: &[u8],
    encoded_signature: &[u8],
) -> Result<(), String> {
    let config: serde_json::Value = serde_json::from_str(TAURI_CONFIG_SOURCE)
        .map_err(|_| "Вбудована конфігурація updater пошкоджена".to_string())?;
    let encoded_public_key = config
        .pointer("/plugins/updater/pubkey")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "У конфігурації updater немає публічного ключа".to_string())?;
    let public_key_text = BASE64_STANDARD
        .decode(encoded_public_key.trim())
        .map_err(|_| "Публічний ключ updater має неправильний формат".to_string())?;
    let public_key_text = std::str::from_utf8(&public_key_text)
        .map_err(|_| "Публічний ключ updater має неправильне кодування".to_string())?;
    let public_key = MinisignPublicKey::decode(public_key_text)
        .map_err(|_| "Не вдалося прочитати публічний ключ updater".to_string())?;

    let encoded_signature = std::str::from_utf8(encoded_signature)
        .map_err(|_| "Підпис manifest має неправильне кодування".to_string())?;
    let signature_text = BASE64_STANDARD
        .decode(encoded_signature.trim())
        .map_err(|_| "Підпис manifest має неправильний формат".to_string())?;
    let signature_text = std::str::from_utf8(&signature_text)
        .map_err(|_| "Підпис manifest має неправильне кодування".to_string())?;
    let signature = MinisignSignature::decode(signature_text)
        .map_err(|_| "Не вдалося прочитати підпис runtime manifest".to_string())?;
    public_key
        .verify(manifest, &signature, true)
        .map_err(|_| "Підпис runtime manifest не пройшов перевірку".to_string())
}

pub(crate) fn validate_runtime_asset(
    asset: &RuntimeAssetManifest,
    label: &str,
) -> Result<(), String> {
    if asset.version.trim().is_empty() {
        return Err(format!("Runtime manifest не містить версію {label}"));
    }
    if asset.sha256.len() != 64 || !asset.sha256.chars().all(|value| value.is_ascii_hexdigit()) {
        return Err(format!(
            "Runtime manifest містить неправильний SHA-256 для {label}"
        ));
    }
    let url = Url::parse(&asset.url)
        .map_err(|_| format!("Runtime manifest містить неправильний URL для {label}"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(format!(
            "Runtime manifest містить небезпечний URL для {label}"
        ));
    }
    let allowed = match url.host_str() {
        Some("github.com") => {
            url.path().starts_with("/yt-dlp/yt-dlp/releases/download/")
                || url
                    .path()
                    .starts_with("/yt-dlp/yt-dlp-nightly-builds/releases/download/")
                || url
                    .path()
                    .starts_with("/yt-dlp/FFmpeg-Builds/releases/download/")
                || url.path().starts_with("/denoland/deno/releases/download/")
                || url
                    .path()
                    .starts_with("/BaldojniSylyUkrainy/yt-dlp-BD/releases/download/")
        }
        Some("ffmpeg.martin-riedl.de") => url.path().starts_with("/download/macos/arm64/"),
        _ => false,
    };
    if !allowed {
        return Err(format!(
            "Runtime manifest вказує неочікуване джерело для {label}"
        ));
    }
    Ok(())
}

pub(crate) fn require_yt_dlp_asset_source(asset: &RuntimeAssetManifest) -> Result<(), String> {
    let url = Url::parse(&asset.url)
        .map_err(|_| "Runtime manifest містить неправильний URL для yt-dlp".to_string())?;
    let allowed = url.host_str() == Some("github.com")
        && (url.path().starts_with("/yt-dlp/yt-dlp/releases/download/")
            || url
                .path()
                .starts_with("/yt-dlp/yt-dlp-nightly-builds/releases/download/"));
    if !allowed {
        return Err("Runtime manifest вказує неправильне джерело для yt-dlp".into());
    }
    Ok(())
}

pub(crate) fn require_runtime_asset_source(
    asset: &RuntimeAssetManifest,
    host: &str,
    path_prefix: &str,
    label: &str,
) -> Result<(), String> {
    let url = Url::parse(&asset.url)
        .map_err(|_| format!("Runtime manifest містить неправильний URL для {label}"))?;
    if url.host_str() != Some(host) || !url.path().starts_with(path_prefix) {
        return Err(format!(
            "Runtime manifest вказує неправильне джерело для {label}"
        ));
    }
    Ok(())
}

pub(crate) fn validate_runtime_platform_manifest(
    manifest: &RuntimePlatformManifest,
) -> Result<(), String> {
    validate_runtime_asset(&manifest.yt_dlp, "yt-dlp")?;
    require_yt_dlp_asset_source(&manifest.yt_dlp)?;
    validate_runtime_asset(&manifest.deno, "Deno")?;
    require_runtime_asset_source(
        &manifest.deno,
        "github.com",
        "/denoland/deno/releases/download/",
        "Deno",
    )?;
    if manifest.ffmpeg.version.trim().is_empty() {
        return Err("Runtime manifest не містить версію FFmpeg".into());
    }
    #[cfg(target_os = "macos")]
    {
        if manifest.ffmpeg.archive.is_some() {
            return Err("Runtime manifest містить неправильний macOS FFmpeg-пакет".into());
        }
        let ffmpeg = manifest
            .ffmpeg
            .ffmpeg
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить macOS FFmpeg".to_string())?;
        let ffprobe = manifest
            .ffmpeg
            .ffprobe
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить macOS FFprobe".to_string())?;
        validate_runtime_asset(ffmpeg, "macOS FFmpeg")?;
        require_runtime_asset_source(
            ffmpeg,
            "ffmpeg.martin-riedl.de",
            "/download/macos/arm64/",
            "macOS FFmpeg",
        )?;
        validate_runtime_asset(ffprobe, "macOS FFprobe")?;
        require_runtime_asset_source(
            ffprobe,
            "ffmpeg.martin-riedl.de",
            "/download/macos/arm64/",
            "macOS FFprobe",
        )?;
    }
    #[cfg(target_os = "windows")]
    {
        if manifest.ffmpeg.ffmpeg.is_some() || manifest.ffmpeg.ffprobe.is_some() {
            return Err("Runtime manifest містить неправильний Windows FFmpeg-пакет".into());
        }
        let archive = manifest
            .ffmpeg
            .archive
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить Windows FFmpeg".to_string())?;
        validate_runtime_asset(archive, "Windows FFmpeg")?;
        require_runtime_asset_source(
            archive,
            "github.com",
            "/BaldojniSylyUkrainy/yt-dlp-BD/releases/download/",
            "Windows FFmpeg",
        )?;
    }
    Ok(())
}

pub(crate) fn parsed_runtime_release_version(value: &str) -> Result<[u64; 4], String> {
    let parts = value
        .split('.')
        .map(|part| part.parse::<u64>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Runtime manifest має неправильну версію релізу".to_string())?;
    parts
        .try_into()
        .map_err(|_| "Runtime manifest має неправильну версію релізу".to_string())
}

pub(crate) fn validate_runtime_manifest_timestamp(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    let separators = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];
    if bytes.len() != 24
        || separators
            .iter()
            .any(|(index, expected)| bytes.get(*index) != Some(expected))
        || bytes.iter().enumerate().any(|(index, value)| {
            !separators.iter().any(|(separator, _)| *separator == index) && !value.is_ascii_digit()
        })
    {
        return Err("Runtime manifest має неправильний час генерації".into());
    }
    Ok(())
}

pub(crate) fn ensure_runtime_manifest_is_current(
    incoming: &RuntimeManifestState,
    accepted: Option<&RuntimeManifestState>,
) -> Result<(), String> {
    let incoming_version = parsed_runtime_release_version(&incoming.release_version)?;
    validate_runtime_manifest_timestamp(&incoming.generated_at)?;
    let Some(accepted) = accepted else {
        return Ok(());
    };
    let accepted_version = parsed_runtime_release_version(&accepted.release_version)
        .map_err(|_| "Збережений стан runtime manifest пошкоджено".to_string())?;
    validate_runtime_manifest_timestamp(&accepted.generated_at)
        .map_err(|_| "Збережений стан runtime manifest пошкоджено".to_string())?;
    if incoming_version < accepted_version
        || (incoming_version == accepted_version && incoming.generated_at < accepted.generated_at)
    {
        return Err("Підписаний runtime manifest старіший за вже прийняту версію".into());
    }
    Ok(())
}

pub(crate) fn runtime_manifest_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(runtime_dir(app)?.join(".runtime-manifest-state.json"))
}

pub(crate) fn read_runtime_manifest_state(
    app: &AppHandle,
) -> Result<Option<RuntimeManifestState>, String> {
    let path = runtime_manifest_state_path(app)?;
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "Збережений стан runtime manifest пошкоджено".to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Не вдалося прочитати стан runtime manifest: {error}"
        )),
    }
}

pub(crate) fn write_runtime_manifest_state(
    app: &AppHandle,
    state: &RuntimeManifestState,
) -> Result<(), String> {
    let destination = runtime_manifest_state_path(app)?;
    let temporary = destination.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
    let bytes = serde_json::to_vec(state)
        .map_err(|error| format!("Не вдалося підготувати стан runtime manifest: {error}"))?;
    fs::write(&temporary, bytes)
        .map_err(|error| format!("Не вдалося записати стан runtime manifest: {error}"))?;
    if let Err(error) = replace_runtime_file(&temporary, &destination, "стан runtime manifest")
    {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}

pub(crate) async fn runtime_platform_manifest(
    app: &AppHandle,
    maintenance: &RuntimeMaintenance,
) -> Result<RuntimePlatformManifest, String> {
    let mut cached = maintenance.manifest.lock().await;
    if let Some((checked, manifest)) = cached.as_ref() {
        if probe_cache::manifest_fresh(*checked) {
            return Ok(manifest.clone());
        }
    }
    let client = download_client()?;
    let (manifest_bytes, signature_bytes) = tokio::try_join!(
        fetch_bytes(&client, RUNTIME_MANIFEST_URL),
        fetch_bytes(&client, RUNTIME_MANIFEST_SIGNATURE_URL),
    )?;
    verify_runtime_manifest_signature(&manifest_bytes, &signature_bytes)?;
    let manifest: RuntimeManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|_| "Підписаний runtime manifest має неправильний формат".to_string())?;
    if manifest.schema_version != 1 {
        return Err("Підписаний runtime manifest має непідтримувану версію".into());
    }
    let incoming_state = RuntimeManifestState {
        release_version: manifest.release_version.clone(),
        generated_at: manifest.generated_at.clone(),
    };
    let accepted_state = read_runtime_manifest_state(app)?;
    ensure_runtime_manifest_is_current(&incoming_state, accepted_state.as_ref())?;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let platform = "darwin-aarch64";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    let platform = "windows-x86_64";
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    return Err("Підписані runtime-компоненти недоступні для цієї платформи".into());

    let platform_manifest = manifest
        .platforms
        .get(platform)
        .cloned()
        .ok_or_else(|| format!("Runtime manifest не містить платформу {platform}"))?;
    validate_runtime_platform_manifest(&platform_manifest)?;
    write_runtime_manifest_state(app, &incoming_state)?;
    cached.replace((Instant::now(), platform_manifest.clone()));
    Ok(platform_manifest)
}

pub(crate) async fn fetch_to_file(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    component: &str,
    expected_size: Option<u64>,
    maximum_size: u64,
) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;

    let mut response = tokio::time::timeout(
        RUNTIME_RESPONSE_TIMEOUT,
        client
            .get(url)
            .header("User-Agent", "yt-dlp-desktop")
            .send(),
    )
    .await
    .map_err(|_| "Сервер runtime-компонентів не відповів вчасно".to_string())?
    .map_err(|error| format!("Помилка мережі: {error}"))?
    .error_for_status()
    .map_err(|error| format!("Сервер повернув помилку: {error}"))?;
    let total = response.content_length();
    if total.is_some_and(|size| size == 0 || size > maximum_size) {
        return Err("Runtime-компонент має неочікуваний розмір".into());
    }
    if let (Some(expected), Some(actual)) = (expected_size, total) {
        if actual != expected {
            return Err("Runtime-компонент повідомив неочікуваний розмір".into());
        }
    }
    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|error| format!("Не вдалося створити тимчасовий файл: {error}"))?;
    let mut downloaded = 0_u64;
    loop {
        let chunk = tokio::time::timeout(RUNTIME_DOWNLOAD_IDLE_TIMEOUT, response.chunk())
            .await
            .map_err(|_| "Завантаження runtime-компонента перестало відповідати".to_string())?
            .map_err(|error| format!("Не вдалося прочитати завантаження: {error}"))?;
        let Some(chunk) = chunk else { break };
        let next_size = checked_runtime_download_size(
            downloaded,
            chunk.len() as u64,
            expected_size,
            maximum_size,
        )?;
        file.write_all(&chunk)
            .await
            .map_err(|error| format!("Не вдалося записати завантаження: {error}"))?;
        downloaded = next_size;
        let _ = app.emit(
            "runtime-install-progress",
            RuntimeInstallProgress {
                component: component.into(),
                downloaded,
                total,
            },
        );
    }
    if expected_size.is_some_and(|expected| downloaded != expected) {
        return Err("Runtime-компонент завантажився не повністю".into());
    }
    file.flush()
        .await
        .map_err(|error| format!("Не вдалося завершити запис завантаження: {error}"))
}

pub(crate) fn checked_runtime_download_size(
    downloaded: u64,
    chunk_size: u64,
    expected_size: Option<u64>,
    maximum_size: u64,
) -> Result<u64, String> {
    let next_size = downloaded
        .checked_add(chunk_size)
        .ok_or_else(|| "Runtime-компонент перевищує дозволений розмір".to_string())?;
    if next_size > maximum_size || expected_size.is_some_and(|expected| next_size > expected) {
        return Err("Runtime-компонент перевищує дозволений розмір".into());
    }
    Ok(next_size)
}

pub(crate) fn sha256_file(path: &Path) -> Result<String, String> {
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

pub(crate) fn verify_sha256_file_expected(
    path: &Path,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if actual != expected {
        return Err(format!("Контрольна сума {label} не збігається"));
    }
    Ok(())
}

pub(crate) fn replace_runtime_file(
    temporary: &Path,
    destination: &Path,
    label: &str,
) -> Result<(), String> {
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

pub(crate) fn write_runtime_text_atomic(
    destination: &Path,
    value: &str,
    label: &str,
) -> Result<(), String> {
    let temporary = destination.with_extension(format!("tmp-{}", Uuid::new_v4()));
    fs::write(&temporary, value)
        .map_err(|error| format!("Не вдалося зберегти {label}: {error}"))?;
    if let Err(error) = replace_runtime_file(&temporary, destination, label) {
        let _ = fs::remove_file(temporary);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn commit_ytdlp_runtime_update(
    temporary: &Path,
    destination: &Path,
    version_stamp: &Path,
    hash_stamp: &Path,
    version: &str,
    hash: &str,
    label: &str,
) -> Result<(), String> {
    replace_runtime_file(temporary, destination, label)?;
    write_runtime_text_atomic(version_stamp, version, "версію yt-dlp")?;
    write_runtime_text_atomic(hash_stamp, hash, "SHA-256 yt-dlp")
}

pub(crate) fn extract_binary_from_file(
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
                    let name = file.name().ok()?;
                    Path::new(name.as_ref())
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

pub(crate) fn install_ffmpeg_set(
    directory: &Path,
    binaries: &[(&Path, &str, PathBuf)],
    stamps: &[(PathBuf, &str)],
) -> Result<(), String> {
    let staging = tempfile::Builder::new()
        .prefix(".ffmpeg-stage-")
        .tempdir_in(directory)
        .map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (archive, name, destination) in binaries {
        let staged = staging.path().join(name);
        extract_binary_from_file(archive, name, &staged)?;
        if command_version(&staged, &["-version"]).is_none() {
            return Err(format!("Новий {name} не пройшов перевірку запуску"));
        }
        files.push((staged, destination.clone()));
    }
    for (index, (destination, digest)) in stamps.iter().enumerate() {
        let staged = staging.path().join(format!("stamp-{index}"));
        fs::write(&staged, digest).map_err(|e| e.to_string())?;
        files.push((staged, destination.clone()));
    }
    runtime_transaction::commit(&files)
}

pub(crate) async fn install_ffmpeg_inner(
    app: AppHandle,
    manifest: &RuntimePlatformManifest,
) -> Result<(), String> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let client = download_client()?;
        let ffmpeg = manifest
            .ffmpeg
            .ffmpeg
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить macOS FFmpeg".to_string())?;
        let ffprobe = manifest
            .ffmpeg
            .ffprobe
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить macOS FFprobe".to_string())?;
        let directory = runtime_dir(&app)?;
        let ffmpeg_path = directory.join("ffmpeg");
        let ffprobe_path = directory.join("ffprobe");
        let ffmpeg_stamp = directory.join(".ffmpeg.sha256");
        let ffprobe_stamp = directory.join(".ffprobe.sha256");
        let current = command_version(&ffmpeg_path, &["-version"]).is_some()
            && command_version(&ffprobe_path, &["-version"]).is_some()
            && fs::read_to_string(&ffmpeg_stamp)
                .map(|value| value.trim() == ffmpeg.sha256.as_str())
                .unwrap_or(false)
            && fs::read_to_string(&ffprobe_stamp)
                .map(|value| value.trim() == ffprobe.sha256.as_str())
                .unwrap_or(false);
        if current {
            eprintln!("Керований ffmpeg вже актуальний");
            return Ok(());
        }

        eprintln!("Завантажуємо керовані ffmpeg та ffprobe");
        let ffmpeg_archive = directory.join(".ffmpeg.zip.download");
        let ffprobe_archive = directory.join(".ffprobe.zip.download");
        let download_result = tokio::try_join!(
            fetch_to_file(
                &app,
                &client,
                &ffmpeg.url,
                &ffmpeg_archive,
                "ffmpeg",
                None,
                MAX_FFMPEG_ARCHIVE_BYTES
            ),
            fetch_to_file(
                &app,
                &client,
                &ffprobe.url,
                &ffprobe_archive,
                "ffmpeg",
                None,
                MAX_FFMPEG_ARCHIVE_BYTES
            )
        );
        if let Err(error) = download_result {
            let _ = fs::remove_file(&ffmpeg_archive);
            let _ = fs::remove_file(&ffprobe_archive);
            return Err(error);
        }
        let install_result = (|| {
            verify_sha256_file_expected(&ffmpeg_archive, &ffmpeg.sha256, "ffmpeg")?;
            verify_sha256_file_expected(&ffprobe_archive, &ffprobe.sha256, "ffprobe")?;
            install_ffmpeg_set(
                &directory,
                &[
                    (&ffmpeg_archive, "ffmpeg", ffmpeg_path),
                    (&ffprobe_archive, "ffprobe", ffprobe_path),
                ],
                &[
                    (ffmpeg_stamp, ffmpeg.sha256.as_str()),
                    (ffprobe_stamp, ffprobe.sha256.as_str()),
                ],
            )
        })();
        let _ = fs::remove_file(&ffmpeg_archive);
        let _ = fs::remove_file(&ffprobe_archive);
        install_result
    }

    #[cfg(target_os = "windows")]
    {
        let archive_asset = manifest
            .ffmpeg
            .archive
            .as_ref()
            .ok_or_else(|| "Runtime manifest не містить Windows FFmpeg".to_string())?;
        let client = download_client()?;
        let directory = runtime_dir(&app)?;
        let ffmpeg_path = directory.join("ffmpeg.exe");
        let ffprobe_path = directory.join("ffprobe.exe");
        let stamp = directory.join(".ffmpeg.sha256");
        let current = command_version(&ffmpeg_path, &["-version"]).is_some()
            && command_version(&ffprobe_path, &["-version"]).is_some()
            && fs::read_to_string(&stamp)
                .map(|value| value.trim() == archive_asset.sha256.as_str())
                .unwrap_or(false);
        if current {
            return Ok(());
        }

        eprintln!("Завантажуємо керовані FFmpeg та FFprobe для Windows");
        let archive = directory.join(".ffmpeg.zip.download");
        if let Err(error) = fetch_to_file(
            &app,
            &client,
            &archive_asset.url,
            &archive,
            "ffmpeg",
            None,
            MAX_FFMPEG_ARCHIVE_BYTES,
        )
        .await
        {
            let _ = fs::remove_file(&archive);
            return Err(error);
        }
        let install_result = (|| {
            verify_sha256_file_expected(&archive, &archive_asset.sha256, "FFmpeg")?;
            install_ffmpeg_set(
                &directory,
                &[
                    (&archive, "ffmpeg.exe", ffmpeg_path),
                    (&archive, "ffprobe.exe", ffprobe_path),
                ],
                &[(stamp, archive_asset.sha256.as_str())],
            )
        })();
        let _ = fs::remove_file(&archive);
        install_result
    }

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        target_os = "windows"
    )))]
    Err("Portable FFmpeg поки підтримує лише macOS Apple Silicon та Windows".into())
}

#[tauri::command]
pub(crate) async fn install_ffmpeg(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    let _process_lock = acquire_runtime_process_lock(&app)?;
    ensure_runtime_idle(manager.inner())?;
    let manifest = runtime_platform_manifest(&app, maintenance.inner()).await?;
    install_ffmpeg_inner(app, &manifest).await
}

pub(crate) async fn install_deno_inner(
    app: AppHandle,
    manifest: &RuntimePlatformManifest,
) -> Result<(), String> {
    let asset = &manifest.deno;
    let client = download_client()
        .map_err(|error| format!("Не вдалося підготувати завантаження Deno: {error}"))?;
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
            .map(|value| value.trim() == asset.sha256.as_str())
            .unwrap_or(false);
    if current {
        return Ok(());
    }

    let archive = directory.join(".deno.zip.download");
    if let Err(error) = fetch_to_file(
        &app,
        &client,
        &asset.url,
        &archive,
        "deno",
        None,
        MAX_DENO_ARCHIVE_BYTES,
    )
    .await
    {
        let _ = fs::remove_file(&archive);
        return Err(error);
    }
    let install_result = (|| {
        verify_sha256_file_expected(&archive, &asset.sha256, "Deno")?;
        extract_binary_from_file(&archive, filename, &destination)
    })();
    let _ = fs::remove_file(&archive);
    install_result?;
    fs::write(stamp, &asset.sha256)
        .map_err(|error| format!("Не вдалося зберегти версію Deno: {error}"))?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn install_deno(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    let _process_lock = acquire_runtime_process_lock(&app)?;
    ensure_runtime_idle(manager.inner())?;
    let manifest = runtime_platform_manifest(&app, maintenance.inner()).await?;
    install_deno_inner(app, &manifest).await
}

pub(crate) async fn install_ytdlp_inner(
    app: AppHandle,
    manifest: &RuntimePlatformManifest,
) -> Result<(), String> {
    let asset = &manifest.yt_dlp;
    let client = download_client()?;
    let destination = yt_dlp_single_file_path(&app)?;
    let version_stamp = runtime_dir(&app)?.join(".yt-dlp.version");
    let accepted_version = fs::read_to_string(&version_stamp).ok();
    ensure_ytdlp_bootstrap_not_rollback(
        &asset.version,
        accepted_version.as_deref().map(str::trim),
        destination.is_file(),
    )?;
    let current = fs::read(&destination)
        .map(|binary| format!("{:x}", Sha256::digest(binary)) == asset.sha256.as_str())
        .unwrap_or(false);
    if current {
        write_runtime_text_atomic(&version_stamp, &asset.version, "версію yt-dlp")?;
        write_runtime_text_atomic(
            &runtime_dir(&app)?.join(".yt-dlp.sha256"),
            &asset.sha256,
            "SHA-256 yt-dlp",
        )?;
        return Ok(());
    }

    eprintln!("Завантажуємо підписаний стартовий yt-dlp");
    let temporary = destination.with_extension("download");
    if let Err(error) = fetch_to_file(
        &app,
        &client,
        &asset.url,
        &temporary,
        "ytDlp",
        None,
        MAX_YT_DLP_BINARY_BYTES,
    )
    .await
    {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    let actual = sha256_file(&temporary)?;
    if actual != asset.sha256.as_str() {
        let _ = fs::remove_file(&temporary);
        return Err("Контрольна сума yt-dlp не збігається. Файл не встановлено".into());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))
            .map_err(|error| format!("Не вдалося дозволити запуск yt-dlp: {error}"))?;
    }

    commit_ytdlp_runtime_update(
        &temporary,
        &destination,
        &version_stamp,
        &runtime_dir(&app)?.join(".yt-dlp.sha256"),
        &asset.version,
        &asset.sha256,
        "yt-dlp",
    )?;

    Ok(())
}

pub(crate) async fn update_ytdlp_to_latest_nightly(app: &AppHandle) -> Result<(), String> {
    let client = download_client()?;
    let (release_bytes, final_url) = tokio::time::timeout(
        Duration::from_secs(8),
        fetch_bytes_with_final_url(&client, YT_DLP_NIGHTLY_RELEASE_API),
    )
    .await
    .map_err(|_| "Перевірка yt-dlp nightly тривала надто довго".to_string())??;
    if final_url.as_str() != YT_DLP_NIGHTLY_RELEASE_API {
        return Err("GitHub перенаправив перевірку nightly на неочікувану адресу".into());
    }
    let release: GithubRelease = serde_json::from_slice(&release_bytes)
        .map_err(|_| "GitHub повернув неправильний опис nightly".to_string())?;
    let version_stamp = runtime_dir(app)?.join(".yt-dlp.version");
    let accepted_version = fs::read_to_string(&version_stamp).ok();
    ensure_ytdlp_nightly_not_rollback(
        &release.tag_name,
        accepted_version.as_deref().map(str::trim),
    )?;
    let binary_name = nightly_asset_name();
    let binary_asset = select_nightly_asset(&release, binary_name)?;
    if binary_asset.size == 0 || binary_asset.size > MAX_YT_DLP_BINARY_BYTES {
        return Err("Офіційний nightly має неочікуваний розмір".into());
    }
    let github_binary_digest = github_asset_sha256(&binary_asset)?;
    let checksum_asset = select_nightly_asset(&release, "SHA2-256SUMS")?;
    let signature_asset = select_nightly_asset(&release, "SHA2-256SUMS.sig")?;
    if checksum_asset.size == 0
        || checksum_asset.size > MAX_RUNTIME_METADATA_BYTES as u64
        || signature_asset.size == 0
        || signature_asset.size > MAX_RUNTIME_METADATA_BYTES as u64
    {
        return Err("Офіційний файл SHA-256 має неочікуваний розмір".into());
    }

    let hash_stamp = runtime_dir(app)?.join(".yt-dlp.sha256");
    let accepted_hash = fs::read_to_string(&hash_stamp).ok();
    let installed_hash = installed_ytdlp_asset_digest(app)?;
    if verified_nightly_is_current(
        &release.tag_name,
        accepted_version.as_deref(),
        &github_binary_digest,
        accepted_hash.as_deref(),
        installed_hash.as_deref(),
    ) {
        return Ok(());
    }

    let checksum_bytes = tokio::time::timeout(
        Duration::from_secs(8),
        fetch_bytes(&client, &checksum_asset.browser_download_url),
    )
    .await
    .map_err(|_| "Перевірка SHA-256 yt-dlp тривала надто довго".to_string())??;
    if checksum_bytes.len() as u64 != checksum_asset.size {
        return Err("Офіційний файл SHA-256 завантажився не повністю".into());
    }
    let checksum_digest = format!("{:x}", Sha256::digest(&checksum_bytes));
    if checksum_digest != github_asset_sha256(&checksum_asset)? {
        return Err("GitHub SHA-256 для офіційного списку сум не збігається".into());
    }
    let signature_bytes = tokio::time::timeout(
        Duration::from_secs(8),
        fetch_bytes(&client, &signature_asset.browser_download_url),
    )
    .await
    .map_err(|_| "Перевірка підпису yt-dlp тривала надто довго".to_string())??;
    ytdlp_signature::verify_checksum_list(&checksum_bytes, &signature_bytes)?;
    let expected = nightly_checksum_for_asset(&checksum_bytes, binary_name)?;
    if expected != github_binary_digest {
        return Err("Офіційні SHA-256 nightly не узгоджені між собою".into());
    }

    if installed_hash.as_deref() == Some(expected.as_str()) {
        write_runtime_text_atomic(&hash_stamp, &expected, "SHA-256 yt-dlp")?;
        write_runtime_text_atomic(&version_stamp, &release.tag_name, "версію yt-dlp")?;
        return Ok(());
    }
    let temporary = yt_dlp_nightly_temporary_path(app)?;
    let _ = fs::remove_file(&temporary);
    if let Err(error) = fetch_to_file(
        app,
        &client,
        &binary_asset.browser_download_url,
        &temporary,
        "ytDlp",
        Some(binary_asset.size),
        MAX_YT_DLP_BINARY_BYTES,
    )
    .await
    {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if let Err(error) = verify_sha256_file_expected(&temporary, &expected, "yt-dlp nightly") {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    if cfg!(target_os = "macos") {
        let result = install_ytdlp_onedir(app, &temporary, &release.tag_name, &expected).await;
        let _ = fs::remove_file(&temporary);
        return result;
    }
    let destination = yt_dlp_single_file_path(app)?;
    let downloaded_version =
        match command_version_bounded(&temporary, &["--version"], YT_DLP_VERSION_PROBE_TIMEOUT)
            .await
        {
            Ok(version) => version,
            Err(error) => {
                let _ = fs::remove_file(&temporary);
                return Err(error);
            }
        };
    if downloaded_version.as_deref() != Some(release.tag_name.as_str()) {
        let _ = fs::remove_file(&temporary);
        return Err("Завантажений yt-dlp nightly повідомив неочікувану версію".into());
    }
    if let Err(error) = commit_ytdlp_runtime_update(
        &temporary,
        &destination,
        &version_stamp,
        &hash_stamp,
        &release.tag_name,
        &expected,
        "yt-dlp nightly",
    ) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

/// Unpack and test the new folder beside the working yt-dlp, then swap it in.
/// The version probe also lets macOS finish its scan before the first download.
pub(crate) async fn install_ytdlp_onedir(
    app: &AppHandle,
    archive: &Path,
    version: &str,
    asset_digest: &str,
) -> Result<(), String> {
    let runtime = runtime_dir(app)?;
    let stage = ytdlp_onedir::new_stage(&runtime)?;
    let result = async {
        let (archive_path, stage_path) = (archive.to_path_buf(), stage.clone());
        tauri::async_runtime::spawn_blocking(move || {
            ytdlp_onedir::extract(&archive_path, &stage_path)
        })
        .await
        .map_err(|error| error.to_string())??;
        let executable = stage.join(ytdlp_onedir::ONEDIR_EXECUTABLE);
        let reported =
            command_version_bounded(&executable, &["--version"], YT_DLP_VERSION_PROBE_TIMEOUT)
                .await?;
        if reported.as_deref() != Some(version) {
            return Err("Завантажений yt-dlp nightly повідомив неочікувану версію".to_string());
        }
        let executable_hash = sha256_file(&executable)?;
        ytdlp_onedir::activate(&stage, &yt_dlp_onedir_path(app)?)?;
        write_runtime_text_atomic(
            &runtime.join(ytdlp_onedir::EXECUTABLE_STAMP),
            &executable_hash,
            "SHA-256 yt-dlp",
        )?;
        write_runtime_text_atomic(
            &runtime.join(".yt-dlp.sha256"),
            asset_digest,
            "SHA-256 yt-dlp",
        )?;
        write_runtime_text_atomic(&runtime.join(".yt-dlp.version"), version, "версію yt-dlp")
    }
    .await;
    if stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    if result.is_ok() {
        // The folder has replaced the single-file bootstrap.
        let _ = fs::remove_file(yt_dlp_single_file_path(app)?);
    }
    result
}

/// macOS installs the folder directly and uses the signed single-file bootstrap
/// only as a fallback, for example when GitHub limits API requests.
pub(crate) async fn install_preferred_ytdlp(
    app: &AppHandle,
    maintenance: &RuntimeMaintenance,
) -> Result<(), String> {
    if cfg!(target_os = "macos") && update_ytdlp_to_latest_nightly(app).await.is_ok() {
        return Ok(());
    }
    let manifest = runtime_platform_manifest(app, maintenance).await?;
    install_ytdlp_inner(app.clone(), &manifest).await
}

#[tauri::command]
pub(crate) async fn install_ytdlp(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    let _process_lock = acquire_runtime_process_lock(&app)?;
    ensure_runtime_idle(manager.inner())?;
    install_preferred_ytdlp(&app, maintenance.inner()).await
}

#[tauri::command]
pub(crate) async fn update_ytdlp(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    manager: State<'_, DownloadManager>,
) -> Result<(), String> {
    let _operation = maintenance.operation.lock().await;
    let _process_lock = acquire_runtime_process_lock(&app)?;
    ensure_runtime_idle(manager.inner())?;
    let result = async {
        if !yt_dlp_path(&app)?.is_file() {
            return install_preferred_ytdlp(&app, maintenance.inner()).await;
        }
        update_ytdlp_to_latest_nightly(&app).await
    }
    .await;
    if let Err(error) = &result {
        eprintln!("Не вдалося оновити yt-dlp: {error}");
    }
    result
}
