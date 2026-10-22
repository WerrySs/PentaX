// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 from PUBLIC KytyPS5 7b7f9a70 agc.cpp/libAgcDriver.cpp.
// Original bounded owner/continuation and checked-memory integration; see NOTICE.

use super::*;
use pentax_platform::native_memory::NativeOwnerId;

#[path = "command_builder/register_builder.rs"]
mod register_builder;

pub(crate) const EXPORTS: [(&str, Function); 52] = [
    ("LtTouSCZjHM", Function::AgcCbNop),
    ("t7PlZ9nt5Lc", Function::AgcCbNopGetSize),
    ("k3GhuSNmBLU", Function::AgcCbDispatch),
    ("Abendgtz+3o", Function::AgcCbDispatchGetSize),
    ("GIIW2J37e70", Function::AgcDcbSetIndexSize),
    ("l4fM9K-Lyks", Function::AgcDcbSetIndexBuffer),
    ("8N2tmT3jmC8", Function::AgcDcbSetIndexCount),
    ("tSBxhAPyytQ", Function::AgcDcbSetNumInstances),
    ("6DFuRKT4C9w", Function::AgcDcbSetNumInstancesGetSize),
    ("q88lQ+GP5Yk", Function::AgcDcbDrawIndex),
    ("6ee9Hd3EWXQ", Function::AgcDcbDrawIndexGetSize),
    ("Yw0jKSqop+E", Function::AgcDcbDrawIndexAuto),
    ("WrdP9Zxx3lQ", Function::AgcDcbDrawIndexAutoGetSize),
    ("B+aG9DUnTKA", Function::AgcDcbDrawIndexOffset),
    ("qMlfB1ZhMDc", Function::AgcDcbDrawIndexOffsetGetSize),
    ("i1jyy49AjXU", Function::AgcDcbWriteData),
    ("p9tI+yTvx68", Function::AgcDcbWriteDataGetSize),
    ("eZ4+17OQz4Q", Function::AgcAcbWriteData),
    ("wr23dPKyWc0", Function::AgcCbReleaseMem),
    ("hL7C0IRpWZI", Function::AgcCbQueueEndOfPipeActionGetSize),
    ("VmW0Tdpy420", Function::AgcDcbWaitRegMem),
    ("43WJ08sSugE", Function::AgcDcbWaitOnAddressGetSize),
    ("htn36gPnBk4", Function::AgcAcbWaitRegMem),
    ("idlaArvdXEs", Function::AgcAcbWaitOnAddressGetSize),
    ("fPSCdQxgpSw", Function::AgcWriteDataPatchSetAddressOrOffset),
    ("3KDcnM3lrcU", Function::AgcWaitRegMemPatchAddress),
    ("7nOoijNPvEU", Function::AgcWaitRegMemPatchReference),
    ("0fWWK5uG9rQ", Function::AgcQueueEndOfPipeActionPatchAddress),
    ("MlEw1feXcjg", Function::AgcQueueEndOfPipeActionPatchData),
    ("u2T2DiA5hRI", Function::AgcDcbStallCommandBufferParser),
    ("n2fD4A+pb+g", Function::AgcCbSetShRegisterRangeDirect),
    (
        "bxGoVxpdSPQ",
        Function::AgcCbSetShRegisterRangeDirectGetSize,
    ),
    ("UZbQjYAwwXM", Function::AgcCbSetShRegistersDirect),
    ("03RZmELWWzw", Function::AgcCbSetUcRegistersDirect),
    ("pFLArOT53+w", Function::AgcDcbSetShRegisterDirect),
    ("LHFXRrlTPD8", Function::AgcDcbSetCxRegisterDirect),
    ("1DeUNpRIDDA", Function::AgcDcbSetCxRegisterDirectGetSize),
    ("w4-d0n60hdo", Function::AgcDcbSetUcRegisterDirect),
    ("ZvwO9euwYzc", Function::AgcDcbSetCxRegistersIndirect),
    ("GBCh3zCihoU", Function::AgcDcbSetCxRegistersIndirectGetSize),
    ("-HOOCn0JY48", Function::AgcDcbSetShRegistersIndirect),
    ("nNlUtdDDvZ0", Function::AgcDcbSetShRegistersIndirectGetSize),
    ("hvUfkUIQcOE", Function::AgcDcbSetUcRegistersIndirect),
    ("vcmNN+AAXnY", Function::AgcSetCxRegIndirectPatchSetAddress),
    ("Qrj4c+61z4A", Function::AgcSetShRegIndirectPatchSetAddress),
    ("6lNcCp+fxi4", Function::AgcSetUcRegIndirectPatchSetAddress),
    (
        "whb1RL7K4Ss",
        Function::AgcSetCxRegIndirectPatchSetNumRegisters,
    ),
    (
        "nCUgItdN2ms",
        Function::AgcSetShRegIndirectPatchSetNumRegisters,
    ),
    (
        "fRG-JOH5+sI",
        Function::AgcSetUcRegIndirectPatchSetNumRegisters,
    ),
    (
        "d-6uF9sZDIU",
        Function::AgcSetCxRegIndirectPatchAddRegisters,
    ),
    (
        "z2duB-hHQSM",
        Function::AgcSetShRegIndirectPatchAddRegisters,
    ),
    (
        "vRoArM9zaIk",
        Function::AgcSetUcRegIndirectPatchAddRegisters,
    ),
];

