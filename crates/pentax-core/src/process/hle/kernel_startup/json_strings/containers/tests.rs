// SPDX-License-Identifier: GPL-2.0-only
//! Original actual-backing trees, ordered mutations and failed publications.
use super::*;

fn invoke(
    f: &mut Fixture,
    function: JsonFunction,
    args: [u64; 3],
) -> Result<u64, KernelStartupError> {
    f.strings.service(
        &mut f.memory.pause().unwrap(),
        &mut f.kernel,
        JsonCall {
            function,
            args: [args[0], args[1], args[2], 0, 0, 0],
            vector_args: None,
            defaults: ScalarDefaults {
                boolean: f.base + 0xf000,
                integer: f.base + 0xf008,
                real: f.base + 0xf010,
            },
            empty_string: f.base + 0xf018,
            empty_cstr: f.base + 0xf020,
        },
        &mut f.work,
        128 * 1024 * 1024,
    )
}
fn index(f: &mut Fixture, value: u64, key: &[u8]) -> u64 {
    let at = f.base + 0x5800;
    f.put(at, key);
    f.put(at + key.len() as u64, &[0]);
    f.call(JsonFunction::ValueIndexString, value, at).unwrap()
}

#[test]
fn parsed_tree_has_source_types_parents_sorted_keys_deep_copy_and_real_serialization() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let root = f.base + 0x100;
    let copy = f.base + 0x200;
    let source = f.base + 0x4000;
    let text = br#"{"r":1.25,"b":[1,-2,true,null],"a":"x\uD83D\uDE00","d":0,"d":7}"#;
    f.put(root - 1, &[0xa7; 34]);
    f.put(source, text);
    assert_eq!(
        invoke(&mut f, ParserParse, [root, source, text.len() as u64]),
        Ok(0)
    );
    assert_eq!(f.read::<1>(root - 1), [0xa7]);
    assert_eq!(f.read::<1>(root + 32), [0xa7]);
    assert_eq!(f.read::<4>(root + 24), [0xa7; 4]);
    assert_eq!(f.call(ValueCount, root, 0), Ok(4));
    let array = index(&mut f, root, b"b");
    assert_eq!(f.word(array), root);
    assert_eq!(f.call(ValueCount, array, 0), Ok(4));
    let first = f.call(ValueIndexUInt, array, 0).unwrap();
    let second = f.call(ValueIndexUInt, array, 1).unwrap();
    assert_eq!(f.word(first), array);
    assert_eq!(f.call(ValueGetType, first, 0), Ok(3));
    assert_eq!(f.call(ValueGetType, second, 0), Ok(2));
    assert_eq!(f.word(second + 16), (-2_i64) as u64);
    assert_eq!(f.call(ValueCopyCtor, copy, root), Ok(copy));
    let copied = index(&mut f, copy, b"b");
    let child = f.call(ValueIndexUInt, copied, 0).unwrap();
    f.call(ValueSetInt, child, 99).unwrap();
    assert_eq!(f.word(first + 16), 1);
    let output = f.base + 0x300;
    f.call(StringCtor, output, 0).unwrap();
    assert_eq!(f.call(ValueSerialize, root, output), Ok(0));
    assert_eq!(
        f.text(output),
        "{\"a\":\"x😀\",\"b\":[1,-2,true,null],\"d\":7,\"r\":1.250000}".as_bytes()
    );
    let wrapper = f.call(ValueGetArray, array, 0).unwrap();
    let begin = f.base + 0x600;
    let end = begin + 8;
    assert_eq!(f.call(ArrayBegin, begin, wrapper), Ok(begin));
    assert_eq!(f.call(ArrayEnd, end, wrapper), Ok(end));
    for expected in [
        first,
        second,
        f.call(ValueIndexUInt, array, 2).unwrap(),
        f.call(ValueIndexUInt, array, 3).unwrap(),
    ] {
        assert_eq!(f.call(ArrayIteratorNotEqual, begin, end), Ok(1));
        assert_eq!(f.call(ArrayIteratorDereference, begin, 0), Ok(expected));
        assert_eq!(f.call(ArrayIteratorIncrement, begin, 0), Ok(begin));
    }
    assert_eq!(f.call(ArrayIteratorNotEqual, begin, end), Ok(0));
    assert!(f.call(ArrayIteratorDereference, begin, 0).is_err());
    assert_eq!(f.call(ArrayIteratorDtor, u64::MAX, u64::MAX), Ok(0));
    // Lexical negative zero is an integer in the pinned parser. Real zeros
    // retain their sign, including values nested inside a duplicate member.
    let zeros = br#"{"z":8,"z":[-0,-0.0,-0e0,0]}"#;
    f.put(source, zeros);
    assert_eq!(
        invoke(&mut f, ParserParse, [copy, source, zeros.len() as u64]),
        Ok(0)
    );
    let zeros = index(&mut f, copy, b"z");
    for (position, expected_type, bits) in [
        (0, 2, 0),
        (1, 4, 1_u64 << 63),
        (2, 4, 1_u64 << 63),
        (3, 3, 0),
    ] {
        let value = f.call(ValueIndexUInt, zeros, position).unwrap();
        assert_eq!(f.call(ValueGetType, value, 0), Ok(expected_type));
        assert_eq!(f.word(value + 16), bits);
    }
    // Sized buffers retain the upstream lexer EOF convention outside strings.
    let terminated = b"{\"z\":\"a\\u0000b\"}\0ignored";
    f.put(source, terminated);
    assert_eq!(
        invoke(&mut f, ParserParse, [copy, source, terminated.len() as u64]),
        Ok(0)
    );
    let value = index(&mut f, copy, b"z");
    let string = f.call(ValueGetString, value, 0).unwrap();
    assert_eq!(f.text(string), b"a\0b");
}

