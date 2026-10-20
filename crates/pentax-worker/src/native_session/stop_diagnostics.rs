// SPDX-License-Identifier: GPL-2.0-only

//! Closed diagnostic labels only. No output text, trap byte, PC or identifier
//! is reflected into the protocol, and observation never resumes a stopped CPU.
use pentax_codegen::{Code, Instruction, Mnemonic, OpKind, UnsupportedReason};
use pentax_core::process::hle::{NativeServiceAction, PreparedNativeKernelStartup};
use pentax_kernel::OutputStream;

mod lookahead;

const TAIL_BYTES: usize = 256;
const HINTS: [(&[u8], &str); 17] = [
    (b"assert", "assertion"),
    (b"fatal", "fatal"),
    (b"abort", "abort"),
    (b"panic", "panic"),
    (b"error", "error"),
    (b"failed", "failure"),
    (b"memory", "memory"),
    (b"alloc", "allocation"),
    (b"thread", "thread"),
    (b"mutex", "mutex"),
    (b"semaphore", "semaphore"),
    (b"clock", "clock"),
    (b"time", "time"),
    (b"gpu", "gpu"),
    (b"file", "file"),
    (b"buffer", "buffer"),
    (b"init", "initialization"),
];

#[derive(Debug)]
enum Trap {
    DebugBreak,
    SoftwareInterrupt,
}
#[derive(Debug, Default)]
pub(super) struct StopDiagnostics {
    form: Option<(Code, Option<UnsupportedReason>)>,
    snapshot_candidates: Option<lookahead::SnapshotCandidates>,
    trap: Option<Trap>,
    primary: bool,
    hints: u32,
    memory_calls: u64,
    memory_errors: u64,
    last_memory: Option<MemoryCall>,
    last_error: Option<(&'static str, &'static str)>,
    last_map_refusal: Option<pentax_kernel::MemoryMapRefusal>,
    mapped_bytes: u64,
    handles: usize,
    native: Option<pentax_platform::native_memory::NativeMemoryStatistics>,
}
#[derive(Debug)]
struct MemoryCall {
    name: &'static str,
    bytes: u64,
    fixed: bool,
}
impl StopDiagnostics {
    /// Planning hints from the pre-step snapshot only. This grants no live-code
    /// assurance and cannot alter execution, cache admission or the actual stop.
    pub(super) fn capture_snapshot_candidates(&mut self, snapshot: &[u8]) {
        self.snapshot_candidates = Some(lookahead::SnapshotCandidates::capture(snapshot));
    }

