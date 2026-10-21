// SPDX-License-Identifier: GPL-2.0-only

//! Dedicated-worker composition over the actual prepared kernel/TLS owner.
//! One pause covers one block, and ends before the core service adapter runs.
//! Ordinary native operands are checked from the current observed guest state.

use pentax_codegen::{
    BlockLimits, CodegenError, CpuFeatures, CpuidFeature, MaterializedSource, Mnemonic,
    TranslationPolicy, plan_materialized_block,
};
use pentax_core::process::hle::{
    DynamicModuleInput, DynamicModuleStart, GeneratedUnavailableProvider, KernelStartupError,
    NativeServiceAction, NativeServiceRequest, PendingModuleLoad, PreparedFontParserCallback,
    PreparedImeCallback, PreparedNativeKernelStartup, PreparedNativeModuleStart,
    PreparedNativeSource, PreparedNetCtlCallback, PreparedPthreadOnce, UnavailableProviderError,
    build_unavailable_provider,
};
use pentax_platform::native_dispatch::{
    NativeBlockMachine, NativeBlockStepper, NativeBlockStop, NativeCpuState, NativeDispatchError,
    NativeDispatchLimits, NativeDispatchUsage, NativeNestedCall, NativeStackFrame, PreSyscallToken,
};
use pentax_platform::native_memory::{NativeMemoryError, PausedNativeMemory};
use std::sync::OnceLock;

mod dispatch_context;
// The optional local window runs on this supervised worker thread, outside
// every guest entry and memory pause. Ordinary probes retain no consumer.
pub(crate) mod frame_bridge;
mod plan_cache;
mod scheduling;
mod startup_profile;
mod stop_diagnostics;
use scheduling::Scheduling;
use stop_diagnostics::StopDiagnostics;

use startup_profile::StartupProfile;
use std::time::Instant;

/// Independent bounds for source-window admission and the reusable engine.
/// `block` bounds each pure plan; `dispatch` cumulatively charges its planning
/// usage, publication and execution. The extra window allocation/copy and
/// metadata selection have their own explicit cumulative work/byte bounds.
#[derive(Debug, Clone, Copy)]
pub(super) struct NativeSessionLimits {
    pub block: BlockLimits,
    pub dispatch: NativeDispatchLimits,
    pub max_window_allocation_bytes: u64,
    pub max_total_window_bytes: u64,
    pub max_admission_work: u64,
    pub report_progress: bool,
    pub progress_interval_blocks: u64,
    /// Explicitly selected supplied libc input, never a guessed module index.
    pub libc_memory_builtins: Option<usize>,
    /// Explicit host-only diagnostic output, never enabled by guest input.
    pub private_samples: bool,
    /// Explicit host lifetime policy; per-operation and resident bounds remain.
    pub interactive: bool,
}
impl Default for NativeSessionLimits {
    fn default() -> Self {
        Self {
            block: BlockLimits::default(),
            dispatch: NativeDispatchLimits::default(),
            // Most checked blocks isolate a single memory operation. A small
            // live window avoids copying/attesting 4 KiB for every such step.
            // Partial trailing instructions are retried at their exact start.
            max_window_allocation_bytes: 64,
            max_total_window_bytes: 16 * 1024 * 1024,
            max_admission_work: 64 * 1024 * 1024,
            report_progress: false,
            progress_interval_blocks: 2048,
            libc_memory_builtins: None,
            private_samples: false,
            interactive: false,
        }
    }
}

#[derive(Debug)]
pub(super) enum NativeSessionError {
    Core(KernelStartupError),
    Dispatch(NativeDispatchError),
    Codegen(CodegenError),
    Memory(NativeMemoryError),
    Extension(pentax_linker::SymbolicExtensionError),
    WrongPreparedSource,
    SourceWindow,
    Allocation,
    Budget(&'static str),
    UnsupportedExit {
        guest_pc: u64,
        mnemonic: Option<Mnemonic>,
    },
    OutputUnavailable,
    FixtureInvariant(&'static str),
    ThreadDeadlock,
    Frame(frame_bridge::Error),
}
impl std::fmt::Display for NativeSessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Core(error) => write!(f, "prepared native core: {error}"),
            Self::Dispatch(error) => write!(f, "prepared native dispatch: {error}"),
            Self::Codegen(error) => write!(f, "prepared native codegen: {error}"),
            Self::Memory(error) => write!(f, "prepared native memory: {error}"),
            Self::Extension(error) => write!(f, "prepared module extension: {error}"),
            Self::WrongPreparedSource => f.write_str("prepared native source identity mismatch"),
            Self::SourceWindow => f.write_str("prepared native initialized source unavailable"),
            Self::Allocation => f.write_str("prepared native window allocation failed"),
            Self::Budget(name) => write!(f, "prepared native budget exceeded: {name}"),
            Self::UnsupportedExit { guest_pc, .. } => {
                write!(f, "prepared native unsupported exit at {guest_pc:#x}")
            }
            Self::OutputUnavailable => f.write_str("execution protocol output unavailable"),
            Self::FixtureInvariant(name) => write!(f, "prepared native fixture invariant: {name}"),
            Self::ThreadDeadlock => {
                f.write_str("all guest threads are blocked without a runnable notifier")
            }
            Self::Frame(error) => write!(f, "frame session: {error:?}"),
        }
    }
}
impl std::error::Error for NativeSessionError {}
impl From<KernelStartupError> for NativeSessionError {
    fn from(value: KernelStartupError) -> Self {
        Self::Core(value)
    }
}
impl From<NativeDispatchError> for NativeSessionError {
    fn from(value: NativeDispatchError) -> Self {
        Self::Dispatch(value)
    }
}
impl From<CodegenError> for NativeSessionError {
    fn from(value: CodegenError) -> Self {
        Self::Codegen(value)
    }
}
impl From<NativeMemoryError> for NativeSessionError {
    fn from(value: NativeMemoryError) -> Self {
        Self::Memory(value)
    }
}

