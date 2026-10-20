// SPDX-License-Identifier: GPL-2.0-or-later

use super::*;

const SITE: u64 = 0x400005;

#[test]
fn exact_registered_trap_preserves_other_registers_and_counts_once() {
    let scope = TrapScope::install(SITE).unwrap();
    let (mut record, mut context) = frame(SITE);
    let mut expected = registers(&context);
    expected[0] = 42;
    expected[1] = SITE + 2;
    expected[11] = u64::from(context.EFlags);
    expected[16] = SITE + 2;
    assert_eq!(
        invoke(&mut record, &mut context),
        EXCEPTION_CONTINUE_EXECUTION
    );
    assert_eq!(registers(&context), expected);
    assert_eq!(scope.traps(), 1);
    let before_repeat = registers(&context);
    assert_eq!(invoke(&mut record, &mut context), EXCEPTION_CONTINUE_SEARCH);
    assert_eq!(registers(&context), before_repeat);
    assert_eq!(scope.traps(), 1);
    scope.close().unwrap();
    assert_inactive();
}

#[test]
fn foreign_codes_sites_numbers_flags_and_arguments_do_not_mutate_context() {
    let scope = TrapScope::install(SITE).unwrap();
    let changes: [fn(&mut EXCEPTION_RECORD, &mut CONTEXT); 8] = [
        |record, _| record.ExceptionCode = 0xc0000005_u32 as i32,
        |record, _| record.ExceptionFlags = 1,
        |record, _| record.ExceptionAddress = (SITE + 1) as usize as *mut c_void,
        |_, context| context.Rip += 1,
        |_, context| context.Rax += 1,
        |_, context| context.Rdi += 1,
        |_, context| context.Rsi += 1,
        |_, context| context.ContextFlags = 0,
    ];
    for change in changes {
        let (mut record, mut context) = frame(SITE);
        change(&mut record, &mut context);
        let before = registers(&context);
        assert_eq!(invoke(&mut record, &mut context), EXCEPTION_CONTINUE_SEARCH);
        assert_eq!(registers(&context), before);
        assert_eq!(scope.traps(), 0);
    }
    let (mut record, mut context) = frame(SITE);
    assert_eq!(
        invoke(&mut record, &mut context),
        EXCEPTION_CONTINUE_EXECUTION
    );
    assert_eq!(scope.traps(), 1);
    scope.close().unwrap();
}

#[test]
fn null_records_and_recursive_phase_continue_search_without_a_trap() {
    let scope = TrapScope::install(SITE).unwrap();
    // SAFETY: A null pointer is explicitly rejected without dereferencing it.
    let null_result = unsafe { fixture_trap_handler(std::ptr::null_mut()) };
    assert_eq!(null_result, EXCEPTION_CONTINUE_SEARCH);
    let mut pointers = EXCEPTION_POINTERS::default();
    // SAFETY: The outer record lives here; both null inner pointers are rejected.
    let empty_result = unsafe { fixture_trap_handler(&mut pointers) };
    assert_eq!(empty_result, EXCEPTION_CONTINUE_SEARCH);
    ACTIVE_TRAP.with(|slot| {
        let mut state = slot.get().unwrap();
        state.phase = Phase::Handling;
        slot.set(Some(state));
    });
    let (mut record, mut context) = frame(SITE);
    let before = registers(&context);
    assert_eq!(invoke(&mut record, &mut context), EXCEPTION_CONTINUE_SEARCH);
    assert_eq!(registers(&context), before);
    assert_eq!(scope.traps(), 0);
    scope.close().unwrap();
}

#[test]
fn a_registration_cannot_handle_another_thread_or_replace_its_scope() {
    let scope = TrapScope::install(SITE).unwrap();
    assert!(matches!(
        TrapScope::install(SITE + 32),
        Err(PlatformError::TrapScopeUnavailable)
    ));
    let foreign = std::thread::spawn(|| {
        let (mut record, mut context) = frame(SITE);
        let before = registers(&context);
        let result = invoke(&mut record, &mut context);
        assert_eq!(registers(&context), before);
        result
    });
    assert_eq!(foreign.join().unwrap(), EXCEPTION_CONTINUE_SEARCH);
    assert_eq!(scope.traps(), 0);
    scope.close().unwrap();
    assert_inactive();
}

