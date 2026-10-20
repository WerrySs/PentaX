// SPDX-License-Identifier: GPL-2.0-only
//
// These unit tests deliberately never call `pentax_block_enter`, `dispatch`,
// `run_one_block` or `execute_native_block_dispatch`: no native guest block is
// executed here. They cover source/attestation/budget/host-context/fixture
// logic plus negative mutated-source, emission, unknown-exit, wrong-policy and
// red-zone-corruption cases. Native block execution is exercised only through
// the isolated worker integration test.

use super::*;
use pentax_codegen::{CpuFeatures, UnsupportedReason};

// These original profiles intentionally use the same closed guest/cache VAs.
// Serialize only their mappings; pure decoder tests remain parallel.
static VM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn vm_lock() -> std::sync::MutexGuard<'static, ()> {
    VM_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

// Leak the image and inputs so the plan is 'static and easy to hold in tests.
// Tests are short-lived processes; the leak is bounded and intentional.
fn leak_plan(image: Vec<u8>) -> SymbolicLinkPlan<'static> {
    leak_plan_at(image, BIAS)
}

fn leak_plan_at(image: Vec<u8>, bias: u64) -> SymbolicLinkPlan<'static> {
    let image: &'static [u8] = Box::leak(image.into_boxed_slice());
    let view = open_executable(image).unwrap();
    let inputs: &'static [ModuleInput<'static>] =
        Box::leak(Box::new([ModuleInput::new(view, bias)]));
    plan_symbolic(inputs, &SymbolicLimits::default()).unwrap()
}

#[test]
fn context_init_policy_never_leaks_host_registers() {
    let ctx = GuestContext::zeroed();
    assert_eq!(ctx.gpr, [0; 16]);
    assert_eq!(ctx.rflags, 0x202);
    assert_eq!(ctx.mxcsr, 0x1f80);
    assert_eq!(ctx.xmm, [0; 16 * 16]);
    assert_eq!(ctx.rip, 0);
    assert_eq!(ctx.host_rsp, 0);
}

