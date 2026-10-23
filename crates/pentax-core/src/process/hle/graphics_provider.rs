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
const ENTRIES: [(&str, Function, usize, usize); 283] = {
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
    let mut entries = [BASE_ENTRIES[0]; 283];
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
    entries[106] = ("YUeqkyT7mEQ", Function::AgcDcbSetFlip, 1, 1);
    entries[107] = ("YUeqkyT7mEQ", Function::AgcDcbSetFlip, 2, 2);
    let presentation = [
        ("HXzjK9yI30k", Function::VideoAddFlipEvent),
        ("-Ozn0F1AFRg", Function::VideoDeleteFlipEvent),
        ("Xru92wHJRmg", Function::VideoAddVblankEvent),
        ("oNOQn3knW6s", Function::VideoDeleteVblankEvent),
        ("U2JJtSqNKZI", Function::VideoGetEventId),
        ("rWUTcKdkUzQ", Function::VideoGetEventData),
        ("Mt4QHHkxkOc", Function::VideoGetEventCount),
        ("U46NwOiJpys", Function::VideoSubmitFlip),
        ("SbU3dwp80lQ", Function::VideoGetFlipStatus),
        ("zgXifHT9ErY", Function::VideoIsFlipPending),
    ];
    i = 0;
    while i < presentation.len() {
        let (nid, function) = presentation[i];
        entries[108 + i] = (nid, function, 3, 3);
        i += 1;
    }
    let builders = crate::gpu::driver::COMMAND_BUILDER_EXPORTS;
    let mut builder = 0;
    while builder < builders.len() {
        let (nid, function) = builders[builder];
        entries[118 + builder * 2] = (nid, function, 1, 1);
        entries[119 + builder * 2] = (nid, function, 2, 2);
        builder += 1;
    }
    entries[222] = ("1FZBKy8HeNU", Function::VideoGetVblankStatus, 3, 3);
    entries[223] = ("DYhhWbJSeRg", Function::VideoColorSettingsSetGamma, 3, 3);
    entries[224] = ("pv9CI5VC+R0", Function::VideoAdjustColor, 3, 3);
    let resets = crate::gpu::driver::RESET_QUEUE_EXPORTS;
    i = 0;
    while i < resets.len() {
        let (nid, function) = resets[i];
        entries[225 + i * 2] = (nid, function, 1, 1);
        entries[226 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let synchronization = crate::gpu::driver::MEMORY_SYNC_EXPORTS;
    i = 0;
    while i < synchronization.len() {
        let (nid, function) = synchronization[i];
        entries[229 + i * 2] = (nid, function, 1, 1);
        entries[230 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let markers = crate::gpu::driver::MARKER_EXPORTS;
    i = 0;
    while i < markers.len() {
        let (nid, function) = markers[i];
        entries[237 + i * 2] = (nid, function, 1, 1);
        entries[238 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let (nid, function) = crate::gpu::driver::CONTEXT_EXPORTS[0];
    entries[245] = (nid, function, 1, 1);
    entries[246] = (nid, function, 2, 2);
    entries[247] = ("D9sr1xGUriE", Function::AgcCreatePrimState, 1, 1);
    entries[248] = ("D9sr1xGUriE", Function::AgcCreatePrimState, 2, 2);
    entries[249] = (
        "V++UgBtQhn0",
        Function::AgcGetDataPacketPayloadAddress,
        1,
        1,
    );
    entries[250] = (
        "V++UgBtQhn0",
        Function::AgcGetDataPacketPayloadAddress,
        2,
        2,
    );
    entries[251] = ("-KRzWekV120", Function::AgcDcbSetIndexSize0600, 1, 1);
    entries[252] = ("-KRzWekV120", Function::AgcDcbSetIndexSize0600, 2, 2);
    let transfers = crate::gpu::driver::TRANSFER_EXPORTS;
    i = 0;
    while i < transfers.len() {
        let (nid, function) = transfers[i];
        entries[253 + i * 2] = (nid, function, 1, 1);
        entries[254 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let indirect = crate::gpu::driver::INDIRECT_DISPATCH_EXPORTS;
    i = 0;
    while i < indirect.len() {
        let (nid, function) = indirect[i];
        entries[261 + i * 2] = (nid, function, 1, 1);
        entries[262 + i * 2] = (nid, function, 2, 2);
        i += 1;
    }
    let (nid, function) = crate::gpu::driver::INDIRECT_DRAW_EXPORTS[0];
    entries[267] = (nid, function, 1, 1);
    entries[268] = (nid, function, 2, 2);
    // Exact appended producer; all prior269 marker IDs and scopes stay fixed.
    entries[269] = (
        "MWiElSNE8j8",
        Function::AgcDcbWaitUntilSafeForRendering,
        1,
        1,
    );
    entries[270] = (
        "MWiElSNE8j8",
        Function::AgcDcbWaitUntilSafeForRendering,
        2,
        2,
    );
    // Exact paired statistics producer; all preceding marker ordinals stay fixed.
    entries[271] = ("vuSXe69VILM", Function::AgcDcbGetLodStats, 1, 1);
    entries[272] = ("vuSXe69VILM", Function::AgcDcbGetLodStats, 2, 2);
    // Explicit same-ABI paired VideoOut spellings. Existing273 marker IDs,
    // function ordinals and RVAs remain unchanged; no mixed/NID-only fallback.
    entries[273] = ("zgXifHT9ErY", Function::VideoIsFlipPending, 4, 4);
    entries[274] = ("HXzjK9yI30k", Function::VideoAddFlipEvent, 4, 4);
    entries[275] = ("1FZBKy8HeNU", Function::VideoGetVblankStatus, 4, 4);
    entries[276] = ("DYhhWbJSeRg", Function::VideoColorSettingsSetGamma, 4, 4);
    entries[277] = ("pv9CI5VC+R0", Function::VideoAdjustColor, 4, 4);
    // Exact same-ABI paired driver spelling; all prior278 markers stay fixed.
    entries[278] = ("gSRnr79F8tQ", Function::Acb, 6, 6);
    // The public direct Packet ABI is distinct from context + Packet submits.
    entries[279] = ("UglJIZjGssM", Function::Dcb, 0, 0);
    entries[280] = ("UglJIZjGssM", Function::Dcb, 6, 6);
    entries[281] = ("AhGvpITrf4M", Function::Dcb, 0, 0);
    entries[282] = ("AhGvpITrf4M", Function::Dcb, 6, 6);
    entries
};
const BASE: u32 = 0x5058_4700;
// Appended immutable metadata stays inside the existing one-page mapping and
// its unchanged16KiB budget. No earlier marker ID/RVA is moved.
const IMAGE_BYTES: usize = PAGE_SIZE as usize;
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
// The two short driver/video names are suffixes of existing complete names.
// Only these exact bounded names are interned; no caller controls offsets.
const NAME_BYTES: usize = 68;
const NAMESPACE_BYTES: &[u8] =
    b"\0Graphics5Driver\0Graphics5\0libSceAgc\0libSceVideoOut\0libSceAgcDriver\0";
const NAME_OFFSETS: [usize; 7] = [1, 17, 27, 43, 37, 58, 52];
const fn align_metadata(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}
// Derive disjoint metadata from the fixed catalogue; appending exact exports
// never relocates existing marker IDs/RVAs or bypasses the image-size bound.
const OLD_MARKERS: usize = 279;
const STRINGS_AT: usize = 64 + 2 * 56;
const DYNAMIC_AT: usize = align_metadata(0x100 + OLD_MARKERS * 16, 8);
const NIDS_AT: usize = align_metadata(DYNAMIC_AT + DYNAMIC_COUNT as usize * 16, 8);
const SYMBOLS_AT: usize = align_metadata(NIDS_AT + ENTRIES.len() * 16, 8);
const _: () = {
    assert!(NAMESPACE_BYTES.len() == NAME_BYTES);
    assert!(STRINGS_AT + NAME_BYTES <= 0x100);
    assert!(ENTRIES.len() - OLD_MARKERS == 4);
    assert!(0x100 + OLD_MARKERS * 16 <= DYNAMIC_AT);
    assert!(DYNAMIC_AT + DYNAMIC_COUNT as usize * 16 <= NIDS_AT);
    assert!(NIDS_AT + ENTRIES.len() * 16 <= SYMBOLS_AT);
    assert!(SYMBOLS_AT + (ENTRIES.len() + 1) * 24 <= IMAGE_BYTES);
};

#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphicsMarker {
    pub symbol: usize,
    pub function: Function,
}
impl GraphicsMarker {
    pub fn rva(self) -> u64 {
        if self.symbol <= OLD_MARKERS {
            0x100 + (self.symbol as u64 - 1) * 16
        } else {
            // Four new eight-byte stubs occupy only the unused second halves
            // of old slots 0..3. Every previous marker byte and RVA is retained.
            0x108 + (self.symbol as u64 - OLD_MARKERS as u64 - 1) * 16
        }
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
    put(&mut image, STRINGS_AT, NAMESPACE_BYTES);
    let name_offsets = NAME_OFFSETS.map(|offset| offset as u64);
    let mut name_at = NIDS_AT - STRINGS_AT;
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

#[cfg(test)]
mod flip_tests {
    use super::*;
    #[test]
    fn appended_flip_markers_keep_prior_ordinals_scopes_and_fixed_image_bound() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(provider.image().len(), PAGE_SIZE as usize);
        let old_last = provider.markers().nth(105).unwrap();
        assert_eq!((old_last.id(), old_last.rva()), (0x5058_476a, 0x790));
        assert_eq!(old_last.function, Function::AgcCreateInterpolantMapping2);
        for (index, rva, id, scope) in [
            (106, 0x7a0, 0x5058_476bu32, 1),
            (107, 0x7b0, 0x5058_476c, 2),
        ] {
            assert_eq!(
                ENTRIES[index],
                ("YUeqkyT7mEQ", Function::AgcDcbSetFlip, scope, scope)
            );
            let marker = provider.markers().nth(index).unwrap();
            assert_eq!((marker.rva(), marker.id()), (rva, id));
            let id = id.to_le_bytes();
            assert_eq!(
                &provider.image()[rva as usize..rva as usize + 8],
                &[0xb8, id[0], id[1], id[2], id[3], 15, 5, 0xc3]
            );
        }
        assert!(matches!(
            build_graphics_provider(PAGE_SIZE - 1),
            Err(KernelProviderError::Budget)
        ));
    }

    #[test]
    fn builder_exports_append_only_two_exact_pairs_with_typed_live_markers() {
        use pentax_linker::{Limits, SceLimits, SymbolBinding, SymbolType, inspect_sce};
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        let view = pentax_loader::open_executable(provider.image()).unwrap();
        let metadata = inspect_sce(&view, &Limits::DEFAULT, &SceLimits::DEFAULT)
            .unwrap()
            .into_metadata()
            .unwrap();
        let descriptors = metadata.descriptors().decoded().unwrap();
        for (index, name) in [(1, b"Graphics5".as_slice()), (2, b"libSceAgc".as_slice())] {
            let module = &descriptors.module_info()[index];
            let library = &descriptors.export_libraries()[index];
            assert_eq!(module.name().as_bytes(), name);
            assert_eq!((module.version_major(), module.version_minor()), (1, 1));
            assert_eq!(library.name().as_bytes(), name);
            assert_eq!(library.version(), 1);
        }
        let previous = provider.markers().nth(117).unwrap();
        assert_eq!((previous.rva(), previous.id()), (0x850, 0x5058_4776));
        // The prior thirty builder identities keep all sixty marker rows.
        let old_last = provider.markers().nth(177).unwrap();
        assert_eq!((old_last.rva(), old_last.id()), (0xc10, 0x5058_47b2));
        assert_eq!(old_last.function, Function::AgcDcbStallCommandBufferParser);
        for (index, (nid, function)) in crate::gpu::driver::COMMAND_BUILDER_EXPORTS
            .into_iter()
            .enumerate()
        {
            assert_eq!(ENTRIES.iter().filter(|entry| entry.0 == nid).count(), 2);
            for (pair, suffix) in [(1, "#C#C"), (2, "#D#D")] {
                let ordinal = 118 + index * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let symbol = &metadata.link().symbols()[ordinal + 1];
                assert_eq!(
                    symbol.name.as_ref().unwrap().as_bytes(),
                    format!("{nid}{suffix}").as_bytes()
                );
                assert_eq!(symbol.binding(), SymbolBinding::GLOBAL);
                assert_eq!(symbol.symbol_type(), SymbolType::FUNC);
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(symbol.value, marker.rva());
                assert_eq!(
                    &provider.image()[marker.rva() as usize..marker.rva() as usize + 8],
                    &marker.bytes()
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "graphics_provider/presentation_tests.rs"]
mod presentation_tests;

#[cfg(test)]
#[path = "graphics_provider/direct_dcb_tests.rs"]
mod direct_dcb_tests;

#[cfg(test)]
mod reset_queue_tests {
    use super::*;
    #[test]
    fn reset_bindings_append_only_two_verified_pairs_after_all_old_graphics_markers() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[224],
            ("pv9CI5VC+R0", Function::VideoAdjustColor, 3, 3)
        );
        for (offset, (nid, function)) in crate::gpu::driver::RESET_QUEUE_EXPORTS
            .into_iter()
            .enumerate()
        {
            assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
            for pair in [1, 2] {
                let ordinal = 225 + offset * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
                assert_eq!(marker.function, function);
            }
        }
    }
}

#[cfg(test)]
mod memory_sync_tests {
    use super::*;
    #[test]
    fn four_sync_roles_append_only_verified_pairs_after_all_reset_markers() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[228],
            ("TRO721eVt4g", Function::AgcDcbResetQueue, 2, 2)
        );
        for (index, (nid, function)) in crate::gpu::driver::MEMORY_SYNC_EXPORTS
            .into_iter()
            .enumerate()
        {
            assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
            for pair in [1, 2] {
                let ordinal = 229 + index * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
                assert_eq!(marker.function, function);
            }
        }
    }
}

#[cfg(test)]
mod marker_binding_tests {
    use super::*;
    #[test]
    fn four_marker_roles_append_only_verified_pairs_after_reset_rows() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[228],
            ("TRO721eVt4g", Function::AgcDcbResetQueue, 2, 2)
        );
        for (index, (nid, function)) in crate::gpu::driver::MARKER_EXPORTS.into_iter().enumerate() {
            assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
            for pair in [1, 2] {
                let ordinal = 237 + index * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
                assert_eq!(marker.function, function);
            }
        }
    }
}

#[cfg(test)]
mod context_binding_tests {
    use super::*;
    #[test]
    fn context_operation_appends_exact_canonical_and_observed_pair_without_moving_old_markers() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[244],
            ("H7uZqCoNuWk", Function::AgcDcbPopMarker, 2, 2)
        );
        for (ordinal, pair) in [(245, 1), (246, 2)] {
            assert_eq!(
                ENTRIES[ordinal],
                ("qj7QZpgr9Uw", Function::AgcDcbContextStateOp, pair, pair)
            );
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
            assert_eq!(marker.function, Function::AgcDcbContextStateOp);
        }
        assert_eq!(
            ENTRIES
                .iter()
                .filter(|entry| entry.0 == "qj7QZpgr9Uw")
                .count(),
            2
        );
        assert!(!ENTRIES.iter().any(|entry| entry.0 == "H6vHS5cidSA"));
    }
}

#[cfg(test)]
mod primitive_binding_tests {
    use super::*;
    #[test]
    fn primitive_creation_appends_only_exact_canonical_and_observed_pairs_after_context_state() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[246],
            ("qj7QZpgr9Uw", Function::AgcDcbContextStateOp, 2, 2)
        );
        for (ordinal, pair) in [(247, 1), (248, 2)] {
            assert_eq!(
                ENTRIES[ordinal],
                ("D9sr1xGUriE", Function::AgcCreatePrimState, pair, pair)
            );
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
            assert_eq!(marker.function, Function::AgcCreatePrimState);
        }
        assert_eq!(
            ENTRIES
                .iter()
                .filter(|entry| entry.0 == "D9sr1xGUriE")
                .count(),
            2
        );
        assert!(!ENTRIES.iter().any(|entry| entry.0 == "Y3ymLfZ1384"));
    }
}

#[cfg(test)]
mod packet_payload_binding_tests {
    use super::*;
    #[test]
    fn packet_payload_address_appends_only_exact_pairs_and_preserves_primitive_markers() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[248],
            ("D9sr1xGUriE", Function::AgcCreatePrimState, 2, 2)
        );
        for (ordinal, pair) in [(249, 1), (250, 2)] {
            assert_eq!(
                ENTRIES[ordinal],
                (
                    "V++UgBtQhn0",
                    Function::AgcGetDataPacketPayloadAddress,
                    pair,
                    pair
                )
            );
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
            assert_eq!(marker.function, Function::AgcGetDataPacketPayloadAddress);
        }
        assert_eq!(
            ENTRIES
                .iter()
                .filter(|entry| entry.0 == "V++UgBtQhn0")
                .count(),
            2
        );
        assert!(!ENTRIES.iter().any(|entry| entry.0 == "s+VGAMDQ0AQ"));
    }
}

