// SPDX-License-Identifier: GPL-2.0-or-later

//! Original two-module byte arrays verify whole-session staging without execution.

use super::*;
use pentax_linker::{ModuleInput, SymbolicError, SymbolicLimits, plan_symbolic};
use pentax_loader::open_executable;

const CONSUMER_BIAS: u64 = 0x10000;
const PROVIDER_BIAS: u64 = 0x40000;
const PROVIDER_SYMBOL: u64 = 0x100;
const POINTER_SLOT: u64 = 0x4100;
const BSS_SLOT: u64 = 0x8000;
const ORIGINAL: u64 = 0xfeed_face_1234_5678;
const PROVIDER_BYTES: [u8; 8] = *b"PENTAPRV";

#[test]
fn staged_provider_pointers_address_the_same_owned_session_memory() {
    let consumer = module(false, false);
    let provider = module(true, false);
    let snapshots = [consumer.clone(), provider.clone()];
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let mut session = load_symbolic(&plan, &SessionLoadLimits::default()).unwrap();
    let provider_address = PROVIDER_BIAS + PROVIDER_SYMBOL;
    assert_eq!(session.primary_entry(), CONSUMER_BIAS + 0x180);
    assert_eq!(session.module_count(), 2);
    assert_eq!(session.applied_relocations(), 5);
    assert_eq!(session.deferred_relocations(), 0);
    assert_eq!(
        session.relocation_status(),
        RelocationStatus::CompleteWithinPolicy
    );
    assert_eq!(session.memory().mapped_bytes(), PAGE_SIZE * 4);
    assert_eq!(session.memory().resident_bytes(), PAGE_SIZE * 4);
    assert_eq!(
        read_u64(session.memory(), CONSUMER_BIAS + POINTER_SLOT),
        provider_address - 8
    );
    for offset in [8, 16] {
        let pointer = read_u64(session.memory(), CONSUMER_BIAS + POINTER_SLOT + offset);
        assert_eq!(pointer, provider_address);
        let mut provider_payload = [0; 8];
        session
            .memory()
            .read(pointer, &mut provider_payload)
            .unwrap();
        assert_eq!(provider_payload, PROVIDER_BYTES);
    }
    assert_eq!(
        read_u64(session.memory(), CONSUMER_BIAS + POINTER_SLOT + 24),
        CONSUMER_BIAS + 0x1234
    );
    assert_eq!(
        read_u64(session.memory(), CONSUMER_BIAS + BSS_SLOT),
        CONSUMER_BIAS + 0x2222
    );
    let mut zero_tail = [1; 8];
    session
        .memory()
        .read(CONSUMER_BIAS + BSS_SLOT + 8, &mut zero_tail)
        .unwrap();
    assert_eq!(zero_tail, [0; 8]);
    // No public mutable getter exists. A child test can verify both original RX
    // flags and post-relocation RELRO revocation on the private staging memory.
    assert_eq!(
        session.memory.write(CONSUMER_BIAS + POINTER_SLOT, &[0]),
        Err(MemoryError::PermissionDenied(CONSUMER_BIAS + POINTER_SLOT))
    );
    assert_eq!(
        session.memory.write(provider_address, &[0]),
        Err(MemoryError::PermissionDenied(provider_address))
    );
    assert_eq!(
        session.memory.fetch(CONSUMER_BIAS + POINTER_SLOT, &mut [0]),
        Err(MemoryError::PermissionDenied(CONSUMER_BIAS + POINTER_SLOT))
    );
    session.memory.fetch(provider_address, &mut [0; 8]).unwrap();
    assert_eq!([consumer, provider], snapshots);
}

#[test]
fn deferred_records_remain_explicit_in_a_read_only_unstarted_session() {
    let consumer = module(false, true);
    let provider = module(true, false);
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let session = load_symbolic(&plan, &SessionLoadLimits::default()).unwrap();
    assert_eq!(session.applied_relocations(), 5);
    assert_eq!(session.deferred_relocations(), 1);
    assert_eq!(
        session.relocation_status(),
        RelocationStatus::Deferred { count: 1 }
    );
    assert_eq!(
        read_u64(session.memory(), CONSUMER_BIAS + POINTER_SLOT + 32),
        ORIGINAL
    );
}

