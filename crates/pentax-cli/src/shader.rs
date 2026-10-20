// SPDX-License-Identifier: GPL-2.0-or-later

//! Closed original shader syntax diagnostic, without file or guest-code input.

use pentax_shader::{Family, Limits, Opcode, OperandKind, ShaderStage, decode};
use std::error::Error;

pub(super) fn run() -> Result<(), Box<dyn Error>> {
    // Original scalar add with one shared literal, backward branch, ENDPGM,
    // then NOP. The caller supplies a linear span rather than a program boundary.
    let mut words = [
        0x8003_ffff,
        0xffff_ffff,
        0xbf82_fffe,
        0xbf81_0000,
        0xbf80_0000,
    ];
    let span = decode(&words, ShaderStage::Compute, &Limits::default())?;
    words.fill(0);
    if span.instruction_count() != 4
        || span.stage() != ShaderStage::Compute
        || span.source_words() == words
    {
        return Err("Original shader syntax ownership/count check failed".into());
    }
    let add = span.instruction(0).ok_or("Missing scalar add")?;
    if add.family() != Family::Sop2
        || add.opcode() != Opcode::SAddU32
        || add.destination().register() != 3
        || add.raw_words().len() != 2
        || add.sources().len() != 2
        || !add.sources().iter().all(|source| {
            source.kind() == OperandKind::LiteralConstant && source.value() == u32::MAX
        })
    {
        return Err("Original shader syntax literal check failed".into());
    }
    let branch = span.instruction(1).ok_or("Missing scalar branch")?;
    if branch.opcode() != Opcode::SBranch
        || !branch.branch().is_some_and(|target| {
            target.displacement_words() == -2 && target.target_byte_offset() == 4
        })
        || span.instruction(2).map(|inst| inst.opcode()) != Some(Opcode::SEndpgm)
        || span.instruction(3).map(|inst| inst.opcode()) != Some(Opcode::SNop)
    {
        return Err("Original shader syntax branch/trailing-word check failed".into());
    }
    println!(
        "Original shader syntax passed: words={}, instructions={}, work={}",
        span.source_words().len(),
        span.instruction_count(),
        span.work_units()
    );
    println!(
        "Syntax metadata only; branch targets are not validated, shaders are not compiled and no frame is rendered."
    );
    Ok(())
}
