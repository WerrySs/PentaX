// SPDX-License-Identifier: GPL-2.0-only
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

//! Original bytes and a source-ordered oracle over actual owned native pages.
use super::*;
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

fn owner() -> (NativeAddressSpace, u64, ScalarDefaults) {
    let mut memory = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = memory.reserved_ranges().next().unwrap().address;
    memory
        .map_at(base, 0x20000, NativePermissions::READ_WRITE)
        .unwrap();
    memory
        .protect(base + 0x10000, 0x10000, NativePermissions::READ)
        .unwrap();
    (
        memory,
        base,
        ScalarDefaults {
            boolean: base + 0x10000,
            integer: base + 0x10008,
            real: base + 0x10010,
        },
    )
}

fn call(
    memory: &mut NativeAddressSpace,
    function: JsonFunction,
    object: u64,
    second: u64,
    defaults: ScalarDefaults,
) -> Result<u64, KernelStartupError> {
    service(
        &mut memory.pause().unwrap(),
        function,
        [object, second, u64::MAX, u64::MAX, u64::MAX, u64::MAX],
        None,
        defaults,
        &mut 0,
        4096,
    )
}

fn call_vectors(
    memory: &mut NativeAddressSpace,
    function: JsonFunction,
    object: u64,
    vectors: &[[u8; 16]; 8],
    defaults: ScalarDefaults,
) -> Result<u64, KernelStartupError> {
    service(
        &mut memory.pause().unwrap(),
        function,
        [
            object,
            0xdead_beef_cafe_f00d,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
        ],
        Some(vectors),
        defaults,
        &mut 0,
        4096,
    )
}

fn read<const N: usize>(memory: &mut NativeAddressSpace, at: u64) -> [u8; N] {
    let mut bytes = [0; N];
    memory.pause().unwrap().read(at, &mut bytes).unwrap();
    bytes
}

fn value(kind: u32) -> [u8; 32] {
    let mut bytes = [0x6d; 32];
    bytes[28..].copy_from_slice(&kind.to_le_bytes());
    bytes
}

#[test]
fn constructors_zero_all_bytes_and_setters_preserve_only_padding_with_exact_widths() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x101;
    for (function, argument, kind, payload, constructor) in [
        (
            ValueBoolCtor,
            0xfedc_ba98_7654_32a7,
            1_u32,
            [0xa7, 0, 0, 0, 0, 0, 0, 0],
            true,
        ),
        (ValueIntCtor, u64::MAX, 2, [0xff; 8], true),
        (ValueTypeCtor, 0x1234_5678_ffff_ffff, u32::MAX, [0; 8], true),
        (
            ValueSetBool,
            0xabcd_ef00_0000_00a7,
            1,
            [0xa7, 0, 0, 0, 0, 0, 0, 0],
            false,
        ),
        (ValueSetInt, u64::MAX, 2, [0xff; 8], false),
        (
            ValueSetUInt,
            0x8877_6655_4433_2211,
            3,
            [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
            false,
        ),
        (
            ValueSetType,
            0x9876_5432_ffff_fffe,
            u32::MAX - 1,
            [0; 8],
            false,
        ),
    ] {
        memory
            .pause()
            .unwrap()
            .write(object - 1, &[0x9b; 34])
            .unwrap();
        memory.pause().unwrap().write(object, &value(4)).unwrap();
        assert_eq!(
            call(&mut memory, function, object, argument, defaults),
            Ok(if constructor { object } else { 0 })
        );
        let mut expected = [0; 32];
        if !constructor {
            expected[24..28].fill(0x6d);
        }
        expected[16..24].copy_from_slice(&payload);
        expected[28..].copy_from_slice(&kind.to_le_bytes());
        assert_eq!(read::<32>(&mut memory, object), expected);
        assert_eq!(read::<1>(&mut memory, object - 1), [0x9b]);
        assert_eq!(read::<1>(&mut memory, object + 32), [0x9b]);
    }
    for function in [
        ValueBoolCtor,
        ValueIntCtor,
        ValueTypeCtor,
        ValueCopyCtor,
        ValueAssign,
        ValueSetBool,
        ValueSetInt,
        ValueSetUInt,
        ValueSetType,
        ValueDtor,
        ValueClearMethod,
    ] {
        assert_eq!(call(&mut memory, function, 0, u64::MAX, defaults), Ok(0));
    }
}

