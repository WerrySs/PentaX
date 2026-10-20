// SPDX-License-Identifier: GPL-2.0-or-later

//! Original generated ET_DYN provider for the fixed catalogue.
//!
//! Layout: one R+X code page, one R+W/NX data page, one PT_DYNAMIC,
//! standard virtual-address dynamic tables, classic ModuleInfo/ExportLibrary
//! descriptors, a genuine null symbol, seventeen GLOBAL/PROTECTED function
//! markers, forty-five additional exact function identities and four object
//! identities sharing one process-owned dependency flag. Function aliases
//! share their canonical eight-byte `MOV EAX, service_id; SYSCALL; RET` stub.
//! There is no entry point, TLS,
//! relocation, firmware byte or SDK-derived code.

use super::catalog::{
    EXT_LIBRARY_NAME, KnownFunction, LIBRARY_NAME, LIBRARY_SELECTOR, LIBRARY_VERSION, MODULE_MAJOR,
    MODULE_MINOR, MODULE_NAME, MODULE_SELECTOR, SCE_EXT_LIBRARY_NAME, SCE_LIBRARY_NAME,
    SCE_MODULE_NAME,
};
use super::{HleBudget, HleError, HleLimits, HleProfile};
use crate::memory::PAGE_SIZE;
use pentax_linker::{DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO, NID_TOKEN_LEN};
use std::fmt;

/// Code and process-owned trace data occupy separate logical pages.
pub(super) const PROVIDER_SPAN: u64 = 2 * PAGE_SIZE;
pub(super) const HEAP_TRACE_RVA: u64 = PAGE_SIZE;
pub(super) const HEAP_TRACE_BYTES: u64 = 8 + 64 * 8;
/// Pinned Kyty's uint32_t LibcInternal::g_need_flag, initially one.
/// See docs/upstream-provenance.md; this is not a callable export or host pointer.
pub(super) const NEED_FLAG_NID: &[u8; 11] = b"ZT4ODD2Ts9o";
pub(super) const NEED_FLAG_RVA: u64 = HEAP_TRACE_RVA + HEAP_TRACE_BYTES;
pub(super) const NEED_FLAG_BYTES: u64 = 4;
// Private ELF symbol rows: canonical functions 1..=17, objects 18..=21,
// and finite function aliases 22..=66. The old sixteen stubs and all data RVAs
// remain fixed; adding a canonical function shifts only these metadata rows.
pub(super) const NEED_FLAG_SYMBOL: usize = 1 + KnownFunction::ALL.len();
pub(super) const NEED_FLAG_SYMBOLS: usize = 4;
pub(super) const LIBC_ALIAS_SYMBOL: usize = NEED_FLAG_SYMBOL + NEED_FLAG_SYMBOLS;
const LIBC_ALIAS_SYMBOLS: usize = KnownFunction::LIBC_FUNCTIONS.len() * 3;
pub(super) const DATA_BYTES: u64 = HEAP_TRACE_BYTES + NEED_FLAG_BYTES;
pub(super) const STUB_SIZE: u64 = 8;
/// SYSCALL offset inside a stub; the interpreter stops after it at `RESUME`.
pub(super) const TRAP_OFFSET: u64 = 5;
pub(super) const RESUME_OFFSET: u64 = 7;

pub(super) const CODE_SIZE: usize = 0x1000;
const IMAGE_SIZE: usize = CODE_SIZE + DATA_BYTES as usize;
const PROGRAM_HEADERS: usize = 3;
const STUBS_AT: usize = 0x100;
const STUB_STRIDE: usize = 0x10;
const DYNAMIC_AT: usize = 0x300;
const STRINGS_AT: usize = 0x400;
pub(super) const SYMBOLS_AT: usize = 0x900;
pub(super) const SYMBOL_SIZE: usize = 24;
const SYMBOLS: usize = 1 + KnownFunction::ALL.len() + NEED_FLAG_SYMBOLS + LIBC_ALIAS_SYMBOLS;
const DYNAMIC_ENTRIES: usize = 12;
/// `NID#B#B`: canonical selector 1 for both scopes, plus a NUL terminator.
const QUALIFIED_LEN: usize = NID_TOKEN_LEN + 5;

/// The independently encoded stub for one catalogue entry.
pub(super) fn stub(function: KnownFunction) -> [u8; STUB_SIZE as usize] {
    let [a, b, c, d] = function.service_id().to_le_bytes();
    [0xb8, a, b, c, d, 0x0f, 0x05, 0xc3]
}

