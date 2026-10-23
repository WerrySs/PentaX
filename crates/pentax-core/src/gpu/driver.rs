// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked guest submit ABI and cumulative admission.
// Adapted from pinned src/libs/agc.{h,cpp}; see docs/upstream-provenance.md.

//! Source-observed submit calls over the exact exclusively paused guest owner.
//! Queue contexts and packets are guest data, never host pointers or handles.
use super::{Error, execute_dispatch_admitted};
use pentax_gpu::compute;
use pentax_gpu::graphics as draw;
use pentax_platform::{
    native_memory::{NativeMemoryWrite, NativeRange, NativeTransactionLimits, PausedNativeMemory},
    vulkan::GpuLimits,
};

#[path = "driver/command_builder.rs"]
mod command_builder;
#[path = "driver/flip_encoder.rs"]
mod flip_encoder;
pub(crate) use command_builder::CONTEXT_EXPORTS;
pub(crate) use command_builder::EXPORTS as COMMAND_BUILDER_EXPORTS;
pub(crate) use command_builder::INDIRECT_DISPATCH_EXPORTS;
pub(crate) use command_builder::INDIRECT_DRAW_EXPORTS;
pub(crate) use command_builder::MARKER_EXPORTS;
pub(crate) use command_builder::RESET_EXPORTS as RESET_QUEUE_EXPORTS;
pub(crate) use command_builder::SYNC_EXPORTS as MEMORY_SYNC_EXPORTS;
pub(crate) use command_builder::TRANSFER_EXPORTS;
pub(crate) use command_builder::supports as is_command_builder;
pub(crate) use command_builder::{CommandBuilderAction, PendingCommandBuilder};
#[path = "driver/function_profile.rs"]
mod function_profile;
pub use function_profile::FunctionProfile;
#[path = "driver/graphics.rs"]
mod graphics;
pub(crate) use graphics::{GraphicsProgress, GraphicsTicket};
#[path = "driver/interrupts.rs"]
mod interrupts;
#[path = "driver/memory_transfer.rs"]
mod memory_transfer;
#[path = "driver/packet_metadata.rs"]
mod packet_metadata;
#[path = "driver/presentation.rs"]
mod presentation;
#[path = "driver/register_execution.rs"]
mod register_execution;
#[path = "driver/video_events.rs"]
mod video_events;
#[path = "driver/video_state.rs"]
mod video_state;
pub(crate) type GraphicsClock =
    fn(&mut pentax_kernel::Kernel, &mut PausedNativeMemory<'_>) -> Result<u64, Failure>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    CommandBuffer,
    MultiCommandBuffers,
    Acb,
    MultiAcbs,
    SuspendPoint,
    AgcInit,
    AgcCreateShader,
    AgcGetFusedShaderSize,
    AgcFuseShaderHalves,
    AgcFuseShaderHalvesLegacy,
    AgcCreateInterpolantMapping,
    AgcCreateInterpolantMapping2,
    AgcAddEqEvent,
    AgcDeleteEqEvent,
    AgcGetEqEventType,
    AgcGetEqContextId,
    AgcGetIsTrinityMode,
    AgcRegistryUnavailable,
    AgcGetRegisterDefaults,
    AgcGetRegisterDefaultsInternal,
    VideoOpen,
    VideoClose,
    VideoSetFlipMaster,
    VideoSetFlipRate,
    VideoInitializeOptions,
    VideoIsOutputSupported,
    VideoConfigureOutput,
    VideoGetOutputStatus,
    VideoSetBufferAttribute2,
    VideoAddBufferAttributeOption,
    VideoRegisterBuffers2,
    VideoChangeBufferAttribute2,
    VideoUnregisterBuffers,
    VideoSubmitFlip,
    VideoGetFlipStatus,
    VideoIsFlipPending,
    AgcDcbSetFlip,
    VideoAddFlipEvent,
    VideoDeleteFlipEvent,
    VideoAddVblankEvent,
    VideoDeleteVblankEvent,
    VideoGetEventId,
    VideoGetEventData,
    VideoGetEventCount,
    AgcCbNop,
    AgcCbNopGetSize,
    AgcCbDispatch,
    AgcCbDispatchGetSize,
    AgcDcbSetIndexSize,
    AgcDcbSetIndexBuffer,
    AgcDcbSetIndexCount,
    AgcDcbSetNumInstances,
    AgcDcbSetNumInstancesGetSize,
    AgcDcbDrawIndex,
    AgcDcbDrawIndexGetSize,
    AgcDcbDrawIndexAuto,
    AgcDcbDrawIndexAutoGetSize,
    AgcDcbDrawIndexOffset,
    AgcDcbDrawIndexOffsetGetSize,
    AgcDcbWriteData,
    AgcDcbWriteDataGetSize,
    AgcAcbWriteData,
    AgcCbReleaseMem,
    AgcCbQueueEndOfPipeActionGetSize,
    AgcDcbWaitRegMem,
    AgcDcbWaitOnAddressGetSize,
    AgcAcbWaitRegMem,
    AgcAcbWaitOnAddressGetSize,
    AgcWriteDataPatchSetAddressOrOffset,
    AgcWaitRegMemPatchAddress,
    AgcWaitRegMemPatchReference,
    AgcQueueEndOfPipeActionPatchAddress,
    AgcQueueEndOfPipeActionPatchData,
    AgcDcbStallCommandBufferParser,
    AgcCbSetShRegisterRangeDirect,
    AgcCbSetShRegisterRangeDirectGetSize,
    AgcCbSetShRegistersDirect,
    AgcCbSetUcRegistersDirect,
    AgcDcbSetShRegisterDirect,
    AgcDcbSetCxRegisterDirect,
    AgcDcbSetCxRegisterDirectGetSize,
    AgcDcbSetUcRegisterDirect,
    AgcDcbSetCxRegistersIndirect,
    AgcDcbSetCxRegistersIndirectGetSize,
    AgcDcbSetShRegistersIndirect,
    AgcDcbSetShRegistersIndirectGetSize,
    AgcDcbSetUcRegistersIndirect,
    AgcSetCxRegIndirectPatchSetAddress,
    AgcSetShRegIndirectPatchSetAddress,
    AgcSetUcRegIndirectPatchSetAddress,
    AgcSetCxRegIndirectPatchSetNumRegisters,
    AgcSetShRegIndirectPatchSetNumRegisters,
    AgcSetUcRegIndirectPatchSetNumRegisters,
    AgcSetCxRegIndirectPatchAddRegisters,
    AgcSetShRegIndirectPatchAddRegisters,
    AgcSetUcRegIndirectPatchAddRegisters,
    VideoGetVblankStatus,
    VideoColorSettingsSetGamma,
    VideoAdjustColor,
    AgcAcbResetQueue,
    AgcDcbResetQueue,
    AgcAcbAcquireMem,
    AgcDcbAcquireMem,
    AgcAcbEventWrite,
    AgcDcbEventWrite,
    AgcAcbPushMarker,
    AgcDcbPushMarker,
    AgcAcbPopMarker,
    AgcDcbPopMarker,
    AgcDcbContextStateOp,
    AgcCreatePrimState,
    AgcGetDataPacketPayloadAddress,
    AgcDcbSetIndexSize0600,
    AgcAcbCopyData,
    AgcDcbCopyData,
    AgcAcbDmaData,
    AgcDcbDmaData,
    AgcAcbDispatchIndirect,
    AgcDcbDispatchIndirect,
    AgcDcbSetBaseIndirectArgs,
    AgcDcbDrawIndexIndirect,
    AgcDcbWaitUntilSafeForRendering,
    AgcDcbGetLodStats,
}

