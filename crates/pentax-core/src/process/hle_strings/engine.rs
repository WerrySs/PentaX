// SPDX-License-Identifier: GPL-2.0-only
// Rust adaptation of the Kyty string interfaces, 2026-10-17.
// Source revisions and retained original copyright/license: ../hle_strings.rs.

use super::*;
use crate::memory::{ADDRESS_LIMIT, FallibleWriteError, PAGE_SIZE};

pub(super) fn invoke_with_gate<M: ServiceMemory>(
    operation: StringOperation,
    args: StringArguments,
    memory: &mut M,
    limits: &StringLimits,
    reservation_gate: &mut dyn FnMut() -> Result<(), StringError>,
) -> StringOutcome {
    let mut call = Call {
        memory,
        limits,
        usage: StringUsage::default(),
        held: 0,
        reservation_gate,
    };
    let result = call.work(1).and_then(|()| call.run(operation, args));
    StringOutcome {
        result,
        usage: call.usage,
    }
}

struct Call<'a, M: ServiceMemory> {
    memory: &'a mut M,
    limits: &'a StringLimits,
    usage: StringUsage,
    held: u64,
    reservation_gate: &'a mut dyn FnMut() -> Result<(), StringError>,
}

impl<M: ServiceMemory> Call<'_, M> {
    fn work(&mut self, amount: u64) -> Result<(), StringError> {
        self.usage.work_units = add(
            self.usage.work_units,
            amount,
            self.limits.max_work_units,
            StringLimit::WorkUnits,
        )?;
        Ok(())
    }

    fn io(&mut self, read: u64, write: u64, work: u64) -> Result<(), StringError> {
        let amount = read
            .checked_add(write)
            .ok_or(StringError::CounterOverflow {
                kind: StringLimit::Bytes,
            })?;
        add(
            self.usage.bytes(),
            amount,
            self.limits.max_bytes,
            StringLimit::Bytes,
        )?;
        self.work(work)?;
        // The combined total above bounds both individual additions as well.
        self.usage.read_bytes += read;
        self.usage.write_bytes += write;
        Ok(())
    }

    fn byte(
        &mut self,
        pointer: u64,
        index: u64,
        argument: StringArgument,
    ) -> Result<u8, StringError> {
        if index >= self.limits.max_scan_bytes {
            return Err(StringError::Unterminated {
                argument,
                limit: self.limits.max_scan_bytes,
            });
        }
        let address = pointer
            .checked_add(index)
            .ok_or(StringError::AddressOverflow { argument })?;
        range(address, 1, argument)?;
        self.io(1, 0, 2)?; // One checked byte access and one scan/termination test.
        let mut value = [0];
        self.memory.read(address, &mut value)?;
        Ok(value[0])
    }

    /// Bytes through NUL, or exactly count if that bounded input has no NUL.
    fn scan(
        &mut self,
        pointer: u64,
        count: Option<u64>,
        argument: StringArgument,
    ) -> Result<u64, StringError> {
        let mut index = 0;
        loop {
            if count == Some(index) {
                return Ok(index);
            }
            let byte = self.byte(pointer, index, argument)?;
            index += 1; // Successful guest access bounds this below ADDRESS_LIMIT.
            if byte == 0 {
                return Ok(index);
            }
        }
    }

    fn zeroed(&mut self, length: u64) -> Result<Vec<u8>, StringError> {
        add(
            self.held,
            length,
            self.limits.max_scratch_bytes,
            StringLimit::ScratchBytes,
        )?;
        self.work(length)?;
        let length = usize::try_from(length).map_err(|_| StringError::Allocation)?;
        let mut buffer = Vec::new();
        if length != 0 {
            (self.reservation_gate)()?;
            buffer
                .try_reserve_exact(length)
                .map_err(|_| StringError::Allocation)?;
        }
        let capacity = u64::try_from(buffer.capacity()).map_err(|_| StringError::Allocation)?;
        self.held = add(
            self.held,
            capacity,
            self.limits.max_scratch_bytes,
            StringLimit::ScratchBytes,
        )?;
        self.usage.scratch_peak_bytes = self.usage.scratch_peak_bytes.max(self.held);
        buffer.resize(length, 0); // Capacity was reserved and admitted above.
        Ok(buffer)
    }

    fn read_into(
        &mut self,
        pointer: u64,
        output: &mut [u8],
        argument: StringArgument,
    ) -> Result<(), StringError> {
        let length = output.len() as u64;
        range(pointer, length, argument)?;
        let pages = page_count(pointer, length);
        self.io(
            length,
            0,
            length
                .checked_add(pages)
                .ok_or(StringError::CounterOverflow {
                    kind: StringLimit::WorkUnits,
                })?,
        )?;
        self.memory.preflight_read(pointer, length)?;
        self.memory.read(pointer, output)?;
        Ok(())
    }

    fn snapshot(
        &mut self,
        pointer: u64,
        length: u64,
        argument: StringArgument,
    ) -> Result<Vec<u8>, StringError> {
        // Scans established the source first; no guest writes can race this borrow.
        let mut buffer = self.zeroed(length)?;
        self.read_into(pointer, &mut buffer, argument)?;
        Ok(buffer)
    }

    fn prepare_write(&mut self, pointer: u64, length: u64) -> Result<(), StringError> {
        range(pointer, length, StringArgument::First)?;
        let pages = page_count(pointer, length);
        // Possible page zeroing plus bounded metadata passes and byte copying.
        let work = pages
            .checked_mul(PAGE_SIZE + 4)
            .and_then(|value| value.checked_add(length))
            .ok_or(StringError::CounterOverflow {
                kind: StringLimit::WorkUnits,
            })?;
        self.io(0, length, work)?;
        self.memory.preflight_write(pointer, length)?;
        Ok(())
    }

    fn write(&mut self, pointer: u64, bytes: &[u8]) -> Result<(), StringError> {
        self.memory
            .write_fallible(pointer, bytes)
            .map_err(|error| match error {
                FallibleWriteError::Memory(error) => StringError::Memory(error),
                FallibleWriteError::Allocation => StringError::Allocation,
            })
    }

    fn compare(&mut self, args: StringArguments, count: Option<u64>) -> Result<u64, StringError> {
        range(
            args.first,
            u64::from(count != Some(0)),
            StringArgument::First,
        )?;
        range(
            args.second,
            u64::from(count != Some(0)),
            StringArgument::Second,
        )?;
        let mut index = 0;
        loop {
            if count == Some(index) {
                return Ok(0);
            }
            let first = self.byte(args.first, index, StringArgument::First)?;
            let second = self.byte(args.second, index, StringArgument::Second)?;
            self.work(1)?;
            if first != second {
                return Ok(if first < second {
                    u64::from(u32::MAX)
                } else {
                    1
                });
            }
            if first == 0 {
                return Ok(0);
            }
            index += 1;
        }
    }

    fn copy(
        &mut self,
        args: StringArguments,
        count: Option<u64>,
        concatenate: bool,
    ) -> Result<u64, StringError> {
        let nonempty = u64::from(count != Some(0));
        range(args.first, nonempty, StringArgument::First)?;
        range(args.second, nonempty, StringArgument::Second)?;
        if count == Some(0) {
            return Ok(args.first);
        }
        // strncpy's whole count is its output even when the first byte is NUL.
        if let Some(length) = count {
            self.prepare_write(args.first, length)?;
        }
        let prefix = if concatenate {
            self.scan(args.first, None, StringArgument::First)? - 1
        } else {
            0
        };
        let source = self.scan(args.second, count, StringArgument::Second)?;
        let length = count.unwrap_or(source);
        let full = prefix
            .checked_add(length)
            .ok_or(StringError::AddressOverflow {
                argument: StringArgument::First,
            })?;
        range(args.first, full, StringArgument::First)?;
        if overlap(args.first, full, args.second, source) {
            return Err(StringError::UnsupportedOverlap);
        }
        let destination = args.first + prefix;
        if count.is_none() {
            self.prepare_write(destination, length)?;
        }
        let mut snapshot = self.zeroed(length)?;
        self.read_into(
            args.second,
            &mut snapshot[..source as usize],
            StringArgument::Second,
        )?;
        self.write(destination, &snapshot)?;
        Ok(args.first)
    }

    fn find_character(&mut self, args: StringArguments, last: bool) -> Result<u64, StringError> {
        range(args.first, 1, StringArgument::First)?;
        let target = args.second as u8;
        let mut index = 0;
        let mut found = 0;
        loop {
            let byte = self.byte(args.first, index, StringArgument::First)?;
            self.work(1)?;
            if byte == target {
                found = args.first + index;
                if !last {
                    return Ok(found);
                }
            }
            if byte == 0 {
                return Ok(found);
            }
            index += 1;
        }
    }

    fn substring(&mut self, args: StringArguments) -> Result<u64, StringError> {
        range(args.first, 1, StringArgument::First)?;
        range(args.second, 1, StringArgument::Second)?;
        let needle_size = self.scan(args.second, None, StringArgument::Second)?;
        if needle_size == 1 {
            return Ok(args.first);
        }
        let haystack_size = self.scan(args.first, None, StringArgument::First)?;
        if needle_size > haystack_size {
            return Ok(0);
        }
        let needle = self.snapshot(args.second, needle_size, StringArgument::Second)?;
        let haystack = self.snapshot(args.first, haystack_size, StringArgument::First)?;
        let needle_length = needle.len() - 1;
        // Every candidate and every comparison is charged before examination;
        // repetitive inputs cannot turn the naive search into unbounded work.
        for start in 0..=haystack.len() - 1 - needle_length {
            self.work(1)?;
            let mut matched = true;
            for index in 0..needle_length {
                self.work(1)?;
                if haystack[start + index] != needle[index] {
                    matched = false;
                    break;
                }
            }
            if matched {
                return Ok(args.first + start as u64);
            }
        }
        Ok(0)
    }

    fn run(
        &mut self,
        operation: StringOperation,
        args: StringArguments,
    ) -> Result<u64, StringError> {
        match operation {
            StringOperation::Strcmp => self.compare(args, None),
            StringOperation::Strncmp => self.compare(args, Some(args.third)),
            StringOperation::Strlen => {
                range(args.first, 1, StringArgument::First)?;
                Ok(self.scan(args.first, None, StringArgument::First)? - 1)
            }
            StringOperation::Strcpy => self.copy(args, None, false),
            StringOperation::Strncpy => self.copy(args, Some(args.third), false),
            StringOperation::Strcat => self.copy(args, None, true),
            StringOperation::Strchr => self.find_character(args, false),
            StringOperation::Strrchr => self.find_character(args, true),
            StringOperation::Strstr => self.substring(args),
        }
    }
}

fn add(used: u64, amount: u64, maximum: u64, kind: StringLimit) -> Result<u64, StringError> {
    let requested = used
        .checked_add(amount)
        .ok_or(StringError::CounterOverflow { kind })?;
    if requested > maximum {
        return Err(StringError::LimitExceeded {
            kind,
            requested,
            maximum,
        });
    }
    Ok(requested)
}

fn range(pointer: u64, length: u64, argument: StringArgument) -> Result<(), StringError> {
    if length != 0 && pointer == 0 {
        return Err(StringError::NullPointer { argument });
    }
    if pointer >= ADDRESS_LIMIT
        || pointer
            .checked_add(length)
            .is_none_or(|end| end > ADDRESS_LIMIT)
    {
        return Err(StringError::AddressOverflow { argument });
    }
    Ok(())
}

fn page_count(pointer: u64, length: u64) -> u64 {
    if length == 0 {
        0
    } else {
        ((pointer % PAGE_SIZE) + length).div_ceil(PAGE_SIZE)
    }
}

fn overlap(first: u64, first_size: u64, second: u64, second_size: u64) -> bool {
    // Callers checked both complete ends before comparing these nonempty spans.
    first < second + second_size && second < first + first_size
}
