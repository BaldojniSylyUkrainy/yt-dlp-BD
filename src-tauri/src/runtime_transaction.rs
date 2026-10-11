//! Install a prepared runtime set together. Backups survive until every binary
//! and stamp is installed; failed rollback deliberately preserves recovery data.
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn commit(files: &[(PathBuf, PathBuf)]) -> Result<(), String> {
    commit_with(files, |source, destination| fs::rename(source, destination))
}

fn commit_with(
    files: &[(PathBuf, PathBuf)],
    mut install: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<(), String> {
    let mut installed: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for (source, destination) in files {
        let backup = destination.with_extension(format!("previous-{}", uuid::Uuid::new_v4()));
        let previous = if destination.exists() {
            if let Err(error) = fs::rename(destination, &backup) {
                return Err(rollback(&installed, error.to_string()));
            }
            Some(backup)
        } else {
            None
        };
        // Include this entry before install so a failed install restores it too.
        installed.push((destination.clone(), previous));
        if let Err(error) = install(source, destination) {
            return Err(rollback(&installed, error.to_string()));
        }
    }
    for (_, backup) in installed {
        if let Some(backup) = backup {
            let _ = fs::remove_file(backup);
        }
    }
    Ok(())
}

fn rollback(installed: &[(PathBuf, Option<PathBuf>)], error: String) -> String {
    let mut message = format!("Не вдалося оновити комплект FFmpeg: {error}");
    for (destination, backup) in installed.iter().rev() {
        if destination.exists() {
            if let Err(error) = fs::remove_file(destination) {
                message.push_str(&format!("; відкат {}: {error}", destination.display()));
                continue;
            }
        }
        if let Some(backup) = backup {
            if let Err(error) = fs::rename(backup, destination) {
                message.push_str(&format!(
                    "; попередній файл збережено в {}: {error}",
                    backup.display()
                ));
            }
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn any_failed_binary_or_stamp_rolls_back_the_whole_set() {
        for fail_at in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            let files: Vec<_> = (0..4)
                .map(|i| {
                    let old = dir.path().join(format!("runtime-{i}"));
                    let new = dir.path().join(format!("staged-{i}"));
                    fs::write(&old, format!("old-{i}")).unwrap();
                    fs::write(&new, format!("new-{i}")).unwrap();
                    (new, old)
                })
                .collect();
            let mut index = 0;
            assert!(commit_with(&files, |a, b| {
                let current = index;
                index += 1;
                if current == fail_at {
                    Err(std::io::Error::other("injected failure"))
                } else {
                    fs::rename(a, b)
                }
            })
            .is_err());
            for (i, (_, old)) in files.iter().enumerate() {
                assert_eq!(fs::read_to_string(old).unwrap(), format!("old-{i}"));
            }
        }
    }
    #[test]
    fn failed_first_install_removes_only_newly_installed_members() {
        let dir = tempfile::tempdir().unwrap();
        let files: Vec<_> = (0..2)
            .map(|i| {
                let source = dir.path().join(format!("new-{i}"));
                fs::write(&source, "new").unwrap();
                (source, dir.path().join(format!("runtime-{i}")))
            })
            .collect();
        assert!(commit_with(&files, |source, destination| {
            if destination == files[1].1 {
                Err(std::io::Error::other("locked"))
            } else {
                fs::rename(source, destination)
            }
        })
        .is_err());
        assert!(files.iter().all(|(_, path)| !path.exists()));
    }
    #[test]
    fn successful_pair_and_stamps_are_committed() {
        let dir = tempfile::tempdir().unwrap();
        let new = dir.path().join("new");
        let old = dir.path().join("old");
        fs::write(&new, "new").unwrap();
        fs::write(&old, "old").unwrap();
        commit(&[(new, old.clone())]).unwrap();
        assert_eq!(fs::read_to_string(old).unwrap(), "new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
