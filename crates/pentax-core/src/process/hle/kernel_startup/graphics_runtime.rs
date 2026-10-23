// SPDX-License-Identifier: GPL-2.0-only

//! Exact factory/source binding and live PRE-SYSCALL graphics admission.
use super::{
    KernelStartupError, KernelStartupLimits, NativeServiceAction, NativeServiceRequest,
    PreparedNativeKernelStartup, UnavailableProviderPlacement, charge, require,
};
use crate::{
    gpu::driver,
    memory::{ADDRESS_LIMIT, PAGE_SIZE},
    process::hle::{
        GeneratedGraphicsProvider, HleError, ProviderPlacement, graphics_provider::GraphicsMarker,
    },
};
use pentax_linker::{ModuleInput, Resolution, SymbolicLinkPlan};
use pentax_loader::open_executable;
use pentax_platform::native_memory::{NativeMemoryError, PausedNativeMemory};

#[derive(Clone, Copy, Debug)]
pub struct GraphicsStartupOptions<'image> {
    pub provider: &'image GeneratedGraphicsProvider,
    pub placement: ProviderPlacement,
    pub limits: driver::Limits,
    pub unavailable: Option<UnavailableProviderPlacement<'image>>,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct GraphicsBinding<'image> {
    pub factory: &'image GeneratedGraphicsProvider,
    pub index: usize,
    pub bias: u64,
    pub limits: driver::Limits,
}
impl GraphicsBinding<'_> {
    pub fn contains(&self, pc: u64) -> bool {
        pc.checked_sub(self.bias).is_some_and(|rva| rva < PAGE_SIZE)
    }
}
pub(super) fn append<'image>(
    inputs: &mut Vec<ModuleInput<'image>>,
    options: GraphicsStartupOptions<'image>,
) -> Result<GraphicsBinding<'image>, KernelStartupError> {
    let bias = options.placement.load_bias;
    require(
        bias.is_multiple_of(PAGE_SIZE)
            && bias
                .checked_add(PAGE_SIZE)
                .is_some_and(|v| v <= ADDRESS_LIMIT),
        "graphics provider placement",
    )?;
    let index = inputs.len();
    let view = open_executable(options.provider.image())
        .map_err(|e| KernelStartupError::Hle(HleError::Load(e)))?;
    inputs.push(ModuleInput::new(view, bias));
    Ok(GraphicsBinding {
        factory: options.provider,
        index,
        bias,
        limits: options.limits,
    })
}
pub(super) fn validate(
    plan: &SymbolicLinkPlan<'_>,
    binding: &GraphicsBinding<'_>,
    limits: &KernelStartupLimits,
    work: &mut u64,
) -> Result<(), KernelStartupError> {
    let cost = (plan.patch_count() as u64)
        .checked_add(4)
        .ok_or(KernelStartupError::Budget("graphics provider work"))?;
    charge(work, cost, limits.max_provider_work)?;
    let module = &plan.modules()[binding.index];
    let runtime = module.runtime_plan();
    require(
        module.load_bias() == binding.bias
            && runtime.segments.len() == 1
            && runtime.tls.is_none()
            && runtime.relro.is_empty(),
        "graphics module mapping",
    )?;
    let segment = &runtime.segments[0];
    require(
        std::ptr::eq(segment.file_bytes, binding.factory.image())
            && segment.vaddr == 0
            && segment.mem_size == PAGE_SIZE
            && segment.flags.raw() == 5,
        "graphics factory source identity",
    )?;
    for patch in plan.patches() {
        require(
            patch.consumer().index() != binding.index,
            "graphics provider relocation",
        )?;
        let Resolution::Definition(symbol) = patch.resolution() else {
            continue;
        };
        if symbol.module.index() != binding.index {
            continue;
        }
        charge(
            work,
            binding.factory.function_count() as u64,
            limits.max_provider_work,
        )?;
        let record = patch.source().record;
        let addend = if record.kind() == 1 { record.addend } else { 0 };
        require(
            binding.factory.markers().any(|m| {
                m.symbol == symbol.symbol
                    && (binding.bias + m.rva()).wrapping_add_signed(addend) == patch.value()
            }),
            "graphics binding outside manifest",
        )?;
    }
    Ok(())
}
fn attest_marker(
    memory: &PausedNativeMemory<'_>,
    binding: &GraphicsBinding<'_>,
    marker: GraphicsMarker,
) -> Result<(), KernelStartupError> {
    let expected = marker.bytes();
    let at = marker.rva() as usize;
    require(
        binding.factory.image().get(at..at + 8) == Some(&expected),
        "graphics source marker",
    )?;
    let mut actual = [0; 8];
    let address = binding.bias + marker.rva();
    require(
        matches!(
            memory.preflight_write(address, 8),
            Err(NativeMemoryError::PermissionDenied(_))
        ),
        "graphics marker immutable permission",
    )?;
    memory
        .fetch_source(address, &mut actual)
        .map_err(|_| KernelStartupError::InvalidProvider("graphics marker fetch permission"))?;
    require(actual == expected, "graphics live marker bytes")
}
pub(super) fn attest_all(
    memory: &PausedNativeMemory<'_>,
    binding: &GraphicsBinding<'_>,
    work: &mut u64,
    maximum: u64,
) -> Result<(), KernelStartupError> {
    charge(
        work,
        binding.factory.function_count() as u64 * 8 + 1,
        maximum,
    )?;
    for marker in binding.factory.markers() {
        attest_marker(memory, binding, marker)?;
    }
    Ok(())
}
pub(super) fn stop(
    session: &mut PreparedNativeKernelStartup<'_, '_>,
    source: &SymbolicLinkPlan<'_>,
    request: NativeServiceRequest,
    actions: bool,
) -> Result<NativeServiceAction, KernelStartupError> {
    let binding = session
        .plan
        .graphics
        .ok_or(KernelStartupError::InvalidProvider(
            "missing graphics provider",
        ))?;
    charge(
        &mut session.provider_work,
        binding.factory.function_count() as u64 + 8,
        session.plan.limits.max_provider_work,
    )?;
    let marker = binding
        .factory
        .markers()
        .find(|m| binding.bias + m.rva() + 5 == request.syscall_pc)
        .ok_or(KernelStartupError::SiteMismatch {
            syscall_pc: request.syscall_pc,
        })?;
    if request.service_id != u64::from(marker.id()) {
        return Err(KernelStartupError::ServiceIdMismatch {
            expected: marker.id(),
            actual: request.service_id,
        });
    }
    let thread_pointer = session
        .tls()
        .instances()
        .get(session.active_tls)
        .map(|tls| tls.thread_pointer);
    let mut paused = session.hle.pause().map_err(KernelStartupError::Hle)?;
    attest_marker(&paused, &binding, marker)?;
    let driver = session
        .graphics
        .as_mut()
        .ok_or(KernelStartupError::InvalidProvider(
            "missing graphics state",
        ))?;
    if driver::is_command_builder(marker.function) {
        let action = driver
            .begin_command_builder(
                &mut paused,
                marker.function,
                request.args,
                request.stack_pointer,
            )
            .map_err(KernelStartupError::Graphics)?;
        if !actions && matches!(action, driver::CommandBuilderAction::Grow(_)) {
            return Err(KernelStartupError::InvalidProvider(
                "AGC callback needs native continuation",
            ));
        }
        return session.agc_callbacks.prepare(
            &mut paused,
            source,
            action,
            request,
            (thread_pointer, session.active_tls),
            (
                &mut session.provider_work,
                session.plan.limits.max_provider_work,
            ),
        );
    }
    if marker.function == driver::Function::VideoColorSettingsSetGamma {
        driver.submit_video_gamma(&mut paused, request.args, request.vector_args)
    } else if matches!(
        marker.function,
        driver::Function::VideoOpen
            | driver::Function::VideoClose
            | driver::Function::VideoConfigureOutput
            | driver::Function::VideoGetVblankStatus
    ) {
        driver.submit_video_clocked(
            &mut paused,
            marker.function,
            request.args,
            &mut session.kernel,
            display_clock,
        )
    } else if marker.function == driver::Function::VideoSubmitFlip {
        driver.submit_video_flip_with_clock(&mut paused, request.args, |paused| {
            let guest = super::super::kernel_adapter::KernelMemory {
                paused,
                source: None,
                supplied_modules: 0,
                lookup_limits: None,
                lookup_work: None,
                loaded_modules: None,
                thread_tls: None,
                initial_entry: None,
            };
            session
                .kernel
                .process_time_counter(&guest)
                .map_err(driver::Failure::Kernel)
        })
    } else if matches!(
        marker.function,
        driver::Function::AgcGetRegisterDefaults | driver::Function::AgcGetRegisterDefaultsInternal
    ) {
        let owner = paused.owner_id();
        let mut guest = super::super::kernel_adapter::KernelMemory {
            paused: &mut paused,
            source: None,
            supplied_modules: 0,
            lookup_limits: None,
            lookup_work: None,
            loaded_modules: None,
            thread_tls: None,
            initial_entry: None,
        };
        driver.register_defaults(
            owner,
            &mut session.kernel,
            &mut guest,
            marker.function == driver::Function::AgcGetRegisterDefaultsInternal,
            request.args[0],
        )
    } else if matches!(
        marker.function,
        driver::Function::CommandBuffer | driver::Function::MultiCommandBuffers
    ) {
        driver.submit_with_kernel_clock(
            &mut paused,
            marker.function,
            request.args,
            request.stack_pointer,
            &mut session.kernel,
            |kernel, paused| {
                let guest = super::super::kernel_adapter::KernelMemory {
                    paused,
                    source: None,
                    supplied_modules: 0,
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: None,
                    initial_entry: None,
                };
                kernel
                    .process_time_counter(&guest)
                    .map_err(driver::Failure::Kernel)
            },
        )
    } else {
        driver.submit_with_kernel(
            &mut paused,
            marker.function,
            request.args,
            request.stack_pointer,
            &mut session.kernel,
        )
    }
    .map_err(KernelStartupError::Graphics)
    .map(NativeServiceAction::Returned)
}
fn display_clock(
    kernel: &mut pentax_kernel::Kernel,
    paused: &mut pentax_platform::native_memory::PausedNativeMemory<'_>,
) -> Result<pentax_kernel::DisplayClock, driver::Failure> {
    let guest = super::super::kernel_adapter::KernelMemory {
        paused,
        source: None,
        supplied_modules: 0,
        lookup_limits: None,
        lookup_work: None,
        loaded_modules: None,
        thread_tls: None,
        initial_entry: None,
    };
    kernel
        .display_clock(&guest)
        .map_err(driver::Failure::Kernel)
}

