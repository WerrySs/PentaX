// SPDX-License-Identifier: GPL-2.0-only
//! Host-only private evidence for one failed compute submission.
//! Captures are local diagnostic payloads, never public release artifacts.
use super::{Failure, PausedNativeMemory, compute};
use std::fmt::{self, Write as _};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

const ENV: &str = "PENTAX_PRIVATE_GPU_FAILURE";
const MAX_ROOT_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 4096;
const PAYLOAD_MARKER: &[u8] = b"\n-- root command bytes (binary) --\n";

/// The guest never supplies this path. No opt-in means no file/memory capture.
/// Every capture failure is ignored by the caller, preserving its original error.
pub(super) fn capture_from_env(
    memory: &PausedNativeMemory<'_>,
    root: compute::CommandRange,
    initial: compute::ComputeState,
    failure: &Failure,
) {
    if let Some(path) = std::env::var_os(ENV) {
        let _ = capture(Path::new(&path), memory, root, initial, failure);
    }
}

struct Header {
    bytes: [u8; MAX_HEADER_BYTES],
    length: usize,
}
impl fmt::Write for Header {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let mut count = value.len().min(self.bytes.len() - self.length);
        while !value.is_char_boundary(count) {
            count -= 1;
        }
        self.bytes[self.length..self.length + count].copy_from_slice(&value.as_bytes()[..count]);
        self.length += count;
        if count == value.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}

fn capture(
    path: &Path,
    memory: &PausedNativeMemory<'_>,
    root: compute::CommandRange,
    initial: compute::ComputeState,
    failure: &Failure,
) -> io::Result<()> {
    // The trusted host chooses an existing private parent directory. Existing
    // paths are never overwritten, including after the first recorded failure.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let length = root.size.min(MAX_ROOT_BYTES as u64) as usize;
    let mut bytes = Vec::new();
    let status = if length == 0 {
        "empty-root"
    } else if bytes.try_reserve_exact(length).is_err() {
        "allocation-unavailable"
    } else {
        bytes.resize(length, 0);
        // Same still-paused owner and selected submission root. GPU-read authority
        // is checked afresh. No IB, shader, inferred tail or guest write occurs.
        if memory.read_gpu(root.address, &mut bytes).is_ok() {
            "captured"
        } else {
            bytes.clear();
            "gpu-read-unavailable"
        }
    };
    let mut header = Header {
        bytes: [0; MAX_HEADER_BYTES],
        length: 0,
    };
    let formatted = write!(
        &mut header,
        "PENTAX_PRIVATE_GPU_FAILURE_V1\nerror={failure:?}\ninitial_compute_state={initial:?}\ninitial_state_note=Debug is redacted: known-register count only; register values are not captured\nroot_selection=first nonempty range of failed submission; not necessarily the failing range or child\nroot_snapshot=bytes read after failure\nroot_address=0x{:016x}\nroot_bytes={}\ncaptured_prefix_bytes={}\ncommand_byte_limit={}\nread_status={}\n",
        root.address,
        root.size,
        bytes.len(),
        MAX_ROOT_BYTES,
        status
    );
    file.write_all(&header.bytes[..header.length])?;
    if formatted.is_err() {
        file.write_all(b"\nheader_truncated=true\n")?;
    }
    file.write_all(PAYLOAD_MARKER)?;
    file.write_all(&bytes)?;
    Ok(())
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    #[test]
    fn private_root_capture_bounds_bytes_refuses_overwrite_and_preserves_the_original_failure() {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x20000,
                NativePermissions::READ_WRITE.with_gpu(true, false),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let source: Vec<u8> = (0..MAX_ROOT_BYTES + 16)
            .map(|index| (index % 251) as u8)
            .collect();
        memory.write(base, &source).unwrap();
        let failure = Failure::Preparation(compute::Error::Packet {
            dword: 7,
            reason: "original-private-packet-reason",
        });
        let root = compute::CommandRange {
            address: base,
            size: source.len() as u64,
        };
        let path = std::env::temp_dir().join(format!(
            "pentax-private-gpu-failure-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        capture(
            &path,
            &memory,
            root,
            compute::ComputeState::empty(),
            &failure,
        )
        .unwrap();
        let recorded = std::fs::read(&path).unwrap();
        let second = capture(
            &path,
            &memory,
            root,
            compute::ComputeState::empty(),
            &failure,
        );
        let unchanged = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(second.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(recorded, unchanged);
        let marker = recorded
            .windows(PAYLOAD_MARKER.len())
            .position(|bytes| bytes == PAYLOAD_MARKER)
            .unwrap();
        assert_eq!(
            &recorded[marker + PAYLOAD_MARKER.len()..],
            &source[..MAX_ROOT_BYTES]
        );
        assert!(marker <= MAX_HEADER_BYTES + b"\nheader_truncated=true\n".len());
        let header = std::str::from_utf8(&recorded[..marker]).unwrap();
        assert!(header.contains("original-private-packet-reason"));
        assert!(header.contains("known_registers: 0"));
        assert!(header.contains("root_selection=first nonempty range of failed submission; not necessarily the failing range or child"));
        assert!(header.contains("root_snapshot=bytes read after failure"));
        assert!(header.contains("captured_prefix_bytes=65536"));
        assert!(header.contains(&format!("root_address=0x{base:016x}")));
        assert!(matches!(
            &failure,
            Failure::Preparation(compute::Error::Packet {
                dword: 7,
                reason: "original-private-packet-reason",
            })
        ));
        assert_eq!(failure.preparation_category(), Some("compute-command"));
    }
}
