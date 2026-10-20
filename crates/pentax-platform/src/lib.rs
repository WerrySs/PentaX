// SPDX-License-Identifier: GPL-2.0-or-later

//! Reviewed native memory, explicitly unsafe block stepping and original diagnostics.
//!
//! Owned reservations progress through reserved, writable, and executable states.
//! No safe API executes caller-supplied bytes. Safe CPU diagnostic entrypoints
//! accept the closed [`OriginalFixture`] enum. The return fixture is a leaf
//! function that modifies only RAX and does not access TLS, a red zone, imports
//! or kernel services. The separate Windows x86-64 `native_dispatch` API accepts
//! retained materialized source/context metadata but executes only through an explicit
//! unsafe contract requiring exact preparation provenance and confinement of
//! every ordinary native memory operand to appropriate live owned pages.
//! The illegal-instruction fixture intentionally terminates its calling process;
//! use this API only in the dedicated worker, never the desktop process.
//!
//! This is native code execution, not a sandbox. One closed entry fixture uses
//! an owned guarded stack and a fixed argument profile ported from KytyPS5.
//! A complete PS5 entry ABI, firmware TCB/DTV, general fault recovery and
//! arbitrary executable loading are not implemented here. The block stepper
//! leaves host FS/GS untouched; segment-prefixed instructions remain mediated.
//! A separate closed sum fixture rewrites its attested SYSCALL to UD2 before RX
//! publication and handles only that registered synthetic service in its worker.
//!
//! The closed [`OriginalFixture::NativeBlockDispatch`] program adds a private
//! translated-block diagnostic: it plans an original multi-block program
//! with `pentax-codegen`, publishes each block's ordinary instructions into an
//! owned writable->RX code cache, runs it under a fully owned guest context,
//! and mediates every control transfer in Rust against owned guest pages. It is
//! not a sandbox or game runtime; its context and normal return are checked by
//! the worker. The reusable stepper shares that bridge, binds the actual native
//! owner and attests live source/cache bytes per block. An original composed
//! kernel/TLS program returns 42 in debug and release; see the platform README.
//!
//! The implementation follows Microsoft's [VirtualAlloc], [VirtualProtect], and
//! [instruction-cache] contracts and Rust's explicit [sysv64 ABI].
//!
//! [VirtualAlloc]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc
//! [VirtualProtect]: https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect
//! [instruction-cache]: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-flushinstructioncache
//! [sysv64 ABI]: https://doc.rust-lang.org/reference/items/external-blocks.html#abi

use std::fmt;

pub mod host_memory;
pub mod native_memory;
pub mod network;
pub mod presenter;
pub mod supervised_process;
pub mod time_counter;
pub mod vulkan;

/// Audited unsafe native stepping; available only on the reviewed native host.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use backend::dispatch::prepared as native_dispatch;

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
#[path = "unsupported.rs"]
mod backend;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[path = "windows.rs"]
mod backend;

/// Guest mapping unit used by the current PS5 memory model.
pub const GUEST_PAGE_SIZE: usize = 0x4000;
/// Maximum size of a single reservation in this initial native boundary.
pub const MAX_RESERVATION_BYTES: usize = 256 * 1024 * 1024;

/// Host page and address-reservation alignment requirements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HostLayout {
    /// Host commitment and protection page size, obtained from GetSystemInfo.
    pub page_size: usize,
    /// Host reservation address granularity, obtained from GetSystemInfo.
    pub allocation_granularity: usize,
}

