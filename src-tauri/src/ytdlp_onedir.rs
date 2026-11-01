//! macOS runs yt-dlp from an unpacked folder. The single-file build unpacks
//! itself on every launch, and macOS then rescans the new files: about 18 s per
//! start, against 0.4 s for a folder that has already been scanned once.
use super::*;

pub(crate) const ONEDIR_DIRECTORY: &str = "yt-dlp-onedir";
pub(crate) const ONEDIR_EXECUTABLE: &str = "yt-dlp_macos";
pub(crate) const EXECUTABLE_STAMP: &str = ".yt-dlp-onedir.sha256";
const MAX_ENTRIES: usize = 4096;
const MAX_UNPACKED_BYTES: u64 = 1024 * 1024 * 1024;
const STAGE_PREFIX: &str = ".yt-dlp-onedir-stage-";
const BACKUP_PREFIX: &str = ".yt-dlp-onedir-previous-";

/// Unpack into `stage`. The archive digest is already verified; these limits
/// still refuse anything that could leave the folder or fill the disk.
pub(crate) fn extract(archive: &Path, stage: &Path) -> Result<(), String> {
    const INVALID: &str = "Архів yt-dlp має неочікувану структуру";
    let file = fs::File::open(archive).map_err(|error| format!("Архів yt-dlp: {error}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|error| format!("Архів yt-dlp: {error}"))?;
    if archive.len() > MAX_ENTRIES {
        return Err(INVALID.into());
    }
    let mut remaining = MAX_UNPACKED_BYTES;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Архів yt-dlp: {error}"))?;
        let relative = entry.enclosed_name().ok_or(INVALID)?;
        if entry.is_symlink() {
            return Err(INVALID.into());
        }
        let destination = stage.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&destination).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::File::create(&destination).map_err(|error| error.to_string())?;
        let copied = std::io::copy(&mut (&mut entry).take(remaining + 1), &mut output)
            .map_err(|error| format!("Архів yt-dlp: {error}"))?;
        if copied > remaining {
            return Err(INVALID.into());
        }
        remaining -= copied;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode & 0o755))
                .map_err(|error| error.to_string())?;
        }
    }
    if !stage.join(ONEDIR_EXECUTABLE).is_file() {
        return Err(INVALID.into());
    }
    Ok(())
}

/// Replace the folder as a whole, restoring the previous one on failure.
pub(crate) fn activate(stage: &Path, destination: &Path) -> Result<(), String> {
    let backup = destination.with_file_name(format!("{BACKUP_PREFIX}{}", Uuid::new_v4()));
    let replacing = destination.exists();
    if replacing {
        fs::rename(destination, &backup)
            .map_err(|error| format!("Не вдалося підготувати заміну yt-dlp: {error}"))?;
    }
    if let Err(error) = fs::rename(stage, destination) {
        if replacing {
            if let Err(restore) = fs::rename(&backup, destination) {
                return Err(format!(
                    "Не вдалося встановити yt-dlp: {error}. Попередню версію збережено в {}: {restore}",
                    backup.display()
                ));
            }
        }
        return Err(format!("Не вдалося встановити yt-dlp: {error}"));
    }
    if replacing {
        let _ = fs::remove_dir_all(backup);
    }
    Ok(())
}

pub(crate) fn new_stage(runtime: &Path) -> Result<PathBuf, String> {
    remove_abandoned_folders(runtime);
    let stage = runtime.join(format!("{STAGE_PREFIX}{}", Uuid::new_v4()));
    fs::create_dir(&stage).map_err(|error| error.to_string())?;
    Ok(stage)
}

