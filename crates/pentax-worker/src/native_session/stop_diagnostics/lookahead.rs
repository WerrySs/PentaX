// SPDX-License-Identifier: GPL-2.0-only

//! Static candidates from an already retained, pre-step initialized code window.
//! The unconsumed suffix is not reattested at the stop and may be stale. These
//! aggregate public forms are planning hints, never an execution trace, a claim
//! of unsupported semantics, or authority to change execution or provider policy.
//!
//! Decoder semantics follow the existing codegen AMD-mode decoder. In particular,
//! `can_decode` alone does not establish a complete instruction; invalid/truncated
//! decoding terminates the scan, as does every non-fall-through flow category.
//! https://docs.rs/iced-x86/1.21.0/iced_x86/struct.Decoder.html
//! https://docs.rs/iced-x86/1.21.0/iced_x86/enum.FlowControl.html

use pentax_codegen::{Code, Decoder, DecoderOptions, FlowControl};
use std::fmt::{self, Write};

const MAX_BYTES: usize = 64;
const MAX_INSTRUCTIONS: usize = 32;
const MAX_FIELD_BYTES: usize = 4096;

#[derive(Debug, Default)]
pub(super) struct SnapshotCandidates {
    forms: [Option<(Code, u8)>; MAX_INSTRUCTIONS],
    used: usize,
}

impl SnapshotCandidates {
    /// No source reads or per-call heap allocations. The caller supplies only the
    /// retained suffix beginning at the checked unsupported-instruction offset.
    pub(super) fn capture(snapshot: &[u8]) -> Self {
        let snapshot = &snapshot[..snapshot.len().min(MAX_BYTES)];
        // IP is deliberately unrelated to the private source address. Only Code
        // is retained: targets, operands, immediate values and bytes are discarded.
        let mut decoder = Decoder::with_ip(64, snapshot, 0, DecoderOptions::AMD);
        let mut result = Self::default();
        for _ in 0..MAX_INSTRUCTIONS {
            if !decoder.can_decode() {
                break;
            }
            let instruction = decoder.decode();
            if instruction.is_invalid() || instruction.flow_control() != FlowControl::Next {
                break;
            }
            let code = instruction.code();
            let mut matched = false;
            for entry in result.forms[..result.used].iter_mut().flatten() {
                if entry.0 == code {
                    entry.1 += 1;
                    matched = true;
                    break;
                }
            }
            if !matched {
                result.forms[result.used] = Some((code, 1));
                result.used += 1;
            }
        }
        // Canonical public-enum order avoids reflecting instruction order.
        result.forms[..result.used]
            .sort_unstable_by_key(|entry| entry.map_or(0, |(code, _)| code as u32));
        result
    }

    /// A bounded stack formatter and fallible reservation keep a diagnostic
    /// formatting/allocation refusal from replacing the original terminal stop.
    pub(super) fn append_fields(&self, output: &mut String) {
        if self.used == 0 {
            return;
        }
        let mut field = FieldBuffer {
            bytes: [0; MAX_FIELD_BYTES],
            len: 0,
        };
        if field.write_str("\tsnapshot_candidate_forms=").is_err() {
            return;
        }
        for (index, entry) in self.forms[..self.used].iter().enumerate() {
            let Some((code, count)) = entry else { return };
            if index != 0 && field.write_str(",").is_err() {
                return;
            }
            if write!(field, "{code:?}:{count}").is_err() {
                return;
            }
        }
        let Ok(text) = std::str::from_utf8(&field.bytes[..field.len]) else {
            return;
        };
        if output.try_reserve_exact(text.len()).is_ok() {
            output.push_str(text);
        }
    }
}

struct FieldBuffer {
    bytes: [u8; MAX_FIELD_BYTES],
    len: usize,
}
impl fmt::Write for FieldBuffer {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.checked_add(text.len()).ok_or(fmt::Error)?;
        let destination = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        destination.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(bytes: &[u8]) -> String {
        let mut fields = String::new();
        SnapshotCandidates::capture(bytes).append_fields(&mut fields);
        fields
    }

    #[test]
    fn static_candidates_are_aggregate_private_value_independent_and_stop_at_bounds() {
        // Two independently chosen immediate values have the same public form.
        // A branch ends static fall-through; the following INC is never counted.
        let first = [
            0xb8, 0x44, 0x33, 0x22, 0x11, 0xb8, 0xdd, 0xcc, 0xbb, 0xaa, 0x90, 0xeb, 0, 0x48, 0xff,
            0xc0,
        ];
        let mut second = first;
        second[1..5].copy_from_slice(&[1, 2, 3, 4]);
        second[6..10].copy_from_slice(&[5, 6, 7, 8]);
        let summary = fields(&first);
        assert_eq!(summary, fields(&second));
        assert!(summary.contains("Mov_r32_imm32:2"));
        assert!(summary.contains("Nopd:1"));
        assert!(!summary.contains("Jmp") && !summary.contains("Inc"));
        assert!(!summary.contains("11223344") && !summary.contains("aabbccdd"));
        assert_eq!(fields(&[0x90, 0x0f]), "\tsnapshot_candidate_forms=Nopd:1");
        assert!(fields(&[0x0f]).is_empty());

        // The instruction cap applies even to the shortest possible encoding.
        assert_eq!(fields(&[0x90; 65]), "\tsnapshot_candidate_forms=Nopd:32");
        // Eight 9-byte NOPs: only seven complete instructions fit in 64 bytes;
        // decoding cannot borrow the remaining bytes past the byte boundary.
        let nop = [0x66, 0x0f, 0x1f, 0x84, 0, 0, 0, 0, 0];
        let mut long = [0; 72];
        for chunk in long.chunks_exact_mut(nop.len()) {
            chunk.copy_from_slice(&nop);
        }
        let bounded = SnapshotCandidates::capture(&long);
        assert_eq!(bounded.used, 1);
        assert_eq!(bounded.forms[0].map(|(_, count)| count), Some(7));

        // Formatter capacity refusal changes no already prepared terminal fields.
        let mut buffer = FieldBuffer {
            bytes: [0; MAX_FIELD_BYTES],
            len: MAX_FIELD_BYTES,
        };
        assert!(buffer.write_str("public").is_err());
        assert_eq!(buffer.len, MAX_FIELD_BYTES);
    }
}