/// Only canonical function rows and the factory's finite alias rows identify
/// a marker. Null, data-object and out-of-range rows never authorize a service.
pub(super) fn function_for_symbol(symbol: usize) -> Option<KnownFunction> {
    if (1..=KnownFunction::ALL.len()).contains(&symbol) {
        return Some(KnownFunction::ALL[symbol - 1]);
    }
    let offset = symbol.checked_sub(LIBC_ALIAS_SYMBOL)?;
    KnownFunction::LIBC_FUNCTIONS.get(offset / 3).copied()
}

/// Private manifest fixed by the factory, never accepted from a caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ManifestEntry {
    pub function: KnownFunction,
    pub symbol: usize,
    pub stub_rva: u64,
}

/// Immutable generated provider bytes and their private fixed manifest.
///
/// There is no constructor besides [`build_hle_providers`], no mutable byte
/// access and no registration-injection API. Read-only bytes may be inspected;
/// supplying a copy as an ordinary input creates an ordinary competing provider.
pub struct GeneratedProviders {
    profile: HleProfile,
    image: Vec<u8>,
    manifest: [ManifestEntry; KnownFunction::ALL.len()],
}

impl fmt::Debug for GeneratedProviders {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedProviders")
            .field("profile", &self.profile)
            .field("image_len", &self.image.len())
            .finish_non_exhaustive()
    }
}

impl GeneratedProviders {
    #[must_use]
    pub const fn profile(&self) -> HleProfile {
        self.profile
    }
    #[must_use]
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    #[must_use]
    pub const fn function_count(&self) -> usize {
        self.manifest.len()
    }
    pub(super) const fn manifest(&self) -> &[ManifestEntry; KnownFunction::ALL.len()] {
        &self.manifest
    }
}

const fn manifest() -> [ManifestEntry; KnownFunction::ALL.len()] {
    let mut entries = [ManifestEntry {
        function: KnownFunction::Memcpy,
        symbol: 0,
        stub_rva: 0,
    }; KnownFunction::ALL.len()];
    let mut index = 0;
    while index < entries.len() {
        entries[index] = ManifestEntry {
            function: KnownFunction::ALL[index],
            symbol: index + 1,
            stub_rva: (STUBS_AT + index * STUB_STRIDE) as u64,
        };
        index += 1;
    }
    entries
}

/// Fixed string-table offsets: module name, library name, then qualified names.
const fn string_offsets() -> (usize, usize, usize, usize) {
    let module = 1;
    let library = module + MODULE_NAME.len() + 1;
    let first_symbol = library
        + LIBRARY_NAME.len()
        + 1
        + EXT_LIBRARY_NAME.len()
        + 1
        + SCE_MODULE_NAME.len()
        + 1
        + SCE_EXT_LIBRARY_NAME.len()
        + 1
        + SCE_LIBRARY_NAME.len()
        + 1;
    let end = first_symbol + (SYMBOLS - 1) * QUALIFIED_LEN;
    (module, library, first_symbol, end)
}

fn check_layout(limits: &HleLimits) -> Result<(), HleError> {
    let (_, _, _, strings_len) = string_offsets();
    let ordered = [
        (64 + PROGRAM_HEADERS * 56, STUBS_AT),
        (
            STUBS_AT + (KnownFunction::ALL.len() - 1) * STUB_STRIDE + STUB_SIZE as usize,
            DYNAMIC_AT,
        ),
        (DYNAMIC_AT + DYNAMIC_ENTRIES * 16, STRINGS_AT),
        (STRINGS_AT + strings_len, SYMBOLS_AT),
        (SYMBOLS_AT + SYMBOLS * SYMBOL_SIZE, CODE_SIZE),
    ];
    if ordered.iter().any(|&(end, next)| end > next) || IMAGE_SIZE as u64 > PROVIDER_SPAN {
        return Err(HleError::InvalidProvider("fixed provider layout"));
    }
    if IMAGE_SIZE as u64 > limits.max_provider_bytes {
        return Err(HleError::LimitExceeded {
            kind: HleBudget::ProviderBytes,
            requested: IMAGE_SIZE as u64,
            maximum: limits.max_provider_bytes,
        });
    }
    Ok(())
}

