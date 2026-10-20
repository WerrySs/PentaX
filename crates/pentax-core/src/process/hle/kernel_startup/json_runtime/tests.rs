// SPDX-License-Identifier: GPL-2.0-only
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

use super::*;
use crate::process::hle::{
    NativeImageLimits, build_json_provider, unavailable::tests::original_input,
    with_native_image_session,
};
use pentax_platform::native_memory::{
    NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
};

fn request(
    session: &PreparedNativeKernelStartup<'_, '_>,
    object: u64,
    slot: usize,
) -> NativeServiceRequest {
    let binding = session.plan.json.unwrap();
    let marker = binding.factory.markers()[slot];
    NativeServiceRequest {
        vector_args: None,
        owner_id: session.owner_id(),
        syscall_pc: binding.bias + marker.rva + 5,
        next_pc: binding.bias + marker.rva + 7,
        service_id: u64::from(marker.service_id),
        args: [object, u64::MAX, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
        stack_pointer: None,
    }
}

#[test]
fn json_initializer_constructor_returns_full_opaque_self_without_reads_or_writes() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let object = layout.entry.stack.address + 0x101;
            let sentinel = [0xa7; 48];
            session
                .pause()
                .unwrap()
                .write(object - 1, &sentinel)
                .unwrap();
            let binding = session.plan.json.unwrap();
            for opaque_self in [
                0,
                u64::MAX,
                0x8123_4567_89ab_cdef,
                object,
                binding.factory.address_point(),
            ] {
                let mut call = request(session, opaque_self, 34);
                call.args[1..].copy_from_slice(&[
                    0xffff_ffff_ffff_fff1,
                    0x9234_5678_9abc_def2,
                    0xa345_6789_abcd_ef03,
                    0xb456_789a_bcde_f014,
                    0xc567_89ab_cdef_0125,
                ]);
                assert_eq!(session.service_request(call), Ok(opaque_self));
            }
            let mut observed = [0; 48];
            session
                .pause()
                .unwrap()
                .read(object - 1, &mut observed)
                .unwrap();
            assert_eq!(observed, sentinel);
            let mut defaults = [0; 104];
            session
                .pause()
                .unwrap()
                .read(binding.bias + PAGE_SIZE, &mut defaults)
                .unwrap();
            assert_eq!(defaults.as_slice(), binding.factory.data_bytes());
            let good = request(session, object, 34);
            let foreign = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
            let mut wrong = good;
            wrong.owner_id = foreign.owner_id();
            assert_eq!(
                session.service_request(wrong),
                Err(KernelStartupError::NativeOwnerMismatch)
            );
            wrong = good;
            wrong.service_id |= 1 << 32;
            assert!(matches!(
                session.service_request(wrong),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            wrong = good;
            wrong.next_pc += 1;
            assert!(matches!(
                session.service_request(wrong),
                Err(KernelStartupError::ContinuationMismatch { .. })
            ));
            session.provider_work = session.plan.limits.max_provider_work - 1;
            assert!(matches!(
                session.service_request(good),
                Err(KernelStartupError::Budget(_))
            ));
            session
                .pause()
                .unwrap()
                .read(object - 1, &mut observed)
                .unwrap();
            assert_eq!(observed, sentinel);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn constructor_writes_one_unaligned_guest_word_and_refuses_forged_requests_before_effects() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let object = layout.entry.stack.address + 0x101;
            let binding = session.plan.json.unwrap();
            let sentinel = [0x5a; 24];
            session
                .pause()
                .unwrap()
                .write(object - 1, &sentinel)
                .unwrap();
            let good = request(session, object, 0);
            let foreign = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
            let mut bad = good;
            bad.owner_id = foreign.owner_id();
            assert_eq!(
                session.service_request(bad),
                Err(KernelStartupError::NativeOwnerMismatch)
            );
            bad = good;
            bad.service_id |= 1 << 32;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            bad = good;
            bad.next_pc += 1;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ContinuationMismatch { .. })
            ));
            bad = good;
            bad.syscall_pc += 1;
            bad.next_pc += 1;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::SiteMismatch { .. })
            ));
            let mut observed = [0; 24];
            session
                .pause()
                .unwrap()
                .read(object - 1, &mut observed)
                .unwrap();
            assert_eq!(observed, sentinel);
            session.service_request(good).unwrap();
            session
                .pause()
                .unwrap()
                .read(object - 1, &mut observed)
                .unwrap();
            assert_eq!(
                &observed[1..9],
                &binding.factory.address_point().to_le_bytes()
            );
            assert_eq!(observed[0], sentinel[0]);
            assert_eq!(&observed[9..], &sentinel[9..]);
            for word in binding.factory.table_bytes()[16..].chunks_exact(8) {
                let mut virtual_call = request(session, object, 1);
                virtual_call.syscall_pc = u64::from_le_bytes(word.try_into().unwrap()) + 5;
                virtual_call.next_pc = virtual_call.syscall_pc + 2;
                assert_eq!(
                    session.service_request(virtual_call),
                    Err(KernelStartupError::UnavailableJsonAllocatorVirtual)
                );
            }
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn invalid_constructor_destinations_preserve_bytes_and_do_not_touch_table() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let tail = layout.entry.stack.address + layout.entry.stack.size - 4;
            session.pause().unwrap().write(tail, &[0x71; 4]).unwrap();
            let binding = session.plan.json.unwrap();
            for object in [0, u64::MAX - 3, tail, binding.factory.address_point()] {
                let call = request(session, object, 0);
                assert!(session.service_request(call).is_err());
            }
            let mut observed = [0; 4];
            session.pause().unwrap().read(tail, &mut observed).unwrap();
            assert_eq!(observed, [0x71; 4]);
            let mut table = [0; 64];
            session
                .pause()
                .unwrap()
                .read(binding.bias + PAGE_SIZE, &mut table)
                .unwrap();
            assert_eq!(table.as_slice(), binding.factory.table_bytes());
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn source_identity_placement_and_work_limits_precede_constructor_effects() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let binding = session.plan.json.unwrap();
            let other = build_json_provider(
                ProviderPlacement {
                    load_bias: binding.bias,
                },
                PAGE_SIZE,
            )
            .unwrap();
            let detached = JsonBinding {
                factory: &other,
                index: binding.index,
                bias: binding.bias,
            };
            assert!(matches!(
                validate(
                    session.plan.link_plan(),
                    &detached,
                    &session.plan.limits,
                    &mut 0
                ),
                Err(KernelStartupError::InvalidProvider(_))
            ));
            let mut inputs = Vec::new();
            assert!(
                append(
                    &mut inputs,
                    JsonStartupOptions {
                        provider: &other,
                        placement: ProviderPlacement {
                            load_bias: binding.bias + PAGE_SIZE
                        }
                    },
                    &session.plan.limits
                )
                .is_err()
            );
            assert!(inputs.is_empty());
            let mut limits = session.plan.limits;
            limits.startup.hle.max_provider_bytes = other.image().len() as u64 - 1;
            assert!(matches!(
                append(
                    &mut inputs,
                    JsonStartupOptions {
                        provider: &other,
                        placement: ProviderPlacement {
                            load_bias: binding.bias
                        }
                    },
                    &limits
                ),
                Err(KernelStartupError::Budget(_))
            ));
            assert!(inputs.is_empty());
            let object = layout.entry.stack.address + 0x100;
            session.pause().unwrap().write(object, &[0x39; 8]).unwrap();
            session.provider_work = session.plan.limits.max_provider_work - 1;
            let call = request(session, object, 0);
            assert!(matches!(
                session.service_request(call),
                Err(KernelStartupError::Budget(_))
            ));
            let mut observed = [0; 8];
            session
                .pause()
                .unwrap()
                .read(object, &mut observed)
                .unwrap();
            assert_eq!(observed, [0x39; 8]);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn table_write_permission_or_live_table_tampering_cannot_authorize_constructor() {
    let input = original_input(0);
    for restore_read_only in [false, true] {
        with_native_image_session(
            &[&input],
            &NativeImageLimits::default(),
            |session, layout| {
                let binding = session.plan.json.unwrap();
                let object = layout.entry.stack.address + 0x100;
                session.pause().unwrap().write(object, &[0x3b; 8]).unwrap();
                {
                    let mut paused = session.pause().unwrap();
                    paused
                        .apply_transaction(
                            NativeMemoryTransaction {
                                effect: NativeMemoryEffect::Protect {
                                    address: binding.bias + PAGE_SIZE,
                                    size: PAGE_SIZE,
                                    permissions: NativePermissions::READ_WRITE,
                                },
                                writes: &[],
                            },
                            &Default::default(),
                        )
                        .unwrap();
                    if restore_read_only {
                        paused
                            .write(binding.factory.address_point(), &0_u64.to_le_bytes())
                            .unwrap();
                        paused
                            .apply_transaction(
                                NativeMemoryTransaction {
                                    effect: NativeMemoryEffect::Protect {
                                        address: binding.bias + PAGE_SIZE,
                                        size: PAGE_SIZE,
                                        permissions: NativePermissions::new(true, false, false),
                                    },
                                    writes: &[],
                                },
                                &Default::default(),
                            )
                            .unwrap();
                    }
                }
                let call = request(session, object, 0);
                assert!(matches!(
                    session.service_request(call),
                    Err(KernelStartupError::InvalidProvider(_))
                ));
                let mut observed = [0; 8];
                session
                    .pause()
                    .unwrap()
                    .read(object, &mut observed)
                    .unwrap();
                assert_eq!(observed, [0x3b; 8]);
                Ok::<_, ()>(())
            },
        )
        .unwrap();
    }
}

