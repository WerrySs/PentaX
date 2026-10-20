// SPDX-License-Identifier: GPL-2.0-only

//! Original ELF provider for exact pinned kernel and system-service catalogues.
//!
//! These are private PentaX service markers, never native kernel syscall
//! numbers. The dispatcher must stop before SYSCALL, retain the original callee
//! stack pointer and six SysV register arguments, attest the exact provider site,
//! and invoke the state-owning kernel implementation. Registration does not promise that an
//! operation's execution semantics are implemented; typed Unsupported remains
//! distinct from a successful guest return.

use crate::memory::PAGE_SIZE;
use pentax_kernel::{Registration, Service};
use pentax_linker::{DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO, NID_TOKEN_LEN, SelectorId};
use std::fmt;

const SERVICE_BASE: u32 = 0x5058_4b00;
// Independent count ceiling; the fixed byte layout below may reject sooner.
// Allocation and attestation always use the actual retained catalogue length.
const MAX_REGISTRATIONS: usize = 2048;
const STUB_START: usize = 0x100;
// Each closed marker is exactly eight bytes. Contiguous aligned markers keep
// reviewed exports/tables inside the bounded read-only source extent.
const STUB_STRIDE: usize = 8;
const STUB_SIZE: usize = 8;
// Pinned namespaces/spellings and the two explicit SSL lifecycle v2 bindings.
const MODULES: [(&str, (u16, u16)); 66] = [
    ("libkernel", (1, 1)),
    ("SystemService", (1, 1)),
    ("libSceSystemService", (1, 1)),
    ("Sysmodule", (1, 1)),
    ("libSceSysmodule", (1, 1)),
    ("Fiber", (1, 1)),
    ("libSceFiber", (1, 1)),
    ("CommonDialog", (1, 1)),
    ("libSceCommonDialog", (1, 1)),
    ("AppContentUtil", (1, 1)),
    ("libSceAppContentUtil", (1, 1)),
    ("UserService", (1, 1)),
    ("libSceUserService", (1, 1)),
    ("NpManager", (1, 1)),
    ("libSceNpManager", (1, 1)),
    ("SaveData", (1, 1)),
    ("libSceSaveData", (1, 1)),
    ("SaveData_native", (1, 1)),
    ("libSceSaveData_native", (1, 1)),
    ("Ampr", (1, 1)),
    ("libSceAmpr", (1, 1)),
    ("Pad", (1, 1)),
    ("libScePad", (1, 1)),
    ("NetCtl", (1, 1)),
    ("libSceNetCtl", (1, 1)),
    ("Rudp", (1, 1)),
    ("libSceRudp", (1, 1)),
    ("Net", (1, 1)),
    ("libSceNet", (1, 1)),
    ("Ssl", (1, 1)),
    ("libSceSsl", (1, 1)),
    ("Http", (1, 1)),
    ("libSceHttp", (1, 1)),
    ("libSceSsl", (2, 1)),
    ("Http2", (1, 1)),
    ("libSceHttp2", (1, 1)),
    ("NpWebApi2", (1, 1)),
    ("libSceNpWebApi2", (1, 1)),
    ("libSceLibcInternal", (1, 1)),
    ("NpGameIntent", (1, 1)),
    ("libSceNpGameIntent", (1, 1)),
    ("NpTrophy2", (1, 1)),
    ("libSceNpTrophy2", (1, 1)),
    ("NpUniversalDataSystem", (1, 1)),
    ("libSceNpUniversalDataSystem", (1, 1)),
    ("NpEntitlementAccess", (1, 1)),
    ("libSceNpEntitlementAccess", (1, 1)),
    ("Font", (1, 1)),
    ("libSceFont", (1, 1)),
    ("FontFt", (1, 1)),
    ("libSceFontFt", (1, 1)),
    ("Ime", (1, 1)),
    ("libSceIme", (1, 1)),
    ("Mouse", (1, 1)),
    ("libSceMouse", (1, 1)),
    ("Keyboard", (1, 1)),
    ("Share", (1, 1)),
    ("libSceShare", (1, 1)),
    ("ContentExport", (1, 1)),
    ("ContentSearch", (1, 0)),
    ("ContentDelete", (1, 1)),
    ("libSceContentExport", (1, 1)),
    ("PlayGo", (1, 0)),
    ("libScePlayGo", (1, 0)),
    ("Acm", (1, 1)),
    ("libSceAcm", (1, 1)),
];
const LIBRARIES: [(&str, u16); 67] = [
    ("libkernel", 1),
    ("Posix", 1),
    ("libScePosix", 1),
    ("SystemService", 1),
    ("libSceSystemService", 1),
    ("Sysmodule", 1),
    ("libSceSysmodule", 1),
    ("Fiber", 1),
    ("libSceFiber", 1),
    ("CommonDialog", 1),
    ("libSceCommonDialog", 1),
    ("AppContent", 1),
    ("libSceAppContent", 1),
    ("UserService", 1),
    ("libSceUserService", 1),
    ("NpManager", 1),
    ("libSceNpManager", 1),
    ("SaveData", 1),
    ("libSceSaveData", 1),
    ("SaveData_native", 1),
    ("libSceSaveData_native", 1),
    ("Ampr", 1),
    ("libSceAmpr", 1),
    ("Pad", 1),
    ("libScePad", 1),
    ("NetCtl", 1),
    ("libSceNetCtl", 1),
    ("Rudp", 1),
    ("libSceRudp", 1),
    ("Net", 1),
    ("libSceNet", 1),
    ("Ssl", 1),
    ("libSceSsl", 1),
    ("Http", 1),
    ("libSceHttp", 1),
    ("Http2", 1),
    ("libSceHttp2", 1),
    ("NpWebApi2", 1),
    ("libSceNpWebApi2", 1),
    ("libSceLibcInternal", 1),
    ("NpGameIntent", 1),
    ("libSceNpGameIntent", 1),
    ("NpTrophy2", 1),
    ("libSceNpTrophy2", 1),
    ("NpUniversalDataSystem", 1),
    ("libSceNpUniversalDataSystem", 1),
    ("NpEntitlementAccess", 1),
    ("libSceNpEntitlementAccess", 1),
    ("Font", 1),
    ("libSceFont", 1),
    ("FontFt", 1),
    ("libSceFontFt", 1),
    ("Ime", 1),
    ("libSceIme", 1),
    ("Mouse", 1),
    ("libSceMouse", 1),
    ("Keyboard", 1),
    ("Share", 1),
    ("libSceShare", 1),
    ("ContentExport", 1),
    ("ContentSearch", 1),
    ("ContentDelete", 1),
    ("libSceContentExport", 1),
    ("PlayGo", 1),
    ("libScePlayGo", 1),
    ("Acm", 1),
    ("libSceAcm", 1),
];
// Canonical selectors use one to three radix64 digits for their actual u16 ID.
// Namespace count, marker count and source-image bytes remain separate limits.
const _: () = assert!(MODULES.len() <= u16::MAX as usize && LIBRARIES.len() <= u16::MAX as usize);
const DYNAMIC_ENTRIES: usize = 9 + MODULES.len() + LIBRARIES.len();
const SYMBOL_SIZE: usize = 24;
// Markers, names, symbols and descriptors must fit the same four-page RX
// extent independently of the count ceiling. Oversized future catalogues
// refuse before allocation; the independent RW/NX object page never moves.
pub(super) const CODE_MAPPED_BYTES: u64 = 4 * PAGE_SIZE;
pub(super) const KERNEL_MAPPED_BYTES: u64 = CODE_MAPPED_BYTES + PAGE_SIZE;
pub(super) const STACK_GUARD_RVA: u64 = CODE_MAPPED_BYTES;
// Observed libKernel.cpp initial value; this guest object is not a host canary.
pub(super) const STACK_GUARD_INITIAL: u64 = 0xdead_beef_0000_0007;
const STACK_GUARD_NID: &[u8; 11] = b"f7uOxY9mM1U";
pub(super) const PROGRAM_NAME_OBJECT_RVA: u64 = STACK_GUARD_RVA + 8;
pub(super) const PROGRAM_NAME_BUFFER_RVA: u64 = STACK_GUARD_RVA + 16;
pub(super) const PROGRAM_NAME_CAPACITY: usize = 512;
const PROGRAM_NAME_NID: &[u8; 11] = b"djxxOmW6-aw";

