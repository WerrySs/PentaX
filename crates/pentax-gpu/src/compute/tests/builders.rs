// SPDX-License-Identifier: GPL-2.0-only
//! Original exact low-byte-zero dispatch and source launch-mode profiles.
use super::*;

#[test]
fn builder_wave32_dispatch_preserves_state_products_and_rejects_other_modes_before_fetch() {
    let memory = source();
    let old = prepare(&commands(), &memory, &Default::default()).unwrap();
    for low in [0u32, 2] {
        let mut input: Vec<u8> = [0xc003_7602u32, 0x204, 0, 0, 0]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();
        input.extend(commands());
        let at = input.len() - 20;
        input[at..at + 4].copy_from_slice(&(0xc003_1500 | low).to_le_bytes());
        let end = input.len();
        input[end - 4..].copy_from_slice(&0x8041u32.to_le_bytes());
        let actual = prepare(&input, &memory, &Default::default()).unwrap();
        assert_eq!(actual.groups(), old.groups());
        assert_eq!(actual.shader().spirv(), old.shader().spirv());
        assert_eq!(actual.input(), old.input());
        assert_eq!(actual.push_data(), old.push_data());
        assert_eq!(actual.usage().invocations, 16);
        for mode in [0x41u32, 0xa041, 0x8061, 0x8049] {
            input[end - 4..].copy_from_slice(&mode.to_le_bytes());
            let memory = source();
            assert!(matches!(
                prepare(&input, &memory, &Default::default()),
                Err(Error::Unsupported(_))
            ));
            assert_eq!(memory.reads.get(), 0);
        }
        input[end - 4..].copy_from_slice(&0x8041u32.to_le_bytes());
        input[8..12].copy_from_slice(&1u32.to_le_bytes());
        let memory = source();
        assert!(matches!(
            prepare(&input, &memory, &Default::default()),
            Err(Error::Unsupported("compute start coordinates"))
        ));
        assert_eq!(memory.reads.get(), 0);
    }
}
