// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned video ports and checked options.
// Adapted from pinned graphics/presentation/videoOut.cpp; see NOTICE.

//! Owned ports, registered surfaces and checked CPU flip snapshots. Actual
//! consumer acknowledgement advances flip completion. Display periods use the
//! independent owned monotonic grid; they are not presented-frame evidence.
use super::driver::{Failure, Function};
use pentax_platform::native_memory::{
    NativeMemoryWrite, NativeTransactionLimits, PausedNativeMemory,
};

mod buffers;
pub use buffers::{Compression, PixelFormat, Surface};
mod pixels;
mod presentation;
mod state;
pub use pixels::{
    ImageLayout, PresentationLimits, SurfaceLayout, detile_32, retile_32, retile_32_into,
};
pub use presentation::{
    FlipCompletion, FlipObservations, FlipSource, FlipStatus, FlipSubmission, FlipTicket, Frame,
    ReadyFrameMetadata,
};
pub(crate) use state::ClockedCall;
pub use state::VblankStatus;

const INVALID_VALUE: u64 = 0x8029_0001;
const INVALID_ADDRESS: u64 = 0x8029_0002;
const BUSY: u64 = 0x8029_0009;
const INVALID_HANDLE: u64 = 0x8029_000b;
const UNSUPPORTED_MODE: u64 = 0x8029_0016;
const UNAVAILABLE_MODE: u64 = 0x8029_0019;
const INVALID_OPTION: u64 = 0x8029_001a;
const UNSUPPORTED_OPERATION: u64 = 0x8029_001c;

/// Caller-admitted guest display policy. None follows the pinned metadata
/// lookup's missing-value fallback; it does not claim known title attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DisplayPolicy {
    pub resolution: pentax_config::Resolution,
    pub application_attribute3: Option<i32>,
    pub refresh_follows_vblank: bool,
}

