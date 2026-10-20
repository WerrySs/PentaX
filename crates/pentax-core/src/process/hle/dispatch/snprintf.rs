// SPDX-License-Identifier: GPL-2.0-only
// Adapted from KytyPS5 f186b357 guestPrintf.cpp and vaContext.h; see provenance.
// Formatting algorithm: Copyright (c) 2014-2019 Marco Paland, PALANDesign,
// with exponential conversion by Martijn Jasperse. Original algorithm is MIT.

//! Bounded source-profile formatting over the same paused guest-memory owner.
//! No host pointer, host variadic call or partial guest output is published.
use super::*;

mod floats;
mod numbers;
mod parser;
mod strings;
#[cfg(test)]
mod tests;

const FLAGS_ZEROPAD: u32 = 1;
const FLAGS_LEFT: u32 = 1 << 1;
const FLAGS_PLUS: u32 = 1 << 2;
const FLAGS_SPACE: u32 = 1 << 3;
const FLAGS_HASH: u32 = 1 << 4;
const FLAGS_UPPERCASE: u32 = 1 << 5;
const FLAGS_CHAR: u32 = 1 << 6;
const FLAGS_SHORT: u32 = 1 << 7;
const FLAGS_LONG: u32 = 1 << 8;
const FLAGS_LONG_LONG: u32 = 1 << 9;
const FLAGS_PRECISION: u32 = 1 << 10;
const FLAGS_ADAPT_EXP: u32 = 1 << 11;

struct Cursor {
    context: VariadicContext,
    gp: usize,
    fp: usize,
    overflow: Option<u64>,
}

impl Cursor {
    fn new(context: VariadicContext) -> Self {
        Self {
            context,
            gp: 0,
            fp: 0,
            overflow: None,
        }
    }

    fn stack(
        &mut self,
        engine: &mut Engine<'_, impl ServiceMemory>,
    ) -> Result<u64, MemoryServiceError> {
        let address = match self.overflow {
            Some(address) => address,
            None => self
                .context
                .stack_pointer
                .ok_or(MemoryServiceError::InvalidServiceState(
                    "missing snprintf stack capture",
                ))?
                .checked_add(8)
                .ok_or(MemoryServiceError::AddressOverflow {
                    argument: PointerArgument::Second,
                })?,
        };
        let next = address
            .checked_add(8)
            .ok_or(MemoryServiceError::AddressOverflow {
                argument: PointerArgument::Second,
            })?;
        let mut word = [0; 8];
        engine.read(address, &mut word)?;
        self.overflow = Some(next);
        Ok(u64::from_le_bytes(word))
    }

    fn integer(
        &mut self,
        engine: &mut Engine<'_, impl ServiceMemory>,
    ) -> Result<u64, MemoryServiceError> {
        if let Some(value) = self.context.gp.get(self.gp).copied() {
            self.gp += 1;
            Ok(value)
        } else {
            self.stack(engine)
        }
    }

    fn real(
        &mut self,
        engine: &mut Engine<'_, impl ServiceMemory>,
    ) -> Result<f64, MemoryServiceError> {
        if self.fp < 8 {
            let vectors =
                self.context
                    .vectors
                    .as_ref()
                    .ok_or(MemoryServiceError::InvalidServiceState(
                        "missing snprintf vector capture",
                    ))?;
            let bits = u64::from_le_bytes(vectors[self.fp][..8].try_into().unwrap());
            self.fp += 1;
            Ok(f64::from_bits(bits))
        } else {
            Ok(f64::from_bits(self.stack(engine)?))
        }
    }
}

struct Engine<'a, M> {
    memory: &'a mut M,
    budget: &'a mut ServiceBudget,
    output: Vec<u8>,
    held_extra: u64,
}