    pub(super) fn observe_failure(
        &mut self,
        name: Option<&'static str>,
        args: [u64; 6],
        usage: pentax_kernel::Usage,
        native: pentax_platform::native_memory::NativeMemoryStatistics,
    ) {
        let Some(name) = name else { return };
        let (bytes, fixed) = match name {
            "KernelAllocateDirectMemory" => (args[2], false),
            "KernelAllocateMainDirectMemory" => (args[0], false),
            "KernelMapDirectMemory"
            | "KernelMapNamedDirectMemory"
            | "KernelMapFlexibleMemory"
            | "KernelMapNamedFlexibleMemory" => (args[1], args[3] & 0x10 != 0),
            _ => return,
        };
        self.memory_calls = self.memory_calls.saturating_add(1);
        self.memory_errors = self.memory_errors.saturating_add(1);
        self.last_memory = Some(MemoryCall { name, bytes, fixed });
        self.last_error = Some((name, "typed-stop"));
        self.last_map_refusal = None;
        self.mapped_bytes = usage.mapped_bytes;
        self.handles = usage.handles_live;
        self.native = Some(native);
    }
    pub(super) fn observe_service(
        &mut self,
        name: Option<&'static str>,
        args: [u64; 6],
        action: &NativeServiceAction,
        refusal: Option<pentax_kernel::MemoryMapRefusal>,
    ) {
        let NativeServiceAction::Returned(value) = action else {
            return;
        };
        let Some(name) = name else { return };
        // Only exact public memory APIs with an integer status result qualify.
        // Pointer returns and arbitrary syscall results must never be decoded.
        let (bytes, fixed) = match name {
            "KernelAllocateDirectMemory" => (args[2], false),
            "KernelAllocateMainDirectMemory" => (args[0], false),
            "KernelMapDirectMemory"
            | "KernelMapNamedDirectMemory"
            | "KernelMapFlexibleMemory"
            | "KernelMapNamedFlexibleMemory" => (args[1], args[3] & 0x10 != 0),
            _ => return,
        };
        self.memory_calls = self.memory_calls.saturating_add(1);
        self.last_memory = Some(MemoryCall { name, bytes, fixed });
        if *value != 0 {
            self.memory_errors = self.memory_errors.saturating_add(1);
            let label = match u32::try_from(*value).ok() {
                Some(0x8002_0001) => "permission",
                Some(0x8002_000c) => "out-of-memory",
                Some(0x8002_000e) => "bad-output",
                Some(0x8002_0010) => "busy",
                Some(0x8002_0016) => "invalid-argument",
                Some(0x8002_0023) => "range-unavailable",
                Some(0x8002_000d) => "access",
                _ => "other-result",
            };
            // A previous error is evidence, not a causal attribution.
            self.last_error = Some((name, label));
            self.last_map_refusal = if label == "out-of-memory" {
                refusal
            } else {
                None
            };
        }
    }
    pub(super) fn capture(
        &mut self,
        instruction: &Instruction,
        reason: Option<UnsupportedReason>,
        primary: bool,
        session: &PreparedNativeKernelStartup<'_, '_>,
    ) {
        // Both values are closed decoder/policy enums, never source bytes,
        // register values, addresses, names or immediate operands.
        self.form = Some((instruction.code(), reason));
        if instruction.mnemonic() != Mnemonic::Int {
            return;
        }
        // The pinned public runtime identifies INT41 as an assertion/debug
        // break. Other software interrupts stay distinct and are not emulated.
        let trap =
            if instruction.op0_kind() == OpKind::Immediate8 && instruction.immediate8() == 0x41 {
                Trap::DebugBreak
            } else {
                Trap::SoftwareInterrupt
            };
        let mut hints = 0;
        for stream in [
            OutputStream::StdinCompatibility,
            OutputStream::Stdout,
            OutputStream::Stderr,
        ] {
            hints |= hint_bits(session.output_tail(stream, TAIL_BYTES));
        }
        self.trap = Some(trap);
        self.primary = primary;
        self.hints = hints;
        let usage = session.kernel_usage();
        self.mapped_bytes = usage.mapped_bytes;
        self.handles = usage.handles_live;
    }
    pub(super) fn protocol_fields(&self) -> String {
        let mut fields = self.form.map_or_else(String::new, |(form, reason)| {
            let mut value = format!("\tstopped_form={form:?}");
            if let Some(reason) = reason {
                value.push_str(&format!("\tstopped_reason={reason:?}"));
            }
            value
        });
        fields.push_str(&self.memory_and_trap_fields());
        if let Some(candidates) = &self.snapshot_candidates {
            candidates.append_fields(&mut fields);
        }
        fields
    }
    fn memory_and_trap_fields(&self) -> String {
        let Some(trap) = &self.trap else {
            if let (Some(call), Some((_, "typed-stop"))) = (&self.last_memory, self.last_error) {
                return format!(
                    "\tmemory_calls={}\tmemory_errors={}\tmemory_mapped_bytes={}\tmemory_handles={}\tmemory_last_operation={}\tmemory_last_bytes={}\tmemory_last_fixed={}\tmemory_last_error=typed-stop{}",
                    self.memory_calls,
                    self.memory_errors,
                    self.mapped_bytes,
                    self.handles,
                    call.name,
                    call.bytes,
                    u8::from(call.fixed),
                    self.native_fields(),
                );
            }
            return String::new();
        };
        let mut labels = String::new();
        for (index, (_, label)) in HINTS.iter().enumerate() {
            if self.hints & (1 << index) != 0 {
                if !labels.is_empty() {
                    labels.push(',');
                }
                labels.push_str(label);
            }
        }
        if labels.is_empty() {
            labels.push_str("none");
        }
        let mut fields = format!(
            "\ttrap_kind={}\ttrap_origin={}\tconsole_hints={labels}\tmemory_calls={}\tmemory_errors={}\tmemory_mapped_bytes={}\tmemory_handles={}",
            match trap {
                Trap::DebugBreak => "debug-break",
                Trap::SoftwareInterrupt => "software-interrupt",
            },
            if self.primary { "primary" } else { "other" },
            self.memory_calls,
            self.memory_errors,
            self.mapped_bytes,
            self.handles,
        );
        if let Some(call) = &self.last_memory {
            fields.push_str(&format!(
                "\tmemory_last_operation={}\tmemory_last_bytes={}\tmemory_last_fixed={}",
                call.name,
                call.bytes,
                u8::from(call.fixed),
            ));
        }
        if let Some((name, label)) = self.last_error {
            fields.push_str(&format!(
                "\tmemory_last_error_operation={name}\tmemory_last_error={label}",
            ));
        }
        if let Some(refusal) = self.last_map_refusal {
            fields.push_str(&format!("\tmemory_map_refusal={}", refusal.label()));
        }
        fields
    }
    fn native_fields(&self) -> String {
        self.native.map_or_else(String::new, |value| {
            let mut fields = format!(
            "\tmemory_native_mapped_bytes={}\tmemory_native_committed_bytes={}\tmemory_native_reserved_bytes={}\tmemory_native_limit_bytes={}\tmemory_native_reserved_limit_bytes={}\tmemory_native_backing_bytes={}\tmemory_native_file_backed_bytes={}\tmemory_native_private_commit_limit_bytes={}",
            value.mapped_bytes, value.committed_bytes, value.reserved_bytes,
            value.limits.mapped_bytes, value.limits.reserved_bytes,
            value.backing_bytes,
            value.file_backed_bytes, value.private_commit_limit_bytes,
            );
            if let Some(failure) = value.host_failure {
                fields.push_str(&format!("\tmemory_host_operation={}\tmemory_host_code={}",
                    failure.operation.label(), failure.code));
            }
            fields
        })
    }
}
fn hint_bits(bytes: &[u8]) -> u32 {
    let bytes = &bytes[bytes.len().saturating_sub(TAIL_BYTES)..];
    HINTS
        .iter()
        .enumerate()
        .fold(0, |bits, (index, (pattern, _))| {
            bits | if bytes.windows(pattern.len()).enumerate().any(|(at, part)| {
                let boundary = at == 0
                    || !bytes[at - 1].is_ascii_alphanumeric()
                    || bytes[at - 1].is_ascii_lowercase() && bytes[at].is_ascii_uppercase();
                boundary && part.eq_ignore_ascii_case(pattern)
            }) {
                1 << index
            } else {
                0
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_hints_are_bounded_closed_labels_without_reflected_text() {
        let bytes = b"AsSeRt failed: untrusted-text-239 memory buffer\n\tguest_pc=private-value";
        let diagnostic = StopDiagnostics {
            trap: Some(Trap::DebugBreak),
            primary: true,
            hints: hint_bits(bytes),
            ..Default::default()
        };
        assert_eq!(
            diagnostic.protocol_fields(),
            "\ttrap_kind=debug-break\ttrap_origin=primary\tconsole_hints=assertion,failure,memory,buffer\tmemory_calls=0\tmemory_errors=0\tmemory_mapped_bytes=0\tmemory_handles=0"
        );
        let mut old = b"assert memory".to_vec();
        old.extend([b'x'; TAIL_BYTES]);
        assert_eq!(hint_bits(&old), 0);
        assert_eq!(hint_bits(b"profile runtime infinite"), 0);
        assert!(StopDiagnostics::default().protocol_fields().is_empty());
    }

    #[test]
    fn memory_hints_accept_only_exact_public_memory_apis_and_emit_no_addresses() {
        let mut diagnostic = StopDiagnostics::default();
        let args = [0xdead_beef, 16384, 3, 0x10, 0, 0];
        diagnostic.observe_service(
            Some("PthreadGetthreadid"),
            args,
            &NativeServiceAction::Returned(0x8002_000c),
            None,
        );
        diagnostic.observe_service(
            Some("KernelMapFlexibleMemory"),
            args,
            &NativeServiceAction::Returned(0),
            None,
        );
        diagnostic.observe_service(
            Some("KernelMapFlexibleMemory"),
            args,
            &NativeServiceAction::Returned(0x8002_000c),
            Some(pentax_kernel::MemoryMapRefusal::VirtualWindow),
        );
        assert!(diagnostic.protocol_fields().is_empty());
        diagnostic.trap = Some(Trap::DebugBreak);
        let fields = diagnostic.protocol_fields();
        assert!(fields.contains("\tmemory_calls=2\tmemory_errors=1\t"));
        assert!(fields.contains("\tmemory_last_error=out-of-memory"));
        assert!(fields.contains("\tmemory_map_refusal=virtual-window"));
        assert!(fields.contains("\tmemory_last_bytes=16384\tmemory_last_fixed=1"));
        assert!(!fields.contains("dead") && !fields.contains("3735928559"));
    }

    #[test]
    fn typed_memory_failure_reports_only_public_operation_size_and_aggregate_usage() {
        let mut diagnostic = StopDiagnostics::default();
        let args = [0xdead_beef, 0xfeed_face, 65536, 0xabba, 3, 0x1234_5678];
        let usage = pentax_kernel::Usage {
            mapped_bytes: 16384,
            handles_live: 2,
            ..Default::default()
        };
        let native = pentax_platform::native_memory::NativeMemoryStatistics {
            mapped_bytes: 32768,
            committed_bytes: 49152,
            backing_bytes: 65536,
            file_backed_bytes: 0,
            private_commit_limit_bytes: 98304,
            host_failure: Some(pentax_platform::native_memory::NativeHostMemoryFailure {
                operation: pentax_platform::native_memory::NativeHostMemoryOperation::SectionCommit,
                code: 1455,
            }),
            reserved_bytes: 65536,
            limits: pentax_platform::native_memory::NativeMemoryLimits {
                mapped_bytes: 98304,
                reserved_bytes: 131072,
            },
        };
        diagnostic.observe_failure(Some("PthreadGetthreadid"), args, usage, native);
        assert!(diagnostic.protocol_fields().is_empty());
        diagnostic.observe_failure(Some("KernelAllocateDirectMemory"), args, usage, native);
        assert_eq!(
            diagnostic.protocol_fields(),
            "\tmemory_calls=1\tmemory_errors=1\tmemory_mapped_bytes=16384\tmemory_handles=2\tmemory_last_operation=KernelAllocateDirectMemory\tmemory_last_bytes=65536\tmemory_last_fixed=0\tmemory_last_error=typed-stop\tmemory_native_mapped_bytes=32768\tmemory_native_committed_bytes=49152\tmemory_native_reserved_bytes=65536\tmemory_native_limit_bytes=98304\tmemory_native_reserved_limit_bytes=131072\tmemory_native_backing_bytes=65536\tmemory_native_file_backed_bytes=0\tmemory_native_private_commit_limit_bytes=98304\tmemory_host_operation=section-commit\tmemory_host_code=1455"
        );
    }
}
