// SPDX-License-Identifier: GPL-2.0-only

//! Original guest consumes real physical aliases through the linked kernel ABI.
use super::*;

#[test]
fn native_gpu_only_mapping_keeps_cpu_reads_denied() {
    let mut p = Program::new();
    p.set(8, 0);
    p.ptr(7, 8);
    p.reg(6, 0x4000);
    p.reg(2, 0x30);
    p.reg(1, 0);
    p.call(0);
    p.expect(0);
    p.load(7, 8);
    p.code.extend([0xc5, 0xf9, 0x6f, 0x07]);
    let (code, _) = p.finish(None);
    let original = image(&code, &["IWIBBdTHit4"]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 2, "{text}");
    assert_eq!(fields["runtime_failure"], "memory-permission");
    assert_eq!(fields["dispatch_cpu_access"], "none");
    assert_eq!(fields["dispatch_gpu_access"], "read-write");
    assert!(!fields.contains_key("value"));
}

#[test]
fn native_cpu_write_mapping_supports_x86_vector_reads_without_changing_reported_protection() {
    let mut p = Program::new();
    p.reg(7, 0x4000);
    p.reg(6, 0);
    p.reg(2, 3);
    p.ptr(1, 0);
    p.call(0);
    p.expect(0);
    p.set(8, 0);
    p.ptr(7, 8);
    p.reg(6, 0x4000);
    p.reg(2, 0x32);
    p.reg(1, 0);
    p.load(8, 0);
    p.reg(9, 0);
    p.call(1);
    p.expect(0);
    p.load(7, 8);
    p.reg(0, 42);
    p.code.extend([0x48, 0x89, 0x07]);
    p.code.extend([0xc5, 0xf9, 0x6f, 0x07]); // VMOVDQA XMM0,[RDI], CPU write implies read.
    p.code.extend([0xc5, 0xf9, 0x7f, 0x44, 0x24, 16]);
    p.word(16, 42);
    p.word(24, 0);
    p.reg(0, 0xa5a5_a5a5_a5a5_a5a5);
    p.save(32);
    p.load(7, 8);
    p.reg(6, 0);
    p.reg(2, 0);
    p.ptr(1, 32);
    p.call(2);
    p.expect(0);
    p.word(32, 0xa5a5_a5a5_0000_0032);
    let (code, calls) = p.finish(None);
    let original = image(&code, &["B+vc2AO2Zrc", "L-Q3LEjIbgA", "WFcfL2lzido"]);
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    assert_modeled_vector_operations(&fields, ["2", "0", "0"]);
}

const PAGE: u32 = 0x4000;

#[test]
fn runtime_memory_profile_allocates_maps_and_reads_beyond_compact_capacity() {
    runtime_memory_consumer(None);
}

