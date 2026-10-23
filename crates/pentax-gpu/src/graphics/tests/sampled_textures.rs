// SPDX-License-Identifier: GPL-2.0-only
//! Original shader/T#/S# and padded mip snapshots, without device effects.
use super::*;
const TEXTURE: u64 = 0x30000;

fn descriptor() -> [u32; 8] {
    [0x300, 0x0380_0000, 0x8001, 0x9002_0fac, 0, 0x20, 0, 0]
}
fn pixel() -> Vec<u32> {
    let mut words = vec![
        unary(1, 0, 255),
        0x3e80_0000,
        unary(1, 1, 255),
        0x3f40_0000,
        unary(1, 2, 242),
    ];
    // Actual IMAGE_SAMPLE_L with 2D normalized coordinates and literal LOD1.
    words.extend([0xf090_0f08, 0x0040_0400]);
    words.extend(export(0, [4, 5, 6, 7]));
    words.push(END);
    words
}
struct Textured {
    original: Source,
    data: RefCell<Vec<u8>>,
    deny: Cell<bool>,
}
impl Textured {
    fn new() -> Self {
        let mut original = Source::new();
        original.pixel = pixel();
        Self {
            original,
            data: RefCell::new((0..1536).map(|index| (index % 251) as u8).collect()),
            deny: Cell::new(false),
        }
    }
}
impl Memory for Textured {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), MemoryError> {
        self.original.read(address, output)
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), MemoryError> {
        if address == TEXTURE {
            assert!(read && !written);
            if self.deny.get() {
                return Err(MemoryError::Unavailable);
            }
            let data = self.data.borrow();
            assert_eq!(output.len(), data.len());
            output.copy_from_slice(&data);
            Ok(())
        } else {
            self.original
                .snapshot_buffer(address, output, read, written)
        }
    }
}
fn textured_plan(image: [u32; 8]) -> CommandPlan {
    let mut words = commands(&[]);
    words.truncate(words.len() - 3);
    set(&mut words, RegisterSpace::Sh, 0x0b, &[24]);
    set(&mut words, RegisterSpace::Sh, 0x0c, &image);
    set(
        &mut words,
        RegisterSpace::Sh,
        0x14,
        &[0x92, 0x0020_0000, 0x0400_0000, 0],
    );
    words.extend([0xc001_2d00, 3, 2]);
    plan(&words)
}

#[test]
fn sampled_draw_captures_whole_owned_backing_and_reorders_exact_padded_mip_rows() {
    let memory = Textured::new();
    let commands = textured_plan(descriptor());
    let draw = prepare_draw(&commands.draws()[0], &memory, &Limits::default()).unwrap();
    assert_eq!(draw.pixel_images().len(), 1);
    assert_eq!(draw.pixel_samplers().len(), 1);
    let image = &draw.pixel_images()[0];
    assert_eq!(image.binding(), 58);
    assert_eq!(draw.pixel_samplers()[0].binding(), 74);
    assert_eq!(
        (image.source().address, image.source().size),
        (TEXTURE, 1536)
    );
    assert_eq!(
        image
            .mips()
            .iter()
            .map(|m| (m.width, m.height, m.offset, m.bytes))
            .collect::<Vec<_>>(),
        [(5, 3, 0, 60), (2, 1, 60, 8), (1, 1, 68, 4)]
    );
    let physical = memory.data.borrow();
    let mut expected = Vec::new();
    for start in [768, 1024, 1280] {
        expected.extend_from_slice(&physical[start..start + 20]);
    }
    expected.extend_from_slice(&physical[256..264]);
    expected.extend_from_slice(&physical[..4]);
    assert_eq!(image.pixels(), expected);
    drop(physical);
    memory.data.borrow_mut().fill(0);
    assert_eq!(image.pixels(), expected);
    assert!(draw.usage().host_bytes <= draw.usage().peak_host_bytes);
}

#[test]
fn sampled_draw_refuses_unimplemented_image_clamps_feedback_aliases_and_unreadable_backing() {
    let memory = Textured::new();
    for (word, mask) in [(1, 0x100), (5, 1 << 25)] {
        let mut image = descriptor();
        image[word] |= mask;
        let commands = textured_plan(image);
        assert!(matches!(
            prepare_draw(&commands.draws()[0], &memory, &Limits::default()),
            Err(Error::Unsupported(_))
        ));
        assert_eq!(memory.original.snapshots.get(), 0);
    }
    let mut image = descriptor();
    image[0] = (TARGET >> 8) as u32;
    let commands = textured_plan(image);
    assert!(matches!(
        prepare_draw(&commands.draws()[0], &memory, &Limits::default()),
        Err(Error::Unsupported("sampled texture source/target extent"))
    ));
    memory.deny.set(true);
    let commands = textured_plan(descriptor());
    assert!(matches!(
        prepare_draw(&commands.draws()[0], &memory, &Limits::default()),
        Err(Error::Memory(MemoryError::Unavailable))
    ));
    assert_eq!(memory.original.snapshots.get(), 0);
}