#[test]
fn real_object_array_lifetimes_copy_before_clear_and_iterator_invalidation() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let object = f.base + 0x100;
    let other = object + 16;
    let key = f.base + 0x200;
    let text = f.base + 0x4000;
    let root = f.base + 0x300;
    f.put(text, b"nested\0");
    f.call(StringCStringCtor, key, text).unwrap();
    assert_eq!(f.call(ObjectCtor, object, 0), Ok(object));
    let nested = f.call(ObjectIndex, object, key).unwrap();
    f.call(ValueSetType, nested, 7).unwrap();
    let inside = f.call(ValueGetObject, nested, 0).unwrap();
    f.put(text, b"leaf\0");
    let leaf_key = key + 16;
    f.call(StringCStringCtor, leaf_key, text).unwrap();
    let leaf = f.call(ObjectIndex, inside, leaf_key).unwrap();
    f.call(ValueSetInt, leaf, 13).unwrap();
    f.call(ObjectCopyCtor, other, object).unwrap();
    let copied_nested = f.call(ObjectIndex, other, key).unwrap();
    let copied_inside = f.call(ValueGetObject, copied_nested, 0).unwrap();
    let copied_leaf = f.call(ObjectIndex, copied_inside, leaf_key).unwrap();
    f.call(ValueSetInt, leaf, 21).unwrap();
    assert_eq!(f.word(copied_leaf + 16), 13);
    f.call(ValueObjectCtor, root, object).unwrap();
    let source_child = index(&mut f, root, b"nested");
    let descendant = f.call(ValueGetObject, source_child, 0).unwrap();
    assert_eq!(f.call(ValueSetObject, root, descendant), Ok(0));
    let selected = index(&mut f, root, b"leaf");
    assert_eq!(f.word(selected + 16), 21);
    let array = f.base + 0x500;
    let array_value = array + 32;
    let iterator = array + 64;
    f.call(ArrayCtor, array, 0).unwrap();
    f.call(ArrayPushBack, array, root).unwrap();
    f.call(ValueArrayCtor, array_value, array).unwrap();
    assert_eq!(f.call(ValueCount, array_value, 0), Ok(1));
    f.call(ArrayBegin, iterator, array).unwrap();
    f.call(ArrayPushBack, array, copied_leaf).unwrap();
    assert!(f.call(ArrayIteratorDereference, iterator, 0).is_err());
    assert_eq!(f.call(ValueCount, array_value, 0), Ok(1));
    f.call(ArrayDtor, array, 0).unwrap();
    assert_eq!(f.word(array), 0);
    f.call(ObjectDtor, object, 0).unwrap();
    f.call(ObjectDtor, other, 0).unwrap();
    f.call(ValueDtor, root, 0).unwrap();
    f.call(ValueDtor, array_value, 0).unwrap();
    assert_eq!(f.call(ValueCount, root, 0), Ok(0));
}

