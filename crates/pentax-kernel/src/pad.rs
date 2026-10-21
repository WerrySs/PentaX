// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: owned controller snapshots and checked transactions.
// Adapted from Kyty and AnyPS5 contributors; see NOTICE for exact revisions.

//! A device boundary, not a host controller driver. Inputs come from the
//! kernel owner; outputs are accepted requests, not hardware delivery.
use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service};

const INVALID_ARGUMENT: u64 = 0x8092_0001;
const INVALID_HANDLE: u64 = 0x8092_0003;
const NO_HANDLE: u64 = 0x8092_0008;
const QUEUE_CAPACITY: usize = 64;
const RECORD_BYTES: usize = 120;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadTouch {
    pub x: u16,
    pub y: u16,
    pub id: u8,
}

/// Resolved input in guest units: acceleration in g and angular velocity in
/// radians/second. The supplying owner is responsible for any motion filter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadInputSnapshot {
    pub buttons: u32,
    pub sticks: [u8; 4],
    pub triggers: [u8; 2],
    pub orientation: [f32; 4],
    pub acceleration: [f32; 3],
    pub angular_velocity: [f32; 3],
    pub touches: [PadTouch; 2],
    pub touch_count: u8,
    pub connected: bool,
    pub connected_count: u8,
    /// Change time in microseconds from this process's owned monotonic epoch.
    pub timestamp: u64,
    pub extension_id: u32,
    pub extension_length: u8,
    pub extension: [u8; 10],
    pub unique_length: u8,
    pub unique: [u8; 12],
}
impl Default for PadInputSnapshot {
    fn default() -> Self {
        Self {
            buttons: 0,
            sticks: [128; 4],
            triggers: [0; 2],
            orientation: [0.0, 0.0, 0.0, 1.0],
            acceleration: [0.0, 1.0, 0.0],
            angular_velocity: [0.0; 3],
            touches: [PadTouch::default(); 2],
            touch_count: 0,
            connected: false,
            connected_count: 0,
            timestamp: 0,
            extension_id: 0,
            extension_length: 0,
            extension: [0; 10],
            unique_length: 0,
            unique: [0; 12],
        }
    }
}
impl PadInputSnapshot {
    fn valid(self) -> bool {
        self.touch_count <= 2
            && self.extension_length <= 10
            && self.unique_length <= 12
            && (self.connected || self.connected_count == 0)
            && self
                .orientation
                .iter()
                .chain(&self.acceleration)
                .chain(&self.angular_velocity)
                .all(|value| value.is_finite())
    }
    fn encode(self) -> [u8; RECORD_BYTES] {
        let mut bytes = [0; RECORD_BYTES];
        bytes[..4].copy_from_slice(&self.buttons.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.sticks);
        bytes[8..10].copy_from_slice(&self.triggers);
        for (index, value) in self
            .orientation
            .iter()
            .chain(&self.acceleration)
            .chain(&self.angular_velocity)
            .enumerate()
        {
            bytes[12 + index * 4..16 + index * 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[52] = self.touch_count;
        for (index, touch) in self.touches[..usize::from(self.touch_count)]
            .iter()
            .enumerate()
        {
            let at = 60 + index * 8;
            bytes[at..at + 2].copy_from_slice(&touch.x.to_le_bytes());
            bytes[at + 2..at + 4].copy_from_slice(&touch.y.to_le_bytes());
            bytes[at + 4] = touch.id;
        }
        bytes[76] = u8::from(self.connected);
        bytes[80..88].copy_from_slice(&self.timestamp.to_le_bytes());
        bytes[88..92].copy_from_slice(&self.extension_id.to_le_bytes());
        bytes[93] = self.extension_length;
        let length = usize::from(self.extension_length);
        bytes[94..94 + length].copy_from_slice(&self.extension[..length]);
        bytes[104] = self.connected_count;
        bytes[107] = self.unique_length;
        let length = usize::from(self.unique_length);
        bytes[108..108 + length].copy_from_slice(&self.unique[..length]);
        bytes
    }
}

/// Static configured characteristics; connection is taken from actual input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadControllerInformation {
    pub pixel_density: f32,
    pub resolution: [u16; 2],
    pub stick_dead_zones: [u8; 2],
    pub connection_type: u8,
    pub device_class: i32,
}
impl Default for PadControllerInformation {
    fn default() -> Self {
        Self {
            pixel_density: 44.86,
            resolution: [1920, 1080],
            stick_dead_zones: [2, 2],
            connection_type: 0,
            device_class: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PadTriggerRequest {
    pub mode: u32,
    pub effect: [u8; 11],
    pub fallback: u8,
    pub valid: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadOutputState {
    pub vibration: [u8; 2],
    pub light_bar: [u8; 3],
    pub light_bar_valid: bool,
    pub triggers: [PadTriggerRequest; 2],
    pub trigger_touched: bool,
    pub tilt_correction: bool,
    /// Every accepted tilt setter requests that the supplying motion owner
    /// reset its correction integrals, including repeated equal settings.
    pub tilt_reset_sequence: u64,
    /// Checked change serial, not an acknowledgement from hardware.
    pub sequence: u64,
}
impl Default for PadOutputState {
    fn default() -> Self {
        Self {
            vibration: [0; 2],
            light_bar: [0; 3],
            light_bar_valid: false,
            triggers: [PadTriggerRequest::default(); 2],
            trigger_touched: false,
            tilt_correction: true,
            tilt_reset_sequence: 0,
            sequence: 0,
        }
    }
}
pub(crate) struct PadState {
    initialized: bool,
    current: PadInputSnapshot,
    information: PadControllerInformation,
    // Fixed bounded storage; publication requires no host allocation.
    queue: [PadInputSnapshot; QUEUE_CAPACITY],
    head: usize,
    length: usize,
    output: PadOutputState,
}
impl Default for PadState {
    fn default() -> Self {
        Self {
            initialized: false,
            current: PadInputSnapshot::default(),
            information: PadControllerInformation::default(),
            queue: [PadInputSnapshot::default(); QUEUE_CAPACITY],
            head: 0,
            length: 0,
            output: PadOutputState::default(),
        }
    }
}
impl PadState {
    pub(crate) const fn initialized(&self) -> bool {
        self.initialized
    }
    pub(crate) fn initialize(&mut self, timestamp: u64) {
        if !self.initialized {
            self.current.timestamp = timestamp;
            self.head = 0;
            self.length = 0;
            self.initialized = true;
        }
    }
    pub(crate) const fn output(&self) -> PadOutputState {
        self.output
    }
    pub(crate) fn publish(
        &mut self,
        snapshot: PadInputSnapshot,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        budget.work(RECORD_BYTES as u64)?;
        if !self.initialized || !snapshot.valid() || snapshot.timestamp < self.current.timestamp {
            return Err(Failure::Unsupported("invalid controller snapshot"));
        }
        if snapshot == self.current {
            return Ok(());
        }
        // Source policy: retain the newest 64 real input changes.
        if self.length == QUEUE_CAPACITY {
            self.head = (self.head + 1) % QUEUE_CAPACITY;
            self.length -= 1;
        }
        self.queue[(self.head + self.length) % QUEUE_CAPACITY] = snapshot;
        self.length += 1;
        self.current = snapshot;
        Ok(())
    }
    pub(crate) fn configure_information(
        &mut self,
        information: PadControllerInformation,
        budget: &mut Budget,
    ) -> Result<(), Failure> {
        budget.work(1)?;
        if self.initialized
            || !information.pixel_density.is_finite()
            || information.pixel_density <= 0.0
            || information.resolution.contains(&0)
        {
            return Err(Failure::Unsupported("controller information profile"));
        }
        self.information = information;
        Ok(())
    }
    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        configured_user: i32,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [first, second, third, ..] = args.0;
        match service {
            Service::PadClose => return Ok(0),
            Service::PadOpen | Service::PadGetHandle => {
                let user = first as u32 as i32;
                let port = second as u32 as i32;
                let personal = user == configured_user && matches!(port, 0 | 2);
                let system_remote = user == 0xff && port == 16;
                return Ok(if third as u32 as i32 == 0 && (personal || system_remote) {
                    1
                } else if service == Service::PadOpen {
                    INVALID_ARGUMENT
                } else {
                    NO_HANDLE
                });
            }
            _ => {}
        }
        if first as u32 as i32 != 1 {
            return Ok(INVALID_HANDLE);
        }
        if service == Service::PadSetTiltCorrectionState {
            let reset = self
                .output
                .tilt_reset_sequence
                .checked_add(1)
                .ok_or(Failure::Limit("controller tilt reset sequence"))?;
            self.output.tilt_correction = second as u8 != 0;
            self.output.tilt_reset_sequence = reset;
            return Ok(0);
        }
        if second == 0 {
            return Ok(INVALID_ARGUMENT);
        }
        match service {
            Service::PadIsRemote => {
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &[0],
                    }],
                )?;
            }
            Service::PadGetTriggerEffectState => {
                // No hardware acknowledgement is supplied by the reference.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &[0; 8],
                    }],
                )?;
            }
            Service::PadSetVibration => {
                let mut bytes = [0; 2];
                budget.read(guest, second, &mut bytes)?;
                if self.output.vibration != bytes {
                    let sequence = self.next_sequence(1)?;
                    self.output.vibration = bytes;
                    self.output.sequence = sequence;
                }
            }
            Service::PadSetLightBar => {
                let mut bytes = [0; 3];
                budget.read(guest, second, &mut bytes)?;
                let sequence = self.next_sequence(1)?;
                self.output.light_bar = bytes;
                self.output.light_bar_valid = true;
                self.output.sequence = sequence;
            }
            Service::PadSetTriggerEffect => {
                let mut bytes = [0; 120];
                budget.read(guest, second, &mut bytes)?;
                let mask = bytes[0];
                let modes = [
                    u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
                    u32::from_le_bytes(bytes[64..68].try_into().unwrap()),
                ];
                if mask & !3 != 0 || modes.iter().any(|mode| *mode > 6) {
                    return Ok(INVALID_ARGUMENT);
                }
                budget.work(20)?;
                let sequence = self.next_sequence(u64::from(mask.count_ones()))?;
                let mut triggers = self.output.triggers;
                for (index, trigger) in triggers.iter_mut().enumerate() {
                    if mask & (1 << index) != 0 {
                        *trigger =
                            decode_trigger(modes[index], &bytes[16 + index * 56..64 + index * 56]);
                    }
                }
                self.output.triggers = triggers;
                self.output.trigger_touched |= mask != 0;
                self.output.sequence = sequence;
            }
            Service::PadGetControllerInformation => {
                let mut bytes = [0; 28];
                bytes[..4].copy_from_slice(&self.information.pixel_density.to_le_bytes());
                bytes[4..6].copy_from_slice(&self.information.resolution[0].to_le_bytes());
                bytes[6..8].copy_from_slice(&self.information.resolution[1].to_le_bytes());
                bytes[8..10].copy_from_slice(&self.information.stick_dead_zones);
                bytes[10] = self.information.connection_type;
                bytes[11] = self.current.connected_count;
                bytes[12] = u8::from(self.current.connected);
                bytes[16..20].copy_from_slice(&self.information.device_class.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &bytes,
                    }],
                )?;
            }
            Service::PadRead => {
                if third as u32 as i32 <= 0 {
                    return Ok(INVALID_ARGUMENT);
                }
                if !self.initialized {
                    return Err(Failure::Unsupported(
                        "controller read before initialization",
                    ));
                }
                let count = self.length.min(third as u32 as usize).max(1);
                let length = count * RECORD_BYTES;
                if length > budget.limits.max_io_bytes {
                    return Err(Failure::Limit("controller read bytes"));
                }
                let mut bytes = budget.buffer(length)?;
                for index in 0..count {
                    let snapshot = if self.length == 0 {
                        self.current
                    } else {
                        self.queue[(self.head + index) % QUEUE_CAPACITY]
                    };
                    bytes[index * RECORD_BYTES..(index + 1) * RECORD_BYTES]
                        .copy_from_slice(&snapshot.encode());
                }
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: second,
                        bytes: &bytes,
                    }],
                )?;
                // Refused output never consumes real input. No guest access
                // follows this transaction's state commit.
                if self.length != 0 {
                    self.head = (self.head + count) % QUEUE_CAPACITY;
                    self.length -= count;
                }
                return Ok(count as u64);
            }
            _ => return Err(Failure::Unsupported("controller entry service")),
        }
        Ok(0)
    }
    fn next_sequence(&self, count: u64) -> Result<u64, Failure> {
        self.output
            .sequence
            .checked_add(count)
            .ok_or(Failure::Limit("controller output sequence"))
    }
}
fn scale_strength(strength: u8) -> u8 {
    (u16::from(strength.min(8)) * 32).min(255) as u8
}
fn encode_zones(request: &mut PadTriggerRequest, mode: u8, strengths: &[u8; 10], frequency: u8) {
    let mut zones = 0u16;
    let mut packed = 0u32;
    for (index, strength) in strengths.iter().copied().enumerate() {
        if (1..=8).contains(&strength) {
            zones |= 1 << index;
            packed |= u32::from(strength - 1) << (index * 3);
        }
    }
    if zones == 0 || (mode == 0x26 && frequency == 0) {
        return;
    }
    request.effect[0] = mode;
    request.effect[1..3].copy_from_slice(&zones.to_le_bytes());
    request.effect[3..7].copy_from_slice(&packed.to_le_bytes());
    request.effect[9] = frequency;
}
fn decode_trigger(mode: u32, parameters: &[u8]) -> PadTriggerRequest {
    let mut request = PadTriggerRequest {
        mode,
        ..Default::default()
    };
    request.effect[0] = 0x05;
    let mut strengths = [0; 10];
    match mode {
        1 | 3 => {
            if parameters[0] <= 9 && (1..=8).contains(&parameters[1]) {
                strengths[usize::from(parameters[0])..].fill(parameters[1]);
            }
            encode_zones(
                &mut request,
                if mode == 1 { 0x21 } else { 0x26 },
                &strengths,
                if mode == 1 { 0 } else { parameters[2] },
            );
            request.fallback = scale_strength(parameters[1]);
        }
        2 => {
            let [start, end, strength] = [parameters[0], parameters[1], parameters[2]];
            if (2..=7).contains(&start) && end > start && end <= 8 && (1..=8).contains(&strength) {
                request.effect[0] = 0x25;
                request.effect[1..3]
                    .copy_from_slice(&((1u16 << start) | (1u16 << end)).to_le_bytes());
                request.effect[3] = strength - 1;
            }
            request.fallback = scale_strength(strength);
        }
        4 | 6 => {
            let offset = usize::from(mode == 6);
            strengths.copy_from_slice(&parameters[offset..offset + 10]);
            encode_zones(
                &mut request,
                if mode == 4 { 0x21 } else { 0x26 },
                &strengths,
                if mode == 4 { 0 } else { parameters[0] },
            );
            request.fallback = if mode == 6 && parameters[0] == 0 {
                0
            } else {
                scale_strength(*strengths.iter().max().unwrap())
            };
        }
        5 => {
            let [start, end, first, last] =
                [parameters[0], parameters[1], parameters[2], parameters[3]];
            if start <= 8
                && end > start
                && end <= 9
                && (1..=8).contains(&first)
                && (1..=8).contains(&last)
            {
                for (index, strength) in strengths.iter_mut().enumerate().skip(usize::from(start)) {
                    // Positive rational lround, independent of host FP controls.
                    let distance = u32::from(end - start);
                    let position = (index as u32 - u32::from(start)).min(distance);
                    let numerator =
                        u32::from(first) * (distance - position) + u32::from(last) * position;
                    *strength = ((2 * numerator + distance) / (2 * distance)) as u8;
                }
            }
            encode_zones(&mut request, 0x21, &strengths, 0);
            request.fallback = scale_strength(first.max(last));
        }
        _ => {}
    }
    request.valid = request.effect[0] != 0x05;
    if !request.valid {
        request.fallback = 0;
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_trigger_modes_decode_original_literal_zones_frequencies_and_ties() {
        for mode in 0..=6 {
            let mut parameters = [0; 48];
            match mode {
                1 | 3 => parameters[..3].copy_from_slice(&[8, 2, 7]),
                2 => parameters[..3].copy_from_slice(&[2, 5, 8]),
                4 => parameters[..2].copy_from_slice(&[1, 2]),
                5 => parameters[..4].copy_from_slice(&[0, 2, 1, 2]),
                6 => parameters[..3].copy_from_slice(&[9, 1, 2]),
                _ => {}
            }
            let request = decode_trigger(mode, &parameters);
            assert_eq!(request.mode, mode);
            assert_eq!(request.valid, mode != 0);
            match mode {
                0 => assert_eq!((request.effect[0], request.fallback), (5, 0)),
                1 | 3 => {
                    assert_eq!(
                        &request.effect[..7],
                        &[if mode == 1 { 0x21 } else { 0x26 }, 0, 3, 0, 0, 0, 9]
                    );
                    assert_eq!(request.effect[9], if mode == 1 { 0 } else { 7 });
                    assert_eq!(request.fallback, 64);
                }
                2 => {
                    assert_eq!(&request.effect[..4], &[0x25, 0x24, 0, 7]);
                    assert_eq!(request.fallback, 255);
                }
                4 | 6 => {
                    assert_eq!(
                        &request.effect[..7],
                        &[if mode == 4 { 0x21 } else { 0x26 }, 3, 0, 8, 0, 0, 0]
                    );
                    assert_eq!(request.effect[9], if mode == 4 { 0 } else { 9 });
                    assert_eq!(request.fallback, 64);
                }
                5 => {
                    assert_eq!(&request.effect[..3], &[0x21, 0xff, 3]);
                    let packed = u32::from_le_bytes(request.effect[3..7].try_into().unwrap());
                    for (index, expected) in [0, 1, 1, 1, 1, 1, 1, 1, 1, 1].into_iter().enumerate()
                    {
                        assert_eq!((packed >> (index * 3)) & 7, expected);
                    }
                    assert_eq!(request.fallback, 64);
                }
                _ => unreachable!(),
            }
        }
        let mut parameters = [0; 48];
        parameters[..3].copy_from_slice(&[8, 2, 0]);
        assert!(!decode_trigger(3, &parameters).valid);
        parameters[..3].copy_from_slice(&[9, 1, 2]);
        assert!(!decode_trigger(2, &parameters).valid);
    }

    #[test]
    fn sequence_overflow_and_repeated_tilt_reset_refuse_or_commit_without_partial_state() {
        let mut k = crate::tests::kernel(Default::default());
        let mut g = crate::tests::TestGuest::new();
        k.pad.output.sequence = u64::MAX;
        let before = k.pad.output;
        g.put(0, &[3; 120]);
        for service in [Service::PadSetVibration, Service::PadSetLightBar] {
            assert!(matches!(
                k.invoke(
                    service,
                    CallArgs([1, crate::tests::ARG, 0, 0, 0, 0]),
                    &mut g
                ),
                Err(Failure::Limit(_))
            ));
            assert_eq!(k.pad.output, before);
        }
        let before_reset = k.pad.output.tilt_reset_sequence;
        k.invoke(
            Service::PadSetTiltCorrectionState,
            CallArgs([1, 1, 0, 0, 0, 0]),
            &mut g,
        )
        .unwrap();
        assert_eq!(k.pad.output.tilt_reset_sequence, before_reset + 1);
        k.invoke(
            Service::PadSetTiltCorrectionState,
            CallArgs([1, 1, 0, 0, 0, 0]),
            &mut g,
        )
        .unwrap();
        assert_eq!(k.pad.output.tilt_reset_sequence, before_reset + 2);
        k.pad.output.tilt_reset_sequence = u64::MAX;
        let before = k.pad.output;
        assert!(matches!(
            k.invoke(
                Service::PadSetTiltCorrectionState,
                CallArgs([1, 0, 0, 0, 0, 0]),
                &mut g
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(k.pad.output, before);
    }
}