#[test]
fn value_constructor_initializes_all_32_bytes_returns_self_and_accepts_null() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let object = layout.entry.stack.address + 0x101;
            session
                .pause()
                .unwrap()
                .write(object - 1, &[0xa7; 34])
                .unwrap();
            let null_call = request(session, 0, 2);
            assert_eq!(session.service_request(null_call).unwrap(), 0);
            let call = request(session, object, 2);
            assert_eq!(session.service_request(call).unwrap(), object);
            let mut observed = [0; 34];
            session
                .pause()
                .unwrap()
                .read(object - 1, &mut observed)
                .unwrap();
            assert_eq!(observed[0], 0xa7);
            assert_eq!(&observed[1..33], &[0; 32]);
            assert_eq!(observed[33], 0xa7);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn value_constructor_refuses_whole_span_errors_and_forged_requests_without_zeroing_prefixes() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let object = layout.entry.stack.address + 0x101;
            let tail = layout.entry.stack.address + layout.entry.stack.size - 16;
            session.pause().unwrap().write(object, &[0x91; 32]).unwrap();
            session.pause().unwrap().write(tail, &[0x72; 16]).unwrap();
            let binding = session.plan.json.unwrap();
            for invalid in [tail, u64::MAX - 15, binding.factory.address_point()] {
                let call = request(session, invalid, 2);
                assert!(matches!(
                    session.service_request(call),
                    Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(_)))
                ));
            }
            let good = request(session, object, 2);
            let mut bad = good;
            bad.service_id |= 1 << 32;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            bad = good;
            bad.service_id = u64::from(binding.factory.markers()[0].service_id);
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            let mut observed = [0; 32];
            session
                .pause()
                .unwrap()
                .read(object, &mut observed)
                .unwrap();
            assert_eq!(observed, [0x91; 32]);
            session
                .pause()
                .unwrap()
                .read(tail, &mut observed[..16])
                .unwrap();
            assert_eq!(&observed[..16], &[0x72; 16]);
            session.provider_work = session.plan.limits.max_provider_work - 1;
            assert!(matches!(
                session.service_request(good),
                Err(KernelStartupError::Budget(_))
            ));
            session
                .pause()
                .unwrap()
                .read(object, &mut observed)
                .unwrap();
            assert_eq!(observed, [0x91; 32]);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn scalar_default_pointers_are_distinct_readonly_nx_and_tampering_stops_dispatch() {
    let input = original_input(0);
    with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
        let binding = session.plan.json.unwrap();
        let mut addresses = Vec::new();
        for function in [
            JsonFunction::ValueGetBoolean,
            JsonFunction::ValueGetInteger,
            JsonFunction::ValueGetReal,
        ] {
            let slot = binding
                .factory
                .markers()
                .iter()
                .position(|m| m.function == function)
                .unwrap();
            let call = request(session, 0, slot);
            let address = session.service_request(call).unwrap();
            assert_eq!(
                address,
                binding.bias + binding.factory.scalar_default_rva(function).unwrap()
            );
            assert!(!addresses.contains(&address));
            addresses.push(address);
            let paused = session.pause().unwrap();
            let mut bytes = [1; 8];
            paused.read(address, &mut bytes).unwrap();
            assert_eq!(bytes, [0; 8]);
            assert!(matches!(
                paused.preflight_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.preflight_gpu_write(address, 8),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.fetch_source(address, &mut [0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
        }
        {
            let mut paused = session.pause().unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias + PAGE_SIZE,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            paused.write(addresses[1], &1_u64.to_le_bytes()).unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias + PAGE_SIZE,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::new(true, false, false),
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
        let slot = binding
            .factory
            .markers()
            .iter()
            .position(|m| m.function == JsonFunction::ValueGetBoolean)
            .unwrap();
        let call = request(session, 0, slot);
        assert!(matches!(
            session.service_request(call),
            Err(KernelStartupError::InvalidProvider(_))
        ));
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn authenticated_scalar_stop_work_is_independent_of_unrelated_import_count() {
    let mut costs = Vec::new();
    for count in [0, 64] {
        let input = original_input(count);
        with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
            let binding = session.plan.json.unwrap();
            let slot = binding
                .factory
                .markers()
                .iter()
                .position(|m| m.function == JsonFunction::ValueGetType)
                .unwrap();
            session.provider_work = 0;
            let call = request(session, 0, slot);
            assert_eq!(session.service_request(call).unwrap(), 0);
            costs.push(session.provider_work);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
    assert_eq!(costs[0], costs[1]);
    assert!(costs[0] > 0);
}

#[test]
fn double_dispatch_requires_captured_vectors_and_publishes_only_authenticated_raw_bits() {
    let input = original_input(0);
    with_native_image_session(
        &[&input],
        &NativeImageLimits::default(),
        |session, layout| {
            let object = layout.entry.stack.address + 0x101;
            let sentinel = [0xa7; 34];
            for (function, returns_self) in [
                (JsonFunction::ValueDoubleCtor, true),
                (JsonFunction::ValueSetDouble, false),
            ] {
                let slot = session
                    .plan
                    .json
                    .unwrap()
                    .factory
                    .markers()
                    .iter()
                    .position(|marker| marker.function == function)
                    .unwrap();
                session
                    .pause()
                    .unwrap()
                    .write(object - 1, &sentinel)
                    .unwrap();
                for destination in [object, 0] {
                    let missing = request(session, destination, slot);
                    assert_eq!(
                        session.service_request(missing),
                        Err(KernelStartupError::UnavailableVectorArguments)
                    );
                }
                let mut observed = [0; 34];
                session
                    .pause()
                    .unwrap()
                    .read(object - 1, &mut observed)
                    .unwrap();
                assert_eq!(observed, sentinel);
                let bits = 0xfff0_1234_5678_9abc_u64;
                let mut vectors = [[0x53; 16]; 8];
                vectors[0][..8].copy_from_slice(&bits.to_le_bytes());
                let mut call = request(session, object, slot);
                call.vector_args = Some(vectors);
                let mut forged = call;
                forged.next_pc += 1;
                assert!(matches!(
                    session.service_request(forged),
                    Err(KernelStartupError::ContinuationMismatch { .. })
                ));
                session
                    .pause()
                    .unwrap()
                    .read(object - 1, &mut observed)
                    .unwrap();
                assert_eq!(observed, sentinel);
                // Unknown old types take the source Clear default, with no
                // allocation or invented ownership. Padding remains observable.
                assert_eq!(
                    session.service_request(call).unwrap(),
                    if returns_self { object } else { 0 }
                );
                session
                    .pause()
                    .unwrap()
                    .read(object - 1, &mut observed)
                    .unwrap();
                let mut expected = sentinel;
                expected[1..25].fill(0);
                expected[17..25].copy_from_slice(&bits.to_le_bytes());
                if returns_self {
                    expected[25..29].fill(0);
                }
                expected[29..33].copy_from_slice(&4_u32.to_le_bytes());
                assert_eq!(observed, expected);
                call.args[0] = 0;
                assert_eq!(session.service_request(call).unwrap(), 0);
            }
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}

#[test]
fn string_default_queries_use_owned_immutable_guest_storage_and_attest_its_live_nul() {
    let input = original_input(0);
    with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
        let binding = session.plan.json.unwrap();
        let slot = |function| {
            binding
                .factory
                .markers()
                .iter()
                .position(|marker| marker.function == function)
                .unwrap()
        };
        let getter = request(session, 0, slot(JsonFunction::ValueGetString));
        let wrapper = session.service_request(getter).unwrap();
        assert_eq!(wrapper, binding.bias + binding.factory.empty_string_rva());
        let cstr = request(session, wrapper, slot(JsonFunction::StringCStr));
        let bytes = session.service_request(cstr).unwrap();
        assert_eq!(bytes, binding.bias + binding.factory.empty_cstr_rva());
        let length = request(session, wrapper, slot(JsonFunction::StringLength));
        assert_eq!(session.service_request(length).unwrap(), 0);
        for address in [wrapper, bytes] {
            let paused = session.pause().unwrap();
            assert!(matches!(
                paused.preflight_write(address, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.preflight_gpu_write(address, 1),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
            assert!(matches!(
                paused.fetch_source(address, &mut [0]),
                Err(NativeMemoryError::PermissionDenied(_))
            ));
        }
        {
            let mut paused = session.pause().unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias + PAGE_SIZE,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            paused.write(bytes, &[1]).unwrap();
            paused
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias + PAGE_SIZE,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::READ,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
        }
        assert!(matches!(
            session.service_request(length),
            Err(KernelStartupError::InvalidProvider(_))
        ));
        Ok::<_, ()>(())
    })
    .unwrap();
}
