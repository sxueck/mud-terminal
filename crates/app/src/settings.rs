pub use mud_ui::settings::{Dialog, Edit, Preferences};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub fn config_path() -> Option<PathBuf> {
    if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("mud/settings.json"))
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|p| p.join("Library/Application Support/mud/settings.json"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
            .map(|p| p.join("mud/settings.json"))
    }
}

pub fn load(path: &Path) -> Result<Preferences, String> {
    match fs::read_to_string(path) {
        Ok(json) => {
            let prefs: Preferences =
                serde_json::from_str(&json).map_err(|e| format!("Settings: {e}"))?;
            prefs.validate()?;
            Ok(prefs)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Preferences::default()),
        Err(e) => Err(format!("Cannot read settings: {e}")),
    }
}

pub fn save(path: &Path, prefs: &Preferences) -> Result<(), String> {
    prefs.validate()?;
    // Never overwrite a configuration we could not decode or validate.
    load(path)?;
    let parent = path.parent().ok_or("Settings directory is unavailable")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|e| format!("Cannot create settings draft: {e}"))?;
    let result = (|| {
        let json = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;
        file.write_all(&json)
            .and_then(|()| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temporary, path).map_err(|e| format!("Cannot save settings: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use mud_agent::LaunchKind;
    #[test]
    fn settings_cancel_does_not_mutate_saved_preferences() {
        let prefs = Preferences::default();
        let mut dialog = Dialog::new(&prefs);
        dialog.edit(Edit::Launch(LaunchKind::Agent));
        assert_eq!(prefs.launch_kind, LaunchKind::Shell);
        assert_eq!(dialog.draft.launch_kind, LaunchKind::Agent);
        dialog.edit(Edit::Search("shell".into()));
        assert_eq!(dialog.search, "shell");
    }
    #[test]
    fn failed_save_preserves_corrupt_settings_and_unowned_drafts() {
        let root = std::env::temp_dir().join(format!("mud-save-guards-{}", std::process::id()));
        fs::create_dir_all(&root).expect("directory");
        let path = root.join("settings.json");
        fs::write(&path, "broken").expect("corrupt settings");
        assert!(save(&path, &Preferences::default()).is_err());
        assert_eq!(fs::read_to_string(&path).expect("original"), "broken");
        fs::remove_file(&path).expect("remove test fixture");
        let draft = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&draft, "in-flight").expect("existing draft");
        assert!(save(&path, &Preferences::default()).is_err());
        assert_eq!(
            fs::read_to_string(draft).expect("draft preserved"),
            "in-flight"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
    #[test]
    fn persisted_settings_roundtrip_and_reject_invalid_confidence() {
        let root = std::env::temp_dir().join(format!("mud-settings-test-{}", std::process::id()));
        let path = root.join("settings.json");
        let prefs = Preferences {
            launch_kind: LaunchKind::Agent,
            ..Preferences::default()
        };
        save(&path, &prefs).expect("save");
        assert_eq!(load(&path).expect("load").launch_kind, LaunchKind::Agent);
        save(&path, &Preferences::default()).expect("replace settings");
        assert_eq!(
            load(&path).expect("load replacement").launch_kind,
            LaunchKind::Shell
        );
        assert!(
            Preferences {
                confidence: f32::NAN,
                ..prefs
            }
            .validate()
            .is_err()
        );
        fs::remove_dir_all(root).expect("test cleanup");
    }
}
