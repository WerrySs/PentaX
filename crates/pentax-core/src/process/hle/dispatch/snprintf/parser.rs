// SPDX-License-Identifier: GPL-2.0-only
// Source grammar adapted from pinned KytyPS5 guestPrintf.cpp; see provenance.
use super::*;

struct Format {
    address: u64,
    examined: u64,
    current: Option<u8>,
}

impl Format {
    fn peek(
        &mut self,
        engine: &mut Engine<'_, impl ServiceMemory>,
    ) -> Result<u8, MemoryServiceError> {
        if let Some(byte) = self.current {
            return Ok(byte);
        }
        if self.examined >= engine.budget.limits.max_span_bytes {
            return Err(MemoryServiceError::UnterminatedString {
                argument: PointerArgument::Second,
                maximum: engine.budget.limits.max_span_bytes,
            });
        }
        // Each fetched format byte has a fixed number of cached parser visits.
        engine.work(8)?;
        let byte = engine.byte(self.address)?;
        self.examined += 1;
        self.current = Some(byte);
        Ok(byte)
    }

    fn advance(&mut self) -> Result<(), MemoryServiceError> {
        self.address = self
            .address
            .checked_add(1)
            .ok_or(MemoryServiceError::AddressOverflow {
                argument: PointerArgument::Second,
            })?;
        self.current = None;
        Ok(())
    }

    fn number(
        &mut self,
        engine: &mut Engine<'_, impl ServiceMemory>,
    ) -> Result<u32, MemoryServiceError> {
        let mut number = 0_u32;
        while self.peek(engine)?.is_ascii_digit() {
            number = number
                .wrapping_mul(10)
                .wrapping_add(u32::from(self.peek(engine)? - b'0'));
            self.advance()?;
        }
        Ok(number)
    }
}

pub(super) fn render(
    engine: &mut Engine<'_, impl ServiceMemory>,
    cursor: &mut Cursor,
    address: u64,
) -> Result<(), MemoryServiceError> {
    let mut format = Format {
        address,
        examined: 0,
        current: None,
    };
    loop {
        let byte = format.peek(engine)?;
        if byte == 0 {
            return Ok(());
        }
        format.advance()?;
        if byte != b'%' {
            engine.emit(byte)?;
            continue;
        }
        let mut flags = 0;
        loop {
            let flag = match format.peek(engine)? {
                b'0' => FLAGS_ZEROPAD,
                b'-' => FLAGS_LEFT,
                b'+' => FLAGS_PLUS,
                b' ' => FLAGS_SPACE,
                b'#' => FLAGS_HASH,
                _ => break,
            };
            flags |= flag;
            format.advance()?;
        }
        let mut width = 0;
        if format.peek(engine)?.is_ascii_digit() {
            width = format.number(engine)?;
        } else if format.peek(engine)? == b'*' {
            let value = cursor.integer(engine)? as i32;
            if value == i32::MIN {
                return Err(MemoryServiceError::UnsupportedFormat(
                    "minimum dynamic width",
                ));
            }
            if value < 0 {
                flags |= FLAGS_LEFT;
            }
            width = value.unsigned_abs();
            format.advance()?;
        }
        let mut precision = 0;
        if format.peek(engine)? == b'.' {
            flags |= FLAGS_PRECISION;
            format.advance()?;
            if format.peek(engine)?.is_ascii_digit() {
                precision = format.number(engine)?;
            } else if format.peek(engine)? == b'*' {
                let value = cursor.integer(engine)? as i32;
                if value < 0 {
                    flags &= !FLAGS_PRECISION;
                } else {
                    precision = value as u32;
                }
                format.advance()?;
            }
        }
        match format.peek(engine)? {
            b'l' => {
                flags |= FLAGS_LONG;
                format.advance()?;
                if format.peek(engine)? == b'l' {
                    flags |= FLAGS_LONG_LONG;
                    format.advance()?;
                }
            }
            b'h' => {
                flags |= FLAGS_SHORT;
                format.advance()?;
                if format.peek(engine)? == b'h' {
                    flags |= FLAGS_CHAR;
                    format.advance()?;
                }
            }
            b't' | b'j' | b'z' => {
                flags |= FLAGS_LONG_LONG;
                format.advance()?;
            }
            _ => {}
        }
        within(
            width as u64,
            engine.budget.limits.max_span_bytes,
            ServiceLimit::SpanBytes,
        )?;
        within(
            precision as u64,
            engine.budget.limits.max_span_bytes,
            ServiceLimit::SpanBytes,
        )?;
        let spec = format.peek(engine)?;
        if spec == 0 {
            return Err(MemoryServiceError::UnsupportedFormat(
                "incomplete conversion",
            ));
        }
        format.advance()?;
        match spec {
            b'd' | b'i' | b'u' | b'x' | b'X' | b'o' | b'b' | b'p' => {
                let value = cursor.integer(engine)?;
                engine.work(128)?;
                numbers::format(engine, value, spec, width, precision, flags)?;
            }
            b'f' | b'F' | b'e' | b'E' | b'g' | b'G' => {
                let value = cursor.real(engine)?;
                engine.work(2048)?;
                floats::format(value, spec, width, precision, flags, &mut |byte| {
                    engine.emit(byte)
                })?;
            }
            b's' => {
                let address = cursor.integer(engine)?;
                strings::format(engine, address, width, precision, flags)?;
            }
            b'c' => {
                let value = cursor.integer(engine)? as u8;
                if flags & FLAGS_LEFT == 0 {
                    engine.padding(width.saturating_sub(1), b' ')?;
                }
                engine.emit(value)?;
                if flags & FLAGS_LEFT != 0 {
                    engine.padding(width.saturating_sub(1), b' ')?;
                }
            }
            // The public parser prints unknown specifier bytes and consumes no
            // value. In particular %n performs no pointer read or guest store.
            other => engine.emit(other)?,
        }
    }
}