impl From<pentax_core::process::hle::LibcBuiltinError> for NativeSessionError {
    fn from(value: pentax_core::process::hle::LibcBuiltinError) -> Self {
        use pentax_core::process::hle::{HleError, LibcBuiltinError};
        match value {
            LibcBuiltinError::Core(error) => Self::Core(error),
            LibcBuiltinError::Dispatch(error) => Self::Dispatch(error),
            LibcBuiltinError::Service(error) => {
                Self::Core(KernelStartupError::Hle(HleError::Service(error)))
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct NativeSessionReport {
    pub return_value: u64,
    pub primary_native_instructions: u64,
    pub state: NativeCpuState,
    pub dispatch: NativeDispatchUsage,
    pub service_stops: u64,
    pub fetched_source_bytes: u64,
    pub admission_work: u64,
}

/// Partial observations survive a typed stop. Block attempts alone are not
/// evidence of native guest execution; native_instructions counts returned bodies.
#[derive(Debug, Default)]
pub(super) struct NativeSessionProgress {
    pub dispatch: NativeDispatchUsage,
    pub service_stops: u64,
    pub fetched_source_bytes: u64,
    pub admission_work: u64,
    /// Successfully returned native bodies from supplied primary module zero,
    /// excluding generated service/provider code and modeled operations.
    pub primary_native_instructions: u64,
    /// Initializer calls observed returning through the exact outer slot.
    pub initializer_calls: u64,
    pub phase: NativeSessionPhase,
    pub plan_cache_hits: u64,
    pub plan_cache_misses: u64,
    /// Public catalogue label captured only when that admitted call fails.
    pub failed_kernel_service: Option<&'static str>,
    pub output_bytes: u64,
    pub kernel_handles: Option<pentax_kernel::HandleUsage>,
    pub audio_out2: Option<pentax_kernel::AudioOut2Usage>,
    pub libc_builtins: pentax_core::process::hle::LibcBuiltinUsage,
    pub libc_builtin_bound_entries: u64,
    pub private_samples_status: Option<&'static str>,
    pub private_console_status: Option<&'static str>,
    window: frame_bridge::Observations,
    stop_diagnostics: StopDiagnostics,
    dispatch_exit: Option<Mnemonic>,
    dispatch_context: dispatch_context::DispatchContext,
    profile: StartupProfile,
    scheduling: scheduling::Observations,
    native_ymm_mode: Option<NativeYmmMode>,
}

#[derive(Debug, Clone, Copy)]
enum NativeYmmMode {
    Sse2,
    Avx,
    Avx2,
}

impl NativeSessionProgress {
    pub fn private_samples_bytes(&self) -> Option<&[u8]> {
        self.profile.private_samples_bytes()
    }

    /// Aggregate observations only: no guest addresses, identifiers or data.
    pub fn protocol_fields(&self) -> String {
        let private = self
            .private_samples_status
            .map_or_else(String::new, |status| {
                format!("\tprivate_samples_status={status}")
            });
        let console = self
            .private_console_status
            .map_or_else(String::new, |status| {
                format!("\tprivate_console_status={status}")
            });
        format!(
            "{}{}{}\tlibc_builtin_attempted_memcpy={}\tlibc_builtin_attempted_memmove={}\tlibc_builtin_attempted_memset={}\tlibc_builtin_completed_memcpy={}\tlibc_builtin_completed_memmove={}\tlibc_builtin_completed_memset={}\tlibc_builtin_bound_entries={}{private}{console}",
            self.progress_fields(),
            self.profile.final_sample_fields(),
            self.scheduling.protocol_fields(),
            self.libc_builtins.attempted[0],
            self.libc_builtins.attempted[1],
            self.libc_builtins.attempted[2],
            self.libc_builtins.completed[0],
            self.libc_builtins.completed[1],
            self.libc_builtins.completed[2],
            self.libc_builtin_bound_entries,
        )
    }

    // Preserve existing PROGRESS records and their total bounded output size.
    fn progress_fields(&self) -> String {
        // Closed aggregate mode only, observed from the actual retained engine.
        // No requested policy, host identity or raw XCR0 value is reported.
        let native_ymm = match self.native_ymm_mode {
            None => "",
            Some(NativeYmmMode::Sse2) => "\tnative_ymm_mode=sse2",
            Some(NativeYmmMode::Avx) => "\tnative_ymm_mode=avx",
            Some(NativeYmmMode::Avx2) => "\tnative_ymm_mode=avx2",
        };
        let counters = format!(
            "block_attempts={}\tnative_instructions={}\tprimary_native_instructions={}\tservice_stops={}\tsegment_ops={}\tstring_elements={}\tstring_bytes={}\tvector_ops={}\tvector_bytes={}\tphase={}\tinitializer_calls={}\tfetched_source_bytes={}\tadmission_work={}\tdispatch_work={}\tplan_cache_hits={}\tplan_cache_misses={}",
            self.dispatch.blocks,
            self.dispatch.native_instructions,
            self.primary_native_instructions,
            self.service_stops,
            self.dispatch.segment_operations,
            self.dispatch.string_elements,
            self.dispatch.string_bytes,
            self.dispatch.vector_operations,
            self.dispatch.vector_bytes,
            self.phase.code(),
            self.initializer_calls,
            self.fetched_source_bytes,
            self.admission_work,
            self.dispatch.work_units,
            self.plan_cache_hits,
            self.plan_cache_misses,
        );
        let exit = self
            .dispatch_exit
            .map(|mnemonic| format!("\tplanned_exit={mnemonic:?}"))
            .unwrap_or_default();
        let handles = self.kernel_handles.map_or_else(String::new, |usage| {
            let mut fields = format!(
            "\tkernel_handles_live={}\tkernel_handles_limit={}\tkernel_handles_accounted={}\tkernel_pthread_objects={}\tkernel_handles_by_type=memory:{},files:{},modules:{},sync:{},events:{},network:{},ampr:{},keys:{},semaphores:{},fibers:{}",
            usage.live, usage.limit, usage.accounted(), usage.pthread_objects,
            usage.memory, usage.files, usage.modules, usage.synchronization,
            usage.events, usage.network, usage.ampr, usage.keys, usage.semaphores, usage.fibers,
            );
            if usage.libc_heaps != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",libc_heaps:{}", usage.libc_heaps);
            }
            if usage.trophies != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",trophies:{}", usage.trophies);
            }
            if usage.uds != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",uds:{}", usage.uds);
            }
            if usage.fonts != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",fonts:{}", usage.fonts);
            }
            if usage.ime != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",ime:{}", usage.ime);
            }
            if usage.acm != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",acm:{}", usage.acm);
            }
            if usage.audio2 != 0 {
                use std::fmt::Write;
                let _ = write!(fields, ",audio2:{}", usage.audio2);
            }
            {
                use std::fmt::Write;
                let _ = write!(
                    fields,
                    "\tkernel_trophy_contexts={}\tkernel_trophy_packages={}\tkernel_trophy_users={}\tkernel_trophy_registered_contexts={}",
                    usage.trophy_contexts, usage.trophy_packages, usage.trophy_users,
                    usage.trophy_registered_contexts,
                );
            }
            fields
        });
        let audio = self.audio_out2.map_or_else(String::new, |usage| {
            format!(
                "\taudio_mix_frames={}\taudio_logical_output_batches={}",
                usage.mix_frames, usage.logical_output_batches
            )
        });
        let window = format!(
            "\twindow_configured={}\twindow_frames_presented={}\twindow_frames_acknowledged={}",
            u8::from(self.window.configured),
            self.window.presented,
            self.window.acknowledged,
        );
        format!(
            "{counters}\tnative_entries={}\tcache_publications={}\tcache_reuses={}\toutput_bytes={}\tinteger_ops={}\tinteger_bytes={}\t{}{}{exit}{handles}{}{native_ymm}{audio}{window}",
            self.dispatch.native_entries,
            self.dispatch.cache_publications,
            self.dispatch.cache_reuses,
            self.output_bytes,
            self.dispatch.integer_operations,
            self.dispatch.integer_bytes,
            self.profile.protocol_fields(),
            self.stop_diagnostics.protocol_fields(),
            self.dispatch_context.protocol_fields(),
        )
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(super) enum NativeSessionPhase {
    #[default]
    Admission,
    Initializer,
    MainEntry,
    ThreadEntry,
}
impl NativeSessionPhase {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Admission => "admission",
            Self::Initializer => "initializer",
            Self::MainEntry => "main-entry",
            Self::ThreadEntry => "thread-entry",
        }
    }
}

/// Closed original diagnostic through the shared prepared-session runner.
/// No path, bytes, caller context or externally selected native operand enters.
pub(super) fn run_original() -> Result<
    (
        pentax_core::process::hle::OriginalPreparedReport,
        NativeSessionReport,
    ),
    pentax_core::process::hle::OriginalPreparedError<NativeSessionError>,
