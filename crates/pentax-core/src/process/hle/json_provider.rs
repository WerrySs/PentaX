// SPDX-License-Identifier: GPL-2.0-only

//! Placement-bound original JSON primitive markers and immutable guest defaults.
//! Public identities use pinned KytyPS5 f186b357 Json2 1/1.1. Independently
//! observed libSceJson/libSceJson2 pairs are separate per-NID exact rows.
//! The table shape follows AnyPS5 6bd Export.cpp: two zero prefix words and six
//! terminal virtual slots. This is not a complete allocator or JSON class ABI.
//! Allocator dispatch implements only the observed first-word constructor write;
//! every virtual slot must stop as explicitly unsupported. The separate Value
//! constructor identities use pinned Kyty's null-safe 32-byte initialization,
//! not the unrelated AnyPS5 Value representation.
//! Boolean/integer/real getters return guest pointers. Their separate immutable
//! zero defaults do not introduce a host node layout or floating-point return ABI.
//! A separate empty String wrapper has a null implementation word; its immutable
//! empty C string occupies a distinct byte, without exposing host storage.

use super::{
    KernelProviderError, ProviderPlacement,
    provider::{program, put},
};
use crate::memory::{ADDRESS_LIMIT, PAGE_SIZE};
use pentax_linker::{DT_SCE_EXPORT_LIB, DT_SCE_MODULE_INFO};
use std::fmt;

const CONSTRUCTOR_NID: &[u8; 11] = b"-hJRce8wn1U";
const INITIALIZER_CONSTRUCTOR_NID: &[u8; 11] = b"cK6bYHf-Q5E";
// Each NID has exactly its pinned source identity and independently observed
// libSceJson/libSceJson2 identity, never an inferred prefix cross-product.
const CONTAINER_NIDS: [&[u8; 11]; 22] = [
    b"OJPTonqdg0I",
    b"a+W7HHlwpBs",
    b"5JmzZt8twAo",
    b"ERuf9y0DY84",
    b"JP-PtKMiI1E",
    b"HJ8GpRT1aiw",
    b"zQtLRTqceMY",
    b"3xUXnmUkXfo",
    b"iZeYfOxtMRg",
    b"dFCphqnd+a4",
    b"IlsmvBtMkak",
    b"ONT8As5R1ug",
    b"HwDt5lD9Bfo",
    b"XlWbvieLj2M",
    b"wLsJlmgEIaI",
    b"bcH5EnFE2xY",
    b"WXF2ihRF+B8",
    b"5AZPp99ogrc",
    b"wcgr5mte7T8",
    b"w5+VCznos5E",
    b"9yLjn46Ypfs",
    b"S5JxQnoGF3E",
];
const VALUE_CONSTRUCTOR_NIDS: [&[u8; 11]; 2] = [b"qBMjqyBn3OM", b"-wa17B7TGnw"];
const MODULES: [&[u8]; 3] = [b"Json2", b"libSceJson2", b"libSceJson"];
const LIBRARIES: [&[u8]; 2] = [b"Json2", b"libSceJson2"];
// Module/library selector indices, never wildcard or normalized identities.
const SCOPES: [(usize, usize); 5] = [(0, 0), (1, 1), (1, 0), (0, 1), (2, 1)];
const VALUE_SCOPES: [(usize, usize); 4] = [(0, 0), (1, 1), (1, 0), (0, 1)];

