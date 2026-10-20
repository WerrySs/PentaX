// SPDX-License-Identifier: GPL-2.0-or-later

use pentax_config::{ConfigError, ConfigStore, GameOverrides, TitleId};

#[derive(Debug)]
pub struct SavedProfile {
    pub title: TitleId,
    pub durability_warning: Option<String>,
}

/// Binds editable overrides to the exact title whose complete document was read.
#[derive(Default)]
pub struct ProfileEditor {
    pub title: String,
    pub document: GameOverrides,
    loaded_for: Option<TitleId>,
}

impl ProfileEditor {
    pub fn is_loaded(&self) -> bool {
        self.loaded_for
            .as_ref()
            .is_some_and(|title| title.as_str() == self.title)
    }

    pub fn invalidate(&mut self) {
        self.loaded_for = None;
    }

    pub fn load(&mut self, store: &ConfigStore) -> Result<(), String> {
        self.invalidate();
        let title = TitleId::new(self.title.clone()).map_err(|error| error.to_string())?;
        let document = store
            .load_game(&title)
            .map_err(|error| error.to_string())?
            .unwrap_or_default();
        self.document = document;
        self.loaded_for = Some(title);
        Ok(())
    }

    pub fn save(&self, store: &ConfigStore) -> Result<SavedProfile, String> {
        if !self.is_loaded() {
            return Err(
                "Load this title profile before saving. Its existing overrides will be preserved."
                    .into(),
            );
        }
        let title = TitleId::new(self.title.clone()).map_err(|error| error.to_string())?;
        committed_profile(title.clone(), store.save_game(&title, &self.document))
    }
}

fn committed_profile(
    title: TitleId,
    result: Result<(), ConfigError>,
) -> Result<SavedProfile, String> {
    match result {
        Ok(()) => Ok(SavedProfile {
            title,
            durability_warning: None,
        }),
        Err(error @ ConfigError::CommittedButNotDurable { .. }) => Ok(SavedProfile {
            title,
            durability_warning: Some(error.to_string()),
        }),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pentax_config::Resolution;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Temporary(PathBuf);
    impl Temporary {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "pentax-profile-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn entering_a_valid_title_id_cannot_overwrite_an_unloaded_profile() {
        let directory = Temporary::new();
        let store = ConfigStore::new(&directory.0);
        let title = TitleId::new("PPSA12345").unwrap();
        let mut existing = GameOverrides::default();
        existing.runtime.cpu_threads = Some(12);
        store.save_game(&title, &existing).unwrap();
        let original = fs::read(store.game_path(&title)).unwrap();
        let editor = ProfileEditor {
            title: title.to_string(),
            ..Default::default()
        };
        assert!(
            editor
                .save(&store)
                .unwrap_err()
                .contains("Load this title profile")
        );
        assert_eq!(fs::read(store.game_path(&title)).unwrap(), original);
    }

    #[test]
    fn editing_visible_preferences_preserves_hidden_overrides_and_binds_title_id() {
        let directory = Temporary::new();
        let store = ConfigStore::new(&directory.0);
        let title = TitleId::new("PPSA12345").unwrap();
        let mut existing = GameOverrides::default();
        existing.graphics.resolution = Some(Resolution {
            width: 3840,
            height: 2160,
        });
        existing.graphics.vsync = Some(false);
        existing.runtime.cpu_threads = Some(12);
        store.save_game(&title, &existing).unwrap();
        let mut editor = ProfileEditor {
            title: title.to_string(),
            ..Default::default()
        };
        editor.load(&store).unwrap();
        editor.document.runtime.enable_debug_logging = Some(true);
        editor.save(&store).unwrap();
        let saved = store.load_game(&title).unwrap().unwrap();
        assert_eq!(saved.graphics, existing.graphics);
        assert_eq!(saved.runtime.cpu_threads, Some(12));
        assert_eq!(saved.runtime.enable_debug_logging, Some(true));
        editor.title = "PPSA54321".into();
        assert!(editor.save(&store).is_err());
        assert!(
            !store
                .game_path(&TitleId::new("PPSA54321").unwrap())
                .exists()
        );
    }

    #[test]
    fn a_committed_profile_remains_associated_when_directory_sync_fails() {
        let title = TitleId::new("PPSA12345").unwrap();
        let outcome = committed_profile(
            title.clone(),
            Err(ConfigError::CommittedButNotDurable {
                path: PathBuf::from("games/PPSA12345.toml"),
                source: std::io::Error::other("directory flush failed"),
            }),
        )
        .unwrap();
        assert_eq!(outcome.title, title);
        assert!(
            outcome
                .durability_warning
                .unwrap()
                .contains("directory flush failed")
        );
    }
}