impl PreparedNativeKernelStartup<'_, '_> {
    /// Final diagnostics: open ports, committed buffer registrations and retained
    /// vblank subscriptions for those exact live port generations. Registration
    /// is not live pixel permission, notification readiness or display proof.
    pub fn video_registration_counts(&self) -> (u64, u64, u64) {
        let Some(driver) = self.graphics.as_ref() else {
            return (0, 0, 0);
        };
        let mut ports = 0;
        let mut buffers = 0;
        let mut subscriptions = 0;
        for handle in 1..=3 {
            if let Some(port) = driver.video_port(handle) {
                ports += 1;
                for buffer in 0..16 {
                    buffers += u64::from(driver.video_surface(handle, buffer).is_some());
                }
                subscriptions += self
                    .kernel
                    .video_vblank_subscription_count(handle, port.generation);
            }
        }
        (ports, buffers, subscriptions)
    }

    pub fn graphics_usage(&self) -> driver::Usage {
        self.graphics
            .as_ref()
            .map_or_else(driver::Usage::default, driver::Driver::usage)
    }
    /// Historical admitted API calls only, not in-flight or successful work.
    pub fn graphics_function_profile(&self) -> driver::FunctionProfile {
        self.graphics.as_ref().map_or_else(
            driver::FunctionProfile::default,
            driver::Driver::function_profile,
        )
    }
    /// Current owner-retained flip phases only; no guest access or readiness
    /// probe. An absent graphics driver has no requests or acknowledgements.
    pub fn graphics_video_flip_observations(&self) -> crate::gpu::video::FlipObservations {
        self.graphics.as_ref().map_or_else(
            crate::gpu::video::FlipObservations::default,
            driver::Driver::video_flip_observations,
        )
    }
    pub fn graphics_frame_state(&self) -> Option<driver::FrameState> {
        self.graphics.as_ref().map(driver::Driver::frame_state)
    }
    pub fn graphics_video_port(&self, handle: i32) -> Option<crate::gpu::video::PortState> {
        self.graphics
            .as_ref()
            .and_then(|driver| driver.video_port(handle))
    }
    /// Registered geometry for this session; consumers still need live backing
    /// admission before accessing color or metadata bytes.
    pub fn graphics_video_surface(
        &self,
        handle: i32,
        buffer: i32,
    ) -> Option<crate::gpu::video::Surface> {
        self.graphics
            .as_ref()
            .and_then(|driver| driver.video_surface(handle, buffer))
    }

