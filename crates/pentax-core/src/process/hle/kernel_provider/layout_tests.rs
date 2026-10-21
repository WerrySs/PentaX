// SPDX-License-Identifier: GPL-2.0-only
//! Original layout, complete sparse materialization and finite capacity proofs.

use super::*;
use crate::memory::MemoryError;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use crate::process::{SessionLoadLimits, load_symbolic};
use pentax_linker::{ModuleInput, SymbolType, inspect_sce, plan_symbolic};
use pentax_loader::open_executable;

fn original_consumer() -> Vec<u8> {
    let row = Service::registrations()
        .iter()
        .find(|row| row.module == "libkernel" && row.library == "libkernel")
        .unwrap();
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = b"libkernel";
    original.library_name = b"libkernel";
    original.imports = vec![Import {
        nid: row.nid.as_bytes().try_into().unwrap(),
        symbol_type: 2,
        binding: 1,
        relocation: 7,
        addend: 0,
    }];
    original.build().unwrap()
}

#[test]
fn metadata_layout_admits_declared_count_and_worst_names_but_refuses_every_bound() {
    // Independent worst case: 138 distinct 255-byte namespaces (68 modules
    // and 70 libraries), 2048 functions plus two objects, each with an
    // 11-byte NID and two three-digit selectors.
    let worst_strings = 1 + 138 * 256 + 2050 * 20;
    assert_eq!(MAX_REGISTRATIONS, 2048);
    assert_eq!(
        (MAX_IMAGE_BYTES, MAX_METADATA_BYTES),
        (147_728, 8 * PAGE_SIZE)
    );
    assert_eq!(MAX_STRINGS_BYTES, worst_strings);
    let layout = Layout::new(2048, worst_strings, MAX_IMAGE_BYTES).unwrap();
    assert_eq!(
        (
            layout.image_size,
            MAX_IMAGE_BYTES - layout.image_size as u64
        ),
        (144_816, 2912)
    );
    assert_eq!(layout.code_bytes, 0x4100);
    assert_eq!(layout.data_offset, layout.code_bytes);
    assert_eq!(layout.metadata_offset, layout.data_offset + 16);
    assert_eq!(layout.dynamic_at - layout.metadata_offset, 4 * 56);
    assert_eq!(layout.strings_at - layout.dynamic_at, DYNAMIC_ENTRIES * 16);
    assert_eq!(layout.rela_at - layout.symbols_at, 2051 * 24);
    assert_eq!(layout.image_size - layout.rela_at, 24);
    assert!(layout.metadata_mapped_bytes <= 8 * PAGE_SIZE);
    assert!(METADATA_RVA + layout.metadata_mapped_bytes <= MAX_MAPPED_BYTES);
    assert!(layout.image_size as u64 <= MAX_IMAGE_BYTES);
    assert_eq!(
        Layout::new(2048, worst_strings, layout.image_size as u64)
            .unwrap()
            .image_size,
        layout.image_size
    );
    for (count, strings, maximum) in [
        (0, worst_strings, MAX_IMAGE_BYTES),
        (2049, worst_strings, MAX_IMAGE_BYTES),
        (usize::MAX, worst_strings, u64::MAX),
        (2048, 0, MAX_IMAGE_BYTES),
        (2048, worst_strings + 1, MAX_IMAGE_BYTES),
        (2048, usize::MAX, u64::MAX),
        (2048, worst_strings, layout.image_size as u64 - 1),
    ] {
        assert!(matches!(
            Layout::new(count, strings, maximum),
            Err(KernelProviderError::Budget)
        ));
    }
    // The executable source never grows across the established object RVAs.
    assert_eq!(
        (
            CODE_MAPPED_BYTES,
            STACK_GUARD_RVA,
            PROGRAM_NAME_OBJECT_RVA,
            PROGRAM_NAME_BUFFER_RVA,
            METADATA_RVA
        ),
        (0x10000, 0x10000, 0x10008, 0x10010, 0x14000)
    );
}