#[derive(Clone, Copy, Debug)]
pub(super) struct KernelObject {
    pub symbol: usize,
    pub rva: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct KernelMarker {
    pub(crate) registration: &'static Registration,
    pub(crate) service_id: u32,
    pub(crate) symbol: usize,
    pub(crate) rva: u64,
}

impl KernelMarker {
    pub(crate) const fn bytes(self) -> [u8; STUB_SIZE] {
        let [a, b, c, d] = self.service_id.to_le_bytes();
        [0xb8, a, b, c, d, 0x0f, 0x05, 0xc3]
    }
}

/// A factory-owned catalogue and original immutable image. No constructor takes
/// a caller's registrations, service IDs or executable bytes.
pub struct GeneratedKernelProvider {
    image: Vec<u8>,
    markers: Vec<KernelMarker>,
    code_bytes: usize,
    data_offset: usize,
    objects: [KernelObject; 2],
}

impl fmt::Debug for GeneratedKernelProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedKernelProvider")
            .field("image_bytes", &self.image.len())
            .field("registrations", &self.markers.len())
            .finish_non_exhaustive()
    }
}

impl GeneratedKernelProvider {
    pub fn image(&self) -> &[u8] {
        &self.image
    }

    pub fn registration_count(&self) -> usize {
        self.markers.len()
    }

