// SPDX-License-Identifier: GPL-2.0-only
#![cfg(all(target_os = "windows", target_arch = "x86_64"))]

//! Original byte fixtures over actual reserved native memory; no guest execution.
use super::*;
use pentax_kernel::{Kernel, Limits, ProcessIdentity};
use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

struct Fixture {
    memory: NativeAddressSpace,
    kernel: Kernel,
    strings: JsonStrings,
    base: u64,
    work: u64,
}

impl Fixture {
    fn new() -> Self {
        let mut memory = NativeAddressSpace::reserve_any(Default::default(), 0x800000).unwrap();
        let base = memory.reserved_ranges().next().unwrap().address;
        memory
            .map_at(base, 0x10000, NativePermissions::READ_WRITE)
            .unwrap();
        let kernel = Kernel::new(
            Limits {
                virtual_base: base + 0x10000,
                virtual_bytes: 0x7f0000,
                max_handles: 1024,
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
        Self {
            memory,
            kernel,
            strings: JsonStrings::new(),
            base,
            work: 0,
        }
    }

    fn call(
        &mut self,
        function: JsonFunction,
        first: u64,
        second: u64,
    ) -> Result<u64, KernelStartupError> {
        self.strings.service(
            &mut self.memory.pause().unwrap(),
            &mut self.kernel,
            JsonCall {
                function,
                args: [first, second, 0, 0, 0, 0],
                vector_args: None,
                defaults: ScalarDefaults {
                    boolean: self.base + 0xf000,
                    integer: self.base + 0xf008,
                    real: self.base + 0xf010,
                },
                empty_string: self.base + 0xf018,
                empty_cstr: self.base + 0xf020,
            },
            &mut self.work,
            128 * 1024 * 1024,
        )
    }

    fn put(&mut self, address: u64, bytes: &[u8]) {
        self.memory.pause().unwrap().write(address, bytes).unwrap();
    }

    fn read<const N: usize>(&mut self, address: u64) -> [u8; N] {
        let mut bytes = [0; N];
        self.memory
            .pause()
            .unwrap()
            .read(address, &mut bytes)
            .unwrap();
        bytes
    }

    fn word(&mut self, address: u64) -> u64 {
        u64::from_le_bytes(self.read(address))
    }

    fn text(&mut self, wrapper: u64) -> Vec<u8> {
        let length = self.call(JsonFunction::StringLength, wrapper, 0).unwrap() as usize;
        let data = self.call(JsonFunction::StringCStr, wrapper, 0).unwrap();
        let mut bytes = vec![0; length];
        self.memory.pause().unwrap().read(data, &mut bytes).unwrap();
        bytes
    }
}

#[test]
fn lifecycle_uses_current_guest_bytes_preserves_impl_on_assignment_and_refuses_stale_tokens() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let a = f.base + 0x100;
    let b = f.base + 0x200;
    let input = f.base + 0x5000;
    f.put(input, b"\xffabc\0");
    assert_eq!(f.call(StringCStringCtor, a, input), Ok(a));
    let token = f.word(a);
    assert_ne!(token, 0);
    assert_eq!(f.text(a), b"\xffabc");
    assert_eq!(f.call(StringCtor, b, 0), Ok(b));
    let b_token = f.word(b);
    let data = f.call(StringCStr, a, 0).unwrap();
    f.put(data + 1, b"Z");
    assert_eq!(f.call(StringAssign, b, a), Ok(b));
    assert_eq!(f.word(b), b_token);
    assert_eq!(f.text(b), b"\xffZbc");
    assert_eq!(f.call(StringAssign, a, a), Ok(a));
    assert_eq!(f.word(a), token);
    assert_eq!(f.text(a), b"\xffZbc");
    assert_eq!(f.call(StringDtor, a, 0), Ok(0));
    assert_eq!(f.word(a), 0);
    assert_eq!(f.call(StringLength, a, 0), Ok(0));
    let forged = f.base + 0x300;
    f.put(forged, &token.to_le_bytes());
    assert_eq!(
        f.call(StringLength, forged, 0),
        Err(KernelStartupError::InvalidJsonString)
    );
    assert_eq!(f.text(b), b"\xffZbc");
}

#[test]
fn string_queries_do_not_read_payload_or_require_a_terminator_read() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let wrapper = f.base + 0x100;
    let input = f.base + 0x5000;
    f.put(input, b"live\0");
    f.call(StringCStringCtor, wrapper, input).unwrap();
    let data = f.call(StringCStr, wrapper, 0).unwrap();
    f.put(data + 4, b"!");
    assert_eq!(f.call(StringLength, wrapper, 0), Ok(4));
    assert_eq!(f.call(StringCStr, wrapper, 0), Ok(data));
    // Exact std::string copying reads four bytes and supplies its own NUL.
    let copied = f.base + 0x200;
    f.call(StringCtor, copied, 0).unwrap();
    f.call(StringAssign, copied, wrapper).unwrap();
    let new_data = f.call(StringCStr, copied, 0).unwrap();
    assert_eq!(f.read::<5>(new_data), *b"live\0");
}

