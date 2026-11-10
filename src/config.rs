use crate::model::{now_secs, AppConfig, Profile};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_config_path() -> PathBuf {
    ProjectDirs::from("dev", "ricaud", "T1 Control")
        .map(|dirs| dirs.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("config.json")
}

pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> AppConfig {
        let Ok(raw) = fs::read_to_string(&self.path) else {
            return AppConfig::default();
        };
        match serde_json::from_str::<AppConfig>(&raw) {
            Ok(mut config) => {
                config.validate();
                config
            }
            Err(error) => {
                eprintln!(
                    "[t1-control] ignoring invalid config {}: {error}",
                    self.path.display()
                );
                AppConfig::default()
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        }
        let temporary = self.path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(config)?;
        fs::write(&temporary, json).with_context(|| format!("writing {}", temporary.display()))?;
        match fs::rename(&temporary, &self.path) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Windows does not replace an existing file with rename.
                let _ = fs::remove_file(&self.path);
                fs::rename(&temporary, &self.path)
                    .with_context(|| format!("replacing {}", self.path.display()))
            }
        }
    }

    pub fn export_profile(&self, path: &Path, profile: &Profile) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, serde_json::to_string_pretty(profile)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn import_profile(&self, path: &Path, existing: &AppConfig) -> Result<Profile> {
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut profile: Profile =
            serde_json::from_str(&raw).with_context(|| "not a T1 Control profile")?;
        let mut wrapper = AppConfig {
            version: crate::model::CONFIG_VERSION,
            active_profile_id: profile.id.clone(),
            profiles: vec![profile.clone()],
            devices: existing.devices.clone(),
            ui: existing.ui.clone(),
        };
        wrapper.validate();
        profile = wrapper.profiles.remove(0);
        if existing.profiles.iter().any(|item| item.id == profile.id) {
            profile.id = format!("imported-{}", now_secs());
        }
        if profile.name.trim().is_empty() {
            profile.name = "Imported profile".into();
        }
        profile.created_at = now_secs();
        profile.updated_at = now_secs();
        Ok(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets::{default_config, demo_config};

    #[test]
    fn defaults_are_valid() {
        let config = default_config();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.active_profile().lighting.zones.len(), 3);
        assert_eq!(config.active_profile().cooling.channels.len(), 2);
    }

    #[test]
    fn round_trips_through_json() {
        let mut config = demo_config();
        config.active_profile_mut().lighting.global.brightness = 91;
        let json = serde_json::to_string(&config).unwrap();
        let mut parsed: AppConfig = serde_json::from_str(&json).unwrap();
        parsed.validate();
        assert_eq!(parsed.active_profile().lighting.global.brightness, 91);
        assert!(parsed.devices.demo_mode);
    }

    #[test]
    fn saves_loads_and_imports_profiles() {
        let dir = std::env::temp_dir().join(format!("t1-control-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let store = ConfigStore::new(dir.join("config.json"));
        let mut config = default_config();
        config.active_profile_mut().lighting.global.brightness = 64;
        store.save(&config).unwrap();
        let loaded = store.load();
        assert_eq!(loaded.active_profile().lighting.global.brightness, 64);

        let export = dir.join("exported.json");
        store
            .export_profile(&export, loaded.active_profile())
            .unwrap();
        let mut other = loaded.clone();
        other.profiles.clear();
        let imported = store.import_profile(&export, &other).unwrap();
        assert_eq!(imported.name, loaded.active_profile().name);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clamps_out_of_range_values() {
        let mut config = default_config();
        let profile = config.active_profile_mut();
        profile.lighting.global.brightness = 250;
        profile.lighting.zones[0].led_count = 9999;
        profile.cooling.channels[0].max_rpm = 9000;
        profile.cooling.channels[0].curve.points.clear();
        config.validate();
        let profile = config.active_profile();
        assert_eq!(profile.lighting.global.brightness, 100);
        assert_eq!(profile.lighting.zones[0].led_count, 512);
        assert_eq!(profile.cooling.channels[0].max_rpm, 6000);
        assert!(profile.cooling.channels[0].curve.points.len() >= 2);
    }
}
