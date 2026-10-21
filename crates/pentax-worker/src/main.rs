// SPDX-License-Identifier: GPL-2.0-or-later

//! Dedicated fault-isolation process. This worker is not a security sandbox.
//! Named original fixtures and an explicit bounded supplied-image entry probe.

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod image_probe;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
#[allow(unsafe_code)]
mod native_session;

use pentax_platform::{OriginalFixture, PlatformError};
use std::io::{self, Write};
use std::process::ExitCode;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod pm4_compute;

const PREFIX: &str = "PENTAX_WORKER\t1";

fn main() -> ExitCode {
    match execute() {
        Ok(status) => ExitCode::from(status),
        Err(error) => {
            eprintln!("PentaX worker protocol I/O failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute() -> io::Result<u8> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let strings: Vec<_> = args.iter().map(|value| value.to_str()).collect();
    if args.first().is_some_and(|value| value == "probe-entry") {
        return execute_image_probe(&args[1..]);
    }
    let fixture = match strings.as_slice() {
        [Some("--version")] => {
            println!("PentaX worker {} (experimental)", env!("CARGO_PKG_VERSION"));
            return Ok(0);
        }
        [] | [Some("--help" | "-h")] => {
            println!(
                "Usage: pentax-worker fixture <return42|trapped-sum42|guest-stack-entry42|native-block-dispatch|prepared-kernel-startup|invalid-instruction|unsupported-syscall>\n       pentax-worker gpu-fixture <compute-buffer|triangle-rgba|pm4-compute>\n       pentax-worker probe-entry --image <path> [--memory-profile compact|runtime] [--execution-profile probe|startup|extended-startup] [--preload-libc <path> | --preload-libc-as <guest-path>=<host-path>] [--libc-memory-builtins off|checked] [--image <path> ...] [--module-file <guest-path>=<host-path> ...] [--data-file <guest-path>=<host-path> ...] [--data-directory <guest-mount>=<host-directory> ...]\nNamed fixtures use original diagnostic programs. probe-entry admits explicit position-independent images read-only and attempts their observed main-call context; use pentax-cli for profile-specific process supervision (30 seconds, or 120 for extended-startup). An explicit preload runs its observed DT_INIT with three zero arguments before main on the same thread and cumulative budgets. A named preload enables existing-module retain requests after its actual initializer returns. Explicit module files remain unread until requested; bounded mapping/relocation, actual nested initialization and caller continuation share the same runtime. New module TLS and typed AMD64 relocations use bounded preadmitted storage. Missing initializers and unknown deferred forms remain unsupported. Explicit data files expose only the selected regular files through read-only guest mounts; their contents are read on guest file calls under kernel I/O budgets. Data files never become executable module inputs. Explicit data directories resolve requested regular files inside retained directory capabilities, without admission-time enumeration or an ambient host-path fallback. Escaping links and invalid paths are refused. Directory opens retain bounded source-format entries; ordinary read, seek, getdirentries/getdents and directory stat use actual packed buffers. Directory pread, writable/device services and a complete game data profile remain unsupported. The optional runtime memory profile uses the pinned direct-memory capacity and a separate host-availability budget; compact remains the default. Unavailable functions stop at use. guest-stack-entry42 checks an owned guarded stack and an observed entry convention. native-block-dispatch checks translated blocks and full context restoration. prepared-kernel-startup uses checked native memory, typed TLS, modeled FS operations, owned entry arguments and libc/libkernel services. gpu-fixture verifies original AMD programs on Vulkan; pm4-compute reads original PM4, shader and buffer data from owned guest pages and verifies fenced writeback. trapped-sum42 uses a synthetic service, not a PS5 syscall. Native execution is not a sandbox. Complete firmware TCB/DTV, module lifecycle, arbitrary-title startup and unsupported kernel services remain unavailable."
            );
            return Ok(0);
        }
        [Some("gpu-fixture"), Some("compute-buffer")] => {
            return execute_gpu_fixture(pentax_platform::vulkan::OriginalGpuFixture::ComputeBuffer);
        }
        [Some("gpu-fixture"), Some("triangle-rgba")] => {
            return execute_gpu_fixture(pentax_platform::vulkan::OriginalGpuFixture::TriangleRgba);
        }
        [Some("gpu-fixture"), Some("pm4-compute")] => {
            return execute_pm4_compute();
        }
        [Some("fixture"), Some("prepared-kernel-startup")] => {
            return execute_prepared_fixture();
        }
        [Some("fixture"), Some("return42")] => OriginalFixture::Return42,
        [Some("fixture"), Some("trapped-sum42")] => OriginalFixture::TrappedSum42,
        [Some("fixture"), Some("guest-stack-entry42")] => OriginalFixture::GuestStackEntry42,
        [Some("fixture"), Some("native-block-dispatch")] => OriginalFixture::NativeBlockDispatch,
        [Some("fixture"), Some("invalid-instruction")] => OriginalFixture::InvalidInstruction,
        [Some("fixture"), Some("unsupported-syscall")] => OriginalFixture::UnsupportedSyscall,
        _ => {
            protocol("RESULT\trejected\tcode=invalid-command")?;
            return Ok(64);
        }
    };
    // The syscall fixture is rejected before even configuring the native backend.
    if fixture == OriginalFixture::UnsupportedSyscall {
        return match pentax_platform::execute_original_fixture(fixture) {
            Err(error) => report_error(error),
            Ok(_) => unreachable!("the closed syscall fixture never executes"),
        };
    }
    if let Err(error) = pentax_platform::configure_worker_fault_policy() {
        return report_error(error);
    }
    protocol("READY\tbackend=windows-x86_64-native")?;
    let name = match fixture {
        OriginalFixture::Return42 => "return42",
        OriginalFixture::TrappedSum42 => "trapped-sum42",
        OriginalFixture::GuestStackEntry42 => "guest-stack-entry42",
        OriginalFixture::NativeBlockDispatch => "native-block-dispatch",
        OriginalFixture::InvalidInstruction => "invalid-instruction",
        OriginalFixture::UnsupportedSyscall => unreachable!("rejected above"),
    };
    protocol(&format!("RUNNING\tfixture={name}"))?;
    match pentax_platform::execute_original_fixture_report(fixture) {
        Ok(result) => {
            if fixture == OriginalFixture::TrappedSum42 {
                protocol(&format!(
                    "RESULT\texit\tvalue={}\ttraps={}",
                    result.value, result.traps
                ))?;
            } else {
                protocol(&format!("RESULT\texit\tvalue={}", result.value))?;
            }
            Ok(0)
        }
        Err(error) => report_error(error),
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn execute_prepared_fixture() -> io::Result<u8> {
    if let Err(error) = pentax_platform::configure_worker_fault_policy() {
        return report_error(error);
    }
    protocol("READY\tbackend=windows-x86_64-prepared")?;
    protocol("RUNNING\tfixture=prepared-kernel-startup")?;
    match native_session::run_original() {
        Ok((observed, executed)) => {
            protocol(&format!(
                "RESULT\texit\tfixture=prepared-kernel-startup\tvalue={}\tresult={}\ttls0={}\ttls1={}\tthread={}\tmodules={}\trelocations={}\tkernel={}\tlibc={}\trequests={}\tblocks={}\tnative_instructions={}\tprimary_native_instructions={}\tsource_bytes={}\temitted_bytes={}\tdispatch_work={}\twindow_bytes={}\tadmission_work={}\tmapped_pages={}\tresident_pages={}\tmemory_checks={}\tmemory_bytes={}\tsegment_ops={}\tsegment_bytes={}",
                observed.value,
                observed.result,
                observed.first_tls_value,
                observed.second_tls_value,
                observed.thread_id,
                observed.modules,
                observed.applied_relocations,
                observed.kernel_services,
                observed.libc_services,
                observed.native_requests,
                executed.dispatch.blocks,
                executed.dispatch.native_instructions,
                executed.primary_native_instructions,
                executed.dispatch.source_bytes,
                executed.dispatch.emitted_bytes,
                executed.dispatch.work_units,
                executed.fetched_source_bytes,
                executed.admission_work,
                observed.mapped_pages,
                observed.resident_pages,
                executed.dispatch.memory_access_checks,
                executed.dispatch.memory_bytes_checked,
                executed.dispatch.segment_operations,
                executed.dispatch.segment_bytes,
            ))?;
            Ok(0)
        }
        Err(error) => {
            protocol(&format!("RESULT\terror\tdetail={error}"))?;
            Ok(1)
        }
    }
}

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn execute_prepared_fixture() -> io::Result<u8> {
    report_error(PlatformError::UnsupportedHost)
}

fn execute_gpu_fixture(fixture: pentax_platform::vulkan::OriginalGpuFixture) -> io::Result<u8> {
    use pentax_platform::vulkan::{GpuLimits, OriginalGpuFixture, VulkanError, run_original};
    if let Err(error) = pentax_platform::configure_worker_fault_policy() {
        return report_error(error);
    }
    let name = match fixture {
        OriginalGpuFixture::ComputeBuffer => "compute-buffer",
        OriginalGpuFixture::TriangleRgba => "triangle-rgba",
    };
    protocol("READY\tbackend=windows-vulkan")?;
    protocol(&format!("RUNNING\tfixture={name}"))?;
    match run_original(fixture, &GpuLimits::default()) {
        Ok(report) => {
            let usage = report.usage();
            protocol(&format!(
                "RESULT\texit\tfixture={name}\tbytes={}\tshader_words={}\tcommands={}",
                report.bytes().len(),
                usage.shader_words,
                usage.commands
            ))?;
            Ok(0)
        }
        Err(VulkanError::UnsupportedHost) => {
            protocol("RESULT\tunsupported\tcode=vulkan-host")?;
            Ok(3)
        }
        Err(error) => {
            protocol(&format!("RESULT\terror\tdetail={error}"))?;
            Ok(1)
        }
    }
}

fn execute_pm4_compute() -> io::Result<u8> {
    if let Err(error) = pentax_platform::configure_worker_fault_policy() {
        return report_error(error);
    }
    protocol("READY\tbackend=windows-pm4-vulkan")?;
    protocol("RUNNING\tfixture=pm4-compute")?;
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        match pm4_compute::run() {
            Ok((bytes, invocations)) => {
                protocol(&format!(
                    "RESULT\texit\tfixture=pm4-compute\tbytes={bytes}\tinvocations={invocations}"
                ))?;
                Ok(0)
            }
            Err(error) => {
                protocol(&format!("RESULT\terror\tdetail={error}"))?;
                Ok(1)
            }
        }
    }
    #[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
    {
        protocol("RESULT\tunsupported\tcode=vulkan-host")?;
        Ok(3)
    }
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
fn execute_image_probe(args: &[std::ffi::OsString]) -> io::Result<u8> {
    if let Err(error) = pentax_platform::configure_worker_fault_policy() {
        return report_error(error);
    }
    protocol("READY\tbackend=windows-x86_64-image-probe")?;
    let outcome = image_probe::run(args);
    let budget = outcome
        .error_budget()
        .map_or_else(String::new, |value| format!("\tbudget={value}"));
    let instruction = outcome.error_instruction();
    let public_function = outcome
        .error_public_function()
        .map_or_else(String::new, |name| format!("\tpublic_function={name}"));
    let runtime_failure = outcome
        .error_runtime_failure()
        .map_or_else(String::new, |name| format!("\truntime_failure={name}"));
    let reference = outcome
        .error_reference_hint()
        .map_or_else(String::new, |(name, scope)| {
            let mut fields = format!("\treference_match={scope}");
            if let Some(name) = name {
                fields.push_str(&format!("\treference_function={name}"));
            }
            fields
        });
    let (status, code, value) = match &outcome.result {
        Ok(report) => ("exit", "outer-return", Some(report.return_value)),
        Err(error) => ("stopped", error.code(), None),
    };
    let reference_library = outcome
        .error_reference_library()
        .map_or_else(String::new, |category| {
            format!("\treference_library={category}")
        });
    // The unused value is omitted on a stop; no synthetic guest return escapes.
    let value = value.map_or_else(String::new, |value| format!("\tvalue={value}"));
    let fp_status = outcome.result.as_ref().map_or_else(
        |_| String::new(),
        |report| format!("\tfp_status={}", report.state.mxcsr & 0x3f),
    );
    let instruction =
        instruction.map_or_else(String::new, |value| format!("\tinstruction={value:?}"));
    let unavailable = outcome
        .unavailable_functions
        .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
    protocol(&format!(
        "RESULT\t{status}\tmode=entry-probe\tcode={code}\t{}\tunavailable_functions={unavailable}{value}{fp_status}{instruction}{budget}{public_function}{reference}{reference_library}{runtime_failure}",
        outcome.progress.protocol_fields(),
    ))?;
    Ok(outcome.exit_status())
}
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
fn execute_image_probe(_: &[std::ffi::OsString]) -> io::Result<u8> {
    report_error(PlatformError::UnsupportedHost)
}

fn report_error(error: PlatformError) -> io::Result<u8> {
    match error {
        PlatformError::UnsupportedGuestSyscall { number, offset } => {
            protocol(&format!(
                "RESULT\trejected\tcode=unsupported-syscall\tnumber={number}\toffset={offset}"
            ))?;
            Ok(2)
        }
        PlatformError::UnsupportedHost => {
            protocol(&format!(
                "RESULT\tunsupported\tos={}\tarch={}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ))?;
            Ok(3)
        }
        error => {
            protocol(&format!("RESULT\terror\tdetail={error}"))?;
            Ok(1)
        }
    }
}

fn protocol(fields: &str) -> io::Result<()> {
    let stdout = io::stdout();
    let mut output = stdout.lock();
    writeln!(output, "{PREFIX}\t{fields}")?;
    output.flush()
}
