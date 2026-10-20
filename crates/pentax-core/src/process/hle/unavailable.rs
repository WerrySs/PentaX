// SPDX-License-Identifier: GPL-2.0-only

//! Original explicit stop-at-use functions. No unavailable call returns success.
//! The provider is ordinary ELF data until exact linking/native preparation;
//! source identifiers stay internal unless a caller explicitly inspects them.

use super::provider::{program, put};
use crate::memory::PAGE_SIZE;
use pentax_linker::{
    DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO, MissingFunctionImport, MissingFunctionPlan, SelectorId,
};
use std::fmt;

const STUBS_AT: usize = 0x100;
const STRIDE: usize = 16;
const SERVICE_BASE: u32 = 0x2000_0000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnavailableProviderLimits {
    pub max_functions: usize,
    pub max_image_bytes: u64,
    pub max_work_steps: u64,
}
impl Default for UnavailableProviderLimits {
    fn default() -> Self {
        Self {
            max_functions: 4096,
            max_image_bytes: 4 * 1024 * 1024,
            max_work_steps: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnavailableProviderError {
    Allocation,
    Layout,
    Budget(&'static str),
}
impl fmt::Display for UnavailableProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unavailable-function provider: {self:?}")
    }
}
impl std::error::Error for UnavailableProviderError {}

/// Immutable original bytes plus the classified unavailable identities. These
/// are explicit missing implementations, not HLE registrations or fake returns.
pub struct GeneratedUnavailableProvider {
    image: Vec<u8>,
    functions: Vec<MissingFunctionImport>,
    mapped_bytes: u64,
    work_steps: u64,
}
impl fmt::Debug for GeneratedUnavailableProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedUnavailableProvider")
            .field("functions", &self.functions.len())
            .field("image_bytes", &self.image.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UnavailableSite {
    pub ordinal: Option<u32>,
    pub rva: u64,
    pub service_id: u32,
}
impl UnavailableSite {
    pub const fn bytes(self) -> [u8; 8] {
        let [a, b, c, d] = self.service_id.to_le_bytes();
        [0xb8, a, b, c, d, 0x0f, 0x05, 0xc3]
    }
}
impl GeneratedUnavailableProvider {
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }
    pub const fn mapped_bytes(&self) -> u64 {
        self.mapped_bytes
    }
    pub(crate) const fn work_steps(&self) -> u64 {
        self.work_steps
    }
    /// Original marker code that must produce an unavailable-exit stop if used.
    /// It does not implement module shutdown.
    pub const fn exit_callback_rva(&self) -> u64 {
        STUBS_AT as u64
    }
    pub fn function(&self, ordinal: usize) -> Option<&MissingFunctionImport> {
        self.functions.get(ordinal)
    }
    pub(super) fn site(&self, slot: usize) -> Option<UnavailableSite> {
        if slot > self.functions.len() {
            return None;
        }
        Some(UnavailableSite {
            ordinal: slot.checked_sub(1).map(|index| index as u32),
            rva: (STUBS_AT + slot * STRIDE) as u64,
            service_id: SERVICE_BASE + slot as u32,
        })
    }
    pub(super) fn symbol_site(&self, symbol: usize) -> Option<UnavailableSite> {
        if symbol == 0 { None } else { self.site(symbol) }
    }
    pub(super) fn syscall_site(&self, bias: u64, pc: u64) -> Option<UnavailableSite> {
        let delta = pc.checked_sub(bias.checked_add(STUBS_AT as u64 + 5)?)?;
        if !delta.is_multiple_of(STRIDE as u64) {
            return None;
        }
        self.site(usize::try_from(delta / STRIDE as u64).ok()?)
    }
}

fn add(left: usize, right: usize) -> Result<usize, UnavailableProviderError> {
    left.checked_add(right)
        .ok_or(UnavailableProviderError::Layout)
}
fn mul(left: usize, right: usize) -> Result<usize, UnavailableProviderError> {
    left.checked_mul(right)
        .ok_or(UnavailableProviderError::Layout)
}
fn aligned(value: usize) -> Result<usize, UnavailableProviderError> {
    Ok(add(value, 15)? & !15)
}
fn charge(
    work: &mut u64,
    count: u64,
    limits: &UnavailableProviderLimits,
) -> Result<(), UnavailableProviderError> {
    *work = work
        .checked_add(count)
        .filter(|value| *value <= limits.max_work_steps)
        .ok_or(UnavailableProviderError::Budget("work"))?;
    Ok(())
}
fn string(
    image: &mut [u8],
    start: usize,
    cursor: &mut usize,
    bytes: &[u8],
) -> Result<u32, UnavailableProviderError> {
    let offset = *cursor;
    let end = add(offset, bytes.len())?;
    image
        .get_mut(add(start, offset)?..add(start, end)?)
        .ok_or(UnavailableProviderError::Layout)?
        .copy_from_slice(bytes);
    *cursor = add(end, 1)?;
    u32::try_from(offset).map_err(|_| UnavailableProviderError::Layout)
}

/// Build bounded GLOBAL/PROTECTED FUNC exports for classified absent identities.
/// Each exact identity receives an independent image-local selector pair and
/// original MOV-EAX/SYSCALL/RET marker. The separate slot0 is an explicit
/// unavailable exit callback. Native service dispatch MUST stop these markers;
/// constructing an image never authorizes execution or claims their behavior.
pub fn build_unavailable_provider(
    plan: MissingFunctionPlan<'_>,
    limits: &UnavailableProviderLimits,
) -> Result<GeneratedUnavailableProvider, UnavailableProviderError> {
    let count = plan.functions().len();
    if count > limits.max_functions || count > u16::MAX as usize {
        return Err(UnavailableProviderError::Budget("functions"));
    }
    let mut work = 0;
    charge(
        &mut work,
        (count as u64)
            .checked_mul(128)
            .ok_or(UnavailableProviderError::Layout)?,
        limits,
    )?;
    let mut strings_len = 1;
    for (index, function) in plan.functions().iter().enumerate() {
        let selector = SelectorId::new((index + 1) as u16).encode();
        let names = add(function.module_name().len(), function.library_name().len())?;
        let qualified = add(14, mul(selector.as_bytes().len(), 2)?)?; // NID, two '#', NUL.
        strings_len = add(strings_len, add(add(names, 2)?, qualified)?)?;
    }
    let dynamic = aligned(add(STUBS_AT, mul(add(count, 1)?, STRIDE)?)?)?;
    let dynamic_entries = add(6, mul(count, 2)?)?;
    let dynamic_bytes = mul(dynamic_entries, 16)?;
    let strings_at = aligned(add(dynamic, dynamic_bytes)?)?;
    let symbols_at = aligned(add(strings_at, strings_len)?)?;
    let symbols_size = mul(add(count, 1)?, 24)?;
    let image_len = add(symbols_at, symbols_size)?;
    if image_len as u64 > limits.max_image_bytes || strings_len > u32::MAX as usize {
        return Err(UnavailableProviderError::Budget("image bytes"));
    }
    charge(
        &mut work,
        (image_len as u64)
            .checked_add(
                (strings_len as u64)
                    .checked_mul(2)
                    .ok_or(UnavailableProviderError::Layout)?,
            )
            .and_then(|value| value.checked_add(512))
            .ok_or(UnavailableProviderError::Layout)?,
        limits,
    )?;
    let mapped_bytes = (image_len as u64)
        .checked_add(PAGE_SIZE - 1)
        .map(|value| value / PAGE_SIZE * PAGE_SIZE)
        .ok_or(UnavailableProviderError::Layout)?;
    let mut image = Vec::new();
    image
        .try_reserve_exact(image_len)
        .map_err(|_| UnavailableProviderError::Allocation)?;
    image.resize(image_len, 0);
    image[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    put(&mut image, 16, &3_u16.to_le_bytes());
    put(&mut image, 18, &62_u16.to_le_bytes());
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    put(&mut image, 52, &64_u16.to_le_bytes());
    put(&mut image, 54, &56_u16.to_le_bytes());
    put(&mut image, 56, &2_u16.to_le_bytes());
    program(
        &mut image,
        0,
        1,
        5,
        [0, 0, image_len as u64, mapped_bytes, PAGE_SIZE],
    );
    program(
        &mut image,
        1,
        2,
        4,
        [
            dynamic as u64,
            dynamic as u64,
            dynamic_bytes as u64,
            dynamic_bytes as u64,
            8,
        ],
    );
    for (index, (tag, value)) in [
        (5_i64, strings_at as u64),
        (10, strings_len as u64),
        (6, symbols_at as u64),
        (39, symbols_size as u64),
        (11, 24),
    ]
    .into_iter()
    .enumerate()
    {
        put(&mut image, dynamic + index * 16, &tag.to_le_bytes());
        put(&mut image, dynamic + index * 16 + 8, &value.to_le_bytes());
    }
    let mut result = GeneratedUnavailableProvider {
        image,
        functions: plan.into_functions(),
        mapped_bytes,
        work_steps: work,
    };
    let callback = result.site(0).ok_or(UnavailableProviderError::Layout)?;
    put(&mut result.image, callback.rva as usize, &callback.bytes());
    let mut cursor = 1;
    for (index, function) in result.functions.iter().enumerate() {
        let id = (index + 1) as u16;
        let selector = SelectorId::new(id).encode();
        let module = string(
            &mut result.image,
            strings_at,
            &mut cursor,
            function.module_name(),
        )?;
        let library = string(
            &mut result.image,
            strings_at,
            &mut cursor,
            function.library_name(),
        )?;
        let name = cursor;
        // Write one canonical qualified name without a second temporary table.
        let length = add(13, mul(selector.as_bytes().len(), 2)?)?;
        let destination = result
            .image
            .get_mut(add(strings_at, name)?..add(add(strings_at, name)?, length)?)
            .ok_or(UnavailableProviderError::Layout)?;
        destination[..11].copy_from_slice(function.nid().as_bytes());
        destination[11] = b'#';
        let width = selector.as_bytes().len();
        destination[12..12 + width].copy_from_slice(selector.as_bytes());
        destination[12 + width] = b'#';
        destination[13 + width..].copy_from_slice(selector.as_bytes());
        cursor = add(name, add(length, 1)?)?;
        let versions = function.versions();
        let module_word = u64::from(module)
            | (u64::from(versions.module_minor) << 32)
            | (u64::from(versions.module_major) << 40)
            | (u64::from(id) << 48);
        let library_word =
            u64::from(library) | (u64::from(versions.library) << 32) | (u64::from(id) << 48);
        let at = dynamic + (5 + index * 2) * 16;
        put(&mut result.image, at, &DT_SCE_MODULE_INFO.to_le_bytes());
        put(&mut result.image, at + 8, &module_word.to_le_bytes());
        put(&mut result.image, at + 16, &DT_SCE_EXPORT_LIB.to_le_bytes());
        put(&mut result.image, at + 24, &library_word.to_le_bytes());
        let site = UnavailableSite {
            ordinal: Some(index as u32),
            rva: (STUBS_AT + (index + 1) * STRIDE) as u64,
            service_id: SERVICE_BASE + index as u32 + 1,
        };
        let row = symbols_at + (index + 1) * 24;
        put(&mut result.image, row, &(name as u32).to_le_bytes());
        result.image[row + 4] = 0x12;
        result.image[row + 5] = 3;
        put(&mut result.image, row + 6, &1_u16.to_le_bytes());
        put(&mut result.image, row + 8, &site.rva.to_le_bytes());
        put(&mut result.image, row + 16, &8_u64.to_le_bytes());
        put(&mut result.image, site.rva as usize, &site.bytes());
    }
    if cursor != strings_len {
        return Err(UnavailableProviderError::Layout);
    }
    Ok(result)
}

#[cfg(test)]
pub(super) mod tests;
