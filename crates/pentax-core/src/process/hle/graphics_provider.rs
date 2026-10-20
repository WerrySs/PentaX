// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: original ELF factory for scoped submit services.
// Identities from pinned libAgcDriver.cpp/libAgc.cpp/libVideoOut.cpp; see NOTICE.

//! Exact graphics/video module1.1/library1 exports. Private marker
//! numbers are PentaX library traps, never native kernel syscall numbers.
use super::{KernelProviderError, provider::program};
use crate::{gpu::driver::Function, memory::PAGE_SIZE};
use pentax_linker::{DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO};

// Namespace indices select explicit module/library descriptors; the existing
// six submit/boundary marker indices and RVAs remain stable. No fallback exists.
const BASE_ENTRIES: [(&str, Function, usize, usize); 6] = [
    ("b4fpgH5ZXxQ", Function::CommandBuffer, 0, 0),
    ("Fj7r9EHzF38", Function::MultiCommandBuffers, 0, 0),
    ("gSRnr79F8tQ", Function::Acb, 0, 0),
    ("HF3YllT3mXU", Function::MultiAcbs, 0, 0),
    ("h9z6+0hEydk", Function::SuspendPoint, 1, 1),
    ("h9z6+0hEydk", Function::SuspendPoint, 2, 2),
];
const ENTRIES: [(&str, Function, usize, usize); 106] = {
    let video = [
        ("Up36PTk687E", Function::VideoOpen),
        ("uquVH4-Du78", Function::VideoClose),
        ("WkYtyOg30do", Function::VideoSetFlipMaster),
        ("CBiu4mCE1DA", Function::VideoSetFlipRate),
        ("+I4K03i3EL0", Function::VideoInitializeOptions),
        ("Nv8c-Kb+DUM", Function::VideoIsOutputSupported),
        ("w0hLuNarQxY", Function::VideoConfigureOutput),
        ("utPrVdxio-8", Function::VideoGetOutputStatus),
        ("PjS5uASwcV8", Function::VideoSetBufferAttribute2),
        ("MBlv8YWoyOk", Function::VideoAddBufferAttributeOption),
    ];
    let mut entries = [BASE_ENTRIES[0]; 106];
    let mut i = 0;
    while i < BASE_ENTRIES.len() {
        entries[i] = BASE_ENTRIES[i];
        i += 1;
    }
    i = 0;
    while i < video.len() {
        let (nid, function) = video[i];
        let at = BASE_ENTRIES.len() + i * 4;
        entries[at] = (nid, function, 3, 3);
        entries[at + 1] = (nid, function, 3, 4);
        entries[at + 2] = (nid, function, 4, 3);
        entries[at + 3] = (nid, function, 4, 4);
        i += 1;
    }
    entries[46] = ("23LRUSvYu1M", Function::AgcInit, 1, 1);
    entries[47] = ("23LRUSvYu1M", Function::AgcInit, 2, 2);
    entries[48] = ("2JtWUUiYBXs", Function::AgcGetRegisterDefaults, 1, 1);
    entries[49] = ("2JtWUUiYBXs", Function::AgcGetRegisterDefaults, 2, 2);
    entries[50] = (
        "wRbq6ZjNop4",
        Function::AgcGetRegisterDefaultsInternal,
        1,
        1,
    );
    entries[51] = (
        "wRbq6ZjNop4",
        Function::AgcGetRegisterDefaultsInternal,
        2,
        2,
    );
    entries[52] = ("BfBDZGbti7A", Function::AgcGetIsTrinityMode, 1, 1);
    entries[53] = ("BfBDZGbti7A", Function::AgcGetIsTrinityMode, 2, 2);
    let registry = ["X-Nm5KLREeg", "W5z4eZrjEas", "ZLJk9r2+2Aw", "pWLG7WOpVcw"];
    let mut r = 0;
    while r < registry.len() {
        entries[54 + r * 2] = (registry[r], Function::AgcRegistryUnavailable, 1, 1);
        entries[55 + r * 2] = (registry[r], Function::AgcRegistryUnavailable, 2, 2);
        // Explicit reviewed driver ABI; this is not Graphics5Driver or a
        // NID-only fallback. The prefixed spelling is declared by the title.
        entries[62 + r * 2] = (registry[r], Function::AgcRegistryUnavailable, 5, 5);
        entries[63 + r * 2] = (registry[r], Function::AgcRegistryUnavailable, 6, 6);
        r += 1;
    }
    entries[70] = ("f3dg2CSgRKY", Function::AgcCreateShader, 1, 1);
    entries[71] = ("f3dg2CSgRKY", Function::AgcCreateShader, 2, 2);
    let events = [
        ("w2rJhmD+dsE", Function::AgcAddEqEvent),
        ("DL2RXaXOy88", Function::AgcDeleteEqEvent),
        ("5CdQTZIQPxM", Function::AgcGetEqEventType),
        ("Zw7uUVPulbw", Function::AgcGetEqContextId),
    ];
    i = 0;
    while i < events.len() {
        let (nid, function) = events[i];
        entries[72 + i * 3] = (nid, function, 0, 0);
        entries[73 + i * 3] = (nid, function, 5, 5);
        entries[74 + i * 3] = (nid, function, 6, 6);
        i += 1;
    }
    let buffers = [
        ("rKBUtgRrtbk", Function::VideoRegisterBuffers2),
        ("HuViW4HnrOw", Function::VideoChangeBufferAttribute2),
        ("N5KDtkIjjJ4", Function::VideoUnregisterBuffers),
    ];
    i = 0;
    while i < buffers.len() {
        let (nid, function) = buffers[i];
        entries[84 + i * 4] = (nid, function, 3, 3);
        entries[85 + i * 4] = (nid, function, 3, 4);
        entries[86 + i * 4] = (nid, function, 4, 3);
        entries[87 + i * 4] = (nid, function, 4, 4);
        i += 1;
    }
    let fusion = [
        ("dolOmWH+huQ", Function::AgcGetFusedShaderSize),
        ("fd5Bp5tGTgo", Function::AgcFuseShaderHalves),
        ("nApJjpKNBl4", Function::AgcFuseShaderHalvesLegacy),
    ];
    i = 0;
    while i < fusion.len() {
        let (nid, function) = fusion[i];
        entries[96 + i * 2] = (nid, function, 1, 1);
        entries[97 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let interpolants = [
        ("HV4j+E0MBHE", Function::AgcCreateInterpolantMapping),
        ("dbOlWdppb4o", Function::AgcCreateInterpolantMapping2),
    ];
    i = 0;
    while i < interpolants.len() {
        let (nid, function) = interpolants[i];
        entries[102 + i * 2] = (nid, function, 1, 1);
        entries[103 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    entries
};
const BASE: u32 = 0x5058_4700;
const IMAGE_BYTES: usize = 0x2000;
const NAMES: [&str; 7] = [
    "Graphics5Driver",
    "Graphics5",
    "libSceAgc",
    "VideoOut",
    "libSceVideoOut",
    "AgcDriver",
    "libSceAgcDriver",
];
const DYNAMIC_COUNT: u64 = 5 + 2 * NAMES.len() as u64 + 1;
const NAME_BYTES: usize = {
    let mut bytes = 1;
    let mut i = 0;
    while i < NAMES.len() {
        bytes += NAMES[i].len() + 1;
        i += 1;
    }
    bytes
};
const fn align_metadata(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}
// Derive disjoint metadata from the fixed catalogue; appending exact exports
// never relocates existing marker IDs/RVAs or bypasses the image-size bound.
const DYNAMIC_AT: usize = align_metadata(0x100 + ENTRIES.len() * 16, 64);
const STRINGS_AT: usize = align_metadata(DYNAMIC_AT + DYNAMIC_COUNT as usize * 16, 64);
const SYMBOLS_AT: usize = align_metadata(STRINGS_AT + NAME_BYTES + ENTRIES.len() * 16, 256);
const _: () = {
    assert!(0x100 + ENTRIES.len() * 16 <= DYNAMIC_AT);
    assert!(DYNAMIC_AT + DYNAMIC_COUNT as usize * 16 <= STRINGS_AT);
    assert!(SYMBOLS_AT + (ENTRIES.len() + 1) * 24 <= IMAGE_BYTES);
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphicsMarker {
    pub symbol: usize,
    pub function: Function,
}
impl GraphicsMarker {
    pub fn rva(self) -> u64 {
        0x100 + (self.symbol as u64 - 1) * 16
    }
    pub fn id(self) -> u32 {
        BASE + self.symbol as u32
    }
    pub fn bytes(self) -> [u8; 8] {
        let [a, b, c, d] = self.id().to_le_bytes();
        [0xb8, a, b, c, d, 0x0f, 0x05, 0xc3]
    }
}
pub struct GeneratedGraphicsProvider {
    image: Vec<u8>,
}
impl std::fmt::Debug for GeneratedGraphicsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GeneratedGraphicsProvider")
            .field("functions", &ENTRIES.len())
            .finish()
    }
}
impl GeneratedGraphicsProvider {
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    pub fn function_count(&self) -> usize {
        ENTRIES.len()
    }
    pub(crate) fn markers(&self) -> impl Iterator<Item = GraphicsMarker> {
        ENTRIES
            .iter()
            .enumerate()
            .map(|(index, (_, function, _, _))| GraphicsMarker {
                symbol: index + 1,
                function: *function,
            })
    }
}
/// Construct the one immutable scoped image; caller-defined NIDs, code and
/// service numbers cannot be substituted into its private manifest.
pub fn build_graphics_provider(
    maximum_image_bytes: u64,
) -> Result<GeneratedGraphicsProvider, KernelProviderError> {
    if maximum_image_bytes < IMAGE_BYTES as u64 {
        return Err(KernelProviderError::Budget);
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(IMAGE_BYTES)
        .map_err(|_| KernelProviderError::Allocation)?;
    if image.capacity() as u64 > maximum_image_bytes {
        return Err(KernelProviderError::Budget);
    }
    image.resize(IMAGE_BYTES, 0);
    put(&mut image, 0, b"\x7fELF\x02\x01\x01\0");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 2)] {
        put(&mut image, at, &value.to_le_bytes());
    }
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    program(
        &mut image,
        0,
        1,
        5,
        [0, 0, IMAGE_BYTES as u64, PAGE_SIZE, PAGE_SIZE],
    );
    program(
        &mut image,
        1,
        2,
        4,
        [
            DYNAMIC_AT as u64,
            DYNAMIC_AT as u64,
            DYNAMIC_COUNT * 16,
            DYNAMIC_COUNT * 16,
            8,
        ],
    );
    let mut name_at = 1;
    let mut name_offsets = [0; NAMES.len()];
    for (index, name) in NAMES.iter().enumerate() {
        name_offsets[index] = name_at as u64;
        put(&mut image, STRINGS_AT + name_at, name.as_bytes());
        name_at += name.len() + 1;
    }
    if STRINGS_AT + name_at + ENTRIES.len() * 16 > SYMBOLS_AT {
        return Err(KernelProviderError::InvalidCatalogue);
    }
    for (index, (nid, function, library, module)) in ENTRIES.iter().enumerate() {
        let marker = GraphicsMarker {
            symbol: index + 1,
            function: *function,
        };
        put(&mut image, marker.rva() as usize, &marker.bytes());
        put(&mut image, STRINGS_AT + name_at, nid.as_bytes());
        put(
            &mut image,
            STRINGS_AT + name_at + 11,
            &[b'#', b'B' + *library as u8, b'#', b'B' + *module as u8],
        );
        let row = SYMBOLS_AT + (index + 1) * 24;
        put(&mut image, row, &(name_at as u32).to_le_bytes());
        image[row + 4] = 0x12;
        image[row + 5] = 3;
        put(&mut image, row + 6, &1_u16.to_le_bytes());
        put(&mut image, row + 8, &marker.rva().to_le_bytes());
        put(&mut image, row + 16, &8_u64.to_le_bytes());
        name_at += 16;
    }
    for (index, (tag, value)) in [
        (5_i64, STRINGS_AT as u64),
        (10, name_at as u64),
        (6, SYMBOLS_AT as u64),
        (39, (ENTRIES.len() as u64 + 1) * 24),
        (11, 24),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut image, DYNAMIC_AT + index * 16, &tag.to_le_bytes());
        put(
            &mut image,
            DYNAMIC_AT + 8 + index * 16,
            &value.to_le_bytes(),
        );
    }
    for (index, offset) in name_offsets.into_iter().enumerate() {
        let id = (index as u64 + 1) << 48;
        for (entry, tag, value) in [
            (
                5 + index * 2,
                DT_SCE_MODULE_INFO,
                offset | (1 << 32) | (1 << 40) | id,
            ),
            (6 + index * 2, DT_SCE_EXPORT_LIB, offset | (1 << 32) | id),
        ] {
            put(&mut image, DYNAMIC_AT + entry * 16, &tag.to_le_bytes());
            put(
                &mut image,
                DYNAMIC_AT + 8 + entry * 16,
                &value.to_le_bytes(),
            );
        }
    }
    Ok(GeneratedGraphicsProvider { image })
}
fn put(image: &mut [u8], at: usize, bytes: &[u8]) {
    image[at..at + bytes.len()].copy_from_slice(bytes);
}