/// Failure before native execution or during an owned memory operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlatformError {
    /// This initial backend supports only Windows x86-64.
    UnsupportedHost,
    /// The reservation size is zero, not guest-page aligned, or exceeds its cap.
    InvalidReservationSize,
    /// The host page geometry cannot represent the guest mapping unit.
    UnsupportedPageLayout,
    /// A checked memory range overflows or exceeds the owned mapping.
    OutOfBounds,
    /// A Windows operation failed with its original GetLastError value.
    OperatingSystem {
        /// API that failed.
        operation: &'static str,
        /// Windows error code, captured immediately after failure.
        code: u32,
    },
    /// The original syscall fixture is rejected before allocation or execution.
    UnsupportedGuestSyscall {
        /// Syscall number declared by the original fixture.
        number: u64,
        /// Byte offset of the rejected syscall instruction in that fixture.
        offset: usize,
    },
    /// Internal fixture identity or its declared trap site failed attestation.
    InvalidFixtureTrap,
    /// The current thread already has an active trap scope or TLS is unavailable.
    TrapScopeUnavailable,
    /// Windows did not register the private fixture exception handler.
    TrapHandlerRegistrationFailed,
    /// Windows did not unregister the handler after its TLS scope was cleared.
    TrapHandlerRemovalFailed,
    /// The closed native fixture did not produce its single expected trap/result.
    UnexpectedFixtureTrapResult { value: u64, traps: u32 },
    /// Closed entry arguments, callback, or normal-return restoration disagreed.
    UnexpectedGuestStackEntryResult { value: u64, callbacks: u32 },
    /// An internal stage of the closed native-block dispatcher failed. The
    /// stage names a planning/staging/publication step, never guest input.
    NativeDispatchFailed { stage: &'static str },
    /// The host does not support a CPUID/OS feature the emission policy declared.
    UnsupportedHostFeature { feature: &'static str },
    /// Paused guest source bytes did not match the retained relocated window.
    SourceAttestationFailed,
    /// A published cache buffer or its engine tail failed byte attestation.
    EmissionAttestationFailed,
    /// A mediated interrupt/exception/unported instruction has no native path.
    UnsupportedGuestExit { guest_pc: u64 },
    /// A guest SYSCALL is refused before execution; metadata is retained only.
    UnknownGuestSyscall { rip: u64, number: u64 },
    /// A resolved transfer target was not mapped-executable or not canonical.
    GuestTargetRejected { target: u64 },
    /// The guest red-zone sentinel changed across a host round trip.
    RedZoneCorrupted { value: u64 },
    /// The closed dispatcher produced a value other than its verified result.
    UnexpectedDispatchResult { value: u64 },
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedHost => {
                formatter.write_str("native execution requires Windows x86-64")
            }
            Self::InvalidReservationSize => formatter
                .write_str("reservation must contain 1 through 16384 complete 16 KiB guest pages"),
            Self::UnsupportedPageLayout => {
                formatter.write_str("host page geometry cannot represent 16 KiB guest pages")
            }
            Self::OutOfBounds => {
                formatter.write_str("memory range exceeds the owned native reservation")
            }
            Self::OperatingSystem { operation, code } => {
                write!(formatter, "{operation} failed with Windows error {code}")
            }
            Self::UnsupportedGuestSyscall { number, offset } => write!(
                formatter,
                "guest syscall {number} at fixture offset {offset} is unsupported; no guest code was executed"
            ),
            Self::InvalidFixtureTrap => {
                formatter.write_str("original fixture trap attestation failed")
            }
            Self::TrapScopeUnavailable => {
                formatter.write_str("fixture trap scope is unavailable on this thread")
            }
            Self::TrapHandlerRegistrationFailed => {
                formatter.write_str("Windows did not register the fixture trap handler")
            }
            Self::TrapHandlerRemovalFailed => {
                formatter.write_str("Windows did not unregister the inactive fixture trap handler")
            }
            Self::UnexpectedFixtureTrapResult { value, traps } => write!(
                formatter,
                "original fixture returned value {value} after {traps} traps; expected value 42 after one trap"
            ),
            Self::UnexpectedGuestStackEntryResult { value, callbacks } => write!(
                formatter,
                "original guest-stack entry returned {value} after {callbacks} callbacks; expected 42 and one verified callback with restored host state"
            ),
            Self::NativeDispatchFailed { stage } => {
                write!(formatter, "native block dispatch failed at stage {stage}")
            }
            Self::UnsupportedHostFeature { feature } => write!(
                formatter,
                "host does not support required native emission feature {feature}"
            ),
            Self::SourceAttestationFailed => formatter
                .write_str("paused guest source bytes did not match the retained relocated window"),
            Self::EmissionAttestationFailed => {
                formatter.write_str("published cache bytes or engine tail failed attestation")
            }
            Self::UnsupportedGuestExit { guest_pc } => write!(
                formatter,
                "mediated exit at guest pc {guest_pc:#x} has no supported native resolution"
            ),
            Self::UnknownGuestSyscall { rip, number } => write!(
                formatter,
                "guest syscall {number} at rip {rip:#x} refused before execution; no kernel service was invoked"
            ),
            Self::GuestTargetRejected { target } => write!(
                formatter,
                "transfer target {target:#x} is not a mapped executable canonical address"
            ),
            Self::RedZoneCorrupted { value } => write!(
                formatter,
                "guest red-zone sentinel changed to {value} across a host round trip"
            ),
            Self::UnexpectedDispatchResult { value } => write!(
                formatter,
                "native block dispatch returned {value}; expected 42"
            ),
        }
    }
}

