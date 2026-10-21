// SPDX-License-Identifier: GPL-2.0-only

use crate::policy::{atomic_memory, gpr, ordinary, potential_memory, prefixes};
use crate::source::{ADDRESS_LIMIT, SourceWindow, source_window};
use crate::*;
use iced_x86::{Decoder, DecoderError, DecoderOptions, Encoder, FlowControl, Instruction, OpKind};
use pentax_linker::SymbolicLinkPlan;

/// Decode only the selected immutable module's initialized executable bytes,
/// apply retained ordinary source writes, and emit one block plus owned UD2.
///
/// Every source instruction and emitted boundary has guest/cache provenance.
/// All control transfers are represented as exits, never encoded into native
/// control flow. An unsupported valid instruction also becomes a typed exit;
/// invalid/truncated input is an error, never skipped. An explicit partial-window
/// policy may return the complete prefix before a truncated trailing instruction.
/// At every bounded split the next PC is retried. No source/guest memory changes.
///
/// # Errors
/// Refuses unresolved plans, missing initialized executable source, unsafe cache
/// placements, invalid/truncated decoding, RIP-relative range errors, bounds and
/// per-plan buffer reservation failures. This is not permission to execute.
pub fn plan_block<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    module_index: usize,
    guest_pc: u64,
    placement: CachePlacement,
    policy: &TranslationPolicy,
    limits: &BlockLimits,
) -> Result<BlockPlan<'plan, 'image>, CodegenError> {
    let mut accounting = begin(limits)?;
    let window = source_window(source, module_index, guest_pc, placement, &mut accounting)?;
    plan_window(source, guest_pc, placement, policy, window, accounting)
}

pub(crate) fn begin(limits: &BlockLimits) -> Result<Accounting, CodegenError> {
    if limits.max_instructions == 0 {
        return Err(CodegenError::Budget {
            kind: Budget::Instructions,
            limit: 0,
        });
    }
    if limits.max_output_bytes < 2 {
        return Err(CodegenError::Budget {
            kind: Budget::OutputBytes,
            limit: limits.max_output_bytes,
        });
    }
    Ok(Accounting {
        work: 0,
        allocated: 0,
        limits: *limits,
    })
}

