// SPDX-License-Identifier: GPL-2.0-only
//! Original exact imported POSIX callers; no host timing or fake wakeup oracle.
use super::*;
const ERRNO: &str = "9BcDykPmo1I";
fn errno_is(p: &mut Program, value: u64) {
    p.load(0, 24);
    p.code.extend([0x8b, 0]);
    p.expect(value);
}
fn errno_seed(p: &mut Program) {
    p.load(0, 24);
    p.code.extend([0xc7, 0, 37, 0, 0, 0]);
}
#[test]
fn native_observed_posix_aliases_preserve_real_errno_direct_pthread_errors_and_zero_sleep_outputs()
{
    for nid in ["NhpspxdjEKU", "yS8U2TGCe1A"] {
        let mut p = Program::new();
        p.call(2);
        p.save(24);
        errno_seed(&mut p);
        for at in [56, 64, 72, 80] {
            p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
            p.save(at);
        }
        p.reg(7, 0);
        p.ptr(6, 64);
        p.call(0);
        p.expect(u64::from(u32::MAX));
        errno_is(&mut p, 14);
        for at in [64, 72] {
            p.word(at, 0xa5a5_a5a5_a5a5_a5a5);
        }
        p.reg(0, u64::MAX);
        p.save(32);
        p.set(40, 0);
        errno_seed(&mut p);
        p.ptr(7, 32);
        p.ptr(6, 64);
        p.call(0);
        p.expect(u64::from(u32::MAX));
        errno_is(&mut p, 22);
        p.set(32, 0);
        errno_seed(&mut p);
        p.ptr(7, 32);
        p.ptr(6, 64);
        p.call(0);
        p.expect(0);
        errno_is(&mut p, 37);
        p.word(56, 0xa5a5_a5a5_a5a5_a5a5);
        p.word(64, 0);
        p.word(72, 0);
        p.word(80, 0xa5a5_a5a5_a5a5_a5a5);
        p.reg(7, 0);
        p.reg(6, 0);
        p.call(1);
        p.expect(22);
        errno_is(&mut p, 37);
        p.call(2);
        p.load(10, 24);
        p.code.extend([0x4c, 0x39, 0xd0]);
        checked(&mut p.code, &mut p.branches);
        let (code, calls) = p.finish(None);
        let (status, fields, text) = probe(&image(&code, &[nid, "Op8TBGY5KHg", ERRNO]));
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
    }
    println!(
        "observed_posix_native_available=true observed_posix_native_programs=2 observed_posix_native_returns=2"
    );
}
#[test]
fn native_observed_posix_condition_alias_reuses_actual_park_notify_and_mutex_reacquisition() {
    super::pthread_condition::condition_wakeups(false, &[Some("libkernel")]);
    println!(
        "observed_posix_condition_native_programs=1 observed_posix_condition_native_returns=1"
    );
}
#[test]
fn native_observed_posix_aliases_refuse_other_namespaces_and_version_domains() {
    let mut cases = 0;
    for nid in ["NhpspxdjEKU", "yS8U2TGCe1A", "Op8TBGY5KHg"] {
        for domain in 0..5 {
            let mut p = Program::new();
            p.reg(7, 0);
            p.reg(6, 0);
            p.call(0);
            let (code, _) = p.finish(None);
            let source = scoped_system_image(
                &code,
                &[nid],
                if domain == 0 {
                    "libSceKernel"
                } else {
                    "libkernel"
                },
                if domain == 1 { "other" } else { "libkernel" },
                if domain == 4 { 2 } else { 1 },
            );
            let mut bytes = source.bytes.clone();
            if domain == 2 || domain == 3 {
                put(
                    &mut bytes,
                    0x258,
                    &(1u64
                        | ((if domain == 3 { 2 } else { 1 }) << 32)
                        | ((if domain == 2 { 2 } else { 1 }) << 40)
                        | (2 << 48))
                        .to_le_bytes(),
                );
            }
            let source = create_input(bytes);
            let (status, fields, text) = probe(&source);
            assert_eq!(status, 2, "{text}");
            assert_eq!(
                fields["code"],
                if domain < 2 {
                    "unavailable-function"
                } else {
                    "import-version-mismatch"
                },
                "{text}"
            );
            assert!(!fields.contains_key("value"), "{text}");
            cases += 1;
        }
    }
    assert_eq!(cases, 15);
    println!("observed_posix_native_refusal_programs=15 observed_posix_native_refusals=15");
}