fn remove_abandoned_folders(runtime: &Path) {
    let Ok(entries) = fs::read_dir(runtime) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(STAGE_PREFIX) || name.starts_with(BACKUP_PREFIX) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn archive(directory: &Path, build: impl FnOnce(&mut ZipWriter<fs::File>)) -> PathBuf {
        let path = directory.join("archive.zip");
        let mut writer = ZipWriter::new(fs::File::create(&path).unwrap());
        build(&mut writer);
        writer.finish().unwrap();
        path
    }

    fn add(writer: &mut ZipWriter<fs::File>, name: &str, mode: u32) {
        writer
            .start_file(name, SimpleFileOptions::default().unix_permissions(mode))
            .unwrap();
        writer.write_all(b"content").unwrap();
    }

    #[test]
    fn extracts_nested_files_and_keeps_the_executable_bit() {
        let directory = tempfile::tempdir().unwrap();
        let path = archive(directory.path(), |writer| {
            add(writer, ONEDIR_EXECUTABLE, 0o755);
            add(writer, "_internal/base_library.zip", 0o644);
        });
        let stage = directory.path().join("stage");
        fs::create_dir(&stage).unwrap();
        extract(&path, &stage).unwrap();
        assert!(stage.join("_internal/base_library.zip").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(stage.join(ONEDIR_EXECUTABLE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o111, 0o111);
        }
    }

    #[test]
    fn refuses_escaping_paths_and_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let stage = directory.path().join("stage");
        fs::create_dir(&stage).unwrap();
        let escaping = archive(directory.path(), |writer| {
            add(writer, "../outside", 0o644);
            add(writer, ONEDIR_EXECUTABLE, 0o755);
        });
        assert!(extract(&escaping, &stage).is_err());
        assert!(!directory.path().join("outside").exists());
        let linked = archive(directory.path(), |writer| {
            writer
                .add_symlink("_internal/link", "/etc", SimpleFileOptions::default())
                .unwrap();
            add(writer, ONEDIR_EXECUTABLE, 0o755);
        });
        assert!(extract(&linked, &stage).is_err());
    }

    #[test]
    fn refuses_an_archive_without_the_executable() {
        let directory = tempfile::tempdir().unwrap();
        let stage = directory.path().join("stage");
        fs::create_dir(&stage).unwrap();
        let path = archive(directory.path(), |writer| add(writer, "other", 0o644));
        assert!(extract(&path, &stage).is_err());
    }

    #[test]
    fn activation_replaces_the_folder_and_failure_keeps_the_old_one() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join(ONEDIR_DIRECTORY);
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("version"), "old").unwrap();

        assert!(activate(&directory.path().join("missing"), &destination).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("version")).unwrap(),
            "old"
        );

        let stage = new_stage(directory.path()).unwrap();
        fs::write(stage.join("version"), "new").unwrap();
        activate(&stage, &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("version")).unwrap(),
            "new"
        );
        let names: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(names, [OsString::from(ONEDIR_DIRECTORY)]);
    }

    /// Real archive: `YT_DLP_ONEDIR_ZIP=/path/yt-dlp_macos.zip cargo test -- --ignored onedir`
    #[test]
    #[ignore = "needs a real yt-dlp_macos.zip"]
    fn real_onedir_archive_runs_after_activation() {
        let archive = PathBuf::from(std::env::var("YT_DLP_ONEDIR_ZIP").unwrap());
        let runtime = tempfile::tempdir().unwrap();
        let stage = new_stage(runtime.path()).unwrap();
        extract(&archive, &stage).unwrap();
        let probe = |path: &Path| {
            let started = Instant::now();
            let output = Command::new(path).arg("--version").output().unwrap();
            assert!(output.status.success());
            (
                String::from_utf8_lossy(&output.stdout).trim().to_string(),
                started.elapsed(),
            )
        };
        let (version, first) = probe(&stage.join(ONEDIR_EXECUTABLE));
        let destination = runtime.path().join(ONEDIR_DIRECTORY);
        activate(&stage, &destination).unwrap();
        let (after_move, second) = probe(&destination.join(ONEDIR_EXECUTABLE));
        let (_, third) = probe(&destination.join(ONEDIR_EXECUTABLE));
        eprintln!("version={version} first={first:?} after_activation={second:?} third={third:?}");
        assert_eq!(version, after_move);
    }
}
