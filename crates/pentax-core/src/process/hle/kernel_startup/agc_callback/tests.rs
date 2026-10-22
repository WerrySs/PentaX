// SPDX-License-Identifier: GPL-2.0-only
//! Original callback target-admission proof; actual execution lives in worker tests.
use super::*;
use crate::process::hle::{NativeImageLimits, with_native_image_session};
use pentax_codegen::BlockLimits;
use pentax_platform::native_memory::{
    NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn request(
    owner: &PreparedNativeKernelStartup<'_, '_>,
    function: crate::gpu::driver::Function,
    args: [u64; 6],
    stack: u64,
) -> NativeServiceRequest {
    let binding = owner.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|marker| marker.function == function)
        .unwrap();
    let pc = binding.bias + marker.rva() + 5;
    NativeServiceRequest {
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.id().into(),
        args,
        vector_args: None,
        stack_pointer: Some(stack),
    }
}
fn descriptor(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    buf: u64,
    up: u64,
    target: u64,
    user: u64,
) {
    let mut bytes = [0; 56];
    bytes[16..24].copy_from_slice(&up.to_le_bytes());
    bytes[24..32].copy_from_slice(&up.to_le_bytes()); // Requires actual growth.
    bytes[32..40].copy_from_slice(&target.to_le_bytes());
    bytes[40..48].copy_from_slice(&user.to_le_bytes());
    bytes[48..52].copy_from_slice(&2u32.to_le_bytes());
    owner.pause().unwrap().write(buf, &bytes).unwrap();
}

#[test]
fn grow_target_requires_actual_initialized_immutable_source_and_failed_admission_has_no_prefix() {
    for function in [
        crate::gpu::driver::Function::AgcDcbWriteData,
        crate::gpu::driver::Function::AgcCbSetShRegistersDirect,
        crate::gpu::driver::Function::AgcCbSetUcRegistersDirect,
    ] {
        for kind in 0..4 {
            let mut primary = crate::process::hle::unavailable::tests::original_input(0);
            primary[104..112].copy_from_slice(&(2 * crate::memory::PAGE_SIZE).to_le_bytes());
            with_native_image_session(
                &[&primary],
                &NativeImageLimits::default(),
                |owner, layout| {
                    let proof = owner
                        .prepare_native_source(&BlockLimits::default())
                        .unwrap();
                    let buf = layout.entry.stack.address + 0x100;
                    let up = buf + 0x100;
                    let module = &proof.materialized_source().source_plan().modules()[0];
                    let segment = &module.runtime_plan().segments[0];
                    let target = match kind {
                        0 => u64::MAX,
                        1 => module.load_bias() + segment.file_bytes.len() as u64,
                        2 => up,
                        _ => {
                            owner
                                .pause()
                                .unwrap()
                                .apply_transaction(
                                    NativeMemoryTransaction {
                                        effect: NativeMemoryEffect::Protect {
                                            address: module.load_bias(),
                                            size: crate::memory::PAGE_SIZE,
                                            permissions: NativePermissions::READ_WRITE,
                                        },
                                        writes: &[],
                                    },
                                    &Default::default(),
                                )
                                .unwrap();
                            layout.entry.entry
                        }
                    };
                    descriptor(owner, buf, up, target, 0);
                    let parent_rsp = layout.entry.return_slot - 64;
                    owner
                        .pause()
                        .unwrap()
                        .write(parent_rsp + 8, &[0; 16])
                        .unwrap();
                    let args = if function == crate::gpu::driver::Function::AgcDcbWriteData {
                        [buf, 1, 0, 0, up, 2]
                    } else {
                        let regs = buf + 0x300;
                        owner.pause().unwrap().write(regs, &[0; 16]).unwrap();
                        [buf, regs, 2, 0, 0, 0]
                    };
                    let raw = request(owner, function, args, parent_rsp);
                    assert!(owner.service_action(&proof, raw).is_err());
                    assert!(owner.agc_callbacks.pending.is_empty());
                    let mut cursor = [0; 8];
                    owner.pause().unwrap().read(buf + 16, &mut cursor).unwrap();
                    assert_eq!(u64::from_le_bytes(cursor), up);
                    Ok::<_, ()>(())
                },
            )
            .unwrap();
        }
    }
}

