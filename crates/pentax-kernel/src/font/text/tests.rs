// SPDX-License-Identifier: GPL-2.0-only
//! Original text bytes and checked owned views, without native execution.

use super::*;
use crate::{
    Limits,
    tests::{ARG, PAGE, TestGuest},
};

struct Fixture {
    text: TextState,
    objects: Objects,
    memory: MemoryState,
    guest: TestGuest,
    budget: Budget,
}
impl Fixture {
    fn new(limits: Limits) -> Self {
        Self {
            text: TextState::default(),
            objects: Objects::default(),
            memory: MemoryState::new(limits).unwrap(),
            guest: TestGuest::new(),
            budget: Budget::new(limits).unwrap(),
        }
    }
    fn call(&mut self, service: Service, args: &[u64]) -> Result<u64, Failure> {
        let mut words = [0; 6];
        words[..args.len()].copy_from_slice(args);
        let held = self.text.metadata_bytes() + self.objects.metadata_bytes();
        self.text.invoke(
            FontCall {
                service,
                args: CallArgs(words),
                vector_args: None,
                stack_pointer: None,
            },
            &mut self.objects,
            &mut self.memory,
            &mut self.guest,
            &mut self.budget,
            held,
        )
    }
    fn source(&mut self, data: &[u8], font: u64) {
        self.guest.put(0x600, data);
        assert_eq!(
            self.call(
                Service::FontTextSourceInit,
                &[ARG + 0x100, ARG + 0x600, data.len() as u64, 0, u64::MAX]
            ),
            Ok(0)
        );
        assert_eq!(
            self.call(Service::FontTextSourceSetDefaultFont, &[ARG + 0x100, font]),
            Ok(0)
        );
    }
    fn create(&mut self) -> Result<u64, Failure> {
        self.call(
            Service::FontCreateString,
            &[u64::MAX, ARG + 0x100, 0, ARG + 0x300],
        )?;
        Ok(self.guest.u64(0x300))
    }
}

