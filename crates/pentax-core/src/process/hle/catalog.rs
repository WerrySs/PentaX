// SPDX-License-Identifier: GPL-2.0-or-later

//! The fixed, reviewed HLE catalogue.
//!
//! Every value here is an exact identity of the single supported profile. It
//! is not an alias table, a NID-only index or an import list inferred from any
//! census. Internal service IDs live in a private PentaX protocol namespace;
//! they are neither NIDs, host addresses nor PS5 system-call numbers.

/// Byte-exact module and library names of the observed declaration pair.
pub(super) const MODULE_NAME: &[u8] = b"LibcInternal";
pub(super) const LIBRARY_NAME: &[u8] = b"LibcInternal";
pub(super) const EXT_LIBRARY_NAME: &[u8] = b"LibcInternalExt";
pub(super) const SCE_MODULE_NAME: &[u8] = b"libSceLibcInternal";
pub(super) const SCE_LIBRARY_NAME: &[u8] = b"libSceLibcInternal";
pub(super) const SCE_EXT_LIBRARY_NAME: &[u8] = b"libSceLibcInternalExt";
pub(super) const MODULE_MAJOR: u8 = 1;
pub(super) const MODULE_MINOR: u8 = 1;
pub(super) const LIBRARY_VERSION: u16 = 1;
/// Image-local descriptor selectors of the generated provider only. They are
/// not runtime identities; consumers use their own import selectors.
pub(super) const MODULE_SELECTOR: u16 = 1;
pub(super) const LIBRARY_SELECTOR: u16 = 1;

const SERVICE_BASE: u32 = 0x5058_4801;

/// One of the exact registered libc functions of the profile.
///
/// A plain value never authorizes dispatch or binds an import; only the owned
/// prepared session's attested sites do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KnownFunction {
    Memcpy,
    Memmove,
    Memset,
    Memcmp,
    Strcmp,
    Strncmp,
    Strlen,
    Strcpy,
    Strncpy,
    Strcat,
    Strchr,
    Strrchr,
    Strstr,
    HeapGetTraceInfo,
    HeapGetTraceInfoSce,
    CxaAtExit,
    Snprintf,
}

impl KnownFunction {
    pub const MEMORY: [Self; 4] = [Self::Memcpy, Self::Memmove, Self::Memset, Self::Memcmp];
    pub const LIBC: [Self; 13] = [
        Self::Memcpy,
        Self::Memmove,
        Self::Memset,
        Self::Memcmp,
        Self::Strcmp,
        Self::Strncmp,
        Self::Strlen,
        Self::Strcpy,
        Self::Strncpy,
        Self::Strcat,
        Self::Strchr,
        Self::Strrchr,
        Self::Strstr,
    ];
    /// Functions registered in the canonical libc scope and its finite,
    /// independently prefix-stripped module/library pairs. LIBC above retains
    /// the original thirteen memory/string operations for their own callers.
    pub const LIBC_FUNCTIONS: [Self; 15] = {
        let mut functions = [Self::Snprintf; 15];
        let mut index = 0;
        while index < Self::LIBC.len() {
            functions[index] = Self::LIBC[index];
            index += 1;
        }
        functions[13] = Self::CxaAtExit;
        functions
    };
    pub const ALL: [Self; 17] = {
        let mut functions = [Self::HeapGetTraceInfo; 17];
        let mut index = 0;
        while index < Self::LIBC.len() {
            functions[index] = Self::LIBC[index];
            index += 1;
        }
        functions[14] = Self::HeapGetTraceInfoSce;
        functions[15] = Self::CxaAtExit;
        functions[16] = Self::Snprintf;
        functions
    };

    pub(super) const fn library_selector(self) -> u16 {
        match self {
            Self::HeapGetTraceInfo => 2,
            Self::HeapGetTraceInfoSce => 3,
            _ => LIBRARY_SELECTOR,
        }
    }

    /// The canonical 11-byte NID registered for this function by the profile.
    #[must_use]
    pub const fn nid(self) -> &'static [u8; 11] {
        match self {
            Self::Memcpy => b"Q3VBxCXhUHs",
            Self::Memmove => b"+P6FRGH4LfA",
            Self::Memset => b"8zTFvBIAIN8",
            Self::Memcmp => b"DfivPArhucg",
            Self::Strcmp => b"aesyjrHVWy4",
            Self::Strncmp => b"Ovb2dSJOAuE",
            Self::Strlen => b"j4ViWNHEgww",
            Self::Strcpy => b"5Xa2ACNECdo",
            Self::Strncpy => b"6sJWiWSRuqk",
            Self::Strcat => b"Ls4tzzhimqQ",
            Self::Strchr => b"ob5xAW4ln-0",
            Self::Strrchr => b"9yDWMxEFdJU",
            Self::Strstr => b"viiwFMaNamA",
            Self::HeapGetTraceInfo | Self::HeapGetTraceInfoSce => b"NWtTN10cJzE",
            // Pinned KytyPS5 f186b357 libC.cpp:796, canonical LibcInternal 1/1.1.
            Self::CxaAtExit => b"tsvEmnenz48",
            Self::Snprintf => b"eLdDw6l0-bU",
        }
    }

    pub(super) const fn index(self) -> usize {
        match self {
            Self::Memcpy => 0,
            Self::Memmove => 1,
            Self::Memset => 2,
            Self::Memcmp => 3,
            Self::Strcmp => 4,
            Self::Strncmp => 5,
            Self::Strlen => 6,
            Self::Strcpy => 7,
            Self::Strncpy => 8,
            Self::Strcat => 9,
            Self::Strchr => 10,
            Self::Strrchr => 11,
            Self::Strstr => 12,
            Self::HeapGetTraceInfo => 13,
            Self::HeapGetTraceInfoSce => 14,
            Self::CxaAtExit => 15,
            Self::Snprintf => 16,
        }
    }

    /// PentaX protocol ID `0x5058_4801..=0x5058_4811`, compared against the
    /// complete 64-bit RAX at the owned trap site.
    pub(super) const fn service_id(self) -> u32 {
        SERVICE_BASE + self.index() as u32
    }

    pub(super) const fn string_operation(
        self,
    ) -> Option<crate::process::hle_strings::StringOperation> {
        use crate::process::hle_strings::StringOperation as Operation;
        match self {
            Self::Strcmp => Some(Operation::Strcmp),
            Self::Strncmp => Some(Operation::Strncmp),
            Self::Strlen => Some(Operation::Strlen),
            Self::Strcpy => Some(Operation::Strcpy),
            Self::Strncpy => Some(Operation::Strncpy),
            Self::Strcat => Some(Operation::Strcat),
            Self::Strchr => Some(Operation::Strchr),
            Self::Strrchr => Some(Operation::Strrchr),
            Self::Strstr => Some(Operation::Strstr),
            Self::Memcpy
            | Self::Memmove
            | Self::Memset
            | Self::Memcmp
            | Self::HeapGetTraceInfo
            | Self::HeapGetTraceInfoSce
            | Self::CxaAtExit
            | Self::Snprintf => None,
        }
    }
}
