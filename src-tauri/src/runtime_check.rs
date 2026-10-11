//! Read-only update discovery; installing still uses the exclusive runtime guard.
use super::*;

#[tauri::command]
pub(crate) async fn runtime_update_available(
    app: AppHandle,
    maintenance: State<'_, RuntimeMaintenance>,
    component: String,
) -> Result<bool, String> {
    let directory = runtime_dir(&app)?;
    if component == "ytDlp" {
        let (bytes, final_url) = tokio::time::timeout(
            Duration::from_secs(8),
            fetch_bytes_with_final_url(&download_client()?, YT_DLP_NIGHTLY_RELEASE_API),
        )
        .await
        .map_err(|_| "Перевірка nightly не відповіла вчасно")??;
        if final_url.as_str() != YT_DLP_NIGHTLY_RELEASE_API {
            return Err("Неочікуване перенаправлення nightly".into());
        }
        let release: GithubRelease =
            serde_json::from_slice(&bytes).map_err(|_| "Некоректна відповідь nightly")?;
        let version = fs::read_to_string(directory.join(".yt-dlp.version")).ok();
        ensure_ytdlp_nightly_not_rollback(&release.tag_name, version.as_deref().map(str::trim))?;
        let asset = select_nightly_asset(
            &release,
            if cfg!(windows) {
                "yt-dlp.exe"
            } else {
                "yt-dlp_macos"
            },
        )?;
        let digest = github_asset_sha256(&asset)?;
        let accepted = fs::read_to_string(directory.join(".yt-dlp.sha256")).ok();
        let path = yt_dlp_path(&app)?;
        let actual = tauri::async_runtime::spawn_blocking(move || sha256_file(&path).ok())
            .await
            .map_err(|e| e.to_string())?;
        return Ok(!verified_nightly_is_current(
            &release.tag_name,
            version.as_deref(),
            &digest,
            accepted.as_deref(),
            actual.as_deref(),
        ));
    }
    let manifest = runtime_platform_manifest(&app, maintenance.inner()).await?;
    let matches = |stamp: &str, hash: &str| {
        fs::read_to_string(directory.join(stamp)).is_ok_and(|v| v.trim() == hash)
    };
    match component.as_str() {
        "deno" => Ok(!matches(".deno.sha256", &manifest.deno.sha256)),
        "ffmpeg" => {
            if let Some(archive) = manifest.ffmpeg.archive {
                return Ok(!matches(".ffmpeg.sha256", &archive.sha256));
            }
            Ok(!manifest
                .ffmpeg
                .ffmpeg
                .is_some_and(|a| matches(".ffmpeg.sha256", &a.sha256))
                || !manifest
                    .ffmpeg
                    .ffprobe
                    .is_some_and(|a| matches(".ffprobe.sha256", &a.sha256)))
        }
        _ => Err("Невідомий компонент".into()),
    }
}