#[test]
fn parser_source_errors_and_late_output_fault_preserve_destination_and_owned_state() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let root = f.base + 0x100;
    let source = f.base + 0x4000;
    f.call(ValueIntCtor, root, 73).unwrap();
    let before = f.read::<32>(root);
    let handles = f.kernel.usage().handles_live;
    for malformed in [
        b"{".as_slice(),
        b"[1,]",
        b"\"\\uD800\"",
        b"\xff",
        b"true false",
        b"\"a\0b\"",
        b"[1,\0]",
    ] {
        f.put(source, malformed);
        assert_eq!(
            invoke(&mut f, ParserParse, [root, source, malformed.len() as u64]),
            Ok(0x8084_8101)
        );
        assert_eq!(f.read::<32>(root), before);
        assert_eq!(f.kernel.usage().handles_live, handles);
    }
    assert_eq!(
        invoke(&mut f, ParserParse, [0, u64::MAX, u64::MAX]),
        Ok(0x8084_8120)
    );
    assert_eq!(
        invoke(&mut f, ParserParse, [u64::MAX, source, 1]),
        Ok(0x8084_8101)
    );
    assert_eq!(
        invoke(&mut f, ParserParse, [root, u64::MAX, 0]),
        Ok(0x8084_8101)
    );
    f.put(source, br#"{"live":[1,2,3]}"#);
    f.memory
        .pause()
        .unwrap()
        .apply_transaction(
            pentax_platform::native_memory::NativeMemoryTransaction {
                effect: pentax_platform::native_memory::NativeMemoryEffect::Protect {
                    address: f.base,
                    size: 0x4000,
                    permissions: NativePermissions::READ,
                },
                writes: &[],
            },
            &Default::default(),
        )
        .unwrap();
    assert!(invoke(&mut f, ParserParse, [root, source, 16]).is_err());
    assert_eq!(f.read::<32>(root), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
}

#[test]
fn defaults_foreign_tokens_depth_work_and_guest_budget_are_controlled_before_publication() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let root = f.base + 0x100;
    let source = f.base + 0x4000;
    f.kernel = Kernel::new(
        Limits {
            virtual_base: f.base + 0x10000,
            virtual_bytes: 0x7f0000,
            max_handles: 1,
            max_bytes: 64 * 1024 * 1024,
            max_work: 128 * 1024 * 1024,
            max_mapped_bytes: 0x7f0000,
            ..Default::default()
        },
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 1,
        },
        Vec::new(),
    )
    .unwrap();
    let empty = f.call(ValueGetArray, 0, u64::MAX).unwrap();
    assert_ne!(empty, 0);
    let begin = root + 64;
    let end = begin + 8;
    f.call(ArrayBegin, begin, empty).unwrap();
    f.call(ArrayEnd, end, empty).unwrap();
    assert_eq!(f.call(ArrayIteratorNotEqual, begin, end), Ok(0));
    let null = f.call(ValueIndexUInt, 0, u64::MAX).unwrap();
    assert_eq!(f.read::<32>(null), [0; 32]);
    let handles = f.kernel.usage().handles_live;
    assert!(f.call(ArrayCtor, 0x8000_0000_0000_0000, 0).is_err());
    assert_eq!(f.kernel.usage().handles_live, handles);
    f.put(root, &0x8000_0000_0000_0000_u64.to_le_bytes());
    assert!(f.call(ArrayPushBack, root, 0).is_err());
    let mut deep = vec![b'['; 66];
    deep.push(b'0');
    deep.extend([b']'; 66]);
    f.put(source, &deep);
    let before = f.read::<32>(root);
    assert!(matches!(
        invoke(&mut f, ParserParse, [root, source, deep.len() as u64]),
        Err(KernelStartupError::Budget(_))
    ));
    assert_eq!(f.read::<32>(root), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
    assert!(matches!(
        f.call(ArrayCtor, root, 0),
        Err(KernelStartupError::Kernel(pentax_kernel::Failure::Limit(_)))
    ));
    assert_eq!(f.read::<32>(root), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
    let mut work = 0;
    assert!(
        f.strings
            .service(
                &mut f.memory.pause().unwrap(),
                &mut f.kernel,
                JsonCall {
                    function: ArrayCtor,
                    args: [root, 0, 0, 0, 0, 0],
                    vector_args: None,
                    defaults: ScalarDefaults {
                        boolean: 0,
                        integer: 0,
                        real: 0
                    },
                    empty_string: 0,
                    empty_cstr: 0,
                },
                &mut work,
                1023
            )
            .is_err()
    );
    assert_eq!(f.read::<32>(root), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
    // Fixed admission succeeds, then a retained-record clone consumes work
    // before its reservation refuses. Retrying must retain that attempted work.
    let mut work = 0;
    assert!(
        f.strings
            .service(
                &mut f.memory.pause().unwrap(),
                &mut f.kernel,
                JsonCall {
                    function: ArrayCtor,
                    args: [root, 0, 0, 0, 0, 0],
                    vector_args: None,
                    defaults: ScalarDefaults {
                        boolean: 0,
                        integer: 0,
                        real: 0
                    },
                    empty_string: 0,
                    empty_cstr: 0,
                },
                &mut work,
                1027
            )
            .is_err()
    );
    assert!(work > 1024);
    assert_eq!(f.read::<32>(root), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
}
