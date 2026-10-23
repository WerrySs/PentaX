// SPDX-License-Identifier: GPL-2.0-only
//! Original decoded shaders and synthetic queue registers; no guest data reads.
use super::*;
use pentax_shader::ShaderStage;
use pentax_shader_compiler::{
    CommonDescription, SubgroupProfile, Unsupported, UnsupportedReason, WaveSize,
};

fn description(users: u8) -> ComputeDescription {
    let mut common = CommonDescription::new(WaveSize::Wave64);
    common.user_sgpr_count = users;
    common.subgroup = Some(SubgroupProfile { size: 64 });
    ComputeDescription {
        common,
        workgroup_size: [1, 1, 1],
        thread_ids: 0,
        group_id: [false; 3],
        workgroup_register: users,
        tg_size: false,
        float_mode: 0xc0,
        shared_memory_bytes: 0,
    }
}
fn program(words: &[u32]) -> ShaderProgram {
    ShaderProgram::decode(words, ShaderStage::Compute, &CompilerLimits::default()).unwrap()
}
fn two_buffers() -> Vec<u32> {
    vec![
        0x7e04_0280, // V_MOV_B32 v2, 0.
        0xe030_1000,
        (128 << 24) | (2 << 16) | (1 << 8) | 2, // Load s[8:11].
        0xe070_1000,
        (128 << 24) | (1 << 16) | (1 << 8) | 2, // Store s[4:7].
        0xbf81_0000,
    ]
}
fn registers() -> ComputeState {
    let mut registers = ComputeState::empty();
    for (first, address) in [(0x244, 0x30000), (0x248, 0x20000)] {
        for (offset, value) in [address, 0, 64, 0].into_iter().enumerate() {
            registers.write(first + offset as u32, value).unwrap();
        }
    }
    registers
}

#[test]
fn zero_live_resources_finish_without_a_fabricated_descriptor_or_buffer() {
    let program = program(&[0xbf81_0000]);
    let discovery = discover(
        &program,
        description(0),
        &ComputeState::empty(),
        &Limits::default(),
    )
    .unwrap();
    assert!(discovery.buffers().is_empty());
    assert_eq!(discovery.usage().buffer_bytes, 0);
    let specialized = discovery.finish().unwrap();
    assert!(specialized.buffers().is_empty());
    assert!(specialized.shader().manifest().buffers().is_empty());
    assert_eq!(specialized.shader().manifest().buffer_binding(), None);
}

#[test]
fn multiple_nonzero_sgpr_origins_preserve_first_use_and_actual_access() {
    let program = program(&two_buffers());
    let registers = registers();
    // s0..s3 remain unknown: this API must never demand a made-up V# there.
    assert!(registers.get(0x240).is_err());
    let discovery = discover(&program, description(12), &registers, &Limits::default()).unwrap();
    let spans = discovery.buffers();
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].requirement().user_sgpr, 8);
    assert!(spans[0].requirement().read && !spans[0].requirement().written);
    assert_eq!(spans[0].address(), 0x20000);
    assert_eq!(spans[1].requirement().user_sgpr, 4);
    assert!(!spans[1].requirement().read && spans[1].requirement().written);
    assert_eq!(spans[1].address(), 0x30000);
    assert!(spans[0].requirement().first_use_pc < spans[1].requirement().first_use_pc);
    assert_eq!(discovery.usage().buffer_bytes, 128);
    let specialized = discovery.finish().unwrap();
    let manifest = specialized.shader().manifest();
    assert_eq!(manifest.buffers().len(), 2);
    let read = manifest
        .buffers()
        .iter()
        .find(|buffer| buffer.user_sgpr == 8)
        .unwrap();
    let write = manifest
        .buffers()
        .iter()
        .find(|buffer| buffer.user_sgpr == 4)
        .unwrap();
    assert!(read.read && !read.written);
    assert!(!write.read && write.written);
    assert_ne!(read.profile_index, write.profile_index);
    assert!(manifest.buffer_binding().is_some());
    assert_eq!(manifest.push_data().unwrap().memory_offset_count, 2);
    assert_eq!(manifest.undefined_register_reads(), 0);
}