/// Shared pure emitter. Only private source-window producers can reach it;
/// the caller-declared path wraps its result in a different public type.
pub(crate) fn plan_window<'plan, 'image>(
    source: &'plan SymbolicLinkPlan<'image>,
    guest_pc: u64,
    placement: CachePlacement,
    policy: &TranslationPolicy,
    window: SourceWindow,
    mut accounting: Accounting,
) -> Result<BlockPlan<'plan, 'image>, CodegenError> {
    let limits = accounting.limits;
    let record_count = usize::try_from(limits.max_instructions.min(window.relocated.len() as u64))
        .map_err(|_| CodegenError::InvalidAddress)?;
    // Memory isolation commonly produces a single-instruction block. Reserve
    // a small batch, then grow within the same precharged aggregate budgets.
    let mut records = accounting.buffer(record_count.min(4))?;
    let output_limit = limits.max_output_bytes.min(placement.capacity);
    let maximum_output = (record_count as u64)
        .checked_mul(15)
        .and_then(|n| n.checked_add(2))
        .ok_or(CodegenError::InvalidAddress)?
        .min(output_limit);
    let maximum_output =
        usize::try_from(maximum_output).map_err(|_| CodegenError::InvalidAddress)?;
    let mut bytes = accounting.buffer(maximum_output.min(64))?;
    // Encoder owns a fallibly pre-reserved 15-byte scratch. No encode call can
    // grow it: x86 instructions contain at most 15 bytes. Global iced tables
    // and diagnostic error allocation are dependency internals, not recoverable
    // per-plan allocations; see the crate README before native integration.
    let scratch = accounting.buffer::<u8>(15)?;
    let mut encoder = Encoder::new(64);
    encoder.set_buffer(scratch);
    // AMD mode preserves 16-bit branch/return distinctions in long mode. Such
    // non-64-bit transfers are mediated rather than treated as native RET/CALL.
    let mut decoder = Decoder::with_ip(64, &window.relocated, guest_pc, DecoderOptions::AMD);
    let mut position = 0_usize;
    let mut decoded = 0_u64;
    let mut memory_analyzer = MemoryAccessAnalyzer::default();
    let mut written_registers = 0_u16;
    let mut emits_non_temporal_store = false;
    let mut emits_vector_memory_move = false;
    let exit;
    loop {
        if records.len() as u64 == limits.max_instructions || position == window.relocated.len() {
            exit = fallthrough(
                guest_pc + position as u64,
                placement.address + bytes.len() as u64,
            );
            break;
        }
        accounting.charge(32)?; // <=15 decoded bytes, <=5 operands, classification.
        let instruction = decoder.decode();
        decoded += 1;
        let pc = instruction.ip();
        if instruction.is_invalid() {
            if decoder.last_error() == DecoderError::NoMoreBytes
                && policy.splits_partial_windows()
                && !records.is_empty()
            {
                exit = fallthrough(pc, placement.address + bytes.len() as u64);
                break;
            }
            return Err(match decoder.last_error() {
                DecoderError::NoMoreBytes => CodegenError::TruncatedInstruction { guest_pc: pc },
                _ => CodegenError::InvalidInstruction { guest_pc: pc },
            });
        }
        let source_len = instruction.len();
        let next = pc
            .checked_add(source_len as u64)
            .filter(|end| *end <= ADDRESS_LIMIT)
            .ok_or(CodegenError::InvalidAddress)?;
        let cache_ip = placement
            .address
            .checked_add(bytes.len() as u64)
            .ok_or(CodegenError::InvalidAddress)?;
        let rip_target = instruction
            .is_ip_rel_memory_operand()
            .then(|| instruction.ip_rel_memory_address());
        let terminator = termination(&instruction, *policy);
        if let Some(kind) = terminator {
            let needed = records.len() + 1;
            accounting.grow(&mut records, needed, record_count)?;
            records.push(InstructionRecord {
                guest_pc: pc,
                source_offset: position,
                source_len,
                cache_offset: bytes.len(),
                emitted_len: 0,
                instruction,
                rip_target,
                memory_template: None,
            });
            exit = BlockExit {
                guest_pc: pc,
                next_guest_pc: next,
                cache_ip,
                kind,
                instruction: Some(instruction),
            };
            break;
        }
        // Atomics always need their actual PRE-instruction context, including
        // the manual runner's mandatory owned-range/alignment admission.
        let memory_template = if policy.batches_independent_memory() {
            accounting.charge(128)?;
            Some(memory_analyzer.prepare(&instruction))
        } else {
            None
        };
        let dependencies = memory_template
            .as_ref()
            .map_or(MemoryRegisterDependencies::default(), |template| {
                template.register_dependencies()
            });
        if dependencies.address & written_registers != 0 {
            exit = fallthrough(pc, cache_ip);
            break;
        }
        let isolated = atomic_memory(&instruction)
            || (policy.isolates_memory() && potential_memory(&instruction));
        if isolated && !records.is_empty() {
            // Decode/classification was charged above, but this instruction is
            // not consumed. Its address must use the next observed CPU state.
            exit = fallthrough(pc, cache_ip);
            break;
        }
        if instruction.memory_base() == Register::EIP && instruction.mnemonic() != Mnemonic::Lea {
            // Actual EIP-relative memory accesses still need a separate live
            // footprint rule. LEA only computes a wrapping 32-bit address.
            let needed = records.len() + 1;
            accounting.grow(&mut records, needed, record_count)?;
            records.push(InstructionRecord {
                guest_pc: pc,
                source_offset: position,
                source_len,
                cache_offset: bytes.len(),
                emitted_len: 0,
                instruction,
                rip_target,
                memory_template: None,
            });
            exit = BlockExit {
                guest_pc: pc,
                next_guest_pc: next,
                cache_ip,
                kind: ExitKind::Mediated {
                    reason: UnsupportedReason::AddressSize,
                },
                instruction: Some(instruction),
            };
            break;
        }
        let mut emission = instruction;
        if let Some(target) = rip_target {
            let delta = i128::from(target) - i128::from(cache_ip) - source_len as i128;
            // Iced retains EIP addressing and encodes its displacement modulo
            // 2^32 at the new IP. Applying a signed 64-bit range test here would
            // reject valid LEA address arithmetic from a high cache placement.
            let eip_lea = instruction.memory_base() == Register::EIP
                && instruction.mnemonic() == Mnemonic::Lea;
            if target >= ADDRESS_LIMIT || !eip_lea && i32::try_from(delta).is_err() {
                if instruction.mnemonic() == Mnemonic::Lea {
                    // An IP-relative LEA computes a constant, not a pointer
                    // dereference. A width-matched MOV preserves flags and all
                    // non-destination bits without a scratch register, even for
                    // high/noncanonical values or a distant cache placement.
                    emission = lea_constant(&instruction, target)?;
                } else {
                    return Err(CodegenError::RipRelativeOutOfRange {
                        guest_pc: pc,
                        cache_ip,
                        target,
                    });
                }
            }
        }
        accounting.charge(30)?; // <=15 encoded bytes plus output copy.
        let encoded_length = match encoder.encode(&emission, cache_ip) {
            Err(_) if emission.mnemonic() == Mnemonic::Lea && rip_target.is_some() => {
                // Normalizing ignored prefixes can shorten the instruction,
                // moving a boundary displacement out of range. Retry only this
                // address-only constant, charging the extra bounded encode.
                accounting.charge(15)?;
                let mut scratch = encoder.take_buffer();
                scratch.clear();
                encoder.set_buffer(scratch);
                let constant = lea_constant(&instruction, rip_target.unwrap())?;
                encoder.encode(&constant, cache_ip)
            }
            result => result,
        };
        let emitted = encoded_length.map_err(|_| {
            if let Some(target) = rip_target {
                CodegenError::RipRelativeOutOfRange {
                    guest_pc: pc,
                    cache_ip,
                    target,
                }
            } else {
                CodegenError::Encoding { guest_pc: pc }
            }
        })?;
        let mut encoded = encoder.take_buffer();
        if (bytes.len() as u64)
            .checked_add(emitted as u64)
            .and_then(|n| n.checked_add(2))
            .is_none_or(|n| n > output_limit)
        {
            // Nothing from this instruction was emitted. One complete source
            // instruction must fit to ensure a split can make progress.
            if records.is_empty() {
                return Err(CodegenError::Budget {
                    kind: Budget::OutputBytes,
                    limit: output_limit,
                });
            }
            exit = fallthrough(pc, cache_ip);
            break;
        }
        let needed = bytes.len() + emitted + 2;
        accounting.grow(&mut bytes, needed, maximum_output)?;
        let needed = records.len() + 1;
        accounting.grow(&mut records, needed, record_count)?;
        let memory_template = if let Some(template) = memory_template {
            template
        } else {
            // Other policies did not need dependency analysis for batching,
            // but execution must use the same bounded immutable information.
            accounting.charge(128)?;
            memory_analyzer.prepare(&instruction)
        };
        bytes.extend_from_slice(&encoded);
        emits_non_temporal_store |= crate::policy::non_temporal_store_width(&instruction).is_some();
        emits_vector_memory_move |= crate::policy::vector_memory_move(&instruction);
        encoded.clear();
        encoder.set_buffer(encoded);
        records.push(InstructionRecord {
            guest_pc: pc,
            source_offset: position,
            source_len,
            cache_offset: bytes.len() - emitted,
            emitted_len: emitted,
            instruction,
            rip_target,
            memory_template: Some(memory_template),
        });
        position += source_len;
        written_registers |= dependencies.written;
        if isolated {
            exit = fallthrough(next, placement.address + bytes.len() as u64);
            break;
        }
    }
    accounting.charge(2)?;
    bytes.extend_from_slice(&[0x0f, 0x0b]);
    let usage = BlockUsage {
        source_bytes: window.original.len() as u64,
        decoded_instructions: decoded,
        emitted_bytes: bytes.len() as u64,
        allocation_bytes: accounting.allocated,
        work_units: accounting.work,
    };
    Ok(BlockPlan {
        source,
        module: window.module,
        guest_pc,
        original: window.original,
        relocated: window.relocated,
        bytes,
        records,
        exit,
        placement,
        policy: *policy,
        emits_non_temporal_store,
        emits_vector_memory_move,
        usage,
    })
}

