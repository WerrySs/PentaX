// SPDX-License-Identifier: GPL-2.0-or-later

//! Exact generated HLE providers and a bounded interpreter gateway.
//!
//! One original guest ELF provider registers reviewed memory/string and lifecycle functions under
//! the single reviewed profile. It passes through the existing exact-image
//! linker: ordinary name/version/NID/type resolution, no HLE priority, no
//! unreviewed alias or NID-only lookup and no automatic discovery. Unknown imports stay
//! missing or mismatched. Rust services use sealed checked memory operations on
//! sparse [`GuestMemory`] or the actual exclusively paused native backing.
//!
//! Ordinary public preparation returns read-only unstarted diagnostics. Native
//! preparation owns physically NX pages with exclusive paused access. The
//! composed kernel startup exposes owner-bound materialization provenance and
//! checked PRE-SYSCALL service requests, without a core native runner. Built-in
//! interpreter diagnostics use closed original [`run_hle_memory_fixture`],
//! [`run_hle_string_fixture`] and [`run_hle_startup_fixture`] consumers. The
//! separate unsafe platform/worker runner consumes prepared native sessions.
//! None proves a PS5 ABI, startup, lifecycle or compatibility with any title.
//!
//! [`GuestMemory`]: crate::memory::GuestMemory

mod catalog;
mod dispatch;
mod fixture;
mod gateway;
mod graphics_provider;
mod json_provider;
mod kernel_adapter;
mod kernel_provider;
mod kernel_startup;
mod native;
mod native_image;
mod native_startup;
mod provider;
mod session;
mod startup;
mod startup_fixture;
mod string_fixture;
#[cfg(test)]
mod tests;
mod unavailable;

pub use catalog::KnownFunction;
pub use dispatch::{MemoryServiceError, MemoryServiceLimits, PointerArgument, ServiceLimit};
pub use fixture::{HleFixtureReport, run_hle_memory_fixture};
pub use graphics_provider::{GeneratedGraphicsProvider, build_graphics_provider};
pub use json_provider::{GeneratedJsonProvider, build_json_provider};
pub use kernel_provider::{GeneratedKernelProvider, KernelProviderError, build_kernel_provider};
pub use kernel_startup::{
    DynamicModuleInput, DynamicModuleStart, KernelStartupError, KernelStartupLimits,
    KernelStartupPlan, LibcBuiltinError, LibcBuiltinUsage, NativeEntryError, NativeEntryLayout,
    NativeEntryLimits, NativeEntryPolicy, NativeEntryRequest, NativeModuleStartError,
    NativeServiceAction, NativeServiceRequest, OriginalPreparedError, OriginalPreparedLayout,
    OriginalPreparedReport, PendingModuleLoad, PendingThreadCreate, PendingThreadWait,
    PreparedFontParserCallback, PreparedImeCallback, PreparedLibcBuiltins, PreparedNativeEntry,
    PreparedNativeKernelStartup, PreparedNativeModuleStart, PreparedNativeSource,
    PreparedNativeThread, PreparedNetCtlCallback, PreparedPthreadOnce, PreparedThreadStart,
    ThreadEntryLayout, plan_kernel_startup, prepare_native_kernel_startup,
    prepare_native_kernel_startup_in, run_original_prepared_fixture,
};
pub use kernel_startup::{GraphicsStartupOptions, plan_kernel_startup_with_graphics};
pub use kernel_startup::{
    JsonStartupOptions, KernelStartupExtensions, plan_kernel_startup_with_extensions,
};
pub use kernel_startup::{UnavailableProviderPlacement, plan_kernel_startup_with_unavailable};
pub use native::{PreparedNativeHleSession, prepare_native_hle_session};
pub use native_image::{
    NativeImageError, NativeImageFileBacking, NativeImageLayout, NativeImageLimits,
    NativeResourceError, with_native_image_session, with_native_image_session_with_roots,
};
pub use native_startup::{PreparedNativeHleStartup, prepare_native_hle_startup};
pub use provider::{GeneratedProviders, build_hle_providers};
pub use session::{HleSessionPlan, PreparedHleSession, plan_hle_session, prepare_hle_session};
pub use startup::{
    HleStartupLimits, HleStartupPlan, HleStartupPolicy, PreparedHleStartup, plan_hle_startup,
    prepare_hle_startup,
};
pub use startup_fixture::{HleStartupFixtureReport, run_hle_startup_fixture};
pub use string_fixture::{HleStringFixtureReport, run_hle_string_fixture};
pub use unavailable::{
    GeneratedUnavailableProvider, UnavailableProviderError, UnavailableProviderLimits,
    build_unavailable_provider,
};

use super::ProcessError;
use crate::cpu::CpuError;
use crate::memory::{MemoryError, PAGE_SIZE};
use pentax_linker::{SymbolicError, SymbolicLimits, TlsBindingError};
use pentax_loader::LoadError;
use std::fmt;

