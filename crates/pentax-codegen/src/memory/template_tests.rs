// SPDX-License-Identifier: GPL-2.0-only
//! Original private-template boundary cases, without a live memory capability.
use super::*;
use iced_x86::{Decoder, DecoderOptions, MemorySize};

fn record(code: &[u8]) -> InstructionRecord {
    let instruction = Decoder::with_ip(64, code, 0x1000, DecoderOptions::NONE).decode();
    assert!(!instruction.is_invalid());
    let template = MemoryAccessAnalyzer::default().prepare(&instruction);
    InstructionRecord {
        guest_pc: instruction.ip(),
        source_offset: 0,
        source_len: instruction.len(),
        cache_offset: 0,
        emitted_len: instruction.len(),
        instruction,
        rip_target: None,
        memory_template: Some(template),
    }
}

#[test]
fn excess_descriptor_count_keeps_earlier_address_and_shape_failures_in_order() {
    let mut record = record(&[0x48, 0x8b, 0x00]); // MOV RAX,[RAX].
    let template = record.memory_template.as_mut().unwrap();
    assert_eq!(template.memory_count, 1);
    template.memory[1] = template.memory[0];
    // Exercise a future decoder reporting extra operands: only the first two
    // descriptors are retained, but the real count must not be truncated.
    template.memory_count = 3;
    let mut analyzer = MemoryAccessAnalyzer::default();
    let mut registers = [0; 16];
    registers[0] = u64::MAX;
    assert_eq!(
        analyzer.analyze(&record, &registers),
        Err(MemoryAccessError::InvalidRange)
    );
    registers[0] = 0x8000;
    assert_eq!(
        analyzer.analyze(&record, &registers),
        Err(MemoryAccessError::TooManyAccesses)
    );

    record.memory_template.as_mut().unwrap().memory[1] = UsedMemory::new2(
        Register::DS,
        Register::RAX,
        Register::None,
        1,
        0,
        MemorySize::UInt64,
        OpAccess::Read,
        CodeSize::Code32,
        0,
    );
    assert_eq!(
        analyzer.analyze(&record, &registers),
        Err(MemoryAccessError::UnsupportedAddress)
    );
    registers[0] = u64::MAX;
    assert_eq!(
        analyzer.analyze(&record, &registers),
        Err(MemoryAccessError::InvalidRange)
    );
}

#[test]
fn template_capture_does_not_admit_an_unsupported_segment_or_missing_access() {
    // Building metadata is never a permission or support decision. Preserve
    // runtime refusal rather than rejecting or silently normalizing the plan.
    let mut record = record(&[0x64, 0x48, 0x8b, 0x00]); // MOV RAX,FS:[RAX].
    let mut analyzer = MemoryAccessAnalyzer::default();
    assert_eq!(
        analyzer.analyze(&record, &[0; 16]),
        Err(MemoryAccessError::UnsupportedAddress)
    );
    record.memory_template.as_mut().unwrap().memory_count = 0;
    assert_eq!(
        analyzer.analyze(&record, &[0; 16]),
        Err(MemoryAccessError::UnsupportedAddress)
    );
    record.emitted_len = 0;
    assert_eq!(
        analyzer.analyze(&record, &[0; 16]),
        Err(MemoryAccessError::NotEmitted)
    );
}