fn lea_constant(instruction: &Instruction, target: u64) -> Result<Instruction, CodegenError> {
    let destination = instruction.op0_register();
    match destination.size() {
        2 => Instruction::with2(Code::Mov_r16_imm16, destination, u32::from(target as u16)),
        4 => Instruction::with2(Code::Mov_r32_imm32, destination, target as u32),
        8 => Instruction::with2(Code::Mov_r64_imm64, destination, target),
        _ => {
            return Err(CodegenError::Encoding {
                guest_pc: instruction.ip(),
            });
        }
    }
    .map_err(|_| CodegenError::Encoding {
        guest_pc: instruction.ip(),
    })
}

fn fallthrough(pc: u64, cache_ip: u64) -> BlockExit {
    BlockExit {
        guest_pc: pc,
        next_guest_pc: pc,
        cache_ip,
        kind: ExitKind::Fallthrough { target: pc },
        instruction: None,
    }
}

fn termination(instruction: &Instruction, policy: TranslationPolicy) -> Option<ExitKind> {
    let mediated = |reason| Some(ExitKind::Mediated { reason });
    if let Err(reason) = prefixes(instruction) {
        return mediated(reason);
    }
    let next = instruction.next_ip();
    match instruction.code() {
        Code::Syscall => return Some(ExitKind::Syscall),
        Code::Retnq => return Some(ExitKind::Return { adjust: 0 }),
        Code::Retnq_imm16 => {
            return Some(ExitKind::Return {
                adjust: instruction.immediate16(),
            });
        }
        _ => {}
    }
    match instruction.flow_control() {
        FlowControl::Call
        | FlowControl::IndirectCall
        | FlowControl::UnconditionalBranch
        | FlowControl::IndirectBranch => {
            let target = match control_target(instruction) {
                Some(target) => target,
                None => return mediated(UnsupportedReason::FarOrNon64Transfer),
            };
            if matches!(
                instruction.flow_control(),
                FlowControl::Call | FlowControl::IndirectCall
            ) {
                Some(ExitKind::Call {
                    target,
                    return_pc: next,
                })
            } else {
                Some(ExitKind::Jump { target })
            }
        }
        FlowControl::ConditionalBranch => {
            let condition = instruction.condition_code();
            if condition == ConditionCode::None {
                return mediated(UnsupportedReason::CounterBranch);
            }
            if instruction.op0_kind() != OpKind::NearBranch64 {
                return mediated(UnsupportedReason::FarOrNon64Transfer);
            }
            Some(ExitKind::Conditional {
                condition,
                target: instruction.near_branch_target(),
                fallthrough: next,
            })
        }
        FlowControl::Interrupt => Some(ExitKind::Interrupt {
            mnemonic: instruction.mnemonic(),
        }),
        FlowControl::Exception => Some(ExitKind::GuestException {
            code: instruction.code(),
        }),
        FlowControl::Return => mediated(UnsupportedReason::FarOrNon64Transfer),
        FlowControl::XbeginXabortXend => mediated(UnsupportedReason::SystemState),
        FlowControl::Next => ordinary(instruction, policy)
            .err()
            .map(|reason| ExitKind::Mediated { reason }),
    }
}

fn control_target(instruction: &Instruction) -> Option<ControlTarget> {
    match instruction.op0_kind() {
        OpKind::NearBranch64 => Some(ControlTarget::Direct(instruction.near_branch_target())),
        OpKind::Register if instruction.op0_register().is_gpr64() => {
            Some(ControlTarget::Register(instruction.op0_register()))
        }
        OpKind::Memory if matches!(instruction.code(), Code::Call_rm64 | Code::Jmp_rm64) => {
            let base = instruction.memory_base();
            let index = instruction.memory_index();
            if !(base == Register::None || base == Register::RIP || base.is_gpr64())
                || !(index == Register::None || gpr(index) && index.is_gpr64())
            {
                return None;
            }
            Some(ControlTarget::Memory(MemoryOperand {
                base,
                index,
                scale: instruction.memory_index_scale(),
                displacement: instruction.memory_displacement64(),
                ip_relative: instruction
                    .is_ip_rel_memory_operand()
                    .then(|| instruction.ip_rel_memory_address()),
                segment: instruction.memory_segment(),
            }))
        }
        _ => None,
    }
}
