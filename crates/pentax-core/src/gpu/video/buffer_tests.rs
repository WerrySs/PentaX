// SPDX-License-Identifier: GPL-2.0-only
use super::tests::{PAGE, backing};
use super::*;
use crate::gpu::driver::{Driver, Limits};

fn attribute(format: u64, width: u32, height: u32, control: u32) -> [u8; 80] {
    let mut bytes = [0; 80];
    bytes[12..16].copy_from_slice(&width.to_le_bytes());
    bytes[16..20].copy_from_slice(&height.to_le_bytes());
    bytes[24..32].copy_from_slice(&40_u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&format.to_le_bytes());
    bytes[48..52].copy_from_slice(&control.to_le_bytes());
    bytes
}
fn descriptors(data: &[u64], metadata: u64) -> Vec<u8> {
    data.iter()
        .flat_map(|data| [*data, metadata, u64::MAX, u64::MAX])
        .flat_map(u64::to_le_bytes)
        .collect()
}
fn open(driver: &mut Driver, memory: &mut PausedNativeMemory<'_>) {
    assert_eq!(
        driver.submit(memory, Function::VideoOpen, [255, 0, 0, u64::MAX, 0, 0]),
        Ok(1)
    );
}
fn register(
    driver: &mut Driver,
    memory: &mut PausedNativeMemory<'_>,
    base: u64,
    set: u64,
    start: u64,
    count: u64,
) -> Result<u64, Failure> {
    driver.submit_with_stack(
        memory,
        Function::VideoRegisterBuffers2,
        [1, set, start, base + 256, count, base + 128],
        Some(base),
    )
}

#[test]
fn video_registered_buffers_retain_groups_geometry_change_and_port_lifetime() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    open(&mut driver, &mut memory);
    memory.write(base + 8, &[0; 16]).unwrap();
    memory
        .write(base + 128, &attribute(0x8000_0000_2200_0000, 1920, 1080, 0))
        .unwrap();
    memory
        .write(base + 256, &descriptors(&[0x1000_0000, 0x1800_0000], 0))
        .unwrap();
    assert_eq!(register(&mut driver, &mut memory, base, 0, 2, 2), Ok(0));
    let first = driver.video_surface(1, 2).unwrap();
    assert_eq!(
        (first.width, first.height, first.pitch, first.data.size),
        (1920, 1080, 1920, 8_847_360)
    );
    assert_eq!(first.format, PixelFormat::Rgba8Srgb);
    assert_eq!(first.compression, Compression::None);
    assert_eq!(first.options, 40);
    assert!(first.metadata.is_none());
    assert!(driver.video_surface(1, 1).is_none());
    assert!(driver.video_surface(2, 2).is_none());
    assert_eq!(
        driver.video_surface(1, 3).unwrap().data.address,
        0x1800_0000
    );
    // Source copies all inputs; later descriptor/header changes confer no update.
    memory
        .write(base + 128, &attribute(0xc001_0006_0000_0000, 129, 65, 0))
        .unwrap();
    memory.write(base + 256, &[0; 64]).unwrap();
    assert_eq!(driver.video_surface(1, 2).unwrap().width, 1920);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoChangeBufferAttribute2,
            [1, 0, base + 128, 0, 0, 0]
        ),
        Ok(0)
    );
    for index in [2, 3] {
        let surface = driver.video_surface(1, index).unwrap();
        assert_eq!(
            (
                surface.width,
                surface.height,
                surface.pitch,
                surface.data.size
            ),
            (129, 65, 256, 262_144)
        );
        assert_eq!(surface.format, PixelFormat::Bgra16Float);
    }
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoUnregisterBuffers,
            [1, 0, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert!(driver.video_surface(1, 2).is_none());
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoUnregisterBuffers,
            [1, 0, 0, 0, 0, 0]
        ),
        Ok(0x8029_000a)
    );
    memory
        .write(base + 256, &descriptors(&[0x2000_0000], 0))
        .unwrap();
    assert_eq!(register(&mut driver, &mut memory, base, 0, 15, 1), Ok(0));
    assert_eq!(
        driver.submit(&mut memory, Function::VideoClose, [1, 0, 0, 0, 0, 0]),
        Ok(0)
    );
    assert!(driver.video_surface(1, 15).is_none());
    open(&mut driver, &mut memory);
    assert!(driver.video_surface(1, 15).is_none());
    assert_eq!(register(&mut driver, &mut memory, base, 0, 15, 1), Ok(0));
    assert_eq!(driver.usage().dispatches, 0);
    assert_eq!(driver.frame_state().boundaries, 0);
}

