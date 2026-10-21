// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::process::hle::{
    NativeImageLimits, unavailable::tests::original_input, with_native_image_session,
};

fn call(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    internal: bool,
    version: u64,
) -> Result<u64, KernelStartupError> {
    let binding = session.plan.graphics.unwrap();
    let marker = binding
        .factory
        .markers()
        .find(|m| {
            m.function
                == if internal {
                    driver::Function::AgcGetRegisterDefaultsInternal
                } else {
                    driver::Function::AgcGetRegisterDefaults
                }
        })
        .unwrap();
    session.service_request(NativeServiceRequest {
        owner_id: session.owner_id(),
        syscall_pc: binding.bias + marker.rva() + 5,
        next_pc: binding.bias + marker.rva() + 7,
        service_id: u64::from(marker.id()),
        args: [version, 0, 0, 0, 0, 0],
        vector_args: None,
        stack_pointer: None,
    })
}
fn word(session: &mut PreparedNativeKernelStartup<'_, '_>, at: u64) -> u64 {
    let mut b = [0; 8];
    session.pause().unwrap().read(at, &mut b).unwrap();
    u64::from_le_bytes(b)
}
fn record(session: &mut PreparedNativeKernelStartup<'_, '_>, at: u64) -> [u8; 64] {
    let mut b = [0; 64];
    session.pause().unwrap().read(at, &mut b).unwrap();
    b
}
fn ptr(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
fn count(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

// Canonical source oracle: counts, complete register pairs, u16 pointer
// offsets and complete type triplets, independent of packed blob placement.
const SOURCE_HASHES: [[u64; 14]; 2] = [
    [
        0x02322b55fcf32d49,
        0x02322b55fcf32d49,
        0x02322b55fcf32d49,
        0x02322b55fcf32d49,
        0xbba73471dfc452fb,
        0xf274583a18b8b7ea,
        0xf274583a18b8b7ea,
        0xc6274f5a51cc988f,
        0xeaed14eaf01d12ff,
        0xe0597f5ba3ac1637,
        0x078932eb257f8e6d,
        0xd65d510bd94ccebe,
        0x6f31677db5d18540,
        0xd65d510bd94ccebe,
    ],
    [
        0x9b87de4f5b86a803,
        0x9b87de4f5b86a803,
        0x9b87de4f5b86a803,
        0x9b87de4f5b86a803,
        0xdaa53ae1ee1a1cb5,
        0x2628b8691fcbfbc9,
        0x2628b8691fcbfbc9,
        0x7cc20ff0ee628569,
        0x7cc20ff0ee628569,
        0x282e46ba552b7dbb,
        0x4ebd40ccabff8845,
        0xc3615f038977fc54,
        0x4ebd40ccabff8845,
        0xbb11d7d4890ae0de,
    ],
];
const POINTER_COUNTS: [[[u32; 4]; 14]; 2] = [
    [
        [80, 28, 42, 0],
        [80, 28, 42, 0],
        [80, 28, 42, 0],
        [80, 28, 42, 0],
        [80, 28, 42, 0],
        [80, 29, 47, 0],
        [80, 29, 47, 0],
        [78, 29, 20, 0],
        [78, 29, 20, 0],
        [78, 29, 20, 0],
        [79, 29, 20, 0],
        [84, 32, 21, 0],
        [79, 29, 20, 0],
        [84, 32, 21, 0],
    ],
    [
        [1, 12, 0, 2],
        [1, 12, 0, 2],
        [1, 12, 0, 2],
        [1, 12, 0, 2],
        [2, 15, 0, 2],
        [3, 15, 0, 3],
        [3, 15, 0, 3],
        [4, 15, 0, 3],
        [4, 15, 0, 3],
        [4, 14, 1, 3],
        [9, 15, 1, 3],
        [8, 12, 1, 3],
        [9, 15, 1, 3],
        [8, 12, 1, 3],
    ],
];
fn audit(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    data: &[u8],
    family: usize,
    version: usize,
) -> u64 {
    fn feed(hash: &mut u64, data: &[u8]) {
        for byte in data {
            *hash = (*hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
        }
    }
    let mut hash = 0xcbf29ce484222325;
    for (bank, pointers) in POINTER_COUNTS[family][version].iter().copied().enumerate() {
        let registers = count(data, 32 + bank * 4);
        feed(&mut hash, &registers.to_le_bytes());
        feed(&mut hash, &pointers.to_le_bytes());
        if registers == 0 {
            continue;
        }
        let table = ptr(data, bank * 8);
        let start = word(session, table);
        let mut bytes = vec![0; registers as usize * 8];
        session.pause().unwrap().read(start, &mut bytes).unwrap();
        feed(&mut hash, &bytes);
        for i in 0..pointers {
            let pointer = word(session, table + u64::from(i) * 8);
            let offset = pointer.checked_sub(start).unwrap();
            assert_eq!(offset % 8, 0);
            assert!(offset / 8 < u64::from(registers));
            feed(&mut hash, &u16::try_from(offset / 8).unwrap().to_le_bytes());
        }
    }
    let mut types = vec![0; count(data, 56) as usize * 12];
    session
        .pause()
        .unwrap()
        .read(ptr(data, 48), &mut types)
        .unwrap();
    feed(&mut hash, &types);
    hash
}

#[test]
fn register_defaults_keep_all_source_versions_aliases_patches_and_mutable_lifetimes() {
    let input = original_input(0);
    with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
        // Independent pinned compact-descriptor counts, including source
        // version aliases and register counts distinct from pointer counts.
        let expected = [
            [
                [523, 161, 43, 0, 150],
                [523, 161, 43, 0, 150],
                [523, 161, 43, 0, 150],
                [523, 161, 43, 0, 150],
                [488, 158, 43, 0, 150],
                [491, 159, 75, 0, 156],
                [491, 159, 75, 0, 156],
                [489, 159, 55, 0, 127],
                [489, 159, 55, 0, 127],
                [489, 160, 55, 0, 127],
                [482, 159, 55, 0, 128],
                [488, 162, 56, 0, 137],
                [482, 159, 55, 0, 128],
                [488, 162, 56, 0, 137],
            ],
            [
                [1, 12, 0, 4, 15],
                [1, 12, 0, 4, 15],
                [1, 12, 0, 4, 15],
                [1, 12, 0, 4, 15],
                [2, 15, 0, 4, 19],
                [3, 15, 0, 6, 21],
                [3, 15, 0, 6, 21],
                [4, 15, 0, 6, 22],
                [4, 15, 0, 6, 22],
                [4, 14, 1, 6, 22],
                [9, 15, 1, 6, 28],
                [8, 12, 1, 6, 24],
                [9, 15, 1, 6, 28],
                [8, 12, 1, 6, 24],
            ],
        ];
        let before = session.memory().mapped_bytes();
        let mut descriptors = [[0; 14]; 2];
        for family in 0..2 {
            for version in 0..14 {
                let address = call(session, family == 1, version as u64).unwrap();
                descriptors[family][version] = address;
                let data = record(session, address);
                for (i, expected_count) in expected[family][version].iter().take(4).enumerate() {
                    assert_eq!(count(&data, 32 + 4 * i), *expected_count);
                }
                assert_eq!(count(&data, 56), expected[family][version][4]);
                assert_eq!(count(&data, 60), 0);
                assert_eq!(
                    audit(session, &data, family, version),
                    SOURCE_HASHES[family][version]
                );
                session
                    .pause()
                    .unwrap()
                    .preflight_non_executable(address, 64)
                    .unwrap();
                session
                    .pause()
                    .unwrap()
                    .preflight_read(ptr(&data, 48), u64::from(count(&data, 56)) * 12)
                    .unwrap();
                for bank in 0..4 {
                    if count(&data, 32 + bank * 4) == 0 {
                        assert_eq!(ptr(&data, bank * 8), 0);
                    } else {
                        let first = word(session, ptr(&data, bank * 8));
                        session.pause().unwrap().preflight_read(first, 8).unwrap();
                    }
                }
            }
        }
        let mapped = 6 * PAGE_SIZE;
        assert_eq!(
            session
                .graphics_function_profile()
                .count(driver::Function::AgcGetRegisterDefaults),
            14
        );
        assert_eq!(
            session
                .graphics_function_profile()
                .count(driver::Function::AgcGetRegisterDefaultsInternal),
            14
        );
        assert_eq!(
            session.graphics_function_profile().last(),
            "AgcGetRegisterDefaultsInternal"
        );
        assert_eq!(session.memory().mapped_bytes() - before, mapped);
        assert_eq!(session.kernel_usage().mapped_bytes, mapped);
        assert_eq!(session.kernel_usage().handles_live, 1);
        assert_eq!(call(session, false, u64::MAX).unwrap(), descriptors[0][13]);
        assert_eq!(call(session, true, u64::MAX).unwrap(), descriptors[1][13]);
        assert_eq!(
            call(session, false, 0x1_0000_0000).unwrap(),
            descriptors[0][0]
        );
        let a = record(session, descriptors[0][0]);
        let b = record(session, descriptors[0][1]);
        assert_ne!(descriptors[0][0], descriptors[0][1]);
        assert_ne!(ptr(&a, 0), ptr(&b, 0));
        assert_eq!(word(session, ptr(&a, 0)), word(session, ptr(&b, 0)));
        assert_eq!(ptr(&a, 48), ptr(&b, 48));
        let shared = word(session, ptr(&a, 0));
        let original = word(session, shared);
        session
            .pause()
            .unwrap()
            .write(shared + 4, &37_u32.to_le_bytes())
            .unwrap();
        session
            .pause()
            .unwrap()
            .write(descriptors[0][0] + 56, &99_u32.to_le_bytes())
            .unwrap();
        assert_eq!(call(session, false, 0).unwrap(), descriptors[0][0]);
        assert_eq!(count(&record(session, descriptors[0][0]), 56), 99);
        let shared_from_other_version = word(session, ptr(&b, 0));
        assert_eq!(word(session, shared_from_other_version) >> 32, 37);
        for version in [11, 12] {
            let d = record(session, descriptors[0][version]);
            let start = word(session, ptr(&d, 0));
            let mut found = false;
            for i in 0..count(&d, 32) {
                let reg = word(session, start + u64::from(i) * 8);
                if reg as u32 == 0x201 {
                    assert_ne!(reg >> 32 & 0x10000, 0);
                    found = true;
                }
            }
            assert!(found);
        }
        let d = record(session, descriptors[1][13]);
        let start = word(session, ptr(&d, 8));
        let mut found = 0;
        for i in 0..count(&d, 36) {
            let reg = word(session, start + u64::from(i) * 8);
            match reg as u32 {
                0x81 => {
                    assert_eq!(reg >> 32, 1);
                    found += 1
                }
                0x101 => {
                    assert_eq!(reg >> 32, 3);
                    found += 1
                }
                _ => {}
            }
        }
        assert_eq!(found, 2);
        assert_eq!(
            session.graphics_function_profile().top(),
            [
                Some(("AgcGetRegisterDefaults", 17)),
                Some(("AgcGetRegisterDefaultsInternal", 15)),
                None,
                None,
            ]
        );
        assert_eq!(
            session.graphics_function_profile().last(),
            "AgcGetRegisterDefaults"
        );
        assert_eq!(session.graphics_usage().calls, 32);
        with_native_image_session(&[&input], &NativeImageLimits::default(), |other, _| {
            let other_address = call(other, false, 0).unwrap();
            let other_record = record(other, other_address);
            let other_table = ptr(&other_record, 0);
            let other_reg = word(other, other_table);
            assert_eq!(word(other, other_reg), original);
            assert_ne!(other_reg, shared);
            Ok::<_, ()>(())
        })
        .unwrap();
        Ok::<_, ()>(())
    })
    .unwrap();
}

#[test]
fn register_defaults_refuse_shared_work_and_kernel_resources_without_publishing_storage() {
    let input = original_input(0);
    for graphics_limit in [true, false] {
        let mut limits = NativeImageLimits::default();
        if graphics_limit {
            limits.graphics.max_preparation_work = 0;
        } else {
            limits.startup.kernel.max_mapped_bytes = 0;
        }
        with_native_image_session(&[&input], &limits, |session, _| {
            let before = session.memory().mapped_bytes();
            assert!(call(session, false, 0).is_err());
            assert_eq!(session.memory().mapped_bytes(), before);
            assert_eq!(session.kernel_usage().handles_live, 0);
            assert_eq!(session.kernel_usage().mapped_bytes, 0);
            assert_eq!(
                session
                    .graphics_function_profile()
                    .count(driver::Function::AgcGetRegisterDefaults),
                1
            );
            assert_eq!(
                session.graphics_function_profile().last(),
                "AgcGetRegisterDefaults"
            );
            assert_eq!(session.graphics_usage().calls, 1);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}