#[test]
fn deep_value_copy_and_scalar_cleanup_share_the_string_owner_and_preserve_padding() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let a = f.base + 0x100;
    let b = f.base + 0x200;
    let input = f.base + 0x5000;
    f.put(input, b"independent\0");
    f.call(ValueCStringCtor, a, input).unwrap();
    let source_wrapper = f.call(ValueGetString, a, 0).unwrap();
    f.put(b, &[0; 32]);
    f.put(b + 24, &[0x97; 4]);
    assert_eq!(f.call(ValueAssign, b, a), Ok(b));
    let copy_wrapper = f.call(ValueGetString, b, 0).unwrap();
    assert_ne!(source_wrapper, copy_wrapper);
    assert_ne!(f.word(source_wrapper), f.word(copy_wrapper));
    let data = f.call(StringCStr, source_wrapper, 0).unwrap();
    f.put(data, b"X");
    assert_eq!(f.text(copy_wrapper), b"independent");
    assert_eq!(f.call(ValueSetInt, b, 42), Ok(0));
    assert_eq!(f.read::<4>(b + 24), [0x97; 4]);
    assert_eq!(f.word(b + 16), 42);
    assert_eq!(
        f.call(StringLength, copy_wrapper, 0),
        Err(KernelStartupError::InvalidJsonString)
    );
    assert_eq!(f.text(source_wrapper), b"Xndependent");
}

#[test]
fn null_const_source_stays_unmaterialized_but_value_copy_publishes_all_four_outputs() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let empty_external = f.base + 0x100;
    let a = f.base + 0x200;
    let b = f.base + 0x300;
    assert_eq!(f.call(StringLength, empty_external, 0), Ok(0));
    let handles = f.kernel.usage().handles_live;
    f.call(ValueStringCtor, a, empty_external).unwrap();
    assert_eq!(f.word(empty_external), 0);
    assert_eq!(f.kernel.usage().handles_live, handles + 1);
    let source_wrapper = f.call(ValueGetString, a, 0).unwrap();
    f.call(StringDtor, source_wrapper, 0).unwrap();
    assert_eq!(f.word(source_wrapper), 0);
    f.put(b, &[0; 32]);
    f.put(b + 24, &[0x74; 4]);
    let handles = f.kernel.usage().handles_live;
    f.call(ValueAssign, b, a).unwrap();
    assert_eq!(f.kernel.usage().handles_live, handles + 1);
    assert_ne!(f.word(source_wrapper), 0);
    let copied = f.call(ValueGetString, b, 0).unwrap();
    assert_ne!(copied, source_wrapper);
    assert_ne!(f.word(copied), f.word(source_wrapper));
    assert_eq!(f.text(copied), b"");
    assert_eq!(f.read::<4>(b + 24), [0x74; 4]);
    f.call(ValueDtor, a, 0).unwrap();
    assert_eq!(f.text(copied), b"");
    f.call(ValueDtor, b, 0).unwrap();
}

#[test]
fn late_value_write_failure_preserves_old_ownership_mapping_counters_and_all_bytes_then_retries() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let value = f.base + 0x4000 - 28;
    let source = f.base + 0x8100;
    let input = f.base + 0x9000;
    f.put(input, b"old\0");
    f.call(ValueCStringCtor, value, input).unwrap();
    let old_wrapper = f.call(ValueGetString, value, 0).unwrap();
    f.put(input, b"new\0");
    f.call(StringCStringCtor, source, input).unwrap();
    let before = f.read::<32>(value);
    let usage = f.kernel.usage();
    let records = f.strings.records.len();
    f.memory
        .protect(f.base + 0x4000, 0x4000, NativePermissions::READ)
        .unwrap();
    assert!(matches!(
        f.call(ValueSetString, value, source),
        Err(KernelStartupError::Kernel(_))
    ));
    assert_eq!(f.read::<32>(value), before);
    assert_eq!(f.strings.records.len(), records);
    assert_eq!(f.kernel.usage().handles_live, usage.handles_live);
    assert_eq!(f.kernel.usage().mapped_bytes, usage.mapped_bytes);
    assert_eq!(f.text(old_wrapper), b"old");
    f.memory
        .protect(f.base + 0x4000, 0x4000, NativePermissions::READ_WRITE)
        .unwrap();
    f.call(ValueSetString, value, source).unwrap();
    let new_wrapper = f.call(ValueGetString, value, 0).unwrap();
    assert_eq!(f.text(new_wrapper), b"new");
    assert_eq!(
        f.call(StringLength, old_wrapper, 0),
        Err(KernelStartupError::InvalidJsonString)
    );
}