> {
    use pentax_core::process::hle::{OriginalPreparedError, run_original_prepared_fixture};
    let limits = NativeSessionLimits::default();
    let mut executed = None;
    let observations = run_original_prepared_fixture(|session, layout| {
        let proof = session.prepare_native_source(&limits.block)?;
        let mut initial = NativeCpuState::new(layout.entry, layout.return_slot);
        initial.gpr[5] = layout.frame_pointer;
        initial.gpr[7] = layout.arguments;
        initial.gpr[6] = layout.exit_handler;
        let stack = NativeStackFrame {
            stack: layout.stack,
            guards: Some(layout.guards),
            return_slot: layout.return_slot,
            return_marker: layout.return_marker,
        };
        let report = run_prepared(
            session,
            &proof,
            initial,
            stack,
            &limits,
            &mut NativeSessionProgress::default(),
        )?;
        if report.state.guest_pc != layout.return_marker
            || report.state.gpr[4] != layout.return_slot + 8
            || report.state.gpr[5] != initial.gpr[5]
            || report.state.gpr[12] != initial.gpr[12]
            || report.service_stops != 4
        {
            return Err(NativeSessionError::FixtureInvariant("final context"));
        }
        let value = report.return_value;
        executed = Some(report);
        Ok(value)
    })?;
    let execution = executed.ok_or(OriginalPreparedError::Fixture("missing execution report"))?;
    Ok((observations, execution))
}

/// Run bounded one-block steps against the actual prepared kernel owner.
/// The proof is constructed by the core from its own typed TLS materializer;
/// safe caller-declared numeric patches cannot substitute for it here.
///
/// Source provenance and owner binding are checked here. The platform separately
/// requires each memory instruction to stand alone and validates every explicit
/// and implicit access against these same exclusive pages before native entry.
/// This remains a dedicated-worker API: arithmetic exceptions/hangs are not
/// recovered, and the parent must retain its timeout/fatal-exception policy.
pub(super) fn run_prepared<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
) -> Result<NativeSessionReport, NativeSessionError> {
    run_prepared_with_initializer(session, proof, initial, stack, None, limits, progress)
}

/// The optional observed initializer and main entry share one thread admission,
/// executor/cache, actual memory owner and cumulative budgets. Only an exact
/// completed call can transition into the next call; errors never resume.
pub(super) fn run_prepared_with_initializer<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    initializer: Option<&PreparedNativeModuleStart<'_, '_>>,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
) -> Result<NativeSessionReport, NativeSessionError> {
    run_prepared_with_consumer(
        session,
        proof,
        initial,
        stack,
        initializer,
        limits,
        progress,
        None,
    )
}