#[cfg(test)]
mod index_size_0600_binding_tests {
    use super::*;
    #[test]
    fn index_size_0600_appends_exact_pairs_without_moving_old_builder_or_payload_rows() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[250],
            (
                "V++UgBtQhn0",
                Function::AgcGetDataPacketPayloadAddress,
                2,
                2
            )
        );
        assert_eq!(
            crate::gpu::driver::COMMAND_BUILDER_EXPORTS[4],
            ("GIIW2J37e70", Function::AgcDcbSetIndexSize)
        );
        for (ordinal, pair) in [(251, 1), (252, 2)] {
            assert_eq!(
                ENTRIES[ordinal],
                ("-KRzWekV120", Function::AgcDcbSetIndexSize0600, pair, pair)
            );
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
            assert_eq!(marker.function, Function::AgcDcbSetIndexSize0600);
        }
        assert_eq!(
            ENTRIES
                .iter()
                .filter(|entry| entry.0 == "-KRzWekV120")
                .count(),
            2
        );
    }
}

#[cfg(test)]
mod transfer_binding_tests {
    use super::*;
    #[test]
    fn four_transfer_roles_append_after_253_immutable_rows_in_exact_verified_pairs() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[252],
            ("-KRzWekV120", Function::AgcDcbSetIndexSize0600, 2, 2)
        );
        for (index, (nid, function)) in crate::gpu::driver::TRANSFER_EXPORTS.into_iter().enumerate()
        {
            assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
            for pair in [1, 2] {
                let ordinal = 253 + index * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
                assert_eq!(marker.function, function);
            }
        }
    }
}

