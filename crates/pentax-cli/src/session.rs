// SPDX-License-Identifier: GPL-2.0-or-later

//! Explicit, bounded image intake for a closed, unstarted module session.

use std::{error::Error, ffi::OsString, fs::File, io::Read};

const MAX_SESSION_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MODULES: usize = 128;
const USAGE: &str = "use plan-session --image <decimal-or-0x-bias> <path> [--image <bias> <path> ...]; the first image is primary";

struct Input<'a> {
    bias: u64,
    path: &'a OsString,
}

pub(super) fn run(args: &[OsString]) -> Result<(), Box<dyn Error>> {
    let inputs = parse(args)?;
    // Validate the complete declared intake before reading any payload. Keep
    // the opened handles so replacement of a pathname cannot change the set.
    let mut files = Vec::with_capacity(inputs.len());
    let mut declared = 0_u64;
    for input in &inputs {
        let file = File::open(input.path)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err("session images must be regular executable files".into());
        }
        declared = declared
            .checked_add(metadata.len())
            .filter(|&total| total <= MAX_SESSION_BYTES)
            .ok_or("session images exceed the aggregate 512 MiB input limit")?;
        files.push(file);
    }
    let mut images = Vec::with_capacity(inputs.len());
    let mut read = 0;
    for file in files {
        let image = read_bounded(file, MAX_SESSION_BYTES - read)?;
        read += image.len() as u64;
        images.push(image);
    }
    let modules: Vec<_> = images
        .iter()
        .zip(&inputs)
        .map(|(bytes, input)| {
            pentax_loader::open_executable(bytes)
                .map(|view| pentax_linker::ModuleInput::new(view, input.bias))
        })
        .collect::<Result<_, _>>()?;
    let plan = pentax_linker::plan_symbolic(&modules, &pentax_linker::SymbolicLimits::default())?;
    let session = pentax_core::process::load_symbolic(
        &plan,
        &pentax_core::process::SessionLoadLimits::default(),
    )?;
    println!(
        "Module session staged: modules={}, applied={}, deferred={}",
        session.module_count(),
        session.applied_relocations(),
        session.deferred_relocations()
    );
    println!(
        "Primary entry={:#x}, mapped={} bytes, resident={} bytes",
        session.primary_entry(),
        session.memory().mapped_bytes(),
        session.memory().resident_bytes()
    );
    println!(
        "Provider eligibility: {} examined definitions were not offered because only import declarations matched.",
        plan.provider_exclusion_count()
    );
    match session.relocation_status() {
        pentax_core::process::RelocationStatus::CompleteWithinPolicy => {
            println!("Relocations are complete within the supported exact matching policy.");
        }
        pentax_core::process::RelocationStatus::Deferred { count } => {
            println!("Relocations remain incomplete: {count} deferred records.");
        }
    }
    println!(
        "The session is unstarted. Guest stack, TLS, services and general instruction execution are not initialized. No guest code was executed."
    );
    Ok(())
}

fn parse(args: &[OsString]) -> Result<Vec<Input<'_>>, Box<dyn Error>> {
    if args.is_empty() || !args.len().is_multiple_of(3) {
        return Err(USAGE.into());
    }
    if args.len() / 3 > MAX_MODULES {
        return Err("session exceeds the 128-module input limit".into());
    }
    args.chunks_exact(3)
        .map(|group| {
            if group[0] != "--image" {
                return Err(USAGE.into());
            }
            let bias = bias(&group[1])?;
            if !bias.is_multiple_of(pentax_loader::RUNTIME_PAGE_SIZE) {
                return Err("session staging requires a 16 KiB aligned image bias".into());
            }
            Ok(Input {
                bias,
                path: &group[2],
            })
        })
        .collect()
}

fn bias(value: &OsString) -> Result<u64, Box<dyn Error>> {
    let value = value
        .to_str()
        .ok_or("image bias must be ASCII decimal or 0x hexadecimal")?;
    let (digits, radix) = value
        .strip_prefix("0x")
        .map_or((value, 10), |digits| (digits, 16));
    if digits.is_empty()
        || !digits.bytes().all(|digit| match radix {
            16 => digit.is_ascii_hexdigit(),
            _ => digit.is_ascii_digit(),
        })
    {
        return Err("image bias must be unsigned decimal or 0x hexadecimal".into());
    }
    Ok(u64::from_str_radix(digits, radix)?)
}

fn read_bounded(reader: impl Read, remaining: u64) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Vec::new();
    reader.take(remaining + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > remaining {
        return Err("session images grew beyond the aggregate 512 MiB input limit".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn preserves_primary_order_paths_and_explicit_biases() {
        let args = arguments(&[
            "--image",
            "0x4000",
            "primary with spaces.elf",
            "--image",
            "262144",
            "provider.elf",
        ]);
        let inputs = parse(&args).unwrap();
        assert_eq!(inputs[0].bias, 0x4000);
        assert_eq!(inputs[0].path, "primary with spaces.elf");
        assert_eq!(inputs[1].bias, 0x40000);
        assert_eq!(inputs[1].path, "provider.elf");
    }

    #[test]
    fn rejects_ambiguous_arguments_overflow_and_unstageable_biases() {
        for values in [
            vec![],
            vec!["--image", "0"],
            vec!["image", "0", "image.elf"],
            vec!["--image", "-1", "image.elf"],
            vec!["--image", "+0", "image.elf"],
            vec!["--image", " 0", "image.elf"],
            vec!["--image", "0x", "image.elf"],
            vec!["--image", "0x10000000000000000", "image.elf"],
            vec!["--image", "1", "image.elf"],
        ] {
            assert!(parse(&arguments(&values)).is_err(), "{values:?}");
        }
        let args: Vec<_> = (0..=MAX_MODULES)
            .flat_map(|_| arguments(&["--image", "0", "image.elf"]))
            .collect();
        assert!(parse(&args).is_err());
    }

    #[test]
    fn rejects_growth_at_the_shared_read_boundary() {
        assert_eq!(read_bounded(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(read_bounded(&b"abcd"[..], 3).is_err());
        assert!(read_bounded(&b"x"[..], 0).is_err());
        assert!(read_bounded(&b""[..], 0).unwrap().is_empty());
    }
}
