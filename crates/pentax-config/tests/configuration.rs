use pentax_config::{
    ConfigError, ConfigStore, GameOverrides, GlobalConfig, GraphicsOverrides, Resolution,
    RuntimeOverrides, SCHEMA_VERSION, TitleId,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn system_preferences_round_trip_independently_and_reject_invalid_values() {
    let old = GlobalConfig::from_toml("schema_version = 1\n[ui]\nlanguage = 'fr-FR'\n").unwrap();
    assert_eq!(old.system.language_id, 1);
    let source = "schema_version = 1\n[system]\nlanguage_id = 4\ntimezone_offset_minutes = -330\nclosed_captions = true\n";
    let config = GlobalConfig::from_toml(source).unwrap();
    assert_eq!(config.ui.language, "en-US");
    assert_eq!(config.effective(None).unwrap().system, config.system);
    assert_eq!(
        GlobalConfig::from_toml(&config.to_toml().unwrap()).unwrap(),
        config
    );
    for setting in [
        "language_id = 30",
        "parental_level = 12",
        "timezone_offset_minutes = 841",
        "daylight_saving_offset_minutes = 181",
        "unknown = true",
    ] {
        assert!(
            GlobalConfig::from_toml(&format!("schema_version = 1\n[system]\n{setting}\n")).is_err()
        );
    }
}

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock must be after epoch")
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "pentax-config-test-{}-{nonce}-{}",
            std::process::id(),
            NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn defaults_round_trip_and_exclude_machine_paths() {
    let global = GlobalConfig::factory_defaults();
    global.validate().unwrap();
    let source = global.to_toml().unwrap();
    assert_eq!(GlobalConfig::from_toml(&source).unwrap(), global);
    assert_eq!(
        global.graphics.resolution,
        Resolution {
            width: 1920,
            height: 1080
        }
    );
    assert!(!source.contains("path"));
    let profile = GameOverrides::default();
    assert_eq!(
        GameOverrides::from_toml(&profile.to_toml().unwrap()).unwrap(),
        profile
    );
}

#[test]
fn overrides_inherit_and_do_not_mutate_global() {
    let mut global = GlobalConfig::default();
    global.runtime.cpu_threads = 12;
    global.runtime.enable_debug_logging = true;
    let snapshot = global.clone();
    let profile = GameOverrides {
        graphics: GraphicsOverrides {
            resolution: None,
            vsync: Some(false),
        },
        runtime: RuntimeOverrides {
            cpu_threads: Some(0),
            enable_debug_logging: Some(false),
        },
        ..GameOverrides::default()
    };
    let effective = global.effective(Some(&profile)).unwrap();
    assert!(!effective.graphics.vsync);
    assert_eq!(effective.graphics.resolution, global.graphics.resolution);
    assert_eq!(effective.runtime.cpu_threads, 0);
    assert!(!effective.runtime.enable_debug_logging);
    assert_eq!(effective.ui, global.ui);
    assert_eq!(effective.audio, global.audio);
    assert_eq!(effective.input, global.input);
    assert_eq!(global, snapshot);
    assert_eq!(global.effective(None).unwrap().runtime, global.runtime);
}

#[test]
fn malformed_unknown_and_unsupported_documents_fail() {
    for source in [
        "this is not TOML [",
        "schema_version = 1\nunknown = true\n",
        "schema_version = 1\n[ui]\nunknown = true\n",
        "schema_version = 1\n[graphics]\nframe_generation = true\n",
        "schema_version = 1\n[graphics.resolution]\nwidth = 1920\nheight = 1080\nunknown = 1\n",
        "[ui]\nscale = 1.0\n",
        "schema_version = 1\n[runtime]\nstrict_memory_checks = false\n",
    ] {
        assert!(
            GlobalConfig::from_toml(source).is_err(),
            "accepted {source:?}"
        );
    }
    for source in [
        "schema_version = 1\n[graphics]\nframe_generation = true\n",
        "schema_version = 1\n[runtime]\nunknown = true\n",
        "schema_version = 1\nunknown = true\n",
    ] {
        assert!(GameOverrides::from_toml(source).is_err());
    }
    assert!(matches!(
        GlobalConfig::from_toml("schema_version = 2\n"),
        Err(ConfigError::UnsupportedSchema {
            found: 2,
            expected: SCHEMA_VERSION
        })
    ));
    assert!(matches!(
        GameOverrides::from_toml("schema_version = 99\n"),
        Err(ConfigError::UnsupportedSchema { found: 99, .. })
    ));
}

#[test]
fn non_finite_and_out_of_range_preferences_fail() {
    for value in ["nan", "inf", "-inf", "0.0", "3.0"] {
        assert!(
            GlobalConfig::from_toml(&format!("schema_version = 1\n[ui]\nscale = {value}\n"))
                .is_err()
        );
    }
    for (section, key, value) in [
        ("audio", "master_volume", "nan"),
        ("audio", "master_volume", "1.1"),
        ("audio", "latency_ms", "0"),
        ("input", "left_stick_deadzone", "inf"),
        ("input", "right_stick_deadzone", "0.6"),
        ("input", "trigger_deadzone", "-0.1"),
        ("input", "mouse_sensitivity", "0.0"),
        ("runtime", "cpu_threads", "257"),
        ("ui", "accent", "'red'"),
        ("ui", "language", "'../../es'"),
    ] {
        assert!(
            GlobalConfig::from_toml(&format!(
                "schema_version = 1\n[{section}]\n{key} = {value}\n"
            ))
            .is_err()
        );
    }
    let mut global = GlobalConfig::default();
    global.input.left_stick_deadzone = f32::NAN;
    assert!(global.to_toml().is_err());
    let mut profile = GameOverrides::default();
    profile.graphics.resolution = Some(Resolution {
        width: 1280,
        height: 720,
    });
    assert!(profile.to_toml().is_err());
    assert!(GlobalConfig::default().effective(Some(&profile)).is_err());
}

#[test]
fn profiles_reject_path_traversal_and_non_ps5_names() {
    for value in [
        "../PPSA12345",
        "PPSA12345/..",
        "PPSA12345\\..",
        "..",
        ".",
        "",
        "C:\\PPSA12345",
        "PPSA１２３４５",
        "ppsa12345",
        "PPSA1234",
        "PPSA123456",
        "PPSA12a45",
        "CUSA12345",
        "PPSA12345.toml",
    ] {
        assert!(TitleId::new(value).is_err(), "accepted {value:?}");
    }
    let title: TitleId = "PPSA12345".parse().unwrap();
    assert_eq!(title.as_str(), "PPSA12345");
    let store = ConfigStore::new("preferences");
    assert_eq!(
        store.game_path(&title),
        PathBuf::from("preferences")
            .join("games")
            .join("PPSA12345.toml")
    );
}

#[test]
fn persistence_replaces_existing_files_and_keeps_global_and_profiles_separate() {
    let directory = TestDirectory::new();
    let store = ConfigStore::new(&directory.0);
    let title = TitleId::new("PPSA12345").unwrap();
    assert_eq!(store.load_global().unwrap(), GlobalConfig::default());
    assert_eq!(store.load_game(&title).unwrap(), None);
    assert!(
        !directory.0.exists(),
        "reads must not create configuration files"
    );

    let mut global = GlobalConfig::default();
    global.ui.scale = 1.25;
    store.save_global(&global).unwrap();
    assert_eq!(store.load_global().unwrap(), global);
    global.ui.scale = 1.5;
    store.save_global(&global).unwrap();
    assert_eq!(store.load_global().unwrap(), global);

    let mut profile = GameOverrides::default();
    profile.graphics.vsync = Some(false);
    store.save_game(&title, &profile).unwrap();
    assert_eq!(store.load_game(&title).unwrap(), Some(profile.clone()));
    assert!(!store.effective_for_game(&title).unwrap().graphics.vsync);
    assert!(store.load_global().unwrap().graphics.vsync);
    profile.runtime.cpu_threads = Some(8);
    store.save_game(&title, &profile).unwrap();
    assert_eq!(store.load_game(&title).unwrap(), Some(profile));
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    assert_eq!(fs::read_dir(directory.0.join("games")).unwrap().count(), 1);
}

#[test]
fn invalid_save_and_invalid_load_preserve_existing_configuration() {
    let directory = TestDirectory::new();
    let store = ConfigStore::new(&directory.0);
    let global = GlobalConfig::default();
    store.save_global(&global).unwrap();
    let original = fs::read(store.global_path()).unwrap();
    let mut invalid = global;
    invalid.ui.scale = f32::NAN;
    assert!(store.save_global(&invalid).is_err());
    assert_eq!(fs::read(store.global_path()).unwrap(), original);

    let unsupported = "schema_version = 100\n";
    fs::write(store.global_path(), unsupported).unwrap();
    assert!(matches!(
        store.load_global(),
        Err(ConfigError::UnsupportedSchema { .. })
    ));
    assert_eq!(
        fs::read_to_string(store.global_path()).unwrap(),
        unsupported
    );
    fs::write(store.global_path(), "invalid [").unwrap();
    assert!(matches!(store.load_global(), Err(ConfigError::Parse(_))));
    assert_eq!(
        fs::read_to_string(store.global_path()).unwrap(),
        "invalid ["
    );
}

#[test]
fn failed_replace_cleans_temporary_file_and_leaves_destination_intact() {
    let directory = TestDirectory::new();
    let store = ConfigStore::new(&directory.0);
    fs::create_dir_all(store.global_path()).unwrap();
    let marker = store.global_path().join("existing-marker");
    fs::write(&marker, "keep").unwrap();
    assert!(store.save_global(&GlobalConfig::default()).is_err());
    assert_eq!(fs::read_to_string(marker).unwrap(), "keep");
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
}

#[test]
fn ordinary_saves_refuse_future_schema_and_malformed_existing_documents() {
    let directory = TestDirectory::new();
    let store = ConfigStore::new(&directory.0);
    let title = TitleId::new("PPSA12345").unwrap();
    fs::create_dir_all(store.game_path(&title).parent().unwrap()).unwrap();
    for existing in [
        "schema_version = 2\n",
        "invalid [",
        "schema_version = 1\n[ui]\nscale = nan\n",
    ] {
        fs::write(store.global_path(), existing).unwrap();
        assert!(store.save_global(&GlobalConfig::default()).is_err());
        assert_eq!(fs::read_to_string(store.global_path()).unwrap(), existing);
    }
    for existing in [
        "schema_version = 2\n",
        "invalid [",
        "schema_version = 1\n[runtime]\ncpu_threads = 999\n",
    ] {
        fs::write(store.game_path(&title), existing).unwrap();
        assert!(store.save_game(&title, &GameOverrides::default()).is_err());
        assert_eq!(
            fs::read_to_string(store.game_path(&title)).unwrap(),
            existing
        );
    }
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    assert_eq!(fs::read_dir(directory.0.join("games")).unwrap().count(), 1);
}

#[test]
fn empty_store_directory_roundtrips_in_the_current_directory() {
    const CHILD_MARKER: &str = "PENTAX_CONFIG_EMPTY_DIRECTORY_TEST_CHILD";
    if std::env::var_os(CHILD_MARKER).is_some() {
        let store = ConfigStore::new("");
        assert_eq!(store.directory(), std::path::Path::new("."));
        store.save_global(&GlobalConfig::default()).unwrap();
        assert_eq!(store.load_global().unwrap(), GlobalConfig::default());
        return;
    }
    let directory = TestDirectory::new();
    fs::create_dir_all(&directory.0).unwrap();
    // Isolate cwd in a child process so parallel tests and the workspace are untouched.
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "empty_store_directory_roundtrips_in_the_current_directory",
            "--nocapture",
        ])
        .env(CHILD_MARKER, "1")
        .current_dir(&directory.0)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(directory.0.join("global.toml").is_file());
}

#[cfg(unix)]
#[test]
fn unix_new_files_are_private_and_replacements_preserve_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let directory = TestDirectory::new();
    let store = ConfigStore::new(&directory.0);
    store.save_global(&GlobalConfig::default()).unwrap();
    assert_eq!(
        fs::metadata(store.global_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o077,
        0
    );
    fs::set_permissions(store.global_path(), fs::Permissions::from_mode(0o640)).unwrap();
    store.save_global(&GlobalConfig::default()).unwrap();
    assert_eq!(
        fs::metadata(store.global_path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
}