pub(crate) fn supports(function: Function) -> bool {
    EXPORTS.iter().any(|(_, found)| *found == function)
}

const INVALID_PACKET: u64 = 0x8a6c_000c;
const BEGIN32: u32 = 0xc801_0000;
const BEGIN64: u32 = 0xc802_0000;
const END_WAIT: u32 = 0xc800_0000;
// Both source write builders retain at most this many payload bytes. Counts
// that wrap the DCB test also fail checked count+4 before allocation.
const MAX_PAYLOAD_BYTES: usize = 0x3ffd * 4;

fn pm4(words: u32, opcode: u32, tag: u32) -> u32 {
    0xc000_0000 | ((words.wrapping_sub(2) & 0x3fff) << 16) | (opcode << 8) | (tag << 2)
}

fn initiator(modifier: u64) -> u32 {
    if modifier & (1 << 32) != 0 {
        0
    } else {
        ((modifier as u32) >> 3) & 0x20
    }
}

#[derive(Clone, Copy)]
struct Packet {
    allocate: u32,
    words: [u32; 16],
    written: usize,
    payload: Option<(u64, usize)>,
}
impl Packet {
    fn fixed(words: &[u32]) -> Self {
        let mut result = Self {
            allocate: words.len() as u32,
            words: [0; 16],
            written: words.len(),
            payload: None,
        };
        result.words[..words.len()].copy_from_slice(words);
        result
    }
}

/// This is a private continuation product, not execution or backing authority.
/// Its source/owner/executor binding is supplied separately by the core ticket.
pub(crate) struct PendingCommandBuilder {
    owner: NativeOwnerId,
    function: Function,
    buffer: u64,
    packet: Packet,
    callback: u64,
    callback_args: [u64; 3],
    payload: Vec<u8>,
    preflight_work: u64,
    registers: Option<register_builder::RegisterBuildState>,
}
impl std::fmt::Debug for PendingCommandBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingCommandBuilder")
            .finish_non_exhaustive()
    }
}
impl PendingCommandBuilder {
    pub(crate) fn retained_payload_bytes(&self) -> Option<usize> {
        self.payload
            .capacity()
            .checked_add(self.registers.as_ref().map_or(
                0,
                register_builder::RegisterBuildState::retained_payload_bytes,
            ))
    }
    pub(crate) fn owner(&self) -> NativeOwnerId {
        self.owner
    }
    pub(crate) fn entry(&self) -> u64 {
        self.callback
    }
    pub(crate) fn args(&self) -> [u64; 3] {
        self.callback_args
    }
}

// Keep the bounded continuation inline: boxing would add a fallible allocation
// at a callback boundary after earlier packet writes may already be visible.
// Retained continuation count and aggregate payload capacity remain capped.
#[allow(clippy::large_enum_variant)]
pub(crate) enum CommandBuilderAction {
    Returned(u64),
    Grow(PendingCommandBuilder),
}

fn size(function: Function, args: &[u64; 6]) -> Option<u64> {
    use Function::*;
    Some(match function {
        AgcCbNopGetSize => u64::from((args[0] as u32).wrapping_mul(4)),
        AgcCbDispatchGetSize | AgcDcbDrawIndexOffsetGetSize => 20,
        AgcCbSetShRegisterRangeDirectGetSize => {
            u64::from((args[0] as u32).wrapping_mul(4).wrapping_add(8))
        }
        AgcDcbSetCxRegisterDirectGetSize => 12,
        AgcDcbSetCxRegistersIndirectGetSize | AgcDcbSetShRegistersIndirectGetSize => 20,
        AgcDcbSetNumInstancesGetSize => 8,
        AgcDcbDrawIndexGetSize => 24,
        AgcDcbDrawIndexAutoGetSize => 12,
        AgcDcbWriteDataGetSize => u64::from((args[0] as u32).wrapping_mul(4).wrapping_add(16)),
        AgcCbQueueEndOfPipeActionGetSize => 32,
        AgcDcbWaitOnAddressGetSize | AgcAcbWaitOnAddressGetSize => {
            match if function == AgcAcbWaitOnAddressGetSize {
                u32::from(args[0] as u8)
            } else {
                args[0] as u32
            } {
                0 => 56,
                1 => 64,
                _ => 0,
            }
        }
        _ => return None,
    })
}