#[test]
fn video_buffer_registration_refusals_preserve_all_groups_and_source_error_order() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    // Invalid port precedes null pointers, bad indices and absent stack.
    assert_eq!(
        driver.submit(&mut memory, Function::VideoRegisterBuffers2, [0; 6]),
        Ok(INVALID_HANDLE)
    );
    open(&mut driver, &mut memory);
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoRegisterBuffers2,
            [1, 99, 99, 0, 99, 0]
        ),
        Ok(INVALID_ADDRESS)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoRegisterBuffers2,
            [1, 99, 99, base, 99, 0]
        ),
        Ok(INVALID_OPTION)
    );
    for (set, start, count) in [
        (4, 0, 1),
        (0, 16, 1),
        (0, 0, 0),
        (0, 15, 2),
        (u64::MAX, 0, 1),
    ] {
        assert_eq!(
            register(&mut driver, &mut memory, base, set, start, count),
            Ok(INVALID_VALUE)
        );
    }
    memory.write(base + 8, &2_u64.to_le_bytes()).unwrap();
    memory.write(base + 16, &1_u64.to_le_bytes()).unwrap();
    assert_eq!(
        register(&mut driver, &mut memory, base, 0, 0, 1),
        Ok(INVALID_OPTION)
    );
    memory.write(base + 16, &0_u64.to_le_bytes()).unwrap();
    assert_eq!(
        register(&mut driver, &mut memory, base, 0, 0, 1),
        Ok(0x8029_001d)
    );
    // Category is an int32; the upper stack-word bits are ignored.
    memory
        .write(base + 8, &0xdead_beef_0000_0000_u64.to_le_bytes())
        .unwrap();
    memory
        .write(base + 128, &attribute(0x8000_0000_0000_0000, 1, 1, 0))
        .unwrap();
    memory
        .write(base + 256, &descriptors(&[0x1000_0000, 0], 0))
        .unwrap();
    assert_eq!(
        register(&mut driver, &mut memory, base, 0, 0, 2),
        Err(Failure::UnsupportedVideo)
    );
    assert!(driver.video_surface(1, 0).is_none());
    memory
        .write(base + 288, &descriptors(&[0x2000_0000], 0))
        .unwrap();
    assert_eq!(register(&mut driver, &mut memory, base, 0, 0, 2), Ok(0));
    assert_eq!(
        register(&mut driver, &mut memory, base, 0, 2, 1),
        Ok(0x8029_000a)
    );
    assert_eq!(
        register(&mut driver, &mut memory, base, 1, 1, 1),
        Ok(0x8029_0010)
    );
    assert!(driver.video_surface(1, 2).is_none());
    assert_eq!(register(&mut driver, &mut memory, base, 1, 2, 1), Ok(0));
    // An invalid replacement preserves every surface in the existing group.
    memory
        .write(base + 128, &attribute(0x8000_0000_0000_0000, 0, 1, 0))
        .unwrap();
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoChangeBufferAttribute2,
            [1, 0, base + 128, 0, 0, 0]
        ),
        Err(Failure::UnsupportedVideo)
    );
    for index in [0, 1, 2] {
        assert_eq!(driver.video_surface(1, index).unwrap().width, 1);
    }
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoChangeBufferAttribute2,
            [1, 4, 0, 0, 0, 0]
        ),
        Ok(INVALID_OPTION)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoChangeBufferAttribute2,
            [1, 4, u64::MAX, 0, 0, 0]
        ),
        Ok(0x8029_000a)
    );
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoChangeBufferAttribute2,
            [1, 3, u64::MAX, 0, 0, 0]
        ),
        Ok(0x8029_000a)
    );
    // Unregister affects only its own group.
    assert_eq!(
        driver.submit(
            &mut memory,
            Function::VideoUnregisterBuffers,
            [1, 0, 0, 0, 0, 0]
        ),
        Ok(0)
    );
    assert!(driver.video_surface(1, 0).is_none());
    assert!(driver.video_surface(1, 1).is_none());
    assert!(driver.video_surface(1, 2).is_some());
}

