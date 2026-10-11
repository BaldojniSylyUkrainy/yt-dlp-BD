//! Serialize process handoffs with cancellation. A stale retry decision must
//! never create an untracked child after Stop has returned successfully.
use super::*;

pub(crate) type Spawned = (
    Arc<Mutex<Child>>,
    Option<std::process::ChildStdout>,
    Option<std::process::ChildStderr>,
);

pub(crate) fn spawn(
    manager: &DownloadManager,
    id: &str,
    command: &mut Command,
) -> Result<Spawned, String> {
    let _handoff = manager
        .handoff
        .lock()
        .map_err(|_| "Внутрішня помилка процесу")?;
    if download_was_cancelled(manager, id) {
        return Err("Завантаження скасовано".into());
    }
    let mut jobs = manager.jobs.lock().map_err(|_| "Внутрішня помилка черги")?;
    let mut process = command.spawn().map_err(|e| e.to_string())?;
    let stdout = process.stdout.take();
    let stderr = process.stderr.take();
    let child = Arc::new(Mutex::new(process));
    jobs.insert(id.into(), child.clone());
    Ok((child, stdout, stderr))
}

pub(crate) fn cancel(manager: &DownloadManager, id: &str) -> Result<(), String> {
    let _handoff = manager
        .handoff
        .lock()
        .map_err(|_| "Внутрішня помилка процесу")?;
    let child = manager
        .jobs
        .lock()
        .map_err(|_| "Внутрішня помилка черги")?
        .get(id)
        .cloned()
        .ok_or("Активне завантаження не знайдено")?;
    manager
        .cancelled
        .lock()
        .map_err(|_| "Внутрішня помилка черги")?
        .insert(id.into());
    stop_child_process_group(&child)
}

pub(crate) fn finish(manager: &DownloadManager, id: &str) -> bool {
    let _handoff = manager.handoff.lock().unwrap_or_else(|e| e.into_inner());
    manager
        .jobs
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    manager
        .cancelled
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sleeper() -> Command {
        #[cfg(unix)]
        let mut command = Command::new("/bin/sleep");
        #[cfg(unix)]
        {
            command.arg("10").process_group(0);
        }
        #[cfg(target_os = "windows")]
        let mut command = Command::new("powershell.exe");
        #[cfg(target_os = "windows")]
        {
            command.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 10",
            ]);
        }
        configure_command(&mut command);
        command
    }
    #[test]
    fn concurrent_stop_and_handoff_never_leave_a_running_replacement() {
        for _ in 0..8 {
            let manager = DownloadManager::default();
            let (old, _, _) = spawn(&manager, "job", &mut sleeper()).unwrap();
            stop_child_process_group(&old).unwrap();
            old.lock().unwrap().wait().unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let other_manager = manager.clone();
            let other_barrier = barrier.clone();
            let retry = thread::spawn(move || {
                other_barrier.wait();
                spawn(&other_manager, "job", &mut sleeper())
            });
            barrier.wait();
            cancel(&manager, "job").unwrap();
            if let Ok((replacement, _, _)) = retry.join().unwrap() {
                assert!(!replacement.lock().unwrap().wait().unwrap().success());
            }
            assert!(download_was_cancelled(&manager, "job"));
            assert!(finish(&manager, "job"));
            assert!(manager.jobs.lock().unwrap().is_empty());
        }
    }
    #[test]
    fn stop_before_retry_or_conversion_prevents_a_new_child() {
        let manager = DownloadManager::default();
        let (old, _, _) = spawn(&manager, "job", &mut sleeper()).unwrap();
        stop_child_process_group(&old).unwrap();
        old.lock().unwrap().wait().unwrap();
        // Same boundary as a finished yt-dlp followed by retry/FFmpeg spawn.
        cancel(&manager, "job").unwrap();
        assert!(spawn(&manager, "job", &mut sleeper()).is_err());
    }
    #[test]
    fn stop_after_handoff_kills_the_replacement_not_the_old_child() {
        let manager = DownloadManager::default();
        let (old, _, _) = spawn(&manager, "job", &mut sleeper()).unwrap();
        stop_child_process_group(&old).unwrap();
        old.lock().unwrap().wait().unwrap();
        let (replacement, _, _) = spawn(&manager, "job", &mut sleeper()).unwrap();
        cancel(&manager, "job").unwrap();
        assert!(!replacement.lock().unwrap().wait().unwrap().success());
        assert!(spawn(&manager, "job", &mut sleeper()).is_err());
    }
}
