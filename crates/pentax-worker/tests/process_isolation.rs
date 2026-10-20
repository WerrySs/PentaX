use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn worker(fixture: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pentax-worker"))
        .args(["fixture", fixture])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("worker process must start");
    let deadline = Instant::now() + Duration::from_secs(5);
    while child
        .try_wait()
        .expect("worker status must be readable")
        .is_none()
    {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("original worker fixture {fixture} exceeded five seconds");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child
        .wait_with_output()
        .expect("worker output must be readable")
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn owned_guest_stack_entry_completes_in_an_isolated_worker() {
    let output = worker("guest-stack-entry42");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "PENTAX_WORKER\t1\tREADY\tbackend=windows-x86_64-native",
            "PENTAX_WORKER\t1\tRUNNING\tfixture=guest-stack-entry42",
            "PENTAX_WORKER\t1\tRESULT\texit\tvalue=42",
        ]
    );
    assert!(worker("return42").status.success());
}

#[test]
fn syscall_fixture_is_rejected_before_the_running_state() {
    let output = worker("unsupported-syscall");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains(
        "PENTAX_WORKER\t1\tRESULT\trejected\tcode=unsupported-syscall\tnumber=65535\toffset=10"
    ));
    assert!(!stdout.contains("RUNNING"));
    assert!(!stdout.contains("READY"));
}

#[test]
fn arbitrary_paths_and_commands_are_rejected() {
    let output = worker("arbitrary-game.elf");
    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("code=invalid-command")
    );
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn native_block_dispatch_runs_multiple_blocks_in_an_isolated_worker() {
    // The closed program checks actual context round trips, arithmetic,
    // a RIP-relative data operand, mediated
    // direct/conditional/indirect transfers and checked CALL/RET stack effects
    // across several native blocks, preserving a red-zone sentinel.
    let output = worker("native-block-dispatch");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .collect::<Vec<_>>(),
        [
            "PENTAX_WORKER\t1\tREADY\tbackend=windows-x86_64-native",
            "PENTAX_WORKER\t1\tRUNNING\tfixture=native-block-dispatch",
            "PENTAX_WORKER\t1\tRESULT\texit\tvalue=42",
        ]
    );
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn prepared_execution_reaches_real_kernel_libc_and_independent_tls_backing() {
    let output = worker("prepared-kernel-startup");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<_> = stdout.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(
        lines[0],
        "PENTAX_WORKER\t1\tREADY\tbackend=windows-x86_64-prepared"
    );
    assert_eq!(
        lines[1],
        "PENTAX_WORKER\t1\tRUNNING\tfixture=prepared-kernel-startup"
    );
    let result = lines[2]
        .strip_prefix("PENTAX_WORKER\t1\tRESULT\texit\t")
        .unwrap();
    let fields: std::collections::BTreeMap<_, _> = result
        .split('\t')
        .map(|field| field.split_once('=').unwrap())
        .collect();
    for (key, expected) in [
        ("fixture", "prepared-kernel-startup"),
        ("value", "42"),
        ("result", "42"),
        ("tls0", "42"),
        ("tls1", "37"),
        ("thread", "73"),
        ("kernel", "3"),
        ("libc", "1"),
        ("requests", "4"),
        ("native_instructions", "36"),
        ("primary_native_instructions", "32"),
        ("mapped_pages", "14"),
        ("resident_pages", "14"),
        ("segment_ops", "3"),
        ("segment_bytes", "24"),
    ] {
        assert_eq!(fields.get(key).copied(), Some(expected));
    }
    // Observe actual execution without tying the contract to a block partition.
    assert!(fields["blocks"].parse::<u64>().unwrap() > 1);
    assert!(fields["memory_checks"].parse::<u64>().unwrap() > 0);
    assert!(fields["memory_bytes"].parse::<u64>().unwrap() > 0);
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn synthetic_service_is_trapped_once_and_other_worker_faults_remain_fatal() {
    let success = worker("trapped-sum42");
    assert!(success.status.success());
    assert!(success.stderr.is_empty());
    let stdout = String::from_utf8(success.stdout).unwrap();
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "PENTAX_WORKER\t1\tREADY\tbackend=windows-x86_64-native",
            "PENTAX_WORKER\t1\tRUNNING\tfixture=trapped-sum42",
            "PENTAX_WORKER\t1\tRESULT\texit\tvalue=42\ttraps=1",
        ]
    );
    let fault = worker("invalid-instruction");
    assert_eq!(
        fault.status.code().map(|value| value as u32),
        Some(0xc000001d)
    );
    assert!(
        !String::from_utf8(fault.stdout)
            .unwrap()
            .contains("RESULT\texit")
    );
    // A different UD2 never becomes the synthetic service, and a fresh healthy
    // worker has its own registration, count, and complete protocol lifecycle.
    let again = worker("trapped-sum42");
    assert!(again.status.success());
    assert!(
        String::from_utf8(again.stdout)
            .unwrap()
            .contains("value=42\ttraps=1")
    );
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[test]
fn native_return_and_illegal_instruction_are_isolated_in_child_processes() {
    let success = worker("return42");
    assert!(success.status.success());
    assert!(
        String::from_utf8(success.stdout)
            .unwrap()
            .contains("RESULT\texit\tvalue=42")
    );

    let fault = worker("invalid-instruction");
    assert_eq!(
        fault.status.code().map(|value| value as u32),
        Some(0xc000001d)
    );
    let stdout = String::from_utf8(fault.stdout).unwrap();
    assert!(stdout.contains("RUNNING\tfixture=invalid-instruction"));
    assert!(!stdout.contains("RESULT\texit"));

    // The test supervisor is still alive and can start another healthy worker.
    assert!(worker("return42").status.success());
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
#[test]
fn unsupported_hosts_do_not_claim_native_execution() {
    for fixture in [
        "return42",
        "trapped-sum42",
        "guest-stack-entry42",
        "native-block-dispatch",
        "prepared-kernel-startup",
    ] {
        let output = worker(fixture);
        assert_eq!(output.status.code(), Some(3));
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("RESULT\tunsupported"));
        assert!(!stdout.contains("RUNNING"));
    }
}