/// The fixed catalogue selected by a caller; it carries no registrations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum HleProfile {
    /// Exactly module `LibcInternal` 1.1 and library `LibcInternal` version 1,
    /// as observed in public KytyPS5 registrations, with four byte-memory and
    /// nine string functions, plus the canonical and libSce-prefixed v1 heap
    /// trace extension pairs, as
    /// GLOBAL/PROTECTED FUNC exports, and the pinned canonical LibcInternal
    /// uint32 dependency flag as a separate writable OBJECT initialized to one,
    /// with explicit canonical/libSce scope pairs from the reviewed normalization.
    /// The source-supported __cxa_atexit registration retains callback/argument/
    /// DSO words in bounded process-owned storage; it does not run finalization.
    /// No unreviewed name,
    /// version or function is aliased or inferred.
    KytyObservedLibcInternalV1,
}

/// Planning, generation and per-session service bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HleLimits {
    /// Applied to the complete supplied-plus-generated session.
    pub linking: SymbolicLimits,
    pub max_provider_bytes: u64,
    pub services: MemoryServiceLimits,
}

impl Default for HleLimits {
    fn default() -> Self {
        Self {
            linking: SymbolicLimits::default(),
            max_provider_bytes: PAGE_SIZE,
            services: MemoryServiceLimits::default(),
        }
    }
}

/// A guest load bias only; never a registration, source or host address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPlacement {
    pub load_bias: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HleBudget {
    Modules,
    ProviderBytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HleError {
    Link(SymbolicError),
    Load(LoadError),
    Staging(ProcessError),
    Memory(MemoryError),
    Service(MemoryServiceError),
    TlsBinding(TlsBindingError),
    Tls(super::tls::TlsError),
    Allocation,
    NoPrimaryInput,
    InvalidPlacement {
        load_bias: u64,
    },
    LimitExceeded {
        kind: HleBudget,
        requested: u64,
        maximum: u64,
    },
    InvalidProvider(&'static str),
    /// Ordinary HLE preparation refuses TLS; the explicit startup API is separate.
    UnsupportedTls {
        module: usize,
    },
    DeferredRelocations {
        count: usize,
    },
    /// A SYSCALL stop whose RIP is not an owned generated resume site.
    SiteMismatch {
        rip: u64,
    },
    ContinuationMismatch {
        rip: u64,
        rcx: u64,
    },
    /// The complete 64-bit RAX differs from the site's PentaX service ID.
    ServiceIdMismatch {
        expected: u32,
        actual: u64,
    },
    InstructionLimit {
        limit: u64,
        retired: u64,
    },
    Cpu(CpuError),
    Fixture(&'static str),
}

impl fmt::Display for HleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Link(error) => write!(f, "HLE session linking: {error}"),
            Self::Load(error) => write!(f, "HLE image: {error}"),
            Self::Staging(error) => write!(f, "HLE session staging: {error}"),
            Self::Memory(error) => write!(f, "HLE fixture memory: {error}"),
            Self::Service(error) => write!(f, "HLE memory service: {error}"),
            Self::TlsBinding(error) => write!(f, "HLE startup TLS binding: {error}"),
            Self::Tls(error) => write!(f, "HLE startup TLS preparation: {error}"),
            Self::Allocation => f.write_str("HLE host allocation failed"),
            Self::NoPrimaryInput => f.write_str("HLE session requires a supplied primary image"),
            Self::InvalidPlacement { load_bias } => write!(
                f,
                "provider bias {load_bias:#x} must be 16 KiB-aligned within the 47-bit space"
            ),
            Self::LimitExceeded {
                kind,
                requested,
                maximum,
            } => write!(f, "HLE {kind:?} {requested} exceeds limit {maximum}"),
            Self::InvalidProvider(reason) => write!(f, "generated provider: {reason}"),
            Self::UnsupportedTls { module } => write!(
                f,
                "module {module} declares TLS; HLE preparation does not initialize TLS"
            ),
            Self::DeferredRelocations { count } => write!(
                f,
                "HLE preparation requires complete relocations; {count} records are deferred"
            ),
            Self::SiteMismatch { rip } => {
                write!(f, "SYSCALL stop at {rip:#x} is not an owned provider site")
            }
            Self::ContinuationMismatch { rip, rcx } => write!(
                f,
                "SYSCALL continuation RCX {rcx:#x} differs from site {rip:#x}"
            ),
            Self::ServiceIdMismatch { expected, actual } => write!(
                f,
                "RAX {actual:#x} differs from PentaX service {expected:#x} at this site"
            ),
            Self::InstructionLimit { limit, retired } => write!(
                f,
                "HLE instruction budget {limit} exhausted after {retired} instructions"
            ),
            Self::Cpu(error) => write!(f, "HLE interpreter: {error}"),
            Self::Fixture(reason) => write!(f, "HLE fixture assertion failed: {reason}"),
        }
    }
}

impl std::error::Error for HleError {}

impl From<MemoryError> for HleError {
    fn from(error: MemoryError) -> Self {
        Self::Memory(error)
    }
}
