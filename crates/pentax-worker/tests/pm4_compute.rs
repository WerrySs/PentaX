// SPDX-License-Identifier: GPL-2.0-only
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]
use std::{
    os::windows::process::CommandExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[test]
fn original_pm4_compute_reaches_vulkan_and_writes_both_dynamic_guest_results() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pentax-worker"))
        .args(["gpu-fixture", "pm4-compute"])
        .creation_flags(0x0800_0000)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("original PM4 compute worker deadline");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(
        text.contains("RESULT\texit\tfixture=pm4-compute\tbytes=112\tinvocations=24"),
        "{text}"
    );
}