#[cfg(test)]
mod indirect_dispatch_binding_tests {
    use super::*;
    #[test]
    fn three_indirect_producers_append_after_261_rows_in_only_verified_pairs() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(ENTRIES[260], ("WmAc2MEj6Io", Function::AgcDcbDmaData, 2, 2));
        for (index, (nid, function)) in [
            ("j3EtxFkSIhQ", Function::AgcAcbDispatchIndirect),
            ("CtB+A9-VxO0", Function::AgcDcbDispatchIndirect),
            ("RmaJwLtc8rY", Function::AgcDcbSetBaseIndirectArgs),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(ENTRIES.iter().filter(|row| row.0 == nid).count(), 2);
            for pair in [1, 2] {
                let ordinal = 261 + index * 2 + pair - 1;
                assert_eq!(ENTRIES[ordinal], (nid, function, pair, pair));
                let marker = provider.markers().nth(ordinal).unwrap();
                assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
                assert_eq!(marker.function, function);
            }
        }
        assert_eq!(crate::gpu::driver::COMMAND_BUILDER_EXPORTS.len(), 52);
        assert!(!ENTRIES.iter().any(|row| row.0 == "w8HVkEeXPv8"));
    }
}

#[cfg(test)]
mod indexed_indirect_binding_tests {
    use super::*;
    #[test]
    fn indexed_indirect_appends_only_exact_pairs_after_all_indirect_dispatch_rows() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        assert_eq!(
            ENTRIES[266],
            ("RmaJwLtc8rY", Function::AgcDcbSetBaseIndirectArgs, 2, 2)
        );
        for (ordinal, pair) in [(267, 1), (268, 2)] {
            assert_eq!(
                ENTRIES[ordinal],
                ("t1vNu082-jM", Function::AgcDcbDrawIndexIndirect, pair, pair)
            );
            let marker = provider.markers().nth(ordinal).unwrap();
            assert_eq!(marker.id(), BASE + ordinal as u32 + 1);
            assert_eq!(marker.function, Function::AgcDcbDrawIndexIndirect);
        }
        assert_eq!(
            ENTRIES.iter().filter(|row| row.0 == "t1vNu082-jM").count(),
            2
        );
        assert!(!ENTRIES.iter().any(|row| row.0 == "ypVBz4uPKcQ"));
    }
    #[test]
    fn wait_safe_appends_exact_scoped_producer_without_replacing_existing_ordinals() {
        let provider = build_graphics_provider(16384).unwrap();
        assert_eq!(provider.function_count(), 283);
        for (at, scope) in [(269, 1), (270, 2)] {
            assert_eq!(
                ENTRIES[at],
                (
                    "MWiElSNE8j8",
                    Function::AgcDcbWaitUntilSafeForRendering,
                    scope,
                    scope
                )
            );
            let marker = provider.markers().nth(at).unwrap();
            assert_eq!(marker.function, Function::AgcDcbWaitUntilSafeForRendering);
            assert_eq!(marker.id(), BASE + at as u32 + 1);
            assert_eq!(marker.rva(), 0x100 + at as u64 * 16);
        }
        assert_eq!(
            ENTRIES[267],
            ("t1vNu082-jM", Function::AgcDcbDrawIndexIndirect, 1, 1)
        );
        assert_eq!(
            ENTRIES[268],
            ("t1vNu082-jM", Function::AgcDcbDrawIndexIndirect, 2, 2)
        );
        assert_eq!(crate::gpu::driver::COMMAND_BUILDER_EXPORTS.len(), 52);
    }
    #[test]
    fn lod_stats_appends_only_exact_paired_producer_after_all_existing_ordinals() {
        let provider = build_graphics_provider(PAGE_SIZE).unwrap();
        assert_eq!(provider.function_count(), 283);
        for (at, scope) in [(271, 1), (272, 2)] {
            assert_eq!(
                ENTRIES[at],
                ("vuSXe69VILM", Function::AgcDcbGetLodStats, scope, scope)
            );
            let marker = provider.markers().nth(at).unwrap();
            assert_eq!(marker.id(), BASE + at as u32 + 1);
            assert_eq!(marker.function, Function::AgcDcbGetLodStats);
        }
        assert_eq!(
            ENTRIES[269],
            (
                "MWiElSNE8j8",
                Function::AgcDcbWaitUntilSafeForRendering,
                1,
                1
            )
        );
        assert_eq!(
            ENTRIES[270],
            (
                "MWiElSNE8j8",
                Function::AgcDcbWaitUntilSafeForRendering,
                2,
                2
            )
        );
        assert_eq!(crate::gpu::driver::COMMAND_BUILDER_EXPORTS.len(), 52);
    }
}