#[test]
fn late_copy_and_patch_budget_failures_discard_the_complete_unpublished_session() {
    let consumer = module(false, false);
    let provider = module(true, false);
    let snapshots = [consumer.clone(), provider.clone()];
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    // This independent old-API image must not be mutated by session attempts.
    let existing = load_runtime(
        plan.modules()[0].runtime_plan(),
        &LoadOptions {
            load_bias: CONSUMER_BIAS,
            ..LoadOptions::default()
        },
    )
    .unwrap();
    for resident_pages in [2, 3] {
        // Two pages allow all consumer bytes, then fail copying the provider.
        // Three allow every payload and four initialized-page patches, then
        // fail allocating the sparse page for the final BSS relocation.
        let limits = SessionLoadLimits {
            resident_limit: resident_pages * PAGE_SIZE,
            ..SessionLoadLimits::default()
        };
        assert!(matches!(
            load_symbolic(&plan, &limits),
            Err(ProcessError::Memory(MemoryError::BudgetExceeded))
        ));
        assert_eq!(
            read_u64(&existing.memory, CONSUMER_BIAS + POINTER_SLOT),
            ORIGINAL
        );
        assert_eq!(read_u64(&existing.memory, CONSUMER_BIAS + BSS_SLOT), 0);
        assert_eq!(consumer, snapshots[0]);
        assert_eq!(provider, snapshots[1]);
    }
    let complete = load_symbolic(&plan, &SessionLoadLimits::default()).unwrap();
    assert_eq!(complete.applied_relocations(), 5);
    assert_eq!(
        read_u64(complete.memory(), CONSUMER_BIAS + BSS_SLOT),
        CONSUMER_BIAS + 0x2222
    );
}

#[test]
fn partial_relro_is_an_explicit_policy_error_before_payload_allocation() {
    let provider = module(true, false);
    for (address, size) in [(0x4001, PAGE_SIZE - 1), (0x4000, PAGE_SIZE - 1)] {
        let mut consumer = module(false, false);
        program(&mut consumer, 3, 0x6474e552, 4, [0, address, 0, size, 1]);
        let snapshot = consumer.clone();
        let inputs = inputs(&consumer, &provider);
        let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        // Zero resident memory would reject the first payload if staging began.
        let limits = SessionLoadLimits {
            resident_limit: 0,
            ..SessionLoadLimits::default()
        };
        assert!(
            matches!(load_symbolic(&plan, &limits), Err(ProcessError::UnsupportedRelroAlignment { module: 0, address: biased, size: actual_size }) if biased == CONSUMER_BIAS + address && actual_size == size)
        );
        assert_eq!(consumer, snapshot);
    }
}

#[test]
fn empty_unaligned_relro_is_a_noop_and_provider_entry_is_not_primary() {
    let mut consumer = module(false, false);
    program(&mut consumer, 3, 0x6474e552, 4, [0, 0x4001, 0, 0, 1]);
    let mut provider = module(true, false);
    put(&mut provider, 24, &0xdead_beef_u64.to_le_bytes());
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let mut session = load_symbolic(&plan, &SessionLoadLimits::default()).unwrap();
    assert_eq!(session.primary_entry(), CONSUMER_BIAS + 0x180);
    session
        .memory
        .write(CONSUMER_BIAS + POINTER_SLOT, &[0])
        .unwrap();
}

#[test]
fn relro_preserves_execute_and_does_not_protect_adjacent_pages() {
    let mut consumer = module(false, false);
    program(
        &mut consumer,
        3,
        0x6474e552,
        4,
        [0, 0, 0, PAGE_SIZE, PAGE_SIZE],
    );
    let provider = module(true, false);
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let mut session = load_symbolic(&plan, &SessionLoadLimits::default()).unwrap();
    let mut code = [0];
    session
        .memory()
        .fetch(session.primary_entry(), &mut code)
        .unwrap();
    assert_eq!(code, [0xc3]);
    assert_eq!(
        session.memory.write(CONSUMER_BIAS + 0x180, &[0]),
        Err(MemoryError::PermissionDenied(CONSUMER_BIAS + 0x180))
    );
    session
        .memory
        .write(CONSUMER_BIAS + POINTER_SLOT, &[0])
        .unwrap();
}

#[test]
fn another_modules_executable_mapping_cannot_validate_the_primary_entry() {
    let mut consumer = module(false, false);
    let provider = module(true, false);
    let foreign_entry = PROVIDER_BIAS + PROVIDER_SYMBOL - CONSUMER_BIAS;
    put(&mut consumer, 24, &foreign_entry.to_le_bytes());
    let inputs = inputs(&consumer, &provider);
    assert!(matches!(
        plan_symbolic(&inputs, &SymbolicLimits::default()),
        Err(SymbolicError::PrimaryEntryOutsideExecutable { address }) if address == foreign_entry
    ));
}