#[test]
fn text_source_zero_before_alias_scan_explicit_unmapped_input_and_exact_setters() {
    let mut f = Fixture::new(Limits::default());
    f.guest.put(0x100, &[0xa5; 104]);
    assert_eq!(
        f.call(
            Service::FontTextSourceInit,
            &[ARG + 0x100, ARG + 0x108, 0, 0x1234, 0x5678]
        ),
        Ok(0)
    );
    let mut expected = [0; 96];
    for (offset, value) in [
        (8, ARG + 0x108),
        (16, ARG + 0x108),
        (24, ARG + 0x108),
        (32, 0x1234),
        (40, 0x5678),
        (56, 0x10),
    ] {
        expected[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    assert_eq!(f.guest.get(ARG + 0x100, 96), expected);
    assert_eq!(f.guest.get(ARG + 0x160, 8), [0xa5; 8]);
    let reads = f.budget.usage().bytes_read;
    assert_eq!(
        f.call(
            Service::FontTextSourceInit,
            &[ARG + 0x100, 0x7000_0000, 5, u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(f.budget.usage().bytes_read, reads);
    assert_eq!(f.guest.u64(0x110), 0x7000_0005);
    assert_eq!(
        f.call(
            Service::FontTextSourceSetWritingForm,
            &[ARG + 0x100, 0x1122_3344_ffff_fffd]
        ),
        Ok(0)
    );
    assert_eq!(f.guest.u64(0x138), (-3_i64) as u64);
    assert_eq!(
        f.call(
            Service::FontTextSourceSetDefaultFont,
            &[ARG + 0x100, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(f.guest.u64(0x130), u64::MAX);
    f.guest.put(0x118, &0_u64.to_le_bytes());
    assert_eq!(f.call(Service::FontTextSourceRewind, &[ARG + 0x100]), Ok(0));
    assert_eq!(f.guest.u64(0x118), 0x7000_0000);
    for service in [
        Service::FontTextSourceInit,
        Service::FontTextSourceSetWritingForm,
        Service::FontTextSourceSetDefaultFont,
        Service::FontTextSourceRewind,
    ] {
        assert_eq!(
            f.call(service, &[0, u64::MAX, u64::MAX, u64::MAX, u64::MAX]),
            Ok(FAILURE)
        );
    }
    assert_eq!(f.guest.get(ARG + 0x160, 8), [0xa5; 8]);
}

#[test]
fn raw_permissive_utf8_owned_views_exclusive_text_codes_and_overlapping_outputs() {
    let mut f = Fixture::new(Limits::default());
    let font = 0x8877_6655_4433_2211;
    f.source(
        &[
            b'A', 0, 0xc2, 0xff, 0xe0, 0x41, 0x42, 0xf0, 0xff, 0xff, 0xff, 0xe2, b' ',
        ],
        font,
    );
    f.guest.put(0x300, &[0xa5; 16]);
    let token = f.create().unwrap();
    assert_eq!(
        f.text.strings[0]
            .chars
            .iter()
            .map(|c| c.code)
            .collect::<Vec<_>>(),
        [65, 0, 191, 66, 0x3ffff, 0xe2, 32]
    );
    assert_eq!(f.guest.u64(0x118), ARG + 0x60d);
    assert_eq!(f.guest.get(ARG + 0x308, 8), [0xa5; 8]);
    f.guest.put(0x380, &[0xa5; 8]);
    let first = f
        .call(
            Service::FontStringRefersTextCharacters,
            &[token, ARG + 0x380],
        )
        .unwrap();
    assert_eq!(
        f.guest.get(ARG + 0x380, 8),
        [7_u32.to_le_bytes(), 0xa5a5_a5a5_u32.to_le_bytes()].concat()
    );
    let mut addresses = vec![first];
    for index in 1..7 {
        let next = f
            .call(
                Service::FontCharacterRefersTextNext,
                &[addresses[index - 1]],
            )
            .unwrap();
        assert_ne!(next, 0);
        addresses.push(next);
        assert_eq!(
            f.call(Service::FontCharacterRefersTextBack, &[next]),
            Ok(addresses[index - 1])
        );
    }
    assert_eq!(
        f.call(Service::FontCharacterRefersTextBack, &[first]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::FontCharacterRefersTextNext, &[addresses[6]]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::FontCharacterLooksWhiteSpace, &[addresses[6]]),
        Ok(1)
    );
    assert_eq!(
        f.call(
            Service::FontStringRefersRenderCharacters,
            &[token, addresses[1], addresses[3], ARG + 0x380]
        ),
        Ok(addresses[1])
    );
    assert_eq!(f.guest.get(ARG + 0x380, 4), 2_u32.to_le_bytes());
    // With an explicit first, string is opaque and last is comparison-only.
    // A one-past or foreign sentinel is never dereferenced as a character.
    for (last, count) in [
        (addresses[3], 2_u32),
        (addresses[6] + CHARACTER_BYTES as u64, 6),
        (u64::MAX, 6),
        (addresses[1], 0),
    ] {
        assert_eq!(
            f.call(
                Service::FontStringRefersRenderCharacters,
                &[u64::MAX, addresses[1], last, ARG + 0x380]
            ),
            Ok(addresses[1])
        );
        assert_eq!(f.guest.get(ARG + 0x380, 4), count.to_le_bytes());
    }
    let reads = f.budget.usage().bytes_read;
    assert_eq!(
        f.call(
            Service::FontStringRefersRenderCharacters,
            &[u64::MAX, u64::MAX, u64::MAX - 1, 0]
        ),
        Ok(u64::MAX)
    );
    assert_eq!(f.budget.usage().bytes_read, reads);
    f.guest.put(0x400, &[0xa5; 16]);
    assert_eq!(
        f.call(
            Service::FontCharacterGetTextFontCode,
            &[first, ARG + 0x400, ARG + 0x404]
        ),
        Ok(0)
    );
    assert_eq!(
        f.guest.get(ARG + 0x400, 16),
        [
            0x11, 0x22, 0x33, 0x44, 65, 0, 0, 0, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5, 0xa5
        ]
    );
    f.guest.put(0x500, &[0xa5; 72]);
    assert_eq!(
        f.call(
            Service::FontCharactersRefersTextCodes,
            &[addresses[1], addresses[3], ARG + 0x500]
        ),
        Ok(ARG + 0x500)
    );
    let mut codes = [0; 64];
    codes[..8].copy_from_slice(&(ARG + 0x601).to_le_bytes());
    codes[16..24].copy_from_slice(&addresses[1].to_le_bytes());
    codes[24..32].copy_from_slice(&addresses[3].to_le_bytes());
    assert_eq!(f.guest.get(ARG + 0x500, 64), codes);
    assert_eq!(f.guest.get(ARG + 0x540, 8), [0xa5; 8]);
    assert_eq!(
        f.call(Service::FontTextCodesStepNext, &[ARG + 0x500]),
        Ok(ARG + 0x500)
    );
    assert_eq!(f.guest.u64(0x510), addresses[2]);
    let before = f.guest.get(ARG + 0x500, 64);
    assert_eq!(
        f.call(Service::FontTextCodesStepNext, &[ARG + 0x500]),
        Ok(0)
    );
    assert_eq!(f.guest.get(ARG + 0x500, 64), before);
    assert_eq!(
        f.call(Service::FontTextCodesStepBack, &[ARG + 0x500]),
        Ok(ARG + 0x500)
    );
    assert_eq!(f.guest.u64(0x510), addresses[1]);
    assert_eq!(f.call(Service::FontDestroyString, &[ARG + 0x300]), Ok(0));
    assert_eq!(f.text.retained_handle_count(), 0);
    assert_eq!(f.memory.retained_handle_count(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
    assert!(
        f.call(Service::FontCharacterRefersTextNext, &[first])
            .is_err()
    );
    assert!(f.call(Service::FontStringGetWritingForm, &[token]).is_err());
}

#[test]
fn text_constructor_late_publication_and_quota_refusal_keep_current_tables_and_resources() {
    for quota in [false, true] {
        let mut f = Fixture::new(Limits {
            max_handles: if quota { 1 } else { 256 },
            ..Limits::default()
        });
        f.source(b"Original", 0x4455);
        f.guest.put(0x300, &[0xa5; 16]);
        let source = f.guest.get(ARG + 0x100, 96);
        f.guest.refuse = !quota;
        assert!(f.create().is_err());
        assert_eq!(f.guest.get(ARG + 0x100, 96), source);
        assert_eq!(f.guest.get(ARG + 0x300, 16), [0xa5; 16]);
        assert_eq!(f.text.metadata_bytes(), 0);
        assert_eq!(f.text.retained_handle_count(), 0);
        assert_eq!(f.memory.retained_handle_count(), 0);
        assert_eq!(f.budget.usage().handles_live, 0);
        if !quota {
            f.guest.refuse = false;
            let token = f.create().unwrap();
            assert_ne!(token, 0);
            assert_eq!(f.budget.usage().handles_live, 2);
            assert_eq!(f.guest.u64(0x118), ARG + 0x608);
        }
    }
    // A genuinely late unmapped output must not publish source.current or map.
    let mut f = Fixture::new(Limits::default());
    f.source(b"Late", 0);
    assert!(
        f.call(
            Service::FontCreateString,
            &[0, ARG + 0x100, 0, ARG + PAGE - 4]
        )
        .is_err()
    );
    assert_eq!(f.guest.u64(0x118), ARG + 0x600);
    assert_eq!(f.text.metadata_bytes(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
}

#[test]
fn opaque_character_constants_and_null_outputs_do_not_require_input_payload_reads() {
    let mut f = Fixture::new(Limits::default());
    f.guest.put(0x80, &[0xa5; 8]);
    let reads = f.budget.usage().bytes_read;
    for service in [
        Service::FontCharacterGetBidiLevel,
        Service::FontCharacterGetSyllableStringState,
    ] {
        assert_eq!(f.call(service, &[u64::MAX, ARG + 0x80]), Ok(0));
        assert_eq!(
            f.guest.get(ARG + 0x80, 8),
            [0_u32.to_le_bytes(), 0xa5a5_a5a5_u32.to_le_bytes()].concat()
        );
        assert_eq!(f.call(service, &[0, u64::MAX]), Ok(FAILURE));
        assert_eq!(f.call(service, &[u64::MAX, 0]), Ok(FAILURE));
    }
    assert_eq!(
        f.call(Service::FontCharacterLooksFormatCharacters, &[u64::MAX]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::FontCharacterGetTextFontCode, &[u64::MAX, 0, 0]),
        Ok(0)
    );
    assert_eq!(
        f.call(Service::FontCharacterGetTextFontCode, &[0, 0, 0]),
        Ok(FAILURE)
    );
    assert_eq!(f.budget.usage().bytes_read, reads);
    assert_eq!(f.text.metadata_bytes(), 0);
    assert_eq!(f.budget.usage().handles_live, 0);
}
