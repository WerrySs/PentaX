// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: safe checked guest access, bounded owned state,
// typed resources and explicit unsupported operations. See NOTICE and Kyty-MIT.txt.

//! Stateful port of the pinned Kyty library-call profile, not raw system calls.
//! A caller must retain the same actual guest backing through [`GuestAccess`].
//! No guest integer is a host pointer and this crate has no execution interface.

mod acm;
mod ampr;
mod application_heap;
mod audio_out2;
pub use audio_out2::{AudioOut2Usage, AudioOut2Wait, AudioOut2WaitAction};
mod budget;
mod calendar;
mod catalog;
mod content_init;
mod convert_keycode;
mod dialog;
mod entitlement;
mod event_queue;
mod fiber;
mod filesystem;
mod font;
mod game_intent;
mod handle_usage;
mod http2;
mod ime;
mod library_results;
mod memory;
mod module_unwind;
mod mspace;
pub use module_unwind::ModuleUnwindInfo;
mod net_ctl;
mod network;
mod np;
mod np_webapi2;
mod output;
mod pad;
pub use pad::{
    PadControllerInformation, PadInputSnapshot, PadOutputState, PadTouch, PadTriggerRequest,
};
mod playgo;
mod posix;
mod pthread_attr;
mod pthread_condition;
mod pthread_create;
mod pthread_detach;
mod pthread_join;
mod pthread_keys;
mod pthread_mutex;
mod pthread_once;
mod pthread_rwlock;
mod pthread_semaphore;
pub use pthread_semaphore::{PthreadSemaphoreWait, PthreadSemaphoreWaitAction};
mod pthread_yield;
mod raw_input;
mod rudp;
mod runtime_data;
mod save;
mod share_media;
mod sync;
mod sysmodule;
pub use sysmodule::game_module_path;
mod system;
mod system_events;
mod system_notice;
mod thread_context;
mod thread_exit;
mod thread_priority;
mod time;
mod trophy2;
mod uds;
mod user;
mod user_privacy;

pub use application_heap::ApplicationHeapCallbacks;
pub use budget::{Limits, Usage};
pub use catalog::{Registration, Service, resolve};
pub use dialog::CommonDialogStatus;
pub use entitlement::{LocalAddcont, LocalEntitlements};
pub use event_queue::{
    EventQueueWait, EventQueueWaitAction, GraphicsInterrupt, VideoBoundary, VideoBoundaryBatch,
    VideoClose, VideoEventKind, VideoInterrupt,
};
pub use fiber::{AdmittedFiberTransfer, FiberAction, FiberKey, FiberTransferKind};
pub use filesystem::{ReadOnlyFile, ReadOnlyRoot};
pub use font::{AdmittedFontParser, FontParserAction};
pub use handle_usage::HandleUsage;
pub use ime::{AdmittedImeCallback, ImeCallbackAction, ImeVisualState};
pub use memory::MemoryMapRefusal;
pub use net_ctl::{AdmittedNetCtlCallback, NetCtlCallbackAction};
pub use np::callback::{AdmittedNpCallback, NpCallbackAction};
pub use output::{OutputStream, OutputUsage};
pub use pentax_config::SystemConfig;
pub use pentax_config::{UserConfig, UserName};
pub use pthread_attr::ThreadAttributes;
pub use pthread_condition::{ConditionWait, ConditionWaitAction};
pub use pthread_create::{AdmittedPthreadCreate, PthreadCreateAction};
pub use pthread_detach::pthread_detach_error;
pub use pthread_mutex::{MutexWait, MutexWaitAction};
pub use pthread_once::{AdmittedPthreadOnce, PthreadOnceAction};
pub use pthread_rwlock::{RwlockWait, RwlockWaitAction};
pub use runtime_data::RuntimeDataOutputs;
pub use save::SaveStorage;
pub use sync::{EventFlagWait, EventFlagWaitAction, SemaphoreWait, SemaphoreWaitAction};
pub use thread_exit::{AdmittedThreadExitCallback, ThreadExitAction};
pub use thread_priority::ThreadPriorityClass;
pub use time::{DisplayClock, ProcessClock, TimeWait, TimeWaitAction};

/// The six AMD64 SysV **library-call** integer register arguments, in order.
///
/// The adapter must preserve original RCX before a SYSCALL marker overwrites it
/// with its continuation. Argument four here is original RCX, not continuation
/// RCX or the independent raw-kernel-syscall R10 convention.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallArgs(pub [u64; 6]);

/// Budget-admitted request data, not a file, mapping or execution capability.
/// Only the retained kernel can construct it after one attempt/name/output
/// admission. An image owner must still resolve an explicit file capability.
pub struct AdmittedModuleLoad {
    path: String,
    raw: CallArgs,
    result: Option<u64>,
    sysmodule: Option<SysmoduleLoadCompletion>,
}

/// Data admitted for deferred reference publication after a real module start.
/// The native owner must attest the corresponding return and prevent replay.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct SysmoduleLoadCompletion {
    service: Service,
    args: CallArgs,
}
impl std::fmt::Debug for SysmoduleLoadCompletion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SysmoduleLoadCompletion")
            .finish_non_exhaustive()
    }
}
impl AdmittedModuleLoad {
    pub fn path(&self) -> &str {
        &self.path
    }
    pub const fn module_args(&self) -> u64 {
        if self.sysmodule.is_some() {
            0
        } else {
            self.raw.0[1]
        }
    }
    pub const fn argument_pointer(&self) -> u64 {
        if self.sysmodule.is_some() {
            0
        } else {
            self.raw.0[2]
        }
    }
    pub const fn result_pointer(&self) -> Option<u64> {
        self.result
    }
    pub const fn raw_arguments(&self) -> CallArgs {
        self.raw
    }
    pub const fn sysmodule_completion(&self) -> Option<SysmoduleLoadCompletion> {
        self.sysmodule
    }
}
impl std::fmt::Debug for AdmittedModuleLoad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdmittedModuleLoad").finish_non_exhaustive()
    }
}

/// Source errno values. These are not Windows or host Rust errno values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Errno {
    Perm = 1,
    NoEntry = 2,
    NoSuchResource = 3,
    Io = 5,
    BadFd = 9,
    Deadlock = 11,
    NoMemory = 12,
    Access = 13,
    Fault = 14,
    Busy = 16,
    Exists = 17,
    NotDirectory = 20,
    IsDirectory = 21,
    Invalid = 22,
    FileTableFull = 23,
    TooManyFiles = 24,
    ReadOnly = 30,
    Again = 35,
    NotSupported = 45,
    TimedOut = 60,
    NameTooLong = 63,
    Overflow = 84,
    Canceled = 85,
}

impl Errno {
    #[must_use]
    pub const fn kernel_code(self) -> i32 {
        (0x8002_0000_u32 + self as u32) as i32
    }
}

/// Limits and unavailable execution semantics are distinct from guest errno.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Errno(Errno),
    Guest(GuestError),
    Limit(&'static str),
    Allocation,
    Unsupported(&'static str),
}

impl From<Errno> for Failure {
    fn from(value: Errno) -> Self {
        Self::Errno(value)
    }
}
impl From<GuestError> for Failure {
    fn from(value: GuestError) -> Self {
        Self::Guest(value)
    }
}

/// Checked backing errors; adapters must not silently ignore unsupported maps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuestError {
    Unmapped,
    ReadDenied,
    WriteDenied,
    AddressOverflow,
    UnsupportedMapping,
    Allocation,
    BudgetExceeded,
    HostFailure,
}

