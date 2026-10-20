// SPDX-License-Identifier: GPL-2.0-only

//! Original ELF inputs and data-only provider/preparation tests. No CPU enters.

use super::*;
use pentax_linker::{
    DT_SCE_IMPORT_LIB, DT_SCE_NEEDED_MODULE, ModuleInput, Resolution, SELECTOR_ALPHABET,
    plan_missing_functions, plan_symbolic,
};
use pentax_loader::open_executable;

pub(crate) fn original_input(count: usize) -> Vec<u8> {
    let symbols = 0x400;
    let strings = (symbols + (count + 1) * 24 + 15) & !15;
    let mut names = vec![0];
    let mut append = |value: &[u8]| {
        let offset = names.len() as u64;
        names.extend_from_slice(value);
        names.push(0);
        offset
    };
    let module = append(b"original-module");
    let library = append(b"original-library");
    let mut offsets = Vec::new();
    for index in 0..count {
        let mut name = *b"AAAAAAAAAAA#D#C";
        name[0] = SELECTOR_ALPHABET[(index / 64) % 64];
        name[1] = SELECTOR_ALPHABET[index % 64];
        offsets.push(append(&name));
    }
    let rela = (strings + names.len() + 15) & !15;
    let targets = (rela + count * 24).div_ceil(PAGE_SIZE as usize) * PAGE_SIZE as usize;
    let length = targets + count * 8;
    let span = length.div_ceil(PAGE_SIZE as usize) * PAGE_SIZE as usize;
    let mut image = vec![0; length];
    image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut image, 16, &3_u16.to_le_bytes());
    put(&mut image, 18, &62_u16.to_le_bytes());
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 24, &0x100_u64.to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    put(&mut image, 52, &64_u16.to_le_bytes());
    put(&mut image, 54, &56_u16.to_le_bytes());
    put(&mut image, 56, &2_u16.to_le_bytes());
    image[0x100] = 0xc3;
    program(
        &mut image,
        0,
        1,
        5,
        [0, 0, length as u64, span as u64, PAGE_SIZE],
    );
    program(&mut image, 1, 2, 4, [0x200, 0x200, 11 * 16, 11 * 16, 8]);
    put(&mut image, strings, &names);
    for (index, name) in offsets.into_iter().enumerate() {
        let row = symbols + (index + 1) * 24;
        put(&mut image, row, &(name as u32).to_le_bytes());
        image[row + 4] = 0x12;
        let at = rela + index * 24;
        put(
            &mut image,
            at,
            &((targets + index * 8) as u64).to_le_bytes(),
        );
        put(
            &mut image,
            at + 8,
            &((((index + 1) as u64) << 32) | 1).to_le_bytes(),
        );
    }
    for (index, (tag, value)) in [
        (5_i64, strings as u64),
        (10, names.len() as u64),
        (6, symbols as u64),
        (39, ((count + 1) * 24) as u64),
        (11, 24),
        (
            DT_SCE_NEEDED_MODULE,
            module | (3 << 32) | (2 << 40) | (2 << 48),
        ),
        (DT_SCE_IMPORT_LIB, library | (7 << 32) | (3 << 48)),
        (7, rela as u64),
        (8, (count * 24) as u64),
        (9, 24),
        (0, 0),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut image, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut image, 0x208 + index * 16, &value.to_le_bytes());
    }
    image
}

#[test]
fn unavailable_exports_link_exact_versions_selectors_and_unique_function_targets() {
    for count in [0, 1, 65] {
        let bytes = original_input(count);
        let inputs = [ModuleInput::new(
            open_executable(&bytes).unwrap(),
            0x100_0000,
        )];
        let missing = plan_missing_functions(&inputs, &Default::default(), 128).unwrap();
        let provider = build_unavailable_provider(missing, &Default::default()).unwrap();
        assert_eq!(provider.function_count(), count);
        assert!(provider.image().len() as u64 <= provider.mapped_bytes());
        assert_eq!(provider.site(0).unwrap().ordinal, None);
        assert_eq!(
            provider.site(0).unwrap().bytes(),
            [0xb8, 0, 0, 0, 0x20, 0x0f, 0x05, 0xc3]
        );
        let mut supplied = inputs.to_vec();
        supplied.push(ModuleInput::new(
            open_executable(provider.image()).unwrap(),
            0x200_0000,
        ));
        let linked = plan_symbolic(&supplied, &Default::default()).unwrap();
        assert_eq!(linked.patch_count(), count);
        for patch in linked.patches() {
            let Resolution::Definition(symbol) = patch.resolution() else {
                panic!("expected generated function")
            };
            assert_eq!(symbol.module.index(), 1);
            let site = provider.symbol_site(symbol.symbol).unwrap();
            assert_eq!(patch.value(), 0x200_0000 + site.rva);
            assert_eq!(
                provider
                    .syscall_site(0x200_0000, patch.value() + 5)
                    .unwrap()
                    .ordinal,
                site.ordinal
            );
            assert!(
                provider
                    .syscall_site(0x200_0000, patch.value() + 4)
                    .is_none()
            );
        }
        assert!(!format!("{provider:?}").contains("original-module"));
    }
}