    pub(crate) fn markers(&self) -> &[KernelMarker] {
        &self.markers
    }

    pub(super) fn code_bytes(&self) -> &[u8] {
        &self.image[..self.code_bytes]
    }

    pub(super) fn data_bytes(&self) -> &[u8] {
        &self.image[self.data_offset..]
    }

    pub(super) const fn objects(&self) -> &[KernelObject; 2] {
        &self.objects
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernelProviderError {
    Budget,
    Allocation,
    InvalidCatalogue,
}

impl fmt::Display for KernelProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Budget => "kernel provider byte or registration budget exceeded",
            Self::Allocation => "kernel provider allocation failed",
            Self::InvalidCatalogue => "pinned kernel catalogue has an unsupported layout",
        })
    }
}

impl std::error::Error for KernelProviderError {}

/// Generate only the fixed, scoped catalogue. All dynamic tables and markers
/// fit inside four logical RX guest source pages. The fixed kernel objects use
/// a separate R-W/NX guest page, with no host pointer or execution permission.
/// Provider bytes alone are never an
/// execution capability or evidence of a resolved import.
pub fn build_kernel_provider(
    maximum_image_bytes: u64,
) -> Result<GeneratedKernelProvider, KernelProviderError> {
    build_kernel_provider_with_base(maximum_image_bytes, 1)
}