/// One checked guest write in a single all-or-nothing transaction.
#[derive(Clone, Copy, Debug)]
pub struct GuestWrite<'a> {
    pub address: u64,
    pub bytes: &'a [u8],
}

/// Logical guest mapping effects. An executable permission is only logical;
/// this crate never makes memory executable on the host. Protection carries only
/// decoded CPU/GPU access bits; full range-query metadata stays in kernel state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryEffect {
    None,
    AllocateDirect {
        physical: u64,
        length: u64,
        memory_type: i32,
    },
    ReleaseDirect {
        physical: u64,
        length: u64,
    },
    MapDirect {
        address: u64,
        length: u64,
        protection: u32,
        physical: u64,
    },
    MapZeroed {
        address: u64,
        length: u64,
        protection: u32,
    },
    Unmap {
        address: u64,
        length: u64,
    },
    Protect {
        address: u64,
        length: u64,
        protection: u32,
    },
}

/// Exclusive access to the caller's actual retained guest backing.
///
/// `read` must validate the whole range before reading. `apply` must validate
/// every output and mapping effect before mutation, reject output overlaps,
/// and either commit everything or leave memory unchanged, except for the
/// explicitly fatal backing-owner failure described below. Fresh physical and
/// flexible backing must be zero-initialized; direct maps must preserve and
/// alias that exact physical backing. Every host view stays NX. Unsupported
/// mapping must return that error.
/// The adapter cannot execute, race native guest access, or use detached bytes.
/// A recoverable error leaves all effects and outputs unchanged. If an OS
/// rollback itself fails or irreversible decommit only partially succeeds, an
/// adapter may instead return `HostFailure` after
/// irreversibly poisoning its backing owner: all subsequent memory access and
/// execution capabilities must be refused. This is a fatal host failure, never
/// a retryable guest errno or a successful service result.
pub trait GuestAccess {
    /// Initialize the admitted host socket backend without opening a socket.
    /// Unsupported adapters must refuse rather than claim host initialization.
    fn initialize_network_backend(&mut self) -> Result<bool, Failure> {
        Err(Failure::Unsupported(
            "native network backend initialization",
        ))
    }
    /// Network and POSIX errno require the same thread/backing/NX proof, but
    /// retain independent cells and never share their guest-visible values.
    fn preflight_network_errno(&self, address: Option<u64>) -> Result<(), Failure> {
        self.preflight_posix_errno(address)
    }
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError>;
    /// Validate writable backing without reading, modifying or retaining bytes.
    /// Callback owners use this to admit predictable output/scratch footprints
    /// before entering actual guest code.
    fn preflight_write(&self, _address: u64, _length: u64) -> Result<(), GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
    /// Prove existing same-owner readable, writable, non-executable backing.
    /// Metadata only: no payload read, allocation, mapping or resident commit.
    /// Callers must precharge the complete bounded range query.
    fn preflight_heap_arena(&self, _address: u64, _size: u64) -> Result<(), GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
    /// Prove that at most 32 guest bytes have distinct actual backing, without
    /// reading payload or publishing effects. This admits source-ordered field
    /// reads after disjoint staged stores within one small structure. Callers
    /// must precharge the bounded metadata query; unbound adapters refuse.
    fn preflight_distinct_backing(&self, _address: u64, _length: u64) -> Result<(), GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
    /// Read one byte after a staged u64 store, including physical aliases.
    /// Implementations must model the overlay or explicitly refuse ambiguous
    /// backing aliases. No backing mutation is permitted by this query.
    fn read_after_u64_write_byte(
        &self,
        _address: u64,
        _destination: u64,
        _value: u64,
    ) -> Result<u8, GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
    /// Read at most128 bytes after at most16 ordered staged writes, with at
    /// most1MiB of write payload. Exact virtual overlap observes the latest
    /// store; distinct physical aliases must be modeled or explicitly refused.
    /// This query changes no backing. Callers precharge the bounded metadata
    /// work and payload read through Budget::read_staged.
    fn read_after_writes(
        &self,
        _address: u64,
        _output: &mut [u8],
        _writes: &[GuestWrite<'_>],
    ) -> Result<(), GuestError> {
        Err(GuestError::UnsupportedMapping)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError>;
    /// Conservative visits for checking already loaded image pages. This is
    /// metadata admission, not permission to map or allocate backing.
    fn image_memory_work(&self, _length: u64) -> Result<u64, Failure> {
        Ok(1)
    }
    /// Require the entire page-aligned range to be committed, same-owner
    /// storage from explicitly admitted input images. Generated providers,
    /// runtime objects, reservations and arbitrary host storage are excluded.
    fn preflight_image_memory(&self, _address: u64, _length: u64) -> Result<bool, Failure> {
        Ok(false)
    }
    /// One native TSC sample and its stable ticks-per-second domain. Unbound
    /// adapters must refuse; process epochs are owned separately by each kernel.
    fn time_counter(&self) -> Result<TimeCounterSample, Failure> {
        Err(Failure::Unsupported("native TSC clock domain"))
    }
    /// Admit the actual, exclusively paused initial guest thread before any
    /// pthread object access. This is neither host TLS nor a running new thread.
    /// Unbound clients must refuse without reading or writing guest payload.
    fn preflight_pthread(&self) -> Result<(), Failure> {
        Err(Failure::Unsupported("bound initial-thread pthread context"))
    }
    /// Actual prepared initial-thread stack, with live owned RW/NX backing and
    /// retained unmapped guards. Metadata only; never host stack discovery.
    fn initial_thread_stack(&self) -> Result<InitialThreadStack, Failure> {
        Err(Failure::Unsupported("bound initial-thread stack"))
    }
    /// Admit an actual initial-thread errno integration before any POSIX I/O.
    /// None checks availability; Some checks the complete live four-byte cell
    /// for read/write and non-executable backing on this same exclusive owner.
    /// No payload read or write occurs. Unbound clients must refuse.
    fn preflight_posix_errno(&self, _address: Option<u64>) -> Result<(), Failure> {
        Err(Failure::Unsupported("bound initial-thread POSIX errno"))
    }
    /// Query the actual retained primary executable's process-parameter block.
    /// Only an image-bound owner may implement this: derive its guest address
    /// from that source's metadata and validate the full live readable extent.
    /// Zero denotes a known absent header, never unavailable image integration.
    /// No host pointer, detached copy or guessed parameter structure is valid.
    /// The owner separately bounds metadata/page admission work. This query
    /// reads no payload bytes and performs no guest write or execution.
    fn process_parameters(&self) -> Result<u64, Failure> {
        Err(Failure::Unsupported("image-bound process parameters"))
    }
    /// Read one precharged two-u64 TLS descriptor against the actual admitted
    /// current thread. Its first word is a private runtime module identity,
    /// never a host Program pointer or an inferred firmware DTV entry.
    fn tls_address(&mut self, _descriptor: u64) -> Result<u64, Failure> {
        Err(Failure::Unsupported("image-bound loader/TLS integration"))
    }
    /// Admit image/thread integration before charging or reading a descriptor.
    /// An unbound backing refuses without claiming guest bytes were accessed.
    fn preflight_tls(&self) -> Result<(), Failure> {
        Err(Failure::Unsupported("image-bound loader/TLS integration"))
    }
    /// Look up an export within one actual loaded guest image. Runtime handles
    /// are process-owned identities, never SCE selectors or host module handles.
    /// Known absence is None; unavailable binding remains Unsupported. A bound
    /// owner must charge cumulative lookup work and validate the live target.
    /// This query neither writes guest output nor executes/initializes a module.
    fn module_symbol(&mut self, _handle: i32, _name: &str) -> Result<Option<u64>, Failure> {
        Err(Failure::Unsupported("image-bound module symbols"))
    }
    /// Return metadata for a currently mapped, owned guest module containing
    /// this address. None is known absence; no host-address boundary is valid.
    /// The adapter precharges its metadata/page work and verifies live extents.
    fn module_unwind_info(&mut self, _address: u64) -> Result<Option<ModuleUnwindInfo>, Failure> {
        Err(Failure::Unsupported("image-bound module unwind metadata"))
    }
    /// Retain one explicitly file-bound, already initialized module. The owner
    /// must validate the actual binding/lifecycle, write optional int32 zero and
    /// increment its real load count atomically. A refusal changes neither.
    /// This hook does not admit a new file or execute an initializer.
    fn retain_loaded_module(&mut self, _path: &str, _result: Option<u64>) -> Result<i32, Failure> {
        Err(Failure::Unsupported("initialized module file binding"))
    }
    /// Atomically change byte-granular labels on actual retained virtual
    /// mappings, including image/stack pages and permissionless mappings.
    /// Holes are ignored and endpoints saturate as in the source profile.
    /// This must retain real metadata, not change permissions or payload, and
    /// must not resurrect a label after an unmap/remap at the same address.
    fn set_virtual_range_name(
        &mut self,
        _address: u64,
        _length: u64,
        _name: &[u8],
    ) -> Result<(), Failure> {
        Err(Failure::Unsupported("bound virtual range names"))
    }
    /// Refine one known base mapping's query extent at semantic name changes.
    /// The default name belongs to its initial mapping; no name change is
    /// supported by unbound clients, so their original bounds remain valid.
    fn virtual_name_bounds(
        &mut self,
        _address: u64,
        start: u64,
        end: u64,
        _default: &[u8],
    ) -> Result<(u64, u64), Failure> {
        Ok((start, end))
    }
}

/// Initial runtime identity. It names owned emulated state, not a started host
/// or guest thread. Additional guest-thread execution remains unsupported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub process_id: u32,
    pub initial_thread_id: u32,
}

/// Observed guest stack metadata, not mapping or execution authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InitialThreadStack {
    pub address: u64,
    pub size: u64,
    pub guard_size: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeCounterSample {
    pub ticks: u64,
    pub frequency: u64,
}

/// Process-owned lifecycle callback addresses, retained as data only. A runtime
/// must independently validate live guest code and lifecycle before invocation.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct ThreadLifecycleCallbacks {
    /// Source signature: void callback(void).
    pub destructor: Option<u64>,
    /// Source signature: int callback(int32 module_handle).
    pub atexit_count: Option<u64>,
    /// Source signature: void callback(int32 module_handle).
    pub atexit_report: Option<u64>,
}