#[test]
fn unavailable_provider_rejects_function_byte_and_work_limits_before_publication() {
    let bytes = original_input(1);
    let inputs = [ModuleInput::new(
        open_executable(&bytes).unwrap(),
        0x100_0000,
    )];
    for limits in [
        UnavailableProviderLimits {
            max_functions: 0,
            ..Default::default()
        },
        UnavailableProviderLimits {
            max_image_bytes: 0,
            ..Default::default()
        },
        UnavailableProviderLimits {
            max_work_steps: 0,
            ..Default::default()
        },
    ] {
        let missing = plan_missing_functions(&inputs, &Default::default(), 1).unwrap();
        assert!(matches!(
            build_unavailable_provider(missing, &limits),
            Err(UnavailableProviderError::Budget(_))
        ));
    }
}

#[test]
fn unavailable_factory_cannot_override_actual_classification_or_mask_an_existing_version() {
    use crate::process::hle::*;
    use pentax_linker::SymbolicError;
    let limits = KernelStartupLimits::default();
    let kernel = build_kernel_provider(limits.max_kernel_provider_bytes).unwrap();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.startup.hle).unwrap();
    let original = original_input(1);
    let input = [ModuleInput::new(
        open_executable(&original).unwrap(),
        0x100_0000,
    )];
    let missing = plan_missing_functions(&input, &Default::default(), 16).unwrap();
    let old = build_unavailable_provider(missing, &Default::default()).unwrap();
    let empty = original_input(0);
    let input = [ModuleInput::new(
        open_executable(&empty).unwrap(),
        0x100_0000,
    )];
    assert!(matches!(
        plan_kernel_startup_with_unavailable(
            &input,
            &kernel,
            ProviderPlacement {
                load_bias: 0x200_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x300_0000
            },
            UnavailableProviderPlacement {
                provider: &old,
                placement: ProviderPlacement {
                    load_bias: 0x400_0000
                }
            },
            &limits
        ),
        Err(KernelStartupError::InvalidProvider(_))
    ));
    let mut newer = original_input(1);
    let mut word = u64::from_le_bytes(newer[0x258..0x260].try_into().unwrap());
    word = (word & !(0xff << 40)) | (3 << 40);
    newer[0x258..0x260].copy_from_slice(&word.to_le_bytes());
    let input = [ModuleInput::new(
        open_executable(&newer).unwrap(),
        0x100_0000,
    )];
    let missing = plan_missing_functions(&input, &Default::default(), 16).unwrap();
    let new = build_unavailable_provider(missing, &Default::default()).unwrap();
    let inputs = [
        input[0].clone(),
        ModuleInput::new(open_executable(old.image()).unwrap(), 0x180_0000),
    ];
    assert!(matches!(
        plan_kernel_startup_with_unavailable(
            &inputs,
            &kernel,
            ProviderPlacement {
                load_bias: 0x200_0000
            },
            &libc,
            ProviderPlacement {
                load_bias: 0x300_0000
            },
            UnavailableProviderPlacement {
                provider: &new,
                placement: ProviderPlacement {
                    load_bias: 0x400_0000
                }
            },
            &limits
        ),
        Err(KernelStartupError::Hle(HleError::Link(
            SymbolicError::VersionMismatch { .. }
        )))
    ));
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod native {
    use super::*;
    use crate::memory::native::NativeLoadLimits;
    use crate::process::SessionLoadLimits;
    use crate::process::hle::*;
    use crate::process::tls::StaticTlsStorage;
    use pentax_kernel::ProcessIdentity;
    use pentax_platform::native_memory::{
        NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
    };

    fn prepared(
        test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, &GeneratedUnavailableProvider, u64),
    ) {
        let owner = NativeAddressSpace::reserve_any(Default::default(), 0x10_0000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        let bytes = original_input(1);
        let limits = KernelStartupLimits::default();
        let kernel = build_kernel_provider(limits.max_kernel_provider_bytes).unwrap();
        let libc = build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.startup.hle)
            .unwrap();
        let kernel_bias = base + 0x2_0000;
        // Place the next provider beyond the complete current kernel mapping,
        // including its separately protected, page-rounded metadata segment.
        let libc_bias = (kernel_bias + kernel.mapped_bytes()).div_ceil(0x1_0000) * 0x1_0000;
        let libc_extent = open_executable(libc.image())
            .unwrap()
            .runtime_plan()
            .unwrap()
            .segments
            .iter()
            .map(|segment| segment.vaddr + segment.mem_size)
            .max()
            .unwrap();
        let thread_pointer =
            (libc_bias + libc_extent).div_ceil(0x1_0000) * 0x1_0000 + 2 * PAGE_SIZE;
        assert!(thread_pointer + PAGE_SIZE <= base + 0x10_0000);
        let inputs = [ModuleInput::new(open_executable(&bytes).unwrap(), base)];
        let mut real = inputs.to_vec();
        real.push(ModuleInput::new(
            open_executable(kernel.image()).unwrap(),
            kernel_bias,
        ));
        real.push(ModuleInput::new(
            open_executable(libc.image()).unwrap(),
            libc_bias,
        ));
        let missing = plan_missing_functions(&real, &limits.startup.hle.linking, 16).unwrap();
        let unavailable = build_unavailable_provider(missing, &Default::default()).unwrap();
        let unavailable_bias = base + 0x1_0000;
        assert!(unavailable_bias + unavailable.mapped_bytes() <= kernel_bias);
        let plan = plan_kernel_startup_with_unavailable(
            &inputs,
            &kernel,
            ProviderPlacement {
                load_bias: kernel_bias,
            },
            &libc,
            ProviderPlacement {
                load_bias: libc_bias,
            },
            UnavailableProviderPlacement {
                provider: &unavailable,
                placement: ProviderPlacement {
                    load_bias: unavailable_bias,
                },
            },
            &limits,
        )
        .unwrap();
        assert_eq!(plan.unavailable_functions(), 1);
        // These original images contain no TLS payload/header/storage. Admit
        // exactly their actual disjoint load pages, rather than an old fixed
        // provider layout plus unused headroom.
        assert!(
            plan.link_plan()
                .modules()
                .iter()
                .all(|module| module.runtime_plan().tls.is_none())
        );
        assert_eq!(limits.startup.static_tls.reserved_static_bytes, 0);
        assert_eq!(
            limits.startup.static_tls.thread_control,
            crate::process::tls::ThreadControlBlockPolicy::None
        );
        let mapped_bytes = plan
            .link_plan()
            .modules()
            .iter()
            .flat_map(|module| &module.runtime_plan().segments)
            .map(|segment| {
                ((segment.vaddr + segment.mem_size).div_ceil(PAGE_SIZE) * PAGE_SIZE)
                    - (segment.vaddr / PAGE_SIZE * PAGE_SIZE)
            })
            .sum();
        let mut session = prepare_native_kernel_startup_in(
            &plan,
            &StaticTlsStorage {
                thread_pointers: &[thread_pointer],
            },
            &NativeLoadLimits {
                session: SessionLoadLimits {
                    resident_limit: mapped_bytes,
                    mapped_limit: mapped_bytes,
                },
                reserved_limit: 0x10_0000,
            },
            ProcessIdentity {
                process_id: 41,
                initial_thread_id: 73,
            },
            vec![],
            owner,
        )
        .unwrap();
        assert_eq!(session.memory().mapped_bytes(), mapped_bytes);
        assert_eq!(session.memory().committed_bytes(), mapped_bytes);
        test(&mut session, &unavailable, unavailable_bias);
        assert_eq!(bytes, original_input(1));
    }
    fn request(
        session: &PreparedNativeKernelStartup<'_, '_>,
        provider: &GeneratedUnavailableProvider,
        bias: u64,
        slot: usize,
    ) -> NativeServiceRequest {
        let site = provider.site(slot).unwrap();
        NativeServiceRequest {
            stack_pointer: None,
            vector_args: None,
            owner_id: session.owner_id(),
            syscall_pc: bias + site.rva + 5,
            next_pc: bias + site.rva + 7,
            service_id: u64::from(site.service_id),
            args: [0; 6],
        }
    }
    #[test]
    fn unavailable_functions_and_exit_callback_stop_without_implementation_or_outputs() {
        prepared(|session, provider, bias| {
            let call = request(session, provider, bias, 1);
            let mapped = session.memory().mapped_bytes();
            assert!(matches!(
                session.service_request(call),
                Err(KernelStartupError::UnavailableImport {
                    ordinal: 0,
                    public_name: None,
                    reference_name: None,
                    reference_match: "unlisted-reference",
                    reference_library: None,
                })
            ));
            let callback = request(session, provider, bias, 0);
            assert!(matches!(
                session.service_request(callback),
                Err(KernelStartupError::UnavailableExitCallback)
            ));
            assert_eq!(session.native_service_requests(), 2);
            assert_eq!(session.kernel_stop_attempts(), 0);
            assert_eq!(session.libc_service_attempts(), 0);
            assert_eq!(session.memory().mapped_bytes(), mapped);
        });
    }
    #[test]
    fn unavailable_stops_require_full_id_exact_site_continuation_and_immutable_live_bytes() {
        prepared(|session, provider, bias| {
            let call = request(session, provider, bias, 1);
            assert!(matches!(
                session.service_request(NativeServiceRequest {
                    service_id: call.service_id | (1 << 32),
                    ..call
                }),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            assert!(matches!(
                session.service_request(NativeServiceRequest {
                    next_pc: call.next_pc + 1,
                    ..call
                }),
                Err(KernelStartupError::ContinuationMismatch { .. })
            ));
            for permissions in [
                NativePermissions::READ_WRITE,
                NativePermissions::READ_EXECUTE,
            ] {
                session
                    .pause()
                    .unwrap()
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: bias,
                                size: provider.mapped_bytes(),
                                permissions,
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
                if permissions.writable() {
                    assert!(matches!(
                        session.service_request(call),
                        Err(KernelStartupError::InvalidProvider(_))
                    ));
                    session
                        .pause()
                        .unwrap()
                        .write(call.syscall_pc - 5, &[0x90])
                        .unwrap();
                } else {
                    assert!(matches!(
                        session.service_request(call),
                        Err(KernelStartupError::InvalidProvider(_))
                    ));
                }
            }
            assert_eq!(session.kernel_stop_attempts(), 0);
        });
    }

    #[test]
    fn late_binding_policy_requires_actual_owner_factory_and_immutable_callable_sites() {
        prepared(|session, provider, bias| {
            let proof = session.prepare_native_source(&Default::default()).unwrap();
            let grant = session.dynamic_fallback_policy(&proof).unwrap().unwrap();
            assert_eq!(
                grant.source_identity(),
                proof.materialized_source().source_plan().identity()
            );
            assert_eq!(grant.symbols().len(), 1);
            assert_eq!(grant.symbols()[0].symbol, 1); // Exit callback is excluded.
            let source = proof.materialized_source().source_plan();
            let module = &source.modules()[grant.provider().index()];
            assert!(std::ptr::eq(module.source_bytes(), provider.image()));
            prepared(|other, _, _| {
                assert!(other.dynamic_fallback_policy(&proof).is_err());
            });
            let site = provider.site(1).unwrap();
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: bias,
                            size: provider.mapped_bytes(),
                            permissions: NativePermissions::READ_WRITE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            assert!(session.dynamic_fallback_policy(&proof).is_err());
            session
                .pause()
                .unwrap()
                .write(bias + site.rva, &[0x90])
                .unwrap();
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: bias,
                            size: provider.mapped_bytes(),
                            permissions: NativePermissions::READ_EXECUTE,
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            assert!(session.dynamic_fallback_policy(&proof).is_err());
            assert_eq!(session.memory().active_source_identity(), source.identity());
            assert_eq!(session.kernel_stop_attempts(), 0);
        });
    }
}