#[test]
fn tls_is_cleared_before_removal_including_a_reported_removal_failure() {
    let mut scope = TrapScope::install(SITE).unwrap();
    let result = scope.close_with(|handle| {
        assert_inactive();
        // Remove the real OS registration, then simulate its failure result.
        // This exercises failure cleanup without leaking a handler into tests.
        assert!(remove_handler(handle));
        false
    });
    assert_eq!(result, Err(PlatformError::TrapHandlerRemovalFailed));
    assert_eq!(scope.traps(), 0);
    let (mut record, mut context) = frame(SITE);
    let before = registers(&context);
    assert_eq!(invoke(&mut record, &mut context), EXCEPTION_CONTINUE_SEARCH);
    assert_eq!(registers(&context), before);
    drop(scope);
    assert_inactive();
    // Drop also clears a live, uncompleted scope, without an explicit close.
    drop(TrapScope::install(SITE).unwrap());
    assert_inactive();
}

#[test]
fn overflowing_continuation_rejects_before_register_mutation() {
    let address = u64::MAX;
    let (record, mut context) = frame(address);
    let before = registers(&context);
    assert!(!apply_registered_trap(address, &record, &mut context));
    assert_eq!(registers(&context), before);
}

#[test]
fn original_native_trap_runs_repeatedly_with_independent_scopes() {
    crate::configure_worker_fault_policy().unwrap();
    for _ in 0..3 {
        assert_eq!(
            execute(),
            Ok(OriginalFixtureResult {
                value: 42,
                traps: 1
            })
        );
        assert_inactive();
    }
}

fn assert_inactive() {
    ACTIVE_TRAP.with(|slot| assert!(slot.get().is_none()));
}

fn invoke(record: &mut EXCEPTION_RECORD, context: &mut CONTEXT) -> i32 {
    let mut pointers = EXCEPTION_POINTERS {
        ExceptionRecord: record,
        ContextRecord: context,
    };
    // SAFETY: Both pointers reference distinct, valid live structures. The
    // callback's saved-context edits are inspected here, never resumed as code.
    unsafe { fixture_trap_handler(&mut pointers) }
}

fn frame(address: u64) -> (EXCEPTION_RECORD, CONTEXT) {
    let record = EXCEPTION_RECORD {
        ExceptionCode: EXCEPTION_ILLEGAL_INSTRUCTION,
        ExceptionAddress: address as usize as *mut c_void,
        ..EXCEPTION_RECORD::default()
    };
    let context = CONTEXT {
        ContextFlags: CONTEXT_CONTROL_AMD64 | CONTEXT_INTEGER_AMD64,
        EFlags: 0x202,
        Rax: SYNTHETIC_SUM_SERVICE,
        Rcx: 1,
        Rdx: 2,
        Rbx: 3,
        Rsp: 0x200000,
        Rbp: 5,
        Rsi: 23,
        Rdi: 19,
        R8: 8,
        R9: 9,
        R10: 10,
        R11: 11,
        R12: 12,
        R13: 13,
        R14: 14,
        R15: 15,
        Rip: address,
        MxCsr: 0x1f80,
        ..CONTEXT::default()
    };
    (record, context)
}

fn registers(context: &CONTEXT) -> [u64; 20] {
    [
        context.Rax,
        context.Rcx,
        context.Rdx,
        context.Rbx,
        context.Rsp,
        context.Rbp,
        context.Rsi,
        context.Rdi,
        context.R8,
        context.R9,
        context.R10,
        context.R11,
        context.R12,
        context.R13,
        context.R14,
        context.R15,
        context.Rip,
        u64::from(context.EFlags),
        u64::from(context.ContextFlags),
        u64::from(context.MxCsr),
    ]
}
