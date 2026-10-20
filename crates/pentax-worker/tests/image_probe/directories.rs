// SPDX-License-Identifier: GPL-2.0-only

//! Real directory-backed reads and containment, using original files only.

use super::*;

struct Directory {
    path: PathBuf,
    junction: bool,
}
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax directory = \u{03b4} {} {}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("assets")).unwrap();
        std::fs::write(path.join("assets/pentax-original.bin"), [37, 42, 99]).unwrap();
        Self {
            path,
            junction: false,
        }
    }
    fn binding(&self, guest: &str) -> OsString {
        let mut value = OsString::from(format!("{guest}="));
        value.push(&self.path);
        value
    }
    fn junction(&mut self, target: &Directory) {
        let status = Command::new("powershell.exe")
            .args(["-NoProfile","-NonInteractive","-Command", "$ErrorActionPreference='Stop'; New-Item -ItemType Junction -Path $env:PENTAX_TEST_LINK -Target $env:PENTAX_TEST_TARGET | Out-Null"])
            .env("PENTAX_TEST_LINK",self.path.join("escape"))
            .env("PENTAX_TEST_TARGET",&target.path)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .creation_flags(0x0800_0000).status().unwrap();
        assert!(status.success(), "original junction fixture setup failed");
        self.junction = true;
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        if self.junction {
            std::fs::remove_dir(self.path.join("escape")).unwrap();
        }
        assert_eq!(
            std::fs::read(self.path.join("assets/pentax-original.bin")).unwrap(),
            [37, 42, 99]
        );
        std::fs::remove_file(self.path.join("assets/pentax-original.bin")).unwrap();
        std::fs::remove_dir(self.path.join("assets")).unwrap();
        std::fs::remove_dir(&self.path).unwrap();
    }
}

#[test]
fn directory_binding_drives_actual_native_open_read_pread_seek_stat_and_close() {
    let directory = Directory::new();
    let main = data_files::consumer(0, false);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-directory".into(),
        directory.binding("/app0"),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], "7");
    assert!(
        fields["primary_native_instructions"]
            .parse::<u64>()
            .unwrap()
            > 0
    );
}

fn open_word(path: &str, flags: u32) -> Input {
    let mut bytes = data_files::consumer(0, false).bytes.clone();
    let mut code = vec![0x55];
    rip(&mut code, 0x800, &[0x48, 0x8d, 0x3d], 0xd00);
    code.push(0xbe);
    code.extend_from_slice(&flags.to_le_bytes());
    code.extend([0x31, 0xd2]);
    rip(&mut code, 0x800, &[0xff, 0x15], 0xf00);
    code.extend([0x5d, 0xc3]);
    put(&mut bytes, 0x800, &code);
    put(&mut bytes, 0xd00, format!("{path}\0").as_bytes());
    create_input(bytes)
}

#[test]
fn directory_escape_returns_a_real_access_error_and_write_requests_remain_read_only() {
    let mut directory = Directory::new();
    let outside = Directory::new();
    directory.junction(&outside);
    for (path, flags, expected) in [
        (
            "/app0/escape/assets/pentax-original.bin",
            0,
            0x8002_000d_u64,
        ),
        ("/app0/assets/pentax-original.bin", 1, 0x8002_001e),
        ("/app0/assets/missing.bin", 0, 0x8002_0002),
        ("/app0/../assets/pentax-original.bin", 0, 0x8002_0002),
    ] {
        let main = open_word(path, flags);
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            main.path.clone().into(),
            "--data-directory".into(),
            directory.binding("/app0"),
        ]);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], expected.to_string());
        assert_eq!(fields["service_stops"], "1");
    }
}

#[test]
fn directory_intake_rejects_bad_bindings_and_overlays_before_native_execution() {
    let directory = Directory::new();
    let main = input(&[0xb8, 42, 0, 0, 0, 0xc3], false);
    let mut file_binding = OsString::from("/app0/original.bin=");
    file_binding.push(directory.path.join("assets/pentax-original.bin"));
    let cases: Vec<(Vec<OsString>, &str)> = vec![
        (
            vec!["--data-directory".into(), "/app0".into()],
            "data-directory-binding",
        ),
        (
            vec!["--data-directory".into(), "/app0=".into()],
            "data-directory-binding",
        ),
        (
            vec!["--data-directory".into(), directory.binding("/app0/..")],
            "data-directory-binding",
        ),
        (
            vec!["--data-directory".into(), directory.binding("/")],
            "data-directory-binding",
        ),
        (
            vec![
                "--data-directory".into(),
                directory.binding("/app0"),
                "--data-directory".into(),
                directory.binding("/app0"),
            ],
            "overlapping-data-mount",
        ),
        (
            vec![
                "--data-directory".into(),
                directory.binding("/app0"),
                "--data-directory".into(),
                directory.binding("/app0/assets"),
            ],
            "overlapping-data-mount",
        ),
        (
            vec![
                "--data-directory".into(),
                directory.binding("/app0/assets"),
                "--data-file".into(),
                file_binding.clone(),
            ],
            "overlapping-data-mount",
        ),
        (
            vec![
                "--data-file".into(),
                file_binding.clone(),
                "--data-directory".into(),
                directory.binding("/app0/assets"),
            ],
            "overlapping-data-mount",
        ),
        (
            vec!["--data-directory".into(), file_binding],
            "data-directory-not-directory",
        ),
    ];
    for (extra, expected) in cases {
        let mut args = vec!["--image".into(), main.path.clone().into()];
        args.extend(extra);
        let (status, fields, text) = probe_arguments(&args);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], expected, "{text}");
        assert_eq!(fields["phase"], "admission");
        assert_eq!(fields["native_instructions"], "0");
        assert!(!fields.contains_key("value"));
    }
}
