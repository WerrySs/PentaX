// SPDX-License-Identifier: GPL-2.0-only
//! Original public-metadata syscall tokens and CPU Frames through the actual
//! scheduler replacement path. No native entry or OS display is involved.
use super::*;
use pentax_codegen::{BlockLimits, TranslationPolicy, plan_materialized_block};
use pentax_core::process::hle::{
    NativeEntryLayout, NativeImageLimits, NativeServiceAction, NativeServiceRequest,
    with_native_image_session,
};
use pentax_linker::{QualifiedNid, SymbolBinding, SymbolType, inspect_sce};
use pentax_loader::open_executable;
fn image() -> Vec<u8> {
    let mut bytes = vec![0; 4096];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    for (offset, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 1)] {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }
    bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
    bytes[24..32].copy_from_slice(&0x100_u64.to_le_bytes());
    bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
    bytes[64..68].copy_from_slice(&1_u32.to_le_bytes());
    bytes[68..72].copy_from_slice(&5_u32.to_le_bytes());
    for (offset, value) in [(96, 4096_u64), (104, 16384), (112, 16384)] {
        bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }
    // An original inert entry keeps the ELF complete; this fixture only
    // produces exit-only syscall plans for independently attested providers.
    let code = [0x31, 0xc0, 0xc3];
    bytes[0x100..0x100 + code.len()].copy_from_slice(&code);
    bytes
}

fn marker(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    nid: &[u8; 11],
    scope: &[u8],
) -> (usize, u64, u64) {
    let mut found = None;
    for (index, input) in proof
        .materialized_source()
        .source_plan()
        .modules()
        .iter()
        .enumerate()
    {
        let view = open_executable(input.source_bytes()).unwrap();
        let inspection = inspect_sce(&view, &Default::default(), &Default::default()).unwrap();
        let Some(metadata) = inspection.metadata() else {
            continue;
        };
        // A plain ELF source may have symbols without SCE scope descriptors.
        // It cannot supply this qualified export; retain the exact typed
        // module/library/version checks on every candidate below.
        let Some(descriptors) = metadata.descriptors().decoded() else {
            continue;
        };
        for symbol in metadata.link().symbols() {
            if symbol.is_undefined()
                || symbol.binding() != SymbolBinding::GLOBAL
                || symbol.symbol_type() != SymbolType::FUNC
            {
                continue;
            }
            let Some(name) = symbol.name.as_ref() else {
                continue;
            };
            let Ok(qualified) = QualifiedNid::parse(name.as_bytes()) else {
                continue;
            };
            if qualified.nid().as_bytes() != nid {
                continue;
            }
            let module = descriptors
                .module_info()
                .iter()
                .find(|item| item.id() == qualified.module())
                .unwrap();
            let library = descriptors
                .export_libraries()
                .iter()
                .find(|item| item.id() == qualified.library())
                .unwrap();
            if module.name().as_bytes() != scope
                || library.name().as_bytes() != scope
                || (
                    module.version_major(),
                    module.version_minor(),
                    library.version(),
                ) != (1, 1, 1)
            {
                continue;
            }
            assert!(found.is_none());
            let address = input.load_bias().checked_add(symbol.value).unwrap();
            let mut bytes = [0; 8];
            owner
                .pause()
                .unwrap()
                .fetch_source(address, &mut bytes)
                .unwrap();
            assert_eq!(bytes[0], 0xb8);
            assert_eq!(&bytes[5..], &[0x0f, 0x05, 0xc3]);
            found = Some((
                index,
                address,
                u64::from(u32::from_le_bytes(bytes[1..5].try_into().unwrap())),
            ));
        }
    }
    found.unwrap()
}

