// SPDX-License-Identifier: GPL-2.0-only
//! Original local files and byte streams only. No child process is launched.
use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
struct Temporary(PathBuf);
impl Temporary {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..32 {
            let path = std::env::temp_dir().join(format!(
                "pentax-launch-original-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => panic!("original fixture directory: {error}"),
            }
        }
        panic!("original fixture directory candidates exhausted");
    }
    fn title(&self) -> LibraryEntry {
        let game = self.0.join("Original title");
        fs::create_dir(&game).unwrap();
        let path = game.join("eboot.bin");
        fs::write(&path, b"original bounded plan fixture").unwrap();
        LibraryEntry {
            path,
            title_id: None,
        }
    }
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn value<'a>(plan: &'a Plan, option: &str) -> Option<&'a std::ffi::OsStr> {
    plan.args[1..]
        .chunks_exact(2)
        .find(|p| p[0] == option)
        .map(|p| p[1].as_os_str())
}

#[test]
fn launch_plan_selects_only_named_sources_and_keeps_title_storage_and_preferences_separate() {
    let temporary = Temporary::new();
    let mut entry = temporary.title();
    entry.title_id = Some("PPSA12345".into());
    let game = entry.path.parent().unwrap();
    let modules = game.join("sce_module");
    fs::create_dir(&modules).unwrap();
    fs::write(modules.join("libc.prx"), b"original libc metadata fixture").unwrap();
    fs::write(
        modules.join("libSceNpCppWebApi.prx"),
        b"original deferred metadata fixture",
    )
    .unwrap();
    fs::write(modules.join("unselected.prx"), b"must remain unselected").unwrap();
    let config = temporary.0.join("preferences");
    let store = pentax_config::ConfigStore::new(&config);
    store
        .save_global(&pentax_config::GlobalConfig::default())
        .unwrap();
    let title = pentax_config::TitleId::new("PPSA12345").unwrap();
    store
        .save_game(&title, &pentax_config::GameOverrides::default())
        .unwrap();
    let global_before = fs::read(store.global_path()).unwrap();
    let title_before = fs::read(store.game_path(&title)).unwrap();
    let plan = Plan::prepare(&entry, &config).unwrap();
    assert_eq!(plan.args[0], "probe-entry");
    assert_eq!(
        value(&plan, "--present-window"),
        Some(std::ffi::OsStr::new("yes"))
    );
    assert_eq!(
        value(&plan, "--execution-profile"),
        Some(std::ffi::OsStr::new("interactive"))
    );
    assert_eq!(
        value(&plan, "--memory-profile"),
        Some(std::ffi::OsStr::new("runtime"))
    );
    let root = game.canonicalize().unwrap();
    assert_eq!(
        value(&plan, "--libc-memory-builtins"),
        Some(std::ffi::OsStr::new("checked"))
    );
    assert_eq!(
        value(&plan, "--data-directory"),
        Some(binding("/app0", &root).as_os_str())
    );
    assert_eq!(
        value(&plan, "--preload-libc-as"),
        Some(
            binding(
                "/app0/sce_module/libc.prx",
                &modules.join("libc.prx").canonicalize().unwrap()
            )
            .as_os_str()
        )
    );
    assert_eq!(
        value(&plan, "--module-file"),
        Some(
            binding(
                "/app0/sce_module/libSceNpCppWebApi.prx",
                &modules
                    .join("libSceNpCppWebApi.prx")
                    .canonicalize()
                    .unwrap()
            )
            .as_os_str()
        )
    );
    assert_eq!(
        plan.args[1..]
            .chunks_exact(2)
            .filter(|p| p[0] == "--module-file")
            .count(),
        1
    );
    let save = Path::new(value(&plan, "--save-directory").unwrap());
    let backing = Path::new(value(&plan, "--physical-backing-directory").unwrap());
    assert!(contains(&config, save).unwrap());
    assert!(contains(&config, backing).unwrap());
    assert!(!overlap(save, &root).unwrap());
    assert!(!overlap(backing, &root).unwrap());
    assert!(!overlap(save, backing).unwrap());
    assert_eq!(fs::read(store.global_path()).unwrap(), global_before);
    assert_eq!(fs::read(store.game_path(&title)).unwrap(), title_before);
    assert_eq!(
        fs::read(&entry.path).unwrap(),
        b"original bounded plan fixture"
    );
}

