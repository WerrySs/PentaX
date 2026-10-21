// SPDX-License-Identifier: GPL-2.0-only
//! Explicit private output of already retained guest-console suffixes.
//! No decoding, guest-memory read, stream consumption or protocol payload.

use super::ProbeError;
use pentax_core::process::hle::PreparedNativeKernelStartup;
use pentax_kernel::OutputStream;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::OpenOptionsExt;

const TAIL_BYTES: usize = 4096;
const HEADER_BYTES: usize = 20;

pub(super) fn open(path: &OsStr) -> Result<File, ProbeError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT.
        .open(path)
        .map_err(|_| ProbeError::Input("private-console-open"))?;
    if !file.metadata().is_ok_and(|metadata| metadata.is_file()) {
        return Err(ProbeError::Input("private-console-file"));
    }
    Ok(file)
}

/// Called only after the execution runner has returned, while this actual
/// session still owns its buffers. Slices borrow output without consuming it.
pub(super) fn capture(
    output: &mut impl Write,
    session: &PreparedNativeKernelStartup<'_, '_>,
) -> io::Result<()> {
    write_tails(
        output,
        [
            session.output_tail(OutputStream::Stdout, TAIL_BYTES),
            session.output_tail(OutputStream::Stderr, TAIL_BYTES),
            session.output_tail(OutputStream::StdinCompatibility, TAIL_BYTES),
        ],
    )
}

fn write_tails(output: &mut impl Write, streams: [&[u8]; 3]) -> io::Result<()> {
    // Clamp independently of the caller. Length conversion and total output
    // are bounded by three 4096-byte tails and this fixed 20-byte header.
    let tails = streams.map(|bytes| &bytes[bytes.len().saturating_sub(TAIL_BYTES)..]);
    let mut header = [0; HEADER_BYTES];
    header[..8].copy_from_slice(b"PXCNSL01");
    for (index, bytes) in tails.iter().enumerate() {
        let at = 8 + index * 4;
        header[at..at + 4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
    }
    output.write_all(&header)?;
    for bytes in tails {
        output.write_all(bytes)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_console_writer_retains_only_bounded_suffixes_in_fixed_stream_order() {
        let stdout: Vec<u8> = (0..5000)
            .map(|index| (index as u8).wrapping_mul(17))
            .collect();
        let stderr = [0, 0xff, 0x80, b'\n', 0x1b];
        let stdin = [0xa5; 9000];
        let mut output = Vec::new();
        write_tails(&mut output, [&stdout, &stderr, &stdin]).unwrap();
        assert_eq!(&output[..8], b"PXCNSL01");
        assert_eq!(&output[8..20], &[0, 16, 0, 0, 5, 0, 0, 0, 0, 16, 0, 0]);
        assert_eq!(&output[20..20 + 4096], &stdout[904..]);
        assert_eq!(&output[20 + 4096..20 + 4101], &stderr);
        assert_eq!(&output[20 + 4101..], &stdin[4904..]);
        assert!(output.len() <= 12_308);
        output.clear();
        write_tails(&mut output, [&stdout; 3]).unwrap();
        assert_eq!(output.len(), 12_308);
        output.clear();
        write_tails(&mut output, [&[]; 3]).unwrap();
        assert_eq!(output.len(), 20);
        assert_eq!(&output[8..], &[0; 12]);
    }

    #[test]
    fn binary_console_writer_propagates_partial_write_failure_without_modifying_inputs() {
        struct Refused {
            accepted: usize,
        }
        impl Write for Refused {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.accepted == 23 {
                    return Err(io::Error::other("original output refusal"));
                }
                let count = bytes.len().min(23 - self.accepted);
                self.accepted += count;
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let streams = [[0, 0xff, 0x80, 0x7f]; 3];
        let mut writer = Refused { accepted: 0 };
        assert!(
            write_tails(
                &mut writer,
                streams.each_ref().map(|bytes| bytes.as_slice())
            )
            .is_err()
        );
        assert_eq!(writer.accepted, 23);
        assert_eq!(streams, [[0, 0xff, 0x80, 0x7f]; 3]);
    }
}