fn stopped<'view, 'plan, 'image>(
    owner: &mut PreparedNativeKernelStartup<'plan, 'image>,
    proof: &'view PreparedNativeSource<'plan, 'image>,
    request: NativeServiceRequest,
    layout: NativeEntryLayout,
    module: usize,
) -> (NativeBlockStepper<'view, 'plan, 'image>, PreSyscallToken) {
    let stack = NativeStackFrame {
        stack: layout.stack,
        guards: None,
        return_slot: request.stack_pointer.unwrap(),
        return_marker: layout.return_marker,
    };
    owner
        .pause()
        .unwrap()
        .write(stack.return_slot, &stack.return_marker.to_le_bytes())
        .unwrap();
    let mut state = NativeCpuState::new(request.syscall_pc, stack.return_slot);
    // This original fixture has only its initial thread and never switches it.
    assert_eq!(owner.tls().instances().len(), 1);
    if owner.tls().thread_control_policy()
        != pentax_core::process::tls::ThreadControlBlockPolicy::None
    {
        state.fs_base = Some(owner.tls().instances()[0].thread_pointer);
    }
    state.xmm.fill(0xa5);
    state.ymm_upper.fill(0x79);
    state.mxcsr = 0x5f85;
    state.rflags = 0xed7;
    state.gpr[0] = request.service_id;
    for (register, value) in [7, 6, 2, 1, 8, 9].into_iter().zip(request.args) {
        state.gpr[register] = value;
    }
    let mut engine = NativeBlockStepper::new(
        proof.materialized_source(),
        &mut owner.pause().unwrap(),
        state,
        stack,
        &Default::default(),
    )
    .unwrap();
    let block = plan_materialized_block(
        proof.materialized_source(),
        module,
        request.syscall_pc,
        &[15, 5],
        engine.placement_for(request.syscall_pc),
        &TranslationPolicy::x64_sse2(),
        &BlockLimits::default(),
    )
    .unwrap();
    let NativeBlockStop::PreSyscall(token) = engine
        .step_syscall_exit(&mut owner.pause().unwrap(), &block)
        .unwrap()
    else {
        panic!("expected syscall")
    };
    (engine, token)
}