/// Source frame boundary bookkeeping, independent of presentation or FPS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameState {
    pub boundaries: u64,
    pub reset_before_graphics: bool,
}
impl Default for FrameState {
    fn default() -> Self {
        Self {
            boundaries: 0,
            reset_before_graphics: true,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Explicit emulated display policy, not a host monitor measurement or
    /// proof that any vblank/flip has occurred.
    pub video_vblank_hz: u32,
    pub video_display: super::video::DisplayPolicy,
    pub video_presentation: super::video::PresentationLimits,
    pub max_calls: u64,
    pub max_batch_buffers: u32,
    pub max_dispatches: u64,
    pub max_draws: u64,
    pub max_vertices: u64,
    pub max_command_buffers: u64,
    pub max_command_bytes: u64,
    pub max_invocations: u64,
    pub max_preparation_work: u64,
    /// Optional per-call shader/interpolant/fusion work bound, independent of
    /// the cumulative driver ledger. None preserves the original allowance.
    pub max_shader_call_work: Option<u64>,
    pub max_memory_write_bytes: u64,
    pub max_shader_mappings: usize,
    pub compute: compute::Limits,
    pub commands: compute::CommandLimits,
    pub graphics: draw::Limits,
    pub graphics_commands: draw::CommandLimits,
    /// Optional graphics-only backend work ceiling, in conservative work units,
    /// not bytes or elapsed time. None preserves `gpu.max_work`; compute always
    /// uses the unchanged `gpu` limits, including its original work ceiling.
    pub graphics_backend_work: Option<u64>,
    pub gpu: GpuLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            video_vblank_hz: 60,
            video_display: Default::default(),
            video_presentation: Default::default(),
            max_calls: 256,
            max_batch_buffers: 32,
            max_dispatches: 128,
            max_draws: 128,
            max_vertices: 16 * 1024 * 1024,
            max_command_buffers: 256,
            max_command_bytes: 4 * 1024 * 1024,
            max_invocations: 16 * 1024 * 1024,
            max_preparation_work: 256 * 1024 * 1024,
            max_shader_call_work: None,
            max_memory_write_bytes: 4 * 1024 * 1024,
            max_shader_mappings: 4096,
            compute: compute::Limits::default(),
            commands: compute::CommandLimits::default(),
            graphics: draw::Limits::default(),
            graphics_commands: draw::CommandLimits::default(),
            graphics_backend_work: None,
            gpu: GpuLimits::default(),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    pub interrupts: u64,
    /// Admitted provider calls, including VideoOut, AGC and shader helpers;
    /// source-defined rejected return values still consume their call attempt.
    pub calls: u64,
    /// Attempts charged before resource preparation and actual GPU execution.
    pub dispatches: u64,
    /// Attempts charged before resource preparation and actual GPU execution.
    pub draws: u64,
    pub vertices: u64,
    /// Nonempty direct command ranges charged before copying and parsing.
    pub command_buffers: u64,
    pub indirect_buffers: u64,
    pub command_bytes: u64,
    pub invocations: u64,
    pub preparation_work: u64,
    /// Saturating committed writes: fenced draw/compute writeback and PM4
    /// memory/label stores. A positive total does not prove a completed draw.
    pub written_bytes: u64,
    pub memory_commands: u64,
    pub memory_written_bytes: u64,
}

/// Typed stops never become invented guest success or errno. Only explicit
/// null-argument branches use the source's EINVAL return domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Limit,
    Allocation,
    Memory,
    UnsupportedQueue,
    UnsupportedVideo,
    Poisoned,
    Waiting,
    Preparation(compute::Error),
    GraphicsPreparation(draw::Error),
    Vulkan(pentax_platform::vulkan::VulkanError),
    Kernel(pentax_kernel::Failure),
    Shader(super::shaders::ShaderFailure),
}

pub(crate) struct Driver {
    limits: Limits,
    usage: Usage,
    function_profile: FunctionProfile,
    queues: [compute::ComputeState; 56],
    graphics: draw::GraphicsState,
    graphics_owner: Option<pentax_platform::native_memory::NativeOwnerId>,
    graphics_execution: Option<graphics::GraphicsExecution>,
    graphics_admissions: Vec<graphics::GraphicsAdmission>,
    graphics_phase: graphics::GraphicsPhase,
    poisoned: bool,
    frame: FrameState,
    video: super::video::State,
    consumer: Option<ConsumerBudget>,
    video_ticks: [Option<(u64, u64)>; 3],
    defaults: Option<(pentax_platform::native_memory::NativeOwnerId, u64)>,
    shaders: super::shaders::State,
}
#[derive(Clone, Copy)]
struct ConsumerBudget {
    max_frame_bytes: usize,
    max_held_bytes: usize,
    external_held: usize,
}
impl Driver {
    fn shader_call_maximum(&self) -> Result<u64, Failure> {
        match self.limits.max_shader_call_work {
            Some(work) => self
                .usage
                .preparation_work
                .checked_add(work)
                .map(|maximum| maximum.min(self.limits.max_preparation_work))
                .ok_or(Failure::Limit),
            None => Ok(self.limits.max_preparation_work),
        }
    }

    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            usage: Usage::default(),
            function_profile: FunctionProfile::default(),
            queues: [compute::ComputeState::empty(); 56],
            graphics: draw::GraphicsState::empty(),
            graphics_owner: None,
            graphics_execution: None,
            graphics_admissions: Vec::new(),
            graphics_phase: graphics::GraphicsPhase::Idle,
            poisoned: false,
            frame: FrameState::default(),
            video: super::video::State::default(),
            consumer: None,
            video_ticks: [None; 3],
            defaults: None,
            shaders: Default::default(),
        }
    }
    pub(crate) fn usage(&self) -> Usage {
        self.usage
    }
    pub(crate) fn function_profile(&self) -> FunctionProfile {
        self.function_profile
    }
    pub(crate) fn video_flip_observations(&self) -> super::video::FlipObservations {
        self.video.flip_observations()
    }
    pub(crate) fn frame_state(&self) -> FrameState {
        self.frame
    }
    pub(crate) fn video_port(&self, handle: i32) -> Option<super::video::PortState> {
        self.video.port(handle)
    }
    pub(crate) fn video_surface(&self, handle: i32, buffer: i32) -> Option<super::video::Surface> {
        self.video.surface(handle, buffer)
    }

    pub(crate) fn shader_mapping(
        &self,
        owner: pentax_platform::native_memory::NativeOwnerId,
        address: u64,
    ) -> Option<super::shaders::MappedShader> {
        self.shaders.mapping(owner, address)
    }

    pub(crate) fn register_defaults(
        &mut self,
        owner: pentax_platform::native_memory::NativeOwnerId,
        kernel: &mut pentax_kernel::Kernel,
        guest: &mut impl pentax_kernel::GuestAccess,
        internal: bool,
        version: u64,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        if self.defaults.is_some_and(|(expected, _)| expected != owner) {
            return Err(Failure::Memory);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile.record(if internal {
            Function::AgcGetRegisterDefaultsInternal
        } else {
            Function::AgcGetRegisterDefaults
        });
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            if self.defaults.is_some() {
                8
            } else {
                super::agc_defaults::WORK
            },
            self.limits.max_preparation_work,
        )?;
        let base = if let Some((_, base)) = self.defaults {
            base
        } else {
            let base = kernel
                .allocate_runtime_data(
                    super::agc_defaults::SIZE,
                    guest,
                    super::agc_defaults::initialize,
                )
                .map_err(Failure::Kernel)?;
            self.defaults = Some((owner, base));
            base
        };
        base.checked_add(super::agc_defaults::descriptor(internal, version))
            .ok_or(Failure::Limit)
    }

    #[cfg(test)]
    pub(crate) fn submit(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        self.submit_with_stack(memory, function, args, None)
    }
    #[cfg(test)]
    pub(crate) fn submit_with_stack(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
    ) -> Result<u64, Failure> {
        self.submit_impl(memory, function, args, stack, None, None)
    }
    pub(crate) fn submit_with_kernel(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
        kernel: &mut pentax_kernel::Kernel,
    ) -> Result<u64, Failure> {
        self.submit_impl(memory, function, args, stack, Some(kernel), None)
    }
    pub(crate) fn submit_with_kernel_clock(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
        kernel: &mut pentax_kernel::Kernel,
        clock: GraphicsClock,
    ) -> Result<u64, Failure> {
        self.submit_impl(memory, function, args, stack, Some(kernel), Some(clock))
    }
    /// The real HLE context supplies the process-relative clock lazily, after
    /// source handle/mode/index/queue admission. No extra kernel call is minted.
    pub(crate) fn submit_video_flip_with_clock(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        args: [u64; 6],
        clock: impl FnOnce(&mut PausedNativeMemory<'_>) -> Result<u64, Failure>,
    ) -> Result<u64, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile.record(Function::VideoSubmitFlip);
        self.video_submit_flip(memory, args, clock)
    }
    fn admit_submit_call(&mut self, function: Function) -> Result<(), Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile.record(function);
        Ok(())
    }

    /// Exact native action entry. The original call is admitted only here;
    /// subsequent waits resume the owned graphics plan, never this method.
    pub(crate) fn begin_native_graphics_submit(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        kernel: &mut pentax_kernel::Kernel,
        clock: GraphicsClock,
    ) -> Result<GraphicsProgress, Failure> {
        if !matches!(
            function,
            Function::CommandBuffer | Function::MultiCommandBuffers | Function::SuspendPoint
        ) {
            return Err(Failure::UnsupportedQueue);
        }
        self.admit_submit_call(function)?;
        if function == Function::SuspendPoint {
            return self.queue_graphics_suspend(memory, Some(kernel), Some(clock));
        }
        if let Some(value) = null_result(function, args) {
            return Ok(GraphicsProgress::Complete(value));
        }
        let (queue, ranges) = self.ranges(memory, function, args)?;
        if !(0x20..0x58).contains(&queue) {
            self.begin_graphics_execution(memory, &ranges, Some(kernel), Some(clock))
        } else {
            self.execute_compute_ranges(memory, queue, ranges, Some(kernel))
                .map(GraphicsProgress::Complete)
        }
    }

    fn submit_impl(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
        mut kernel: Option<&mut pentax_kernel::Kernel>,
        clock: Option<GraphicsClock>,
    ) -> Result<u64, Failure> {
        self.admit_submit_call(function)?;
        if matches!(
            function,
            Function::VideoGetVblankStatus
                | Function::VideoColorSettingsSetGamma
                | Function::VideoAdjustColor
        ) {
            return self.video_state_call(memory, function, args, None);
        }
        if matches!(
            function,
            Function::VideoAddFlipEvent
                | Function::VideoDeleteFlipEvent
                | Function::VideoAddVblankEvent
                | Function::VideoDeleteVblankEvent
                | Function::VideoGetEventId
                | Function::VideoGetEventData
                | Function::VideoGetEventCount
        ) || (function == Function::VideoClose && kernel.is_some())
        {
            return self.video_event_call(memory, function, args, stack, kernel);
        }
        if function == Function::AgcDcbSetFlip {
            return self.encode_flip(memory, args);
        }
        if matches!(
            function,
            Function::VideoGetFlipStatus | Function::VideoIsFlipPending
        ) {
            return self.video_flip_query(memory, function, args);
        }
        if function == Function::VideoSubmitFlip {
            return Err(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                "bound VideoOut process-counter context",
            )));
        }
        if matches!(
            function,
            Function::AgcAddEqEvent
                | Function::AgcDeleteEqEvent
                | Function::AgcGetEqEventType
                | Function::AgcGetEqContextId
        ) {
            return self.event_queue(memory, function, args, kernel);
        }
        if matches!(
            function,
            Function::AgcCreateInterpolantMapping | Function::AgcCreateInterpolantMapping2
        ) {
            let maximum = self.shader_call_maximum()?;
            return super::shaders::interpolants::create(
                memory,
                args,
                function == Function::AgcCreateInterpolantMapping2,
                &mut self.usage.preparation_work,
                maximum,
            );
        }
        if function == Function::AgcGetDataPacketPayloadAddress {
            return self.packet_payload_address(memory, args);
        }
        if function == Function::AgcCreatePrimState {
            let maximum = self.shader_call_maximum()?;
            return super::shaders::primitive::create(
                memory,
                args,
                &mut self.usage.preparation_work,
                maximum,
            );
        }
        if function == Function::AgcGetFusedShaderSize {
            let maximum = self.shader_call_maximum()?;
            let result = super::shaders::fusion::size(
                memory,
                args,
                &mut self.usage.preparation_work,
                maximum,
            );
            self.function_profile.record_shader_fuse_size(&result);
            return result;
        }
        if matches!(
            function,
            Function::AgcFuseShaderHalves | Function::AgcFuseShaderHalvesLegacy
        ) {
            let maximum = self.shader_call_maximum()?;
            let result = super::shaders::fusion::fuse(
                memory,
                args,
                function == Function::AgcFuseShaderHalves,
                &mut self.usage.preparation_work,
                maximum,
            );
            if function == Function::AgcFuseShaderHalves {
                self.function_profile.record_shader_fuse(&result);
            }
            return result;
        }
        if function == Function::AgcCreateShader {
            let maximum = self.shader_call_maximum()?;
            let result = self.shaders.create(
                memory,
                args,
                self.limits.max_shader_mappings,
                &mut self.usage.preparation_work,
                maximum,
            );
            self.function_profile.record_shader_create(&result);
            return result;
        }
        if function == Function::AgcGetIsTrinityMode {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                8,
                self.limits.max_preparation_work,
            )?;
            // Source is void and writes exactly one byte from its fixed false
            // base-console profile. The transport return conveys no status.
            memory
                .apply_transaction(
                    pentax_platform::native_memory::NativeMemoryTransaction {
                        effect: pentax_platform::native_memory::NativeMemoryEffect::None,
                        writes: &[NativeMemoryWrite {
                            address: args[0],
                            bytes: &[0],
                        }],
                    },
                    &NativeTransactionLimits::default(),
                )
                .map_err(|_| Failure::Memory)?;
            return Ok(0);
        }
        if matches!(
            function,
            Function::AgcInit | Function::AgcRegistryUnavailable
        ) {
            // Pinned AgcInit returns OK without state access. The four pinned
            // registry APIs return the exact int32 error, never fake handles or
            // successful registration. None reads or writes guest memory.
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                8,
                self.limits.max_preparation_work,
            )?;
            return Ok(if function == Function::AgcRegistryUnavailable {
                0x8a6c_9018
            } else {
                0
            });
        }
        if super::video::supported(function) {
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                match function {
                    Function::VideoRegisterBuffers2 | Function::VideoChangeBufferAttribute2 => 4096,
                    Function::VideoSetBufferAttribute2 => 256,
                    _ => 128,
                },
                self.limits.max_preparation_work,
            )?;
            return self.video.invoke(
                function,
                args,
                self.limits.video_vblank_hz,
                self.limits.video_display,
                stack,
                memory,
            );
        }
        if function == Function::SuspendPoint {
            if self.graphics_execution.is_some() || !self.graphics_admissions.is_empty() {
                return Err(Failure::Waiting);
            }
            // Every admitted compute submission waits for its Vulkan fence and
            // same-owner writeback before returning. There is no pending queue
            // to drain here. Poisoned/partial submissions are rejected above.
            // Preserve Done's actual frame number and next-graphics reset flag;
            // compute register state is independent and must remain retained.
            let next = self.frame.boundaries.checked_add(1).ok_or(Failure::Limit)?;
            self.frame.boundaries = next;
            self.frame.reset_before_graphics = true;
            return Ok(0);
        }
        if let Some(value) = null_result(function, args) {
            return Ok(value);
        }
        let (queue, ranges) = self.ranges(memory, function, args)?;
        // The pinned context submit routes every non-ACB queue selector to
        // the single graphics processor. Explicit ACB calls remain compute-only.
        if ranges.iter().any(|r| r.address != 0 && r.size != 0) && !(0x20..0x58).contains(&queue) {
            if matches!(
                function,
                Function::CommandBuffer | Function::MultiCommandBuffers
            ) {
                return self.submit_graphics_with_context(
                    memory,
                    &ranges,
                    kernel.as_deref_mut(),
                    clock,
                );
            }
            return Err(Failure::UnsupportedQueue);
        }
        self.execute_compute_ranges(memory, queue, ranges, kernel)
    }

    fn execute_compute_ranges(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        queue: u32,
        ranges: Vec<NativeRange>,
        mut kernel: Option<&mut pentax_kernel::Kernel>,
    ) -> Result<u64, Failure> {
        for range in ranges.into_iter().filter(|r| r.address != 0 && r.size != 0) {
            let next_bytes = add(
                self.usage.command_bytes,
                range.size,
                self.limits.max_command_bytes,
            )?;
            self.usage.command_buffers = add(
                self.usage.command_buffers,
                1,
                self.limits.max_command_buffers,
            )?;
            self.usage.command_bytes = next_bytes;
            let queue_index = (queue - 0x20) as usize;
            let mut command_limits = self.limits.commands;
            command_limits.max_command_bytes = command_limits.max_command_bytes.min(
                usize::try_from(self.limits.max_command_bytes - next_bytes + range.size)
                    .unwrap_or(usize::MAX),
            );
            command_limits.max_dispatches = command_limits.max_dispatches.min(
                usize::try_from(
                    self.limits
                        .max_dispatches
                        .saturating_sub(self.usage.dispatches),
                )
                .unwrap_or(usize::MAX),
            );
            command_limits.max_work = command_limits.max_work.min(
                self.limits
                    .max_preparation_work
                    .saturating_sub(self.usage.preparation_work),
            );
            let mut plan = register_execution::ComputeCommands::prepare(
                memory,
                compute::CommandRange {
                    address: range.address,
                    size: range.size,
                },
                self.queues[queue_index],
                &command_limits,
            )?;
            self.usage.command_bytes = add(
                self.usage.command_bytes,
                (plan.usage().command_bytes as u64)
                    .checked_sub(range.size)
                    .ok_or(Failure::Limit)?,
                self.limits.max_command_bytes,
            )?;
            self.usage.preparation_work = add(
                self.usage.preparation_work,
                plan.usage().work,
                self.limits.max_preparation_work,
            )?;
            self.usage.indirect_buffers = self
                .usage
                .indirect_buffers
                .saturating_add(plan.usage().indirect_buffers);
            let mut staging = self.transfer_staging(
                plan.usage().staging_bytes,
                plan.usage().write_bytes,
                plan.usage().host_bytes,
                command_limits
                    .max_host_bytes
                    .min(self.limits.compute.max_host_bytes),
            )?;
            while let Some(step) = plan.next_step()? {
                let owned_dispatch;
                let command = match step {
                    register_execution::ComputeStep::Array => {
                        plan.resolve_array(memory)?;
                        continue;
                    }
                    register_execution::ComputeStep::Command(index) => plan.command(index)?,
                    register_execution::ComputeStep::Dispatch(dispatch) => {
                        owned_dispatch = dispatch;
                        compute::Command::Dispatch(&owned_dispatch)
                    }
                };
                let dispatch = match command {
                    compute::Command::Synchronize(sync) => {
                        self.synchronize_commands(sync)?;
                        continue;
                    }
                    compute::Command::Annotation(marker) => {
                        self.annotation_command(marker)?;
                        continue;
                    }
                    compute::Command::Drain => {
                        self.drain_commands()?;
                        continue;
                    }
                    compute::Command::Release(release) => {
                        self.retire_release(
                            memory,
                            release,
                            plan.command_ranges(),
                            &mut kernel,
                            queue,
                        )?;
                        continue;
                    }
                    compute::Command::Dispatch(dispatch) => dispatch,
                    compute::Command::Write(write) => {
                        self.memory_write(memory, write, plan.command_ranges())?;
                        continue;
                    }
                    compute::Command::Transfer(transfer) => {
                        self.transfer_command(
                            memory,
                            transfer,
                            plan.command_ranges(),
                            &mut staging,
                        )?;
                        continue;
                    }
                    compute::Command::Wait(wait) => {
                        self.usage.preparation_work = add(
                            self.usage.preparation_work,
                            wait.byte_count() as u64 + 1,
                            self.limits.max_preparation_work,
                        )?;
                        self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
                        if !wait
                            .satisfied(&super::Memory(memory))
                            .map_err(Failure::Preparation)?
                        {
                            return Err(Failure::Waiting);
                        }
                        continue;
                    }
                };
                let remaining_invocations = self
                    .limits
                    .max_invocations
                    .saturating_sub(self.usage.invocations);
                let mut limits = self.limits.compute;
                if self.shaders.lookup_work() > 1
                    && let Ok(address) = dispatch.shader_address()
                {
                    self.usage.preparation_work = add(
                        self.usage.preparation_work,
                        self.shaders.lookup_work(),
                        self.limits.max_preparation_work,
                    )?;
                    if let Some(mapped) = self.shaders.mapping(memory.owner_id(), address) {
                        if mapped.kind != 0 {
                            return Err(Failure::Shader(super::shaders::ShaderFailure::Stage));
                        }
                        // Zero size follows the source's unspecified-size metadata
                        // behavior. A nonzero declared extent bounds real decoding.
                        if mapped.code_bytes != 0 {
                            limits.max_shader_words =
                                limits.max_shader_words.min(mapped.code_bytes as usize / 4);
                        }
                    }
                }
                limits.max_invocations = limits.max_invocations.min(remaining_invocations);
                limits.max_work = limits.max_work.min(
                    self.limits
                        .max_preparation_work
                        .saturating_sub(self.usage.preparation_work),
                );
                limits.max_host_bytes = limits
                    .max_host_bytes
                    .checked_sub(plan.usage().host_bytes)
                    .and_then(|bytes| bytes.checked_sub(staging.capacity()))
                    .ok_or(Failure::Limit)?;
                self.usage.dispatches = add(self.usage.dispatches, 1, self.limits.max_dispatches)?;
                let report = execute_dispatch_admitted(
                    memory,
                    dispatch,
                    &limits,
                    &self.limits.gpu,
                    plan.command_ranges(),
                    |prepared, preflight, may_execute| {
                        let invocations = add(
                            self.usage.invocations,
                            prepared.invocations,
                            self.limits.max_invocations,
                        )
                        .map_err(|_| Error::Limit)?;
                        let work = prepared.work.checked_add(preflight).ok_or(Error::Limit)?;
                        let work = add(
                            self.usage.preparation_work,
                            work,
                            self.limits.max_preparation_work,
                        )
                        .map_err(|_| Error::Limit)?;
                        self.usage.invocations = invocations;
                        self.usage.preparation_work = work;
                        // Once GPU work could have happened, a later failure is
                        // terminal. Never replay a partially completed stream with
                        // an earlier register snapshot.
                        self.poisoned |= may_execute;
                        Ok(())
                    },
                )
                .map_err(|e| match e {
                    Error::Prepare(e) => Failure::Preparation(e),
                    Error::Memory(_) => Failure::Memory,
                    Error::Vulkan(e) => Failure::Vulkan(e),
                    Error::Allocation => Failure::Allocation,
                    Error::Limit => Failure::Limit,
                })?;
                // max_dispatches * max_buffer_bytes can overflow for caller limits;
                // accounting is saturated, never a post-effect fallible operation.
                self.usage.written_bytes = self
                    .usage
                    .written_bytes
                    .saturating_add(report.written_bytes as u64);
            }
            self.queues[queue_index] = plan.final_state()?;
            self.poisoned = true;
        }
        self.poisoned = false;
        Ok(0)
    }

    fn memory_write(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        write: &compute::MemoryWrite,
        commands: &[compute::CommandRange],
    ) -> Result<(), Failure> {
        self.memory_write_bytes(memory, write, commands, write.bytes())
    }
    fn memory_write_bytes(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        write: &compute::MemoryWrite,
        commands: &[compute::CommandRange],
        payload: &[u8],
    ) -> Result<(), Failure> {
        if payload.len() != write.bytes().len() {
            return Err(Failure::Memory);
        }
        if payload.is_empty() {
            // The source DCB encoder can emit a zero-payload WRITE_DATA. It is
            // an admitted command, with no address access or invented write.
            self.drain_commands()?;
            return Ok(());
        }
        let address = write.address();
        let size = write.bytes().len() as u64;
        let bytes = add(
            self.usage.memory_written_bytes,
            size,
            self.limits.max_memory_write_bytes,
        )?;
        let remaining = self
            .limits
            .max_preparation_work
            .saturating_sub(self.usage.preparation_work);
        let mut work = memory
            .preflight_distinct_backing(address, size, remaining)
            .map_err(|_| Failure::Memory)?;
        memory
            .preflight_gpu_write(address, size)
            .map_err(|_| Failure::Memory)?;
        for command in commands {
            let charge = memory
                .preflight_disjoint_backing(
                    NativeRange { address, size },
                    NativeRange {
                        address: command.address,
                        size: command.size,
                    },
                    remaining.checked_sub(work).ok_or(Failure::Limit)?,
                )
                .map_err(|_| Failure::Memory)?;
            work = work.checked_add(charge).ok_or(Failure::Limit)?;
        }
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work.checked_add(size).ok_or(Failure::Limit)?,
            self.limits.max_preparation_work,
        )?;
        self.usage.memory_written_bytes = bytes;
        self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
        self.poisoned = true;
        memory
            .apply_gpu_writes(
                &[NativeMemoryWrite {
                    address,
                    bytes: payload,
                }],
                &NativeTransactionLimits {
                    max_outputs: 1,
                    max_output_bytes: size,
                    max_preflight_steps: work,
                    ..Default::default()
                },
            )
            .map_err(|_| Failure::Memory)?;
        self.usage.written_bytes = self.usage.written_bytes.saturating_add(size);
        Ok(())
    }

    fn annotation_command(&mut self, marker: &compute::Marker) -> Result<(), Failure> {
        // Sealed source annotation. No GPU state, memory, barrier, signal or
        // presentation effect; no host string interpretation or text logging.
        let _ = marker.kind();
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            1,
            self.limits.max_preparation_work,
        )?;
        Ok(())
    }

    fn synchronize_commands(&mut self, sync: &compute::Synchronization) -> Result<(), Failure> {
        // The parser retains the exact payload and admits only supported source
        // barriers. Every preceding GPU job has already fenced and completed
        // owned writeback; no outstanding cache replica remains to invalidate.
        // This performs no range dereference, label write, event or fake fence.
        match (sync.kind(), sync.payload().len()) {
            (compute::SynchronizationKind::Acquire, 7)
            | (compute::SynchronizationKind::Event, 1) => self.drain_commands(),
            _ => Err(Failure::UnsupportedQueue),
        }
    }

    fn drain_commands(&mut self) -> Result<(), Failure> {
        // Every prior draw/dispatch already completed its real fence and owned
        // writeback. Accept the serialized parser barrier without a fake job.
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            1,
            self.limits.max_preparation_work,
        )?;
        self.usage.memory_commands = self.usage.memory_commands.saturating_add(1);
        Ok(())
    }

    fn ranges(
        &self,
        memory: &PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<(u32, Vec<NativeRange>), Failure> {
        let (queue, packet, arrays) = match function {
            Function::Acb => (args[0] as u32, Some(args[1]), None),
            Function::CommandBuffer => (context_queue(memory, args[0])?, Some(args[1]), None),
            Function::MultiAcbs => (
                args[0] as u32,
                None,
                Some((args[1], args[2], args[3] as u32)),
            ),
            Function::MultiCommandBuffers => {
                // A zero count returns before touching even the context.
                if args[3] as u32 == 0 {
                    return Ok((0, Vec::new()));
                }
                (
                    context_queue(memory, args[0])?,
                    None,
                    Some((args[1], args[2], args[3] as u32)),
                )
            }
            _ => return Err(Failure::UnsupportedQueue),
        };
        let count = arrays.map_or(1, |(_, _, n)| n);
        if count > self.limits.max_batch_buffers {
            return Err(Failure::Limit);
        }
        let mut ranges = Vec::new();
        ranges
            .try_reserve_exact(count as usize)
            .map_err(|_| Failure::Allocation)?;
        // Capacity must stay within the retained metadata bound, even if the
        // allocator returned more elements than requested.
        if ranges.capacity() > self.limits.max_batch_buffers as usize {
            return Err(Failure::Limit);
        }
        if let Some(packet) = packet {
            let mut raw = [0; 16];
            memory.read(packet, &mut raw).map_err(|_| Failure::Memory)?;
            // flags/reserved are ignored by this pinned submit path.
            ranges.push(NativeRange {
                address: u64::from_le_bytes(raw[..8].try_into().unwrap()),
                size: u64::from(u32::from_le_bytes(raw[8..12].try_into().unwrap())) * 4,
            });
        } else if let Some((addresses, sizes, count)) = arrays {
            for index in 0..u64::from(count) {
                let mut address = [0; 8];
                let mut size = [0; 4];
                let address_at = addresses.checked_add(index * 8).ok_or(Failure::Memory)?;
                let size_at = sizes.checked_add(index * 4).ok_or(Failure::Memory)?;
                memory
                    .read(address_at, &mut address)
                    .map_err(|_| Failure::Memory)?;
                memory
                    .read(size_at, &mut size)
                    .map_err(|_| Failure::Memory)?;
                ranges.push(NativeRange {
                    address: u64::from_le_bytes(address),
                    size: u64::from(u32::from_le_bytes(size)) * 4,
                });
            }
        }
        let mut bytes = 0_u64;
        let maximum_range = if matches!(
            function,
            Function::CommandBuffer | Function::MultiCommandBuffers
        ) && !(0x20..0x58).contains(&queue)
        {
            self.limits.graphics_commands.max_command_bytes
        } else {
            self.limits.commands.max_command_bytes
        };
        for range in &ranges {
            if range.address != 0 && range.size != 0 {
                if range.size > maximum_range as u64 {
                    return Err(Failure::Limit);
                }
                bytes = add(
                    bytes,
                    range.size,
                    self.limits
                        .max_command_bytes
                        .saturating_sub(self.usage.command_bytes),
                )?;
                memory
                    .preflight_gpu_read(range.address, range.size)
                    .map_err(|_| Failure::Memory)?;
            }
        }
        Ok((queue, ranges))
    }
}