// Private variation permits original round-trip fixtures to exercise IDs beyond
// 63 without adding fake registrations or a caller-controlled catalogue.
fn build_kernel_provider_with_base(
    maximum_image_bytes: u64,
    selector_base: u16,
) -> Result<GeneratedKernelProvider, KernelProviderError> {
    let namespaces = MODULES.len().max(LIBRARIES.len());
    if selector_base == 0 || usize::from(selector_base) + namespaces - 1 > u16::MAX as usize {
        return Err(KernelProviderError::InvalidCatalogue);
    }
    let registrations = Service::registrations();
    if registrations.is_empty() || registrations.len() > MAX_REGISTRATIONS {
        return Err(KernelProviderError::Budget);
    }
    let first = &registrations[0];
    let valid_name = |name: &str| {
        !name.is_empty()
            && name.len() <= 255
            && name
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'#')
    };
    if !valid_name(first.module)
        || !valid_name(first.library)
        || first.module_version.0 > u8::MAX as u16
        || first.module_version.1 > u8::MAX as u16
    {
        return Err(KernelProviderError::InvalidCatalogue);
    }
    for (index, entry) in registrations.iter().enumerate() {
        if !MODULES.contains(&(entry.module, entry.module_version))
            || !LIBRARIES.contains(&(entry.library, entry.library_version))
            || entry.arguments > 9
            || entry.nid.len() != NID_TOKEN_LEN
            || !entry
                .nid
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-'))
            || registrations[..index].iter().any(|other| {
                other.nid == entry.nid
                    && other.library == entry.library
                    && other.library_version == entry.library_version
                    && other.module == entry.module
                    && other.module_version == entry.module_version
            })
        {
            return Err(KernelProviderError::InvalidCatalogue);
        }
    }

    // Every size is bounded above by the fixed catalogue/name limits before
    // arithmetic or allocation; no guest-supplied length reaches this layout.
    let dynamic_at = STUB_START + registrations.len() * STUB_STRIDE;
    let strings_at = dynamic_at + DYNAMIC_ENTRIES * 16;
    let mut module_names = [0; MODULES.len()];
    let mut library_names = [0; LIBRARIES.len()];
    let mut first_symbol = 1;
    for (index, (name, _)) in MODULES.iter().enumerate() {
        module_names[index] = first_symbol;
        first_symbol += name.len() + 1;
    }
    for (index, (name, _)) in LIBRARIES.iter().enumerate() {
        // Module and library descriptors may share the exact same namespace
        // string. Intern those bytes while keeping their independent IDs and
        // versions and all marker RVAs within the fixed RX geometry.
        if let Some(module) = MODULES.iter().position(|(module, _)| module == name) {
            library_names[index] = module_names[module];
        } else {
            library_names[index] = first_symbol;
            first_symbol += name.len() + 1;
        }
    }
    let object_symbol = registrations.len() + 1;
    let mut object_name = first_symbol;
    for registration in registrations {
        object_name += registration_name(registration, selector_base)?.1;
    }
    let object_size = qualified_name(STACK_GUARD_NID, selector_base, selector_base).1;
    let strings_size = object_name + 2 * object_size;
    let symbols_at = (strings_at + strings_size + 7) & !7;
    let rela_at = symbols_at + (registrations.len() + 3) * SYMBOL_SIZE;
    let code_bytes = rela_at + 24;
    let data_offset = (code_bytes + 7) & !7;
    // Only the guard and relocated name pointer have initialized data. The
    // name buffer is already zero-filled by the RW segment's memory extent;
    // keep it as BSS instead of serializing redundant zero bytes in the image.
    let image_size = data_offset + 16;
    if code_bytes as u64 > CODE_MAPPED_BYTES || image_size as u64 > maximum_image_bytes {
        return Err(KernelProviderError::Budget);
    }

    let mut image = Vec::new();
    image
        .try_reserve_exact(image_size)
        .map_err(|_| KernelProviderError::Allocation)?;
    image.resize(image_size, 0);
    let mut markers = Vec::new();
    markers
        .try_reserve_exact(registrations.len())
        .map_err(|_| KernelProviderError::Allocation)?;
    for (index, registration) in registrations.iter().enumerate() {
        markers.push(KernelMarker {
            registration,
            service_id: SERVICE_BASE + index as u32 + 1,
            symbol: index + 1,
            rva: (STUB_START + index * STUB_STRIDE) as u64,
        });
    }

    image[..8].copy_from_slice(b"\x7fELF\x02\x01\x01\0");
    put(&mut image, 16, &3_u16.to_le_bytes());
    put(&mut image, 18, &62_u16.to_le_bytes());
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    put(&mut image, 52, &64_u16.to_le_bytes());
    put(&mut image, 54, &56_u16.to_le_bytes());
    put(&mut image, 56, &3_u16.to_le_bytes());
    super::provider::program(
        &mut image,
        0,
        1,
        5,
        [0, 0, code_bytes as u64, CODE_MAPPED_BYTES, PAGE_SIZE],
    );
    super::provider::program(
        &mut image,
        1,
        2,
        4,
        [
            dynamic_at as u64,
            dynamic_at as u64,
            (DYNAMIC_ENTRIES * 16) as u64,
            (DYNAMIC_ENTRIES * 16) as u64,
            8,
        ],
    );
    super::provider::program(
        &mut image,
        2,
        1,
        6,
        [data_offset as u64, STACK_GUARD_RVA, 16, PAGE_SIZE, 8],
    );

    for (index, (name, _)) in MODULES.iter().enumerate() {
        put(
            &mut image,
            strings_at + module_names[index],
            name.as_bytes(),
        );
    }
    for (index, (name, _)) in LIBRARIES.iter().enumerate() {
        put(
            &mut image,
            strings_at + library_names[index],
            name.as_bytes(),
        );
    }
    let mut name = first_symbol;
    for marker in &markers {
        let (qualified, length) = registration_name(marker.registration, selector_base)?;
        put(&mut image, strings_at + name, &qualified[..length]);
        let row = symbols_at + marker.symbol * SYMBOL_SIZE;
        put(&mut image, row, &(name as u32).to_le_bytes());
        image[row + 4] = 0x12; // GLOBAL FUNC.
        image[row + 5] = 3; // PROTECTED.
        put(&mut image, row + 6, &1_u16.to_le_bytes());
        put(&mut image, row + 8, &marker.rva.to_le_bytes());
        put(&mut image, row + 16, &(STUB_SIZE as u64).to_le_bytes());
        put(&mut image, marker.rva as usize, &marker.bytes());
        name += length;
    }
    let objects = [
        KernelObject {
            symbol: object_symbol,
            rva: STACK_GUARD_RVA,
        },
        KernelObject {
            symbol: object_symbol + 1,
            rva: PROGRAM_NAME_OBJECT_RVA,
        },
    ];
    for (index, (object, nid)) in objects
        .iter()
        .zip([STACK_GUARD_NID, PROGRAM_NAME_NID])
        .enumerate()
    {
        let name = object_name + index * object_size;
        let (qualified, length) = qualified_name(nid, selector_base, selector_base);
        put(&mut image, strings_at + name, &qualified[..length]);
        let row = symbols_at + object.symbol * SYMBOL_SIZE;
        put(&mut image, row, &(name as u32).to_le_bytes());
        image[row + 4] = 0x11; // GLOBAL OBJECT, never a callable marker.
        image[row + 5] = 3; // PROTECTED.
        put(&mut image, row + 6, &2_u16.to_le_bytes());
        put(&mut image, row + 8, &object.rva.to_le_bytes());
        put(&mut image, row + 16, &8_u64.to_le_bytes());
    }
    put(&mut image, data_offset, &STACK_GUARD_INITIAL.to_le_bytes());
    // The pointer is a genuine guest-relative relocation, never a host address.
    put(&mut image, rela_at, &PROGRAM_NAME_OBJECT_RVA.to_le_bytes());
    put(&mut image, rela_at + 8, &8_u64.to_le_bytes());
    put(
        &mut image,
        rela_at + 16,
        &PROGRAM_NAME_BUFFER_RVA.to_le_bytes(),
    );
    let module_word = |index: usize| {
        module_names[index] as u64
            | (u64::from(MODULES[index].1.1) << 32)
            | (u64::from(MODULES[index].1.0) << 40)
            | ((index as u64 + u64::from(selector_base)) << 48)
    };
    let library_word = |index: usize| {
        library_names[index] as u64
            | (u64::from(LIBRARIES[index].1) << 32)
            | ((index as u64 + u64::from(selector_base)) << 48)
    };
    let mut entries = [(0_i64, 0_u64); DYNAMIC_ENTRIES];
    entries[..5].copy_from_slice(&[
        (5_i64, strings_at as u64),
        (10, strings_size as u64),
        (6, symbols_at as u64),
        (39, ((registrations.len() + 3) * SYMBOL_SIZE) as u64),
        (11, SYMBOL_SIZE as u64),
    ]);
    let mut cursor = 5;
    for index in 0..MODULES.len() {
        entries[cursor] = (DT_SCE_MODULE_INFO, module_word(index));
        cursor += 1;
    }
    for index in 0..LIBRARIES.len() {
        entries[cursor] = (DT_SCE_EXPORT_LIB, library_word(index));
        cursor += 1;
    }
    entries[cursor..].copy_from_slice(&[(7, rela_at as u64), (8, 24), (9, 24), (0, 0)]);
    for (index, (tag, value)) in entries.into_iter().enumerate() {
        put(&mut image, dynamic_at + index * 16, &tag.to_le_bytes());
        put(
            &mut image,
            dynamic_at + index * 16 + 8,
            &value.to_le_bytes(),
        );
    }
    Ok(GeneratedKernelProvider {
        image,
        markers,
        code_bytes,
        data_offset,
        objects,
    })
}