    /// Inspect flip metadata through this session's actual paused memory owner.
    /// This does not bind a fresh queue or grant access to surface bytes.
    pub fn graphics_video_flip_status(
        &mut self,
        handle: i32,
    ) -> Result<Option<crate::gpu::video::FlipStatus>, KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        self.graphics
            .as_ref()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?
            .video_flip_status(&paused, handle)
            .map_err(KernelStartupError::Graphics)
    }

    /// Whether a registered buffer is retained by an unfinished flip.
    pub fn graphics_video_buffer_pending(
        &mut self,
        handle: i32,
        buffer: i32,
    ) -> Result<bool, KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        self.graphics
            .as_ref()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?
            .video_buffer_pending(&paused, handle, buffer)
            .map_err(KernelStartupError::Graphics)
    }

    /// Hand off real captured pixels at a caller-observed nonzero vblank tick.
    /// The transport must bound retained frame capacities independently. Taking
    /// a frame does not confirm presentation or deliver a kernel event.
    pub fn graphics_video_take_frame(
        &mut self,
        vblank_count: u64,
    ) -> Result<Option<crate::gpu::video::Frame>, KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        self.graphics
            .as_mut()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?
            .video_take_frame(&paused, vblank_count)
            .map_err(KernelStartupError::Graphics)
    }

    /// Read-only oldest READY metadata from the actual paused owner. A later
    /// ready frame cannot bypass an older request; no tick, dequeue or ack.
    pub fn graphics_video_peek_ready_frame(
        &mut self,
    ) -> Result<Option<crate::gpu::video::ReadyFrameMetadata>, KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        self.graphics.as_ref().map_or(Ok(None), |driver| {
            driver
                .video_peek_ready_frame(&paused)
                .map_err(KernelStartupError::Graphics)
        })
    }

    /// Tighten live capture policy to include transport-held allocations. The
    /// caller accounts actual capacities and keeps handed-off Frames charged
    /// until dropped, including after by-reference acknowledgement.
    /// Returns only the current queued-frame capacity census for a non-guest
    /// transport pump's allocation admission; this is not frame authority.
    pub fn graphics_video_consumer_admit(
        &mut self,
        max_frame_bytes: usize,
        max_staging_bytes: usize,
        external_held: usize,
    ) -> Result<usize, KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        self.graphics
            .as_mut()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?
            .video_consumer_admit(&paused, max_frame_bytes, max_staging_bytes, external_held)
            .map_err(KernelStartupError::Graphics)
    }

    /// Deliver the actual acknowledged flip to retained VideoOut subscriptions.
    /// Failure preserves the same opaque completion; success consumes it once.
    pub fn graphics_video_deliver_completion(
        &mut self,
        completion: &mut Option<crate::gpu::video::FlipCompletion>,
    ) -> Result<(), KernelStartupError> {
        let paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        let driver = self
            .graphics
            .as_mut()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?;
        let receipt = completion
            .as_ref()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing presentation completion",
            ))?;
        if receipt.owner() != paused.owner_id()
            || driver
                .video_port(receipt.handle())
                .is_none_or(|p| p.generation != receipt.generation())
        {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        let tsc = pentax_platform::time_counter::sample()
            .map_err(|_| KernelStartupError::InvalidProvider("video event counter"))?
            .ticks;
        driver
            .video_deliver_completion(&paused, &mut self.kernel, completion, tsc)
            .map_err(KernelStartupError::Graphics)
    }

    /// Only the actual window consumer supplies observed ticks. Numeric tick
    /// metadata never acknowledges a Frame or signals an AGC interrupt.
    pub fn graphics_video_observe_vblank(
        &mut self,
        handle: i32,
        generation: u64,
        tick: u64,
    ) -> Result<(), KernelStartupError> {
        let mut paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        let driver = self
            .graphics
            .as_mut()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?;
        if driver
            .video_port(handle)
            .is_none_or(|p| p.generation != generation)
        {
            return Err(KernelStartupError::NativeOwnerMismatch);
        }
        driver
            .video_preflight_vblank(&paused, handle, generation, tick)
            .map_err(KernelStartupError::Graphics)?;
        let clock =
            display_clock(&mut self.kernel, &mut paused).map_err(KernelStartupError::Graphics)?;
        driver
            .video_observe_vblank(
                &mut paused,
                &mut self.kernel,
                handle,
                generation,
                tick,
                clock,
            )
            .map_err(KernelStartupError::Graphics)
    }

    /// Paused scheduling and queue waits use this same grid even before any
    /// pixels or window configuration exist. Returned delay is a future display
    /// boundary, never a guest wait completion or a presentation acknowledgement.
    pub fn graphics_video_poll_vblank(
        &mut self,
    ) -> Result<Option<std::time::Duration>, KernelStartupError> {
        let Some(driver) = self.graphics.as_mut() else {
            return Ok(None);
        };
        let mut paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        driver
            .video_poll_clocked(&mut paused, &mut self.kernel, display_clock)
            .map_err(KernelStartupError::Graphics)
    }

    /// Confirm actual consumer presentation once using this session's opaque
    /// frame. Exact owner, phase and generation admission precedes clock reads.
    /// Clock fields share the kernel services' process epoch; this adds no HLE
    /// call or quota after presentation. The returned receipt still requires
    /// separate real equeue delivery; this method creates neither UI nor IPC.
    pub fn graphics_video_acknowledge_frame(
        &mut self,
        frame: &crate::gpu::video::Frame,
    ) -> Result<crate::gpu::video::FlipCompletion, KernelStartupError> {
        let mut paused = self.hle.pause().map_err(KernelStartupError::Hle)?;
        let driver = self
            .graphics
            .as_mut()
            .ok_or(KernelStartupError::InvalidProvider(
                "missing graphics state",
            ))?;
        driver
            .video_acknowledge_frame(&mut paused, frame, |paused| {
                let guest = super::super::kernel_adapter::KernelMemory {
                    paused,
                    source: None,
                    supplied_modules: 0,
                    lookup_limits: None,
                    lookup_work: None,
                    loaded_modules: None,
                    thread_tls: None,
                    initial_entry: None,
                };
                let clock = self
                    .kernel
                    .process_clock(&guest)
                    .map_err(driver::Failure::Kernel)?;
                Ok((clock.microseconds, clock.counter))
            })
            .map_err(KernelStartupError::Graphics)
    }

    /// Retained guest-address metadata, not a memory or executable capability.
    pub fn graphics_shader_metadata(
        &self,
        address: u64,
    ) -> Option<crate::gpu::shaders::MappedShader> {
        self.graphics
            .as_ref()
            .and_then(|driver| driver.shader_mapping(self.owner_id(), address))
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod defaults_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod interrupt_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod video_event_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use crate::process::hle::{
        NativeImageLimits, unavailable::tests::original_input, with_native_image_session,
    };
    use pentax_platform::native_memory::{
        NativeAddressSpace, NativeMemoryEffect, NativeMemoryTransaction, NativePermissions,
    };
    #[test]
    fn graphics_imports_keep_exact_namespaces_and_version_conflicts() {
        for (module, library, version, nid, missing) in [
            ("Graphics5Driver", "Graphics5Driver", 1, "b4fpgH5ZXxQ", 0),
            ("libkernel", "libkernel", 1, "b4fpgH5ZXxQ", 1),
            ("Graphics5Driver", "Graphics5Driver", 2, "b4fpgH5ZXxQ", 0),
            ("Graphics5", "Graphics5", 1, "h9z6+0hEydk", 0),
            ("libSceAgc", "libSceAgc", 1, "h9z6+0hEydk", 0),
            ("libSceAgc", "libSceAgc", 2, "h9z6+0hEydk", 0),
            ("Graphics5Driver", "Graphics5Driver", 1, "h9z6+0hEydk", 1),
            ("Graphics5", "Graphics5", 1, "b4fpgH5ZXxQ", 1),
            ("Graphics5", "Graphics5", 1, "23LRUSvYu1M", 0),
            ("libSceAgc", "libSceAgc", 1, "23LRUSvYu1M", 0),
            ("Graphics5", "Graphics5", 2, "23LRUSvYu1M", 0),
            ("Graphics5Driver", "Graphics5Driver", 1, "23LRUSvYu1M", 1),
            ("Graphics5", "Graphics5", 1, "2JtWUUiYBXs", 0),
            ("libSceAgc", "libSceAgc", 1, "wRbq6ZjNop4", 0),
            ("Graphics5", "Graphics5", 2, "wRbq6ZjNop4", 0),
            ("Graphics5Driver", "Graphics5Driver", 1, "2JtWUUiYBXs", 1),
            ("Graphics5", "Graphics5", 1, "BfBDZGbti7A", 0),
            ("libSceAgc", "libSceAgc", 1, "BfBDZGbti7A", 0),
            ("Graphics5", "Graphics5", 2, "BfBDZGbti7A", 0),
            ("Graphics5", "Graphics5", 1, "X-Nm5KLREeg", 0),
            ("libSceAgc", "libSceAgc", 1, "pWLG7WOpVcw", 0),
            ("Graphics5", "Graphics5", 2, "W5z4eZrjEas", 0),
            ("Graphics5Driver", "Graphics5Driver", 1, "X-Nm5KLREeg", 1),
            ("AgcDriver", "AgcDriver", 1, "X-Nm5KLREeg", 0),
            ("libSceAgcDriver", "libSceAgcDriver", 1, "W5z4eZrjEas", 0),
            ("libSceAgcDriver", "libSceAgcDriver", 2, "X-Nm5KLREeg", 0),
            ("Graphics5", "Graphics5", 1, "f3dg2CSgRKY", 0),
            ("libSceAgc", "libSceAgc", 1, "f3dg2CSgRKY", 0),
            ("libSceAgc", "libSceAgc", 2, "f3dg2CSgRKY", 0),
            ("Graphics5Driver", "Graphics5Driver", 1, "f3dg2CSgRKY", 1),
            ("Graphics5", "Graphics5", 1, "YUeqkyT7mEQ", 0),
            ("libSceAgc", "libSceAgc", 1, "YUeqkyT7mEQ", 0),
            ("Graphics5", "Graphics5", 2, "YUeqkyT7mEQ", 0),
            ("Graphics5", "libSceAgc", 1, "YUeqkyT7mEQ", 1),
            ("Graphics5Driver", "Graphics5Driver", 1, "YUeqkyT7mEQ", 1),
            ("Graphics5", "Graphics5", 1, "k3GhuSNmBLU", 0),
            ("libSceAgc", "libSceAgc", 1, "k3GhuSNmBLU", 0),
            ("Graphics5", "libSceAgc", 1, "k3GhuSNmBLU", 1),
            ("libSceAgc", "Graphics5", 1, "k3GhuSNmBLU", 1),
            ("Graphics5", "Graphics5", 2, "k3GhuSNmBLU", 0),
        ] {
            let mut input = original_input(1);
            let strings = u64::from_le_bytes(input[0x208..0x210].try_into().unwrap()) as usize;
            input[strings + 1..strings + 17].fill(0);
            input[strings + 1..strings + 1 + module.len()].copy_from_slice(module.as_bytes());
            input[strings + 17..strings + 34].fill(0);
            input[strings + 17..strings + 17 + library.len()].copy_from_slice(library.as_bytes());
            let name = u32::from_le_bytes(input[0x418..0x41c].try_into().unwrap()) as usize;
            input[strings + name..strings + name + 11].copy_from_slice(nid.as_bytes());
            input[0x258..0x260]
                .copy_from_slice(&(1_u64 | (1 << 32) | (1 << 40) | (2 << 48)).to_le_bytes());
            input[0x268..0x270]
                .copy_from_slice(&(17_u64 | (version << 32) | (3 << 48)).to_le_bytes());
            let result = with_native_image_session(
                &[&input],
                &NativeImageLimits {
                    stack_bytes: PAGE_SIZE,
                    kernel_window_bytes: 65536,
                    ..Default::default()
                },
                |session, layout| {
                    assert_eq!(layout.unavailable_functions, missing);
                    assert_eq!(session.graphics_usage(), driver::Usage::default());
                    Ok::<_, ()>(())
                },
            );
            if version == 2 {
                assert_eq!(result.unwrap_err().code(), "import-version-mismatch");
            } else {
                result.unwrap();
            }
        }
    }
    #[test]
    fn video_requests_retain_same_owner_ports_and_refuse_forged_calls_before_effects() {
        let input = original_input(0);
        with_native_image_session(&[&input], &NativeImageLimits::default(), |session, _| {
            let binding = session.plan.graphics.unwrap();
            assert_eq!(binding.factory.function_count(), 251);
            let open = binding
                .factory
                .markers()
                .find(|m| m.function == driver::Function::VideoOpen)
                .unwrap();
            let close = binding
                .factory
                .markers()
                .find(|m| m.function == driver::Function::VideoClose)
                .unwrap();
            let request = NativeServiceRequest {
                stack_pointer: None,
                vector_args: None,
                owner_id: session.owner_id(),
                syscall_pc: binding.bias + open.rva() + 5,
                next_pc: binding.bias + open.rva() + 7,
                service_id: u64::from(open.id()),
                args: [255, 0, 0, u64::MAX, 0, 0],
            };
            let kernel_usage = session.kernel_usage();
            assert_eq!(session.graphics_video_flip_status(1).unwrap(), None);
            assert_eq!(
                session.graphics_video_buffer_pending(1, 0),
                Err(KernelStartupError::Graphics(
                    driver::Failure::UnsupportedVideo
                ))
            );
            assert!(session.graphics_video_take_frame(0).unwrap().is_none());
            assert_eq!(session.graphics_video_port(1), None);
            assert_eq!(session.graphics_usage().calls, 0);
            assert_eq!(session.kernel_usage(), kernel_usage);
            let other = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
            let mut bad = request;
            bad.owner_id = other.owner_id();
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::NativeOwnerMismatch)
            ));
            assert_eq!(session.graphics_video_port(1), None);
            bad = request;
            bad.service_id |= 1 << 32;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            assert_eq!(session.graphics_video_port(1), None);
            assert_eq!(session.service_request(request).unwrap(), 1);
            assert_eq!(session.graphics_video_port(1).unwrap().generation, 1);
            assert_eq!(
                session.graphics_video_flip_status(1).unwrap(),
                Some(Default::default())
            );
            assert!(!session.graphics_video_buffer_pending(1, 0).unwrap());
            assert!(session.graphics_video_take_frame(1).unwrap().is_none());
            assert_eq!(session.graphics_usage().calls, 1);
            assert_eq!(session.kernel_usage(), kernel_usage);
            let close_request = NativeServiceRequest {
                syscall_pc: binding.bias + close.rva() + 5,
                next_pc: binding.bias + close.rva() + 7,
                service_id: u64::from(close.id()),
                args: [1, 0, 0, 0, 0, 0],
                ..request
            };
            assert_eq!(session.service_request(close_request).unwrap(), 0);
            assert_eq!(session.graphics_video_port(1), None);
            assert_eq!(session.graphics_video_flip_status(1).unwrap(), None);
            assert_eq!(
                session.graphics_video_buffer_pending(1, 0),
                Err(KernelStartupError::Graphics(
                    driver::Failure::UnsupportedVideo
                ))
            );
            assert_eq!(session.service_request(request).unwrap(), 1);
            assert_eq!(session.graphics_video_port(1).unwrap().generation, 3);
            assert_eq!(session.graphics_frame_state().unwrap().boundaries, 0);
            assert_eq!(session.graphics_usage().dispatches, 0);
            Ok::<_, ()>(())
        })
        .unwrap();
    }

    #[test]
    fn graphics_requests_attest_owner_site_full_id_permissions_and_budgets_before_gpu() {
        let input = original_input(0);
        let limits = NativeImageLimits {
            stack_bytes: PAGE_SIZE,
            kernel_window_bytes: 65536,
            graphics: driver::Limits {
                max_calls: 10,
                max_batch_buffers: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        with_native_image_session(&[&input], &limits, |session, layout| {
            let binding = session.plan.graphics.unwrap();
            // This case exercises the original four submit/two boundary APIs;
            // video ports have separate stateful request coverage below.
            let markers: Vec<_> = binding.factory.markers().take(6).collect();
            let request = |marker: GraphicsMarker, args| NativeServiceRequest {
                stack_pointer: None,
                vector_args: None,
                owner_id: session.owner_id(),
                syscall_pc: binding.bias + marker.rva() + 5,
                next_pc: binding.bias + marker.rva() + 7,
                service_id: u64::from(marker.id()),
                args,
            };
            let requests: Vec<_> = markers.iter().map(|m| request(*m, [0; 6])).collect();
            let mut bad = requests[0];
            bad.service_id |= 1 << 32;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ServiceIdMismatch { .. })
            ));
            bad = requests[0];
            bad.next_pc += 1;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::ContinuationMismatch { .. })
            ));
            bad = requests[0];
            bad.syscall_pc += 1;
            bad.next_pc += 1;
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::SiteMismatch { .. })
            ));
            let other = NativeAddressSpace::reserve_any(Default::default(), 65536).unwrap();
            bad = requests[0];
            bad.owner_id = other.owner_id();
            assert!(matches!(
                session.service_request(bad),
                Err(KernelStartupError::NativeOwnerMismatch)
            ));
            for (i, request) in requests.iter().enumerate() {
                assert_eq!(
                    session.service_request(*request).unwrap(),
                    if i == 0 { 0x8002_0016 } else { 0 }
                );
            }
            assert_eq!(session.graphics_frame_state().unwrap().boundaries, 2);
            let mut bad = requests[2];
            bad.args = [0x20, 1, 0, 0, 0, 0];
            assert_eq!(
                session.service_request(bad),
                Err(KernelStartupError::Graphics(driver::Failure::Memory))
            );
            let base = layout.entry.stack.address + 0x100;
            let mut packet = [0; 16];
            packet[..8].copy_from_slice(&(base + 64).to_le_bytes());
            packet[8..12].copy_from_slice(&1_u32.to_le_bytes());
            {
                let mut paused = session.pause().unwrap();
                // The packet fixture is CPU data, while its command payload
                // requires an explicitly admitted GPU-readable backing page.
                paused
                    .apply_transaction(
                        NativeMemoryTransaction {
                            effect: NativeMemoryEffect::Protect {
                                address: layout.entry.stack.address,
                                size: PAGE_SIZE,
                                permissions: NativePermissions::READ_WRITE.with_gpu(true, true),
                            },
                            writes: &[],
                        },
                        &Default::default(),
                    )
                    .unwrap();
                paused.write(base, &packet).unwrap();
                paused
                    .write(base + 64, &0x8000_0000_u32.to_le_bytes())
                    .unwrap();
            }
            bad = requests[2];
            bad.args = [0x1f, base, 0, 0, 0, 0];
            assert_eq!(
                session.service_request(bad),
                Err(KernelStartupError::Graphics(
                    driver::Failure::UnsupportedQueue
                ))
            );
            bad = requests[3];
            bad.args = [0x20, 1, 1, 3, 0, 0];
            assert_eq!(
                session.service_request(bad),
                Err(KernelStartupError::Graphics(driver::Failure::Limit))
            );
            // Live writable code cannot gain authority from its original bytes.
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::new(true, true, true),
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            assert!(matches!(
                session.service_request(requests[2]),
                Err(KernelStartupError::InvalidProvider(_))
            ));
            session
                .pause()
                .unwrap()
                .apply_transaction(
                    NativeMemoryTransaction {
                        effect: NativeMemoryEffect::Protect {
                            address: binding.bias,
                            size: PAGE_SIZE,
                            permissions: NativePermissions::new(true, false, true),
                        },
                        writes: &[],
                    },
                    &Default::default(),
                )
                .unwrap();
            assert_eq!(session.service_request(requests[2]).unwrap(), 0);
            assert_eq!(
                session.service_request(requests[2]),
                Err(KernelStartupError::Graphics(driver::Failure::Limit))
            );
            assert_eq!(session.graphics_usage().calls, 10);
            assert_eq!(session.graphics_usage().dispatches, 0);
            Ok::<_, ()>(())
        })
        .unwrap();
    }
}