fn allocation_failure(function: Function) -> Result<u64, Failure> {
    use Function::*;
    match function {
        AgcCbNop
        | AgcCbDispatch
        | AgcDcbSetNumInstances
        | AgcDcbWriteData
        | AgcAcbWriteData
        | AgcCbReleaseMem
        | AgcDcbWaitRegMem
        | AgcAcbWaitRegMem => Ok(0),
        AgcCbSetShRegisterRangeDirect
        | AgcDcbSetShRegisterDirect
        | AgcDcbSetCxRegisterDirect
        | AgcDcbSetUcRegisterDirect => Ok(0),
        _ => Err(Failure::UnsupportedQueue),
    }
}

fn stack_count(function: Function) -> usize {
    use Function::*;
    match function {
        AgcCbReleaseMem => 6,
        AgcDcbWaitRegMem => 3,
        AgcDcbWriteData | AgcAcbWriteData | AgcAcbWaitRegMem => 2,
        _ => 0,
    }
}

fn packet(function: Function, args: [u64; 12]) -> Result<Option<Packet>, Failure> {
    use Function::*;
    let a = args.map(|value| value as u32);
    let packet = match function {
        AgcCbNop => {
            let mut value = Packet::fixed(&[pm4(a[1], 0x10, 0)]);
            value.allocate = a[1];
            value
        }
        AgcCbDispatch => {
            Packet::fixed(&[pm4(5, 0x15, 0), a[1], a[2], a[3], (a[4] & 0xa038) | 0x41])
        }
        AgcDcbSetIndexSize => Packet::fixed(&[
            pm4(3, 0x7a, 0),
            0x2000_0243,
            0x400 | (a[1] & 3) | ((a[2] & 3) << 6),
        ]),
        AgcDcbSetIndexBuffer => {
            if args[1] & 1 != 0 {
                return Err(Failure::UnsupportedQueue);
            }
            Packet::fixed(&[pm4(3, 0x26, 0), a[1], (args[1] >> 32) as u32])
        }
        AgcDcbSetIndexCount => Packet::fixed(&[pm4(2, 0x13, 0), a[1]]),
        AgcDcbSetNumInstances => Packet::fixed(&[pm4(2, 0x2f, 0), a[1]]),
        AgcDcbDrawIndex => {
            if args[2] == 0 || args[2] & 1 != 0 {
                return Err(Failure::UnsupportedQueue);
            }
            Packet::fixed(&[
                pm4(6, 0x27, 0),
                a[1].max(1),
                a[2],
                (args[2] >> 32) as u32,
                a[1],
                initiator(args[3]),
            ])
        }
        AgcDcbDrawIndexAuto => Packet::fixed(&[pm4(3, 0x2d, 0), a[1], initiator(args[2]) | 2]),
        AgcDcbDrawIndexOffset => {
            Packet::fixed(&[pm4(5, 0x35, 0), a[2].max(1), a[1], a[2], initiator(args[3])])
        }
        AgcDcbStallCommandBufferParser => Packet::fixed(&[pm4(2, 0x42, 0), 0]),
        AgcDcbWriteData | AgcAcbWriteData => {
            let count = a[5];
            if function == AgcAcbWriteData && (count == 0 || count > 0x3ffd)
                || function == AgcDcbWriteData && count.wrapping_add(2) > 0x3fff
            {
                return Ok(None);
            }
            let destination = a[1] & 0xff;
            let confirm = if destination == 0 { 0 } else { a[7] & 1 };
            let control = if function == AgcDcbWriteData {
                ((destination & 1) << 30) | ((destination & 0x1e) << 7)
            } else {
                (destination & 0xf) << 8
            };
            let control = control | ((a[6] & 1) << 16) | (confirm << 20) | ((a[2] & 3) << 25);
            let address = if function == AgcDcbWriteData {
                a[3] & !3
            } else {
                a[3]
            };
            let mut value = Packet::fixed(&[
                pm4(count.wrapping_add(4), 0x37, 0),
                control,
                address,
                (args[3] >> 32) as u32,
            ]);
            value.allocate = count.checked_add(4).ok_or(Failure::Limit)?;
            value.payload = Some((
                args[4],
                (count as usize).checked_mul(4).ok_or(Failure::Limit)?,
            ));
            value
        }
        AgcCbReleaseMem => {
            let action = a[1] & 0xff;
            let mut gcr = a[2] & 0xffff;
            let destination = a[3] & 0xff;
            let data_select = a[6] & 0xff;
            let interrupt = a[10] & 0xff;
            if destination > 1 || !matches!(data_select, 0 | 1 | 2 | 3 | 5) || interrupt > 4 {
                return Err(Failure::UnsupportedQueue);
            }
            if gcr & 0x300 == 0x100 {
                gcr |= 0x200;
            }
            let address = if interrupt == 4 { 0 } else { args[5] };
            let data = if interrupt == 4 {
                0
            } else if data_select == 5 {
                (args[8] & 0xffff) | ((args[9] & 0xffff) << 16)
            } else {
                args[7]
            };
            Packet::fixed(&[
                pm4(8, 0x10, 24),
                (action & 0x3f)
                    | ((if action >= 0x2f { 6 } else { 5 }) << 8)
                    | ((gcr & 0xfff) << 12)
                    | ((a[4] & 3) << 25),
                (destination << 16) | (interrupt << 24) | (data_select << 29),
                address as u32 & !3,
                (address >> 32) as u32,
                data as u32,
                (data >> 32) as u32,
                a[11] & 0x07ff_ffff,
            ])
        }
        AgcDcbWaitRegMem | AgcAcbWaitRegMem => {
            let a = if function == AgcAcbWaitRegMem {
                [
                    args[0], args[1], args[2], 0, args[3], args[4], args[5], args[6], args[7],
                ]
            } else {
                args[..9]
                    .try_into()
                    .map_err(|_| Failure::UnsupportedQueue)?
            };
            let size = a[1] as u8;
            if size > 1 {
                return Ok(None);
            }
            let wide = size == 1;
            let count = if wide { 16 } else { 14 };
            let mut value = Packet {
                allocate: count,
                words: [0; 16],
                written: count as usize,
                payload: None,
            };
            let w = &mut value.words;
            w[..4].copy_from_slice(&[
                pm4(4, 0x79, 1),
                0x342,
                (if wide { BEGIN64 } else { BEGIN32 }) | ((a[5] >> 32) as u32 & 0xffff),
                a[5] as u32,
            ]);
            let operation = a[3] as u8 as u32;
            let control = 0x10
                | (a[2] as u32 & 7)
                | ((a[4] as u32 & 3) << 25)
                | if wide {
                    ((operation & 1) << 8) | ((operation & 6) << 5)
                } else {
                    ((operation & 3) << 8) | ((operation & 12) << 4)
                };
            w[4..8].copy_from_slice(&[
                pm4(if wide { 9 } else { 7 }, if wide { 0x93 } else { 0x3c }, 0),
                control,
                a[5] as u32 & if wide { !7 } else { !3 },
                (a[5] >> 32) as u32 & 0x3ffff,
            ]);
            w[8] = a[6] as u32;
            let end = if wide {
                w[9] = (a[6] >> 32) as u32;
                w[10] = a[7] as u32;
                w[11] = (a[7] >> 32) as u32;
                w[12] = ((a[8] as u32) >> 4).min(0xffff);
                13
            } else {
                w[9] = a[7] as u32;
                w[10] = ((a[8] as u32) >> 4).min(0xffff);
                11
            };
            w[end..end + 3].copy_from_slice(&[pm4(3, 0x79, 1), 0x342, END_WAIT]);
            value
        }
        AgcCbSetShRegisterRangeDirect => {
            let mut value = Packet::fixed(&[pm4(a[3].wrapping_add(2), 0x76, 0), a[1] & 0xffff]);
            value.allocate = a[3].checked_add(2).ok_or(Failure::Limit)?;
            if args[2] != 0 {
                value.payload = Some((
                    args[2],
                    usize::try_from(u64::from(a[3]) * 4).map_err(|_| Failure::Limit)?,
                ));
            }
            value
        }
        AgcDcbSetShRegisterDirect | AgcDcbSetCxRegisterDirect | AgcDcbSetUcRegisterDirect => {
            let op = match function {
                AgcDcbSetShRegisterDirect => 0x76,
                AgcDcbSetCxRegisterDirect => 0x69,
                _ => 0x79,
            };
            Packet::fixed(&[pm4(3, op, 0), a[1] & 0xffff, (args[1] >> 32) as u32])
        }
        AgcDcbSetCxRegistersIndirect
        | AgcDcbSetShRegistersIndirect
        | AgcDcbSetUcRegistersIndirect => {
            let op = match function {
                AgcDcbSetCxRegistersIndirect => 0x9f,
                AgcDcbSetShRegistersIndirect => 0x63,
                _ => 0x64,
            };
            Packet::fixed(&[
                pm4(5, op, 0),
                a[1] & !3,
                (args[1] >> 32) as u32,
                0x8000_0000,
                a[2] & 0x3fff,
            ])
        }
        _ => return Err(Failure::UnsupportedQueue),
    };
    Ok(Some(packet))
}