#[test]
fn only_required_descriptors_are_checked_and_aggregate_limits_are_not_per_buffer_credits() {
    let program = program(&two_buffers());
    let mut registers = registers();
    for register in 0x240..=0x243 {
        registers.write(register, u32::MAX).unwrap();
    }
    assert!(discover(&program, description(12), &registers, &Limits::default()).is_ok());
    let limits = Limits {
        max_buffer_bytes: 127,
        ..Default::default()
    };
    assert!(matches!(
        discover(&program, description(12), &registers, &limits),
        Err(Error::Limit("compute aggregate buffer bytes"))
    ));
    registers.write(0x24b, 1).unwrap();
    assert!(matches!(
        discover(&program, description(12), &registers, &Limits::default()),
        Err(Error::Unsupported(
            "compute raw unswizzled buffer descriptor"
        ))
    ));
    let empty = ComputeState::empty();
    assert!(matches!(
        discover(&program, description(12), &empty, &Limits::default()),
        Err(Error::Register(RegisterError::MissingRegister(0x248)))
    ));
}

#[test]
fn transformed_descriptor_words_stay_explicit_compiler_refusals() {
    let mut words = vec![0xbe88_0380]; // S_MOV_B32 s8, 0 changes the descriptor origin.
    words.extend(two_buffers());
    let program = program(&words);
    assert!(matches!(
        discover(&program, description(12), &registers(), &Limits::default()),
        Err(Error::Compiler(CompileError::Unsupported(Unsupported {
            reason: UnsupportedReason::DynamicDescriptor,
            ..
        })))
    ));
}

struct ExecutionSource {
    code: Vec<u32>,
    preflights: std::cell::Cell<usize>,
    snapshots: std::cell::RefCell<Vec<(u64, bool, bool)>>,
    refuse: bool,
}
impl super::super::Memory for ExecutionSource {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), super::super::MemoryError> {
        let offset = address
            .checked_sub(0x10000)
            .ok_or(super::super::MemoryError::Unavailable)?;
        if output.len() != 4 || !offset.is_multiple_of(4) {
            return Err(super::super::MemoryError::Unavailable);
        }
        let word = self
            .code
            .get(offset as usize / 4)
            .ok_or(super::super::MemoryError::Unavailable)?;
        output.copy_from_slice(&word.to_le_bytes());
        Ok(())
    }
    fn preflight_buffers(&self, buffers: &[BufferSpan], _: u64) -> Result<u64, RegisterError> {
        self.preflights.set(self.preflights.get() + 1);
        assert!(buffers.len() <= 4);
        if self.refuse {
            Err(RegisterError::Memory(
                super::super::MemoryError::WriteDenied,
            ))
        } else {
            Ok(0)
        }
    }
    fn snapshot_buffer(
        &self,
        address: u64,
        output: &mut [u8],
        read: bool,
        written: bool,
    ) -> Result<(), super::super::MemoryError> {
        self.snapshots.borrow_mut().push((address, read, written));
        output.fill(0xa5);
        Ok(())
    }
}
fn execution_source(code: Vec<u32>, refuse: bool) -> ExecutionSource {
    ExecutionSource {
        code,
        refuse,
        preflights: std::cell::Cell::new(0),
        snapshots: std::cell::RefCell::new(Vec::new()),
    }
}
fn execution_dispatch(users: u32, descriptors: &[(u32, u64)]) -> super::super::Dispatch {
    let mut state = ComputeState::for_new_queue();
    for (first, values) in [
        (0x207, vec![1, 1, 1]),
        (0x20c, vec![0x100, 0]),
        (0x212, vec![0x2c0000, users << 1]),
        (0x228, vec![0]),
    ] {
        for (offset, value) in values.into_iter().enumerate() {
            state.write(first + offset as u32, value).unwrap();
        }
    }
    for &(user, address) in descriptors {
        for (offset, value) in [address as u32, (address >> 32) as u32, 64, 0]
            .into_iter()
            .enumerate()
        {
            state.write(0x240 + user + offset as u32, value).unwrap();
        }
    }
    super::super::Dispatch {
        state,
        groups: [1; 3],
        mode: 0x41,
        indirect: None,
    }
}
fn four_stores() -> Vec<u32> {
    let mut words = vec![0x7e04_0280]; // v2=0 byte address.
    for (index, user) in [12u32, 0, 8, 4].into_iter().enumerate() {
        words.extend([
            0x7e02_0280 | (index as u32 + 1),
            0xe070_1000,
            (128 << 24) | ((user / 4) << 16) | (1 << 8) | 2,
        ]);
    }
    words.push(0xbf81_0000);
    words
}