#[test]
fn double_operations_preserve_xmm0_raw_bits_and_ignore_other_register_bytes() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x101;
    // Signed zeros, infinities, distinct NaN payloads, a subnormal and a finite
    // extreme remain byte patterns throughout; no host floating-point operation.
    let patterns = [
        0_u64,
        0x8000_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0xfff0_0000_0000_0000,
        0x7ff8_1234_5678_9abc,
        0x7ff0_1234_5678_9abc,
        0xfff8_fedc_ba98_7654,
        1,
        0x7fef_ffff_ffff_ffff,
    ];
    for function in [ValueDoubleCtor, ValueSetDouble] {
        let constructor = function == ValueDoubleCtor;
        for bits in patterns {
            let mut vectors = [[0xa7; 16]; 8];
            vectors[0][..8].copy_from_slice(&bits.to_le_bytes());
            memory
                .pause()
                .unwrap()
                .write(object - 1, &[0x9b; 34])
                .unwrap();
            // Construction initializes without clearing an old owned payload;
            // assignment instead follows the ordinary non-owned Clear path.
            let before = value(if constructor { 7 } else { 4 });
            memory.pause().unwrap().write(object, &before).unwrap();
            assert_eq!(
                call_vectors(&mut memory, function, object, &vectors, defaults),
                Ok(if constructor { object } else { 0 })
            );
            let mut expected = [0; 32];
            if !constructor {
                expected[24..28].copy_from_slice(&before[24..28]);
            }
            expected[16..24].copy_from_slice(&bits.to_le_bytes());
            expected[28..].copy_from_slice(&4_u32.to_le_bytes());
            assert_eq!(read::<32>(&mut memory, object), expected);
            assert_eq!(read::<1>(&mut memory, object - 1), [0x9b]);
            assert_eq!(read::<1>(&mut memory, object + 32), [0x9b]);
        }
        assert_eq!(
            call_vectors(&mut memory, function, 0, &[[0x81; 16]; 8], defaults),
            Ok(0)
        );
    }
}

#[test]
fn double_missing_capture_owned_storage_and_late_write_refusals_are_atomic() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x100;
    let before = value(4);
    memory.pause().unwrap().write(object, &before).unwrap();
    for function in [ValueDoubleCtor, ValueSetDouble] {
        // Absence is an ABI failure even for null or inaccessible destinations;
        // no destination read or write may obscure that failure.
        for destination in [0, object, u64::MAX] {
            assert_eq!(
                call(&mut memory, function, destination, 42, defaults),
                Err(KernelStartupError::UnavailableVectorArguments)
            );
            assert_eq!(read::<32>(&mut memory, object), before);
        }
    }
    let mut vectors = [[0x93; 16]; 8];
    vectors[0][..8].copy_from_slice(&0x8000_0000_0000_0000_u64.to_le_bytes());
    for kind in 5..=7 {
        let owned = value(kind);
        memory.pause().unwrap().write(object, &owned).unwrap();
        assert_eq!(
            call_vectors(&mut memory, ValueSetDouble, object, &vectors, defaults),
            Err(KernelStartupError::UnavailableJsonValueStorage)
        );
        assert_eq!(read::<32>(&mut memory, object), owned);
    }
    // The payload is writable but the trailing type occupies the next RO page.
    // Both the full initializer and Clear/setter must publish no prefix bytes.
    let boundary = base + 0x10000 - 28;
    memory
        .pause()
        .unwrap()
        .write(boundary, &[0x71; 28])
        .unwrap();
    let original = read::<32>(&mut memory, boundary);
    for function in [ValueDoubleCtor, ValueSetDouble] {
        assert_eq!(
            call_vectors(&mut memory, function, boundary, &vectors, defaults),
            Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                pentax_kernel::GuestError::WriteDenied
            )))
        );
        assert_eq!(read::<32>(&mut memory, boundary), original);
    }
    let mut work = 7;
    assert_eq!(
        service(
            &mut memory.pause().unwrap(),
            ValueDoubleCtor,
            [object, 0, 0, 0, 0, 0],
            Some(&vectors),
            defaults,
            &mut work,
            7 + MUTATION_WORK - 1,
        ),
        Err(KernelStartupError::Budget("provider work"))
    );
    assert_eq!(work, 7);
    assert_eq!(read::<32>(&mut memory, object), value(7));
}