fn request(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    nid: &[u8; 11],
    scope: &[u8],
    args: [u64; 6],
    stack: u64,
) -> (usize, NativeServiceRequest) {
    let (module, at, id) = marker(owner, proof, nid, scope);
    (
        module,
        NativeServiceRequest {
            owner_id: owner.owner_id(),
            syscall_pc: at + 5,
            next_pc: at + 7,
            service_id: id,
            args,
            vector_args: None,
            stack_pointer: Some(stack),
        },
    )
}
fn call(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    nid: &[u8; 11],
    scope: &[u8],
    args: [u64; 6],
    stack: u64,
) -> u64 {
    let (_, request) = request(owner, proof, nid, scope, args, stack);
    owner.service_request(request).unwrap()
}
fn write(owner: &mut PreparedNativeKernelStartup<'_, '_>, at: u64, bytes: &[u8]) {
    owner.pause().unwrap().write(at, bytes).unwrap();
}
fn word(owner: &mut PreparedNativeKernelStartup<'_, '_>, at: u64) -> u64 {
    let mut bytes = [0; 8];
    owner.pause().unwrap().read(at, &mut bytes).unwrap();
    u64::from_le_bytes(bytes)
}
fn value(owner: &mut PreparedNativeKernelStartup<'_, '_>, at: u64) -> u32 {
    let mut bytes = [0; 4];
    owner.pause().unwrap().read(at, &mut bytes).unwrap();
    u32::from_le_bytes(bytes)
}
#[test]
fn replacement_wait_keeps_actual_scheduler_token_cpu_phase_and_saved_frame_storage_until_one_return()
 {
    let primary = image();
    with_native_image_session(
        &[&primary],
        &NativeImageLimits::default(),
        |owner, layout| {
            let proof = owner
                .prepare_native_source(&BlockLimits::default())
                .unwrap();
            let scratch = layout.entry.stack.address + 0x2000;
            let stack = layout.entry.return_slot - 64;
            write(owner, stack + 8, &[0; 16]);
            assert_eq!(
                call(
                    owner,
                    &proof,
                    b"IWIBBdTHit4",
                    b"libkernel",
                    [scratch, 0x40000, 0x33, 0, 0, 0],
                    stack
                ),
                0
            );
            let base = word(owner, scratch);
            let target = (base + 65535) & !65535;
            assert_eq!(
                call(
                    owner,
                    &proof,
                    b"Up36PTk687E",
                    b"VideoOut",
                    [255, 0, 0, 0, 0, 0],
                    stack
                ),
                1
            );
            let mut attribute = [0; 80];
            attribute[12..16].copy_from_slice(&2u32.to_le_bytes());
            attribute[16..20].copy_from_slice(&2u32.to_le_bytes());
            attribute[32..40].copy_from_slice(&0x8000_0000_2200_0000u64.to_le_bytes());
            let mut buffers = [0; 64];
            buffers[..8].copy_from_slice(&target.to_le_bytes());
            buffers[32..40].copy_from_slice(&(target + 65536).to_le_bytes());
            write(owner, scratch + 0x100, &attribute);
            write(owner, scratch + 0x200, &buffers);
            assert_eq!(
                call(
                    owner,
                    &proof,
                    b"rKBUtgRrtbk",
                    b"VideoOut",
                    [1, 0, 0, scratch + 0x200, 2, scratch + 0x100],
                    stack
                ),
                0
            );
            for buffer in [0, 1] {
                assert_eq!(
                    call(
                        owner,
                        &proof,
                        b"U46NwOiJpys",
                        b"VideoOut",
                        [1, buffer, 1, 0, 0, 0],
                        stack
                    ),
                    0
                );
            }
            let mut words = Vec::new();
            for (offset, value, selected) in [(0, 11, Some(0)), (4, 22, Some(1)), (8, 33, None)] {
                let at = base + 0x3000 + offset;
                words.extend([0xc003_3700, 0x100, at as u32, (at >> 32) as u32, value]);
                if let Some(buffer) = selected {
                    words.extend([0xc005_1018, 1, buffer, 0, 0, 0, 0]);
                }
            }
            write(
                owner,
                base + 0x1000,
                &words
                    .iter()
                    .flat_map(|w| w.to_le_bytes())
                    .collect::<Vec<_>>(),
            );
            write(owner, base + 0x100, &[0; 8]);
            write(owner, base + 0x200, &(base + 0x1000).to_le_bytes());
            write(owner, base + 0x208, &(words.len() as u32).to_le_bytes());
            let (module, request) = request(
                owner,
                &proof,
                b"b4fpgH5ZXxQ",
                b"Graphics5Driver",
                [base + 0x100, base + 0x200, 0, 0, 0, 0],
                stack,
            );
            let (mut engine, token) = stopped(owner, &proof, request, layout.entry, module);
            let before = *engine.state();
            let NativeServiceAction::WaitThread(first) =
                owner.service_action(&proof, request).unwrap()
            else {
                panic!("real CPU selected wait missing")
            };
            let mut scheduling = Scheduling::new(None, 1).unwrap();
            scheduling.threads[0].frames.try_reserve_exact(4).unwrap();
            let capacity = scheduling.threads[0].frames.capacity();
            scheduling.check_service_wait_slot(&engine, token).unwrap();
            scheduling.wait(owner, first, token).unwrap();
            assert!(scheduling.active_blocked());
            assert!(scheduling.check_service_wait_slot(&engine, token).is_err());
            let frame = owner.graphics_video_take_frame(1).unwrap().unwrap();
            assert!(
                !owner
                    .thread_wait_ready(&scheduling.threads[0].waiting.as_ref().unwrap().0)
                    .unwrap()
            );
            owner.graphics_video_acknowledge_frame(&frame).unwrap();
            assert!(!scheduling.resume(owner, &proof, &mut engine).unwrap());
            assert_eq!(scheduling.threads[0].waiting.as_ref().unwrap().1, token);
            assert_eq!(*engine.state(), before);
            assert_eq!(scheduling.active, 0);
            assert!(matches!(
                scheduling.threads[0].phase,
                NativeSessionPhase::MainEntry
            ));
            assert_eq!(scheduling.threads[0].frames.capacity(), capacity);
            assert!(scheduling.threads[0].frames.is_empty());
            assert!(scheduling.force_switch);
            assert_eq!(
                (
                    value(owner, base + 0x3000),
                    value(owner, base + 0x3004),
                    value(owner, base + 0x3008)
                ),
                (11, 22, 0)
            );
            let frame = owner.graphics_video_take_frame(2).unwrap().unwrap();
            owner.graphics_video_acknowledge_frame(&frame).unwrap();
            assert!(scheduling.resume(owner, &proof, &mut engine).unwrap());
            assert!(scheduling.threads[0].waiting.is_none());
            assert!(!scheduling.active_blocked());
            assert_eq!(value(owner, base + 0x3008), 33);
            let mut expected = before;
            expected.gpr[0] = 0;
            expected.gpr[1] = token.next_pc();
            expected.gpr[11] = before.rflags;
            expected.guest_pc = token.next_pc();
            assert_eq!(*engine.state(), expected);
            assert!(engine.validate_pending_syscall(token).is_err());
            assert_eq!(engine.usage().native_entries, 0);
            Ok::<_, ()>(())
        },
    )
    .unwrap();
}