#[test]
fn pending_range_and_group_capacities_share_the_original_aggregate_payload_bound() {
    use crate::gpu::driver::{Driver, Function, Limits};
    for large_group in [false, true] {
        let primary = crate::process::hle::unavailable::tests::original_input(0);
        with_native_image_session(
            &[&primary],
            &NativeImageLimits::default(),
            |owner, layout| {
                let proof = owner
                    .prepare_native_source(&BlockLimits::default())
                    .unwrap();
                // This original fixture admits one larger source array through a
                // caller-selected per-call command limit. The aggregate callback
                // payload cap and all production defaults remain unchanged.
                let mut limits = Limits::default();
                if large_group {
                    limits.commands.max_command_bytes = 128 * 1024;
                }
                owner.graphics = Some(Driver::new(limits));
                let base = layout.entry.stack.address;
                let (buf, up, regs) = (base + 0x100, base + 0x1000, base + 0x10000);
                descriptor(owner, buf, up, layout.entry.entry, 0);
                owner.pause().unwrap().write(up, &[0xa5; 32]).unwrap();
                let parent_rsp = layout.entry.return_slot - 64;
                let mut admitted = 0;
                if large_group {
                    let source = vec![0; 9_000 * 8];
                    owner.pause().unwrap().write(regs, &source).unwrap();
                    let raw = request(
                        owner,
                        Function::AgcCbSetShRegistersDirect,
                        [buf, regs, 9_000, 0, 0, 0],
                        parent_rsp,
                    );
                    assert!(matches!(
                        owner.service_action(&proof, raw).unwrap(),
                        NativeServiceAction::AgcCommandGrow(_)
                    ));
                    let body = owner.agc_callbacks.pending[0].body.as_ref().unwrap();
                    assert!(body.retained_payload_bytes().unwrap() >= 72_000);
                    admitted += 1;
                }
                let range = request(
                    owner,
                    Function::AgcCbSetShRegisterRangeDirect,
                    [buf, 0, regs, 16_382, 0, 0],
                    parent_rsp,
                );
                // These are genuine source-bound callback admissions only. No
                // callback is executed and no nested-return receipt is fabricated.
                let rejected = loop {
                    let serial = owner.agc_callbacks.next_serial;
                    let previous: Vec<_> = owner
                        .agc_callbacks
                        .pending
                        .iter()
                        .map(|pending| pending.start)
                        .collect();
                    let retained: usize = owner
                        .agc_callbacks
                        .pending
                        .iter()
                        .map(|pending| {
                            pending
                                .body
                                .as_ref()
                                .unwrap()
                                .retained_payload_bytes()
                                .unwrap()
                        })
                        .sum();
                    match owner.service_action(&proof, range) {
                        Ok(NativeServiceAction::AgcCommandGrow(_)) => {
                            admitted += 1;
                            assert_eq!(owner.agc_callbacks.pending.len(), admitted);
                            let now: usize = owner
                                .agc_callbacks
                                .pending
                                .iter()
                                .map(|pending| {
                                    pending
                                        .body
                                        .as_ref()
                                        .unwrap()
                                        .retained_payload_bytes()
                                        .unwrap()
                                })
                                .sum();
                            assert!(now > retained && now <= MAX_RETAINED_PAYLOAD_BYTES);
                            assert!(admitted < MAX_PENDING);
                        }
                        Err(KernelStartupError::Budget("AGC retained callback payload")) => {
                            assert!(retained + 65_528 > MAX_RETAINED_PAYLOAD_BYTES);
                            assert_eq!(owner.agc_callbacks.next_serial, serial);
                            assert_eq!(owner.agc_callbacks.pending.len(), admitted);
                            assert!(owner.agc_callbacks.pending.iter().zip(previous).all(
                                |(pending, start)| pending.start == start
                                    && matches!(pending.phase, Phase::Admitted)
                                    && pending.body.is_some()
                            ));
                            break true;
                        }
                        _ => panic!("expected an admitted callback or aggregate payload refusal"),
                    }
                };
                assert!(rejected && admitted < MAX_PENDING);
                let mut cursor = [0; 8];
                owner.pause().unwrap().read(buf + 16, &mut cursor).unwrap();
                assert_eq!(u64::from_le_bytes(cursor), up);
                let mut canary = [0; 32];
                owner.pause().unwrap().read(up, &mut canary).unwrap();
                assert_eq!(canary, [0xa5; 32]);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}
