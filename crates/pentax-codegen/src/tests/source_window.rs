// SPDX-License-Identifier: GPL-2.0-only

//! Original relocation tables exercise lookup cost and exact window boundaries.

use super::*;
use crate::source::{materialized_window, source_window};

const VALUE: u64 = 0x5152_5354_5556_5758;

fn put(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn many_patches(count: usize, kind: u32) -> Vec<u8> {
    let mut bytes = with_relocation(kind, ENTRY as u64 + 2, 0);
    let table = DATA as usize + count * 8 + 0x100;
    let size = table + count * 24;
    bytes.resize(size, 0);
    put(&mut bytes, 64 + 56 + 32, (size as u64) - DATA);
    put(&mut bytes, 64 + 56 + 40, (size as u64) - DATA);
    put(&mut bytes, 0x808, table as u64);
    put(&mut bytes, 0x818, (count * 24) as u64);
    for index in 0..count {
        let target = if index == 0 {
            ENTRY as u64 + 2
        } else {
            DATA + (index * 8) as u64
        };
        put(&mut bytes, table + index * 24, target);
        put(&mut bytes, table + index * 24 + 8, u64::from(kind));
        put(&mut bytes, table + index * 24 + 16, VALUE - BIAS);
    }
    bytes
}

fn account(size: usize) -> Accounting {
    Accounting {
        work: 0,
        allocated: 0,
        limits: BlockLimits {
            max_source_bytes: size as u64,
            max_work_units: 128,
            ..Default::default()
        },
    }
}

#[test]
fn large_ordinary_and_declared_tables_only_visit_intersecting_writes() {
    for kind in [8, 18] {
        let bytes = many_patches(4096, kind);
        let source = link(&[(&bytes, BIAS)]);
        let writes = source
            .deferred()
            .iter()
            .enumerate()
            .map(|(deferred_index, record)| DeclaredAppliedWrite {
                deferred_index,
                consumer: record.module(),
                source: record.source(),
                address: BIAS + record.source().record.offset,
                value: VALUE,
            })
            .collect::<Vec<_>>();
        let view = declare_materialized_source(&source, &writes, &BlockLimits::default()).unwrap();
        let mut expected = vec![0x48, 0xb8];
        expected.extend_from_slice(&VALUE.to_le_bytes());
        expected.push(0xc3);

        // Includes, excludes, straddles, and truncates both patch boundaries.
        for (offset, size) in [(0, 11), (0, 2), (1, 3), (2, 8), (9, 1), (9, 2), (10, 1)] {
            let mut usage = account(size);
            let staged = &expected[offset..offset + size];
            let pc = BIAS + ENTRY as u64 + offset as u64;
            let window = if kind == 8 {
                source_window(&source, 0, pc, placement(), &mut usage).unwrap()
            } else {
                materialized_window(&view, 0, pc, placement(), staged, &mut usage).unwrap()
            };
            assert_eq!(window.relocated, staged);
            assert_eq!(
                window.original,
                bytes[ENTRY + offset..ENTRY + offset + size]
            );
            assert!(usage.work <= 128);
            let mut exact = account(size);
            exact.limits.max_work_units = usage.work - 1;
            let result = if kind == 8 {
                source_window(&source, 0, pc, placement(), &mut exact)
            } else {
                materialized_window(&view, 0, pc, placement(), staged, &mut exact)
            };
            assert!(matches!(
                result,
                Err(CodegenError::Budget {
                    kind: Budget::Work,
                    ..
                })
            ));
        }

        let mut limited = account(expected.len());
        limited.limits.max_patches = 4095;
        assert!(matches!(
            materialized_window(
                &view,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &expected,
                &mut limited
            ),
            Err(CodegenError::Budget {
                kind: Budget::Patches,
                ..
            })
        ));
        expected[9] ^= 1;
        assert!(matches!(
            materialized_window(
                &view,
                0,
                BIAS + ENTRY as u64,
                placement(),
                &expected,
                &mut account(expected.len())
            ),
            Err(CodegenError::MaterializedWindowMismatch { offset: 9, .. })
        ));
    }
}

#[test]
fn appended_source_finds_an_earlier_module_patch_and_preserves_last_patch() {
    use pentax_linker::{ModuleInput, SymbolicExtensionLimits, append_symbolic_module};
    use pentax_loader::open_executable;

    let original = with_relocation(8, ENTRY as u64 + 2, 0x11);
    let added = with_relocation(8, ENTRY as u64 + 2, 0x22);
    let base = link(&[(&original, BIAS)]);
    let lower_bias = BIAS - PAGE * 4;
    let extended = append_symbolic_module(
        &base,
        ModuleInput::new(open_executable(&added).unwrap(), lower_bias),
        &SymbolicExtensionLimits::default(),
    )
    .unwrap();
    assert_eq!(extended.parent_identity(), Some(base.identity()));
    for (module, bias, value) in [(0, BIAS, BIAS + 0x11), (1, lower_bias, lower_bias + 0x22)] {
        let window = source_window(
            &extended,
            module,
            bias + ENTRY as u64 + 9,
            placement(),
            &mut account(2),
        )
        .unwrap();
        assert_eq!(window.module, extended.modules()[module].id());
        assert_eq!(window.relocated, [value.to_le_bytes()[7], 0xc3]);
    }
}
