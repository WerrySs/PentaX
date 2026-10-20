// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::fixture::{CODE, CONSUMER_BIAS, Consumer, Import, PROVIDER_BIAS};
use pentax_linker::{ModuleInput, Resolution, SymbolicError, UnsupportedSymbol, plan_symbolic};
use pentax_loader::open_executable;

fn consumer(module: &'static [u8], library: &'static [u8], rows: &[Registration]) -> Consumer {
    let mut original = Consumer::importing(vec![(CODE, vec![0xcc])]);
    original.module_name = module;
    original.library_name = library;
    original.imports = rows
        .iter()
        .map(|row| Import {
            nid: row.nid.as_bytes().try_into().unwrap(),
            symbol_type: 2,
            binding: 1,
            relocation: 7,
            addend: 0,
        })
        .collect();
    original
}

fn link<'a>(
    bytes: &'a [u8],
    provider: &'a GeneratedKernelProvider,
) -> Result<pentax_linker::SymbolicLinkPlan<'a>, SymbolicError> {
    plan_symbolic(
        &[
            ModuleInput::new(open_executable(bytes).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(open_executable(provider.image()).unwrap(), PROVIDER_BIAS),
        ],
        &Default::default(),
    )
}

#[test]
fn entitlement_access_exact_bindings_preserve_prior_sites_and_fixed_data_pages() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    assert_eq!(provider.registration_count(), 1281);
    assert_eq!((MODULES.len(), LIBRARIES.len()), (64, 65));
    assert_eq!(
        &MODULES[45..47],
        &[
            ("NpEntitlementAccess", (1, 1)),
            ("libSceNpEntitlementAccess", (1, 1))
        ]
    );
    assert_eq!(
        &LIBRARIES[46..48],
        &[("NpEntitlementAccess", 1), ("libSceNpEntitlementAccess", 1)]
    );
    for (index, marker) in provider.markers().iter().enumerate() {
        assert_eq!(
            (marker.symbol, marker.service_id, marker.rva),
            (
                index + 1,
                0x5058_4b01 + index as u32,
                0x100 + index as u64 * 8
            )
        );
        assert_eq!(
            &provider.image()[marker.rva as usize..marker.rva as usize + 8],
            &marker.bytes()
        );
    }
    assert_eq!(
        (
            provider.markers()[1049].service_id,
            provider.markers()[1049].rva
        ),
        (0x5058_4f1a, 0x21c8)
    );
    assert_eq!(
        (
            provider.markers()[1050].service_id,
            provider.markers()[1050].rva
        ),
        (0x5058_4f1b, 0x21d0)
    );
    assert_eq!(
        (
            provider.markers()[1056].service_id,
            provider.markers()[1056].rva
        ),
        (0x5058_4f21, 0x2200)
    );
    assert_eq!(
        (provider.code_bytes().len(), provider.image().len()),
        (0xfdb8, 0xfdc8)
    );
    assert_eq!(
        (provider.objects()[0].symbol, provider.objects()[0].rva),
        (1282, 0x10000)
    );
    assert_eq!(
        (provider.objects()[1].symbol, provider.objects()[1].rva),
        (1283, 0x10008)
    );
    assert_eq!(PROGRAM_NAME_BUFFER_RVA, 0x10010);
    assert_eq!(
        &provider.data_bytes()[..8],
        &STACK_GUARD_INITIAL.to_le_bytes()
    );
    assert_eq!(&provider.data_bytes()[8..], &[0; 8]);
    let view = open_executable(provider.image()).unwrap();
    let runtime = view.runtime_plan().unwrap();
    assert_eq!(
        (
            runtime.segments[0].flags.raw(),
            runtime.segments[0].mem_size
        ),
        (5, 4 * PAGE_SIZE)
    );
    assert_eq!(
        (runtime.segments[1].flags.raw(), runtime.segments[1].vaddr),
        (6, 4 * PAGE_SIZE)
    );
    for (scope, start, end) in [
        (b"NpEntitlementAccess".as_slice(), 1050, 1055),
        (b"libSceNpEntitlementAccess".as_slice(), 1055, 1057),
    ] {
        let rows = &Service::registrations()[start..end];
        let bytes = consumer(scope, scope, rows).build().unwrap();
        let plan = link(&bytes, &provider).unwrap();
        assert_eq!(plan.patch_count(), rows.len() + 1);
        assert_eq!(
            plan.patches()
                .iter()
                .filter(|patch| patch.consumer().index() == 0)
                .count(),
            rows.len()
        );
        for (offset, patch) in plan
            .patches()
            .iter()
            .filter(|patch| patch.consumer().index() == 0)
            .enumerate()
        {
            let index = start + offset;
            assert_eq!(patch.value(), PROVIDER_BIAS + 0x100 + index as u64 * 8);
            assert!(
                matches!(patch.resolution(), Resolution::Definition(reference)
                if reference.module.index() == 1 && reference.symbol == index + 1)
            );
            assert_eq!(provider.markers()[index].registration, &rows[offset]);
        }
    }
    assert!(matches!(
        build_kernel_provider(provider.image().len() as u64 - 1),
        Err(KernelProviderError::Budget)
    ));
}

#[test]
fn entitlement_access_bindings_refuse_unobserved_roles_mixed_scopes_versions_and_object_type() {
    let provider = build_kernel_provider(CODE_MAPPED_BYTES).unwrap();
    let initialize = &Service::registrations()[1050..1051];
    for (module, library) in [
        (
            b"NpEntitlementAccess".as_slice(),
            b"libSceNpEntitlementAccess".as_slice(),
        ),
        (
            b"libSceNpEntitlementAccess".as_slice(),
            b"NpEntitlementAccess".as_slice(),
        ),
        (
            b"EntitlementAccess".as_slice(),
            b"EntitlementAccess".as_slice(),
        ),
    ] {
        assert!(matches!(
            link(
                &consumer(module, library, initialize).build().unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for index in [1051, 1053, 1054] {
        let rows = &Service::registrations()[index..index + 1];
        assert!(matches!(
            link(
                &consumer(
                    b"libSceNpEntitlementAccess",
                    b"libSceNpEntitlementAccess",
                    rows
                )
                .build()
                .unwrap(),
                &provider
            ),
            Err(SymbolicError::MissingProvider { .. })
        ));
    }
    for scope in [
        b"NpEntitlementAccess".as_slice(),
        b"libSceNpEntitlementAccess".as_slice(),
    ] {
        for (major, minor, version) in [(2, 1, 1), (1, 2, 1), (1, 1, 2)] {
            let mut original = consumer(scope, scope, initialize);
            (original.major, original.minor, original.version) = (major, minor, version);
            assert!(matches!(
                link(&original.build().unwrap(), &provider),
                Err(SymbolicError::VersionMismatch { .. })
            ));
        }
        let mut original = consumer(scope, scope, initialize);
        original.imports[0].symbol_type = 1;
        original.imports[0].relocation = 6;
        assert!(matches!(
            link(&original.build().unwrap(), &provider),
            Err(SymbolicError::UnsupportedSymbol {
                reason: UnsupportedSymbol::TypeMismatch,
                ..
            })
        ));
    }
}