#[derive(Clone, Copy)]
struct JsonExport {
    nid: &'static [u8; 11],
    module: usize,
    library: usize,
    marker: usize,
}
// Exact standalone owner metadata confirms this pair only for qBMjqyBn3OM;
// the other public Value constructor NID has no such observed declaration.
const OBSERVED_VALUE_EXPORT: JsonExport = JsonExport {
    nid: VALUE_CONSTRUCTOR_NIDS[0],
    module: 2,
    library: 1,
    marker: 2,
};
#[derive(Clone, Copy)]
struct PrimitiveExport {
    nid: &'static [u8; 11],
    marker: usize,
    observed_scope: bool,
}
// Public NIDs from blob d6e968f019fa61e0adc5224cd20be1ecd3261487.
// Fifth-scope flags are fixed by the trusted 57-public-import inventory;
// Absent TypeCtor, ClearMethod, alternate Dtor and StringAssign identities
// stay canonical4; observing one NID never grants a fifth scope to another.
const PRIMITIVES: [PrimitiveExport; 32] = [
    PrimitiveExport {
        nid: b"UeuWT+yNdCQ",
        marker: 3,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"0lLK8+kDqmE",
        marker: 4,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"CbrT3dwDILo",
        marker: 5,
        observed_scope: false,
    },
    PrimitiveExport {
        nid: b"5yHuiWXo2gg",
        marker: 6,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"QxVVYhP-mvg",
        marker: 7,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"SIe1ZmW7e7s",
        marker: 8,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"IKQimvG9Wqs",
        marker: 9,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"SHtAad20YYM",
        marker: 10,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"zTwZdI8AZ5Y",
        marker: 11,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"DIxvoy7Ngvk",
        marker: 12,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"3qrge7L-AU4",
        marker: 13,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"fSb2oQTNrgA",
        marker: 14,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"4zrm6VrgIAw",
        marker: 15,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"WTtYf+cNnXI",
        marker: 16,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"0eUrW9JAxM0",
        marker: 16,
        observed_scope: false,
    },
    PrimitiveExport {
        nid: b"FIjXN2TkuTs",
        marker: 17,
        observed_scope: false,
    },
    PrimitiveExport {
        nid: b"RBw+4NukeGQ",
        marker: 18,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"OcAgPxcq5Vk",
        marker: 19,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"sOmU4vnx3s0",
        marker: 20,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"BSmWDIkV4w4",
        marker: 21,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"qSmqLXXCPas",
        marker: 22,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"9KUZFjI1IxA",
        marker: 23,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"cG1VE2HMl6c",
        marker: 24,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"cn9svYGWKDQ",
        marker: 25,
        observed_scope: false,
    },
    PrimitiveExport {
        nid: b"L1KAkYWml-M",
        marker: 26,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"EUH+EmT-v9E",
        marker: 27,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"b9V6fmppLXY",
        marker: 28,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"sZIoMRGO+jk",
        marker: 29,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"6l3Bv2gysNc",
        marker: 30,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"epJ6x2LV0kU",
        marker: 31,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"Ncel8t2Rrpc",
        marker: 32,
        observed_scope: true,
    },
    PrimitiveExport {
        nid: b"R7FDWtcN6f8",
        marker: 33,
        observed_scope: true,
    },
];
const PRIMITIVE_EXPORT_COUNT: usize = {
    let mut count = 0;
    let mut index = 0;
    while index < PRIMITIVES.len() {
        count += VALUE_SCOPES.len() + PRIMITIVES[index].observed_scope as usize;
        index += 1;
    }
    count
};
const OLD_EXPORT_COUNT: usize =
    SCOPES.len() + VALUE_CONSTRUCTOR_NIDS.len() * VALUE_SCOPES.len() + 1;
