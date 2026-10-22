// SPDX-License-Identifier: GPL-2.0-only
//! Original native APR consumers over an explicitly supplied three-byte file.
use super::*;

fn run(main: &Input, data: &Input) -> (i32, HashMap<String, String>, String) {
    let mut binding = OsString::from(PATH);
    binding.push("=");
    binding.push(&data.path);
    probe_arguments(&[
        "--image".into(),
        main.path.clone().into(),
        "--data-file".into(),
        binding,
    ])
}

fn strings(input: &mut Input, prefixed: bool) {
    if prefixed {
        put(&mut input.bytes, NAME as usize, b"pentax-results.bin\0");
        put(
            &mut input.bytes,
            MISSING as usize,
            b"missing-original.bin\0",
        );
        put(&mut input.bytes, 0xdc0, b"/app0/assets/\0");
        std::fs::write(&input.path, &input.bytes).unwrap();
    }
}

fn list(p: &mut Program) {
    for (offset, string) in [(0, NAME), (8, MISSING), (16, NAME)] {
        p.path(0, string);
        p.save(offset);
    }
    p.reg(0, 0);
    for offset in [32, 40, 80, 88, 104] {
        p.save(offset);
    }
    p.reg(0, 85);
    for offset in [56, 64, 72] {
        p.save(offset);
    }
}

#[test]
fn native_apr_ids_stat_and_cached_size_execute_real_capability_metadata_queries() {
    let mut p = Program::new();
    p.path(0, NAME);
    p.save(120);
    p.reg(0, 0);
    p.save(112);
    p.ptr(7, 120);
    p.reg(6, (31 << 32) | 1);
    p.ptr(2, 112);
    p.reg(1, 0);
    p.call(0);
    p.expect(0);
    p.word(112, 1);
    p.reg(7, (9 << 32) | 1);
    p.ptr(6, 120);
    p.call(1);
    p.expect(0);
    p.word(120, 3);
    p.reg(7, 1);
    p.ptr(6, 0);
    p.call(2);
    p.expect(0);
    p.word(8, 0o100777);
    p.word(72, 3);
    p.word(80, 1);
    p.word(88, 512);
    p.reg(7, 99);
    p.ptr(6, 120);
    p.call(1);
    p.expect(u64::from(u32::MAX));
    p.word(120, 3); // Unknown identities do not overwrite the size.
    let (code, calls) = p.finish(None);
    let main = image(&code, &["WT-5NKy42fw", "WvEu7yl3Ivg", "ApkYaHb8Sek"]);
    let data = create_input(vec![37, 42, 99]);
    let (status, fields, text) = run(&main, &data);
    assert_eq!(status, 0, "{text}");
    assert_eq!(fields["value"], "42");
    assert_eq!(fields["service_stops"], calls.to_string());
    println!("native-apr-identity-stat-size: one program, four calls passed");
}

#[test]
fn native_apr_prefix_and_foreach_variants_preserve_real_partial_outputs_and_counts() {
    // Two first-error prefix ABIs and two result-array ABIs without a prefix.
    for (nid, prefixed, sized, each) in [
        ("i3HWvW35jao", true, false, false),
        ("w5fcCG+t31g", true, true, false),
        ("eYAh2vlCY-U", false, false, true),
        ("QzB4O+bJQyA", false, true, true),
    ] {
        let mut p = Program::new();
        list(&mut p);
        if prefixed {
            p.path(7, 0xdc0);
            p.ptr(6, 0);
            p.reg(2, 3);
            p.ptr(1, 32);
            if sized {
                p.ptr(8, 56);
                p.ptr(9, 104);
            } else {
                p.ptr(8, 104);
            }
        } else {
            p.ptr(7, 0);
            p.reg(6, 3);
            p.ptr(2, 32);
            if sized {
                p.ptr(1, 56);
                p.ptr(8, 80);
            } else {
                p.ptr(1, 80);
            }
        }
        p.call(0);
        p.expect(if each { 2 } else { u64::from(u32::MAX) });
        p.word(32, (u64::from(u32::MAX) << 32) | 1);
        p.word(40, u64::from(each));
        if sized {
            p.word(56, 3);
            p.word(64, 0);
            p.word(72, if each { 3 } else { 85 });
        }
        if each {
            p.word(80, 0x8002_0002_0000_0000);
            p.word(88, 0);
        } else {
            p.word(104, 1);
        }
        let (code, calls) = p.finish(None);
        let mut main = image(&code, &[nid]);
        strings(&mut main, prefixed);
        let data = create_input(vec![37, 42, 99]);
        let (status, fields, text) = run(&main, &data);
        assert_eq!(status, 0, "{text}");
        assert_eq!(fields["value"], "42");
        assert_eq!(fields["service_stops"], calls.to_string());
    }
    println!("native-apr-prefix-and-foreach: four programs, four calls passed");
}

#[test]
fn native_apr_prefixed_result_arrays_continue_and_refused_output_stops_explicitly() {
    for (nid, sized) in [("VB-BtuIW8Xc", false), ("C+Khtbbx2g8", true)] {
        for refused in [false, true] {
            let mut p = Program::new();
            list(&mut p);
            p.path(7, 0xdc0);
            p.ptr(6, 0);
            p.reg(2, 3);
            p.ptr(1, 32);
            let result_register = if sized {
                p.ptr(8, 56);
                9
            } else {
                8
            };
            if refused {
                p.reg(result_register, 1);
            } else {
                p.ptr(result_register, 80);
            }
            p.call(0);
            p.expect(2);
            p.word(32, (u64::from(u32::MAX) << 32) | 1);
            p.word(40, 1);
            p.word(80, 0x8002_0002_0000_0000);
            p.word(88, 0);
            if sized {
                p.word(56, 3);
                p.word(64, 0);
                p.word(72, 3);
            }
            let (code, calls) = p.finish(None);
            let mut main = image(&code, &[nid]);
            strings(&mut main, true);
            let data = create_input(vec![37, 42, 99]);
            let (status, fields, text) = run(&main, &data);
            if refused {
                assert_eq!(status, 2, "{text}");
                assert_eq!(fields["code"], "runtime-service");
                assert!(!fields.contains_key("value"));
            } else {
                assert_eq!(status, 0, "{text}");
                assert_eq!(fields["value"], "42");
                assert_eq!(fields["service_stops"], calls.to_string());
            }
        }
    }
    println!("native-apr-prefixed-results: two success and two typed-refusal programs passed");
}