fn qualified_name(nid: &[u8], library: u16, module: u16) -> ([u8; NID_TOKEN_LEN + 9], usize) {
    let library = SelectorId::new(library).encode();
    let module = SelectorId::new(module).encode();
    let mut output = [0; NID_TOKEN_LEN + 9];
    output[..NID_TOKEN_LEN].copy_from_slice(nid);
    let mut cursor = NID_TOKEN_LEN;
    for selector in [library, module] {
        output[cursor] = b'#';
        cursor += 1;
        output[cursor..cursor + selector.as_bytes().len()].copy_from_slice(selector.as_bytes());
        cursor += selector.as_bytes().len();
    }
    (output, cursor + 1) // Include the pre-zeroed terminating byte.
}

fn registration_name(
    row: &Registration,
    base: u16,
) -> Result<([u8; NID_TOKEN_LEN + 9], usize), KernelProviderError> {
    let library = LIBRARIES
        .iter()
        .position(|(name, version)| *name == row.library && *version == row.library_version)
        .ok_or(KernelProviderError::InvalidCatalogue)?;
    let module = MODULES
        .iter()
        .position(|(name, version)| *name == row.module && *version == row.module_version)
        .ok_or(KernelProviderError::InvalidCatalogue)?;
    // Namespace/base bounds were proved before allocation or serialization.
    Ok(qualified_name(
        row.nid.as_bytes(),
        base + library as u16,
        base + module as u16,
    ))
}

fn put(image: &mut [u8], offset: usize, value: &[u8]) {
    image[offset..offset + value.len()].copy_from_slice(value);
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod mspace_tests;

#[cfg(test)]
mod game_intent_tests;

#[cfg(test)]
mod trophy2_tests;

#[cfg(test)]
mod uds_tests;

#[cfg(test)]
mod entitlement_access_tests;

#[cfg(test)]
mod font_tests;

#[cfg(test)]
#[path = "kernel_provider/ime_tests.rs"]
mod ime_tests;

#[cfg(test)]
#[path = "kernel_provider/raw_input_tests.rs"]
mod raw_input_tests;