#[test]
fn metadata_segment_round_trips_and_materializes_with_distinct_permissions_and_exact_quota() {
    let factory = build_kernel_provider(MAX_IMAGE_BYTES).unwrap();
    let view = open_executable(factory.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(runtime.segments.len(), 3);
    let expected = [
        (0, CODE_MAPPED_BYTES, 5, factory.code_bytes()),
        (STACK_GUARD_RVA, PAGE_SIZE, 6, factory.data_bytes()),
        (
            METADATA_RVA,
            factory.metadata_mapped_bytes(),
            4,
            factory.metadata_bytes(),
        ),
    ];
    for (segment, (rva, size, flags, bytes)) in runtime.segments.iter().zip(expected) {
        assert_eq!(
            (segment.vaddr, segment.mem_size, segment.flags.raw()),
            (rva, size, flags)
        );
        assert!(std::ptr::eq(segment.file_bytes, bytes));
    }
    assert_eq!(
        runtime
            .segments
            .iter()
            .map(|segment| segment.file_bytes.len())
            .sum::<usize>(),
        factory.image().len()
    );
    assert_eq!(
        factory.code_bytes().len(),
        0x100 + 8 * Service::registrations().len()
    );
    assert_eq!(factory.data_bytes().len(), 16);
    let phoff = u64::from_le_bytes(factory.image()[32..40].try_into().unwrap()) as usize;
    assert_eq!(phoff, factory.metadata_offset);
    assert_eq!(&factory.code_bytes()[64..0x100], &[0; 0xc0]);
    let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
    let metadata = inspection.metadata().unwrap();
    let symbols = metadata.link().symbols();
    assert_eq!(symbols.len(), Service::registrations().len() + 3);
    for (index, marker) in factory.markers().iter().enumerate() {
        assert_eq!(marker.registration, &Service::registrations()[index]);
        assert_eq!(
            (marker.service_id, marker.rva),
            (0x5058_4b01 + index as u32, 0x100 + index as u64 * 8)
        );
        assert_eq!(
            &factory.code_bytes()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
        assert_eq!(
            (
                symbols[marker.symbol].symbol_type(),
                symbols[marker.symbol].value
            ),
            (SymbolType::FUNC, marker.rva)
        );
    }
    let original = original_consumer();
    let plan = plan_symbolic(
        &[
            ModuleInput::new(open_executable(&original).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(view, PROVIDER_BIAS),
        ],
        &Default::default(),
    )
    .unwrap();
    let total: u64 = plan
        .modules()
        .iter()
        .flat_map(|module| &module.runtime_plan().segments)
        .map(|segment| {
            ((segment.vaddr + segment.mem_size + PAGE_SIZE - 1) & !(PAGE_SIZE - 1))
                - (segment.vaddr & !(PAGE_SIZE - 1))
        })
        .sum();
    let limits = SessionLoadLimits {
        mapped_limit: total,
        resident_limit: total,
    };
    let session = load_symbolic(&plan, &limits).unwrap();
    assert_eq!(session.memory().mapped_bytes(), total);
    for offset in (0..factory.metadata_mapped_bytes()).step_by(PAGE_SIZE as usize) {
        let address = PROVIDER_BIAS + METADATA_RVA + offset;
        session.memory().read(address, &mut [0]).unwrap();
        assert!(matches!(
            session.memory().fetch(address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
        assert!(matches!(
            session.memory().preflight_write(address, 1),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
    let mut bytes = vec![0; factory.metadata_bytes().len()];
    session
        .memory()
        .read(PROVIDER_BIAS + METADATA_RVA, &mut bytes)
        .unwrap();
    assert_eq!(bytes, factory.metadata_bytes());
    for (address, expected) in [
        (PROVIDER_BIAS + STACK_GUARD_RVA, STACK_GUARD_INITIAL),
        (
            PROVIDER_BIAS + PROGRAM_NAME_OBJECT_RVA,
            PROVIDER_BIAS + PROGRAM_NAME_BUFFER_RVA,
        ),
    ] {
        let mut actual = [0; 8];
        session.memory().read(address, &mut actual).unwrap();
        assert_eq!(u64::from_le_bytes(actual), expected);
        session.memory().preflight_write(address, 8).unwrap();
        assert!(matches!(
            session.memory().fetch(address, &mut [0]),
            Err(MemoryError::PermissionDenied(_))
        ));
    }
    assert!(
        load_symbolic(
            &plan,
            &SessionLoadLimits {
                mapped_limit: total - 1,
                ..limits
            }
        )
        .is_err()
    );
    assert!(matches!(
        build_kernel_provider(factory.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn metadata_factory_binding_refuses_missing_or_changed_segment_domains_and_permissions() {
    use crate::process::hle::{
        HleProfile, KernelStartupError, KernelStartupLimits, ProviderPlacement,
        build_hle_providers, plan_kernel_startup,
    };
    let limits = KernelStartupLimits::default();
    let libc =
        build_hle_providers(HleProfile::KytyObservedLibcInternalV1, &limits.startup.hle).unwrap();
    let original = original_consumer();
    for case in 0..6 {
        let mut factory = build_kernel_provider(MAX_IMAGE_BYTES).unwrap();
        let table = factory.metadata_offset;
        match case {
            0 => put(&mut factory.image, table + 2 * 56 + 4, &6_u32.to_le_bytes()),
            1 => put(&mut factory.image, table + 2 * 56 + 4, &5_u32.to_le_bytes()),
            2 => {
                let size = factory.metadata_mapped_bytes() + PAGE_SIZE;
                put(&mut factory.image, table + 2 * 56 + 40, &size.to_le_bytes());
            }
            3 => put(&mut factory.image, 56, &3_u16.to_le_bytes()),
            4 => put(&mut factory.image, 32, &64_u64.to_le_bytes()),
            _ => put(
                &mut factory.image,
                table + 2 * 56 + 16,
                &(METADATA_RVA + PAGE_SIZE).to_le_bytes(),
            ),
        }
        let result = plan_kernel_startup(
            &[ModuleInput::new(
                open_executable(&original).unwrap(),
                CONSUMER_BIAS,
            )],
            &factory,
            ProviderPlacement {
                load_bias: PROVIDER_BIAS,
            },
            &libc,
            ProviderPlacement {
                load_bias: PROVIDER_BIAS + 0x40000,
            },
            &limits,
        );
        if case < 3 {
            assert!(matches!(
                result,
                Err(KernelStartupError::InvalidProvider(
                    "kernel factory metadata identity"
                ))
            ));
        } else {
            assert!(result.is_err());
        }
    }
}