#[test]
fn constructors_observe_init_overlay_and_self_copy_ctor_does_not_become_assignment() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let value = f.base + 0x100;
    f.put(value, b"before\0");
    f.call(ValueCStringCtor, value, value).unwrap();
    let wrapper = f.call(ValueGetString, value, 0).unwrap();
    assert_eq!(f.text(wrapper), b"");
    let input = f.base + 0x5000;
    f.put(input, b"retained\0");
    f.call(ValueCStringCtor, value, input).unwrap();
    let old = f.call(ValueGetString, value, 0).unwrap();
    f.call(ValueCopyCtor, value, value).unwrap();
    assert_eq!(f.read::<32>(value), [0; 32]);
    // Pinned construction overwrites; it does not delete the old payload.
    assert_eq!(f.text(old), b"retained");
    f.put(value, &[0; 32]);
    f.call(ValueStringCtor, value, value).unwrap();
    let empty = f.call(ValueGetString, value, 0).unwrap();
    assert_eq!(f.text(empty), b"");
}

#[test]
fn empty_type_and_clear_method_have_real_storage_and_source_link_semantics() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let value = f.base + 0x100;
    f.call(ValueTypeCtor, value, 5).unwrap();
    let wrapper = f.call(ValueGetString, value, 0).unwrap();
    assert_ne!(wrapper, 0);
    assert_eq!(f.text(wrapper), b"");
    f.put(value, &[0x38; 16]);
    f.put(value + 24, &[0x91; 4]);
    f.call(ValueClearMethod, value, 0).unwrap();
    assert_eq!(f.read::<16>(value), [0x38; 16]);
    assert_eq!(f.read::<4>(value + 24), [0x91; 4]);
    assert_eq!(f.word(value + 16), 0);
    assert_eq!(
        f.call(StringLength, wrapper, 0),
        Err(KernelStartupError::InvalidJsonString)
    );
    f.call(ValueSetType, value, 5).unwrap();
    let new_wrapper = f.call(ValueGetString, value, 0).unwrap();
    assert_eq!(f.text(new_wrapper), b"");
    assert_eq!(f.read::<16>(value), [0; 16]);
}

#[test]
fn serializer_retains_pinned_unescaped_and_self_append_behavior_and_refuses_real_atomically() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let value = f.base + 0x100;
    let dst = f.base + 0x200;
    let input = f.base + 0x5000;
    f.put(input, b"a\"\\\n\0");
    f.call(ValueCStringCtor, value, input).unwrap();
    f.call(StringCtor, dst, 0).unwrap();
    f.call(ValueToString, value, dst).unwrap();
    assert_eq!(f.text(dst), b"a\"\\\n");
    f.call(ValueSerialize, value, dst).unwrap();
    assert_eq!(f.text(dst), b"\"a\"\\\n\"");
    let own = f.call(ValueGetString, value, 0).unwrap();
    f.call(ValueSerialize, value, own).unwrap();
    assert_eq!(f.text(own), b"\"\"\"");
    f.call(ValueSetInt, value, i64::MIN as u64).unwrap();
    f.call(ValueToString, value, dst).unwrap();
    assert_eq!(f.text(dst), b"-9223372036854775808");
    f.call(ValueSetUInt, value, u64::MAX).unwrap();
    f.call(ValueSerialize, value, dst).unwrap();
    assert_eq!(f.text(dst), b"18446744073709551615");
    f.put(value + 28, &4_u32.to_le_bytes());
    let before = f.text(dst);
    let token = f.word(dst);
    let handles = f.kernel.usage().handles_live;
    assert_eq!(
        f.call(ValueToString, value, dst),
        Err(KernelStartupError::UnavailableJsonFormat)
    );
    assert_eq!(f.word(dst), token);
    assert_eq!(f.text(dst), before);
    assert_eq!(f.kernel.usage().handles_live, handles);
}

#[test]
fn address_dependent_partial_overlap_and_budget_failure_publish_no_lazy_wrapper() {
    use JsonFunction::*;
    let mut f = Fixture::new();
    let value = f.base + 0x100;
    let destination = value + 24;
    let handles = f.kernel.usage().handles_live;
    assert_eq!(
        f.call(ValueToString, value, destination),
        Err(KernelStartupError::InvalidJsonString)
    );
    assert_eq!(f.read::<32>(value), [0; 32]);
    assert_eq!(f.kernel.usage().handles_live, handles);
    let wrapper = f.base + 0x200;
    let mut work = 0;
    assert!(matches!(
        f.strings.service(
            &mut f.memory.pause().unwrap(),
            &mut f.kernel,
            JsonCall {
                function: StringCtor,
                args: [wrapper, 0, 0, 0, 0, 0],
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
            1
        ),
        Err(KernelStartupError::Budget(_))
    ));
    assert_eq!(f.word(wrapper), 0);
    assert_eq!(f.kernel.usage().handles_live, handles);
}
