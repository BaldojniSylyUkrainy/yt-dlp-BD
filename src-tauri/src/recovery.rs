//! Explicit, ledger-based recovery. Never scan or clean arbitrary user folders.
use super::*;

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct InterruptedWork {
    pub id: String,
    pub directory: PathBuf,
}

fn ledger(app: &AppHandle) -> Result<PathBuf, String> {
    let path = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("interrupted-work");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}
pub(crate) fn record(app: &AppHandle, id: &str, directory: &Path) -> Result<(), String> {
    let work = InterruptedWork {
        id: Uuid::parse_str(id).map_err(|e| e.to_string())?.to_string(),
        directory: fs::canonicalize(directory).map_err(|e| e.to_string())?,
    };
    fs::write(
        ledger(app)?.join(format!("{id}.json")),
        serde_json::to_vec(&work).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
pub(crate) fn finish(app: &AppHandle, id: &str, directory: &Path) {
    if !directory.exists() {
        if let Ok(path) = ledger(app) {
            let _ = fs::remove_file(path.join(format!("{id}.json")));
        }
    }
}
fn valid(work: &InterruptedWork) -> bool {
    Uuid::parse_str(&work.id).is_ok()
        && work
            .directory
            .file_name()
            .is_some_and(|name| name == OsStr::new(&format!(".yt-dlp-bd-{}.tmp", work.id)))
        && fs::symlink_metadata(&work.directory)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && fs::canonicalize(&work.directory).is_ok_and(|path| path == work.directory)
}
#[tauri::command]
pub(crate) async fn interrupted_work(
    app: AppHandle,
    manager: State<'_, DownloadManager>,
) -> Result<Vec<InterruptedWork>, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || interrupted_work_inner(app, manager))
        .await
        .map_err(|e| e.to_string())?
}
fn interrupted_work_inner(
    app: AppHandle,
    manager: DownloadManager,
) -> Result<Vec<InterruptedWork>, String> {
    let active: HashSet<String> = manager
        .jobs
        .lock()
        .map_err(|e| e.to_string())?
        .keys()
        .cloned()
        .collect();
    let mut result = Vec::new();
    for entry in fs::read_dir(ledger(&app)?)
        .map_err(|e| e.to_string())?
        .flatten()
        .take(1000)
    {
        if entry.metadata().is_ok_and(|m| m.len() <= 8192) {
            if let Some(work) = fs::read(entry.path())
                .ok()
                .and_then(|b| serde_json::from_slice::<InterruptedWork>(&b).ok())
            {
                if valid(&work) && !active.contains(&work.id) {
                    result.push(work);
                }
            }
        }
    }
    Ok(result)
}
#[tauri::command]
pub(crate) async fn remove_interrupted_work(
    app: AppHandle,
    manager: State<'_, DownloadManager>,
    id: String,
) -> Result<(), String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || remove_interrupted_work_inner(app, manager, id))
        .await
        .map_err(|e| e.to_string())?
}
fn remove_interrupted_work_inner(
    app: AppHandle,
    manager: DownloadManager,
    id: String,
) -> Result<(), String> {
    let id = Uuid::parse_str(&id)
        .map_err(|_| "Некоректний ідентифікатор")?
        .to_string();
    let active = manager.jobs.lock().map_err(|e| e.to_string())?;
    if active.contains_key(&id) {
        return Err("Це завантаження ще працює".into());
    }
    let path = ledger(&app)?.join(format!("{id}.json"));
    let work: InterruptedWork =
        serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if work.id != id || !valid(&work) {
        return Err("Не вдалося підтвердити власника тимчасових файлів".into());
    }
    fs::remove_dir_all(&work.directory).map_err(|e| e.to_string())?;
    fs::remove_file(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unowned_folder() {
        assert!(!valid(&InterruptedWork {
            id: Uuid::new_v4().to_string(),
            directory: std::env::temp_dir()
        }));
    }
}