/// One bounded service owner. Nothing is process-global or registered by input.
pub struct Kernel {
    budget: budget::Budget,
    files: filesystem::FileState,
    output: output::OutputState,
    system: SystemConfig,
    system_events: system_events::SystemEvents,
    event_queues: event_queue::EventQueues,
    user: user::UserState,
    pad: pad::PadState,
    np: np::NpState,
    entitlements: entitlement::LocalEntitlements,
    acm: acm::AcmState,
    audio_out2: audio_out2::AudioState,
    fonts: font::FontState,
    ime: ime::ImeState,
    content: content_init::ContentState,
    playgo: playgo::PlayGoState,
    save: save::SaveState,
    ampr: ampr::AmprState,
    net_ctl: net_ctl::NetCtlState,
    network: network::NetworkState,
    http2: http2::Http2State,
    np_webapi2: np_webapi2::NpWebApi2State,
    rudp: rudp::RudpState,
    sysmodules: sysmodule::ModuleState,
    memory: memory::MemoryState,
    mspaces: mspace::MspaceState,
    trophies: trophy2::TrophyState,
    uds: uds::UdsState,
    sync: sync::SyncState,
    pthread_mutex: pthread_mutex::MutexState,
    fibers: fiber::FiberState,
    dialogs: dialog::DialogState,
    pthread_condition: pthread_condition::ConditionState,
    pthread_rwlock: pthread_rwlock::RwlockState,
    pthread_semaphores: pthread_semaphore::SemaphoreState,
    pthread_attr: pthread_attr::AttributeState,
    pthread_keys: pthread_keys::KeyState,
    time: time::TimeState,
    identity: ProcessIdentity,
    threads: thread_context::ThreadContexts,
    thread_lifecycle: ThreadLifecycleCallbacks,
    application_heap: Option<ApplicationHeapCallbacks>,
    gpo: u32,
}

impl Kernel {
    /// Supply a real resolved input change. The fixed 64-change queue retains
    /// the newest events. This discovers no host controller or hardware status.
    pub fn publish_pad_snapshot(&mut self, snapshot: PadInputSnapshot) -> Result<(), Failure> {
        self.pad.publish(snapshot, &mut self.budget)
    }

    /// Configure static characteristics before the first PadInit.
    pub fn configure_pad_information(
        &mut self,
        information: PadControllerInformation,
    ) -> Result<(), Failure> {
        self.pad
            .configure_information(information, &mut self.budget)
    }

    /// Desired output; acceptance does not acknowledge hardware application.
    #[must_use]
    pub const fn pad_output(&self) -> PadOutputState {
        self.pad.output()
    }

    /// Microseconds from the process-owned device monotonic origin.
    pub fn pad_monotonic_microseconds(&mut self) -> Result<u64, Failure> {
        self.budget.work(1)?;
        self.time.device_microseconds()
    }

    pub fn new(
        limits: Limits,
        identity: ProcessIdentity,
        roots: Vec<ReadOnlyRoot>,
    ) -> Result<Self, Failure> {
        if identity.process_id == 0
            || identity.initial_thread_id == 0
            || identity.process_id > i32::MAX as u32
            || identity.initial_thread_id > i32::MAX as u32
        {
            return Err(Errno::Invalid.into());
        }
        let mut budget = budget::Budget::new(limits)?;
        Ok(Self {
            files: filesystem::FileState::new(limits, roots, &mut budget)?,
            output: output::OutputState::default(),
            system: SystemConfig::default(),
            system_events: system_events::SystemEvents::new(limits)?,
            event_queues: event_queue::EventQueues::new(limits)?,
            user: user::UserState::default(),
            pad: pad::PadState::default(),
            np: np::NpState::new(limits)?,
            entitlements: entitlement::LocalEntitlements::default(),
            acm: acm::AcmState::default(),
            audio_out2: audio_out2::AudioState::default(),
            fonts: font::FontState::default(),
            ime: ime::ImeState::default(),
            content: content_init::ContentState::default(),
            playgo: playgo::PlayGoState::default(),
            save: save::SaveState::new()?,
            ampr: ampr::AmprState::default(),
            net_ctl: net_ctl::NetCtlState::default(),
            network: network::NetworkState::default(),
            http2: http2::Http2State::default(),
            np_webapi2: np_webapi2::NpWebApi2State::default(),
            rudp: rudp::RudpState::default(),
            sysmodules: sysmodule::ModuleState::new(limits)?,
            memory: memory::MemoryState::new(limits)?,
            mspaces: mspace::MspaceState::default(),
            trophies: trophy2::TrophyState::default(),
            uds: uds::UdsState::default(),
            sync: sync::SyncState::new(limits)?,
            pthread_mutex: pthread_mutex::MutexState::new(limits)?,
            fibers: fiber::FiberState::new(limits)?,
            dialogs: dialog::DialogState::default(),
            pthread_condition: pthread_condition::ConditionState::new(limits)?,
            pthread_rwlock: pthread_rwlock::RwlockState::new(limits)?,
            pthread_semaphores: pthread_semaphore::SemaphoreState::new(limits)?,
            pthread_attr: pthread_attr::AttributeState::new(limits)?,
            pthread_keys: pthread_keys::KeyState::new()?,
            time: time::TimeState::new(),
            budget,
            identity,
            threads: thread_context::ThreadContexts::new(limits, identity.initial_thread_id)?,
            thread_lifecycle: ThreadLifecycleCallbacks::default(),
            application_heap: None,
            gpo: 0,
        })
    }