#[test]
fn session_plan_rejects_logical_overlap_and_disjoint_cross_module_page_sharing() {
    for second_address in [0x108, 0x200] {
        let first = static_module(0x100);
        let second = static_module(second_address);
        let inputs = [
            ModuleInput::new(open_executable(&first).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(open_executable(&second).unwrap(), CONSUMER_BIAS),
        ];
        // The first pair overlaps logical bytes; the second is disjoint but
        // would merge ownership of a single guest page. Neither can be staged.
        assert!(matches!(
            plan_symbolic(&inputs, &SymbolicLimits::default()),
            Err(SymbolicError::OverlappingModulePages { address, .. }) if address == CONSUMER_BIAS
        ));
    }
}

#[test]
fn total_mapping_budget_is_charged_across_both_modules_before_writes() {
    let consumer = module(false, false);
    let provider = module(true, false);
    let inputs = inputs(&consumer, &provider);
    let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
    let limits = SessionLoadLimits {
        mapped_limit: PAGE_SIZE * 3,
        resident_limit: 0,
    };
    assert!(matches!(
        load_symbolic(&plan, &limits),
        Err(ProcessError::ImageTooLarge)
    ));
}

#[test]
fn retained_provider_bias_must_obey_core_alignment_and_address_limits() {
    let consumer = module(false, false);
    let provider = module(true, false);
    for provider_bias in [PROVIDER_BIAS + 1, ADDRESS_LIMIT] {
        let inputs = [
            ModuleInput::new(open_executable(&consumer).unwrap(), CONSUMER_BIAS),
            ModuleInput::new(open_executable(&provider).unwrap(), provider_bias),
        ];
        // The immutable pure plan uses checked u64 address arithmetic. Staging
        // imposes its narrower guest address/alignment policy before any copy.
        let plan = plan_symbolic(&inputs, &SymbolicLimits::default()).unwrap();
        let limits = SessionLoadLimits {
            resident_limit: 0,
            ..SessionLoadLimits::default()
        };
        let result = load_symbolic(&plan, &limits);
        if provider_bias == ADDRESS_LIMIT {
            assert!(matches!(result, Err(ProcessError::AddressOverflow)));
        } else {
            assert!(matches!(result, Err(ProcessError::InvalidSegment)));
        }
    }
}

#[test]
fn complete_patch_fields_cannot_cross_module_ownership_or_logical_gaps() {
    // Core retains a defense at the consumer boundary even though safe public
    // plans already enforce these rules and cannot contain caller-made patches.
    let ranges = [(0x10000, 0x10004, 0), (0x10004, 0x10010, 1)];
    assert_eq!(
        validate_session_patch(&ranges, 0, 0x10000),
        Err(ProcessError::InvalidSegment)
    );
    let gap = [(0x10000, 0x10004, 0), (0x10005, 0x10010, 0)];
    assert_eq!(
        validate_session_patch(&gap, 0, 0x10000),
        Err(ProcessError::InvalidSegment)
    );
    let adjacent = [(0x10000, 0x10004, 0), (0x10004, 0x10010, 0)];
    assert_eq!(validate_session_patch(&adjacent, 0, 0x10000), Ok(()));
}

fn inputs<'a>(consumer: &'a [u8], provider: &'a [u8]) -> [ModuleInput<'a>; 2] {
    [
        ModuleInput::new(open_executable(consumer).unwrap(), CONSUMER_BIAS),
        ModuleInput::new(open_executable(provider).unwrap(), PROVIDER_BIAS),
    ]
}

