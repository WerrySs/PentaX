// SPDX-License-Identifier: GPL-2.0-only
//! Host-selected private shader evidence at an actual failed dispatch preparation.
//! Captured bytes stay local and never belong in public artifacts.
use super::{PausedNativeMemory, compute};
use std::fmt::{self, Write as _};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::Path;

const ENV: &str = "PENTAX_PRIVATE_COMPUTE_SHADER_FAILURE";
const MAX_WORDS: usize = 16 * 1024;
const MAX_HEADER_BYTES: usize = 4096;
const MARKER: &[u8] = b"\n-- shader prefix bytes (binary) --\n";

pub(super) fn capture_from_env(
    memory: &PausedNativeMemory<'_>,
    dispatch: &compute::Dispatch,
    error: &compute::Error,
) {
    // Diagnostic extent is independent of the emulated decoder budget. Every
    // word still needs current owner GPU-read authority; the prefix may include
    // padding and never proves a complete program or a resource descriptor.
    if let Some(path) = std::env::var_os(ENV)
        && let Ok(address) = dispatch.shader_address()
    {
        let _ = capture(Path::new(&path), memory, address, MAX_WORDS, error);
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
    address: u64,
    requested_words: usize,
    error: &compute::Error,
) -> io::Result<()> {
    // This trusted host path is separate from the root-command capture. Failure
    // or an existing path cannot overwrite evidence or change the original Err.
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let maximum = requested_words.min(MAX_WORDS) * 4;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(maximum)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    let mut status = "word-limit";
    for offset in (0..maximum).step_by(4) {
        let Some(at) = address.checked_add(offset as u64) else {
            status = "address-overflow";
            break;
        };
        let mut word = [0; 4];
        // Recheck current GPU-read authority through the same paused owner for
        // every word. No pointer, indirect table, decoder or guessed tail is used.
        if memory.read_gpu(at, &mut word).is_err() {
            status = "gpu-read-unavailable";
            break;
        }
        bytes.extend_from_slice(&word);
    }
    let mut header = Header {
        bytes: [0; MAX_HEADER_BYTES],
        length: 0,
    };
    write!(
        &mut header,
        "PENTAX_PRIVATE_COMPUTE_SHADER_FAILURE_V1\nerror={error:?}\nshader_address=0x{address:016x}\nmaximum_prefix_bytes={maximum}\ncaptured_prefix_bytes={}\nread_status={status}\nsnapshot=after failed preparation; prefix is not a complete shader or failing instruction proof\n",
        bytes.len()
    ).map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
    file.write_all(&header.bytes[..header.length])?;
    file.write_all(MARKER)?;
    file.write_all(&bytes)
}

#[cfg(all(test, target_os = "windows", target_arch = "x86_64"))]
mod tests {
    use super::*;
    use pentax_platform::native_memory::{NativeAddressSpace, NativePermissions};

    #[test]
    fn failed_preparation_shader_prefix_is_owned_bounded_partial_and_never_overwritten() {
        let mut owner = NativeAddressSpace::reserve_any(Default::default(), 0x20000).unwrap();
        let base = owner.reserved_ranges().next().unwrap().address;
        owner
            .map_at(
                base,
                0x10000,
                NativePermissions::READ_WRITE.with_gpu(true, false),
            )
            .unwrap();
        let mut memory = owner.pause().unwrap();
        let source: Vec<u8> = (0..0x10000).map(|n| (n % 239) as u8).collect();
        memory.write(base, &source).unwrap();
        let error = compute::Error::Unsupported("original synthetic preparation refusal");
        let directory = std::env::temp_dir().join(format!(
            "pentax-private-shader-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        for (name, start, requested, expected, status) in [
            ("bounded", base, usize::MAX, 65536, "word-limit"),
            ("partial", base + 0xfff0, 8, 16, "gpu-read-unavailable"),
            ("denied", base + 0x10000, 8, 0, "gpu-read-unavailable"),
            ("small", base, 2, 8, "word-limit"),
        ] {
            let path = directory.join(name);
            capture(&path, &memory, start, requested, &error).unwrap();
            let raw = std::fs::read(&path).unwrap();
            let marker = raw.windows(MARKER.len()).position(|b| b == MARKER).unwrap();
            assert!(marker <= MAX_HEADER_BYTES);
            assert!(raw.len() <= MAX_HEADER_BYTES + MARKER.len() + MAX_WORDS * 4);
            let header = std::str::from_utf8(&raw[..marker]).unwrap();
            assert!(header.starts_with("PENTAX_PRIVATE_COMPUTE_SHADER_FAILURE_V1\n"));
            assert!(header.contains("original synthetic preparation refusal"));
            assert!(header.contains(&format!("read_status={status}\n")));
            let payload = &raw[marker + MARKER.len()..];
            assert_eq!(payload.len(), expected);
            let offset = (start - base) as usize;
            assert_eq!(payload, &source[offset..offset + expected]);
            assert_eq!(
                capture(&path, &memory, base, 1, &error).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert_eq!(std::fs::read(&path).unwrap(), raw);
            std::fs::remove_file(&path).unwrap();
        }
        std::fs::remove_dir(&directory).unwrap();
        assert!(matches!(
            error,
            compute::Error::Unsupported("original synthetic preparation refusal")
        ));
    }
}
