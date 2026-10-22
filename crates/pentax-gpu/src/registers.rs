// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Adapted 2026 for PentaX: bounded ordered indirect register-array descriptors.
// Public KytyPS5/KytyPS5@7b7f9a70c652afd6ec8ea8599884d9b4b3abc9f3,
// agc.cpp RegIndirectPacket and pm4Handlers.cpp CpOpIndirectCx/Sh/UcRegs.

use crate::{
    Header, RegisterSpace,
    compute::{CommandRange, Error, Memory},
};
use std::mem::size_of;

/// A closed descriptor from one actual five-DWORD packet. It is preparation
/// data, not permission to access guest backing or to execute a GPU command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndirectRegisters {
    space: RegisterSpace,
    address: u64,
    count: u32,
}

impl IndirectRegisters {
    pub(crate) fn decode(words: [u32; 5]) -> Result<Self, Error> {
        let space = match words[0] {
            0xc003_9f00 => RegisterSpace::Context,
            0xc003_6300 => RegisterSpace::Sh,
            0xc003_6400 => RegisterSpace::Uconfig,
            _ => return Err(Error::Unsupported("indirect register packet profile")),
        };
        if words[3] != 0x8000_0000 {
            return Err(Error::Unsupported("indirect register control"));
        }
        let address = u64::from(words[1] & !3) | (u64::from(words[2]) << 32);
        let count = words[4] & 0x3fff;
        let bytes = u64::from(count) * 8;
        // Zero count returns before any address use in the public handler.
        if count != 0
            && (address == 0 || address.checked_add(bytes).is_none_or(|end| end > 1 << 47))
        {
            return Err(Error::Unsupported("indirect register extent"));
        }
        Ok(Self {
            space,
            address,
            count,
        })
    }

    pub fn space(self) -> RegisterSpace {
        self.space
    }
    pub fn count(self) -> u32 {
        self.count
    }
    pub fn byte_count(self) -> usize {
        self.count as usize * 8
    }
    pub fn range(self) -> Option<CommandRange> {
        (self.count != 0).then_some(CommandRange {
            address: self.address,
            size: u64::from(self.count) * 8,
        })
    }

    /// Read the full array at the caller's actual ordered execution point.
    /// Scratch must have been reserved before any command effects. No pair is
    /// exposed until the complete current snapshot has been read successfully.
    pub(crate) fn snapshot<'scratch>(
        self,
        memory: &impl Memory,
        scratch: &'scratch mut [u8],
    ) -> Result<&'scratch [u8], Error> {
        let output = scratch
            .get_mut(..self.byte_count())
            .ok_or(Error::Limit("register scratch"))?;
        if !output.is_empty() {
            memory.read(self.address, output).map_err(Error::Memory)?;
        }
        Ok(output)
    }
}

pub(crate) fn indirect_opcode(header: Header) -> bool {
    matches!(header.opcode(), 0x9f | 0x63 | 0x64)
}

pub(crate) fn pair(bytes: &[u8]) -> (u32, u32) {
    let raw = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    let value = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    // Only these source-proven control bits are removed. Sentinels, extended
    // unknown offsets and fake-register conventions are not silently ignored.
    (raw & !0x7000_0000, value)
}

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub buffers: usize,
    pub command_bytes: usize,
    pub packets: u64,
    pub host: usize,
    pub work: u64,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Usage {
    pub command_bytes: usize,
    pub packets: u64,
    pub host: usize,
    pub work: u64,
}

#[derive(Clone, Copy)]
pub(crate) struct Packet {
    pub at: usize,
    pub words: usize,
}

/// A flat, immutable command snapshot and bounded reusable register scratch.
/// Array contents deliberately do not exist in this product until execution.
pub(crate) struct Program {
    bytes: Vec<u8>,
    packets: Vec<Packet>,
    scratch: Vec<u8>,
    ranges: Vec<CommandRange>,
    pub usage: Usage,
    limits: Limits,
}

impl Program {
    pub fn new(buffers: &[&[u8]], roots: &[CommandRange], limits: Limits) -> Result<Self, Error> {
        if buffers.len() > limits.buffers || roots.len() != buffers.len() {
            return Err(Error::Limit("register command buffers"));
        }
        for (bytes, root) in buffers.iter().zip(roots) {
            if root.size != bytes.len() as u64 {
                return Err(Error::Unsupported("register command extent"));
            }
        }
        Self::build(buffers, roots, limits)
    }

    /// Only the sealed graphics capture can supply expanded bytes independently
    /// of their original whole source ranges. No synthetic guest range exists.
    pub(crate) fn captured(
        capture: &crate::graphics::CapturedCommands,
        limits: Limits,
    ) -> Result<Self, Error> {
        Self::build(&[capture.bytes()], capture.command_ranges(), limits)
    }

