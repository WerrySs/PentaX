// SPDX-License-Identifier: GPL-2.0-only

//! Original native consumer for the pinned Net, Ssl and Http startup profile.

use super::*;

const IMPORTS: [(&str, usize); 14] = [
    ("Nlev7Lg8k3A", 0), // NetInit
    ("HQOwnfMGipQ", 0), // NetGetErrorAddr
    ("dgJBaeJnGpo", 0), // NetPoolCreate
    ("K7RlrTkI-mw", 0), // NetPoolDestroy
    ("C4UgDHHPvdw", 0), // NetResolverCreate
    ("kJlYH5uMAWI", 0), // NetResolverDestroy
    ("AzqoBha7js4", 0), // NetResolverAbort
    ("hdpVEUDFW3s", 1), // SslInit
    ("0K1yQ6Lv-Yc", 1), // SslTerm
    ("TDfQqO-gMbY", 1), // SslGetCaCerts
    ("qIvLs0gYxi0", 1), // SslFreeCaCerts
    ("A9cVMUtEp4Y", 2), // HttpInit
    ("Ik-KpLTlf7Q", 2), // HttpTerm
    ("9BcDykPmo1I", 3), // Posix errno location
];

fn mixed_network_image(code: &[u8], ssl_lifecycle_major: u64) -> Input {
    assert!(matches!(ssl_lifecycle_major, 1 | 2));
    // These tables deliberately accommodate all14 imports without overlapping
    // one another, the original code at0x800, or Program's GOT at0xf00.
    const STRINGS: usize = 0x1400;
    const SYMBOLS: usize = 0x1800;
    const RELOCATIONS: usize = 0x2000;
    let mut bytes = module_bytes(&[], true, None);
    bytes.resize(0x2400, 0);
    bytes[0x200..0x800].fill(0);
    put(&mut bytes, 24, &0x800_u64.to_le_bytes());
    let file_size = bytes.len() as u64;
    put(&mut bytes, 96, &file_size.to_le_bytes());
    put(&mut bytes, 0x800, code);
    put(&mut bytes, 0x1300, &[0xff, 0x80, 0]); // Opaque source name, not UTF-8.

    let mut names = vec![0];
    let mut scopes = Vec::new();
    let mut scope_names = vec![
        ("libSceNet", "libSceNet"),
        ("libSceSsl", "libSceSsl"),
        ("libSceHttp", "libSceHttp"),
        ("libkernel", "libkernel"), // POSIX errno export belongs to libkernel.
    ];
    if ssl_lifecycle_major == 2 {
        // Only lifecycle imports use this separate2.1 module descriptor.
        // Both certificate imports retain the original SSL1.1 declaration.
        scope_names.push(("libSceSsl", "libSceSsl"));
    }
    for (module, library) in scope_names {
        let module_offset = names.len() as u64;
        names.extend_from_slice(module.as_bytes());
        names.push(0);
        let library_offset = names.len() as u64;
        names.extend_from_slice(library.as_bytes());
        names.push(0);
        scopes.push((module_offset, library_offset));
    }
    for (index, (nid, scope)) in IMPORTS.iter().enumerate() {
        let scope = if ssl_lifecycle_major == 2 && matches!(index, 7 | 8) {
            4
        } else {
            *scope
        };
        let symbol = SYMBOLS + (index + 1) * 24;
        put(&mut bytes, symbol, &(names.len() as u32).to_le_bytes());
        bytes[symbol + 4] = 0x12; // Undefined GLOBAL FUNC.
        let module_id = char::from(b'C' + scope as u8 * 2);
        let library_id = char::from(b'D' + scope as u8 * 2);
        names.extend_from_slice(format!("{nid}#{library_id}#{module_id}\0").as_bytes());
        put(
            &mut bytes,
            RELOCATIONS + index * 24,
            &(0xf00_u64 + index as u64 * 8).to_le_bytes(),
        );
        put(
            &mut bytes,
            RELOCATIONS + index * 24 + 8,
            &(((index as u64 + 1) << 32) | 7).to_le_bytes(),
        );
    }
    assert!(STRINGS + names.len() <= SYMBOLS);
    assert!(SYMBOLS + (IMPORTS.len() + 1) * 24 <= RELOCATIONS);
    put(&mut bytes, STRINGS, &names);
    let mut dynamic = vec![
        (5_i64, STRINGS as u64),
        (10, names.len() as u64),
        (6, SYMBOLS as u64),
        (39, (IMPORTS.len() as u64 + 1) * 24),
        (11, 24),
    ];
    for (index, (module, library)) in scopes.into_iter().enumerate() {
        let module_id = 2 + index as u64 * 2;
        let major = if index == 4 { 2 } else { 1 };
        dynamic.push((
            0x6100_000f,
            module | (1 << 32) | (major << 40) | (module_id << 48),
        ));
        dynamic.push((0x6100_0015, library | (1 << 32) | ((module_id + 1) << 48)));
    }
    dynamic.extend([
        (7, RELOCATIONS as u64),
        (8, IMPORTS.len() as u64 * 24),
        (9, 24),
        (0, 0),
    ]);
    for offset in [152, 160] {
        put(
            &mut bytes,
            offset,
            &((dynamic.len() * 16) as u64).to_le_bytes(),
        );
    }
    for (index, (tag, value)) in dynamic.into_iter().enumerate() {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    create_input(bytes)
}

fn set_cell(p: &mut Program, slot: u8, value: u32) {
    p.load(11, slot);
    p.code.extend([0x41, 0xc7, 0x03]); // MOV dword [R11],imm32.
    p.code.extend(value.to_le_bytes());
}

fn cell(p: &mut Program, slot: u8, value: u32) {
    p.load(11, slot);
    p.code.extend([0x41, 0x8b, 0x03]); // MOV EAX,[R11], including zero extension.
    p.expect(u64::from(value));
}

fn same_cell(p: &mut Program, slot: u8) {
    p.load(10, slot);
    p.code.extend([0x4c, 0x39, 0xd0]); // Returned RAX must retain this exact cell.
    checked(&mut p.code, &mut p.branches);
}

#[test]
fn native_network_startup_preserves_errno_domains_exact_outputs_and_independent_resource_lifetimes()
{
    let mut p = Program::new();
    p.call(1);
    p.save(0);
    cell(&mut p, 0, 0);
    set_cell(&mut p, 0, 77);
    p.call(13);
    p.save(8);
    p.load(10, 0);
    p.code.extend([0x4c, 0x39, 0xd0, 0x0f, 0x84, 0, 0, 0, 0]);
    p.branches.push(p.code.len() - 4); // Equal Net/POSIX pointers fail.
    cell(&mut p, 8, 0);
    set_cell(&mut p, 8, 55);
    for _ in 0..2 {
        p.call(0); // Actual host backend initialization; no sockets/connections.
        p.expect(0);
    }
    cell(&mut p, 0, 77);
    cell(&mut p, 8, 55);

    p.path(7, 0x1300);
    p.reg(6, u64::MAX); // Source signed32 size=-1 is nonzero and admitted.
    p.reg(2, 1_u64 << 32); // Source flags use the low32 bits only.
    p.call(2);
    p.expect(1);
    p.path(7, 0x1300);
    p.reg(6, 1);
    p.reg(2, 1_u64 << 32);
    p.call(4);
    p.expect(0); // Resolver slot0 is a real successful resource.
    p.reg(7, u64::MAX);
    p.call(7);
    p.expect(0x2_0000);
    p.reg(7, 1);
    p.reg(6, 0x2_0000);
    p.reg(2, u64::MAX);
    p.call(11);
    p.expect(0x1_0000);

    p.reg(0, u64::MAX);
    for offset in [32, 40, 48, 56] {
        p.save(offset);
    }
    p.reg(7, 0x2_0000);
    p.ptr(6, 32);
    p.call(9);
    p.expect(0x8095_f004);
    for offset in [32, 40, 48] {
        p.word(offset, 0);
    }
    p.word(56, u64::MAX); // Exact24-byte write, no adjacent clobber.
    p.reg(0, u64::MAX);
    for offset in [32, 40, 48] {
        p.save(offset);
    }
    p.reg(7, 0x2_0000);
    p.ptr(6, 32);
    p.call(10);
    p.expect(0);
    for offset in [32, 40, 48] {
        p.word(offset, 0);
    }
    p.word(56, u64::MAX);

    p.reg(7, 1);
    p.call(3);
    p.expect(0); // Removing the pool does not cascade to its resolver or HTTP.
    p.reg(7, 1);
    p.call(3);
    p.expect(0x8041_0109); // Source return is a zero-extended32-bit error word.
    cell(&mut p, 0, 9);
    p.reg(7, 0);
    p.call(5);
    p.expect(0);
    p.reg(7, u64::MAX);
    p.reg(6, u64::MAX);
    p.call(6);
    p.expect(0); // Explicit pinned Abort no-op, no cancellation claim.
    p.reg(7, 0x2_0000);
    p.call(8);
    p.expect(0); // HTTP remains owned after its SSL parent is removed.
    for expected in [0, 0x8043_1100] {
        p.reg(7, 0x1_0000);
        p.call(12);
        p.expect(expected);
    }
    p.call(1);
    same_cell(&mut p, 0);
    cell(&mut p, 0, 9); // HTTP errors do not overwrite Net errno.
    p.call(13);
    same_cell(&mut p, 8);
    cell(&mut p, 8, 55);

    let (code, calls) = p.finish(None);
    for ssl_lifecycle_major in [1, 2] {
        let input = mixed_network_image(&code, ssl_lifecycle_major);
        let (status, fields, text) = probe(&input);
        let text = format!("SSL lifecycle {ssl_lifecycle_major}.1: {text}");
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42", "{text}");
        assert_eq!(fields["unavailable_functions"], "0", "{text}");
        assert_eq!(fields["service_stops"], calls.to_string(), "{text}");
        assert_eq!(fields["kernel_handles_live"], "2", "{text}");
        assert_eq!(fields["kernel_handles_accounted"], "2", "{text}");
        assert_eq!(
            fields["kernel_handles_by_type"],
            "memory:2,files:0,modules:0,sync:0,events:0,network:0,ampr:0,keys:0,semaphores:0,fibers:0",
            "{text}"
        );
        assert!(
            fields["primary_native_instructions"]
                .parse::<u64>()
                .unwrap()
                > 0
        );
    }
}
