// SPDX-License-Identifier: GPL-2.0-only
//! Original package-backed Trophy2 consumer, real saved unlocks and stack arg7.
use super::json_primitives::Program;
use super::*;

const CANARY: u64 = 0xa7a7_a7a7_a7a7_a7a7;
const TICK: u64 = 62_135_596_800_000_123;

pub(super) fn package() -> Input {
    let config = br#"{"defaultLanguage":"en-US","trophies":[{"id":2,"grade":"G","groupId":0},{"id":4,"grade":"B","groupId":0,"unlockCondition":{"progressive":true,"targetValue":5}}]}"#;
    let meta = br#"{"metadata":{"titleMetadata":{"name":"Original Trophy Fixture"},"trophyMetadata":[{"id":2,"name":"Gold","detail":"Original descriptor"},{"id":4,"name":"Steps"}],"groupMetadata":[{"id":0,"name":"Main"}]}}"#;
    let mut image = vec![0; 0xe0 + config.len() + meta.len()];
    put(&mut image, 0, &0xb228_c60a_u32.to_be_bytes());
    put(&mut image, 4, &1_u32.to_be_bytes());
    let length = image.len() as u64;
    put(&mut image, 8, &length.to_be_bytes());
    put(&mut image, 0x10, &2_u32.to_be_bytes());
    put(&mut image, 0x14, &0x40_u32.to_be_bytes());
    let mut offset = 0xe0;
    for (index, (name, bytes)) in [
        (b"tropconf.json".as_slice(), config.as_slice()),
        (b"tropmeta_en-US.json".as_slice(), meta.as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let at = 0x60 + index * 0x40;
        put(&mut image, at, name);
        put(&mut image, at + 0x20, &(offset as u64).to_be_bytes());
        put(&mut image, at + 0x28, &(bytes.len() as u64).to_be_bytes());
        put(&mut image, offset, bytes);
        offset += bytes.len();
    }
    create_input(image)
}
fn imm(p: &mut Program, register: usize, value: u64) {
    let (prefix, opcode) = [
        (0x48, 0xbf),
        (0x48, 0xbe),
        (0x48, 0xba),
        (0x48, 0xb9),
        (0x49, 0xb8),
        (0x49, 0xb9),
    ][register];
    p.code.extend([prefix, opcode]);
    p.code.extend(value.to_le_bytes());
}
fn pointer(p: &mut Program, register: usize, offset: u32) {
    let (prefix, modrm) = [
        (0x48, 0xbc),
        (0x48, 0xb4),
        (0x48, 0x94),
        (0x48, 0x8c),
        (0x4c, 0x84),
        (0x4c, 0x8c),
    ][register];
    p.code.extend([prefix, 0x8d, modrm, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn memory(p: &mut Program, offset: u32, value: u64, wide: bool) {
    if wide {
        p.code.push(0x48);
    }
    p.code.extend([0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.result(value);
}
fn call(p: &mut Program, nid: &'static [u8; 11], result: u64) {
    p.call_nid(nid);
    p.result(result);
}
fn context(p: &mut Program) {
    imm(p, 0, 1);
    imm(p, 1, u64::MAX);
}
fn seventh(p: &mut Program) {
    // Caller stack slot0 is calleeRSP+8 after CALL pushes its continuation.
    p.code.extend([0x48, 0x8d, 0x84, 0x24]);
    p.code.extend(160_u32.to_le_bytes());
    p.code.extend([0x48, 0x89, 0x04, 0x24]);
}
fn program(queries: bool) -> Input {
    let mut p = Program::new();
    p.code[4..8].copy_from_slice(&8192_u32.to_le_bytes());
    pointer(&mut p, 0, 0);
    p.code.extend([0x48, 0xb8]);
    p.code.extend(CANARY.to_le_bytes());
    p.code.push(0xb9);
    p.code.extend(1024_u32.to_le_bytes());
    p.code.extend([0xf3, 0x48, 0xab]);
    pointer(&mut p, 0, 64);
    call(&mut p, b"Gz1rmUZpROM", 0);
    p.dword(64, 1);
    pointer(&mut p, 0, 72);
    imm(&mut p, 1, 7);
    imm(&mut p, 2, 0);
    imm(&mut p, 3, 0);
    call(&mut p, b"Bagshr7OQ6Q", 0);
    p.dword(72, 1);
    context(&mut p);
    imm(&mut p, 2, 0);
    call(&mut p, b"bIDov3wBu5Q", 0);
    if queries {
        context(&mut p);
        pointer(&mut p, 2, 512);
        pointer(&mut p, 3, 3200);
        call(&mut p, b"4IzqhhUQ3nk", 0);
        memory(&mut p, 512, 2, false);
        memory(&mut p, 516, 2, false);
        memory(&mut p, 524, 1, false);
        memory(&mut p, 532, 1, false);
        memory(&mut p, 3200, 1, false);
        memory(&mut p, 3220, 85, false);
        memory(&mut p, 512 + 152, CANARY, true);
        context(&mut p);
        imm(&mut p, 2, 0);
        pointer(&mut p, 3, 512);
        pointer(&mut p, 4, 3200);
        call(&mut p, b"DoZWauG8mu0", 0);
        memory(&mut p, 512, 0, false);
        memory(&mut p, 3224, 85, false);
        context(&mut p);
        imm(&mut p, 2, 2);
        pointer(&mut p, 3, 512);
        pointer(&mut p, 4, 3200);
        call(&mut p, b"EwNylPdWUTM", 0);
        memory(&mut p, 512, 2, false);
        memory(&mut p, 516, 2, false);
        memory(&mut p, 3200, 2, false);
        memory(&mut p, 3204, 1, false);
        memory(&mut p, 3224, TICK, true);
        for nid in [b"+PDSI6WgPRc", b"y3zHpdZO6ME"] {
            context(&mut p);
            imm(&mut p, 2, 0);
            imm(&mut p, 3, 2);
            pointer(&mut p, 4, 512);
            pointer(&mut p, 5, 3200);
            seventh(&mut p);
            call(&mut p, nid, 0);
            memory(&mut p, 160, 2, false);
            if nid == b"+PDSI6WgPRc" {
                memory(&mut p, 512, u64::from(u32::MAX), false);
                memory(&mut p, 664, 0, false);
            } else {
                memory(&mut p, 512, 2, false);
                memory(&mut p, 1824, 4, false);
                memory(&mut p, 1824 + 16, 1, false);
                memory(&mut p, 1824 + 24, 5, true);
                memory(&mut p, 3232, 4, false);
                memory(&mut p, 3236, 0, false);
                memory(&mut p, 3240, 1, false);
                memory(&mut p, 3248, 0, true);
            }
        }
        context(&mut p);
        imm(&mut p, 2, u64::MAX);
        pointer(&mut p, 3, 160);
        call(&mut p, b"2QgUy+xJqS0", 0x8055_3911);
        memory(&mut p, 160, 0, true);
        for nid in [b"6IjXJUy6ZnA", b"-9LLVU0uvs8"] {
            context(&mut p);
            imm(&mut p, 2, u64::MAX);
            imm(&mut p, 3, u64::MAX);
            pointer(&mut p, 4, 160);
            call(&mut p, nid, 0x8055_3911);
            memory(&mut p, 160, 0, true);
        }
        imm(&mut p, 0, u64::MAX);
        imm(&mut p, 1, u64::MAX);
        call(&mut p, b"sUXGfNMalIo", 0);
        imm(&mut p, 0, u64::MAX);
        call(&mut p, b"fYapWA9xVmA", 0);
    }
    imm(&mut p, 0, 1);
    call(&mut p, b"d8P11CI40KE", 0);
    imm(&mut p, 0, 1);
    call(&mut p, b"sysY2FHYff4", 0);
    p.code.extend([0x48, 0x81, 0xc4]);
    p.code.extend(7936_u32.to_le_bytes());
    let scope = if queries {
        b"NpTrophy2".as_slice()
    } else {
        b"libSceNpTrophy2".as_slice()
    };
    p.finish(scope, scope)
}

#[test]
fn native_trophy2_registers_real_package_and_save_then_executes_all_source_queries() {
    let package = package();
    let directory = package.path.with_extension("trophy-save");
    std::fs::create_dir(&directory).unwrap();
    let saved = br#"{"unlockedTrophies":[2],"unlockedAt":{"2":62135596800000123}}"#;
    let save_path = directory.join("trophies-u7-s0.json");
    std::fs::write(&save_path, saved).unwrap();
    for queries in [true, false] {
        let input = program(queries);
        let mut binding = OsString::from("/app0/sce_sys/trophy2/trophy00.ucp=");
        binding.push(&package.path);
        let (status, fields, text) = probe_arguments(&[
            "--image".into(),
            input.path.clone().into(),
            "--data-file".into(),
            binding,
            "--save-directory".into(),
            directory.clone().into(),
        ]);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(
            fields["kernel_handles_live"], fields["kernel_handles_accounted"],
            "{text}"
        );
        // Source destruction retains one package and one per-user cache record.
        assert_eq!(fields["kernel_handles_live"], "2", "{text}");
        assert_eq!(fields["kernel_trophy_contexts"], "0", "{text}");
        assert_eq!(fields["kernel_trophy_registered_contexts"], "0", "{text}");
        assert_eq!(fields["kernel_trophy_packages"], "1", "{text}");
        assert_eq!(fields["kernel_trophy_users"], "1", "{text}");
        assert!(
            fields["kernel_handles_by_type"].contains("trophies:2"),
            "{text}"
        );
        assert_eq!(std::fs::read(&save_path).unwrap(), saved);
    }
    std::fs::remove_file(save_path).unwrap();
    std::fs::remove_file(directory.join(".pentax-save.lock")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
