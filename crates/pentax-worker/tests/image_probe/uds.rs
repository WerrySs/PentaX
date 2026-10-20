// SPDX-License-Identifier: GPL-2.0-only
//! Original native UDS events, shared Trophy2 state and real persistence.
use super::json_primitives::Program;
use super::*;

fn immediate(p: &mut Program, register: usize, value: u64) {
    let (prefix, opcode) = [(0x48, 0xbf), (0x48, 0xbe), (0x48, 0xba), (0x48, 0xb9)][register];
    p.code.extend([prefix, opcode]);
    p.code.extend(value.to_le_bytes());
}
fn pointer(p: &mut Program, register: usize, offset: u8) {
    p.code
        .extend([0x48, 0x8d, [0xbc, 0xb4, 0x94, 0x8c][register], 0x24]);
    p.code.extend(u32::from(offset).to_le_bytes());
}
fn token(p: &mut Program, register: usize, offset: u8) {
    p.code
        .extend([0x48, 0x8b, [0xbc, 0xb4, 0x94, 0x8c][register], 0x24]);
    p.code.extend(u32::from(offset).to_le_bytes());
}
fn store(p: &mut Program, offset: u32, value: u64) {
    p.code.extend([0x48, 0xb8]);
    p.code.extend(value.to_le_bytes());
    p.code.extend([0x48, 0x89, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
}
fn dword(p: &mut Program, offset: u32, value: u64) {
    p.code.extend([0x8b, 0x84, 0x24]);
    p.code.extend(offset.to_le_bytes());
    p.result(value);
}
fn call(p: &mut Program, nid: &'static [u8; 11]) {
    p.call_nid(nid);
    p.result(0);
}

/// Add a second exact namespace to the original ELF's dynamic metadata. Only
/// the known Trophy2 references are reassigned; unrelated imports are untouched.
fn mixed_scopes(p: Program) -> Input {
    let mut input = p.finish(b"NpUniversalDataSystem", b"NpUniversalDataSystem");
    let image = &mut input.bytes;
    let names_len = u64::from_le_bytes(image[0x1018..0x1020].try_into().unwrap()) as usize;
    let symbol_count = u64::from_le_bytes(image[0x1038..0x1040].try_into().unwrap()) as usize / 24;
    let trophy = [
        b"Bagshr7OQ6Q",
        b"bIDov3wBu5Q",
        b"EwNylPdWUTM",
        b"sysY2FHYff4",
    ];
    for index in 1..symbol_count {
        let at = 0x1800 + index * 24;
        let name = u32::from_le_bytes(image[at..at + 4].try_into().unwrap()) as usize;
        if trophy
            .iter()
            .any(|nid| image[0x1200 + name..0x1200 + name + 11] == **nid)
        {
            put(image, 0x1200 + name + 11, b"#F#E");
        }
    }
    put(image, 0x1200 + names_len, b"NpTrophy2\0NpTrophy2\0");
    put(image, 0x1018, &((names_len + 20) as u64).to_le_bytes());
    for (index, tag, value) in [
        (
            10,
            0x6100_000f_i64,
            names_len as u64 | (1 << 32) | (1 << 40) | (4 << 48),
        ),
        (
            11,
            0x6100_0015_i64,
            (names_len + 10) as u64 | (1 << 32) | (5 << 48),
        ),
        (12, 0, 0),
    ] {
        put(image, 0x1000 + index * 16, &tag.to_le_bytes());
        put(image, 0x1008 + index * 16, &value.to_le_bytes());
    }
    for at in [152, 160] {
        put(image, at, &(13_u64 * 16).to_le_bytes());
    }
    std::fs::write(&input.path, image).unwrap();
    input
}

fn event_program() -> Input {
    let mut p = Program::new();
    p.store(0, 16);
    p.store(8, 1024);
    pointer(&mut p, 0, 0);
    call(&mut p, b"sjaobBgqeB4");
    pointer(&mut p, 0, 32);
    immediate(&mut p, 1, 7);
    immediate(&mut p, 2, 0);
    immediate(&mut p, 3, 0);
    call(&mut p, b"5zBnau1uIEo");
    p.dword(32, 1);
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 0);
    call(&mut p, b"tpFJ8LIKvPw");
    pointer(&mut p, 0, 36);
    call(&mut p, b"hT0IAEvN+M0");
    p.dword(36, 1);
    pointer(&mut p, 0, 40);
    immediate(&mut p, 1, 7);
    immediate(&mut p, 2, 0);
    immediate(&mut p, 3, 0);
    call(&mut p, b"Bagshr7OQ6Q");
    p.dword(40, 1);
    pointer(&mut p, 0, 48);
    call(&mut p, b"s6W4Zl4Slgk");
    store(&mut p, 128, u64::from_le_bytes(*b"_trophy_"));
    store(&mut p, 136, u64::from_le_bytes(*b"id\0\0\0\0\0\0"));
    store(&mut p, 144, u64::from_le_bytes(*b"_UnlockT"));
    store(&mut p, 152, u64::from_le_bytes(*b"rophy\0\0\0"));
    token(&mut p, 0, 48);
    pointer(&mut p, 1, 128);
    immediate(&mut p, 2, 2);
    call(&mut p, b"YE4dbtbz6OE");
    pointer(&mut p, 0, 144);
    token(&mut p, 1, 48);
    pointer(&mut p, 2, 56);
    pointer(&mut p, 3, 64);
    call(&mut p, b"p+GcLqwpL9M");
    // Source int64 setter erases the original key, leaving the event's deep
    // copy intact. Its int32 key must still unlock the real package trophy.
    token(&mut p, 0, 48);
    pointer(&mut p, 1, 128);
    immediate(&mut p, 2, 2);
    call(&mut p, b"56QLTqx911s");
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    token(&mut p, 2, 56);
    immediate(&mut p, 3, u64::MAX);
    call(&mut p, b"CzkKf7ahIyU");
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 0);
    call(&mut p, b"bIDov3wBu5Q");
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 2);
    immediate(&mut p, 3, 0);
    // SysV argument5 is R8, independent of the original RCX details pointer.
    p.code.extend([0x4c, 0x8d, 0x84, 0x24]);
    p.code.extend(192_u32.to_le_bytes());
    immediate(&mut p, 3, 0);
    call(&mut p, b"EwNylPdWUTM");
    dword(&mut p, 192, 2);
    dword(&mut p, 196, 1);
    // Timestamp is real UTC microseconds in the source epoch, never a dummy.
    token(&mut p, 0, 216);
    immediate(&mut p, 1, 62_135_596_800_000_000);
    p.code
        .extend([0x48, 0x39, 0xf7, 0x0f, 0x97, 0xc0, 0x0f, 0xb6, 0xc0]);
    p.result(1);
    token(&mut p, 0, 56);
    pointer(&mut p, 1, 88);
    call(&mut p, b"+s14jq-KGYw");
    p.word(88, 3);
    p.store(72, 0xa5a5_a5a5_a5a5_a5a5);
    p.store(80, 0xa5a5_a5a5_a5a5_a5a5);
    p.store(96, 0xa5a5_a5a5_a5a5_a5a5);
    token(&mut p, 0, 56);
    pointer(&mut p, 1, 80);
    immediate(&mut p, 2, 3);
    pointer(&mut p, 3, 88);
    call(&mut p, b"vj6CQGWtEBg");
    p.word(72, 0xa5a5_a5a5_a5a5_a5a5);
    p.word(80, 0xa5a5_a5a5_a500_7d7b);
    p.word(88, 3);
    p.word(96, 0xa5a5_a5a5_a5a5_a5a5);
    token(&mut p, 0, 48);
    call(&mut p, b"kKUH0Viib3c");
    token(&mut p, 0, 56);
    call(&mut p, b"wG+84pnNIuo");
    immediate(&mut p, 0, 1);
    call(&mut p, b"wB7IWzGp2v0");
    immediate(&mut p, 0, 1);
    call(&mut p, b"sysY2FHYff4");
    immediate(&mut p, 0, 1);
    call(&mut p, b"AUIHb7jUX3I");
    call(&mut p, b"47UAEuQl+iI");
    mixed_scopes(p)
}