fn pointer(memory: &PausedNativeMemory<'_>, address: u64, offset: u64) -> Result<u64, Failure> {
    let mut bytes = [0; 8];
    memory
        .read(
            address.checked_add(offset).ok_or(Failure::Memory)?,
            &mut bytes,
        )
        .map_err(|_| Failure::Memory)?;
    Ok(u64::from_le_bytes(bytes))
}

fn word(memory: &PausedNativeMemory<'_>, address: u64, index: u64) -> Result<u32, Failure> {
    let mut bytes = [0; 4];
    memory
        .read(
            address.checked_add(index * 4).ok_or(Failure::Memory)?,
            &mut bytes,
        )
        .map_err(|_| Failure::Memory)?;
    Ok(u32::from_le_bytes(bytes))
}

fn wait_packet(memory: &PausedNativeMemory<'_>, address: u64) -> Result<Option<u32>, Failure> {
    let header = word(memory, address, 0)?;
    // AgcIsInternalDataPacket checks opcode, tag, selector and minimum length;
    // get_agc_wait_packet then requires the exact four-DWORD prefix.
    if ((header >> 16) & 0x3fff) + 2 != 4
        || (header >> 8) & 0xff != 0x79
        || (header >> 2) & 0x3f != 1
        || word(memory, address, 1)? != 0x342
    {
        return Ok(None);
    }
    let wait = word(memory, address, 4)?;
    let tag = word(memory, address, 2)? & 0xffff_0000;
    let op = (wait >> 8) & 0xff;
    let len = ((wait >> 16) & 0x3fff) + 2;
    if (wait >> 2) & 0x3f != 0 || !matches!((op, len, tag), (0x3c, 7, BEGIN32) | (0x93, 9, BEGIN64))
    {
        return Ok(None);
    }
    Ok(Some(op))
}