#[test]
fn video_buffer_formats_compression_and_single_level_source_footprints() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    open(&mut driver, &mut memory);
    for (format, kind, bytes) in [
        (0x8000_0000_2200_0000, PixelFormat::Rgba8Srgb, 4),
        (0x8000_0000_0000_0000, PixelFormat::Bgra8Srgb, 4),
        (0x8100_0000_2200_0000, PixelFormat::Rgb10A2, 4),
        (0x8100_0000_0000_0000, PixelFormat::Bgr10A2, 4),
        (0x8100_0704_2200_0000, PixelFormat::Rgb10A2Pq, 4),
        (0xc001_0006_2200_0000, PixelFormat::Rgba16Float, 8),
        (0xc001_0006_0000_0000, PixelFormat::Bgra16Float, 8),
    ] {
        for (category, control, compression) in [
            (0, 0, Compression::None),
            (1, 0x48, Compression::Dcc256_256_0),
            (1, 0x208, Compression::Dcc256_64_64),
        ] {
            memory
                .write(base + 8, &(category as u64).to_le_bytes())
                .unwrap();
            memory.write(base + 16, &0_u64.to_le_bytes()).unwrap();
            memory
                .write(base + 128, &attribute(format, 513, 513, control))
                .unwrap();
            let metadata = if category == 0 { 0 } else { 0x3000_0100 };
            memory
                .write(base + 256, &descriptors(&[0x1000_0000], metadata))
                .unwrap();
            assert_eq!(register(&mut driver, &mut memory, base, 0, 0, 1), Ok(0));
            let surface = driver.video_surface(1, 0).unwrap();
            assert_eq!(surface.format, kind);
            assert_eq!(surface.compression, compression);
            assert_eq!((surface.pitch, surface.bytes_per_pixel), (640, bytes));
            assert_eq!(
                surface.data.size,
                if bytes == 4 { 1_638_400 } else { 2_949_120 }
            );
            assert_eq!(
                surface.metadata.map(|range| range.size),
                if category == 0 {
                    None
                } else {
                    Some(if bytes == 4 { 16384 } else { 24576 })
                }
            );
            assert_eq!(
                driver.submit(
                    &mut memory,
                    Function::VideoUnregisterBuffers,
                    [1, 0, 0, 0, 0, 0]
                ),
                Ok(0)
            );
        }
    }
}