impl std::error::Error for PlatformError {}

/// Whether the initial native backend is compiled for this host.
#[must_use]
pub const fn supported_host() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
}

/// Query the host page size and reservation granularity without changing memory.
pub fn host_layout() -> Result<HostLayout, PlatformError> {
    backend::host_layout()
}

/// Disable crash dialogs in the calling worker process only.
///
/// This does not install an exception handler or attempt to resume after faults.
/// It must not be called by the desktop process as part of normal UI startup.
pub fn configure_worker_fault_policy() -> Result<(), PlatformError> {
    backend::configure_worker_fault_policy()
}

/// A private, inaccessible virtual-address reservation with one owning release.
///
/// Dropping any mapping state releases the reservation through VirtualFree.
/// There are no borrowed host slices or executable function pointers in this API.
pub struct GuestReservation {
    region: backend::Region,
}

impl GuestReservation {
    /// Reserve guest-page-aligned bytes without committing storage.
    pub fn reserve(length: usize) -> Result<Self, PlatformError> {
        validate_length(length)?;
        Ok(Self {
            region: backend::Region::reserve(length)?,
        })
    }

    /// Reserved byte length, always nonzero and a multiple of 16 KiB.
    #[must_use]
    pub fn len(&self) -> usize {
        self.region.len()
    }

    /// Owned reservations are never empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Numeric host address for diagnostics; it is not a safe dereference capability.
    #[must_use]
    pub fn address(&self) -> u64 {
        self.region.address()
    }

    /// Commit the complete owned reservation as read/write and non-executable.
    pub fn commit_writable(mut self) -> Result<WritableGuestMemory, PlatformError> {
        self.region.commit_writable()?;
        Ok(WritableGuestMemory {
            region: self.region,
        })
    }
}

/// Committed staging memory. It cannot be executed through this API.
pub struct WritableGuestMemory {
    region: backend::Region,
}

impl WritableGuestMemory {
    /// Number of owned bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.region.len()
    }

    /// Owned mappings are never empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Numeric host address for diagnostics.
    #[must_use]
    pub fn address(&self) -> u64 {
        self.region.address()
    }

    /// Copy bytes into a checked range; a rejected write changes no bytes.
    pub fn write(&mut self, offset: usize, bytes: &[u8]) -> Result<(), PlatformError> {
        checked_range(self.len(), offset, bytes.len())?;
        self.region.write(offset, bytes)
    }

    /// Revoke writes, grant read/execute, and flush the instruction cache.
    ///
    /// No read/write/execute protection is used at any stage. If either OS call
    /// fails, this consumed mapping is released without executing its contents.
    pub fn seal_executable(mut self) -> Result<ExecutableGuestMemory, PlatformError> {
        self.region.seal_executable()?;
        Ok(ExecutableGuestMemory {
            region: self.region,
        })
    }
}

/// Sealed read/execute memory with no safe write or arbitrary execution API.
pub struct ExecutableGuestMemory {
    region: backend::Region,
}

impl ExecutableGuestMemory {
    /// Number of owned bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.region.len()
    }

    /// Owned mappings are never empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Numeric host address for diagnostics.
    #[must_use]
    pub fn address(&self) -> u64 {
        self.region.address()
    }
}

/// The only executable inputs accepted by the initial worker boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginalFixture {
    /// Original leaf sums System V RDI/RSI arguments 19 and 23 and returns 42.
    Return42,
    /// Original `ud2` instruction, which intentionally faults the worker process.
    InvalidInstruction,
    /// A syscall-number 65535 fixture, rejected before native execution.
    UnsupportedSyscall,
    /// Original synthetic sum service, trapped before any native SYSCALL executes.
    /// Its service number and behavior are private diagnostics, not a PS5 ABI.
    TrappedSum42,
    /// Original argument/stack/red-zone/callback fixture on an owned RW/NX stack.
    /// It uses a fixed PentaX profile, not a claim of general PS5 entry support.
    GuestStackEntry42,
    /// Original multi-block program run by the private translated-block engine.
    /// It uses actual pentax-codegen block plans, owned native guest pages and a
    /// full guest-context bridge, mediating every control transfer in Rust. It
    /// returns 42. This is real native execution of straight-line blocks, not a
    /// sandbox, byte interpreter, PS5 ABI or arbitrary-target dispatcher.
    NativeBlockDispatch,
}

