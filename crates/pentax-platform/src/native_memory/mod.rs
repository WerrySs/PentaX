// SPDX-License-Identifier: GPL-2.0-only

//! Owned paused native guest storage. Source pages are physically NX at every
//! point; logical EXECUTE permits checked source-byte fetch only. No function
//! pointer, raw slice, running state or executable conversion is exposed.
//!
//! Reservations use owned Windows placeholders, holes remain PAGE_NOACCESS, and
//! protections follow [VirtualProtect]. Only owned reservation bases reach
//! [VirtualFree] with MEM_RELEASE. Decommit ranges remain inside those owned
//! reservations. External raw-pointer mutation is outside Rust's safe API.
//!
//! Mapping commits backing eagerly; an explicit physical-section policy can
//! defer section commitment until first mapping. Full section capacity still
//! consumes the backing budget. Committed offsets count once across aliases;
//! virtual mapped bytes count every alias.
//! Neither count is physical working-set size. Complete spans precede copies.
//! New mapping/protection failures roll back unpublished OS changes; an
//! unexpected rollback failure poisons the owner and refuses further accesses.
//! This storage module exposes no execution API; the owner is neither Send nor Sync.
//!
//! The separate `native_dispatch` API supplies reviewed context/cache and
//! stack/target checks through an explicit unsafe one-block boundary. Its caller
//! must independently confine ordinary native operands to live owned pages;
//! source equality and worker fault isolation do not establish that guarantee.
//! An original prepared kernel/TLS program runs in debug and release;
//! cancellation, broader state and general guest fault recovery remain unfinished.
//! Source logical-X pages remain OS NX and must be mediated through a separately
//! owned code cache. HLE must use the same paused backing, never a sparse snapshot;
//! writable-code/source/service changes require cache invalidation before resume.
//!
//! [VirtualAlloc]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc
//! [VirtualProtect]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect
//! [VirtualFree]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree

use std::fmt;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
use std::sync::atomic::{AtomicU64, Ordering};

mod transaction;
pub use transaction::{
    NativeMemoryEffect, NativeMemoryTransaction, NativeMemoryWrite, NativeTransactionLimits,
    NativeWordReplacement,
};

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod windows;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use windows::{NativeAddressSpace, NativeFileBacking, PausedNativeMemory};
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
mod unsupported;
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub use unsupported::{NativeAddressSpace, NativeFileBacking, PausedNativeMemory};

pub const NATIVE_GUEST_PAGE: u64 = 16 * 1024;
pub const NATIVE_ADDRESS_LIMIT: u64 = 1 << 47;

