//! Small successful-probe cache, invalidated by executable/file identity and age.
use super::*;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Identity(PathBuf, u64, SystemTime);
pub(crate) fn identity(path: &Path) -> Option<Identity> {
    let meta = fs::metadata(path).ok()?;
    Some(Identity(
        path.to_path_buf(),
        meta.len(),
        meta.modified().ok()?,
    ))
}
pub(crate) fn manifest_fresh(checked: Instant) -> bool {
    checked.elapsed() < Duration::from_secs(3600)
}

type Versions = HashMap<(Identity, Vec<String>), (Instant, String)>;
pub(crate) fn version(
    path: &Path,
    args: &[&str],
    probe: impl FnOnce() -> Option<String>,
) -> Option<String> {
    static CACHE: OnceLock<Mutex<Versions>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = identity(path).map(|id| (id, args.iter().map(|s| s.to_string()).collect()));
    if let Some(key) = &key {
        if let Some((time, value)) = cache.lock().ok()?.get(key) {
            if time.elapsed() < Duration::from_secs(3600) {
                return Some(value.clone());
            }
        }
    }
    let value = probe()?; // Never cache transient failures.
    if let Some(key) = key {
        let mut cache = cache.lock().ok()?;
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert(key, (Instant::now(), value.clone()));
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_refreshes_without_an_app_restart() {
        assert!(manifest_fresh(Instant::now()));
        assert!(!manifest_fresh(Instant::now() - Duration::from_secs(3601)));
    }
    #[test]
    fn identity_and_arguments_control_cache() {
        let path = std::env::temp_dir().join(format!("probe-cache-{}", Uuid::new_v4()));
        fs::write(&path, b"one").unwrap();
        assert_eq!(
            version(&path, &["-v"], || Some("1".into())),
            Some("1".into())
        );
        assert_eq!(
            version(&path, &["-v"], || panic!("cached")),
            Some("1".into())
        );
        assert_eq!(version(&path, &["--version"], || None), None);
        fs::write(&path, b"changed").unwrap();
        assert_eq!(
            version(&path, &["-v"], || Some("2".into())),
            Some("2".into())
        );
        fs::remove_file(path).unwrap();
    }
}
