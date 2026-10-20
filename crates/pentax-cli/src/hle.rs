// SPDX-License-Identifier: GPL-2.0-or-later

//! Closed original consumers exercise the retained generated-provider gateway.

use pentax_core::process::hle::{
    run_hle_memory_fixture, run_hle_startup_fixture, run_hle_string_fixture,
};
use std::error::Error;

pub(super) fn run() -> Result<(), Box<dyn Error>> {
    let memory = run_hle_memory_fixture()?;
    let strings = run_hle_string_fixture()?;
    let startup = run_hle_startup_fixture()?;
    println!(
        "Original HLE memory passed: memcpy={}, memmove={}, memset={}, memcmp={}, attempts={}, instructions={}, bindings={}",
        memory.memcpy_calls,
        memory.memmove_calls,
        memory.memset_calls,
        memory.memcmp_calls,
        memory.service_attempts,
        memory.retired,
        memory.provider_bindings
    );
    println!(
        "Original HLE strings passed: registered functions={}, string calls={}, memory calls={}, attempts={}, instructions={}, bindings={}",
        strings.registered_functions,
        strings.string_calls,
        strings.memory_calls,
        strings.service_attempts,
        strings.retired,
        strings.provider_bindings
    );
    println!(
        "Retained service accounting: memory bytes={}, memory work={}, string/mixed bytes={}, string/mixed work={}",
        memory.requested_bytes, memory.work_units, strings.charged_bytes, strings.work_units
    );
    println!(
        "Original combined startup passed: modules={}, TLS instances={}, TLS bindings={}, HLE bindings={}, relocations={}, services={}, instructions={}, value={}",
        startup.module_count,
        startup.tls_instances,
        startup.tls_bindings,
        startup.provider_bindings,
        startup.applied_relocations,
        startup.service_attempts,
        startup.retired,
        startup.final_value
    );
    println!(
        "Original interpreted fixtures only; complete firmware startup, native service mediation and commercial games remain unsupported."
    );
    Ok(())
}