/// Opaque process-local identity of one address-space owner. Moving an owner
/// preserves it; replacement owners receive distinct identities even if the OS
/// reuses their addresses. Identity alone proves neither liveness nor access.
/// There is no public constructor or conversion from a numeric address.
///
/// ```compile_fail
/// use pentax_platform::native_memory::NativeOwnerId;
/// let forged = NativeOwnerId(1);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeOwnerId(u64);

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
static NEXT_OWNER_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
fn allocate_owner_id(counter: &AtomicU64) -> Result<NativeOwnerId, NativeMemoryError> {
    // Uniqueness needs only an atomic modification order, not publication of
    // memory. Exhaustion never wraps, resets or reuses an earlier identity.
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(1)
        })
        .map(NativeOwnerId)
        .map_err(|_| NativeMemoryError::OwnerIdentityExhausted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativePermissions(u8);
impl NativePermissions {
    pub const fn new(read: bool, write: bool, execute: bool) -> Self {
        Self(read as u8 | ((write as u8) << 1) | ((execute as u8) << 2))
    }
    pub const READ: Self = Self(1);
    pub const READ_WRITE: Self = Self(3);
    pub const READ_EXECUTE: Self = Self(5);
    pub const fn readable(self) -> bool {
        self.0 & 1 != 0
    }
    pub const fn writable(self) -> bool {
        self.0 & 2 != 0
    }
    pub const fn executable(self) -> bool {
        self.0 & 4 != 0
    }
    /// Set GPU access independently; it never grants logical CPU access.
    pub const fn with_gpu(self, read: bool, write: bool) -> Self {
        Self((self.0 & !0x30) | ((read as u8) << 4) | ((write as u8) << 5))
    }
    pub const fn gpu_readable(self) -> bool {
        self.0 & 0x10 != 0
    }
    pub const fn gpu_writable(self) -> bool {
        self.0 & 0x20 != 0
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeMemoryLimits {
    pub mapped_bytes: u64,
    pub reserved_bytes: u64,
}
/// Actual retained owner counters and ceilings, not RSS or access authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeMemoryStatistics {
    pub mapped_bytes: u64,
    /// Pagefile-backed commitment, excluding data-file views and working set.
    pub committed_bytes: u64,
    pub backing_bytes: u64,
    pub file_backed_bytes: u64,
    pub private_commit_limit_bytes: u64,
    pub reserved_bytes: u64,
    pub limits: NativeMemoryLimits,
    /// Only the latest memory transaction's closed OS failure category/code.
    pub host_failure: Option<NativeHostMemoryFailure>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeHostMemoryFailure {
    pub operation: NativeHostMemoryOperation,
    pub code: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeHostMemoryOperation {
    SectionCreate,
    SectionCommit,
    SectionMap,
    PrivateCommit,
    Reserve,
    Protect,
    BackingFile,
    Other,
}
impl NativeHostMemoryOperation {
    pub const fn label(self) -> &'static str {
        match self {
            Self::SectionCreate => "section-create",
            Self::SectionCommit => "section-commit",
            Self::SectionMap => "section-map",
            Self::PrivateCommit => "private-commit",
            Self::Reserve => "reserve",
            Self::Protect => "protect",
            Self::BackingFile => "backing-file",
            Self::Other => "other",
        }
    }
}
impl Default for NativeMemoryLimits {
    fn default() -> Self {
        Self {
            mapped_bytes: 256 * 1024 * 1024,
            reserved_bytes: 512 * 1024 * 1024,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeRange {
    pub address: u64,
    pub size: u64,
}

/// Independent bounded virtual-name metadata; none of these limits grant
/// mapping, payload access or execution authority. Work is cumulative per owner.
#[derive(Debug, Clone, Copy)]
pub struct NativeNameLimits {
    pub max_records: usize,
    pub max_work: u64,
    pub max_scratch_bytes: usize,
}
impl Default for NativeNameLimits {
    fn default() -> Self {
        Self {
            max_records: 4096,
            max_work: 16 * 1024 * 1024,
            max_scratch_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Copied guest label and semantic extent, never a host string or pointer.
#[derive(Clone, Copy)]
pub struct NativeRangeName {
    pub range: NativeRange,
    bytes: [u8; 32],
    length: u8,
}
impl NativeRangeName {
    pub fn name(&self) -> &[u8] {
        &self.bytes[..usize::from(self.length)]
    }
}
impl fmt::Debug for NativeRangeName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeRangeName")
            .field("range", &self.range)
            .field("name_bytes", &self.length)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeMemoryError {
    UnsupportedHost,
    UnsupportedPageLayout,
    InvalidRange,
    Unaligned,
    Overlap,
    BudgetExceeded,
    Allocation,
    /// A trusted loader replacement no longer matches its retained old word.
    ReplacementMismatch,
    Unmapped(u64),
    PermissionDenied(u64),
    Poisoned,
    /// The process-local owner identity counter cannot advance without wrap.
    /// No host query or allocation is attempted for that constructor call.
    OwnerIdentityExhausted,
    OperatingSystem {
        operation: &'static str,
        address: u64,
        code: u32,
    },
}
impl fmt::Display for NativeMemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "native guest memory: {self:?}")
    }
}
impl std::error::Error for NativeMemoryError {}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn identity_exhaustion_is_permanent_and_does_not_wrap() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_owner_id(&counter), Ok(NativeOwnerId(u64::MAX - 1)));
        for _ in 0..2 {
            assert_eq!(
                allocate_owner_id(&counter),
                Err(NativeMemoryError::OwnerIdentityExhausted)
            );
            assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
        }
    }

    #[test]
    fn simultaneous_identity_allocations_never_repeat() {
        let counter = AtomicU64::new(1);
        let mut observed = std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                (0..32)
                    .map(|_| allocate_owner_id(&counter).unwrap())
                    .collect::<Vec<_>>()
            });
            let second = scope.spawn(|| {
                (0..32)
                    .map(|_| allocate_owner_id(&counter).unwrap())
                    .collect::<Vec<_>>()
            });
            let mut values = first.join().unwrap();
            values.extend(second.join().unwrap());
            values
        });
        observed.sort_unstable_by_key(|id| id.0);
        observed.dedup();
        assert_eq!(observed.len(), 64);
        assert_eq!(counter.load(Ordering::Relaxed), 65);
    }
}