/// Numeric diagnostics from one closed original fixture execution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OriginalFixtureResult {
    pub value: u64,
    pub traps: u32,
}

// mov eax,0x50580001; syscall; ret. This is an original synthetic service,
// never a PS5 or Windows syscall number. The raw form is never executable.
const TRAPPED_SUM_ORIGINAL: [u8; 8] = [0xb8, 0x01, 0x00, 0x58, 0x50, 0x0f, 0x05, 0xc3];
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const TRAPPED_SUM_OFFSET: usize = 5;
#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
const SYNTHETIC_SUM_SERVICE: u64 = 0x5058_0001;

#[cfg(any(test, all(target_os = "windows", target_arch = "x86_64")))]
fn prepare_trapped_sum(original: &[u8]) -> Result<[u8; 8], PlatformError> {
    if original != TRAPPED_SUM_ORIGINAL {
        return Err(PlatformError::InvalidFixtureTrap);
    }
    let mut prepared = TRAPPED_SUM_ORIGINAL;
    prepared[TRAPPED_SUM_OFFSET..TRAPPED_SUM_OFFSET + 2].copy_from_slice(&[0x0f, 0x0b]);
    Ok(prepared)
}

fn fixture_bytes(fixture: OriginalFixture) -> &'static [u8] {
    match fixture {
        // lea rax,[rdi+rsi]; ret. Using arguments proves the SysV64 register
        // convention on Windows; a zero-argument immediate return would not.
        OriginalFixture::Return42 => &[0x48, 0x8d, 0x04, 0x37, 0xc3],
        OriginalFixture::InvalidInstruction => &[0x0f, 0x0b],
        // mov rax,65535; syscall; ud2. These original bytes are deliberately
        // never mapped or executed. This is a closed-fixture rejection policy,
        // not an instruction decoder or a general syscall interception claim.
        OriginalFixture::UnsupportedSyscall => &[
            0x48, 0xb8, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0x0f, 0x05, 0x0f, 0x0b,
        ],
        OriginalFixture::TrappedSum42 => &TRAPPED_SUM_ORIGINAL,
        // This fixture is prepared only by the private owned-stack path.
        OriginalFixture::GuestStackEntry42 | OriginalFixture::NativeBlockDispatch => &[],
    }
}

/// Run a closed original fixture in the calling worker process.
///
/// The illegal-instruction case intentionally does not return on a supported host.
/// The unsupported-syscall fixture never allocates, maps, or executes code. These
/// cases do not establish general guest ABI, kernel, TLS, or game compatibility.
pub fn execute_original_fixture(fixture: OriginalFixture) -> Result<u64, PlatformError> {
    execute_original_fixture_report(fixture).map(|result| result.value)
}

/// Run a closed fixture and report its observed synthetic trap count.
///
/// The trapped sum uses no custom stack, red zone, guest TLS or host service.
/// General syscall interception and arbitrary executable loading remain absent.
pub fn execute_original_fixture_report(
    fixture: OriginalFixture,
) -> Result<OriginalFixtureResult, PlatformError> {
    if fixture == OriginalFixture::UnsupportedSyscall {
        return Err(PlatformError::UnsupportedGuestSyscall {
            number: 65535,
            offset: 10,
        });
    }
    if fixture == OriginalFixture::TrappedSum42 {
        return backend::execute_trapped_sum42();
    }
    if fixture == OriginalFixture::GuestStackEntry42 {
        return backend::execute_guest_stack_entry42();
    }
    if fixture == OriginalFixture::NativeBlockDispatch {
        return backend::execute_native_block_dispatch();
    }
    let bytes = fixture_bytes(fixture);
    let mut staging = GuestReservation::reserve(GUEST_PAGE_SIZE)?.commit_writable()?;
    staging.write(0, bytes)?;
    let executable = staging.seal_executable()?;
    // SAFETY: The closed match above is the only source of these instructions.
    // Return42 is a complete SysV64 leaf function with only the allowed RAX effect.
    // UD2 intentionally terminates the worker and cannot return with corrupt ABI
    // state. No caller-provided bytes, TLS, stack switch, imports, or syscalls run.
    let value = unsafe { executable.region.call_original_fixture() };
    Ok(OriginalFixtureResult { value, traps: 0 })
}