impl<M: ServiceMemory> Engine<'_, M> {
    fn work(&mut self, amount: u64) -> Result<(), MemoryServiceError> {
        self.budget.work = total(
            self.budget.work,
            amount,
            self.budget.limits.max_work_units,
            ServiceLimit::WorkUnits,
        )?;
        Ok(())
    }

    fn read(&mut self, address: u64, output: &mut [u8]) -> Result<(), MemoryServiceError> {
        let count = output.len() as u64;
        self.budget.total_bytes = total(
            self.budget.total_bytes,
            count,
            self.budget.limits.max_total_bytes,
            ServiceLimit::TotalBytes,
        )?;
        self.work(count)?;
        self.budget.string_reads += count;
        check_pointer(address, count, PointerArgument::Second)?;
        self.memory.preflight_read(address, count)?;
        self.memory.read(address, output)?;
        Ok(())
    }

    fn byte(&mut self, address: u64) -> Result<u8, MemoryServiceError> {
        let mut byte = [0];
        self.read(address, &mut byte)?;
        Ok(byte[0])
    }

    fn emit(&mut self, byte: u8) -> Result<(), MemoryServiceError> {
        let needed = self.output.len() as u64 + 1;
        within(
            needed,
            self.budget.limits.max_span_bytes,
            ServiceLimit::SpanBytes,
        )?;
        self.work(1)?;
        if self.output.len() == self.output.capacity() {
            let mut held = (self.output.capacity() as u64)
                .checked_add(self.held_extra)
                .ok_or(MemoryServiceError::CounterOverflow {
                    kind: ServiceLimit::ScratchBytes,
                })?;
            let available = self.budget.limits.max_scratch_bytes.saturating_sub(held);
            let target = (self.output.capacity() as u64)
                .saturating_mul(2)
                .max(16)
                .max(needed)
                .min(self.budget.limits.max_span_bytes)
                .min(available);
            within(
                held.checked_add(needed)
                    .ok_or(MemoryServiceError::CounterOverflow {
                        kind: ServiceLimit::ScratchBytes,
                    })?,
                self.budget.limits.max_scratch_bytes,
                ServiceLimit::ScratchBytes,
            )?;
            self.work(target + self.output.len() as u64)?;
            let mut next = self.budget.reserve(target, &mut held)?;
            self.budget.string_scratch_peak = self.budget.string_scratch_peak.max(held);
            next.clear();
            next.extend_from_slice(&self.output);
            self.output = next;
        }
        self.output.push(byte);
        Ok(())
    }

    fn padding(&mut self, amount: u32, byte: u8) -> Result<(), MemoryServiceError> {
        within(
            amount as u64,
            self.budget.limits.max_span_bytes,
            ServiceLimit::SpanBytes,
        )?;
        for _ in 0..amount {
            self.emit(byte)?;
        }
        Ok(())
    }
}

fn at(address: u64, offset: u64) -> Result<u64, MemoryServiceError> {
    address
        .checked_add(offset)
        .ok_or(MemoryServiceError::AddressOverflow {
            argument: PointerArgument::Second,
        })
}

pub(super) fn invoke(
    args: AbiArgs,
    context: Option<VariadicContext>,
    memory: &mut impl ServiceMemory,
    budget: &mut ServiceBudget,
) -> Result<ServiceReturn, MemoryServiceError> {
    let context = context.ok_or(MemoryServiceError::InvalidServiceState(
        "missing snprintf pre-syscall capture",
    ))?;
    let mut engine = Engine {
        memory,
        budget,
        output: Vec::new(),
        held_extra: 0,
    };
    engine.work(1)?;
    let mut cursor = Cursor::new(context);
    parser::render(&mut engine, &mut cursor, args.rdx)?;
    let count = engine.output.len();
    within(count as u64, i32::MAX as u64, ServiceLimit::SpanBytes)?;
    // The pinned source finally copies its staged vector through host "%s".
    // Embedded NUL therefore ends the copy but not the logical return count.
    engine.work(count as u64 + 1)?;
    let prefix = engine
        .output
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(count);
    if args.rsi == 0 || prefix as u64 >= args.rsi {
        return Err(MemoryServiceError::UnsupportedFormat(
            "source snprintf truncation or zero capacity",
        ));
    }
    if prefix == count {
        engine.emit(0)?;
    }
    let bytes = prefix as u64 + 1;
    engine.budget.total_bytes = total(
        engine.budget.total_bytes,
        bytes,
        engine.budget.limits.max_total_bytes,
        ServiceLimit::TotalBytes,
    )?;
    let pages = bytes
        .checked_add(2 * PAGE_SIZE - 2)
        .and_then(|value| value.checked_div(PAGE_SIZE))
        .and_then(|value| value.checked_mul(PAGE_SIZE))
        .ok_or(MemoryServiceError::CounterOverflow {
            kind: ServiceLimit::WorkUnits,
        })?;
    engine.work(bytes + pages)?;
    check_pointer(args.rdi, bytes, PointerArgument::First)?;
    engine.memory.preflight_write(args.rdi, bytes)?;
    engine
        .memory
        .write_fallible(args.rdi, &engine.output[..prefix + 1])?;
    engine.budget.string_writes += bytes;
    Ok(engine
        .budget
        .complete(KnownFunction::Snprintf, count as u64))
}