    #[must_use]
    pub const fn usage(&self) -> Usage {
        self.budget.usage()
    }

    /// Observe this kernel process's actual native counter relative to the same
    /// lazy epoch used by ProcessTimeCounter and ProcessTime. Internal owners
    /// must admit their operation before calling; this adds no synthetic HLE
    /// attempt, guest I/O or independent clock domain. First use samples twice,
    /// exactly as the guest counter service does, and propagates adapter errors.
    pub fn process_time_counter(&mut self, guest: &impl GuestAccess) -> Result<u64, Failure> {
        self.time.process_time_counter(guest)
    }

    /// Observe paired process ticks and microseconds from one validated sample
    /// in the guest services' existing epoch. Trusted callers pre-admit their
    /// operation; no HLE attempt, guest I/O or fresh work quota is consumed here.
    /// Internal display consumer of the existing process counter authority.
    pub fn display_clock(&mut self, guest: &impl GuestAccess) -> Result<DisplayClock, Failure> {
        self.time.display_clock(guest)
    }

    pub fn process_clock(&mut self, guest: &impl GuestAccess) -> Result<ProcessClock, Failure> {
        self.time.process_clock(guest)
    }

    /// Provenance of a memory-map refusal in the latest invocation; no guest data.
    pub const fn memory_map_refusal(&self) -> Option<MemoryMapRefusal> {
        self.memory.last_map_refusal
    }

    /// Publish one initialized RW/NX runtime-owned blob into this kernel's
    /// actual guest adapter. The trusted Rust initializer receives only the
    /// selected guest address and bounded staging bytes, never a host pointer.
    /// Backing, retained metadata and mapped-byte/handle accounting commit
    /// together. Refused attempts still consume work/write budgets. This does
    /// not admit a new guest service attempt.
    pub fn allocate_runtime_data(
        &mut self,
        size: usize,
        guest: &mut impl GuestAccess,
        initialize: impl FnOnce(u64, &mut [u8]) -> Result<(), Failure>,
    ) -> Result<u64, Failure> {
        self.publish_runtime_data(size, 0, guest, |address, bytes| {
            initialize(address, bytes)?;
            Ok(RuntimeDataOutputs::new())
        })
    }

    /// Prepare one new RW/NX runtime blob and up to three copied external
    /// outputs, then publish all four possible writes in ONE mapping transaction.
    /// `held_scratch` is the caller's simultaneous retained staging capacity;
    /// it is added to the allocator-reported blob-buffer capacity before use.
    /// The preparation closure may fill staging and return bounded outputs but
    /// must defer its own ownership changes until this method succeeds.
    /// Recoverable failures publish no backing, outputs or mapping accounting;
    /// charged work/write budgets remain consumed. No service attempt is added.
    pub fn publish_runtime_data(
        &mut self,
        size: usize,
        held_scratch: usize,
        guest: &mut impl GuestAccess,
        prepare: impl FnOnce(u64, &mut [u8]) -> Result<RuntimeDataOutputs, Failure>,
    ) -> Result<u64, Failure> {
        self.memory
            .runtime_data(size, held_scratch, guest, &mut self.budget, prepare)
    }

    /// Registered process-wide lifecycle callbacks as guest address data.
    /// This is not execution authority: a future thread-exit or module-unload
    /// owner must check live initialized executable source before invocation. Registration
    /// neither calls the callback nor claims that thread cleanup has run.
    #[must_use]
    pub const fn thread_lifecycle_callbacks(&self) -> ThreadLifecycleCallbacks {
        self.thread_lifecycle
    }

    /// Admit one load request for an action-based image owner, charging the same
    /// retained kernel budget as invoke. This performs no output or lifecycle
    /// change and does not authorize a guessed return value or new host lookup.
    pub fn admit_module_load(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<AdmittedModuleLoad, Failure> {
        self.budget.attempt()?;
        self.module_load_request(args, guest)
    }

    fn module_load_request(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
    ) -> Result<AdmittedModuleLoad, Failure> {
        let [path, _, _, flags, opt, result] = args.0;
        if flags as u32 != 0 || opt != 0 {
            return Err(Failure::Unsupported("module load flags/options"));
        }
        let path = self.budget.name(guest, path)?;
        let result = (result != 0).then_some(result);
        if let Some(address) = result {
            self.budget.reserve_write(address, 4)?;
        }
        Ok(AdmittedModuleLoad {
            path,
            raw: args,
            result,
            sysmodule: None,
        })
    }

    /// Admit a source-listed game module without publishing loaded state or
    /// output. Its static path still grants no host-file or execution authority.
    pub fn admit_sysmodule_load(
        &mut self,
        service: Service,
        args: CallArgs,
    ) -> Result<AdmittedModuleLoad, Failure> {
        self.budget.attempt()?;
        let output = sysmodule::load_output(service, args)?;
        let public_path = game_module_path(args.0[0] as u16)
            .ok_or(Failure::Unsupported("system game module identity"))?;
        if public_path.len() >= self.budget.limits.max_name_bytes {
            return Err(Failure::Limit("system game module path"));
        }
        self.budget.work(public_path.len() as u64 + 16)?;
        if let Some(address) = output {
            self.budget.reserve_write(address, 4)?;
        }
        let mut path = String::new();
        path.try_reserve_exact(public_path.len())
            .map_err(|_| Failure::Allocation)?;
        path.push_str(public_path);
        Ok(AdmittedModuleLoad {
            path,
            raw: args,
            result: None,
            sysmodule: Some(SysmoduleLoadCompletion { service, args }),
        })
    }

    /// Publish the source reference/output after the native owner attests a real
    /// initializer return, or verifies that this exact file is already mapped.
    /// Admission charged the attempt; this commit does not charge it twice.
    pub fn complete_sysmodule_load(
        &mut self,
        completion: SysmoduleLoadCompletion,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.sysmodules
            .complete_load(completion.service, completion.args, guest, &mut self.budget)
    }

    pub fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.invoke_with_stack(service, args, None, guest)
    }

    /// Invoke with an optional captured callee RSP. Only an exact registration
    /// needing stack arguments reads it; ordinary six-register calls ignore it.
    /// RSP is guest request data, never a host pointer or execution capability.
    pub fn invoke_with_stack(
        &mut self,
        service: Service,
        args: CallArgs,
        stack_pointer: Option<u64>,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.invoke_with_vectors(service, args, stack_pointer, None, guest)
    }