fn read_u64(memory: &GuestMemory, address: u64) -> u64 {
    let mut bytes = [0; 8];
    memory.read(address, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}

fn module(provider: bool, deferred: bool) -> Vec<u8> {
    let mut bytes = vec![0; if provider { 0x600 } else { 0x4200 }];
    bytes[..9].copy_from_slice(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 9, 2]);
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(
        &mut bytes,
        24,
        &(if provider { 0_u64 } else { 0x180 }).to_le_bytes(),
    );
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(
        &mut bytes,
        56,
        &(if provider { 2_u16 } else { 4 }).to_le_bytes(),
    );
    program(&mut bytes, 0, 1, 5, [0, 0, 0x600, PAGE_SIZE, PAGE_SIZE]);
    let dynamic_header = if provider { 1 } else { 2 };
    if !provider {
        program(
            &mut bytes,
            1,
            1,
            6,
            [0x4000, 0x4000, 0x200, PAGE_SIZE * 2, PAGE_SIZE],
        );
        program(
            &mut bytes,
            3,
            0x6474e552,
            4,
            [0, 0x4000, 0, PAGE_SIZE, PAGE_SIZE],
        );
        bytes[0x180] = 0xc3;
    } else {
        put(&mut bytes, PROVIDER_SYMBOL as usize, &PROVIDER_BYTES);
    }
    let mut strings = vec![0];
    let qualified = add_string(
        &mut strings,
        if provider {
            b"AAAAAAAAAAA#R#f"
        } else {
            b"AAAAAAAAAAA#B#C"
        },
    );
    let module_name = add_string(&mut strings, b"original-module");
    let library_name = add_string(&mut strings, b"original-library");
    put(&mut bytes, 0x200, &strings);
    let symbol = 0x260 + 24;
    put(&mut bytes, symbol, &qualified.to_le_bytes());
    bytes[symbol + 4] = 0x12; // GLOBAL FUNC, DEFAULT visibility.
    put(&mut bytes, symbol + 6, &(u16::from(provider)).to_le_bytes());
    put(
        &mut bytes,
        symbol + 8,
        &(if provider { PROVIDER_SYMBOL } else { 0 }).to_le_bytes(),
    );
    put(&mut bytes, symbol + 16, &8_u64.to_le_bytes());
    let mut entries = vec![
        (5_i64, 0x200),
        (10, strings.len() as u64),
        (6, 0x260),
        (11, 24),
        (39, 48),
    ];
    let (module_id, library_id, module_tag, library_tag) = if provider {
        (31_u16, 17_u16, 0x6100000d_i64, 0x61000013_i64)
    } else {
        (2, 1, 0x6100000f, 0x61000015)
    };
    entries.push((
        module_tag,
        u64::from(module_name) | (2_u64 << 32) | (1_u64 << 40) | (u64::from(module_id) << 48),
    ));
    entries.push((
        library_tag,
        u64::from(library_name) | (3_u64 << 32) | (u64::from(library_id) << 48),
    ));
    if !provider {
        let mut records = vec![
            (POINTER_SLOT, 1_u32, 1_u32, -8_i64),
            (POINTER_SLOT + 8, 1, 6, 0x7755),
            (POINTER_SLOT + 24, 0, 8, 0x1234),
            (BSS_SLOT, 0, 8, 0x2222),
        ];
        if deferred {
            records.push((POINTER_SLOT + 32, 0, 42, 7));
        }
        for (index, &(target, symbol, kind, addend)) in records.iter().enumerate() {
            relocation(&mut bytes, 0x400 + index * 24, target, symbol, kind, addend);
        }
        relocation(&mut bytes, 0x500, POINTER_SLOT + 16, 1, 7, -99);
        entries.extend([
            (7, 0x400),
            (8, records.len() as u64 * 24),
            (9, 24),
            (23, 0x500),
            (2, 24),
            (20, 7),
        ]);
        for target in [
            POINTER_SLOT,
            POINTER_SLOT + 8,
            POINTER_SLOT + 16,
            POINTER_SLOT + 24,
            POINTER_SLOT + 32,
        ] {
            put(&mut bytes, target as usize, &ORIGINAL.to_le_bytes());
        }
    }
    for (index, &(tag, value)) in entries.iter().chain(&[(0, 0)]).enumerate() {
        put(&mut bytes, 0x300 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x308 + index * 16, &value.to_le_bytes());
    }
    let dynamic_size = (entries.len() as u64 + 1) * 16;
    assert!(0x300 + dynamic_size <= 0x400);
    program(
        &mut bytes,
        dynamic_header,
        2,
        4,
        [0x300, 0x300, dynamic_size, dynamic_size, 8],
    );
    bytes
}

fn static_module(address: u64) -> Vec<u8> {
    let mut bytes = vec![0; 0x1001];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3_u16.to_le_bytes());
    put(&mut bytes, 18, &62_u16.to_le_bytes());
    put(&mut bytes, 20, &1_u32.to_le_bytes());
    put(&mut bytes, 24, &address.to_le_bytes());
    put(&mut bytes, 32, &64_u64.to_le_bytes());
    put(&mut bytes, 52, &64_u16.to_le_bytes());
    put(&mut bytes, 54, &56_u16.to_le_bytes());
    put(&mut bytes, 56, &1_u16.to_le_bytes());
    program(&mut bytes, 0, 1, 5, [0x1000, address, 1, 0x10, 1]);
    bytes[0x1000] = 0xc3;
    bytes
}

fn add_string(strings: &mut Vec<u8>, name: &[u8]) -> u32 {
    let offset = strings.len() as u32;
    strings.extend_from_slice(name);
    strings.push(0);
    offset
}

fn relocation(bytes: &mut [u8], at: usize, target: u64, symbol: u32, kind: u32, addend: i64) {
    put(bytes, at, &target.to_le_bytes());
    put(
        bytes,
        at + 8,
        &((u64::from(symbol) << 32) | u64::from(kind)).to_le_bytes(),
    );
    put(bytes, at + 16, &addend.to_le_bytes());
}

fn program(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, address, file_size, mem_size, alignment]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &address.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &alignment.to_le_bytes());
}

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