#[test]
fn launch_refuses_storage_overlap_before_creation_and_uses_stable_full_digest_keys() {
    let temporary = Temporary::new();
    let entry = temporary.title();
    let image = entry.path.canonicalize().unwrap();
    let game = image.parent().unwrap();
    let nested = game.join("must-not-create");
    assert!(matches!(
        Plan::prepare(&entry, &nested),
        Err(Error::Overlap)
    ));
    assert!(!nested.exists());
    let key = title_key(&image, None).unwrap();
    let no_libc = Plan::prepare(&entry, &temporary.0.join("preferences")).unwrap();
    assert_eq!(value(&no_libc, "--libc-memory-builtins"), None);
    assert_eq!(key.len(), 70);
    assert_eq!(key, title_key(&image, None).unwrap());
    assert_ne!(
        key,
        title_key(&game.join("other-original.elf"), None).unwrap()
    );
    assert!(matches!(
        title_key(&image, Some("../invalid")),
        Err(Error::Preferences)
    ));
    #[cfg(unix)]
    {
        let alias = temporary.0.join("alias-config");
        std::os::unix::fs::symlink(game, &alias).unwrap();
        assert!(matches!(Plan::prepare(&entry, &alias), Err(Error::Overlap)));
        let config = temporary.0.join("safe-config");
        let title = config.join("runtime").join(key);
        fs::create_dir_all(&title).unwrap();
        std::os::unix::fs::symlink(game, title.join("backing")).unwrap();
        assert!(matches!(
            Plan::prepare(&entry, &config),
            Err(Error::Storage | Error::Overlap)
        ));
    }
}

#[test]
fn fragmented_worker_results_produce_only_closed_summaries_and_refuse_malformed_or_unbounded_output()
 {
    let original = b"PENTAX_WORKER\t1\tREADY\tbackend=original\nPENTAX_WORKER\t1\tRUNNING\tmode=entry-probe\nPENTAX_WORKER\t1\tRESULT\tunsupported\tcode=unavailable-function\tprivate=unrendered-original-field\n";
    let mut protocol = Protocol::new();
    for byte in original {
        protocol.feed(&[*byte]).unwrap();
    }
    assert_eq!(protocol.result, Some(Summary::MissingService));
    assert_eq!(protocol.used, 0);
    assert!(!protocol.result.unwrap().message().contains("unrendered"));
    assert_eq!(
        protocol.feed(b"PENTAX_WORKER\t1\tRESULT\texit\tcode=outer-return\n"),
        Err(Error::Protocol)
    );
    let mut malformed = Protocol::new();
    assert_eq!(
        malformed.feed(b"not-the-worker-protocol\n"),
        Err(Error::Protocol)
    );
    let mut duplicate = Protocol::new();
    assert_eq!(
        duplicate
            .feed(b"PENTAX_WORKER\t1\tRESULT\texit\tcode=outer-return\tcode=runtime-service\n"),
        Err(Error::Protocol)
    );
    for fields in [
        "code=native-dispatch\tbudget=dispatch-blocks\truntime_failure=dispatch-budget",
        "code=native-dispatch\tbudget=dispatch-memory-checks",
        "code=native-dispatch\truntime_failure=memory-budget",
        "code=runtime-service\tbudget=kernel-handles\truntime_failure=runtime-budget",
        "code=runtime-service\tbudget=module-publication-work",
        "code=runtime-service\truntime_failure=guest-memory-budget",
        "code=runtime-service\truntime_failure=graphics-budget",
        "code=execution-budget\tbudget=idle-waits",
    ] {
        let result = format!("PENTAX_WORKER\t1\tRESULT\tunsupported\t{fields}\n");
        let mut limited = Protocol::new();
        for chunk in result.as_bytes().chunks(3) {
            limited.feed(chunk).unwrap();
        }
        assert_eq!(limited.result, Some(Summary::Limit), "{fields}");
    }
    for fields in [
        "code=native-dispatch\tbudget=dispatch-blocks\tbudget=dispatch-work",
        "code=native-dispatch\truntime_failure=dispatch-budget\truntime_failure=memory-budget",
    ] {
        let mut duplicate = Protocol::new();
        assert_eq!(
            duplicate.feed(format!("PENTAX_WORKER\t1\tRESULT\tunsupported\t{fields}\n").as_bytes()),
            Err(Error::Protocol)
        );
    }
    for (fields, expected) in [
        (
            "code=native-dispatch\tbudget=dispatch-not-defined\truntime_failure=dispatch-budget-extra",
            Summary::Failed,
        ),
        (
            "code=runtime-service\tbudget=kernel-not-defined\truntime_failure=runtime-budget-extra",
            Summary::RuntimeStop,
        ),
        (
            "code=native-dispatch\tbudget=kernel-handles",
            Summary::Failed,
        ),
    ] {
        let mut unknown = Protocol::new();
        unknown
            .feed(format!("PENTAX_WORKER\t1\tRESULT\tunsupported\t{fields}\n").as_bytes())
            .unwrap();
        assert_eq!(unknown.result, Some(expected));
    }
    let mut bounded = Protocol::new();
    bounded.feed(&[b'x'; LINE_BYTES]).unwrap();
    assert_eq!(bounded.feed(b"x"), Err(Error::Limit));
    let mut finished = Protocol::new();
    finished
        .feed(b"PENTAX_WORKER\t1\tRESULT\texit\tcode=outer-return\tpresented_frames=999\n")
        .unwrap();
    assert_eq!(finished.result, Some(Summary::Finished));
    assert!(!Summary::Finished.message().contains("frame"));
}