/// Generate the profile's original provider image.
///
/// The complete fixed layout and the provider byte budget are checked before
/// the single fallible image reservation. All registration values come from
/// the fixed catalogue.
///
/// # Errors
/// Returns `LimitExceeded` for an insufficient provider budget, `Allocation`
/// when the image cannot be reserved and `InvalidProvider` for a layout fault.
pub fn build_hle_providers(
    profile: HleProfile,
    limits: &HleLimits,
) -> Result<GeneratedProviders, HleError> {
    // The single reviewed catalogue; a future profile must be added explicitly.
    let HleProfile::KytyObservedLibcInternalV1 = profile;
    check_layout(limits)?;
    let mut image = Vec::new();
    image
        .try_reserve_exact(IMAGE_SIZE)
        .map_err(|_| HleError::Allocation)?;
    image.resize(IMAGE_SIZE, 0);
    let manifest = manifest();
    write_image(&mut image, &manifest);
    Ok(GeneratedProviders {
        profile,
        image,
        manifest,
    })
}

fn write_image(bytes: &mut [u8], manifest: &[ManifestEntry; KnownFunction::ALL.len()]) {
    bytes[..8].copy_from_slice(b"\x7fELF\x02\x01\x01\x00");
    put(bytes, 16, &3_u16.to_le_bytes()); // ET_DYN
    put(bytes, 18, &62_u16.to_le_bytes());
    put(bytes, 20, &1_u32.to_le_bytes());
    put(bytes, 24, &0_u64.to_le_bytes()); // No provider entry.
    put(bytes, 32, &64_u64.to_le_bytes());
    put(bytes, 52, &64_u16.to_le_bytes());
    put(bytes, 54, &56_u16.to_le_bytes());
    put(bytes, 56, &(PROGRAM_HEADERS as u16).to_le_bytes());
    program(
        bytes,
        0,
        1,
        5,
        [0, 0, CODE_SIZE as u64, PAGE_SIZE, PAGE_SIZE],
    );

    program(
        bytes,
        2,
        1,
        6,
        [CODE_SIZE as u64, HEAP_TRACE_RVA, DATA_BYTES, PAGE_SIZE, 8],
    );
    let dynamic_size = (DYNAMIC_ENTRIES * 16) as u64;
    program(
        bytes,
        1,
        2,
        4,
        [
            DYNAMIC_AT as u64,
            DYNAMIC_AT as u64,
            dynamic_size,
            dynamic_size,
            8,
        ],
    );

    let (module_name, library_name, first_symbol, strings_len) = string_offsets();
    put(bytes, STRINGS_AT + module_name, MODULE_NAME);
    put(bytes, STRINGS_AT + library_name, LIBRARY_NAME);
    let extension_name = library_name + LIBRARY_NAME.len() + 1;
    put(bytes, STRINGS_AT + extension_name, EXT_LIBRARY_NAME);
    let sce_module_name = extension_name + EXT_LIBRARY_NAME.len() + 1;
    let sce_library_name = sce_module_name + SCE_MODULE_NAME.len() + 1;
    let need_sce_library_name = sce_library_name + SCE_EXT_LIBRARY_NAME.len() + 1;
    put(bytes, STRINGS_AT + sce_module_name, SCE_MODULE_NAME);
    put(bytes, STRINGS_AT + sce_library_name, SCE_EXT_LIBRARY_NAME);
    put(bytes, STRINGS_AT + need_sce_library_name, SCE_LIBRARY_NAME);
    for entry in manifest {
        let name = first_symbol + (entry.symbol - 1) * QUALIFIED_LEN;
        put(bytes, STRINGS_AT + name, entry.function.nid());
        put(
            bytes,
            STRINGS_AT + name + NID_TOKEN_LEN,
            if entry.function.library_selector() == 3 {
                b"#D#C"
            } else if entry.function.library_selector() == 2 {
                b"#C#B"
            } else {
                b"#B#B"
            },
        );
        // Row zero stays the genuine all-zero null symbol.
        let row = SYMBOLS_AT + entry.symbol * SYMBOL_SIZE;
        put(bytes, row, &(name as u32).to_le_bytes());
        bytes[row + 4] = 0x12; // GLOBAL FUNC
        bytes[row + 5] = 3; // PROTECTED
        put(bytes, row + 6, &1_u16.to_le_bytes());
        put(bytes, row + 8, &entry.stub_rva.to_le_bytes());
        put(bytes, row + 16, &STUB_SIZE.to_le_bytes());
        put(bytes, entry.stub_rva as usize, &stub(entry.function));
    }
    // Pinned Kyty strips libSce independently from module and library names.
    // Register only this reviewed object's finite pairs; never normalize keys.
    for (alias, suffix) in [b"#B#B", b"#E#C", b"#B#C", b"#E#B"].into_iter().enumerate() {
        let flag_name = first_symbol + (KnownFunction::ALL.len() + alias) * QUALIFIED_LEN;
        put(bytes, STRINGS_AT + flag_name, NEED_FLAG_NID);
        put(bytes, STRINGS_AT + flag_name + NID_TOKEN_LEN, suffix);
        let flag_row = SYMBOLS_AT + (NEED_FLAG_SYMBOL + alias) * SYMBOL_SIZE;
        put(bytes, flag_row, &(flag_name as u32).to_le_bytes());
        bytes[flag_row + 4] = 0x11; // GLOBAL OBJECT, distinct from every marker.
        bytes[flag_row + 5] = 3; // PROTECTED, PentaX factory metadata policy.
        put(bytes, flag_row + 6, &1_u16.to_le_bytes());
        put(bytes, flag_row + 8, &NEED_FLAG_RVA.to_le_bytes());
        put(bytes, flag_row + 16, &NEED_FLAG_BYTES.to_le_bytes());
    }
    put(
        bytes,
        CODE_SIZE + HEAP_TRACE_BYTES as usize,
        &1_u32.to_le_bytes(),
    );
    for (position, function) in KnownFunction::LIBC_FUNCTIONS.into_iter().enumerate() {
        for (alias, suffix) in [b"#E#C", b"#B#C", b"#E#B"].into_iter().enumerate() {
            let symbol = LIBC_ALIAS_SYMBOL + position * 3 + alias;
            let name = first_symbol + (symbol - 1) * QUALIFIED_LEN;
            put(bytes, STRINGS_AT + name, function.nid());
            put(bytes, STRINGS_AT + name + NID_TOKEN_LEN, suffix);
            let row = SYMBOLS_AT + symbol * SYMBOL_SIZE;
            put(bytes, row, &(name as u32).to_le_bytes());
            bytes[row + 4] = 0x12;
            bytes[row + 5] = 3;
            put(bytes, row + 6, &1_u16.to_le_bytes());
            let rva = (STUBS_AT + function.index() * STUB_STRIDE) as u64;
            put(bytes, row + 8, &rva.to_le_bytes());
            put(bytes, row + 16, &STUB_SIZE.to_le_bytes());
        }
    }

    let module_word = module_name as u64
        | (u64::from(MODULE_MINOR) << 32)
        | (u64::from(MODULE_MAJOR) << 40)
        | (u64::from(MODULE_SELECTOR) << 48);
    let library_word = library_name as u64
        | (u64::from(LIBRARY_VERSION) << 32)
        | (u64::from(LIBRARY_SELECTOR) << 48);
    let entries: [(i64, u64); DYNAMIC_ENTRIES] = [
        (5, STRINGS_AT as u64),
        (10, strings_len as u64),
        (6, SYMBOLS_AT as u64),
        (39, (SYMBOLS * SYMBOL_SIZE) as u64),
        (11, SYMBOL_SIZE as u64),
        (DT_SCE_MODULE_INFO, module_word),
        (
            DT_SCE_MODULE_INFO,
            sce_module_name as u64
                | (u64::from(MODULE_MINOR) << 32)
                | (u64::from(MODULE_MAJOR) << 40)
                | (2 << 48),
        ),
        (DT_SCE_EXPORT_LIB, library_word),
        (
            DT_SCE_EXPORT_LIB,
            extension_name as u64 | (u64::from(LIBRARY_VERSION) << 32) | (2 << 48),
        ),
        (
            DT_SCE_EXPORT_LIB,
            sce_library_name as u64 | (u64::from(LIBRARY_VERSION) << 32) | (3 << 48),
        ),
        (
            DT_SCE_EXPORT_LIB,
            need_sce_library_name as u64 | (u64::from(LIBRARY_VERSION) << 32) | (4 << 48),
        ),
        (0, 0),
    ];
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(bytes, DYNAMIC_AT + index * 16, &tag.to_le_bytes());
        put(bytes, DYNAMIC_AT + index * 16 + 8, &value.to_le_bytes());
    }
}

pub(super) fn program(
    bytes: &mut [u8],
    index: usize,
    kind: u32,
    flags: u32,
    [offset, address, file_size, mem_size, alignment]: [u64; 5],
) {
    let at = 64 + index * 56;
    put(bytes, at, &kind.to_le_bytes());
    put(bytes, at + 4, &flags.to_le_bytes());
    put(bytes, at + 8, &offset.to_le_bytes());
    put(bytes, at + 16, &address.to_le_bytes());
    put(bytes, at + 32, &file_size.to_le_bytes());
    put(bytes, at + 40, &mem_size.to_le_bytes());
    put(bytes, at + 48, &alignment.to_le_bytes());
}

pub(super) fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

#[cfg(test)]
#[path = "provider/snprintf.rs"]
mod snprintf_tests;