#[test]
fn destruction_and_clear_method_differ_in_links_and_owned_storage_is_never_discarded() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x100;
    for function in [ValueDtor, ValueClearMethod] {
        let before = value(2);
        memory.pause().unwrap().write(object, &before).unwrap();
        assert_eq!(call(&mut memory, function, object, 0, defaults), Ok(0));
        let mut expected = [0; 32];
        expected[24..28].copy_from_slice(&before[24..28]);
        if function == ValueClearMethod {
            expected[..16].copy_from_slice(&before[..16]);
        }
        assert_eq!(read::<32>(&mut memory, object), expected);
    }
    let source = base + 0x200;
    memory.pause().unwrap().write(source, &value(2)).unwrap();
    for kind in 5..=7 {
        for function in [
            ValueDtor,
            ValueClearMethod,
            ValueSetBool,
            ValueSetInt,
            ValueSetUInt,
            ValueSetType,
            ValueAssign,
        ] {
            let before = value(kind);
            memory.pause().unwrap().write(object, &before).unwrap();
            assert_eq!(
                call(
                    &mut memory,
                    function,
                    object,
                    if function == ValueAssign { source } else { 0 },
                    defaults
                ),
                Err(KernelStartupError::UnavailableJsonValueStorage)
            );
            assert_eq!(read::<32>(&mut memory, object), before);
        }
        for function in [ValueTypeCtor, ValueSetType] {
            let before = value(0);
            memory.pause().unwrap().write(object, &before).unwrap();
            assert_eq!(
                call(&mut memory, function, object, u64::from(kind), defaults),
                Err(KernelStartupError::UnavailableJsonValueStorage)
            );
            assert_eq!(read::<32>(&mut memory, object), before);
        }
        memory.pause().unwrap().write(source, &value(kind)).unwrap();
        memory.pause().unwrap().write(object, &value(0)).unwrap();
        let before = read::<32>(&mut memory, object);
        assert_eq!(
            call(&mut memory, ValueCopyCtor, object, source, defaults),
            Err(KernelStartupError::UnavailableJsonValueStorage)
        );
        assert_eq!(read::<32>(&mut memory, object), before);
        memory.pause().unwrap().write(source, &value(2)).unwrap();
    }
}

#[test]
fn getters_return_field_or_distinct_readonly_default_pointers_and_count_is_not_faked() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x100;
    assert_ne!(defaults.boolean, defaults.integer);
    assert_ne!(defaults.integer, defaults.real);
    for kind in [0, 1, 2, 3, 4, 5, 6, 7, u32::MAX] {
        memory.pause().unwrap().write(object, &value(kind)).unwrap();
        assert_eq!(
            call(&mut memory, ValueGetType, object, 0, defaults),
            Ok(u64::from(kind))
        );
        for (function, matching, fallback) in [
            (ValueGetBoolean, kind == 1, defaults.boolean),
            (ValueGetInteger, matches!(kind, 2 | 3), defaults.integer),
            (ValueGetReal, kind == 4, defaults.real),
        ] {
            assert_eq!(
                call(&mut memory, function, object, 0, defaults),
                Ok(if matching { object + 16 } else { fallback })
            );
            assert_eq!(
                call(&mut memory, function, 0, u64::MAX, defaults),
                Ok(fallback)
            );
        }
        if matches!(kind, 6 | 7) {
            assert_eq!(
                call(&mut memory, ValueCount, object, 0, defaults),
                Err(KernelStartupError::UnavailableJsonValueStorage)
            );
        } else {
            assert_eq!(call(&mut memory, ValueCount, object, 0, defaults), Ok(0));
        }
    }
    assert_eq!(call(&mut memory, ValueGetType, 0, 0, defaults), Ok(0));
    assert_eq!(call(&mut memory, ValueCount, 0, 0, defaults), Ok(0));
    assert_eq!(read::<1>(&mut memory, defaults.boolean), [0]);
    assert_eq!(read::<8>(&mut memory, defaults.integer), [0; 8]);
    assert_eq!(read::<8>(&mut memory, defaults.real), [0; 8]);
    assert!(
        memory
            .pause()
            .unwrap()
            .preflight_write(defaults.integer, 8)
            .is_err()
    );
}

fn oracle(
    bytes: &mut [u8; 96],
    destination: usize,
    source: usize,
    constructor: bool,
) -> Result<(), ()> {
    let kind = |bytes: &[u8; 96], at: usize| {
        u32::from_le_bytes(bytes[at + 28..at + 32].try_into().unwrap())
    };
    if constructor {
        bytes[destination..destination + 32].fill(0);
    }
    if source == destination {
        return Ok(());
    }
    if matches!(kind(bytes, destination), 5..=7)
        && bytes[destination + 16..destination + 24] != [0; 8]
    {
        return Err(());
    }
    bytes[destination..destination + 24].fill(0);
    bytes[destination + 28..destination + 32].fill(0);
    let first = kind(bytes, source);
    bytes[destination + 28..destination + 32].copy_from_slice(&first.to_le_bytes());
    let second = kind(bytes, source);
    if matches!(second, 5..=7) {
        return Err(());
    }
    let width = match second {
        1 => 1,
        2..=4 => 8,
        _ => 0,
    };
    let mut payload = [0; 8];
    payload[..width].copy_from_slice(&bytes[source + 16..source + 16 + width]);
    if width == 0 {
        bytes[destination + 16..destination + 24].fill(0);
    } else {
        bytes[destination + 16..destination + 16 + width].copy_from_slice(&payload[..width]);
    }
    Ok(())
}

