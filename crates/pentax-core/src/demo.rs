// SPDX-License-Identifier: GPL-2.0-or-later
//! Original synthetic guest fixture. No firmware or commercial game content.
use crate::{
    cpu::{CpuError, ReferenceCpu, StopReason},
    process::{self, LoadOptions, ProcessError},
};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoResult {
    pub result_rax: u64,
    pub instructions: u64,
    pub mapped_bytes: u64,
    pub resident_bytes: u64,
}

#[derive(Debug)]
pub enum DemoError {
    Parse(pentax_loader::LoadError),
    Load(ProcessError),
    Cpu(CpuError),
    UnexpectedResult,
}
impl fmt::Display for DemoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "{e}"),
            Self::Load(e) => write!(f, "{e}"),
            Self::Cpu(e) => write!(f, "{e}"),
            Self::UnexpectedResult => write!(f, "reference guest produced an unexpected result"),
        }
    }
}
impl std::error::Error for DemoError {}

pub fn run() -> Result<DemoResult, DemoError> {
    let bytes = elf();
    let image = pentax_loader::parse(&bytes).map_err(DemoError::Parse)?;
    let mut process = process::load(&image, &LoadOptions::default()).map_err(DemoError::Load)?;
    let mut cpu = ReferenceCpu::at(process.entry);
    let stop = cpu.run(&mut process.memory, 100).map_err(DemoError::Cpu)?;
    if stop != StopReason::Breakpoint || cpu.registers[0] != 42 || cpu.retired != 9 {
        return Err(DemoError::UnexpectedResult);
    }
    Ok(DemoResult {
        result_rax: cpu.registers[0],
        instructions: cpu.retired,
        mapped_bytes: process.memory.mapped_bytes(),
        resident_bytes: process.memory.resident_bytes(),
    })
}

/// ELF64 ET_EXEC containing `mov eax,3; sub eax,1; jnz -7; mov eax,42; int3`.
pub fn elf() -> Vec<u8> {
    let code = [
        0xb8, 3, 0, 0, 0, 0x2d, 1, 0, 0, 0, 0x75, 0xf9, 0xb8, 42, 0, 0, 0, 0xcc,
    ];
    let mut bytes = vec![0_u8; 0x4000 + code.len()];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2_u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x400000_u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[52..54].copy_from_slice(&64_u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56_u16.to_le_bytes());
    bytes[56..58].copy_from_slice(&1_u16.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5_u32.to_le_bytes());
    bytes[72..80].copy_from_slice(&0x4000_u64.to_le_bytes());
    bytes[80..88].copy_from_slice(&0x400000_u64.to_le_bytes());
    bytes[96..104].copy_from_slice(&(code.len() as u64).to_le_bytes());
    bytes[104..112].copy_from_slice(&0x4000_u64.to_le_bytes());
    bytes[112..120].copy_from_slice(&0x4000_u64.to_le_bytes());
    bytes[0x4000..].copy_from_slice(&code);
    bytes
}

#[cfg(test)]
mod tests {
    #[test]
    fn guest_runs_through_parser_mapper_and_reference_cpu() {
        let result = super::run().unwrap();
        assert_eq!(result.result_rax, 42);
        assert_eq!(result.instructions, 9);
        assert_eq!(result.mapped_bytes, 16384);
    }
}
