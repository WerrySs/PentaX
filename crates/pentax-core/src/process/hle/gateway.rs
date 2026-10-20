// SPDX-License-Identifier: GPL-2.0-or-later

//! Private interpreter loop dispatching only at attested generated sites.
//!
//! At a `StopReason::Syscall`, RIP and RCX must both equal one owned stub's
//! resume address and the complete 64-bit RAX must equal that stub's service
//! ID. How guest control reached the stub is not authenticated: ordinary
//! calls, tail jumps, repeated calls and a direct jump to the trap with the
//! exact ID are equally ordinary guest calls with no extra host privilege.
//! A matching stop validates every argument and budget independently.
//!
//! Only RAX changes on success; RSP, callee-saved registers and the modeled FS
//! base are untouched, and the guest's own RET then reads its stack. A failed
//! service leaves the post-trap CPU and guest bytes unchanged. A completed
//! service write is not undone if the following RET faults. RCX is clobbered
//! by the marker, R11/RFLAGS syscall state is not modeled, and no native
//! SYSCALL, host stack switch or exception handler is involved.

use super::HleError;
use super::catalog::KnownFunction;
use super::dispatch::{AbiArgs, ServiceBudget, invoke};
use super::session::{AttestedSite, PreparedHleSession};
use crate::cpu::{ReferenceCpu, StopReason};
use crate::memory::GuestMemory;

pub(super) struct Gateway {
    pub memory: GuestMemory,
    pub cpu: ReferenceCpu,
    pub budget: ServiceBudget,
    sites: [AttestedSite; KnownFunction::ALL.len()],
    instruction_limit: u64,
}

impl Gateway {
    pub(super) fn new(prepared: PreparedHleSession, instruction_limit: u64) -> Self {
        let budget =
            ServiceBudget::new(prepared.service_limits()).with_provider(prepared.provider_bias());
        let sites = prepared.sites;
        let entry = prepared.primary_entry();
        Self {
            memory: prepared.session.memory,
            cpu: ReferenceCpu::at(entry),
            budget,
            sites,
            instruction_limit,
        }
    }

    pub(super) const fn sites(&self) -> &[AttestedSite; KnownFunction::ALL.len()] {
        &self.sites
    }

    /// Step under one retired-instruction budget shared across every resume.
    /// Returns the RIP after the first breakpoint; callers decide its meaning.
    pub(super) fn run(&mut self) -> Result<u64, HleError> {
        run(
            &mut self.memory,
            &mut self.cpu,
            &mut self.budget,
            &self.sites,
            self.instruction_limit,
        )
    }

    #[cfg(test)]
    pub(super) fn service(&mut self) -> Result<(), HleError> {
        service(
            &mut self.memory,
            &mut self.cpu,
            &mut self.budget,
            &self.sites,
        )
    }
}

/// Shared only by closed original consumers; no public interpreter gateway.
pub(super) fn run(
    memory: &mut GuestMemory,
    cpu: &mut ReferenceCpu,
    budget: &mut ServiceBudget,
    sites: &[AttestedSite; KnownFunction::ALL.len()],
    instruction_limit: u64,
) -> Result<u64, HleError> {
    loop {
        if cpu.retired >= instruction_limit {
            return Err(HleError::InstructionLimit {
                limit: instruction_limit,
                retired: cpu.retired,
            });
        }
        match cpu.step(memory).map_err(HleError::Cpu)? {
            None => {}
            Some(StopReason::Syscall) => service(memory, cpu, budget, sites)?,
            Some(StopReason::Breakpoint) => return Ok(cpu.rip),
        }
    }
}

fn service(
    memory: &mut GuestMemory,
    cpu: &mut ReferenceCpu,
    budget: &mut ServiceBudget,
    sites: &[AttestedSite; KnownFunction::ALL.len()],
) -> Result<(), HleError> {
    // Counted before lookup, so unknown and mismatched traps are charged.
    let attempt = budget.begin_attempt().map_err(HleError::Service)?;
    let function = attested_function(sites, cpu.rip, cpu.registers[1], cpu.registers[0])?;
    let args = AbiArgs {
        rdi: cpu.registers[7],
        rsi: cpu.registers[6],
        rdx: cpu.registers[2],
    };
    let result = invoke(attempt, function, args, memory, budget).map_err(HleError::Service)?;
    cpu.registers[0] = result.rax;
    Ok(())
}

pub(super) fn attested_function(
    sites: &[AttestedSite; KnownFunction::ALL.len()],
    rip: u64,
    rcx: u64,
    rax: u64,
) -> Result<KnownFunction, HleError> {
    let site = sites
        .iter()
        .find(|site| site.resume == rip)
        .ok_or(HleError::SiteMismatch { rip })?;
    if rcx != site.resume {
        return Err(HleError::ContinuationMismatch { rip, rcx });
    }
    let expected = site.function.service_id();
    if rax != u64::from(expected) {
        return Err(HleError::ServiceIdMismatch {
            expected,
            actual: rax,
        });
    }
    Ok(site.function)
}