fn prefixed_context_program() -> Input {
    let mut p = Program::new();
    p.store(0, 16);
    p.store(8, 4096);
    pointer(&mut p, 0, 0);
    call(&mut p, b"sjaobBgqeB4");
    pointer(&mut p, 0, 32);
    immediate(&mut p, 1, 7);
    immediate(&mut p, 2, 0);
    immediate(&mut p, 3, 0);
    call(&mut p, b"5zBnau1uIEo");
    p.dword(32, 1);
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 0);
    call(&mut p, b"tpFJ8LIKvPw");
    // Source registration requires no package capability. Terminate retains
    // the live context, which remains valid for the following real destruction.
    call(&mut p, b"47UAEuQl+iI");
    immediate(&mut p, 0, 1);
    call(&mut p, b"wB7IWzGp2v0");
    p.finish(
        b"libSceNpUniversalDataSystem",
        b"libSceNpUniversalDataSystem",
    )
}

fn persisted_query_program(tick: u64) -> Input {
    let mut p = Program::new();
    pointer(&mut p, 0, 40);
    immediate(&mut p, 1, 7);
    immediate(&mut p, 2, 0);
    immediate(&mut p, 3, 0);
    call(&mut p, b"Bagshr7OQ6Q");
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 0);
    call(&mut p, b"bIDov3wBu5Q");
    immediate(&mut p, 0, 1);
    immediate(&mut p, 1, 1);
    immediate(&mut p, 2, 2);
    immediate(&mut p, 3, 0);
    p.code.extend([0x4c, 0x8d, 0x84, 0x24]);
    p.code.extend(192_u32.to_le_bytes());
    call(&mut p, b"EwNylPdWUTM");
    dword(&mut p, 196, 1);
    p.code.extend([0x48, 0x8b, 0x84, 0x24]);
    p.code.extend(216_u32.to_le_bytes());
    p.result(tick);
    immediate(&mut p, 0, 1);
    call(&mut p, b"sysY2FHYff4");
    p.finish(b"NpTrophy2", b"NpTrophy2")
}