#[test]
fn execution_specializes_zero_and_four_live_resources_with_one_compiler_ledger() {
    let limits = Limits::default();
    let source = execution_source(vec![0xbf81_0000], false);
    let (product, _) =
        super::super::prepare_dispatch(&execution_dispatch(0, &[]), &source, &limits)
            .unwrap()
            .into_parts();
    let prepared = product.unwrap();
    assert!(prepared.buffers().is_empty());
    assert_eq!(prepared.input(), None);
    assert_eq!(prepared.buffer_address(), None);
    assert_eq!(prepared.shader().manifest().buffer_binding(), None);
    assert!(source.snapshots.borrow().is_empty());

    let mut allocation = execution_dispatch(0, &[]);
    allocation.state.write(0x213, 1 << 15).unwrap();
    let (product, _) = super::super::prepare_dispatch(&allocation, &source, &limits)
        .unwrap()
        .into_parts();
    assert_eq!(
        product.unwrap().shader().manifest().shared_memory_bytes(),
        512
    );
    allocation.state.write(0x213, 129 << 15).unwrap();
    assert!(matches!(
        super::super::prepare_dispatch(&allocation, &source, &limits),
        Err(RegisterError::Unsupported("compute LDS allocation"))
    ));

    let source = execution_source(four_stores(), false);
    let dispatch = execution_dispatch(
        16,
        &[(0, 0x20000), (4, 0x30000), (8, 0x40000), (12, 0x50000)],
    );
    let (product, _) = super::super::prepare_dispatch(&dispatch, &source, &limits)
        .unwrap()
        .into_parts();
    let prepared = product.unwrap();
    assert_eq!(
        prepared
            .buffers()
            .iter()
            .map(|buffer| buffer.address())
            .collect::<Vec<_>>(),
        vec![0x50000, 0x20000, 0x40000, 0x30000]
    );
    assert!(
        prepared
            .buffers()
            .iter()
            .all(|buffer| !buffer.read() && buffer.written())
    );
    assert_eq!(prepared.input(), None);
    assert_eq!(prepared.buffer_address(), None);
    assert_eq!(source.snapshots.borrow().len(), 4);
    assert_eq!(source.preflights.get(), 1);
    // This original shader has ten instructions and one direct invocation.
    // The unchanged bounded walker is separate from the three compiler phases;
    // four descriptors cost sixteen words and four snapshots cost 256 bytes.
    let shader_walk_work = 64 * (limits.max_shader_words as u64).pow(2);
    let execution_work = prepared.usage().invocations * 10;
    assert_eq!(prepared.usage().invocations, 1);
    assert_eq!(
        prepared.usage().work,
        3 * limits.compiler_phase_work + shader_walk_work + execution_work + 16 + 256
    );
    assert!(prepared.usage().host_bytes < limits.compiler_phase_bytes * 4);
}

#[test]
fn complete_span_admission_and_shared_extent_limit_precede_every_payload_snapshot() {
    let dispatch = execution_dispatch(
        16,
        &[(0, 0x20000), (4, 0x30000), (8, 0x40000), (12, 0x50000)],
    );
    let refused = execution_source(four_stores(), true);
    assert!(matches!(
        super::super::prepare_dispatch(&dispatch, &refused, &Limits::default()),
        Err(RegisterError::Memory(
            super::super::MemoryError::WriteDenied
        ))
    ));
    assert_eq!(refused.preflights.get(), 1);
    assert!(refused.snapshots.borrow().is_empty());
    let source = execution_source(four_stores(), false);
    let limits = Limits {
        max_buffer_bytes: 255,
        ..Default::default()
    };
    assert!(matches!(
        super::super::prepare_dispatch(&dispatch, &source, &limits),
        Err(RegisterError::Limit("compute aggregate buffer bytes"))
    ));
    assert_eq!(source.preflights.get(), 0);
    assert!(source.snapshots.borrow().is_empty());
}

#[test]
fn live_readonly_scalar_descriptor_outside_sgpr0_requires_no_write_access() {
    // S_BUFFER_LOAD s16,s[8:11],NULL; compare drives a forward branch.
    // Branch-condition liveness retains the actual scalar read without a store.
    let words = vec![
        0xf420_0404,
        125 << 25,
        0xbf8c_007f,
        0xbf06_8010,
        0xbf84_0001,
        0xbf80_0000,
        0xbf81_0000,
    ];
    let source = execution_source(words, false);
    let dispatch = execution_dispatch(12, &[(8, 0x20000)]);
    assert!(dispatch.state.get(0x240).is_err());
    let (product, _) = super::super::prepare_dispatch(&dispatch, &source, &Limits::default())
        .unwrap()
        .into_parts();
    let prepared = product.unwrap();
    assert_eq!(prepared.buffers().len(), 1);
    let resource = &prepared.buffers()[0];
    assert!(resource.read());
    assert!(!resource.written());
    assert!(!prepared.writes_buffer());
    assert!(prepared.shader().manifest().buffers()[0].scalar);
    assert_eq!(*source.snapshots.borrow(), vec![(0x20000, true, false)]);
}