#[test]
fn video_buffer_exact_input_spans_and_limits_refuse_before_registration() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    open(&mut driver, &mut memory);
    memory.write(base + 8, &[0; 16]).unwrap();
    memory
        .write(
            base + 128,
            &attribute(0x8000_0000_2200_0000, 16384, 16384, 0),
        )
        .unwrap();
    let last = base + PAGE - 32;
    memory
        .write(last, &descriptors(&[(1 << 40) - 65536], 0))
        .unwrap();
    // Source tracker geometry rejects the complete range, even when its base fits.
    assert_eq!(
        driver.submit_with_stack(
            &mut memory,
            Function::VideoRegisterBuffers2,
            [1, 0, 0, last, 1, base + 128],
            Some(base)
        ),
        Err(Failure::UnsupportedVideo)
    );
    memory.write(last, &descriptors(&[0x1000_0000], 0)).unwrap();
    for (source, header, stack) in [
        (last + 1, base + 128, Some(base)),
        (last, base + PAGE - 79, Some(base)),
        (last, base + 128, Some(base + PAGE - 23)),
        (last, base + 128, None),
    ] {
        assert!(
            driver
                .submit_with_stack(
                    &mut memory,
                    Function::VideoRegisterBuffers2,
                    [1, 0, 0, source, 1, header],
                    stack
                )
                .is_err()
        );
        assert!(driver.video_surface(1, 0).is_none());
    }
    assert_eq!(
        driver.submit_with_stack(
            &mut memory,
            Function::VideoRegisterBuffers2,
            [1, 0, 0, last, 1, base + 128],
            Some(base)
        ),
        Ok(0)
    );
    assert_eq!(driver.video_surface(1, 0).unwrap().data.size, 1_073_741_824);
    let mut limited = Driver::new(Limits {
        max_preparation_work: 4095,
        ..Default::default()
    });
    open(&mut limited, &mut memory);
    assert_eq!(
        register(&mut limited, &mut memory, base, 0, 0, 1),
        Err(Failure::Limit)
    );
    assert!(limited.video_surface(1, 0).is_none());
}

#[test]
fn video_invalid_surface_attributes_and_compression_never_publish_a_group() {
    let (mut owner, base) = backing();
    let mut memory = owner.pause().unwrap();
    let mut driver = Driver::new(Default::default());
    open(&mut driver, &mut memory);
    memory.write(base + 8, &[0; 16]).unwrap();
    memory
        .write(base + 256, &descriptors(&[0x1000_0000], 0))
        .unwrap();
    let valid = attribute(0x8000_0000_2200_0000, 128, 128, 0);
    for (offset, value) in [
        (0, 1),
        (4, 1),
        (8, 1),
        (12, 0),
        (16, 0),
        (20, 1),
        (24, 1),
        (32, 1),
        (40, 1),
        (48, 1),
        (52, 1),
        (56, 1),
        (64, 1),
        (72, 1),
    ] {
        let mut invalid = valid;
        invalid[offset..offset + 4].copy_from_slice(&(value as u32).to_le_bytes());
        memory.write(base + 128, &invalid).unwrap();
        assert_eq!(
            register(&mut driver, &mut memory, base, 0, 0, 1),
            Err(Failure::UnsupportedVideo)
        );
        assert!(driver.video_surface(1, 0).is_none());
    }
    for (category, data, metadata, control, clear) in [
        (0, 0, 0, 0, 0),
        (0, 0x1000_0001, 0, 0, 0),
        (0, 1 << 40, 0, 0, 0),
        (0, 0x1000_0000, 0x2000_0000, 0, 0),
        (1, 0x1000_0000, 0, 0x48, 0),
        (1, 0x1000_0000, 0x2000_0001, 0x48, 0),
        (1, 0x1000_0000, (1 << 40) - 256, 0x48, 0),
        (1, 0x1000_0000, 0x2000_0000, 0x49, 0),
        (1, 0x1000_0000, 0x2000_0000, 0x48, 1),
    ] {
        memory
            .write(base + 8, &(category as u64).to_le_bytes())
            .unwrap();
        let mut bytes = attribute(0x8000_0000_2200_0000, 128, 128, control);
        bytes[40..48].copy_from_slice(&(clear as u64).to_le_bytes());
        memory.write(base + 128, &bytes).unwrap();
        memory
            .write(base + 256, &descriptors(&[data], metadata))
            .unwrap();
        assert_eq!(
            register(&mut driver, &mut memory, base, 0, 0, 1),
            Err(Failure::UnsupportedVideo)
        );
        assert!(driver.video_surface(1, 0).is_none());
    }
}