fn available(memory: &PausedNativeMemory<'_>, buffer: u64) -> Result<(u64, u64), Failure> {
    let up = pointer(memory, buffer, 16)?;
    let down = pointer(memory, buffer, 24)?;
    let mut bytes = [0; 4];
    memory
        .read(buffer.checked_add(48).ok_or(Failure::Memory)?, &mut bytes)
        .map_err(|_| Failure::Memory)?;
    let reserved = u64::from(u32::from_le_bytes(bytes));
    // Preserve the source's signed distance, arithmetic shift and subsequent
    // unsigned interpretation. Actual accesses still require the live owner.
    let distance = (down.wrapping_sub(up) as i64 >> 2) as u64;
    Ok((up, distance.saturating_sub(reserved)))
}

fn null_result(function: Function, args: &[u64; 6]) -> Option<Result<u64, Failure>> {
    use Function::*;
    if function == AgcCbNop && args[1] as u32 == 0 {
        return Some(Ok(0));
    }
    if matches!(
        function,
        AgcCbSetShRegistersDirect | AgcCbSetUcRegistersDirect
    ) && args[2] as u32 == 0
    {
        return Some(Ok(0));
    }
    if matches!(function, AgcDcbWriteData | AgcAcbWriteData) && args[4] == 0 {
        return Some(Ok(0));
    }
    if args[0] != 0 {
        return None;
    }
    Some(match function {
        AgcCbNop | AgcCbDispatch | AgcDcbSetNumInstances | AgcDcbWriteData | AgcAcbWriteData => {
            Ok(0)
        }
        AgcWaitRegMemPatchAddress | AgcWaitRegMemPatchReference => Ok(INVALID_PACKET),
        _ => Err(Failure::UnsupportedQueue),
    })
}