fn output(memory: &mut PausedNativeMemory<'_>, address: u64, bytes: &[u8]) -> Result<(), Failure> {
    memory
        .apply_transaction(
            pentax_platform::native_memory::NativeMemoryTransaction {
                effect: pentax_platform::native_memory::NativeMemoryEffect::None,
                writes: &[NativeMemoryWrite { address, bytes }],
            },
            &NativeTransactionLimits::default(),
        )
        .map_err(|_| Failure::Memory)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortState {
    pub generation: u64,
    pub flip_rate: u32,
    pub output_mode: u64,
    pub master_handle: Option<i32>,
    pub slave_count: u32,
    /// Retained source color setting; not proof of host display correction.
    pub gamma_bits: u32,
}
impl Default for PortState {
    fn default() -> Self {
        Self {
            generation: 0,
            flip_rate: 0,
            output_mode: 1,
            master_handle: None,
            slave_count: 0,
            gamma_bits: 0x3f80_0000,
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Port {
    state: PortState,
    open: bool,
    buffers: buffers::Buffers,
    vblank: VblankStatus,
    grid: Option<state::DisplayGrid>,
}
#[derive(Default)]
pub(crate) struct State {
    ports: [Port; 3],
    presentation: presentation::Queue,
}
fn index(handle: i32) -> Option<usize> {
    usize::try_from(handle.checked_sub(1)?)
        .ok()
        .filter(|i| *i < 3)
}
impl State {
    pub(crate) fn surface(&self, handle: i32, buffer: i32) -> Option<Surface> {
        let port = &self.ports[index(handle)?];
        if !port.open {
            return None;
        }
        port.buffers.surface(buffer)
    }
    pub(crate) fn port(&self, handle: i32) -> Option<PortState> {
        let slot = index(handle)?;
        self.ports[slot].open.then_some(self.ports[slot].state)
    }
    pub(crate) fn invoke(
        &mut self,
        function: Function,
        args: [u64; 6],
        vblank_hz: u32,
        display: DisplayPolicy,
        stack: Option<u64>,
        memory: &mut PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        // Port/registration metadata never migrates between native owners.
        self.presentation.bind_owner(memory.owner_id())?;
        if matches!(
            function,
            Function::VideoSetBufferAttribute2 | Function::VideoAddBufferAttributeOption
        ) {
            if args[0] == 0 {
                return Err(Failure::UnsupportedVideo);
            }
            if function == Function::VideoAddBufferAttributeOption {
                let address = args[0].checked_add(24).ok_or(Failure::Memory)?;
                let mut bytes = [0; 8];
                memory
                    .read(address, &mut bytes)
                    .map_err(|_| Failure::Memory)?;
                output(
                    memory,
                    address,
                    &(u64::from_le_bytes(bytes) | args[1]).to_le_bytes(),
                )?;
            } else {
                // Capture both real SysV stack arguments before output can
                // overlap them. The pointer is guest data, never a host pointer.
                let address = stack
                    .ok_or(Failure::UnsupportedVideo)?
                    .checked_add(8)
                    .ok_or(Failure::Memory)?;
                let mut extra = [0; 16];
                memory
                    .read(address, &mut extra)
                    .map_err(|_| Failure::Memory)?;
                let mut bytes = [0; 80];
                for (at, value) in [(4, args[2]), (12, args[3]), (16, args[4])] {
                    bytes[at..at + 4].copy_from_slice(&(value as u32).to_le_bytes());
                }
                bytes[24..32].copy_from_slice(&args[5].to_le_bytes());
                bytes[32..40].copy_from_slice(&args[1].to_le_bytes());
                bytes[40..48].copy_from_slice(&extra[8..]);
                bytes[48..52].copy_from_slice(&extra[..4]);
                output(memory, args[0], &bytes)?;
            }
            // These source functions are void. The transport register value is
            // unspecified by their ABI and is not a guest success/status code.
            return Ok(0);
        }
        let first = args[0] as u32 as i32;
        if function == Function::VideoOpen {
            if !matches!(first, 0 | 255) {
                return Err(Failure::UnsupportedVideo);
            }
            let bus = args[1] as u32 as i32;
            let Some(slot) = usize::try_from(bus).ok().filter(|i| *i < 3) else {
                return Ok(INVALID_VALUE);
            };
            if args[2] as u32 as i32 != 0 {
                return Err(Failure::UnsupportedVideo);
            }
            if self.ports[slot].open {
                return Ok(BUSY);
            }
            let generation = self.ports[slot]
                .state
                .generation
                .checked_add(1)
                .ok_or(Failure::Limit)?;
            self.ports[slot] = Port {
                open: true,
                state: PortState {
                    generation,
                    gamma_bits: self.ports[slot].state.gamma_bits,
                    ..Default::default()
                },
                buffers: Default::default(),
                vblank: VblankStatus::default(),
                grid: None,
            };
            self.presentation.reset(slot);
            // The optional source parameter is ignored, never dereferenced.
            return Ok(slot as u64 + 1);
        }
        if function == Function::VideoInitializeOptions {
            if args[0] == 0 {
                return Ok(INVALID_ADDRESS);
            }
            output(memory, args[0], &[0; 64])?;
            return Ok(0);
        }
        // The source checks the output pointer before even looking up a port.
        if function == Function::VideoGetOutputStatus && args[1] == 0 {
            return Ok(INVALID_ADDRESS);
        }
        let Some(slot) = index(first).filter(|i| self.ports[*i].open) else {
            return Ok(INVALID_HANDLE);
        };
        match function {
            Function::VideoRegisterBuffers2 => {
                self.ports[slot].buffers.register(args, stack, memory)
            }
            Function::VideoChangeBufferAttribute2 => self.ports[slot].buffers.change(args, memory),
            Function::VideoUnregisterBuffers => {
                // The source waits for pending flips before retiring buffers.
                // Cooperative callers retain the request until it completes.
                if self
                    .flip_status(first)
                    .is_some_and(|status| status.pending != 0)
                {
                    return Err(Failure::Waiting);
                }
                Ok(self.ports[slot].buffers.unregister(args[1] as u32 as i32))
            }
            Function::VideoGetOutputStatus => {
                let detects_resolution = display.application_attribute3.unwrap_or(0) & 4 != 0;
                let resolution: u32 = if detects_resolution
                    && display.resolution.width < 3840
                    && display.resolution.height < 2160
                {
                    1
                } else {
                    2
                };
                let fast = self.ports[slot].state.output_mode == 15
                    || display.refresh_follows_vblank && vblank_hz >= 119;
                let refresh: u64 = if fast { 13 } else { 3 };
                let mut bytes = [0; 48];
                bytes[..4].copy_from_slice(&resolution.to_le_bytes());
                bytes[4..8].copy_from_slice(&1_u32.to_le_bytes());
                bytes[8..16].copy_from_slice(&refresh.to_le_bytes());
                output(memory, args[1], &bytes)?;
                Ok(0)
            }
            Function::VideoClose => {
                if self.ports[slot].state.slave_count != 0 {
                    return Err(Failure::UnsupportedVideo);
                }
                let generation = self.ports[slot]
                    .state
                    .generation
                    .checked_add(1)
                    .ok_or(Failure::Limit)?;
                let master = self.ports[slot].state.master_handle;
                let master_slot = master.and_then(index);
                if master_slot
                    .is_some_and(|i| !self.ports[i].open || self.ports[i].state.slave_count == 0)
                {
                    return Err(Failure::UnsupportedVideo);
                }
                // Cancel retained requests and prepared pixels before the
                // generation changes. Exported stale frames cannot acknowledge.
                self.cancel_flips(first);
                self.ports[slot] = Port {
                    open: false,
                    state: PortState {
                        generation,
                        gamma_bits: self.ports[slot].state.gamma_bits,
                        ..Default::default()
                    },
                    buffers: Default::default(),
                    vblank: VblankStatus::default(),
                    grid: None,
                };
                if let Some(i) = master_slot {
                    self.ports[i].state.slave_count -= 1;
                }
                Ok(0)
            }
            Function::VideoSetFlipRate => {
                let rate = args[1] as u32 as i32;
                if !(0..=2).contains(&rate) {
                    return Ok(INVALID_VALUE);
                }
                self.ports[slot].state.flip_rate = rate as u32;
                Ok(0)
            }
            Function::VideoSetFlipMaster => {
                let master_handle = args[1] as u32 as i32;
                let Some(master) = index(master_handle).filter(|i| self.ports[*i].open) else {
                    return Ok(INVALID_HANDLE);
                };
                if master == slot
                    || self.ports[slot].state.master_handle.is_some()
                    || self.ports[slot].state.slave_count != 0
                    || self.ports[master].state.master_handle.is_some()
                {
                    return Ok(UNSUPPORTED_OPERATION);
                }
                if self
                    .flip_status(first)
                    .is_some_and(|status| status.pending != 0)
                    || self
                        .flip_status(master_handle)
                        .is_some_and(|status| status.pending != 0)
                {
                    return Ok(BUSY);
                }
                let slaves = self.ports[master]
                    .state
                    .slave_count
                    .checked_add(1)
                    .ok_or(Failure::Limit)?;
                self.ports[slot].state.master_handle = Some(master_handle);
                self.ports[master].state.slave_count = slaves;
                Ok(0)
            }
            Function::VideoIsOutputSupported | Function::VideoConfigureOutput => {
                if args[3] != 0 || args[4] != 0 {
                    return Ok(INVALID_VALUE);
                }
                if args[2] != 0 {
                    // Preserve the source's first-nonzero-word error ordering;
                    // no later word is read once invalid options are known.
                    for offset in (0..64).step_by(4) {
                        let address = args[2].checked_add(offset).ok_or(Failure::Memory)?;
                        let mut word = [0; 4];
                        memory
                            .read(address, &mut word)
                            .map_err(|_| Failure::Memory)?;
                        if word != [0; 4] {
                            return Ok(INVALID_OPTION);
                        }
                    }
                }
                let mode = args[1];
                if !matches!(mode, 1 | 15) {
                    return Ok(UNSUPPORTED_MODE);
                }
                let supported = mode != 15 || vblank_hz >= 119;
                if function == Function::VideoIsOutputSupported {
                    return Ok(u64::from(supported));
                }
                if !supported {
                    return Ok(UNAVAILABLE_MODE);
                }
                // No output-mode event registrations are admitted yet. Future
                // event delivery must be part of this same state publication.
                self.ports[slot].state.output_mode = mode;
                Ok(0)
            }
            _ => Err(Failure::UnsupportedVideo),
        }
    }
}

pub(crate) fn supported(function: Function) -> bool {
    matches!(
        function,
        Function::VideoOpen
            | Function::VideoClose
            | Function::VideoSetFlipMaster
            | Function::VideoSetFlipRate
            | Function::VideoInitializeOptions
            | Function::VideoIsOutputSupported
            | Function::VideoConfigureOutput
            | Function::VideoGetOutputStatus
            | Function::VideoSetBufferAttribute2
            | Function::VideoAddBufferAttributeOption
            | Function::VideoRegisterBuffers2
            | Function::VideoChangeBufferAttribute2
            | Function::VideoUnregisterBuffers
    )
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "video/tests.rs"]
mod tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "video/buffer_tests.rs"]
mod buffer_tests;

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "video/presentation_tests.rs"]
mod presentation_tests;
