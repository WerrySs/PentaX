// SPDX-License-Identifier: GPL-2.0-or-later
use std::{env, error::Error, fs::File, io::Read, process::ExitCode};

mod entry;
mod gpu;
mod hle;
mod session;
mod shader;

fn main() -> ExitCode {
    match execute() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("PentaX: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.first().and_then(|s| s.to_str()) {
        Some("--version") if args.len() == 1 => {
            println!("PentaX {} (experimental)", env!("CARGO_PKG_VERSION"))
        }
        Some("self-test") if args.len() == 1 => {
            let result = pentax_core::demo::run()?;
            println!(
                "Reference guest passed: RAX={}, instructions={}, mapped={} bytes, resident={} bytes",
                result.result_rax, result.instructions, result.mapped_bytes, result.resident_bytes
            );
            println!("This validates a synthetic guest; commercial games are not supported yet.");
        }
        Some("self-test-gpu") if args.len() == 1 => gpu::run()?,
        Some("self-test-shader") if args.len() == 1 => shader::run()?,
        Some("self-test-hle") if args.len() == 1 => hle::run()?,
        Some("plan-session") => session::run(&args[1..])?,
        Some("probe-entry") => entry::run(&args[1..])?,
        Some("self-test-startup") if args.len() == 1 => {
            let report = pentax_core::process::startup::run_startup_fixture()?;
            println!(
                "Original interpreted startup passed: value={}, provider={}, service={}, constructors={}, services={}, instructions={}",
                report.value,
                report.provider_value,
                report.service_value,
                report.constructor_calls,
                report.service_calls,
                report.retired
            );
            println!(
                "Fixture state: modules=2, argc={}, argument assertions={}, TLS={}, independent TLS={}, independent zero tail={}, relocations={}",
                report.argc,
                report.argument_assertions,
                report.tls_initialized,
                report.independent_tls_initialized,
                report.independent_tls_zero_tail,
                report.applied_relocations
            );
            println!(
                "This validates only an original fixture policy through the bounded interpreter. PS5 startup, services and commercial games remain unsupported."
            );
        }
        Some("plan") if args.len() == 2 => {
            let bytes = read_runtime_image(&args[1])?;
            let view = pentax_loader::open_executable(&bytes)?;
            let plan = view.runtime_plan()?;
            let process = pentax_core::process::load_runtime(
                &plan,
                &pentax_core::process::LoadOptions::default(),
            )?;
            println!(
                "Runtime mapping: {:?}, headers={}, load segments={}, dynamic entries={}, TLS={}",
                plan.metadata.format,
                plan.metadata.program_headers.len(),
                plan.segments.len(),
                plan.dynamic.as_ref().map_or(0, |table| table.entries.len()),
                plan.tls.is_some()
            );
            println!(
                "Guest entry={:#x}, mapped={} bytes, resident={} bytes",
                process.entry,
                process.memory.mapped_bytes(),
                process.memory.resident_bytes()
            );
            println!(
                "Relocations, imports, guest stack and TLS are not initialized. No guest code was executed."
            );
        }
        Some("plan-relative") if args.len() == 2 => {
            let bytes = read_runtime_image(&args[1])?;
            let view = pentax_loader::open_executable(&bytes)?;
            let options = pentax_core::process::LoadOptions::default();
            let plan = pentax_linker::plan_relative(
                &view,
                &pentax_linker::Limits::default(),
                options.load_bias,
            )?;
            let process = pentax_core::process::load_relative(&plan, &options)?;
            println!(
                "Relative relocation staging: applied={}, deferred={}",
                process.applied_relocations(),
                process.deferred_relocations()
            );
            println!(
                "Guest entry={:#x}, mapped={} bytes, resident={} bytes",
                process.entry(),
                process.memory().mapped_bytes(),
                process.memory().resident_bytes()
            );
            println!(
                "Image remains unlinked: symbol and other relocations, imports, guest stack and TLS are not initialized. No guest code was executed."
            );
        }
        Some("link-info") if args.len() == 2 => {
            let bytes = read_runtime_image(&args[1])?;
            let view = pentax_loader::open_executable(&bytes)?;
            match pentax_linker::inspect(&view, &pentax_linker::Limits::default())? {
                pentax_linker::Inspection::NotDynamic => {
                    println!("No dynamic section; no linking tables were inspected.");
                }
                pentax_linker::Inspection::Dynamic(metadata) => {
                    let counts = metadata.counts();
                    let undefined = metadata
                        .symbols()
                        .iter()
                        .filter(|symbol| symbol.is_undefined() && symbol.name.is_some())
                        .count();
                    println!(
                        "Dynamic link metadata: symbols={}, named={}, undefined={}, RELA={}, PLT RELA={}",
                        counts.symbols,
                        counts.named_symbols,
                        undefined,
                        counts.rela,
                        counts.plt_rela
                    );
                    println!(
                        "Inspected tables={} bytes, retained names={} bytes, symbol references={}",
                        counts.table_bytes, counts.name_bytes, counts.symbol_references
                    );
                }
            }
            println!(
                "Symbols were not resolved and relocations were not applied. No guest code was executed."
            );
        }
        Some("sce-info") if args.len() == 2 => {
            let bytes = read_runtime_image(&args[1])?;
            let view = pentax_loader::open_executable(&bytes)?;
            match pentax_linker::inspect_sce(
                &view,
                &pentax_linker::Limits::default(),
                &pentax_linker::SceLimits::default(),
            )? {
                pentax_linker::SceInspection::NotDynamic => {
                    println!("No dynamic section; no SCE metadata was inspected.");
                }
                pentax_linker::SceInspection::Dynamic(metadata) => {
                    match metadata.descriptors() {
                        pentax_linker::SceDescriptors::Absent => {
                            println!("SCE descriptor family: absent");
                        }
                        pentax_linker::SceDescriptors::Decoded(descriptors) => {
                            println!(
                                "Decoded SCE descriptors: self modules={}, needed modules={}, exported libraries={}, imported libraries={}",
                                descriptors.module_info().len(),
                                descriptors.needed_modules().len(),
                                descriptors.export_libraries().len(),
                                descriptors.import_libraries().len()
                            );
                            let encodings = [
                                pentax_linker::DescriptorRole::ModuleInfo,
                                pentax_linker::DescriptorRole::NeededModule,
                                pentax_linker::DescriptorRole::ExportLibrary,
                                pentax_linker::DescriptorRole::ImportLibrary,
                            ]
                            .map(|role| descriptors.encoding(role));
                            println!(
                                "Descriptor role encodings: classic={}, observed _1={}",
                                encodings
                                    .iter()
                                    .filter(|&&encoding| encoding
                                        == Some(pentax_linker::DescriptorEncoding::Classic))
                                    .count(),
                                encodings
                                    .iter()
                                    .filter(|&&encoding| encoding
                                        == Some(
                                            pentax_linker::DescriptorEncoding::Variant1Observed
                                        ))
                                    .count()
                            );
                        }
                        pentax_linker::SceDescriptors::Conflicted(descriptors) => {
                            println!(
                                "SCE descriptor role conflicts: roles={}, raw classic={}, raw observed _1={}",
                                descriptors.conflicts().len(),
                                descriptors
                                    .conflicts()
                                    .iter()
                                    .map(|conflict| conflict.classic().len())
                                    .sum::<usize>(),
                                descriptors
                                    .conflicts()
                                    .iter()
                                    .map(|conflict| conflict.variant1().len())
                                    .sum::<usize>()
                            );
                        }
                    }
                    println!(
                        "Other metadata: DT_NEEDED={}, opaque attributes={}, opaque filenames={}, table bytes={}, metadata name bytes={}",
                        metadata.needed().len(),
                        metadata.attributes().len(),
                        metadata.original_filenames().len(),
                        metadata.table_bytes(),
                        metadata.metadata_name_bytes()
                    );
                    println!(
                        "Qualified symbol names: total={}, malformed={}",
                        metadata.qualified_symbols().len(),
                        metadata
                            .qualified_symbols()
                            .iter()
                            .filter(|symbol| symbol.name().is_err())
                            .count()
                    );
                    if let Some(associations) = metadata.import_associations() {
                        println!(
                            "Import selector declarations: checked={}, both declared={}, missing library={}, missing module={}",
                            associations.len(),
                            associations
                                .iter()
                                .filter(|entry| entry.is_declared())
                                .count(),
                            associations
                                .iter()
                                .filter(|entry| !entry.library().is_declared())
                                .count(),
                            associations
                                .iter()
                                .filter(|entry| !entry.module().is_declared())
                                .count()
                        );
                    } else {
                        println!(
                            "Import selector associations are unavailable for this descriptor family."
                        );
                    }
                }
            }
            println!(
                "Selector declarations do not resolve providers or addresses. No guest code was executed."
            );
        }
        Some("inspect") if args.len() == 2 => {
            let file = File::open(&args[1])?;
            let metadata = file.metadata()?;
            if !metadata.is_file() {
                return Err("inspection requires a regular executable file".into());
            }
            const MAX_HEADER_BYTES: u64 = 8 * 1024 * 1024;
            let mut bytes = Vec::new();
            file.take(MAX_HEADER_BYTES).read_to_end(&mut bytes)?;
            let image = pentax_loader::inspect_executable(&bytes, metadata.len())?;
            println!(
                "{:?} / ELF64 x86-64 | type={:#06x} | OSABI={} ABI={} | entry={:#x}",
                image.format,
                image.elf_type.raw(),
                image.os_abi,
                image.abi_version,
                image.entry
            );
            println!(
                "{} program headers, {} load segments",
                image.program_headers.len(),
                image
                    .program_headers
                    .iter()
                    .filter(|header| header.kind == pentax_loader::PT_LOAD)
                    .count()
            );
            for (index, segment) in image
                .program_headers
                .iter()
                .filter(|header| header.kind == pentax_loader::PT_LOAD)
                .enumerate()
            {
                println!(
                    "  {index}: VA={:#x} file={} memory={} flags={:#x} align={:#x}",
                    segment.vaddr,
                    segment.file_size,
                    segment.mem_size,
                    segment.flags.raw(),
                    segment.align
                );
            }
            if let Some(container) = image.self_metadata {
                println!(
                    "SELF descriptors={}, header inspection only; payload decryption and execution are not performed",
                    container.segments.len()
                );
            }
            println!("Inspection only. No guest code was executed.");
        }
        Some("default-config") if args.len() == 1 => {
            print!("{}", pentax_config::GlobalConfig::default().to_toml()?)
        }
        Some("--help" | "-h") | None => {
            println!(
                "PentaX {} — experimental PS5 emulator\n\nUsage: pentax-cli <command>\n  inspect <path>       Inspect ELF/SELF headers without running guest code\n  plan <path>          Validate and map required runtime segments without execution\n  plan-relative <path> Stage relative relocations and report deferred linking\n  plan-session --image <bias> <path> ...\n                      Stage an explicit primary and provider module set\n  link-info <path>     Inspect bounded dynamic symbols and RELA metadata\n  sce-info <path>      Inspect SCE descriptors and NID selector declarations\n  self-test            Load and execute the original reference guest\n  self-test-startup   Run the original interpreted module startup fixture\n  self-test-gpu        Inspect original GPU syntax without rendering or execution\n  self-test-hle        Run original linked memory/string service fixtures\n  self-test-shader     Check original scalar shader syntax without compilation\n  default-config       Print default global TOML preferences\n  --version            Print version",
                env!("CARGO_PKG_VERSION")
            );
            println!(
                "  probe-entry --image <path> [--memory-profile compact|runtime] [--physical-backing-directory <host-directory>] [--execution-profile probe|startup|extended-startup|long-startup] [--preload-libc <path> | --preload-libc-as <guest-path>=<host-path>] [--image <path> ...] [--module-file <guest-path>=<host-path> ...] [--data-file <guest-path>=<host-path> ...] [--data-directory <guest-mount>=<host-directory> ...] [--save-directory <host-directory>]\n                      Attempt the observed entry context in a supervised worker\n\nEntry probes use explicit position-independent inputs and a 30-second deadline, or 120 seconds with extended-startup. An explicit preload runs its observed DT_INIT with three zero arguments before main, on the same thread and cumulative budgets. A named preload enables existing-module retain requests after its actual initializer returns. Explicit module files remain unread until requested; bounded mapping/relocation, actual nested initialization and caller continuation share the same runtime. New module TLS and typed AMD64 relocations use bounded preadmitted storage. Missing initializers and unknown deferred forms remain unsupported. Explicit data files expose only the selected regular files through read-only guest mounts, with guest I/O budgets and retained read-only handles. They are separate from executable modules and are not read into the image arena. Explicit data directories resolve requested regular files inside retained directory capabilities, without admission-time enumeration or an ambient host-path fallback. Escaping links and invalid paths are refused. Directory opens retain bounded source-format entries; ordinary read, seek, getdirentries/getdents and directory stat use actual packed buffers. Directory pread, writable/device services and a complete game data profile remain unsupported. An optional save directory must already exist, be dedicated to this title and remain separate from game data roots. Save-memory services retain and reload checksummed PentaX bundles under exclusive session locking. The optional runtime memory profile uses the pinned direct-memory capacity and a separate host-availability budget; compact remains the default. An explicit physical backing directory enables bounded private sparse temporary files for runtime direct memory, independently capped private commitment, and automatic deletion on close; it must stay separate from data and save roots. Unavailable functions stop at use. Complete module lifecycle, game boot and a security sandbox are not provided."
            );
        }
        _ => return Err("invalid command; use --help".into()),
    }
    Ok(())
}

fn read_runtime_image(path: &std::ffi::OsStr) -> Result<Vec<u8>, Box<dyn Error>> {
    const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES {
        return Err(
            "payload inspection requires a regular executable no larger than 512 MiB".into(),
        );
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err("image grew beyond the payload inspection limit".into());
    }
    Ok(bytes)
}
