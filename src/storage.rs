//! Versioned JSON with atomic replacement and conservative backup recovery.
use crate::model::{AppState, SCHEMA_VERSION};
use std::{
    fs, io,
    io::Write,
    path::{Path, PathBuf},
};

pub struct LoadOutcome {
    pub state: AppState,
    pub recovered: bool,
    pub warning: Option<String>,
}

pub fn default_path() -> PathBuf {
    data_directory(
        std::env::var_os("NIVUMI_DATA_DIR"),
        std::env::var_os("NUSA_REBUILD_DATA_DIR"),
        std::env::current_exe().ok().as_deref(),
        std::env::var_os("LOCALAPPDATA"),
        &std::env::temp_dir(),
    )
    .join("state.json")
}
fn data_directory(
    primary: Option<std::ffi::OsString>,
    legacy: Option<std::ffi::OsString>,
    executable: Option<&Path>,
    local: Option<std::ffi::OsString>,
    temporary: &Path,
) -> PathBuf {
    primary
        .or(legacy)
        .map(PathBuf::from)
        .or_else(|| executable.and_then(portable_data_dir))
        // Keep the existing profile, caches, captures and offline models in place.
        .or_else(|| local.map(|p| PathBuf::from(p).join("NusaRebuild")))
        .unwrap_or_else(|| temporary.join("NusaRebuild"))
}
fn portable_data_dir(exe: &Path) -> Option<PathBuf> {
    let directory = exe.parent()?;
    directory
        .join("portable.flag")
        .is_file()
        .then(|| directory.join("data"))
}
fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
fn read_state(path: &Path) -> io::Result<AppState> {
    let bytes = fs::read(path)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "State exceeds 32 MiB",
        ));
    }
    let mut state: AppState = serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if state.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("Unsupported state schema {}", state.schema_version),
        ));
    }
    state.normalize();
    Ok(state)
}
pub fn load(path: &Path) -> io::Result<LoadOutcome> {
    match read_state(path) {
        Ok(state) => Ok(LoadOutcome {
            state,
            recovered: false,
            warning: None,
        }),
        Err(error)
            if error.kind() == io::ErrorKind::NotFound && !sibling(path, ".bak").exists() =>
        {
            Ok(LoadOutcome {
                state: AppState::default(),
                recovered: false,
                warning: None,
            })
        }
        Err(error) if error.kind() == io::ErrorKind::Unsupported => Err(error),
        Err(error) => {
            // Never overwrite an invalid primary file while recovering it.
            let state = read_state(&sibling(path, ".bak")).map_err(|backup_error| {
                io::Error::new(
                    error.kind(),
                    format!("Cannot load state: {error}; backup: {backup_error}"),
                )
            })?;
            Ok(LoadOutcome {
                state,
                recovered: true,
                warning: Some(format!("Recovered backup; original preserved: {error}")),
            })
        }
    }
}

pub fn save(path: &Path, state: &AppState) -> io::Result<()> {
    if state.schema_version != SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Refusing to write unsupported schema",
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp = sibling(path, &format!(".{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        if path.exists() {
            match read_state(path) {
                Ok(_) => {
                    let backup_temp = sibling(path, &format!(".{}.bak.tmp", std::process::id()));
                    fs::copy(path, &backup_temp)?;
                    replace(&backup_temp, &sibling(path, ".bak"))?;
                }
                Err(error) if error.kind() == io::ErrorKind::Unsupported => return Err(error),
                Err(_) => {
                    // Keep corrupt bytes for diagnosis, even after a recovered state is saved.
                    let stamp = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    fs::copy(path, sibling(path, &format!(".corrupt-{stamp}")))?;
                }
            }
        }
        replace(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH; both buffers live for the call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "nusa-rebuild-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }
    #[test]
    fn portable_marker_selects_data_beside_executable() {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("nusa-rebuild.exe");
        assert_eq!(portable_data_dir(&exe), None);
        fs::write(dir.join("portable.flag"), b"").unwrap();
        assert_eq!(portable_data_dir(&exe), Some(dir.join("data")));
        let path = portable_data_dir(&exe).unwrap().join("state.json");
        save(&path, &AppState::default()).unwrap();
        assert!(!load(&path).unwrap().recovered);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn new_state_then_atomic_replace_and_recover() {
        let dir = test_dir();
        let path = dir.join("state.json");
        let mut state = load(&path).unwrap().state;
        save(&path, &state).unwrap();
        state.settings.language = "en".into();
        save(&path, &state).unwrap();
        assert_eq!(load(&path).unwrap().state.settings.language, "en");
        fs::write(&path, b"{broken").unwrap();
        let recovered = load(&path).unwrap();
        assert!(recovered.recovered);
        assert_eq!(recovered.state.settings.language, "pt-BR");
        assert_eq!(fs::read(&path).unwrap(), b"{broken");
        save(&path, &recovered.state).unwrap();
        assert!(fs::read_dir(&dir).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("corrupt-")));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn newer_schema_is_never_overwritten() {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(&path, br#"{"schema_version":999}"#).unwrap();
        assert_eq!(
            load(&path).err().unwrap().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(save(&path, &AppState::default()).is_err());
        assert!(String::from_utf8(fs::read(&path).unwrap())
            .unwrap()
            .contains("999"));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_settings_are_normalized_after_load() {
        let dir = test_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        fs::write(
            &path,
            br#"{"settings":{"water_start_hour":99,"water_end_hour":0,"focus_minutes":0}}"#,
        )
        .unwrap();
        let settings = load(&path).unwrap().state.settings;
        assert_eq!(
            (
                settings.water_start_hour,
                settings.water_end_hour,
                settings.focus_minutes
            ),
            (23, 24, 1)
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[cfg(test)]
mod brand_path_tests {
    use super::*;
    #[test]
    fn new_override_wins_and_legacy_profile_remains_the_default() {
        let temp = Path::new("temporary");
        let local = Some("local".into());
        assert_eq!(
            data_directory(
                Some("new".into()),
                Some("old".into()),
                None,
                local.clone(),
                temp
            ),
            PathBuf::from("new")
        );
        assert_eq!(
            data_directory(None, Some("old".into()), None, local.clone(), temp),
            PathBuf::from("old")
        );
        assert_eq!(
            data_directory(None, None, None, local, temp),
            Path::new("local").join("NusaRebuild")
        );
        assert_eq!(
            data_directory(None, None, None, None, temp),
            temp.join("NusaRebuild")
        );
    }
    #[test]
    fn rebranding_loads_old_state_and_preserves_user_content() {
        let root =
            std::env::temp_dir().join(format!("nivumi-legacy-profile-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        let mut old = serde_json::to_value(AppState::default()).unwrap();
        old["settings"]["mascot"] = true.into();
        old["settings"]["water_goal_ml"] = 2750.into();
        old["settings"]["startup"] = true.into();
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
        fs::write(root.join("capture.png"), b"preserved capture").unwrap();
        let loaded = load(&path).unwrap();
        assert_eq!(loaded.state.settings.water_goal_ml, 2750);
        assert!(loaded.state.settings.startup);
        save(&path, &loaded.state).unwrap();
        assert!(sibling(&path, ".bak").is_file());
        assert_eq!(
            fs::read(root.join("capture.png")).unwrap(),
            b"preserved capture"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
