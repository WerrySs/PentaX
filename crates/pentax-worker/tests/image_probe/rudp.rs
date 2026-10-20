// SPDX-License-Identifier: GPL-2.0-only

use super::*;

#[test]
fn native_rudp_bootstrap_ignores_pool_and_thread_arguments_and_never_calls_retained_handler() {
    let mut p = Program::new();
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(16);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.call(1); // The source accepts this before Init and creates no I/O thread.
    p.expect(0);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.call(2); // Unmapped guest handler is retained as data, never executed.
    p.expect(0);
    for pool in [u64::MAX, 0] {
        p.reg(7, pool);
        p.reg(6, u64::MAX);
        p.call(0);
        p.expect(0);
        p.word(16, 0xa5a5_a5a5_a5a5_a5a5);
    }
    p.reg(7, 0);
    p.reg(6, u64::MAX);
    p.call(2); // Null handler still retains the opaque argument word.
    p.expect(0);
    p.word(16, 0xa5a5_a5a5_a5a5_a5a5);
    let (code, calls) = p.finish(None);
    let input = scoped_system_image(
        &code,
        &["amuBfI-AQc4", "6PBNpsgyaxw", "SUEVes8gvmw"],
        "libSceRudp",
        "libSceRudp",
        1,
    );
    let (status, fields, text) = probe(&input);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