    fn build(buffers: &[&[u8]], roots: &[CommandRange], limits: Limits) -> Result<Self, Error> {
        if buffers.len() > limits.buffers || roots.len() > limits.buffers {
            return Err(Error::Limit("register command buffers"));
        }
        let mut usage = Usage::default();
        let mut count = 0usize;
        let mut arrays = 0usize;
        let mut scratch = 0usize;
        add_work(
            &mut usage,
            limits,
            (buffers.len() as u64)
                .checked_mul(2)
                .ok_or(Error::Limit("register metadata"))?,
        )?;
        for bytes in buffers {
            if !bytes.len().is_multiple_of(4) {
                return Err(Error::Unsupported("register command extent"));
            }
            usage.command_bytes = usage
                .command_bytes
                .checked_add(bytes.len())
                .filter(|n| *n <= limits.command_bytes)
                .ok_or(Error::Limit("register command bytes"))?;
            add_work(
                &mut usage,
                limits,
                (bytes.len() as u64)
                    .checked_mul(2)
                    .ok_or(Error::Limit("register command work"))?,
            )?;
            let mut at = 0;
            while at < bytes.len() {
                let words = packet_words(&bytes[at..])?;
                let end = at
                    .checked_add(
                        words
                            .checked_mul(4)
                            .ok_or(Error::Limit("register packet extent"))?,
                    )
                    .filter(|n| *n <= bytes.len())
                    .ok_or(Error::Unsupported("truncated register command"))?;
                usage.packets = usage
                    .packets
                    .checked_add(1)
                    .filter(|n| *n <= limits.packets)
                    .ok_or(Error::Limit("register command packets"))?;
                count = count
                    .checked_add(1)
                    .ok_or(Error::Limit("register command metadata"))?;
                add_work(&mut usage, limits, 2)?;
                let header = Header::new(word(bytes, at));
                if indirect_opcode(header) {
                    if words != 5 {
                        return Err(Error::Unsupported("indirect register length"));
                    }
                    let array = IndirectRegisters::decode(std::array::from_fn(|i| {
                        word(bytes, at + i * 4)
                    }))?;
                    arrays = arrays
                        .checked_add(1)
                        .ok_or(Error::Limit("register range metadata"))?;
                    scratch = scratch.max(array.byte_count());
                    // Reserve both actual snapshot transfer and all pair
                    // decoding before the first command can have an effect.
                    add_work(
                        &mut usage,
                        limits,
                        (array.byte_count() as u64)
                            .checked_mul(2)
                            .and_then(|n| n.checked_add(u64::from(array.count())))
                            .ok_or(Error::Limit("register array work"))?,
                    )?;
                }
                at = end;
            }
        }
        add_host(&mut usage, limits, size_of::<Self>())?;
        let byte_count = usage.command_bytes;
        let mut bytes = reserved::<u8>(&mut usage, limits, byte_count)?;
        for buffer in buffers {
            bytes.extend_from_slice(buffer);
        }
        let mut packets = reserved::<Packet>(&mut usage, limits, count)?;
        let mut base = 0;
        for buffer in buffers {
            let mut at = 0;
            while at < buffer.len() {
                let words = packet_words(&buffer[at..])?;
                packets.push(Packet {
                    at: base + at,
                    words,
                });
                at += words * 4;
            }
            base += buffer.len();
        }
        let mut scratch_bytes = reserved::<u8>(&mut usage, limits, scratch)?;
        scratch_bytes.resize(scratch, 0);
        let range_count = roots
            .len()
            .checked_add(arrays)
            .ok_or(Error::Limit("register range metadata"))?;
        let mut ranges = reserved::<CommandRange>(&mut usage, limits, range_count)?;
        ranges.extend_from_slice(roots);
        Ok(Self {
            bytes,
            packets,
            scratch: scratch_bytes,
            ranges,
            usage,
            limits,
        })
    }