#[test]
fn native_uds_event_unlocks_real_shared_trophy_state_and_persists_after_owned_cleanup() {
    let package = super::trophy2::package();
    let directory = package.path.with_extension("uds-save");
    std::fs::create_dir(&directory).unwrap();
    let input = event_program();
    let mut binding = OsString::from("/app0/sce_sys/trophy2/trophy00.ucp=");
    binding.push(&package.path);
    let source_tick = || {
        62_135_596_800_000_000_u64
            + u64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_micros(),
            )
            .unwrap()
    };
    let earliest = source_tick();
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        input.path.clone().into(),
        "--data-file".into(),
        binding.clone(),
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
    assert_eq!(fields["kernel_handles_live"], "2", "{text}");
    assert!(
        fields["kernel_handles_by_type"].contains("trophies:2"),
        "{text}"
    );
    let saved = std::fs::read_to_string(directory.join("trophies-u7-s0.json")).unwrap();
    assert!(saved.contains("\"unlockedTrophies\":[2]"), "{saved}");
    assert!(saved.contains("\"unlockedAt\":{\"2\":"), "{saved}");
    let tick: u64 = saved
        .split_once("\"unlockedAt\":{\"2\":")
        .unwrap()
        .1
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .unwrap();
    assert!(
        (earliest..=source_tick()).contains(&tick),
        "timestamp must be inside the actual run interval"
    );
    let reload = persisted_query_program(tick);
    let (status, fields, text) = probe_arguments(&[
        "--image".into(),
        reload.path.clone().into(),
        "--data-file".into(),
        binding,
        "--save-directory".into(),
        directory.clone().into(),
    ]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(
        fields["kernel_handles_live"], fields["kernel_handles_accounted"],
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("trophies-u7-s0.json")).unwrap(),
        saved
    );
    // These observed prefixed marker IDs are above catalogue index1024.
    let prefixed = prefixed_context_program();
    let (status, fields, text) = probe_arguments(&["--image".into(), prefixed.path.clone().into()]);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42", "{text}");
    assert_eq!(fields["unavailable_functions"], "0", "{text}");
    assert_eq!(fields["kernel_handles_live"], "0", "{text}");
    assert_eq!(fields["kernel_handles_accounted"], "0", "{text}");
    std::fs::remove_file(directory.join("trophies-u7-s0.json")).unwrap();
    std::fs::remove_file(directory.join(".pentax-save.lock")).unwrap();
    std::fs::remove_dir(directory).unwrap();
}