/// Explicit same-process consumer path. Its checked serial is a nonsecret
/// identity, not a security token or a cross-process execution capability.
pub(super) fn run_prepared_with_window<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    initializer: Option<&PreparedNativeModuleStart<'_, '_>>,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
) -> Result<NativeSessionReport, NativeSessionError> {
    use frame_bridge::local_presenter::{Limits as WindowLimits, LocalPresenterTransport};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_LOCAL_WINDOW: AtomicU64 = AtomicU64::new(1);
    let serial = NEXT_LOCAL_WINDOW
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| NativeSessionError::Budget("local window identities"))?;
    let mut identity = [0; 16];
    identity[..8].copy_from_slice(b"PXWINDOW");
    identity[8..].copy_from_slice(&serial.to_le_bytes());
    let nonce = pentax_session::Nonce::new(identity)
        .map_err(|_| NativeSessionError::Frame(frame_bridge::Error::Protocol))?;
    let window = WindowLimits::default();
    let mut transport =
        LocalPresenterTransport::new(nonce, 1, window).map_err(NativeSessionError::Frame)?;
    let mut bridge = frame_bridge::FrameBridge::new(
        &mut transport,
        nonce,
        1,
        frame_bridge::Limits {
            wire: window.wire,
            ..if limits.interactive {
                frame_bridge::Limits::interactive()
            } else {
                Default::default()
            }
        },
    )
    .map_err(NativeSessionError::Frame)?;
    run_prepared_with_consumer(
        session,
        proof,
        initial,
        stack,
        initializer,
        limits,
        progress,
        Some(&mut bridge),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_prepared_with_consumer<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    initializer: Option<&PreparedNativeModuleStart<'_, '_>>,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
    mut consumer: Option<&mut frame_bridge::FrameBridge<'_>>,
) -> Result<NativeSessionReport, NativeSessionError> {
    let mut result = run_prepared_consumer_inner(
        session,
        proof,
        initial,
        stack,
        initializer,
        limits,
        progress,
        consumer.as_deref_mut(),
    );
    if let Some(consumer) = consumer {
        progress.window = consumer.observations();
        if result.is_ok() && consumer.is_pending() {
            result = Err(NativeSessionError::Frame(frame_bridge::Error::Cancelled));
        }
        consumer.cancel();
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn run_prepared_consumer_inner<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    initial: NativeCpuState,
    stack: NativeStackFrame,
    initializer: Option<&PreparedNativeModuleStart<'_, '_>>,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
    mut consumer: Option<&mut frame_bridge::FrameBridge<'_>>,
) -> Result<NativeSessionReport, NativeSessionError> {
    *progress = NativeSessionProgress::default();
    if limits.private_samples {
        progress.profile.enable_private_samples();
    }
    let mut initial = initial;
    let mut main_thread = None;
    let source = proof.materialized_source();
    if proof.owner_id() != session.owner_id()
        || !std::ptr::eq(source.source_plan(), session.plan().link_plan())
        || !std::ptr::eq(source.source_plan(), session.memory().source_plan())
    {
        return Err(NativeSessionError::WrongPreparedSource);
    }
    if session.tls().thread_control_policy()
        != pentax_core::process::tls::ThreadControlBlockPolicy::None
    {
        let thread = session.prepare_main_thread()?;
        if thread.owner_id() != proof.owner_id()
            || !std::ptr::eq(thread.source_plan(), source.source_plan())
            || initial
                .fs_base
                .is_some_and(|base| base != thread.thread_pointer())
            || initial.gs_base.is_some()
        {
            return Err(NativeSessionError::WrongPreparedSource);
        }
        initial.fs_base = Some(thread.thread_pointer());
        main_thread = Some(thread);
    } else if initial.fs_base.is_some() || initial.gs_base.is_some() {
        return Err(NativeSessionError::WrongPreparedSource);
    }
    let main = initial;
    let mut pending_main = false;
    if let Some(module) = initializer {
        let process = module.process_entry();
        if module.owner_id() != proof.owner_id()
            || !std::ptr::eq(module.source_plan(), source.source_plan())
            || module.module_index() == 0
            || main.guest_pc != process.entry
            || stack.stack != process.stack
            || stack.guards != Some(process.guards)
            || stack.return_slot != process.return_slot
            || stack.return_marker != process.return_marker
            || main.gpr[4] != process.return_slot
            || main.gpr[5] != process.frame_pointer
            || main.gpr[7] != process.arguments
            || main.gpr[6] != process.exit_handler
        {
            return Err(NativeSessionError::WrongPreparedSource);
        }
        initial.guest_pc = module.entry();
        initial.gpr[7] = 0;
        initial.gpr[6] = 0;
        initial.gpr[2] = 0;
        pending_main = true;
    }
    let mut state = RunState {
        scheduling: Scheduling::new(main_thread, limits.dispatch.max_thread_contexts)?,
        main,
        pending_main,
        initial: Some(initial),
        stack,
        frames: Vec::new(),
        admission_work: 0,
        fetched_source_bytes: 0,
        service_stops: 0,
        reported_blocks: 0,
        report: None,
        frame_pump_blocks: None,
    };
    state
        .frames
        .try_reserve_exact(32)
        .map_err(|_| NativeSessionError::Allocation)?;
    let files = session.dynamic_file_count();
    let mut arena: Vec<OnceLock<DynamicModuleInput>> = Vec::new();
    arena
        .try_reserve_exact(files)
        .map_err(|_| NativeSessionError::Allocation)?;
    arena.resize_with(files, OnceLock::new);
    // Stable storage exists before any epoch borrows its source. Both arenas
    // outlive current and are never resized while a plan refers to their data.
    let mut unavailable_arena: Vec<OnceLock<GeneratedUnavailableProvider>> = Vec::new();
    unavailable_arena
        .try_reserve_exact(files)
        .map_err(|_| NativeSessionError::Allocation)?;
    unavailable_arena.resize_with(files, OnceLock::new);
    let base = source.source_plan();
    let mut current = None;
    let mut machine = None;
    let mut starting = None;
    progress.phase = if pending_main {
        NativeSessionPhase::Initializer
    } else {
        NativeSessionPhase::MainEntry
    };
    loop {
        let active = current.as_ref().unwrap_or(base);
        let epoch = session.prepare_epoch_native_source(active, &limits.block)?;
        let outcome = execute_epoch(
            session,
            &epoch,
            &mut machine,
            starting.take(),
            &mut state,
            initializer,
            limits,
            progress,
            consumer.as_deref_mut(),
        );
        // Normalize while the exact final epoch plan is still borrowed. An
        // intermediate dynamic-module transition must not leave a stale file.
        if limits.private_samples && !matches!(&outcome, Ok(Some(_))) {
            progress.profile.capture_private_samples(active);
        }
        progress.libc_builtins = session.libc_builtin_usage();
        // Preserve a terminal queue observation before propagating typed stops.
        // This bounded snapshot never probes readiness or changes scheduling.
        progress.scheduling = state.scheduling.observations(
            session.video_registration_counts(),
            session.graphics_usage(),
            session.graphics_video_flip_observations(),
            session.graphics_function_profile(),
        );
        match outcome? {
            None => {
                return state
                    .report
                    .take()
                    .ok_or(NativeSessionError::FixtureInvariant("missing final report"));
            }
            Some((pending, token)) => {
                let input = session.read_dynamic_module(pending)?;
                let slot = arena
                    .get(input.slot())
                    .ok_or(NativeSessionError::WrongPreparedSource)?;
                slot.set(input)
                    .map_err(|_| NativeSessionError::WrongPreparedSource)?;
                let input = slot.get().ok_or(NativeSessionError::WrongPreparedSource)?;
                let bias = session.dynamic_load_bias(input)?;
                let view = pentax_loader::open_executable(input.bytes())
                    .map_err(|_| NativeSessionError::SourceWindow)?;
                let policies = session.dynamic_fallback_policies(&epoch)?;
                let added = pentax_linker::ModuleInput::new(view, bias);
                let extension_limits = session.dynamic_extension_limits();
                let unavailable_limits = session.dynamic_unavailable_limits()?;
                let missing = pentax_linker::plan_missing_functions_for_extension(
                    active,
                    added,
                    &policies,
                    &extension_limits,
                    unavailable_limits.max_functions as u64,
                )
                .map_err(NativeSessionError::Extension)?;
                let delta = if missing.functions().is_empty() {
                    None
                } else {
                    let provider = build_unavailable_provider(missing, &unavailable_limits)
                        .map_err(|error| match error {
                            UnavailableProviderError::Allocation => NativeSessionError::Allocation,
                            UnavailableProviderError::Budget(name) => {
                                NativeSessionError::Budget(name)
                            }
                            UnavailableProviderError::Layout => NativeSessionError::Core(
                                KernelStartupError::InvalidProvider("dynamic unavailable layout"),
                            ),
                        })?;
                    let slot = unavailable_arena
                        .get(input.slot())
                        .ok_or(NativeSessionError::WrongPreparedSource)?;
                    slot.set(provider)
                        .map_err(|_| NativeSessionError::WrongPreparedSource)?;
                    Some(slot.get().ok_or(NativeSessionError::WrongPreparedSource)?)
                };
                let guest = pentax_linker::ModuleInput::new(
                    pentax_loader::open_executable(input.bytes())
                        .map_err(|_| NativeSessionError::SourceWindow)?,
                    bias,
                );
                let next = if let Some(delta) = delta {
                    let delta_input = pentax_linker::ModuleInput::new(
                        pentax_loader::open_executable(delta.image())
                            .map_err(|_| NativeSessionError::SourceWindow)?,
                        session.dynamic_unavailable_bias(input)?,
                    );
                    pentax_linker::append_symbolic_modules_with_fallbacks(
                        active,
                        &[guest, delta_input],
                        &policies,
                        &extension_limits,
                    )
                } else {
                    pentax_linker::append_symbolic_modules_with_fallbacks(
                        active,
                        &[guest],
                        &policies,
                        &extension_limits,
                    )
                }
                .map_err(NativeSessionError::Extension)?;
                let start =
                    session.publish_dynamic_batch(&epoch, input, &next, delta, &limits.block)?;
                current = Some(next);
                starting = Some((start, token));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SuspendedInitializer {
    Module(DynamicModuleStart),
    Once(PreparedPthreadOnce),
    NetCtl(PreparedNetCtlCallback),
    FontParser(PreparedFontParserCallback),
    ImeCallback(PreparedImeCallback),
}
struct RunState<'plan, 'image> {
    scheduling: Scheduling<'plan, 'image>,
    main: NativeCpuState,
    pending_main: bool,
    initial: Option<NativeCpuState>,
    stack: NativeStackFrame,
    frames: Vec<(SuspendedInitializer, NativeNestedCall, NativeSessionPhase)>,
    admission_work: u64,
    fetched_source_bytes: u64,
    service_stops: u64,
    reported_blocks: u64,
    report: Option<NativeSessionReport>,
    frame_pump_blocks: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
fn execute_epoch<'plan, 'image>(
    session: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &PreparedNativeSource<'_, '_>,
    machine: &mut Option<NativeBlockMachine>,
    starting: Option<(DynamicModuleStart, PreSyscallToken)>,
    state: &mut RunState<'plan, 'image>,
    initializer: Option<&PreparedNativeModuleStart<'_, '_>>,
    limits: &NativeSessionLimits,
    progress: &mut NativeSessionProgress,
    mut consumer: Option<&mut frame_bridge::FrameBridge<'_>>,
) -> Result<Option<(PendingModuleLoad, PreSyscallToken)>, NativeSessionError> {
    let source = proof.materialized_source();
    if proof.owner_id() != session.owner_id()
        || source.source_plan().identity() != session.memory().active_source_identity()
    {
        return Err(NativeSessionError::WrongPreparedSource);
    }
    let libc_builtins = limits
        .libc_memory_builtins
        .map(|module| session.prepare_libc_builtins(proof, module))
        .transpose()?;
    progress.libc_builtin_bound_entries = libc_builtins
        .as_ref()
        .map_or(0, |manifest| manifest.function_count());
    if limits.report_progress {
        progress
            .profile
            .begin_sample_epoch(source.source_plan().identity());
    }
    let metadata_visits = count_metadata(source, limits, &mut state.admission_work)?;
    progress.admission_work = state.admission_work;
    let mut engine = {
        let mut paused = session.pause()?;
        if let Some(initial) = state.initial.take() {
            NativeBlockStepper::new(source, &mut paused, initial, state.stack, &limits.dispatch)?
        } else {
            match NativeBlockStepper::from_machine(source, &mut paused, machine) {
                Ok(engine) => engine,
                Err(error) => {
                    if let Some(retained) = machine {
                        progress.dispatch = retained.usage();
                    }
                    return Err(error.into());
                }
            }
        }
    };
    progress.dispatch = engine.usage();
    let native_ymm_features = engine.native_ymm_features();
    progress.native_ymm_mode = Some(match native_ymm_features {
        None => NativeYmmMode::Sse2,
        Some(features) if features.contains(CpuidFeature::AVX2) => NativeYmmMode::Avx2,
        Some(_) => NativeYmmMode::Avx,
    });
    if let Some((start, token)) = starting {
        let call =
            engine.begin_syscall_call(&mut session.pause()?, token, start.entry(), start.args());
        progress.dispatch = engine.usage();
        let call = call?;
        session.observe_dynamic_module_call(start, &call)?;
        if state.frames.len() >= 32 {
            return Err(NativeSessionError::Budget("initializer frames"));
        }
        state
            .frames
            .push((SuspendedInitializer::Module(start), call, progress.phase));
        progress.phase = NativeSessionPhase::Initializer;
    }
    // This cache borrows only this exact source view; module publication returns
    // from the epoch and drops every retained plan before another view is bound.
    // Native feature admission describes actual host support. Count operations
    // without it remain modeled exits; other unported instructions stay typed.
    let mut native_features = CpuFeatures::x64_sse2();
    for (feature, available) in [
        (CpuidFeature::BMI1, std::is_x86_feature_detected!("bmi1")),
        (CpuidFeature::BMI2, std::is_x86_feature_detected!("bmi2")),
        (
            CpuidFeature::POPCNT,
            std::is_x86_feature_detected!("popcnt"),
        ),
    ] {
        if available {
            native_features = native_features.with(feature);
        }
    }
    // AVX declarations come only from this exact engine's admitted full-YMM
    // bridge mode. Raw host AVX flags alone never enable native vector plans.
    if let Some(features) = native_ymm_features {
        for feature in [CpuidFeature::AVX, CpuidFeature::AVX2] {
            if features.contains(feature) {
                native_features = native_features.with(feature);
            }
        }
    }
    let mut policy = TranslationPolicy::new(native_features);
    if native_ymm_features.is_some() {
        policy = policy
            .with_native_ymm_registers()
            .with_native_packed_float_memory()
            .with_native_streaming_stores()
            .with_native_vector_moves()
            .with_native_scalar_float();
    }
    let plans = plan_cache::PlanCache::new(
        source,
        policy
            .with_independent_memory()
            .with_partial_window_splits(),
        limits.block,
        &mut state.admission_work,
        limits.max_admission_work,
    );
    progress.admission_work = state.admission_work;
    let mut plans = plans?;
    // Reuse only allocation storage within this source epoch. Every attempt
    // still fetches its complete live executable window before cache lookup.
    let mut window_storage = Vec::new();
    loop {
        let mut external = None;
        if let Some(consumer) = consumer.as_deref_mut() {
            let blocked = state.scheduling.active_blocked();
            if blocked
                || state
                    .frame_pump_blocks
                    .is_none_or(|old| engine.usage().blocks.saturating_sub(old) >= 1024)
            {
                let pumped = consumer.pump(session);
                progress.window = consumer.observations();
                match pumped.map_err(NativeSessionError::Frame)? {
                    frame_bridge::Pump::RetryEventDelivery => {
                        let wake = consumer
                            .external_wake()
                            .map_err(NativeSessionError::Frame)?
                            .ok_or(NativeSessionError::Frame(frame_bridge::Error::Protocol))?;
                        state.scheduling.wait_consumer_retry(wake)?;
                        state.frame_pump_blocks = None;
                        continue;
                    }
                    frame_bridge::Pump::NeedsEventDelivery => {
                        return Err(NativeSessionError::Frame(
                            frame_bridge::Error::NeedsEventDelivery,
                        ));
                    }
                    frame_bridge::Pump::Idle | frame_bridge::Pump::Progress => {}
                }
                state.frame_pump_blocks = Some(engine.usage().blocks);
            }
            if blocked {
                external = consumer
                    .external_wake()
                    .map_err(NativeSessionError::Frame)?;
            }
        }
        let runnable = state.scheduling.rotate(
            session,
            proof,
            &mut engine,
            &mut state.frames,
            &mut progress.phase,
            external,
            limits.interactive,
        )?;
        progress.dispatch = engine.usage();
        if !runnable {
            continue;
        }
        // Profiles retain at most 48 progress records. Publication is outside
        // native entry and follows completed service observations.
        if limits.report_progress
            && progress
                .dispatch
                .blocks
                .saturating_sub(state.reported_blocks)
                >= limits.progress_interval_blocks.max(1)
        {
            crate::protocol(&format!(
                "PROGRESS\tmode=entry-probe\t{}",
                progress.progress_fields()
            ))
            .map_err(|_| NativeSessionError::OutputUnavailable)?;
            state.reported_blocks = progress.dispatch.blocks;
        }
        let sampled = StartupProfile::sampled(limits.report_progress, progress.dispatch.blocks);
        if sampled {
            charge(
                &mut state.admission_work,
                32,
                limits.max_admission_work,
                "admission work",
            )?;
            progress.admission_work = state.admission_work;
            progress.profile.samples = progress.profile.samples.saturating_add(1);
        }
        let mut stamp = sampled.then(Instant::now);
        let block = {
            let paused = session.pause()?;
            if paused.owner_id() != proof.owner_id() {
                return Err(NativeSessionError::WrongPreparedSource);
            }
            charge(
                &mut state.admission_work,
                metadata_visits,
                limits.max_admission_work,
                "admission work",
            )?;
            progress.admission_work = state.admission_work;
            let (module, length) = plans.initialized_window(
                engine.state().guest_pc,
                limits
                    .block
                    .max_source_bytes
                    .min(limits.max_window_allocation_bytes),
            )?;
            charge(
                &mut state.fetched_source_bytes,
                length,
                limits.max_total_window_bytes,
                "source windows",
            )?;
            progress.fetched_source_bytes = state.fetched_source_bytes;
            let copy_work = length
                .checked_mul(2)
                .ok_or(NativeSessionError::Budget("admission work"))?;
            charge(
                &mut state.admission_work,
                copy_work,
                limits.max_admission_work,
                "admission work",
            )?;
            progress.admission_work = state.admission_work;
            let window = fetch_window_into(
                &paused,
                engine.state().guest_pc,
                length,
                limits.max_window_allocation_bytes,
                &mut window_storage,
            )?;
            StartupProfile::checkpoint(&mut stamp, &mut progress.profile.window_ns);
            let prepared = plans.prepare(
                module,
                engine.state().guest_pc,
                window,
                engine.placement_for(engine.state().guest_pc),
                &mut state.admission_work,
                limits.max_admission_work,
            );
            progress.admission_work = state.admission_work;
            let (block, hit) = prepared?;
            charge(
                if hit {
                    &mut progress.plan_cache_hits
                } else {
                    &mut progress.plan_cache_misses
                },
                1,
                u64::MAX,
                "plan cache observations",
            )?;
            StartupProfile::checkpoint(&mut stamp, &mut progress.profile.plan_ns);
            block
        };
        // Exact supplied-library API entries are an explicit opt-in. A normal
        // pre-effect fallback still runs the original native block below.
        let builtin = if let Some(manifest) = libc_builtins
            .as_ref()
            .filter(|manifest| manifest.contains_pc(engine.state().guest_pc))
        {
            let next_stops = state
                .service_stops
                .checked_add(1)
                .ok_or(NativeSessionError::Budget("service observations"))?;
            let service_start = limits.report_progress.then(Instant::now);
            let result = session.try_libc_builtin(manifest, proof, &mut engine, block);
            progress.profile.service_finished(service_start);
            progress.dispatch = engine.usage();
            if !matches!(&result, Ok(None)) {
                state.service_stops = next_stops;
                progress.service_stops = next_stops;
            }
            let stop = result?;
            if stop.is_some() && limits.report_progress {
                // Standard libc is not a kernel catalogue label. This updates
                // only the existing other-provider aggregate, without a scan.
                progress.profile.services.record(
                    None,
                    false,
                    &mut state.admission_work,
                    limits.max_admission_work,
                )?;
            }
            stop
        } else {
            None
        };
        let stop = if let Some(stop) = builtin {
            stop
        } else {
            let plan = session.plan();
            let mut paused = session.pause()?;
            let previous_native = engine.usage().native_instructions;
            // Optional telemetry owns an independent bounded work allowance.
            // It preadmits every record/ring visit before guest dispatch and
            // never changes guest limits or makes an observer refusal fatal.
            let observation = if sampled {
                progress.profile.prepare_block_sample(
                    source.source_plan().identity(),
                    block,
                    limits.block.max_instructions,
                    state.scheduling.is_main(),
                )
            } else {
                None
            };
            // SAFETY: The opaque core proof, exact owner and active published
            // source epoch were
            // checked above. This exclusive pause owns the actual backing and
            // ends before service work; the process-wide bridge guard excludes
            // concurrent entry. The checked engine derives ordinary accesses
            // from current state and validates their live bounds/permissions.
            let result = unsafe { engine.step_checked(&mut paused, block) };
            StartupProfile::checkpoint(&mut stamp, &mut progress.profile.dispatch_ns);
            progress.dispatch = engine.usage();
            if result.is_ok() {
                if let Some(observation) = observation {
                    progress.profile.complete_block_sample(observation);
                }
                let count = if progress.dispatch.native_instructions > previous_native {
                    &mut progress.profile.native_body_blocks
                } else {
                    &mut progress.profile.exit_only_blocks
                };
                *count = count.saturating_add(1);
            } else {
                // This is the planned exit mnemonic, not a faulting body PC.
                progress.dispatch_exit = block.exit().instruction().map(|i| i.mnemonic());
                if let Err(error) = &result {
                    progress.dispatch_context.capture(
                        error,
                        block,
                        &paused,
                        plan,
                        source,
                        progress.dispatch.native_instructions > previous_native,
                    );
                }
            }
            if block.module().index() == 0 {
                let completed = progress
                    .dispatch
                    .native_instructions
                    .checked_sub(previous_native)
                    .ok_or(NativeSessionError::Budget("native observations"))?;
                progress.primary_native_instructions = progress
                    .primary_native_instructions
                    .checked_add(completed)
                    .ok_or(NativeSessionError::Budget("primary native observations"))?;
            }
            result?
        };
        match stop {
            NativeBlockStop::Resume { .. } => {}
            NativeBlockStop::Returned { value } => {
                if let Some((start, nested, phase)) = state.frames.last().copied() {
                    let returned =
                        engine
                            .observed_return()
                            .ok_or(NativeSessionError::FixtureInvariant(
                                "missing initializer return",
                            ))?;
                    let action = match start {
                        SuspendedInitializer::Module(start) => NativeServiceAction::Returned(
                            session.complete_dynamic_module(proof, start, &returned)?,
                        ),
                        SuspendedInitializer::Once(start) => NativeServiceAction::Returned(
                            session.complete_pthread_once_call(proof, start, &returned)?,
                        ),
                        SuspendedInitializer::NetCtl(start) => {
                            session.complete_netctl_callback_call(proof, start, &returned)?
                        }
                        SuspendedInitializer::FontParser(start) => session
                            .complete_font_parser_call(proof, start, &returned, nested, &engine)?,
                        SuspendedInitializer::ImeCallback(start) => session
                            .complete_ime_callback_call(proof, start, &returned, nested, &engine)?,
                    };
                    match action {
                        NativeServiceAction::Returned(value) => {
                            let completed =
                                engine.complete_syscall_call(&mut session.pause()?, nested, value);
                            progress.dispatch = engine.usage();
                            completed?;
                            state.frames.pop();
                            if !matches!(
                                start,
                                SuspendedInitializer::NetCtl(_)
                                    | SuspendedInitializer::FontParser(_)
                                    | SuspendedInitializer::ImeCallback(_)
                            ) {
                                progress.initializer_calls =
                                    progress.initializer_calls.checked_add(1).ok_or(
                                        NativeSessionError::Budget("initializer observations"),
                                    )?;
                            }
                            progress.phase = phase;
                        }
                        NativeServiceAction::NetCtl(next) => {
                            let call = engine.continue_syscall_call(
                                &mut session.pause()?,
                                nested,
                                next.entry(),
                                next.args(),
                            );
                            progress.dispatch = engine.usage();
                            let call = call?;
                            session.observe_netctl_callback_call(proof, next, &call)?;
                            let frame = state.frames.last_mut().ok_or(
                                NativeSessionError::FixtureInvariant(
                                    "missing NetCtl callback frame",
                                ),
                            )?;
                            *frame = (SuspendedInitializer::NetCtl(next), call, phase);
                        }
                        NativeServiceAction::FontParser(next) => {
                            let call = engine.continue_syscall_call(
                                &mut session.pause()?,
                                nested,
                                next.entry(),
                                next.args(),
                            );
                            progress.dispatch = engine.usage();
                            let call = call?;
                            session.observe_font_parser_call(proof, next, &call)?;
                            let frame = state.frames.last_mut().ok_or(
                                NativeSessionError::FixtureInvariant("missing font parser frame"),
                            )?;
                            *frame = (SuspendedInitializer::FontParser(next), call, phase);
                        }
                        NativeServiceAction::ImeCallback(next) => {
                            let call = engine.continue_syscall_call(
                                &mut session.pause()?,
                                nested,
                                next.entry(),
                                next.args(),
                            );
                            progress.dispatch = engine.usage();
                            let call = call?;
                            session.observe_ime_callback_call(proof, next, &call)?;
                            let frame = state.frames.last_mut().ok_or(
                                NativeSessionError::FixtureInvariant("missing IME callback frame"),
                            )?;
                            *frame = (SuspendedInitializer::ImeCallback(next), call, phase);
                        }
                        _ => {
                            return Err(NativeSessionError::FixtureInvariant(
                                "unexpected nested callback continuation",
                            ));
                        }
                    }
                    continue;
                }
                if state.pending_main && state.scheduling.is_main() {
                    state.pending_main = false;
                    let returned =
                        engine
                            .observed_return()
                            .ok_or(NativeSessionError::FixtureInvariant(
                                "missing actual call return",
                            ))?;
                    session.observe_module_initializer(
                        initializer
                            .ok_or(NativeSessionError::FixtureInvariant("missing initializer"))?,
                        &returned,
                    )?;
                    progress.initializer_calls = progress
                        .initializer_calls
                        .checked_add(1)
                        .ok_or(NativeSessionError::Budget("initializer observations"))?;
                    // Preserve observed guest control/vector/segment state and
                    // its actual TLS contents. Establish only the main call's
                    // defined argument, frame and return-slot registers.
                    let mut next = *engine.state();
                    next.guest_pc = state.main.guest_pc;
                    for register in [4, 5, 6, 7] {
                        next.gpr[register] = state.main.gpr[register];
                    }
                    let transitioned = engine.begin_call(&mut session.pause()?, next);
                    progress.dispatch = engine.usage();
                    transitioned?;
                    progress.phase = NativeSessionPhase::MainEntry;
                    continue;
                }
                if !state.scheduling.is_main() {
                    state.scheduling.returned(session, proof, &mut engine)?;
                    progress.dispatch = engine.usage();
                    continue;
                }
                state.report = Some(NativeSessionReport {
                    return_value: value,
                    primary_native_instructions: progress.primary_native_instructions,
                    state: *engine.state(),
                    dispatch: engine.usage(),
                    service_stops: state.service_stops,
                    fetched_source_bytes: state.fetched_source_bytes,
                    admission_work: state.admission_work,
                });
                return Ok(None);
            }
            NativeBlockStop::PreSyscall(token) => {
                state.service_stops = state
                    .service_stops
                    .checked_add(1)
                    .ok_or(NativeSessionError::Budget("service stops"))?;
                progress.service_stops = state.service_stops;
                if limits.report_progress {
                    charge(
                        &mut state.admission_work,
                        16,
                        limits.max_admission_work,
                        "admission work",
                    )?;
                    progress.admission_work = state.admission_work;
                }
                let service_start = limits.report_progress.then(Instant::now);
                let action = session.service_action(
                    proof,
                    NativeServiceRequest {
                        owner_id: token.owner_id(),
                        syscall_pc: token.syscall_pc(),
                        next_pc: token.next_pc(),
                        service_id: token.service_id(),
                        args: token.args(),
                        vector_args: Some(token.vector_args()),
                        stack_pointer: Some(token.stack_pointer()),
                    },
                );
                progress.profile.service_finished(service_start);
                progress.output_bytes = session.output_usage().accepted_bytes;
                if action.is_err() {
                    progress.failed_kernel_service = session.last_admitted_kernel_service();
                    if matches!(
                        &action,
                        Err(KernelStartupError::Kernel(pentax_kernel::Failure::Guest(_)))
                    ) {
                        progress.stop_diagnostics.observe_failure(
                            progress.failed_kernel_service,
                            token.args(),
                            session.kernel_usage(),
                            session.memory().statistics(),
                        );
                    }
                }
                let action = action?;
                progress.stop_diagnostics.observe_service(
                    session.last_admitted_kernel_service(),
                    token.args(),
                    &action,
                    session.memory_map_refusal(),
                );
                if limits.report_progress {
                    let observed = progress.profile.services.record(
                        session.last_admitted_kernel_service(),
                        matches!(&action, NativeServiceAction::Returned(value) if *value != 0),
                        &mut state.admission_work,
                        limits.max_admission_work,
                    );
                    progress.admission_work = state.admission_work;
                    observed?;
                }
                // A typed service error returns above and drops this engine.
                // Never invent a return-domain mapping or resume with zero.
                match action {
                    NativeServiceAction::WaitThread(pending) => {
                        state.scheduling.wait(session, pending, token)?;
                    }
                    NativeServiceAction::CreateThread(pending) => {
                        state
                            .scheduling
                            .create(session, proof, &mut engine, pending, token)
                            .inspect_err(|_| {
                                progress.failed_kernel_service =
                                    session.last_admitted_kernel_service();
                            })?;
                    }
                    NativeServiceAction::Returned(value) => {
                        engine.complete_syscall(token, value)?
                    }
                    NativeServiceAction::Load(pending) => {
                        *machine = Some(engine.into_machine());
                        return Ok(Some((pending, token)));
                    }
                    NativeServiceAction::Once(start) => {
                        if state.frames.len() >= 32 {
                            return Err(NativeSessionError::Budget("initializer frames"));
                        }
                        let call = engine.begin_syscall_call(
                            &mut session.pause()?,
                            token,
                            start.entry(),
                            [0; 3],
                        );
                        progress.dispatch = engine.usage();
                        let call = call?;
                        session.observe_pthread_once_call(proof, start, &call)?;
                        state.frames.push((
                            SuspendedInitializer::Once(start),
                            call,
                            progress.phase,
                        ));
                        progress.phase = NativeSessionPhase::Initializer;
                    }
                    NativeServiceAction::NetCtl(start) => {
                        if state.frames.len() >= 32 {
                            return Err(NativeSessionError::Budget("callback frames"));
                        }
                        let call = engine.begin_syscall_call(
                            &mut session.pause()?,
                            token,
                            start.entry(),
                            start.args(),
                        );
                        progress.dispatch = engine.usage();
                        let call = call?;
                        session.observe_netctl_callback_call(proof, start, &call)?;
                        state.frames.push((
                            SuspendedInitializer::NetCtl(start),
                            call,
                            progress.phase,
                        ));
                    }
                    NativeServiceAction::FontParser(start) => {
                        if state.frames.len() >= 32 {
                            return Err(NativeSessionError::Budget("callback frames"));
                        }
                        let call = engine.begin_syscall_call(
                            &mut session.pause()?,
                            token,
                            start.entry(),
                            start.args(),
                        );
                        progress.dispatch = engine.usage();
                        let call = call?;
                        session.observe_font_parser_call(proof, start, &call)?;
                        state.frames.push((
                            SuspendedInitializer::FontParser(start),
                            call,
                            progress.phase,
                        ));
                    }
                    NativeServiceAction::ImeCallback(start) => {
                        if state.frames.len() >= 32 {
                            return Err(NativeSessionError::Budget("callback frames"));
                        }
                        let call = engine.begin_syscall_call(
                            &mut session.pause()?,
                            token,
                            start.entry(),
                            start.args(),
                        );
                        progress.dispatch = engine.usage();
                        let call = call?;
                        session.observe_ime_callback_call(proof, start, &call)?;
                        state.frames.push((
                            SuspendedInitializer::ImeCallback(start),
                            call,
                            progress.phase,
                        ));
                    }
                }
            }
            NativeBlockStop::Unsupported { guest_pc } => {
                if let Some(instruction) = block.exit().instruction() {
                    progress.stop_diagnostics.capture(
                        instruction,
                        match block.exit().kind() {
                            pentax_codegen::ExitKind::Mediated { reason } => Some(reason),
                            _ => None,
                        },
                        block.module().index() == 0,
                        session,
                    );
                    // The retained window was captured before this step. Its
                    // unconsumed suffix may be stale: decode only static public
                    // form candidates, never an execution/unsupported trace.
                    // Invalid bounds omit this optional diagnostic and preserve
                    // the original stop. No additional guest bytes are read.
                    if let Some(snapshot) = block
                        .exit()
                        .guest_pc()
                        .checked_sub(block.guest_pc())
                        .and_then(|offset| usize::try_from(offset).ok())
                        .and_then(|offset| block.relocated_source_bytes().get(offset..))
                    {
                        progress
                            .stop_diagnostics
                            .capture_snapshot_candidates(snapshot);
                    }
                }
                return Err(NativeSessionError::UnsupportedExit {
                    guest_pc,
                    mnemonic: block
                        .exit()
                        .instruction()
                        .map(|instruction| instruction.mnemonic()),
                });
            }
        }
    }
}

fn charge(
    value: &mut u64,
    amount: u64,
    limit: u64,
    name: &'static str,
) -> Result<(), NativeSessionError> {
    *value = value
        .checked_add(amount)
        .filter(|v| *v <= limit)
        .ok_or(NativeSessionError::Budget(name))?;
    Ok(())
}

fn count_metadata(
    source: &MaterializedSource<'_, '_>,
    limits: &NativeSessionLimits,
    work: &mut u64,
) -> Result<u64, NativeSessionError> {
    let modules = source.source_plan().modules();
    if modules.len() as u64 > limits.block.max_modules {
        return Err(NativeSessionError::Budget("modules"));
    }
    charge(
        work,
        modules.len() as u64,
        limits.max_admission_work,
        "admission work",
    )?;
    let mut segments = 0_u64;
    for module in modules {
        segments = segments
            .checked_add(module.runtime_plan().segments.len() as u64)
            .filter(|v| *v <= limits.block.max_segments)
            .ok_or(NativeSessionError::Budget("segments"))?;
    }
    (modules.len() as u64)
        .checked_add(segments)
        .ok_or(NativeSessionError::Budget("admission work"))
}

fn initialized_window(
    source: &MaterializedSource<'_, '_>,
    pc: u64,
    maximum: u64,
) -> Result<(usize, u64), NativeSessionError> {
    let mut found = None;
    for (index, module) in source.source_plan().modules().iter().enumerate() {
        for segment in &module.runtime_plan().segments {
            if !segment.flags.executable() || segment.flags.writable() {
                continue;
            }
            let start = module
                .load_bias()
                .checked_add(segment.vaddr)
                .ok_or(NativeSessionError::SourceWindow)?;
            let end = start
                .checked_add(segment.file_bytes.len() as u64)
                .ok_or(NativeSessionError::SourceWindow)?;
            if start <= pc && pc < end {
                if found.is_some() {
                    return Err(NativeSessionError::SourceWindow);
                }
                let len = (end - pc).min(maximum);
                if len == 0 {
                    return Err(NativeSessionError::Budget("source window"));
                }
                found = Some((index, len));
            }
        }
    }
    found.ok_or(NativeSessionError::SourceWindow)
}

fn fetch_window_into<'window>(
    memory: &PausedNativeMemory<'_>,
    address: u64,
    length: u64,
    allocation_limit: u64,
    storage: &'window mut Vec<u8>,
) -> Result<&'window [u8], NativeSessionError> {
    if length == 0 || length > allocation_limit || storage.capacity() as u64 > allocation_limit {
        return Err(NativeSessionError::Budget("window allocation"));
    }
    let length =
        usize::try_from(length).map_err(|_| NativeSessionError::Budget("window allocation"))?;
    memory.preflight_fetch_source(address, length as u64)?;
    if storage.capacity() < length {
        // Vec reservation is relative to length, including after a shorter
        // previous fetch. The capacity check above implies len < length here.
        storage
            .try_reserve_exact(length - storage.len())
            .map_err(|_| NativeSessionError::Allocation)?;
    }
    if storage.capacity() as u64 > allocation_limit {
        return Err(NativeSessionError::Budget("window allocation"));
    }
    storage.resize(length, 0);
    // Never return bytes from the previous attempt, including on a plan hit.
    memory.fetch_source(address, storage)?;
    Ok(storage.as_slice())
}

#[cfg(test)]
mod netctl_tests;
#[cfg(test)]
#[path = "native_session/segment_tests.rs"]
mod segment_tests;
#[cfg(test)]
mod thread_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{
        NATIVE_GUEST_PAGE, NativeAddressSpace, NativeMemoryLimits, NativePermissions,
    };

    #[test]
    fn admission_budget_is_checked_without_changing_usage_on_refusal() {
        let mut usage = 7;
        assert!(matches!(
            charge(&mut usage, 4, 10, "work"),
            Err(NativeSessionError::Budget("work"))
        ));
        assert_eq!(usage, 7);
        usage = u64::MAX;
        assert!(charge(&mut usage, 1, u64::MAX, "work").is_err());
        assert_eq!(usage, u64::MAX);
    }

    #[test]
    fn source_window_fetch_requires_actual_executable_backing_and_bounded_allocation() {
        let mut storage = Vec::new();
        let mut owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10000).unwrap();
        let address = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(address, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        owner.pause().unwrap().write(address, b"original").unwrap();
        assert!(matches!(
            fetch_window_into(&owner.pause().unwrap(), address, 8, 8, &mut storage),
            Err(NativeSessionError::Memory(
                NativeMemoryError::PermissionDenied(_)
            ))
        ));
        assert_eq!(storage.capacity(), 0);
        owner
            .protect(address, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            fetch_window_into(&owner.pause().unwrap(), address, 8, 8, &mut storage).unwrap(),
            b"original"
        );
        assert!(matches!(
            fetch_window_into(&owner.pause().unwrap(), address, 8, 7, &mut storage),
            Err(NativeSessionError::Budget("window allocation"))
        ));
        assert!(matches!(
            fetch_window_into(&owner.pause().unwrap(), address, 0, 8, &mut storage),
            Err(NativeSessionError::Budget("window allocation"))
        ));
        assert!(matches!(
            fetch_window_into(
                &owner.pause().unwrap(),
                address + NATIVE_GUEST_PAGE - 1,
                2,
                8,
                &mut storage,
            ),
            Err(NativeSessionError::Memory(NativeMemoryError::Unmapped(_)))
        ));
        // A smaller request cannot silently retain allocation above its limit.
        assert!(matches!(
            fetch_window_into(&owner.pause().unwrap(), address, 4, 4, &mut storage),
            Err(NativeSessionError::Budget("window allocation"))
        ));
    }

    #[test]
    fn source_window_storage_reuses_allocation_but_refetches_short_and_changed_full_windows() {
        let mut owner =
            NativeAddressSpace::reserve_any(NativeMemoryLimits::default(), 0x10000).unwrap();
        let address = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(address, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let original: [u8; 64] = std::array::from_fn(|index| index as u8);
        owner.pause().unwrap().write(address, &original).unwrap();
        owner
            .protect(address, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        let mut storage = Vec::new();
        assert_eq!(
            fetch_window_into(&owner.pause().unwrap(), address, 64, 64, &mut storage).unwrap(),
            &original
        );
        let allocation = storage.as_ptr();
        let capacity = storage.capacity();
        assert_eq!(
            fetch_window_into(&owner.pause().unwrap(), address + 7, 5, 64, &mut storage).unwrap(),
            &original[7..12]
        );
        assert_eq!(storage.len(), 5);
        assert_eq!(storage.as_ptr(), allocation);
        assert_eq!(storage.capacity(), capacity);

        owner
            .protect(address, NATIVE_GUEST_PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        let changed: [u8; 64] = std::array::from_fn(|index| 255 - index as u8);
        owner.pause().unwrap().write(address, &changed).unwrap();
        assert!(matches!(
            fetch_window_into(&owner.pause().unwrap(), address, 64, 64, &mut storage),
            Err(NativeSessionError::Memory(
                NativeMemoryError::PermissionDenied(_)
            ))
        ));
        owner
            .protect(address, NATIVE_GUEST_PAGE, NativePermissions::READ_EXECUTE)
            .unwrap();
        assert_eq!(
            fetch_window_into(&owner.pause().unwrap(), address, 64, 64, &mut storage).unwrap(),
            &changed
        );
        assert_eq!(storage.len(), 64);
        assert_eq!(storage.as_ptr(), allocation);
        assert_eq!(storage.capacity(), capacity);
    }
}