fn context_queue(memory: &PausedNativeMemory<'_>, context: u64) -> Result<u32, Failure> {
    let mut raw = [0; 4];
    memory
        .read(context.checked_add(4).ok_or(Failure::Memory)?, &mut raw)
        .map_err(|_| Failure::Memory)?;
    Ok(u32::from_le_bytes(raw))
}
fn add(value: u64, count: u64, maximum: u64) -> Result<u64, Failure> {
    value
        .checked_add(count)
        .filter(|n| *n <= maximum)
        .ok_or(Failure::Limit)
}

/// Source null-argument ordering, separated from reads and GPU effects.
pub(crate) fn null_result(function: Function, args: [u64; 6]) -> Option<u64> {
    const INVALID: u64 = 0x8002_0016; // int return: low 32 bits, as the kernel ABI.
    match function {
        Function::Acb if args[1] == 0 => Some(0),
        Function::CommandBuffer if args[0] == 0 || args[1] == 0 => Some(INVALID),
        Function::MultiAcbs | Function::MultiCommandBuffers if args[3] as u32 == 0 => Some(0),
        Function::MultiAcbs if args[1] == 0 || args[2] == 0 => Some(INVALID),
        Function::MultiCommandBuffers if args[..3].contains(&0) => Some(INVALID),
        _ => None,
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod suspend_tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    #[test]
    fn registry_calls_return_the_source_error_without_creating_resources() {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let mut memory = space.pause().unwrap();
        let mut driver = Driver::new(Limits {
            max_calls: 1,
            ..Default::default()
        });
        assert_eq!(driver.function_profile().last(), "none");
        assert_eq!(driver.function_profile().top(), [None; 4]);
        assert_eq!(
            driver.submit(&mut memory, Function::AgcRegistryUnavailable, [u64::MAX; 6]),
            Ok(0x8a6c_9018)
        );
        assert_eq!(driver.frame_state(), FrameState::default());
        assert_eq!(driver.usage().dispatches, 0);
        assert_eq!(driver.video_port(1), None);
        let admitted = driver.function_profile();
        assert_eq!(admitted.count(Function::AgcRegistryUnavailable), 1);
        assert_eq!(admitted.last(), "AgcRegistryUnavailable");
        assert_eq!(
            admitted.top(),
            [Some(("AgcRegistryUnavailable", 1)), None, None, None]
        );
        assert_eq!(
            driver.submit(&mut memory, Function::AgcInit, [0; 6]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.function_profile(), admitted);
        let mut limited = Driver::new(Limits {
            max_preparation_work: 7,
            ..Default::default()
        });
        assert_eq!(
            limited.submit(&mut memory, Function::AgcRegistryUnavailable, [0; 6]),
            Err(Failure::Limit)
        );
        limited.poisoned = true;
        let admitted = limited.function_profile();
        assert_eq!(admitted.count(Function::AgcRegistryUnavailable), 1);
        assert_eq!(limited.usage.calls, 1); // Work refusal followed admitted call.
        assert_eq!(
            limited.submit(&mut memory, Function::AgcRegistryUnavailable, [0; 6]),
            Err(Failure::Poisoned)
        );
        assert_eq!(limited.function_profile(), admitted);
    }

    #[test]
    fn agc_trinity_query_writes_one_source_byte_and_preserves_neighbors_on_refusal() {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(base, 0x4000, NativePermissions::READ_WRITE)
            .unwrap();
        let mut memory = space.pause().unwrap();
        let mut driver = Driver::new(Default::default());
        memory.write(base + 8, &[0xa5; 8]).unwrap();
        driver
            .submit(
                &mut memory,
                Function::AgcGetIsTrinityMode,
                [base + 11, 0, 0, 0, 0, 0],
            )
            .unwrap();
        let mut bytes = [0; 8];
        memory.read(base + 8, &mut bytes).unwrap();
        assert_eq!(bytes, [0xa5, 0xa5, 0xa5, 0, 0xa5, 0xa5, 0xa5, 0xa5]);
        driver
            .submit(
                &mut memory,
                Function::AgcGetIsTrinityMode,
                [base + 0x3fff, 0, 0, 0, 0, 0],
            )
            .unwrap();
        assert_eq!(
            driver.submit(
                &mut memory,
                Function::AgcGetIsTrinityMode,
                [base + 0x4000, 0, 0, 0, 0, 0]
            ),
            Err(Failure::Memory)
        );
        assert_eq!(
            driver.submit(&mut memory, Function::AgcGetIsTrinityMode, [0; 6]),
            Err(Failure::Memory)
        );
        let mut limited = Driver::new(Limits {
            max_preparation_work: 7,
            ..Default::default()
        });
        assert_eq!(
            limited.submit(
                &mut memory,
                Function::AgcGetIsTrinityMode,
                [base + 8, 0, 0, 0, 0, 0]
            ),
            Err(Failure::Limit)
        );
        let mut actual = [0; 8];
        memory.read(base + 8, &mut actual).unwrap();
        assert_eq!(actual, bytes);
        limited.poisoned = true;
        assert_eq!(
            limited.submit(
                &mut memory,
                Function::AgcGetIsTrinityMode,
                [base + 8, 0, 0, 0, 0, 0]
            ),
            Err(Failure::Poisoned)
        );
    }

    #[test]
    fn agc_init_keeps_source_no_op_semantics_under_shared_limits() {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let mut memory = space.pause().unwrap();
        let mut driver = Driver::new(Limits {
            max_calls: 2,
            ..Default::default()
        });
        for args in [[0; 6], [u64::MAX; 6]] {
            assert_eq!(driver.submit(&mut memory, Function::AgcInit, args), Ok(0));
            assert_eq!(driver.frame_state(), FrameState::default());
            assert_eq!(driver.video_port(1), None);
        }
        assert_eq!(driver.usage().calls, 2);
        assert_eq!(driver.usage().preparation_work, 16);
        assert_eq!(driver.usage().dispatches, 0);
        assert_eq!(
            driver.submit(&mut memory, Function::AgcInit, [0; 6]),
            Err(Failure::Limit)
        );
        let mut driver = Driver::new(Limits {
            max_preparation_work: 7,
            ..Default::default()
        });
        assert_eq!(
            driver.submit(&mut memory, Function::AgcInit, [0; 6]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.usage().preparation_work, 0);
        driver.poisoned = true;
        let before = driver.usage();
        assert_eq!(
            driver.submit(&mut memory, Function::AgcInit, [0; 6]),
            Err(Failure::Poisoned)
        );
        assert_eq!(driver.usage(), before);
    }

    #[test]
    fn suspend_records_actual_boundaries_and_refuses_limits_or_partial_submission_failure() {
        let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x10000).unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        space
            .map_at(
                base,
                0x4000,
                NativePermissions::READ_WRITE.with_gpu(true, true),
            )
            .unwrap();
        let mut memory = space.pause().unwrap();
        let mut driver = Driver::new(Limits {
            max_calls: 2,
            ..Default::default()
        });
        for boundary in 1..=2 {
            assert_eq!(
                driver.submit(&mut memory, Function::SuspendPoint, [u64::MAX; 6]),
                Ok(0)
            );
            assert_eq!(
                driver.frame_state(),
                FrameState {
                    boundaries: boundary,
                    reset_before_graphics: true
                }
            );
        }
        assert_eq!(
            driver.submit(&mut memory, Function::SuspendPoint, [0; 6]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.frame_state().boundaries, 2);
        let mut driver = Driver::new(Default::default());
        driver.frame.boundaries = u64::MAX;
        assert_eq!(
            driver.submit(&mut memory, Function::SuspendPoint, [0; 6]),
            Err(Failure::Limit)
        );
        assert_eq!(driver.frame_state().boundaries, u64::MAX);
        let mut driver = Driver::new(Default::default());
        let label = base + 0x200;
        let words = [
            0xc005_3700,
            0x100,
            label as u32,
            (label >> 32) as u32,
            1,
            1,
            1,
            0xc005_3c00,
            0x13,
            label as u32,
            (label >> 32) as u32,
            17,
            u32::MAX,
            0,
        ];
        let bytes: Vec<_> = words.into_iter().flat_map(u32::to_le_bytes).collect();
        memory.write(base, &bytes).unwrap();
        let mut packet = [0; 16];
        packet[..8].copy_from_slice(&base.to_le_bytes());
        packet[8..12].copy_from_slice(&(words.len() as u32).to_le_bytes());
        memory.write(base + 0x100, &packet).unwrap();
        assert_eq!(
            driver.submit(&mut memory, Function::Acb, [0x20, base + 0x100, 0, 0, 0, 0]),
            Err(Failure::Waiting)
        );
        let mut actual = [0; 4];
        memory.read(label, &mut actual).unwrap();
        assert_eq!(u32::from_le_bytes(actual), 1); // A real earlier effect happened.
        let usage = driver.usage();
        assert_eq!(
            driver.submit(&mut memory, Function::SuspendPoint, [0; 6]),
            Err(Failure::Poisoned)
        );
        assert_eq!(driver.frame_state(), FrameState::default());
        assert_eq!(driver.usage(), usage);
    }
}