    /// Captured vector registers belong to the same original library call as
    /// the integer arguments and callee RSP. Services read them only when their
    /// exact ABI requires floating arguments; no host floating state is sampled.
    pub fn invoke_with_vectors(
        &mut self,
        service: Service,
        args: CallArgs,
        stack_pointer: Option<u64>,
        vector_args: Option<[[u8; 16]; 8]>,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.memory.last_map_refusal = None;
        self.budget.attempt()?;
        use Service::*;
        match service {
            AudioOut2Initialize
            | AudioOut2ContextResetParam
            | AudioOut2ContextQueryMemory
            | AudioOut2GetSpeakerInfo => audio_out2::invoke(service, args, guest, &mut self.budget),
            AudioOut2ContextCreate
            | AudioOut2ContextDestroy
            | AudioOut2ContextAdvance
            | AudioOut2ContextPush
            | AudioOut2ContextGetQueueLevel
            | AudioOut2UserCreate
            | AudioOut2UserDestroy
            | AudioOut2PortCreate
            | AudioOut2PortDestroy
            | AudioOut2PortGetState
            | AudioOut2PortSetAttributes => self.invoke_audio_out2(service, args, guest),
            AcmContextCreate => self.acm_context_create(args, guest),
            SystemHideSplashScreen
            | SystemParamGetString
            | SystemGetNoticeScreenSkipFlag
            | SystemDisableNoticeScreenSkipFlagAutoSet
            | SystemSetNoticeScreenSkipFlag
            | SystemPowerTick
            | SystemReportAbnormalTermination => self.invoke_system_notice(service, args, guest),
            PthreadGetprio
            | PthreadSetprio
            | PosixSchedGetPriorityMax
            | PosixSchedGetPriorityMin
            | PosixThreadSetprio
            | PosixThreadGetSchedParam
            | PosixThreadSetSchedParam => self.invoke_thread_priority(service, args, guest),
            ContentExportInit2 | ContentSearchInit | ContentDeleteInitialize => {
                self.content.invoke(service, args, &mut self.budget)
            }
            // Pinned libNet.cpp LibShare has no retained state, input reads or
            // host sharing operation. Its parameters are logging-only values.
            ShareInitialize | ShareTerminate => {
                self.budget.work(1)?;
                Ok(0)
            }
            ShareFeaturePermit
            | ShareCaptureVideoClip
            | ShareFeatureProhibit
            | ShareSetContentParam
            | ShareOpenMenuForContent
            | ShareCaptureVideoClipExtended
            | ShareCaptureScreenshot
            | ShareCaptureScreenshotExtended
            | ShareUnregisterContentEventCallback
            | ShareSetContentParamForApplicationTitle
            | ShareGetCurrentStatus
            | ShareRegisterContentEventCallback
            | ShareSetScreenshotOverlayImage
            | ShareGetRunningStatus
            | ShareSetCaptureSource => share_media::invoke(service, args, guest, &mut self.budget),
            PlayGoInitialize
            | PlayGoTerminate
            | PlayGoOpen
            | PlayGoClose
            | PlayGoGetLocus
            | PlayGoGetInstallSpeed
            | PlayGoSetInstallSpeed
            | PlayGoGetChunkId
            | PlayGoGetToDoList
            | PlayGoSetToDoList
            | PlayGoPrefetch
            | PlayGoGetEta
            | PlayGoGetLanguageMask
            | PlayGoGetProgress
            | PlayGoGetOptionalChunk
            | PlayGoPrefetchOptionalChunk
            | PlayGoGetInstallChunkId
            | PlayGoGetSupportedOptionalChunk => {
                self.playgo
                    .invoke(service, args, guest, &self.files, &mut self.budget)
            }
            MouseInit | MouseOpen | MouseClose | MouseRead | KeyboardInit | KeyboardOpen
            | KeyboardClose | KeyboardReadState | KeyboardRead | KeyboardGetKey2Char => {
                raw_input::invoke(service, args, guest, &mut self.budget)
            }
            ImeParamInit
            | ImeGetPanelSize
            | ImeOpen
            | ImeUpdate
            | ImeClose
            | ImeSetText
            | ImeSetCaret
            | ImeSetTextGeometry
            | ImeKeyboardOpen
            | ImeKeyboardClose
            | ImeKeyboardGetResourceId
            | ImeKeyboardGetInfo
            | ImeKeyboardSetMode => self.ime.invoke(service, args, guest, &mut self.budget),
            FontAttachDeviceCacheBuffer
            | FontCloseFont
            | FontCreateRendererWithEdition
            | FontDestroyRenderer
            | FontBindRenderer
            | FontRebindRenderer
            | FontUnbindRenderer
            | FontSetScalePixel
            | FontSetEffectSlant
            | FontSetEffectWeight
            | FontSetupRenderScalePixel
            | FontSetupRenderEffectSlant
            | FontSetupRenderEffectWeight
            | FontGetHorizontalLayout
            | FontGetVerticalLayout
            | FontDefineAttribute
            | FontCreateLibrary
            | FontCreateLibraryWithEdition
            | FontCreateString
            | FontCreateWritingLine
            | FontDestroyString
            | FontDestroyWritingLine
            | FontDeleteGlyph
            | FontDestroyLibrary
            | FontGenerateCharGlyph
            | FontGetCharGlyphMetrics
            | FontGetRenderCharGlyphMetrics
            | FontGlyphDefineAttribute
            | FontMemoryInit
            | FontMemoryTerm
            | FontOpenFontInstance
            | FontOpenFontMemory
            | FontOpenFontSet
            | FontRenderCharGlyphImageHorizontal
            | FontRenderSurfaceInit
            | FontRenderSurfaceSetScissor
            | FontStringGetTerminateCode
            | FontStringGetTerminateOrder
            | FontStringGetWritingForm
            | FontStringRefersRenderCharacters
            | FontStringRefersTextCharacters
            | FontTextSourceInit
            | FontTextSourceRewind
            | FontTextSourceSetDefaultFont
            | FontTextSourceSetWritingForm
            | FontCharacterGetBidiLevel
            | FontCharacterGetSyllableStringState
            | FontCharacterGetTextFontCode
            | FontCharacterGetTextOrder
            | FontCharacterLooksFormatCharacters
            | FontCharacterLooksWhiteSpace
            | FontCharacterRefersTextBack
            | FontCharacterRefersTextNext
            | FontCharactersRefersTextCodes
            | FontTextCodesStepBack
            | FontTextCodesStepNext
            | FontWritingGetRenderMetrics
            | FontWritingInit
            | FontWritingLineClear
            | FontWritingLineGetOrderingSpace
            | FontWritingLineGetRenderMetrics
            | FontWritingLineRefersRenderStep
            | FontWritingLineWritesOrder
            | FontWritingRefersRenderStep
            | FontWritingRefersRenderStepCharacter
            | FontWritingSetMaskInvisible
            | FontSupportExternalFonts
            | FontSupportSystemFonts
            | FontSelectLibraryFt
            | FontSelectRendererFt => self.fonts.invoke(
                font::FontCall {
                    service,
                    args,
                    stack_pointer,
                    vector_args,
                },
                &mut self.memory,
                guest,
                &mut self.budget,
            ),
            EntitlementAccessInitialize
            | EntitlementAccessGetSkuFlag
            | EntitlementAccessGetAddcontEntitlementInfoList
            | EntitlementAccessGetAddcontEntitlementInfo
            | EntitlementAccessGetEntitlementKey => {
                self.entitlements
                    .invoke(service, args, guest, &mut self.budget)
            }
            UdsInitialize
            | UdsCreateContext
            | UdsCreateHandle
            | UdsCreateEvent
            | UdsPostEvent
            | UdsDestroyHandle
            | UdsAbortHandle
            | UdsDestroyContext
            | UdsGetMemoryStat
            | UdsEventEstimateSize
            | UdsEventToString
            | UdsDestroyEvent
            | UdsRegisterContext
            | UdsCreateEventPropertyObject
            | UdsDestroyEventPropertyObject
            | UdsEventPropertyObjectSetString
            | UdsEventPropertyObjectSetInt32
            | UdsEventPropertyObjectSetUInt32
            | UdsEventPropertyObjectSetInt64
            | UdsEventPropertyObjectSetUInt64
            | UdsEventPropertyObjectSetFloat32
            | UdsEventPropertyObjectSetFloat64
            | UdsEventPropertyObjectSetBool
            | UdsEventPropertyObjectSetBinary
            | UdsEventPropertyObjectSetObject
            | UdsEventPropertyObjectSetArray
            | UdsCreateEventPropertyArray
            | UdsDestroyEventPropertyArray
            | UdsEventPropertyArraySetString
            | UdsEventPropertyArraySetInt32
            | UdsEventPropertyArraySetUInt32
            | UdsEventPropertyArraySetInt64
            | UdsEventPropertyArraySetUInt64
            | UdsEventPropertyArraySetFloat32
            | UdsEventPropertyArraySetFloat64
            | UdsEventPropertyArraySetBool
            | UdsEventPropertyArraySetBinary
            | UdsEventPropertyArraySetObject
            | UdsEventPropertyArraySetArray
            | UdsGetStorageStat
            | UdsTerminate => self.uds.invoke(
                service,
                args,
                uds::UdsResources {
                    memory: &mut self.memory,
                    trophies: &mut self.trophies,
                    files: &self.files,
                    storage: self.save.trophy_storage(),
                    language: self.system.language_id,
                },
                guest,
                &mut self.budget,
            ),
            Trophy2CreateContext
            | Trophy2CreateHandle
            | Trophy2RegisterContext
            | Trophy2GetGameInfo
            | Trophy2GetGroupInfo
            | Trophy2GetGroupInfoArray
            | Trophy2GetTrophyInfo
            | Trophy2GetTrophyInfoArray
            | Trophy2GetGameIcon
            | Trophy2GetGroupIcon
            | Trophy2GetTrophyIcon
            | Trophy2RegisterUnlockCallback
            | Trophy2AbortHandle
            | Trophy2DestroyHandle
            | Trophy2DestroyContext => self.trophies.invoke(
                service,
                args,
                stack_pointer,
                &self.files,
                self.save.trophy_storage(),
                self.system.language_id,
                self.uds.metadata_bytes(&mut self.budget)?,
                guest,
                &mut self.budget,
            ),
            GameIntentInitialize
            | GameIntentTerminate
            | GameIntentReceiveIntent
            | GameIntentGetPropertyValueString => {
                game_intent::invoke(service, args, guest, &mut self.budget)
            }
            MspaceCreate
            | MspaceDestroy
            | MspaceMalloc
            | MspaceFree
            | MspaceCalloc
            | MspaceRealloc
            | MspacePosixMemalign
            | MspaceMemalign
            | MspaceAlignedAlloc
            | MspaceReallocalign
            | MspaceMallocStats
            | MspaceMallocStatsFast
            | MspaceMallocUsableSize
            | PafMspaceCreate
            | PafMspaceDestroy
            | PafMspaceMalloc
            | PafMspaceMemalign
            | PafMspaceFree
            | PafMspaceCalloc
            | PafMspaceRealloc
            | PafMspaceReallocalign
            | PafMspaceMallocStats
            | PafMspaceMallocStatsFast => {
                match self
                    .mspaces
                    .invoke(service, args, guest, &mut self.budget)?
                {
                    mspace::MspaceResult::Value(value) => Ok(value),
                    mspace::MspaceResult::Errno { value, number } => {
                        self.publish_mspace_errno(number, guest)?;
                        Ok(value)
                    }
                }
            }
            CreateEqueue | DeleteEqueue | WaitEqueue | GetEventUserData | GetEventId
            | GetEventFilter | GetEventData | GetEventFflags | GetEventError | AddUserEvent
            | AddUserEventEdge | TriggerUserEvent | DeleteUserEvent | AddTimerEvent
            | AddHRTimerEvent | DeleteHRTimerEvent => {
                self.event_queues
                    .invoke(service, args, guest, &mut self.budget)
            }
            PadInit => {
                self.budget.work(1)?;
                if !self.pad.initialized() {
                    let timestamp = self.time.device_microseconds()?;
                    self.pad.initialize(timestamp);
                }
                Ok(0)
            }
            PadClose
            | PadOpen
            | PadGetHandle
            | PadIsRemote
            | PadSetTiltCorrectionState
            | PadGetTriggerEffectState
            | PadSetVibration
            | PadRead
            | PadGetControllerInformation
            | PadSetTriggerEffect
            | PadSetLightBar => {
                self.budget.work(1)?;
                let user = if matches!(service, PadOpen | PadGetHandle) {
                    self.user.observed_id()
                } else {
                    0
                };
                self.pad
                    .invoke(service, args, user, guest, &mut self.budget)
            }
            AmprConstruct | AmprAprConstruct | AmprAprDestruct | AmprDestruct | AmprSetBuffer
            | AmprReset | AmprClearBuffer | AmprGetType | AmprGetOffset | AmprGetNum
            | AmprGetSize | AmprGetBase => self.ampr.invoke(service, args, guest, &mut self.budget),
            AmprReadFile => {
                let stack =
                    stack_pointer.ok_or(Failure::Unsupported("missing library stack arguments"))?;
                let slot = stack.checked_add(8).ok_or(GuestError::AddressOverflow)?;
                let offset = self.budget.u64(guest, slot)?;
                self.ampr.append_read(args, offset, guest, &mut self.budget)
            }
            AmprMeasureReadFile => ampr::AmprState::measure_read(args),
            CommonDialogInitialize | CommonDialogIsUsed => self.dialogs.invoke(service),
            SaveTransferringMount
            | SaveMount3
            | SaveUmount2
            | SaveGetMountInfo
            | SaveDirNameSearch
            | SaveDelete
            | SaveGetParam
            | SaveSetParam
            | SaveLoadIcon
            | SaveSaveIcon
            | SaveSaveIconByPath
            | SaveBackup => {
                self.save
                    .invoke_directory(service, args, guest, &mut self.budget, &mut self.files)
            }
            SaveInitialize
            | SaveTerminate
            | SaveCreateTransaction
            | SaveDeleteTransaction
            | SavePrepare
            | SaveCommit
            | SaveGetEvent
            | SaveMemorySetup
            | SaveMemoryGet
            | SaveMemorySet
            | SaveMemorySync => self.save.invoke(service, args, guest, &mut self.budget),
            NpCheckCallback
            | NpRegisterStateCallbackA
            | NpGetId
            | NpGetOnlineId
            | NpGetAccountId
            | NpGetAccountCountry
            | NpGetAccountAge
            | NpGetAccountLanguage
            | NpGetState
            | NpGetReachabilityState
            | NpHasSignedUp
            | NpCreateRequest
            | NpCreateAsyncRequest
            | NpDeleteRequest
            | NpAbortRequest
            | NpCheckAvailability
            | NpCheckReachability
            | NpPollAsync
            | NpCheckPremium => self.np.invoke(service, args, guest, &mut self.budget),
            UserGetPlatformPrivacyWs1 => user_privacy::invoke(args, guest, &mut self.budget),
            ConvertKeycodeGetImeKeyboardType => {
                convert_keycode::invoke(args, guest, &mut self.budget)
            }
            UserInitialize
            | UserInitialize2
            | UserGetInitial
            | UserGetEvent
            | UserGetLoginList
            | UserGetName
            | UserGetNumber
            | UserGetGamePresets
            | UserGetAgeLevel
            | UserGetChatTranscription
            | UserGetPressAndHoldDelay
            | UserGetVibration
            | UserGetTriggerEffect
            | UserGetZoomEnabled
            | UserGetZoomFollowFocus => self.user.invoke(service, args, guest, &mut self.budget),
            FiberInitialize
            | FiberInitializeInternal
            | FiberOptParamInitialize
            | FiberFinalize
            | FiberGetInfo
            | FiberRename
            | FiberRun
            | FiberSwitch
            | FiberReturnToThread
            | FiberGetSelf
            | FiberStartContextSizeCheck
            | FiberStopContextSizeCheck
            | FiberGetThreadFramePointerAddress => self.fibers.invoke(
                service,
                self.threads.active().context(),
                args,
                stack_pointer,
                guest,
                &mut self.budget,
            ),
            MapNamedDirect => {
                let stack =
                    stack_pointer.ok_or(Failure::Unsupported("missing library stack arguments"))?;
                let slot = stack.checked_add(8).ok_or(GuestError::AddressOverflow)?;
                let name = self.budget.u64(guest, slot)?;
                self.memory
                    .map_named_direct(args, name, guest, &mut self.budget)
            }
            SetApplicationHeapApi => self.set_application_heap_api(args.0[0], guest),
            SetThreadDtors | SetThreadAtexitCount | SetThreadAtexitReport => {
                // Registration belongs to the process context; it reads no
                // guest thread header and confers no execution authority.
                // The pinned source is void and accepts a null callback only
                // while no nonnull callback has been registered. Replacing an
                // existing registration is an explicit unsupported operation.
                let slot = match service {
                    SetThreadDtors => &mut self.thread_lifecycle.destructor,
                    SetThreadAtexitCount => &mut self.thread_lifecycle.atexit_count,
                    SetThreadAtexitReport => &mut self.thread_lifecycle.atexit_report,
                    _ => unreachable!(),
                };
                if slot.is_some() {
                    return Err(Failure::Unsupported(
                        "thread lifecycle callback replacement",
                    ));
                }
                *slot = (args.0[0] != 0).then_some(args.0[0]);
                Ok(0) // RAX is unspecified by the source's void signature.
            }
            PthreadOnce => self.invoke_pthread_once(args, guest),
            ThreadAttrDestroy
            | ThreadAttrGet
            | ThreadAttrGetAffinity
            | ThreadAttrInit
            | ThreadAttrGetDetachState
            | ThreadAttrGetStack
            | ThreadAttrGetStackAddr
            | ThreadAttrGetStackSize
            | ThreadAttrGetGuardSize
            | ThreadAttrGetSoloSched
            | ThreadAttrSetStack
            | ThreadAttrSetStackAddr
            | ThreadAttrSetStackSize
            | ThreadAttrSetDetachState
            | ThreadAttrSetGuardSize
            | ThreadAttrSetInheritSched
            | ThreadAttrSetSchedParam
            | ThreadAttrSetSchedPolicy
            | ThreadAttrSetSoloSched
            | ThreadAttrSetAffinity
            | ThreadAttrGetSchedParam
            | ThreadAttrGetSchedPolicy
            | PosixThreadAttrInit
            | PosixThreadAttrDestroy
            | PosixThreadAttrGetStack
            | PosixThreadAttrSetStack
            | PosixThreadAttrSetStackSize
            | PosixThreadAttrGet
            | PosixThreadAttrGetSchedPolicy
            | PosixThreadAttrSetSchedPolicy
            | PosixThreadAttrSetDetachState
            | PosixThreadAttrSetSchedParam
            | PosixThreadAttrSetInheritSched
            | PosixThreadAttrGetStackSize
            | PosixThreadAttrGetDetachState
            | PosixThreadAttrSetGuardSize
            | PosixThreadAttrGetGuardSize
            | PosixThreadAttrGetSchedParam => self.invoke_pthread_attr(service, args, guest),
            RwlockInit | RwlockDestroy | RwlockReadLock | RwlockWriteLock | RwlockTryReadLock
            | RwlockTryWriteLock | RwlockTimedReadLock | RwlockTimedWriteLock | RwlockUnlock
            | RwlockAttrInit | RwlockAttrDestroy | RwlockAttrSetType | PosixRwlockInit
            | PosixRwlockDestroy | PosixRwlockWriteLock => {
                self.invoke_pthread_rwlock(service, args, guest)
            }
            CondAttrInit
            | CondAttrDestroy
            | CondAttrSetClock
            | CondInit
            | CondDestroy
            | CondSignal
            | CondBroadcast
            | PosixCondAttrInit
            | PosixCondAttrDestroy
            | PosixCondAttrSetClock
            | PosixCondInit
            | PosixCondSignal
            | PosixCondBroadcast => self.invoke_pthread_condition(service, args, guest),
            KeyCreate | KeyDelete | SetSpecific | GetSpecific | PosixKeyCreate | PosixKeyDelete
            | PosixSetSpecific => self.invoke_pthread_keys(service, args, guest),
            MutexAttrInit
            | MutexAttrDestroy
            | MutexAttrSetType
            | MutexAttrSetProtocol
            | MutexInit
            | MutexDestroy
            | MutexLock
            | MutexTryLock
            | MutexTimedLock
            | MutexUnlock
            | PosixMutexAttrInit
            | PosixMutexAttrDestroy
            | PosixMutexAttrSetType
            | PosixMutexAttrSetProtocol
            | PosixMutexInit
            | PosixMutexDestroy
            | PosixMutexLock
            | PosixMutexTryLock
            | PosixMutexTimedLock
            | PosixMutexUnlock => self.invoke_pthread_mutex(service, args, guest),
            ErrnoLocation => self.errno_address(guest),
            PageSize => Ok(0x4000),
            // The pinned profile reports disabled; this runtime has no guest
            // ASan instrumentation. Host build sanitizers are unrelated.
            IsAddressSanitizerEnabled => Ok(0),
            PosixOpen
            | PosixClose
            | PosixRead
            | PosixPread
            | PosixPwrite
            | PosixFtruncate
            | PosixMkdir
            | PosixLseek
            | PosixFstat
            | PosixStat
            | PosixClockGettime
            | PosixClockGetres
            | PosixNanosleep
            | PosixWrite
            | AprResolveFilepathsToIdsAndFileSizes
            | AprResolveFilepathsToIds
            | AprGetFileStat
            | AprGetFileSize
            | AprResolveFilepathsWithPrefixToIds
            | AprResolveFilepathsWithPrefixToIdsAndFileSizes
            | AprResolveFilepathsToIdsForEach
            | AprResolveFilepathsToIdsAndFileSizesForEach
            | AprResolveFilepathsWithPrefixToIdsForEach
            | AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach
            | AprSubmitCommandBufferAndGetResult
            | AprSubmitCommandBuffer
            | AprSubmitCommandBufferAndGetId
            | AprWaitCommandBuffer => self.invoke_posix(service, args, guest),
            NetCtlInit
            | NetCtlTerm
            | NetCtlGetState
            | NetCtlGetStateV6
            | NetCtlGetNatInfo
            | NetCtlRegisterCallback
            | NetCtlUnregisterCallback
            | NetCtlCheckCallback
            | NetCtlGetResult
            | NetCtlGetInfo => self.net_ctl.invoke(service, args, guest, &mut self.budget),
            RudpInit | RudpEnableInternalIOThread | RudpSetEventHandler => {
                self.rudp.invoke(service, args, &mut self.budget)
            }
            NetInit | NetGetErrorAddr | NetPoolCreate | NetPoolDestroy | NetResolverCreate
            | NetResolverDestroy | NetResolverAbort | SslInit | SslTerm | SslGetCaCerts
            | SslFreeCaCerts | HttpInit | HttpTerm => self.invoke_network(service, args, guest),
            Http2Init
            | Http2Term
            | Http2CreateTemplate
            | Http2DeleteTemplate
            | Http2CreateRequestWithURL
            | Http2DeleteRequest
            | Http2AddRequestHeader
            | Http2SetRequestContentLength
            | Http2SetAuthEnabled
            | Http2SetAutoRedirect
            | Http2SetInflateGZIPEnabled
            | Http2SslDisableOption
            | Http2SslEnableOption
            | Http2SetRedirectCallback
            | Http2SetSslCallback
            | Http2SetRecvTimeOut
            | Http2SetSendTimeOut
            | Http2SetConnectTimeOut
            | Http2SetConnectionWaitTimeOut
            | Http2SetResolveTimeOut
            | Http2SetTimeOut => self.http2.invoke(service, args, guest, &mut self.budget),
            NpWebApi2Initialize
            | NpWebApi2GetMemoryPoolStats
            | NpWebApi2PushEventCreateHandle
            | NpWebApi2CreateUserContext
            | NpWebApi2DeleteUserContext
            | NpWebApi2CreateRequest
            | NpWebApi2SendRequest
            | NpWebApi2ReadData
            | NpWebApi2AbortRequest
            | NpWebApi2DeleteRequest
            | NpWebApi2AddHttpRequestHeader
            | NpWebApi2GetHttpResponseHeaderValueLength
            | NpWebApi2GetHttpResponseHeaderValue
            | NpWebApi2PushEventDeleteHandle
            | NpWebApi2PushEventDeletePushContext
            | NpWebApi2PushEventCreateFilter
            | NpWebApi2PushEventRegisterCallback
            | NpWebApi2CheckTimeout
            | NpWebApi2Terminate => self
                .np_webapi2
                .invoke(service, args, guest, &mut self.budget),
            Write => self.write_output(args, guest, false),
            SystemParamGetInt => self.system_param_get_int(args, guest),
            SystemGetHdrToneMapLuminance | SystemGetDisplaySafeAreaInfo => {
                self.system_display_query(service, args, guest)
            }
            IsTrinityMode | GetOperationMode => self.hardware_mode(service, args, guest),
            AppContentInitialize
            | AppContentAppParamGetInt
            | SystemReceiveEvent
            | SystemGetStatus => self.system_events.invoke(
                service,
                args,
                self.dialogs.status().in_use,
                guest,
                &mut self.budget,
            ),
            PosixSemInit | PosixSemDestroy | PosixSemWait | PosixSemTryWait
            | PosixSemRelativeWait | PosixSemTimedWait | PosixSemPost | PosixSemGetValue
            | PthreadSemInit | PthreadSemDestroy | PthreadSemWait | PthreadSemTryWait
            | PthreadSemTimedWait | PthreadSemPost | PthreadSemGetValue => {
                self.pthread_semaphore(service, args, guest)
            }
            ModuleInfoForUnwind => module_unwind::invoke(args, guest, &mut self.budget),
            SysmoduleLoad | SysmoduleUnload | SysmoduleIsLoaded | SysmoduleLoadInternal => self
                .sysmodules
                .invoke(service, args, guest, &mut self.budget),
            Gettimezone | ConvertUtcToLocaltime | ConvertLocaltimeToUtc => {
                self.calendar(service, args, guest)
            }
            Open | Close | Read | Pread | Lseek | Fstat | Stat | CheckReachability
            | Getdirentries | Getdents | Pwrite | Ftruncate | Fsync | Mkdir | Rmdir | Rename
            | Unlink => self.files.invoke(service, args, guest, &mut self.budget),
            AllocateDirect
            | AllocateMainDirect
            | ReleaseDirect
            | DirectMemorySize
            | ConfiguredFlexibleMemorySize
            | AvailableFlexibleMemorySize
            | MapDirect
            | MapFlexible
            | MapNamedFlexible
            | Munmap
            | Mprotect
            | QueryProtection
            | SetVirtualRangeName
            | ClearVirtualRangeName => self.memory.invoke(service, args, guest, &mut self.budget),
            CreateSema | DeleteSema | WaitSema | PollSema | SignalSema | CancelSema
            | CreateEventFlag | DeleteEventFlag | WaitEventFlag | PollEventFlag | SetEventFlag
            | ClearEventFlag | CancelEventFlag => {
                self.sync.invoke(service, args, guest, &mut self.budget)
            }
            ClockGettime
            | ClockGetres
            | Gettimeofday
            | ProcessTime
            | ProcessTimeCounter
            | ProcessTimeCounterFrequency
            | ReadTsc
            | TscFrequency
            | Usleep
            | Sleep
            | Nanosleep => self.time.invoke(service, args, guest, &mut self.budget),
            Getpid => Ok(u64::from(self.identity.process_id)),
            PthreadSelf => {
                if let Some(address) = self.threads.active().self_pointer {
                    return Ok(address);
                }
                let address = self.memory.initial_thread(
                    self.threads.active().id,
                    guest,
                    &mut self.budget,
                )?;
                self.threads.active_mut().self_pointer = Some(address);
                Ok(address)
            }
            PthreadGetthreadid => Ok(u64::from(self.threads.active().id)),
            PthreadSetaffinity | PthreadGetaffinity => {
                self.threads
                    .invoke_affinity(service, args, guest, &mut self.budget)
            }
            PthreadEqual => Ok(u64::from(args.0[0] == args.0[1])),
            SetGpo => {
                self.gpo = args.0[0] as u32;
                Ok(0)
            }
            GetGpi => Ok(u64::from(self.gpo)),
            PthreadCreate
            | PosixPthreadCreate
            | PosixPthreadCreateName
            | CondWait
            | PosixCondWait
            | PthreadJoin
            | PthreadExit
            | PthreadDetach
            | PosixPthreadDetach => {
                Err(Failure::Unsupported("guest-thread execution and lifecycle"))
            }
            PthreadYield => Err(Failure::Unsupported("cooperative guest-thread yield")),
            GetProcParam => guest.process_parameters(),
            Dlsym => {
                let [handle, name, output, ..] = args.0;
                if output == 0 {
                    return Ok(u64::from(Errno::Fault.kernel_code() as u32));
                }
                let (address, code) = if name == 0 {
                    (0, Errno::Fault.kernel_code() as u32)
                } else {
                    let name = self.budget.name(guest, name)?;
                    match guest.module_symbol(handle as u32 as i32, &name)? {
                        Some(address) => (address, 0),
                        None => (0, Errno::NoSuchResource.kernel_code() as u32),
                    }
                };
                self.budget.write(
                    guest,
                    &[GuestWrite {
                        address: output,
                        bytes: &address.to_le_bytes(),
                    }],
                )?;
                // This ONE service has an observed int32 return domain. The
                // explicit source EFAULT/ESRCH cases write null first; unrelated
                // typed failures are not converted or resumed generically.
                Ok(u64::from(code))
            }
            LoadStartModule => {
                let request = self.module_load_request(args, guest)?;
                // The pinned existing-program branch ignores args/argp and
                // returns its positive process handle. Unknown/new files stay
                // Unsupported in the adapter, rather than fabricated ENOENT.
                guest
                    .retain_loaded_module(request.path(), request.result_pointer())
                    .map(|handle| u64::from(handle as u32))
            }
            TlsGetAddr => {
                guest.preflight_tls()?;
                self.budget.precharge_read(16)?;
                guest.tls_address(args.0[0])
            }
        }
    }
}

#[cfg(test)]
mod tests;