const LIFECYCLE_NIDS: [&[u8; 11]; 4] = [
    b"Cxwy7wHq4J0",
    b"PR5k1penBLM",
    b"RujUxbr3haM",
    b"00oCq0RwSAY",
];
// Retain the original allocator rows 1..=5 and Value constructor rows 6..=14.
// New primitives append after them; the terminal marker has no symbol or NID.
const EXPORTS: [JsonExport; OLD_EXPORT_COUNT + PRIMITIVE_EXPORT_COUNT + 2 + 44 + 8] = {
    let mut exports = [JsonExport {
        nid: CONSTRUCTOR_NID,
        module: 0,
        library: 0,
        marker: 0,
    }; OLD_EXPORT_COUNT + PRIMITIVE_EXPORT_COUNT + 2 + 44 + 8];
    let mut index = 0;
    while index < SCOPES.len() {
        let (module, library) = SCOPES[index];
        exports[index] = JsonExport {
            nid: CONSTRUCTOR_NID,
            module,
            library,
            marker: 0,
        };
        index += 1;
    }
    let mut nid = 0;
    while nid < VALUE_CONSTRUCTOR_NIDS.len() {
        let mut scope = 0;
        while scope < VALUE_SCOPES.len() {
            let (module, library) = VALUE_SCOPES[scope];
            exports[index] = JsonExport {
                nid: VALUE_CONSTRUCTOR_NIDS[nid],
                module,
                library,
                marker: 2,
            };
            index += 1;
            scope += 1;
        }
        nid += 1;
    }
    exports[index] = OBSERVED_VALUE_EXPORT;
    index += 1;
    let mut primitive = 0;
    while primitive < PRIMITIVES.len() {
        let entry = PRIMITIVES[primitive];
        let mut scope = 0;
        while scope < VALUE_SCOPES.len() {
            let (module, library) = VALUE_SCOPES[scope];
            exports[index] = JsonExport {
                nid: entry.nid,
                module,
                library,
                marker: entry.marker,
            };
            index += 1;
            scope += 1;
        }
        if entry.observed_scope {
            exports[index] = JsonExport {
                nid: entry.nid,
                module: 2,
                library: 1,
                marker: entry.marker,
            };
            index += 1;
        }
        primitive += 1;
    }
    // The source declares Json2/Json2. Only the separately observed exact
    // libSceJson/libSceJson2 pair is added for this NID, without prefix expansion.
    exports[index] = JsonExport {
        nid: INITIALIZER_CONSTRUCTOR_NID,
        module: 0,
        library: 0,
        marker: 34,
    };
    exports[index + 1] = JsonExport {
        nid: INITIALIZER_CONSTRUCTOR_NID,
        module: 2,
        library: 1,
        marker: 34,
    };
    index += 2;
    let mut container = 0;
    while container < CONTAINER_NIDS.len() {
        exports[index] = JsonExport {
            nid: CONTAINER_NIDS[container],
            module: 0,
            library: 0,
            marker: container + 35,
        };
        exports[index + 1] = JsonExport {
            nid: CONTAINER_NIDS[container],
            module: 2,
            library: 1,
            marker: container + 35,
        };
        index += 2;
        container += 1;
    }
    let mut lifecycle = 0;
    while lifecycle < LIFECYCLE_NIDS.len() {
        exports[index] = JsonExport {
            nid: LIFECYCLE_NIDS[lifecycle],
            module: 0,
            library: 0,
            marker: 57 + lifecycle,
        };
        exports[index + 1] = JsonExport {
            nid: LIFECYCLE_NIDS[lifecycle],
            module: 2,
            library: 1,
            marker: 57 + lifecycle,
        };
        index += 2;
        lifecycle += 1;
    }
    exports
};
pub(super) const CODE_FILE_BYTES: usize = 0x3800;
pub(super) const TABLE_RVA: u64 = PAGE_SIZE;
pub(super) const TABLE_BYTES: usize = 64;
// Preserve table/scalar positions; append one null String implementation word,
// one NUL character and seven explicit zero padding bytes.
pub(super) const DATA_BYTES: usize = TABLE_BYTES + 3 * 8 + 8 + 8;
const IMAGE_BYTES: usize = CODE_FILE_BYTES + DATA_BYTES;
const DYNAMIC_AT: usize = 0x600;
const DYNAMIC_ENTRIES: usize = 5 + MODULES.len() + LIBRARIES.len() + 1;
const STRINGS_AT: usize = 0x800;
const SYMBOLS_AT: usize = 0x1800;
const SYMBOL_BYTES: usize = 24;
const NAME_BYTES: usize = 1 + 6 + 12 + 11;
const QUALIFIED_BYTES: usize = 16;
const STRINGS_BYTES: usize = NAME_BYTES + EXPORTS.len() * QUALIFIED_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JsonFunction {
    MemAllocatorCtor,
    VirtualCall,
    ValueCtor,
    ValueBoolCtor,
    ValueIntCtor,
    ValueTypeCtor,
    ValueSetBool,
    ValueSetInt,
    ValueSetUInt,
    ValueSetType,
    ValueGetType,
    ValueGetBoolean,
    ValueGetInteger,
    ValueGetReal,
    ValueCopyCtor,
    ValueAssign,
    ValueDtor,
    ValueClearMethod,
    ValueCount,
    MemAllocatorDtor,
    ValueDoubleCtor,
    ValueSetDouble,
    StringCtor,
    StringCStringCtor,
    StringDtor,
    StringAssign,
    StringCStr,
    StringLength,
    ValueCStringCtor,
    ValueStringCtor,
    ValueSetString,
    ValueGetString,
    ValueToString,
    ValueSerialize,
    InitializerCtor,
    ObjectCtor,
    ObjectCopyCtor,
    ObjectDtor,
    ObjectIndex,
    ArrayCtor,
    ArrayDtor,
    ArrayPushBack,
    ValueObjectCtor,
    ValueArrayCtor,
    ValueSetObject,
    ValueGetObject,
    ValueGetArray,
    ValueIndexString,
    ValueIndexUInt,
    ValueReferValueKey,
    ArrayBegin,
    ArrayEnd,
    ArrayIteratorNotEqual,
    ArrayIteratorDereference,
    ArrayIteratorIncrement,
    ArrayIteratorDtor,
    ParserParse,
    InitializerInitializeV1,
    InitializerTerminate,
    InitializerDtor,
    InitializerSetGlobalNullAccessCallback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct JsonMarker {
    pub function: JsonFunction,
    pub service_id: u32,
    pub rva: u64,
}
impl JsonMarker {
    pub(crate) const fn bytes(self) -> [u8; 8] {
        let [a, b, c, d] = self.service_id.to_le_bytes();
        [0xb8, a, b, c, d, 0x0f, 0x05, 0xc3]
    }
}
const MARKERS: [JsonMarker; 61] = {
    let mut markers = [JsonMarker {
        function: JsonFunction::MemAllocatorCtor,
        service_id: 0x5058_4a01,
        rva: 0x100,
    }; 61];
    markers[1] = JsonMarker {
        function: JsonFunction::VirtualCall,
        service_id: 0x5058_4a02,
        rva: 0x110,
    };
    markers[2] = JsonMarker {
        function: JsonFunction::ValueCtor,
        service_id: 0x5058_4a03,
        rva: 0x120,
    };
    let functions = [
        JsonFunction::ValueBoolCtor,
        JsonFunction::ValueIntCtor,
        JsonFunction::ValueTypeCtor,
        JsonFunction::ValueSetBool,
        JsonFunction::ValueSetInt,
        JsonFunction::ValueSetUInt,
        JsonFunction::ValueSetType,
        JsonFunction::ValueGetType,
        JsonFunction::ValueGetBoolean,
        JsonFunction::ValueGetInteger,
        JsonFunction::ValueGetReal,
        JsonFunction::ValueCopyCtor,
        JsonFunction::ValueAssign,
        JsonFunction::ValueDtor,
        JsonFunction::ValueClearMethod,
        JsonFunction::ValueCount,
        JsonFunction::MemAllocatorDtor,
        JsonFunction::ValueDoubleCtor,
        JsonFunction::ValueSetDouble,
        JsonFunction::StringCtor,
        JsonFunction::StringCStringCtor,
        JsonFunction::StringDtor,
        JsonFunction::StringAssign,
        JsonFunction::StringCStr,
        JsonFunction::StringLength,
        JsonFunction::ValueCStringCtor,
        JsonFunction::ValueStringCtor,
        JsonFunction::ValueSetString,
        JsonFunction::ValueGetString,
        JsonFunction::ValueToString,
        JsonFunction::ValueSerialize,
        JsonFunction::InitializerCtor,
        JsonFunction::ObjectCtor,
        JsonFunction::ObjectCopyCtor,
        JsonFunction::ObjectDtor,
        JsonFunction::ObjectIndex,
        JsonFunction::ArrayCtor,
        JsonFunction::ArrayDtor,
        JsonFunction::ArrayPushBack,
        JsonFunction::ValueObjectCtor,
        JsonFunction::ValueArrayCtor,
        JsonFunction::ValueSetObject,
        JsonFunction::ValueGetObject,
        JsonFunction::ValueGetArray,
        JsonFunction::ValueIndexString,
        JsonFunction::ValueIndexUInt,
        JsonFunction::ValueReferValueKey,
        JsonFunction::ArrayBegin,
        JsonFunction::ArrayEnd,
        JsonFunction::ArrayIteratorNotEqual,
        JsonFunction::ArrayIteratorDereference,
        JsonFunction::ArrayIteratorIncrement,
        JsonFunction::ArrayIteratorDtor,
        JsonFunction::ParserParse,
        JsonFunction::InitializerInitializeV1,
        JsonFunction::InitializerTerminate,
        JsonFunction::InitializerDtor,
        JsonFunction::InitializerSetGlobalNullAccessCallback,
    ];
    let mut index = 0;
    while index < functions.len() {
        let slot = index + 3;
        markers[slot] = JsonMarker {
            function: functions[index],
            service_id: 0x5058_4a01 + slot as u32,
            rva: 0x100 + slot as u64 * 16,
        };
        index += 1;
    }
    markers
};

/// Immutable original image. Its absolute table entries bind this factory to
/// one checked guest load bias; copying it never creates factory provenance.
pub struct GeneratedJsonProvider {
    image: Vec<u8>,
    load_bias: u64,
}
impl fmt::Debug for GeneratedJsonProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedJsonProvider")
            .field("image_bytes", &self.image.len())
            .field("exports", &self.function_count())
            .finish_non_exhaustive()
    }
}
impl GeneratedJsonProvider {
    pub fn image(&self) -> &[u8] {
        &self.image
    }
    pub const fn load_bias(&self) -> u64 {
        self.load_bias
    }
    pub const fn mapped_bytes(&self) -> u64 {
        2 * PAGE_SIZE
    }
    pub const fn function_count(&self) -> usize {
        EXPORTS.len()
    }
    pub(super) fn table_bytes(&self) -> &[u8] {
        &self.data_bytes()[..TABLE_BYTES]
    }
    pub(super) fn data_bytes(&self) -> &[u8] {
        &self.image[CODE_FILE_BYTES..]
    }
    pub(super) const fn table_rva(&self) -> u64 {
        TABLE_RVA
    }
    pub(super) const fn address_point(&self) -> u64 {
        self.load_bias + self.table_rva() + 16
    }
    pub(crate) const fn markers(&self) -> &[JsonMarker; 61] {
        &MARKERS
    }
    pub(super) const fn scalar_default_rva(&self, function: JsonFunction) -> Option<u64> {
        match function {
            JsonFunction::ValueGetBoolean => Some(TABLE_RVA + 64),
            JsonFunction::ValueGetInteger => Some(TABLE_RVA + 72),
            JsonFunction::ValueGetReal => Some(TABLE_RVA + 80),
            _ => None,
        }
    }
    /// Immutable eight-byte String wrapper with a null implementation word.
    pub(super) const fn empty_string_rva(&self) -> u64 {
        TABLE_RVA + 88
    }
    /// Immutable NUL character, separate from the empty String wrapper.
    pub(super) const fn empty_cstr_rva(&self) -> u64 {
        TABLE_RVA + 96
    }
    /// Only the fixed exported FUNC rows identify a service. Null and
    /// out-of-range rows grant nothing; the terminal marker has no export row.
    pub(crate) fn export_marker(&self, symbol: usize) -> Option<JsonMarker> {
        symbol
            .checked_sub(1)
            .and_then(|index| EXPORTS.get(index))
            .map(|export| MARKERS[export.marker])
    }
}