impl Driver {
    fn patch_command(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
    ) -> Result<u64, Failure> {
        use Function::*;
        // Fixed bounded validation and at most four individual source stores.
        // The write allowance is committed before the first store; late faults
        // preserve the source-ordered prefix and poison subsequent submissions.
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            4096,
            self.limits.max_preparation_work,
        )?;
        let address = args[0];
        let header = word(memory, address, 0)?;
        let op = (header >> 8) & 0xff;
        let release = op == 0x49 || op == 0x10 && (header >> 2) & 0x3f == 24;
        let mut indices = [0; 4];
        let mut values = [0; 4];
        let mut keep = [0; 4];
        let count = match function {
            AgcWriteDataPatchSetAddressOrOffset => {
                if op != 0x37 {
                    return Ok(INVALID_PACKET);
                }
                indices[..2].copy_from_slice(&[2, 3]);
                values[..2].copy_from_slice(&[args[1] as u32, (args[1] >> 32) as u32]);
                2
            }
            AgcWaitRegMemPatchAddress => {
                let Some(op) = wait_packet(memory, address)? else {
                    return Ok(INVALID_PACKET);
                };
                let mask = if op == 0x3c { 3 } else { 7 };
                indices.copy_from_slice(&[2, 3, 6, 7]);
                keep.copy_from_slice(&[0xffff_0000, 0, mask, 0xfffc_0000]);
                values.copy_from_slice(&[
                    (args[1] >> 32) as u32 & 0xffff,
                    args[1] as u32,
                    args[1] as u32 & !mask,
                    (args[1] >> 32) as u32 & 0x3ffff,
                ]);
                4
            }
            AgcWaitRegMemPatchReference => {
                if wait_packet(memory, address)?.is_none() {
                    return Ok(INVALID_PACKET);
                }
                indices[0] = 8;
                values[0] = args[1] as u32;
                1
            }
            AgcQueueEndOfPipeActionPatchAddress => {
                if release {
                    indices[..2].copy_from_slice(&[3, 4]);
                    values[..2].copy_from_slice(&[args[1] as u32, (args[1] >> 32) as u32]);
                } else if op == 0x47 {
                    indices[..2].copy_from_slice(&[2, 3]);
                    keep[1] = 0xffff_0000;
                    values[..2].copy_from_slice(&[args[1] as u32, (args[1] >> 32) as u32 & 0xffff]);
                } else {
                    return Err(Failure::UnsupportedQueue);
                }
                2
            }
            AgcQueueEndOfPipeActionPatchData => {
                if !release {
                    return Ok(INVALID_PACKET);
                }
                let control = word(memory, address, 2)?;
                if (control >> 24) & 7 == 4 || (control >> 29) & 7 == 5 {
                    return Ok(0);
                }
                indices[..2].copy_from_slice(&[5, 6]);
                values[..2].copy_from_slice(&[args[1] as u32, (args[1] >> 32) as u32]);
                2
            }
            _ => return Err(Failure::UnsupportedQueue),
        };
        self.usage.memory_written_bytes = add(
            self.usage.memory_written_bytes,
            count as u64 * 4,
            self.limits.max_memory_write_bytes,
        )?;
        for ((&index, &value), &mask) in indices[..count]
            .iter()
            .zip(&values[..count])
            .zip(&keep[..count])
        {
            // Source read-modify-write operands are fetched at their actual
            // position, after prior stores. A late read fault must preserve
            // those prefixes even when the command crosses a page boundary.
            let value = if mask == 0 {
                value
            } else {
                value | (word(memory, address, index)? & mask)
            };
            self.command_write(
                memory,
                address.checked_add(index * 4).ok_or(Failure::Memory)?,
                &value.to_le_bytes(),
                4096,
            )?;
        }
        self.poisoned = false;
        Ok(0)
    }
    pub(crate) fn begin_command_builder(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        function: Function,
        args: [u64; 6],
        stack: Option<u64>,
    ) -> Result<CommandBuilderAction, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        self.usage.calls = add(self.usage.calls, 1, self.limits.max_calls)?;
        self.function_profile.record(function);
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            256,
            self.limits.max_preparation_work,
        )?;
        if let Some(value) = size(function, &args) {
            return Ok(CommandBuilderAction::Returned(value));
        }
        if let Some(result) = null_result(function, &args) {
            return result.map(CommandBuilderAction::Returned);
        }
        if matches!(
            function,
            Function::AgcCbSetShRegistersDirect | Function::AgcCbSetUcRegistersDirect
        ) {
            return self.begin_register_group(memory, function, args);
        }
        if register_builder::patch_kind(function).is_some() {
            return self
                .patch_register_command(memory, function, args)
                .map(CommandBuilderAction::Returned);
        }
        if matches!(
            function,
            Function::AgcWriteDataPatchSetAddressOrOffset
                | Function::AgcWaitRegMemPatchAddress
                | Function::AgcWaitRegMemPatchReference
                | Function::AgcQueueEndOfPipeActionPatchAddress
                | Function::AgcQueueEndOfPipeActionPatchData
        ) {
            return self
                .patch_command(memory, function, args)
                .map(CommandBuilderAction::Returned);
        }
        let mut full = [0; 12];
        full[..6].copy_from_slice(&args);
        let extra = stack_count(function);
        if extra != 0 {
            let at = stack
                .ok_or(Failure::UnsupportedQueue)?
                .checked_add(8)
                .ok_or(Failure::Memory)?;
            let mut bytes = [0; 48];
            memory
                .read(at, &mut bytes[..extra * 8])
                .map_err(|_| Failure::Memory)?;
            for (index, bytes) in bytes[..extra * 8].chunks_exact(8).enumerate() {
                full[6 + index] =
                    u64::from_le_bytes(bytes.try_into().map_err(|_| Failure::Memory)?);
            }
        }
        let Some(packet) = packet(function, full)? else {
            return Ok(CommandBuilderAction::Returned(0));
        };
        let allocation = u64::from(packet.allocate)
            .checked_mul(4)
            .ok_or(Failure::Limit)?;
        if allocation > self.limits.commands.max_command_bytes as u64 {
            return Err(Failure::Limit);
        }
        let payload_length = packet.payload.map_or(0, |(_, length)| length);
        let output = (packet.written as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(payload_length as u64))
            .and_then(|n| n.checked_add(8))
            .ok_or(Failure::Limit)?;
        // Fixed4096 covers the descriptor/argument reads and at most eighteen
        // one-output transactions. Cursor/header outputs cross at most two
        // guest pages; the <=65524B payload crosses at most five16KiB pages.
        // Their sum(1+4*p+p*p) plus the payload's disjoint-page comparison
        // (lp*rp+3*(lp+rp)+1) is below this envelope. The allocation term also
        // covers payload staging/copying. Passing the cap to each primitive
        // does not refund or debit this admitted allowance a second time.
        let mut work = allocation
            .checked_mul(8)
            .and_then(|n| n.checked_add(4096))
            .ok_or(Failure::Limit)?;
        if function == Function::AgcCbSetShRegisterRangeDirect && payload_length != 0 {
            let page = crate::memory::PAGE_SIZE;
            let pages = (payload_length as u64)
                .checked_add(page - 2)
                .map(|bytes| bytes / page + 1)
                .ok_or(Failure::Limit)?;
            work = pages
                .checked_mul(pages)
                .and_then(|n| pages.checked_mul(6).and_then(|m| n.checked_add(m)))
                .and_then(|n| n.checked_add(1))
                .and_then(|n| work.checked_add(n))
                .ok_or(Failure::Limit)?;
        }
        if work > self.limits.commands.max_work
            || payload_length > self.limits.commands.max_host_bytes
        {
            return Err(Failure::Limit);
        }
        // These are monotonically admitted CPU-write/work bytes. Debiting the
        // full body before a real callback prevents nested calls from spending
        // the same allowance. A late fault never refunds an admitted budget or
        // claims a GPU writeback through usage.written_bytes.
        let write_bytes = add(
            self.usage.memory_written_bytes,
            output,
            self.limits.max_memory_write_bytes,
        )?;
        self.usage.preparation_work = add(
            self.usage.preparation_work,
            work,
            self.limits.max_preparation_work,
        )?;
        let mut payload = Vec::new();
        payload
            .try_reserve_exact(payload_length)
            .map_err(|_| Failure::Allocation)?;
        if payload.capacity() > self.limits.commands.max_host_bytes
            || function != Function::AgcCbSetShRegisterRangeDirect
                && payload.capacity() > MAX_PAYLOAD_BYTES
        {
            return Err(Failure::Limit);
        }
        self.usage.memory_written_bytes = write_bytes;
        payload.resize(payload_length, 0);
        let (up, count) = available(memory, args[0])?;
        let mut pending = PendingCommandBuilder {
            owner: memory.owner_id(),
            function,
            buffer: args[0],
            packet,
            callback: 0,
            callback_args: [0; 3],
            payload,
            preflight_work: work,
            registers: None,
        };
        if u64::from(packet.allocate) <= count {
            let value = self.publish_command(memory, pending, up)?;
            return Ok(CommandBuilderAction::Returned(value));
        }
        pending.callback = pointer(memory, args[0], 32)?;
        if pending.callback == 0 {
            return allocation_failure(function).map(CommandBuilderAction::Returned);
        }
        let mut bytes = [0; 4];
        memory
            .read(args[0].checked_add(48).ok_or(Failure::Memory)?, &mut bytes)
            .map_err(|_| Failure::Memory)?;
        let required = packet.allocate.wrapping_add(u32::from_le_bytes(bytes));
        pending.callback_args = [args[0], u64::from(required), pointer(memory, args[0], 40)?];
        Ok(CommandBuilderAction::Grow(pending))
    }

    pub(crate) fn complete_command_builder(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        pending: PendingCommandBuilder,
        returned: u64,
    ) -> Result<CommandBuilderAction, Failure> {
        if self.poisoned {
            return Err(Failure::Poisoned);
        }
        if pending.owner != memory.owner_id() {
            return Err(Failure::Memory);
        }
        if pending.registers.is_some() {
            return self.resume_register_group(memory, pending, Some(returned));
        }
        // A source bool uses AL. The actual callback receipt, not this numeric
        // value alone, is authenticated by the core continuation before entry.
        if returned as u8 == 0 {
            return allocation_failure(pending.function).map(CommandBuilderAction::Returned);
        }
        let (up, count) = available(memory, pending.buffer)?;
        if u64::from(pending.packet.allocate) > count {
            return allocation_failure(pending.function).map(CommandBuilderAction::Returned);
        }
        self.publish_command(memory, pending, up)
            .map(CommandBuilderAction::Returned)
    }

    fn command_write(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        address: u64,
        bytes: &[u8],
        work: u64,
    ) -> Result<(), Failure> {
        // Once the source allocation or a header word may have been published,
        // a later fault is terminal and never restarts this source body.
        self.poisoned = true;
        memory
            .apply_transaction(
                pentax_platform::native_memory::NativeMemoryTransaction {
                    effect: pentax_platform::native_memory::NativeMemoryEffect::None,
                    writes: &[NativeMemoryWrite { address, bytes }],
                },
                &NativeTransactionLimits {
                    max_outputs: 1,
                    max_output_bytes: bytes.len() as u64,
                    max_preflight_steps: work,
                    ..Default::default()
                },
            )
            .map_err(|_| Failure::Memory)
    }

    fn publish_command(
        &mut self,
        memory: &mut PausedNativeMemory<'_>,
        mut pending: PendingCommandBuilder,
        up: u64,
    ) -> Result<u64, Failure> {
        let next = up
            .checked_add(u64::from(pending.packet.allocate) * 4)
            .ok_or(Failure::Memory)?;
        self.command_write(
            memory,
            pending.buffer.checked_add(16).ok_or(Failure::Memory)?,
            &next.to_le_bytes(),
            pending.preflight_work,
        )?;
        if up == 0 {
            // AllocateDW advances a null cursor before returning nullptr.
            // Its caller follows the individual failure branch, without
            // touching either a packet header or an optional source payload.
            let value = allocation_failure(pending.function)?;
            self.poisoned = false;
            return Ok(value);
        }
        for ordinal in 0..pending.packet.written {
            // The source writes a wait's end tag immediately after its prefix,
            // before filling the central WAIT_REG_MEM packet. Preserve even
            // this failure-prefix order rather than publishing linear bytes.
            let index = if matches!(
                pending.function,
                Function::AgcDcbWaitRegMem | Function::AgcAcbWaitRegMem
            ) && ordinal >= 4
            {
                if ordinal < 7 {
                    pending.packet.written - 3 + ordinal - 4
                } else {
                    ordinal - 3
                }
            } else {
                ordinal
            };
            self.command_write(
                memory,
                up.checked_add(index as u64 * 4).ok_or(Failure::Memory)?,
                &pending.packet.words[index].to_le_bytes(),
                pending.preflight_work,
            )?;
        }
        if let Some((source, length)) = pending.packet.payload
            && length != 0
        {
            let destination = up
                .checked_add(pending.packet.written as u64 * 4)
                .ok_or(Failure::Memory)?;
            // Cursor and header are now the actual current guest bytes.
            // Descriptor/header-as-input cases therefore read the source's
            // post-allocation values. memcpy payload overlap is undefined;
            // its checked refusal preserves the already published prefix.
            memory
                .preflight_disjoint_backing(
                    NativeRange {
                        address: source,
                        size: length as u64,
                    },
                    NativeRange {
                        address: destination,
                        size: length as u64,
                    },
                    pending.preflight_work,
                )
                .map_err(|_| Failure::Memory)?;
            memory
                .read(source, &mut pending.payload)
                .map_err(|_| Failure::Memory)?;
            self.command_write(
                memory,
                destination,
                &pending.payload,
                pending.preflight_work,
            )?;
        }
        self.poisoned = false;
        Ok(up)
    }
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
#[path = "command_builder/tests.rs"]
mod tests;