#[test]
fn original_program_entry_block_is_a_conditional_with_a_rip_relative_operand() {
    let _mapping_guard = vm_lock();
    let plan = leak_plan(original_image());
    // This pure planning check needs a placement request, no OS mapping.
    let placement = CachePlacement {
        address: BIAS + 0x40_0000,
        capacity: (CACHE_BYTES - TAIL_CELL) as u64,
    };
    let block = plan_block(
        &plan,
        0,
        plan.primary_entry(),
        placement,
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    assert!(matches!(
        block.exit().kind(),
        ExitKind::Conditional {
            condition: ConditionCode::e,
            ..
        }
    ));
    // lea, mov [rsp-16], mov rcx,[rip], cmp are emitted; the je is the exit.
    let emitted: Vec<_> = block
        .instructions()
        .iter()
        .filter(|r| r.emitted_len() > 0)
        .collect();
    assert_eq!(emitted.len(), 4);
    assert!(
        emitted
            .iter()
            .any(|r| r.rip_relative_target() == Some(BIAS + DATA_OFF))
    );
    assert!(block.bytes().ends_with(&[0x0f, 0x0b]));
}

#[test]
fn condition_evaluation_matches_rflags_bits() {
    let zf = 1 << 6;
    let cf = 1 << 0;
    assert_eq!(condition_holds(ConditionCode::e, zf), Some(true));
    assert_eq!(condition_holds(ConditionCode::ne, zf), Some(false));
    assert_eq!(condition_holds(ConditionCode::e, 0), Some(false));
    assert_eq!(condition_holds(ConditionCode::b, cf), Some(true));
    assert_eq!(condition_holds(ConditionCode::ae, cf), Some(false));
    assert_eq!(condition_holds(ConditionCode::None, 0), None);
    // SF != OF for 'l'.
    assert_eq!(condition_holds(ConditionCode::l, 1 << 7), Some(true));
    assert_eq!(condition_holds(ConditionCode::ge, 1 << 7), Some(false));
}

#[test]
fn gpr_slots_cover_all_sixteen_and_reject_non_gpr64() {
    assert_eq!(gpr_slot(Register::RAX), Some(0));
    assert_eq!(gpr_slot(Register::RSP), Some(4));
    assert_eq!(gpr_slot(Register::R15), Some(15));
    assert_eq!(gpr_slot(Register::XMM0), None);
    assert_eq!(gpr_slot(Register::EAX), None);
}

#[test]
fn executable_target_accepts_code_and_rejects_data_stack_and_noncanonical() {
    let plan = leak_plan(original_image());
    assert!(executable_target(&plan, BIAS + ENTRY as u64));
    assert!(executable_target(&plan, BIAS + ENTRY as u64 + 55));
    assert!(!executable_target(&plan, BIAS + DATA_OFF));
    assert!(!executable_target(&plan, BIAS + STACK_OFF));
    assert!(!executable_target(&plan, 1 << 48));
    assert!(!executable_target(&plan, STOP));
}

#[test]
fn host_supports_baseline_and_refuses_unknown_features() {
    assert!(host_supports(CpuidFeature::X64));
    assert!(host_supports(CpuidFeature::CMOV));
    // AVX is never admitted by the policy; host_supports refuses it explicitly.
    assert!(!host_supports(CpuidFeature::AVX));
    assert!(!host_supports(CpuidFeature::AVX2));
    assert_eq!(feature_name(CpuidFeature::AVX), "unsupported-feature");
    assert_eq!(feature_name(CpuidFeature::SSE2), "sse2");
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows_only {
    use super::*;

    fn staged() -> (
        SymbolicLinkPlan<'static>,
        NativeAddressSpace,
        u64,
        CodeCache,
    ) {
        staged_image(original_image())
    }

    fn staged_image(
        image: Vec<u8>,
    ) -> (
        SymbolicLinkPlan<'static>,
        NativeAddressSpace,
        u64,
        CodeCache,
    ) {
        let arena = fresh_guest_space().unwrap();
        let bias = arena.reserved_ranges().next().unwrap().address;
        let plan = leak_plan_at(image, bias);
        let (space, rsp) = stage_guest(&plan, arena).unwrap();
        let cache = CodeCache::reserve_near(bias, CACHE_BYTES).unwrap();
        (plan, space, rsp, cache)
    }

    fn entry_block<'p, 'i>(plan: &'p SymbolicLinkPlan<'i>, cache: &CodeCache) -> BlockPlan<'p, 'i> {
        plan_block(
            plan,
            0,
            plan.primary_entry(),
            CachePlacement {
                address: cache.address(),
                capacity: cache.capacity(),
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap()
    }

    #[test]
    fn publish_seals_rx_and_attests_ordinary_bytes_plus_engine_tail() {
        let _mapping_guard = vm_lock();
        let (plan, mut space, _rsp, mut cache) = staged();
        let block = entry_block(&plan, &cache);
        // Publishing seals RX but does NOT execute the block.
        publish_block(&mut cache, &mut space, &block, plan.primary_entry()).unwrap();
        assert!(cache.executable);
        // The ordinary prefix in the cache equals the planner's emission minus
        // the terminal UD2; the tail is the engine's jmp [rip+cell].
        let ud2_off = block.bytes().len() - 2;
        let mut got = vec![0u8; ud2_off];
        cache.read(0, &mut got).unwrap();
        assert_eq!(got, block.bytes()[..ud2_off]);
        let mut tail = [0u8; 2];
        cache.read(ud2_off, &mut tail).unwrap();
        assert_eq!(tail, [0xff, 0x25]);
        let mut cell = [0u8; 8];
        cache.read(cache.capacity, &mut cell).unwrap();
        assert_eq!(
            u64::from_le_bytes(cell),
            pentax_block_trailer as *const () as u64
        );
    }

    #[test]
    fn mutated_guest_source_fails_attestation_before_publication() {
        let _mapping_guard = vm_lock();
        let (plan, mut space, _rsp, mut cache) = staged();
        let block = entry_block(&plan, &cache);
        // Flip one byte of the guest's executable source: re-arm RW, patch, RX.
        let bias = plan.modules()[0].load_bias();
        let code_va = bias + ENTRY as u64;
        space
            .protect(bias, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        {
            let mut paused = space.pause().unwrap();
            paused.write(code_va, &[0x90]).unwrap(); // was 0x48 (lea prefix)
        }
        space
            .protect(bias, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            publish_block(&mut cache, &mut space, &block, plan.primary_entry()),
            Err(PlatformError::SourceAttestationFailed)
        );
        assert!(!cache.executable);
    }

    #[test]
    fn wrong_emission_base_fails_attestation() {
        let _mapping_guard = vm_lock();
        let (plan, mut space, _rsp, mut cache) = staged();
        // Plan at a placement whose base differs from the cache base: the
        // verbatim ordinary copy would corrupt RIP-relative operands, so
        // attest_emission must reject it.
        let block = plan_block(
            &plan,
            0,
            plan.primary_entry(),
            CachePlacement {
                address: cache.address() + NATIVE_GUEST_PAGE,
                capacity: cache.capacity() - NATIVE_GUEST_PAGE,
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap();
        assert_eq!(
            publish_block(&mut cache, &mut space, &block, plan.primary_entry()),
            Err(PlatformError::EmissionAttestationFailed)
        );
    }

    #[test]
    fn wrong_policy_turns_the_first_instruction_into_a_mediated_exit() {
        let _mapping_guard = vm_lock();
        let (plan, _space, _rsp, cache) = staged();
        // An empty feature policy admits no integer/SSE family, so the very
        // first instruction becomes a mediated exit rather than native output.
        let block = plan_block(
            &plan,
            0,
            plan.primary_entry(),
            CachePlacement {
                address: cache.address(),
                capacity: cache.capacity(),
            },
            &TranslationPolicy::new(CpuFeatures::empty()),
            &BlockLimits::default(),
        )
        .unwrap();
        assert!(matches!(
            block.exit().kind(),
            ExitKind::Mediated {
                reason: UnsupportedReason::MissingFeature(_)
            }
        ));
        assert_eq!(block.bytes(), [0x0f, 0x0b]);
    }

    #[test]
    fn unknown_mediated_and_syscall_exits_refuse_without_execution() {
        let _mapping_guard = vm_lock();
        // A tiny image whose entry is a SYSCALL, and one whose entry is HLT.
        for (bytes, expect_syscall) in [(vec![0x0f, 0x05], true), (vec![0xf4], false)] {
            let (plan, mut space, rsp, cache) = staged_image(build_elf(&bytes));
            let block = plan_block(
                &plan,
                0,
                plan.primary_entry(),
                CachePlacement {
                    address: cache.address(),
                    capacity: cache.capacity(),
                },
                &TranslationPolicy::x64_sse2(),
                &BlockLimits::default(),
            )
            .unwrap();
            let ctx = GuestContext::zeroed();
            let result = resolve_exit(&plan, &mut space, &block, &ctx, rsp);
            if expect_syscall {
                assert!(matches!(
                    result,
                    Err(PlatformError::UnknownGuestSyscall { .. })
                ));
            } else {
                assert!(matches!(
                    result,
                    Err(PlatformError::UnsupportedGuestExit { .. })
                ));
            }
        }
    }

    #[test]
    fn red_zone_checks_the_fixed_outer_slot_and_rejects_zero_and_unmapped_backing() {
        let _mapping_guard = vm_lock();
        let (_plan, mut space, rsp, _cache) = staged();
        // After the first block the slot must contain the exact live sentinel.
        assert!(matches!(
            verify_sentinel(&mut space, rsp),
            Err(PlatformError::RedZoneCorrupted { value: 0 })
        ));
        // Write the legitimate sentinel: accepted.
        {
            let mut paused = space.pause().unwrap();
            paused.write(rsp - 16, &SENTINEL.to_le_bytes()).unwrap();
        }
        verify_sentinel(&mut space, rsp).unwrap();
        assert!(verify_sentinel(&mut space, _plan.modules()[0].load_bias() + 0x80_0000).is_err());
        // Corrupt it: rejected.
        {
            let mut paused = space.pause().unwrap();
            paused
                .write(rsp - 16, &0xdead_beefu64.to_le_bytes())
                .unwrap();
        }
        assert!(matches!(
            verify_sentinel(&mut space, rsp),
            Err(PlatformError::RedZoneCorrupted { .. })
        ));
    }

    #[test]
    fn indirect_memory_and_register_targets_resolve_from_guest_state() {
        let _mapping_guard = vm_lock();
        // jmp [r12+rcx*8+24] decoded as an indirect memory transfer.
        let (plan, mut space, _rsp, cache) =
            staged_image(build_elf(&[0x41, 0xff, 0x64, 0xcc, 0x18]));
        let bias = plan.modules()[0].load_bias();
        let block = plan_block(
            &plan,
            0,
            plan.primary_entry(),
            CachePlacement {
                address: cache.address(),
                capacity: cache.capacity(),
            },
            &TranslationPolicy::x64_sse2(),
            &BlockLimits::default(),
        )
        .unwrap();
        let ExitKind::Jump {
            target: ControlTarget::Memory(operand),
        } = block.exit().kind()
        else {
            panic!("expected indirect memory jump exit");
        };
        // Point r12 at the owned stack and store a pointer value at the computed
        // effective address; resolve_target must read exactly that pointer.
        let stack_va = bias + STACK_OFF;
        let mut ctx = GuestContext::zeroed();
        ctx.gpr[12] = stack_va; // r12
        ctx.gpr[1] = 1; // rcx
        let effective = memory_effective_address(&ctx, &operand).unwrap();
        assert_eq!(effective, stack_va + 8 + 24);
        {
            let mut paused = space.pause().unwrap();
            paused
                .write(effective, &(bias + ENTRY as u64).to_le_bytes())
                .unwrap();
        }
        let resolved = resolve_target(&ctx, &mut space, ControlTarget::Memory(operand)).unwrap();
        assert_eq!(resolved, bias + ENTRY as u64);
        // A register target reads straight from the context.
        ctx.gpr[11] = 0x1234;
        assert_eq!(
            resolve_target(&ctx, &mut space, ControlTarget::Register(Register::R11)).unwrap(),
            0x1234
        );
    }

    #[test]
    fn dispatch_cannot_run_concurrently() {
        let _mapping_guard = vm_lock();
        let _guard = DispatchGuard::acquire().unwrap();
        assert!(matches!(
            DispatchGuard::acquire(),
            Err(PlatformError::TrapScopeUnavailable)
        ));
    }

    #[test]
    fn return_rejection_preserves_rsp_and_only_the_outer_stop_slot_completes() {
        let _mapping_guard = vm_lock();
        let _dispatch_guard = DispatchGuard::acquire().unwrap();
        let (plan, mut space, rsp, cache) = staged_image(build_elf(&[0xc3]));
        let bias = plan.modules()[0].load_bias();
        let block = entry_block(&plan, &cache);
        let mut ctx = GuestContext::zeroed();
        ctx.gpr[RSP] = rsp;
        commit_rsp(0xfeed);
        space
            .pause()
            .unwrap()
            .write(rsp, &(bias + DATA_OFF).to_le_bytes())
            .unwrap();
        assert!(matches!(
            resolve_exit(&plan, &mut space, &block, &ctx, rsp),
            Err(PlatformError::GuestTargetRejected { .. })
        ));
        // SAFETY: The exclusive DispatchGuard is held and no bridge runs.
        assert_eq!(unsafe { (*CONTEXT.0.get()).gpr[RSP] }, 0xfeed);
        ctx.gpr[RSP] = rsp - 8;
        space
            .pause()
            .unwrap()
            .write(rsp - 8, &STOP.to_le_bytes())
            .unwrap();
        assert!(matches!(
            resolve_exit(&plan, &mut space, &block, &ctx, rsp),
            Err(PlatformError::GuestTargetRejected { target: STOP })
        ));
        // SAFETY: The same exclusive guard still owns the paused context.
        assert_eq!(unsafe { (*CONTEXT.0.get()).gpr[RSP] }, 0xfeed);
        ctx.gpr[RSP] = rsp;
        space
            .pause()
            .unwrap()
            .write(rsp, &STOP.to_le_bytes())
            .unwrap();
        assert!(matches!(
            resolve_exit(&plan, &mut space, &block, &ctx, rsp).unwrap(),
            Next::Done
        ));
        // SAFETY: The same exclusive guard still owns the paused context.
        assert_eq!(unsafe { (*CONTEXT.0.get()).gpr[RSP] }, rsp + 8);
    }
}