#[test]
fn copy_and_assignment_observe_ordered_virtual_overlays_in_both_directions() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    for constructor in [false, true] {
        for source in [8, 16, 23, 24, 25, 28, 32, 40, 48, 56] {
            let mut original = [0xa7; 96];
            original[24 + 28..24 + 32].copy_from_slice(&0_u32.to_le_bytes());
            original[source + 28..source + 32].copy_from_slice(&2_u32.to_le_bytes());
            // This unaligned case exercises the source's second type read:
            // first type5 becomes1285 after dst.type overlaps src.type.
            if source == 23 {
                original[24 + 27] = 5;
                original[24 + 28..24 + 32].fill(0);
            }
            let mut expected = original;
            let result = oracle(&mut expected, 24, source, constructor);
            memory
                .pause()
                .unwrap()
                .write(base + 0x100, &original)
                .unwrap();
            let actual = call(
                &mut memory,
                if constructor {
                    ValueCopyCtor
                } else {
                    ValueAssign
                },
                base + 0x100 + 24,
                base + 0x100 + source as u64,
                defaults,
            );
            if result.is_ok() {
                assert_eq!(actual, Ok(base + 0x100 + 24));
                assert_eq!(read::<96>(&mut memory, base + 0x100), expected);
            } else {
                assert_eq!(actual, Err(KernelStartupError::UnavailableJsonValueStorage));
                assert_eq!(read::<96>(&mut memory, base + 0x100), original);
            }
        }
    }
    // Self assignment is a true no-op; a same-address copy constructor zeros
    // all32bytes before the source copy notices pointer equality.
    memory.pause().unwrap().write(base, &value(7)).unwrap();
    assert_eq!(
        call(&mut memory, ValueAssign, base, base, defaults),
        Ok(base)
    );
    assert_eq!(read::<32>(&mut memory, base), value(7));
    assert_eq!(
        call(&mut memory, ValueCopyCtor, base, base, defaults),
        Ok(base)
    );
    assert_eq!(read::<32>(&mut memory, base), [0; 32]);
    assert_eq!(
        call(&mut memory, ValueAssign, u64::MAX, 0, defaults),
        Ok(u64::MAX)
    );
    assert_eq!(
        call(&mut memory, ValueAssign, u64::MAX, u64::MAX, defaults),
        Ok(u64::MAX)
    );
}

#[test]
fn precise_type_only_getter_and_late_payload_failure_preserve_the_destination() {
    use JsonFunction::*;
    let mut memory = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = memory.reserved_ranges().next().unwrap().address;
    memory
        .map_at(base + 0x10000, 0x20000, NativePermissions::READ_WRITE)
        .unwrap();
    memory
        .pause()
        .unwrap()
        .write(base + 0x10000, &2_u32.to_le_bytes())
        .unwrap();
    memory
        .protect(base + 0x10000, 0x10000, NativePermissions::READ)
        .unwrap();
    let defaults = ScalarDefaults {
        boolean: base + 0x10008,
        integer: base + 0x10010,
        real: base + 0x10018,
    };
    let source = base + 0x10000 - 28;
    assert_eq!(call(&mut memory, ValueGetType, source, 0, defaults), Ok(2));
    assert_eq!(
        call(&mut memory, ValueGetInteger, source, 0, defaults),
        Ok(source + 16)
    );
    assert!(
        memory
            .pause()
            .unwrap()
            .preflight_read(source + 16, 8)
            .is_err()
    );
    let destination = base + 0x20100;
    for function in [ValueCopyCtor, ValueAssign] {
        memory
            .pause()
            .unwrap()
            .write(destination, &value(0))
            .unwrap();
        assert_eq!(
            call(&mut memory, function, destination, source, defaults),
            Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
                pentax_kernel::GuestError::Unmapped
            )))
        );
        assert_eq!(read::<32>(&mut memory, destination), value(0));
    }
}