struct BackingArea(PathBuf);
impl BackingArea {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax-private-backing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for BackingArea {
    fn drop(&mut self) {
        assert_eq!(std::fs::read_dir(&self.0).unwrap().count(), 0);
        std::fs::remove_dir(&self.0).unwrap();
    }
}

#[test]
fn file_backed_runtime_consumes_real_large_mapping_and_alias_then_deletes_files() {
    let area = BackingArea::new();
    runtime_memory_consumer(Some(&area.0));
}

fn runtime_memory_consumer(backing: Option<&PathBuf>) {
    const LENGTH: u32 = 128 * 1024 * 1024;
    let mut p = Program::new();
    p.call(0); // Exact public KernelGetDirectMemorySize identity below.
    p.expect(12_800 * 1024 * 1024);
    p.reg(7, u64::from(LENGTH));
    p.reg(6, 0);
    p.reg(2, 3);
    p.ptr(1, 0);
    p.call(1);
    p.expect(0);
    p.word(0, 0);
    p.set(8, 0);
    p.ptr(7, 8);
    p.reg(6, u64::from(LENGTH));
    p.reg(2, 3);
    p.reg(1, 0);
    p.load(8, 0);
    p.reg(9, 0);
    p.call(2);
    p.expect(0);
    // Touch only the endpoints, not the entire backing, and consume real data.
    write(&mut p, 8, 0, 37);
    write(&mut p, 8, LENGTH - 8, 42);
    read(&mut p, 8, 0, 37);
    read(&mut p, 8, LENGTH - 8, 42);
    p.set(16, 0);
    p.ptr(7, 16);
    p.reg(6, u64::from(PAGE));
    p.reg(2, 3);
    p.reg(1, 0);
    p.reg(8, u64::from(LENGTH - PAGE));
    p.reg(9, 0);
    p.call(2);
    p.expect(0);
    read(&mut p, 16, PAGE - 8, 42);
    p.load(7, 0);
    p.reg(6, u64::from(LENGTH));
    p.call(3);
    p.expect(0);
    let (code, calls) = p.finish(None);
    let original = image(
        &code,
        &["pO96TwzOm5E", "B+vc2AO2Zrc", "L-Q3LEjIbgA", "MBuItvba6z8"],
    );
    let mut arguments = vec![
        "--image".into(),
        original.path.as_os_str().to_owned(),
        "--memory-profile".into(),
        "runtime".into(),
    ];
    if let Some(path) = backing {
        arguments.extend([
            "--physical-backing-directory".into(),
            path.as_os_str().to_owned(),
        ]);
    }
    let (status, fields, text) = probe_arguments(&arguments);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

#[test]
fn file_backing_requires_runtime_and_rejects_source_data_and_save_overlap() {
    let area = BackingArea::new();
    let original = image(&[0xb8, 42, 0, 0, 0, 0xc3], &[]);
    let base = vec![
        "--image".into(),
        original.path.as_os_str().to_owned(),
        "--physical-backing-directory".into(),
        area.0.as_os_str().to_owned(),
    ];
    let (status, _, _) = probe_arguments(&base);
    assert_ne!(status, 0);
    for selected in ["--data-directory", "--save-directory", "--image"] {
        let mut args = base.clone();
        args.extend(["--memory-profile".into(), "runtime".into()]);
        if selected == "--image" {
            // Its parent contains the explicit source, so even this ancestor
            // cannot be repurposed as writable backing authority.
            args[3] = original.path.parent().unwrap().as_os_str().to_owned();
        } else {
            let mut binding = OsString::from("/app0=");
            binding.push(&area.0);
            args.extend([
                selected.into(),
                if selected == "--data-directory" {
                    binding
                } else {
                    area.0.as_os_str().to_owned()
                },
            ]);
        }
        let (status, fields, text) = probe_arguments(&args);
        assert_eq!(status, 2, "{text}");
        assert_eq!(fields["code"], "backing-input-overlap", "{text}");
    }
}

fn pointer(p: &mut Program, slot: u8, offset: u32) {
    p.load(7, slot);
    p.code.extend([0x48, 0x81, 0xc7]); // ADD RDI, imm32
    p.code.extend(offset.to_le_bytes());
}
fn allocate(p: &mut Program) {
    p.reg(7, u64::from(3 * PAGE));
    p.reg(6, 0);
    p.reg(2, 3);
    p.ptr(1, 0);
    p.call(0);
    p.expect(0);
    p.word(0, 0);
}
fn map(p: &mut Program, slot: u8) {
    p.ptr(7, slot);
    p.reg(6, u64::from(2 * PAGE));
    p.reg(2, 3);
    p.reg(1, 0);
    p.reg(8, u64::from(PAGE)); // A non-64 KiB physical offset.
    p.reg(9, 0);
    p.call(1);
    p.expect(0);
}
fn unmap(p: &mut Program, slot: u8) {
    p.load(7, slot);
    p.reg(6, u64::from(2 * PAGE));
    p.call(2);
    p.expect(0);
}
fn release(p: &mut Program) {
    p.load(7, 0);
    p.reg(6, u64::from(3 * PAGE));
    p.call(3);
    p.expect(0);
}
fn write(p: &mut Program, slot: u8, offset: u32, value: u32) {
    pointer(p, slot, offset);
    p.code.extend([0x48, 0xc7, 0x07]);
    p.code.extend(value.to_le_bytes());
}
fn read(p: &mut Program, slot: u8, offset: u32, value: u64) {
    pointer(p, slot, offset);
    p.code.extend([0x48, 0x8b, 0x07]);
    p.expect(value);
}

#[test]
fn guest_physical_memory_aliases_share_native_writes_and_persist_until_real_release() {
    let mut p = Program::new();
    p.set(8, 0);
    p.set(16, 0);
    allocate(&mut p);
    map(&mut p, 8);
    map(&mut p, 16);
    write(&mut p, 8, 0, 33);
    write(&mut p, 8, PAGE, 42);
    read(&mut p, 16, 0, 33);
    read(&mut p, 16, PAGE, 42);
    pointer(&mut p, 16, 1);
    p.reg(6, 1);
    p.reg(2, 1);
    p.call(4);
    p.expect(0);
    write(&mut p, 8, 0, 37);
    read(&mut p, 16, 0, 37);
    unmap(&mut p, 8);
    map(&mut p, 8);
    read(&mut p, 8, 0, 37);
    read(&mut p, 8, PAGE, 42);
    unmap(&mut p, 8);
    unmap(&mut p, 16);
    release(&mut p);
    allocate(&mut p);
    map(&mut p, 8);
    read(&mut p, 8, 0, 0);
    read(&mut p, 8, PAGE, 0);
    unmap(&mut p, 8);
    release(&mut p);
    let (code, calls) = p.finish(None);
    let original = image(
        &code,
        &[
            "B+vc2AO2Zrc",
            "L-Q3LEjIbgA",
            "cQke9UuBQOk",
            "MBuItvba6z8",
            "vSMAm3cxYTY",
        ],
    );
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}

fn query_hole(p: &mut Program, slot: u8, offset: u32) {
    pointer(p, slot, offset);
    for reg in [6, 2, 1] {
        p.reg(reg, 0);
    }
    p.call(5);
    p.expect(0x8002_000d);
}
fn map_piece(p: &mut Program, original: u8, output: u8) {
    pointer(p, original, PAGE);
    p.stack(0x89, 7, output);
    p.ptr(7, output);
    p.reg(6, u64::from(PAGE));
    p.reg(2, 3);
    p.reg(1, 0);
    p.load(8, 24);
    p.reg(9, 0);
    p.call(1);
    p.expect(0);
}

#[test]
fn guest_physical_memory_partial_release_updates_alias_queries_and_reuses_the_freed_range() {
    let mut p = Program::new();
    p.set(8, 0);
    p.set(16, 0);
    allocate(&mut p);
    map(&mut p, 8);
    map(&mut p, 16);
    write(&mut p, 8, 0, 37);
    write(&mut p, 8, PAGE, 42);
    p.reg(7, u64::from(2 * PAGE));
    p.reg(6, u64::from(PAGE));
    p.call(3);
    p.expect(0);
    query_hole(&mut p, 8, PAGE);
    read(&mut p, 8, 0, 37);
    read(&mut p, 16, 0, 37);
    p.reg(7, u64::from(PAGE));
    p.reg(6, 0);
    p.reg(2, 3);
    p.ptr(1, 24);
    p.call(0);
    p.expect(0);
    p.word(24, u64::from(2 * PAGE));
    map_piece(&mut p, 8, 32);
    read(&mut p, 8, PAGE, 0);
    write(&mut p, 8, PAGE, 99);
    map_piece(&mut p, 16, 40);
    read(&mut p, 16, PAGE, 99);
    release(&mut p); // Active aliases span two physical pieces and two OS sections.
    query_hole(&mut p, 8, 0);
    allocate(&mut p);
    map(&mut p, 8);
    read(&mut p, 8, 0, 0);
    read(&mut p, 8, PAGE, 0);
    let (code, calls) = p.finish(None);
    let original = image(
        &code,
        &[
            "B+vc2AO2Zrc",
            "L-Q3LEjIbgA",
            "cQke9UuBQOk",
            "MBuItvba6z8",
            "vSMAm3cxYTY",
            "WFcfL2lzido",
        ],
    );
    let (status, fields, text) = probe(&original);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
}
