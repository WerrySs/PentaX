// SPDX-License-Identifier: GPL-2.0-or-later
use super::*;
use crate::open_executable;
fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x1000];
    put(&mut bytes, 0, b"\x7fELF\x02\x01\x01");
    put(&mut bytes, 16, &3u16.to_le_bytes());
    put(&mut bytes, 18, &62u16.to_le_bytes());
    put(&mut bytes, 20, &1u32.to_le_bytes());
    put(&mut bytes, 32, &64u64.to_le_bytes());
    put(&mut bytes, 52, &64u16.to_le_bytes());
    put(&mut bytes, 54, &56u16.to_le_bytes());
    put(&mut bytes, 56, &3u16.to_le_bytes());
    for (index, kind, offset, size) in [
        (0, 1u32, 0, 0x800),
        (1, 0x6474_e550, 0x700, 8),
        (2, 2, 0x200, 64),
    ] {
        let at = 64 + index * 56;
        put(&mut bytes, at, &kind.to_le_bytes());
        put(&mut bytes, at + 4, &4u32.to_le_bytes());
        put(&mut bytes, at + 8, &(offset as u64).to_le_bytes());
        put(&mut bytes, at + 16, &(offset as u64).to_le_bytes());
        put(&mut bytes, at + 32, &(size as u64).to_le_bytes());
        put(&mut bytes, at + 40, &(size as u64).to_le_bytes());
        put(&mut bytes, at + 48, &1u64.to_le_bytes());
    }
    put(&mut bytes, 0x700, &[1, 0x1b, 0xff, 0xff]);
    put(&mut bytes, 0x704, &(-0x104i32).to_le_bytes());
    let strings = b"\0original-module.so\0";
    put(&mut bytes, 0x300, strings);
    for (index, (tag, value)) in [(5u64, 0x300), (10, strings.len() as u64), (14, 1), (0, 0)]
        .into_iter()
        .enumerate()
    {
        put(&mut bytes, 0x200 + index * 16, &tag.to_le_bytes());
        put(&mut bytes, 0x208 + index * 16, &value.to_le_bytes());
    }
    bytes
}
fn inspect(bytes: &[u8]) -> Result<UnwindMetadata, UnwindError> {
    open_executable(bytes)
        .unwrap()
        .runtime_plan()
        .unwrap()
        .unwind_metadata(bytes)
}

#[test]
fn unwind_header_decodes_checked_signed_pc_relative_and_bounded_frame_extent() {
    let bytes = fixture();
    let metadata = inspect(&bytes).unwrap();
    assert_eq!(
        metadata.header,
        Some(VirtualRange {
            address: 0x700,
            size: 8
        })
    );
    assert_eq!(
        metadata.frame,
        Some(VirtualRange {
            address: 0x600,
            size: 0x100
        })
    );
    assert_eq!(&metadata.name[..19], b"original-module.so\0");
    assert_eq!(
        decode_pointer(&(-4i32).to_le_bytes(), 0x1b, 2),
        Err(UnwindError::Extent)
    );
    assert_eq!(
        decode_pointer(&u64::MAX.to_le_bytes(), 0x14, 1),
        Err(UnwindError::Extent)
    );
    assert_eq!(
        decode_pointer(&[0; 8], 0x9b, 1),
        Err(UnwindError::Unsupported)
    );
}

#[test]
fn unwind_allocated_eh_frame_section_supplies_exact_size_and_rejects_false_file_extent() {
    let mut bytes = fixture();
    put(&mut bytes, 40, &0x900u64.to_le_bytes());
    put(&mut bytes, 58, &64u16.to_le_bytes());
    put(&mut bytes, 60, &3u16.to_le_bytes());
    put(&mut bytes, 62, &1u16.to_le_bytes());
    put(&mut bytes, 0x880, b"\0.eh_frame\0");
    put(&mut bytes, 0x940 + 4, &3u32.to_le_bytes());
    put(&mut bytes, 0x940 + 24, &0x880u64.to_le_bytes());
    put(&mut bytes, 0x940 + 32, &11u64.to_le_bytes());
    put(&mut bytes, 0x980, &1u32.to_le_bytes());
    put(&mut bytes, 0x980 + 4, &1u32.to_le_bytes());
    put(&mut bytes, 0x980 + 8, &2u64.to_le_bytes());
    put(&mut bytes, 0x980 + 16, &0x600u64.to_le_bytes());
    put(&mut bytes, 0x980 + 24, &0x600u64.to_le_bytes());
    put(&mut bytes, 0x980 + 32, &0x80u64.to_le_bytes());
    assert_eq!(
        inspect(&bytes).unwrap().frame,
        Some(VirtualRange {
            address: 0x600,
            size: 0x80
        })
    );
    put(&mut bytes, 0x980 + 24, &0x500u64.to_le_bytes());
    assert_eq!(inspect(&bytes), Err(UnwindError::Extent));
}

#[test]
fn unwind_unknown_or_truncated_pointer_encoding_never_becomes_zero_success() {
    let mut bytes = fixture();
    bytes[0x701] = 4;
    assert_eq!(inspect(&bytes), Err(UnwindError::Extent));
    bytes[0x701] = 1;
    assert_eq!(inspect(&bytes), Err(UnwindError::Unsupported));
    bytes[0x701] = 0x1b;
    put(&mut bytes, 0x704, &(-0x705i32).to_le_bytes());
    assert_eq!(inspect(&bytes), Err(UnwindError::Extent));
    bytes[0x701] = 0xff;
    assert_eq!(inspect(&bytes).unwrap().frame, None);
}