/// Generate only the finite primitive identities and a private terminal
/// marker. Table entries are guest addresses at the checked placement, never
/// host implementation pointers. No relocations, TLS or initializer are emitted.
pub fn build_json_provider(
    placement: ProviderPlacement,
    max_image_bytes: u64,
) -> Result<GeneratedJsonProvider, KernelProviderError> {
    if max_image_bytes < IMAGE_BYTES as u64 {
        return Err(KernelProviderError::Budget);
    }
    let bias = placement.load_bias;
    let mapped_bytes = 2 * PAGE_SIZE;
    let terminal = bias
        .checked_add(MARKERS[1].rva)
        .ok_or(KernelProviderError::InvalidCatalogue)?;
    if !bias.is_multiple_of(PAGE_SIZE)
        || bias
            .checked_add(mapped_bytes)
            .is_none_or(|end| end > ADDRESS_LIMIT)
        || 64 + 3 * 56 > MARKERS[0].rva as usize
        || MARKERS[MARKERS.len() - 1].rva as usize + 8 > DYNAMIC_AT
        || DYNAMIC_AT + DYNAMIC_ENTRIES * 16 > STRINGS_AT
        || STRINGS_AT + STRINGS_BYTES > SYMBOLS_AT
        || SYMBOLS_AT + (EXPORTS.len() + 1) * SYMBOL_BYTES > CODE_FILE_BYTES
        || CODE_FILE_BYTES as u64 > PAGE_SIZE
        || IMAGE_BYTES as u64 > mapped_bytes
    {
        return Err(KernelProviderError::InvalidCatalogue);
    }
    let mut image = Vec::new();
    image
        .try_reserve_exact(IMAGE_BYTES)
        .map_err(|_| KernelProviderError::Allocation)?;
    if image.capacity() as u64 > max_image_bytes {
        return Err(KernelProviderError::Budget);
    }
    image.resize(IMAGE_BYTES, 0);
    put(&mut image, 0, b"\x7fELF\x02\x01\x01\0");
    for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 3)] {
        put(&mut image, at, &value.to_le_bytes());
    }
    put(&mut image, 20, &1_u32.to_le_bytes());
    put(&mut image, 32, &64_u64.to_le_bytes());
    program(
        &mut image,
        0,
        1,
        5,
        [0, 0, CODE_FILE_BYTES as u64, PAGE_SIZE, PAGE_SIZE],
    );
    program(
        &mut image,
        1,
        1,
        4,
        [
            CODE_FILE_BYTES as u64,
            TABLE_RVA,
            DATA_BYTES as u64,
            PAGE_SIZE,
            8,
        ],
    );
    program(
        &mut image,
        2,
        2,
        4,
        [
            DYNAMIC_AT as u64,
            DYNAMIC_AT as u64,
            (DYNAMIC_ENTRIES * 16) as u64,
            (DYNAMIC_ENTRIES * 16) as u64,
            8,
        ],
    );
    for marker in MARKERS {
        put(&mut image, marker.rva as usize, &marker.bytes());
    }
    for slot in 2..8 {
        put(
            &mut image,
            CODE_FILE_BYTES + slot * 8,
            &terminal.to_le_bytes(),
        );
    }
    let mut cursor = 1;
    let mut names = [0; MODULES.len()];
    for (index, name) in MODULES.iter().enumerate() {
        names[index] = cursor as u64;
        put(&mut image, STRINGS_AT + cursor, name);
        cursor += name.len() + 1;
    }
    if cursor != NAME_BYTES {
        return Err(KernelProviderError::InvalidCatalogue);
    }
    for (index, export) in EXPORTS.iter().enumerate() {
        let row = SYMBOLS_AT + (index + 1) * SYMBOL_BYTES;
        put(&mut image, STRINGS_AT + cursor, export.nid);
        put(
            &mut image,
            STRINGS_AT + cursor + 11,
            &[
                b'#',
                b'B' + export.library as u8,
                b'#',
                b'B' + export.module as u8,
            ],
        );
        put(&mut image, row, &(cursor as u32).to_le_bytes());
        image[row + 4] = 0x12;
        image[row + 5] = 3;
        put(&mut image, row + 6, &1_u16.to_le_bytes());
        put(
            &mut image,
            row + 8,
            &MARKERS[export.marker].rva.to_le_bytes(),
        );
        put(&mut image, row + 16, &8_u64.to_le_bytes());
        cursor += QUALIFIED_BYTES;
    }
    let base_entries = [
        (5_i64, STRINGS_AT as u64),
        (10, cursor as u64),
        (6, SYMBOLS_AT as u64),
        (39, ((EXPORTS.len() + 1) * SYMBOL_BYTES) as u64),
        (11, SYMBOL_BYTES as u64),
    ];
    for (index, (tag, value)) in base_entries.into_iter().enumerate() {
        put(&mut image, DYNAMIC_AT + index * 16, &tag.to_le_bytes());
        put(
            &mut image,
            DYNAMIC_AT + index * 16 + 8,
            &value.to_le_bytes(),
        );
    }
    for (index, &name) in names.iter().enumerate() {
        let at = DYNAMIC_AT + (5 + index) * 16;
        let value = name | (1 << 32) | (1 << 40) | ((index as u64 + 1) << 48);
        put(&mut image, at, &DT_SCE_MODULE_INFO.to_le_bytes());
        put(&mut image, at + 8, &value.to_le_bytes());
    }
    for (index, &name) in names.iter().take(LIBRARIES.len()).enumerate() {
        let at = DYNAMIC_AT + (5 + MODULES.len() + index) * 16;
        let value = name | (1 << 32) | ((index as u64 + 1) << 48);
        put(&mut image, at, &DT_SCE_EXPORT_LIB.to_le_bytes());
        put(&mut image, at + 8, &value.to_le_bytes());
    }
    Ok(GeneratedJsonProvider {
        image,
        load_bias: bias,
    })
}

#[cfg(test)]
mod tests;
