// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned color state and actual observed vblank snapshots.
// Adapted from pinned videoOut.cpp; see core NOTICE.
use super::*;

/// Exact source layout, populated by elapsed boundaries of the owned display
/// clock. Counts do not measure presented frames or acknowledge any frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VblankStatus {
    pub count: u64,
    pub process_time: u64,
    pub tsc: u64,
    pub process_counter: u64,
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests;
impl VblankStatus {
    pub fn bytes(self) -> [u8; 40] {
        let mut bytes = [0; 40];
        for (offset, value) in [
            (0, self.count),
            (8, self.process_time),
            (16, self.tsc),
            (24, self.process_counter),
        ] {
            bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        // The pinned reference never changes flags, phase or its six pad bytes.
        bytes
    }
}

impl State {
    pub(crate) fn color_or_vblank(
        &mut self,
        function: Function,
        args: [u64; 6],
        vectors: Option<[[u8; 16]; 8]>,
        memory: &mut PausedNativeMemory<'_>,
    ) -> Result<u64, Failure> {
        self.check_owner(memory.owner_id())?;
        if function == Function::VideoColorSettingsSetGamma {
            if args[0] == 0 {
                return Ok(INVALID_ADDRESS);
            }
            let vectors = vectors.ok_or(Failure::Kernel(pentax_kernel::Failure::Unsupported(
                "captured VideoOut gamma XMM0 argument",
            )))?;
            let bits = u32::from_le_bytes(vectors[0][..4].try_into().unwrap());
            let nan = (bits & 0x7fff_ffff) > 0x7f80_0000;
            // Exact unordered IEEE comparisons from `gamma <0.1f || >2.0f`.
            // Integer classification avoids touching the host FP environment;
            // source NaNs remain accepted and their actual payload is retained.
            if !nan && !(0x3dcc_cccd..=0x4000_0000).contains(&bits) {
                return Ok(INVALID_VALUE);
            }
            output(memory, args[0], &bits.to_le_bytes())?;
            return Ok(0);
        }
        if args[1] == 0 {
            return Ok(INVALID_ADDRESS);
        }
        let Some(slot) = index(args[0] as u32 as i32).filter(|i| self.ports[*i].open) else {
            return Ok(INVALID_HANDLE);
        };
        match function {
            Function::VideoGetVblankStatus => {
                output(memory, args[1], &self.ports[slot].vblank.bytes())?
            }
            Function::VideoAdjustColor => {
                let mut bytes = [0; 4];
                memory
                    .read(args[1], &mut bytes)
                    .map_err(|_| Failure::Memory)?;
                // The source applies the supplied field without revalidating
                // gamma or dereferencing the three reserved settings words.
                self.ports[slot].state.gamma_bits = u32::from_le_bytes(bytes);
            }
            _ => return Err(Failure::UnsupportedVideo),
        }
        Ok(0)
    }
}

/// One per-open display epoch. Values use the kernel's already admitted TSC
/// frequency/process epoch; no per-query increment or second timer is involved.
#[derive(Clone, Copy)]
pub(super) struct DisplayGrid {
    counter: u64,
    raw: u64,
    frequency: u64,
    rate: u32,
    base_count: u64,
}
#[derive(Clone, Copy)]
struct ClockPort {
    state: PortState,
    open: bool,
    vblank: VblankStatus,
    grid: Option<DisplayGrid>,
}
impl ClockPort {
    fn from_port(port: &Port) -> Self {
        Self {
            state: port.state,
            open: port.open,
            vblank: port.vblank,
            grid: port.grid,
        }
    }
}
pub(crate) struct ClockedCall {
    ports: [ClockPort; 3],
    pub(crate) result: u64,
    open_slot: Option<usize>,
    query: Option<(usize, u64)>,
    close_slot: Option<usize>,
}
pub(crate) struct GridUpdate {
    ports: [ClockPort; 3],
    pub(crate) boundaries: [Option<pentax_kernel::VideoBoundary>; 3],
    open_slot: Option<usize>,
    query: Option<(usize, u64)>,
    pub(crate) close: Option<(i32, u64)>,
}
impl DisplayGrid {
    fn new(
        clock: pentax_kernel::DisplayClock,
        rate: u32,
        base_count: u64,
    ) -> Result<Self, Failure> {
        if clock.frequency == 0 || rate == 0 {
            return Err(Failure::UnsupportedVideo);
        }
        Ok(Self {
            counter: clock.counter,
            raw: clock.raw_ticks,
            frequency: clock.frequency,
            rate,
            base_count,
        })
    }
    fn observe(
        self,
        clock: pentax_kernel::DisplayClock,
    ) -> Result<(VblankStatus, std::time::Duration), Failure> {
        if clock.frequency != self.frequency {
            return Err(Failure::UnsupportedVideo);
        }
        let elapsed = clock
            .counter
            .checked_sub(self.counter)
            .ok_or(Failure::UnsupportedVideo)?;
        if clock.raw_ticks.checked_sub(self.raw) != Some(elapsed) {
            return Err(Failure::UnsupportedVideo);
        }
        let periods = u128::from(elapsed) * u128::from(self.rate) / u128::from(self.frequency);
        let count = self
            .base_count
            .checked_add(u64::try_from(periods).map_err(|_| Failure::Limit)?)
            .ok_or(Failure::Limit)?;
        let boundary = periods
            .checked_mul(u128::from(self.frequency))
            .ok_or(Failure::Limit)?
            .div_ceil(u128::from(self.rate));
        let boundary = u64::try_from(boundary).map_err(|_| Failure::Limit)?;
        let counter = self.counter.checked_add(boundary).ok_or(Failure::Limit)?;
        let tsc = self.raw.checked_add(boundary).ok_or(Failure::Limit)?;
        let process_time =
            u64::try_from(u128::from(counter) * 1_000_000 / u128::from(self.frequency))
                .map_err(|_| Failure::Limit)?;
        let next = (periods + 1)
            .checked_mul(u128::from(self.frequency))
            .ok_or(Failure::Limit)?
            .div_ceil(u128::from(self.rate));
        let remaining = next
            .checked_sub(u128::from(elapsed))
            .ok_or(Failure::Limit)?;
        let nanos = remaining
            .checked_mul(1_000_000_000)
            .ok_or(Failure::Limit)?
            .div_ceil(u128::from(self.frequency));
        let delay = std::time::Duration::from_nanos(
            u64::try_from(nanos).map_err(|_| Failure::Limit)?.max(1),
        );
        Ok((
            VblankStatus {
                count,
                process_time,
                tsc,
                process_counter: counter,
            },
            delay,
        ))
    }
}
impl State {
    pub(crate) fn grid_work() -> u64 {
        (std::mem::size_of::<[ClockPort; 3]>() as u64) * 8 + 1024
    }
    pub(crate) fn call_work() -> u64 {
        (std::mem::size_of::<[Port; 3]>() as u64) * 2
    }
    pub(crate) fn clocked_generations(&self) -> [Option<u64>; 3] {
        self.ports
            .each_ref()
            .map(|port| (port.open && port.grid.is_some()).then_some(port.state.generation))
    }
    pub(crate) fn has_clocked_port(&self) -> bool {
        self.ports
            .iter()
            .any(|port| port.open && port.grid.is_some())
    }
    pub(crate) fn prepare_clocked_call(
        &self,
        function: Function,
        args: [u64; 6],
        hz: u32,
        display: DisplayPolicy,
        memory: &mut PausedNativeMemory<'_>,
    ) -> Result<ClockedCall, Failure> {
        self.check_owner(memory.owner_id())?;
        if !matches!(
            function,
            Function::VideoOpen
                | Function::VideoClose
                | Function::VideoConfigureOutput
                | Function::VideoGetVblankStatus
        ) {
            return Err(Failure::UnsupportedVideo);
        }
        if function == Function::VideoGetVblankStatus {
            let result = if args[1] == 0 {
                INVALID_ADDRESS
            } else if index(args[0] as u32 as i32).is_none_or(|slot| !self.ports[slot].open) {
                INVALID_HANDLE
            } else {
                0
            };
            return Ok(ClockedCall {
                ports: self.ports.each_ref().map(ClockPort::from_port),
                result,
                open_slot: None,
                close_slot: None,
                query: (result == 0)
                    .then_some((index(args[0] as u32 as i32).unwrap_or(0), args[1])),
            });
        }
        // The source validations inspect a fixed-size metadata copy. No actual
        // presentation queue, registration, or open generation changes here.
        let mut staged = State {
            ports: self.ports,
            presentation: Default::default(),
        };
        let result = staged.invoke(function, args, hz, display, None, memory)?;
        let open_slot = (function == Function::VideoOpen && (1..=3).contains(&result))
            .then(|| (result - 1) as usize);
        Ok(ClockedCall {
            ports: staged.ports.each_ref().map(ClockPort::from_port),
            result,
            open_slot,
            query: None,
            close_slot: (function == Function::VideoClose && result == 0)
                .then(|| index(args[0] as u32 as i32).unwrap()),
        })
    }
    pub(crate) fn prepare_grid(
        &self,
        clock: pentax_kernel::DisplayClock,
        hz: u32,
        display: DisplayPolicy,
        call: Option<ClockedCall>,
    ) -> Result<GridUpdate, Failure> {
        let mut ports = self.ports.each_ref().map(ClockPort::from_port);
        let mut boundaries = [None; 3];
        for (slot, port) in ports.iter_mut().enumerate() {
            if !port.open {
                continue;
            }
            let Some(grid) = port.grid else {
                return Err(Failure::UnsupportedVideo);
            };
            let (status, _) = grid.observe(clock)?;
            let occurrences = status
                .count
                .checked_sub(port.vblank.count)
                .ok_or(Failure::UnsupportedVideo)?;
            if occurrences != 0 {
                boundaries[slot] = Some(pentax_kernel::VideoBoundary {
                    handle: slot as i32 + 1,
                    generation: port.state.generation,
                    count: status.count,
                    tsc: status.tsc,
                    occurrences,
                });
                port.vblank = status;
            }
        }
        let mut update = GridUpdate {
            ports,
            boundaries,
            open_slot: None,
            query: None,
            close: None,
        };
        if let Some(call) = call {
            update.open_slot = call.open_slot;
            update.query = call.query;
            update.close = call
                .close_slot
                .map(|slot| (slot as i32 + 1, self.ports[slot].state.generation));
            if let Some(slot) = call.close_slot {
                update.boundaries[slot] = None;
            }
            // Only open/configuration metadata can differ; retained snapshots
            // computed above must not be overwritten by the validation copy.
            for (slot, port) in update.ports.iter_mut().enumerate() {
                port.state = call.ports[slot].state;
                if Some(slot) == call.open_slot || Some(slot) == call.close_slot {
                    *port = call.ports[slot];
                }
            }
        }
        let rate = if display.refresh_follows_vblank {
            hz.max(1)
        } else if update.ports[0].open && update.ports[0].state.output_mode == 15 {
            120
        } else {
            60
        };
        for port in &mut update.ports {
            if port.open && port.grid.is_none_or(|grid| grid.rate != rate) {
                port.grid = Some(DisplayGrid::new(clock, rate, port.vblank.count)?);
            }
        }
        Ok(update)
    }
    pub(crate) fn grid_wait_delay(
        &self,
        clock: pentax_kernel::DisplayClock,
        subscriptions: [bool; 3],
    ) -> Result<Option<std::time::Duration>, Failure> {
        let mut delay: Option<std::time::Duration> = None;
        for (slot, port) in self.ports.iter().enumerate() {
            if !port.open || !subscriptions[slot] {
                continue;
            }
            let (_, next) = port.grid.ok_or(Failure::UnsupportedVideo)?.observe(clock)?;
            delay = Some(delay.map_or(next, |old| old.min(next)));
        }
        Ok(delay)
    }
    pub(crate) fn commit_grid(
        &mut self,
        update: GridUpdate,
        memory: &mut PausedNativeMemory<'_>,
    ) -> Result<(), Failure> {
        if let Some((slot, address)) = update.query {
            output(memory, address, &update.ports[slot].vblank.bytes())?;
        }
        // All fallible reads, event admission and optional guest output precede
        // both owned port publication and the caller's infallible event commit.
        if let Some(slot) = update.open_slot {
            self.presentation.bind_owner(memory.owner_id())?;
            self.presentation.reset(slot);
        }
        if let Some((handle, _)) = update.close {
            self.cancel_flips(handle);
        }
        for (slot, metadata) in update.ports.into_iter().enumerate() {
            let port = &mut self.ports[slot];
            port.state = metadata.state;
            port.open = metadata.open;
            port.vblank = metadata.vblank;
            port.grid = metadata.grid;
            if update.open_slot == Some(slot)
                || update
                    .close
                    .is_some_and(|(handle, _)| handle == slot as i32 + 1)
            {
                port.buffers = Default::default();
            }
        }
        Ok(())
    }
}