fn validate_length(length: usize) -> Result<(), PlatformError> {
    if length == 0 || length > MAX_RESERVATION_BYTES || !length.is_multiple_of(GUEST_PAGE_SIZE) {
        return Err(PlatformError::InvalidReservationSize);
    }
    Ok(())
}

fn checked_range(length: usize, offset: usize, size: usize) -> Result<(), PlatformError> {
    if offset.checked_add(size).is_none_or(|end| end > length) {
        return Err(PlatformError::OutOfBounds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_syscall_is_replaced_before_executable_publication() {
        let original = TRAPPED_SUM_ORIGINAL;
        assert_eq!(
            u32::from_le_bytes(original[1..5].try_into().unwrap()) as u64,
            SYNTHETIC_SUM_SERVICE
        );
        assert_eq!(
            &original[TRAPPED_SUM_OFFSET..TRAPPED_SUM_OFFSET + 2],
            &[0x0f, 0x05]
        );
        let prepared = prepare_trapped_sum(&original).unwrap();
        assert_eq!(
            &prepared[..TRAPPED_SUM_OFFSET],
            &original[..TRAPPED_SUM_OFFSET]
        );
        assert_eq!(
            &prepared[TRAPPED_SUM_OFFSET..TRAPPED_SUM_OFFSET + 2],
            &[0x0f, 0x0b]
        );
        assert_eq!(prepared[7], original[7]);
        assert!(!prepared.windows(2).any(|pair| pair == [0x0f, 0x05]));
        assert_eq!(original, TRAPPED_SUM_ORIGINAL);
        let mut wrong_site = original;
        wrong_site[TRAPPED_SUM_OFFSET + 1] = 0x0b;
        assert_eq!(
            prepare_trapped_sum(&wrong_site),
            Err(PlatformError::InvalidFixtureTrap)
        );
        let mut wrong_service = original;
        wrong_service[1] ^= 1;
        assert_eq!(
            prepare_trapped_sum(&wrong_service),
            Err(PlatformError::InvalidFixtureTrap)
        );
    }

    #[test]
    fn rejects_invalid_reservations_and_overflowing_ranges() {
        for size in [
            0,
            1,
            GUEST_PAGE_SIZE - 1,
            MAX_RESERVATION_BYTES + GUEST_PAGE_SIZE,
            usize::MAX,
        ] {
            assert!(matches!(
                GuestReservation::reserve(size),
                Err(PlatformError::InvalidReservationSize)
            ));
        }
        assert_eq!(
            checked_range(GUEST_PAGE_SIZE, usize::MAX, 2),
            Err(PlatformError::OutOfBounds)
        );
        assert_eq!(
            checked_range(GUEST_PAGE_SIZE, GUEST_PAGE_SIZE - 1, 2),
            Err(PlatformError::OutOfBounds)
        );
        assert_eq!(checked_range(GUEST_PAGE_SIZE, GUEST_PAGE_SIZE, 0), Ok(()));
    }

    #[test]
    fn unsupported_syscall_is_rejected_without_native_execution() {
        let bytes = fixture_bytes(OriginalFixture::UnsupportedSyscall);
        assert_eq!(&bytes[10..12], &[0x0f, 0x05]);
        assert_eq!(u64::from_le_bytes(bytes[2..10].try_into().unwrap()), 65535);
        assert_eq!(
            execute_original_fixture(OriginalFixture::UnsupportedSyscall),
            Err(PlatformError::UnsupportedGuestSyscall {
                number: 65535,
                offset: 10
            })
        );
    }

    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    #[test]
    fn unsupported_hosts_fail_explicitly() {
        assert!(!supported_host());
        assert_eq!(host_layout(), Err(PlatformError::UnsupportedHost));
        assert_eq!(
            execute_original_fixture(OriginalFixture::Return42),
            Err(PlatformError::UnsupportedHost)
        );
        assert_eq!(
            execute_original_fixture_report(OriginalFixture::TrappedSum42),
            Err(PlatformError::UnsupportedHost)
        );
        assert_eq!(
            execute_original_fixture_report(OriginalFixture::GuestStackEntry42),
            Err(PlatformError::UnsupportedHost)
        );
        assert_eq!(
            execute_original_fixture_report(OriginalFixture::NativeBlockDispatch),
            Err(PlatformError::UnsupportedHost)
        );
    }
}