    pub fn array_count(&self) -> usize {
        self.packets
            .iter()
            .filter(|packet| indirect_opcode(Header::new(self.word(**packet, 0))))
            .count()
    }
    pub fn has_arrays(&self) -> bool {
        self.packets
            .iter()
            .any(|packet| indirect_opcode(Header::new(self.word(*packet, 0))))
    }
    pub fn packet(&self, index: usize) -> Option<Packet> {
        self.packets.get(index).copied()
    }
    pub fn sanitized(&mut self, reset_state_bytes: usize) -> Result<Vec<u8>, Error> {
        let mut bytes = self.reserve::<u8>(self.bytes.len())?;
        bytes.extend_from_slice(&self.bytes);
        let mut reset_work = 0u64;
        for packet in &self.packets {
            if crate::queue_reset::is_header(Header::new(self.word(*packet, 0)).raw()) {
                reset_work = reset_work
                    .checked_add(reset_state_bytes as u64)
                    .ok_or(Error::Limit("ordered reset work"))?;
            }
            if indirect_opcode(Header::new(self.word(*packet, 0))) {
                // Preserve the complete five-DWORD extent as one opaque NOP.
                // Only the static grammar pass sees this temporary stream;
                // execution always consumes the original sealed packet.
                bytes[packet.at..packet.at + 4].copy_from_slice(&0xc003_1000u32.to_le_bytes());
            }
        }
        // The original sanitization walk also precharges the live replay's
        // reset-state copy. There is no additional packet scan or allocation.
        self.charge(0, reset_work)?;
        Ok(bytes)
    }
    pub fn packets(&self) -> &[Packet] {
        &self.packets
    }
    pub fn word(&self, packet: Packet, index: usize) -> u32 {
        word(&self.bytes, packet.at + index * 4)
    }
    pub fn ranges(&self) -> &[CommandRange] {
        &self.ranges
    }
    pub fn array(&self, packet: Packet) -> Result<IndirectRegisters, Error> {
        IndirectRegisters::decode(std::array::from_fn(|i| self.word(packet, i)))
    }
    pub fn charge(&mut self, host: usize, work: u64) -> Result<(), Error> {
        add_host(&mut self.usage, self.limits, host)?;
        add_work(&mut self.usage, self.limits, work)
    }
    pub fn remaining(&self) -> (usize, u64) {
        (
            self.limits.host - self.usage.host,
            self.limits.work - self.usage.work,
        )
    }
    pub fn reserve<T>(&mut self, count: usize) -> Result<Vec<T>, Error> {
        reserved(&mut self.usage, self.limits, count)
    }
    pub fn resolve(
        &mut self,
        array: IndirectRegisters,
        memory: &impl Memory,
        mut apply: impl FnMut(RegisterSpace, u32, u32) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let bytes = array.snapshot(memory, &mut self.scratch)?;
        for bytes in bytes.chunks_exact(8) {
            let (offset, value) = pair(bytes);
            apply(array.space(), offset, value)?;
        }
        if let Some(range) = array.range() {
            // Capacity for every possible array was reserved before effects.
            if self.ranges.len() == self.ranges.capacity() {
                return Err(Error::Limit("register ranges"));
            }
            self.ranges.push(range);
        }
        Ok(())
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn packet_words(bytes: &[u8]) -> Result<usize, Error> {
    if bytes.len() < 4 {
        return Err(Error::Unsupported("truncated register command"));
    }
    let header = Header::new(word(bytes, 0));
    if header.raw() == 0x8000_0000 {
        return Ok(1);
    }
    if crate::queue_reset::is_header(header.raw()) {
        if bytes.len() < 8 {
            return Err(Error::Unsupported("truncated queue reset packet"));
        }
        crate::queue_reset::decode(header.raw(), word(bytes, 4)).map_err(Error::Unsupported)?;
        return Ok(2);
    }
    if header.raw() == 0xc004_105c {
        return Ok(6);
    }
    if header.raw() == 0xc006_1060 {
        return Ok(8);
    }
    if header.raw() == 0xc002_7904 {
        return crate::compute::tagged_wait_dwords(bytes);
    }
    if header.packet_type() != 3 || !matches!(header.low_byte(), 0 | 2) {
        return Err(Error::Unsupported("register command header"));
    }
    if header.opcode() == 0x10 && header.count() == 0x3fff {
        Ok(1)
    } else {
        Ok(usize::from(header.count()) + 2)
    }
}

fn add_work(usage: &mut Usage, limits: Limits, count: u64) -> Result<(), Error> {
    usage.work = usage
        .work
        .checked_add(count)
        .filter(|n| *n <= limits.work)
        .ok_or(Error::Limit("ordered register work"))?;
    Ok(())
}

fn add_host(usage: &mut Usage, limits: Limits, count: usize) -> Result<(), Error> {
    usage.host = usage
        .host
        .checked_add(count)
        .filter(|n| *n <= limits.host)
        .ok_or(Error::Limit("ordered register host bytes"))?;
    Ok(())
}

fn reserved<T>(usage: &mut Usage, limits: Limits, count: usize) -> Result<Vec<T>, Error> {
    let bytes = count
        .checked_mul(size_of::<T>())
        .ok_or(Error::Limit("register allocation"))?;
    add_host(usage, limits, bytes)?;
    add_work(usage, limits, bytes as u64)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| Error::Allocation)?;
    let excess = values
        .capacity()
        .checked_sub(count)
        .and_then(|n| n.checked_mul(size_of::<T>()))
        .ok_or(Error::Limit("register allocation capacity"))?;
    add_host(usage, limits, excess)?;
    Ok(values)
}

/// Bounded callers charge this immutable packet walk before planning. It never
/// reads array backing and never treats opaque NOP payloads as packets.
pub fn contains_indirect(buffers: &[&[u8]]) -> Result<bool, Error> {
    let mut found = false;
    for bytes in buffers {
        let mut at = 0usize;
        while at < bytes.len() {
            let words = packet_words(&bytes[at..]).map_err(|error| match error {
                Error::Unsupported(reason) => Error::Packet {
                    dword: at / 4,
                    reason,
                },
                Error::Packet { dword, reason } => Error::Packet {
                    dword: at / 4 + dword,
                    reason,
                },
                other => other,
            })?;
            let end = at
                .checked_add(words.checked_mul(4).ok_or(Error::Limit("packet extent"))?)
                .filter(|end| *end <= bytes.len())
                .ok_or(Error::Packet {
                    dword: at / 4,
                    reason: "truncated command packet",
                })?;
            found |= indirect_opcode(Header::new(word(bytes, at)));
            at = end;
        }
    }
    Ok(found)
}