#[test]
fn readonly_late_output_boundary_and_work_refusal_publish_nothing() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let boundary = base + 0x10000 - 28;
    memory
        .pause()
        .unwrap()
        .write(boundary, &[0x71; 28])
        .unwrap();
    let before = read::<32>(&mut memory, boundary);
    assert_eq!(
        call(&mut memory, ValueSetInt, boundary, 42, defaults),
        Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
            pentax_kernel::GuestError::WriteDenied
        )))
    );
    assert_eq!(read::<32>(&mut memory, boundary), before);
    assert!(call(&mut memory, ValueBoolCtor, defaults.boolean, 1, defaults).is_err());
    assert_eq!(read::<24>(&mut memory, defaults.boolean), [0; 24]);
    let object = base + 0x100;
    memory.pause().unwrap().write(object, &value(0)).unwrap();
    let mut work = 7;
    assert_eq!(
        service(
            &mut memory.pause().unwrap(),
            ValueSetUInt,
            [object, 42, 0, 0, 0, 0],
            None,
            defaults,
            &mut work,
            7 + MUTATION_WORK - 1
        ),
        Err(KernelStartupError::Budget("provider work"))
    );
    assert_eq!(work, 7);
    assert_eq!(read::<32>(&mut memory, object), value(0));
    assert_eq!(
        service(
            &mut memory.pause().unwrap(),
            ValueSetUInt,
            [object, 42, 0, 0, 0, 0],
            None,
            defaults,
            &mut work,
            7 + MUTATION_WORK
        ),
        Ok(0)
    );
    assert_eq!(work, 7 + MUTATION_WORK);
}

#[test]
fn distinct_physical_aliases_refuse_before_ordered_copy_can_publish() {
    let mut memory = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    let base = memory.reserved_ranges().next().unwrap().address;
    for effect in [
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: 0x10000,
        },
        NativeMemoryEffect::MapPhysical {
            address: base,
            size: 0x10000,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
        NativeMemoryEffect::MapPhysical {
            address: base + 0x10000,
            size: 0x10000,
            physical: 0,
            permissions: NativePermissions::READ_WRITE,
        },
    ] {
        memory
            .pause()
            .unwrap()
            .apply_transaction(
                NativeMemoryTransaction {
                    effect,
                    writes: &[],
                },
                &Default::default(),
            )
            .unwrap();
    }
    let destination = base + 0x100;
    let source = base + 0x10100;
    memory
        .pause()
        .unwrap()
        .write(destination, &value(2))
        .unwrap();
    let defaults = ScalarDefaults {
        boolean: 0,
        integer: 0,
        real: 0,
    };
    assert_eq!(
        call(
            &mut memory,
            JsonFunction::ValueAssign,
            destination,
            source,
            defaults
        ),
        Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(
            pentax_kernel::GuestError::WriteDenied
        )))
    );
    assert_eq!(read::<32>(&mut memory, destination), value(2));
    assert_eq!(read::<32>(&mut memory, source), value(2));
}

#[test]
fn null_owned_payloads_clear_and_count_without_fabricating_nonnull_storage() {
    use JsonFunction::*;
    let (mut memory, base, defaults) = owner();
    let object = base + 0x100;
    let source = base + 0x200;
    for kind in 5..=7 {
        let mut null_owned = value(kind);
        null_owned[16..24].fill(0);
        for function in [ValueDtor, ValueClearMethod, ValueSetInt] {
            memory.pause().unwrap().write(object, &null_owned).unwrap();
            assert_eq!(call(&mut memory, function, object, 42, defaults), Ok(0));
            let mut expected = [0; 32];
            expected[24..28].copy_from_slice(&null_owned[24..28]);
            if function == ValueClearMethod {
                expected[..16].copy_from_slice(&null_owned[..16]);
            }
            if function == ValueSetInt {
                expected[16..24].copy_from_slice(&42_u64.to_le_bytes());
                expected[28..32].copy_from_slice(&2_u32.to_le_bytes());
            }
            assert_eq!(read::<32>(&mut memory, object), expected);
        }
        memory.pause().unwrap().write(object, &null_owned).unwrap();
        assert_eq!(call(&mut memory, ValueCount, object, 0, defaults), Ok(0));
        // Copying even an empty owned value allocates a fresh wrapper upstream.
        memory.pause().unwrap().write(source, &null_owned).unwrap();
        for function in [ValueCopyCtor, ValueAssign] {
            memory.pause().unwrap().write(object, &value(0)).unwrap();
            assert_eq!(
                call(&mut memory, function, object, source, defaults),
                Err(KernelStartupError::UnavailableJsonValueStorage)
            );
            assert_eq!(read::<32>(&mut memory, object), value(0));
        }
    }
}
