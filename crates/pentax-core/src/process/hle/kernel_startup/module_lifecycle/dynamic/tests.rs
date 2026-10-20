// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::process::hle::{NativeImageLimits, NativeServiceRequest, with_native_image_session};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const PATH: &str = "/app0/original-dynamic.sprx";
mod delta;
struct Input {
    path: PathBuf,
    bytes: Vec<u8>,
}
impl Input {
    fn new() -> Self {
        let mut bytes = vec![0; 4096];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        for (at, value) in [(16, 3_u16), (18, 62), (52, 64), (54, 56), (56, 2)] {
            bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in [(20, 1_u32), (64, 1), (68, 5), (120, 2), (124, 4)] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (at, value) in [
            (24, 0x100_u64),
            (32, 64),
            (96, 4096),
            (104, 16384),
            (112, 16384),
            (128, 0x200),
            (136, 0x200),
            (152, 32),
            (160, 32),
            (168, 8),
            (0x200, 12),
            (0x208, 0x100),
        ] {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        }
        bytes[0x100..0x106].copy_from_slice(&[0xb8, 7, 0, 0, 0, 0xc3]);
        Self::from_bytes(bytes)
    }
    fn from_bytes(bytes: Vec<u8>) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "pentax-dynamic-cap-{}-{}.elf",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&bytes).unwrap();
        Self { path, bytes }
    }
    fn open(&self) -> File {
        File::open(&self.path).unwrap()
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        assert_eq!(std::fs::read(&self.path).unwrap(), self.bytes);
        std::fs::remove_file(&self.path).unwrap();
    }
}
fn session(test: impl FnOnce(&mut PreparedNativeKernelStartup<'_, '_>, u64)) {
    let primary = crate::process::hle::unavailable::tests::original_input(0);
    let limits = NativeImageLimits {
        dynamic_window_bytes: 4 * GRANULE,
        ..Default::default()
    };
    with_native_image_session(&[&primary], &limits, |owner, layout| {
        test(owner, layout.entry.stack.address + 0x100);
        Ok::<_, ()>(())
    })
    .unwrap();
}
fn load_request(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    scratch: u64,
) -> NativeServiceRequest {
    let marker = *owner
        .plan
        .factory
        .markers()
        .iter()
        .find(|marker| marker.registration.service == pentax_kernel::Service::LoadStartModule)
        .unwrap();
    owner
        .pause()
        .unwrap()
        .write(scratch, format!("{PATH}\0").as_bytes())
        .unwrap();
    let pc = owner.plan.entry(marker) + 5;
    NativeServiceRequest {
        stack_pointer: None,
        vector_args: None,
        owner_id: owner.owner_id(),
        syscall_pc: pc,
        next_pc: pc + 2,
        service_id: marker.service_id.into(),
        args: [scratch, 17, 0xdead, 0, 0, scratch + 128],
    }
}
fn read(
    owner: &mut PreparedNativeKernelStartup<'_, '_>,
    proof: &PreparedNativeSource<'_, '_>,
    request: NativeServiceRequest,
) -> DynamicModuleInput {
    let NativeServiceAction::Load(pending) = owner.service_action(proof, request).unwrap() else {
        panic!("new file was treated as already loaded")
    };
    assert_eq!(owner.kernel_usage().attempts, 1);
    owner.read_dynamic_module(pending).unwrap()
}

#[test]
fn dynamic_publication_binds_exact_input_owner_active_proof_and_real_handle_then_refuses_replay() {
    let file = Input::new();
    session(|owner, scratch| {
        owner
            .register_dynamic_module_file(PATH, file.open())
            .unwrap();
        let block = BlockLimits::default();
        let proof = owner.prepare_native_source(&block).unwrap();
        let request = load_request(owner, scratch);
        let sentinel = 0x1122_3344_5566_7788_u64.to_le_bytes();
        owner
            .pause()
            .unwrap()
            .write(scratch + 128, &sentinel)
            .unwrap();
        let input = read(owner, &proof, request);
        let bias = owner.dynamic_load_bias(&input).unwrap();
        let appended = pentax_linker::append_symbolic_module(
            proof.materialized_source().source_plan(),
            pentax_linker::ModuleInput::new(open_executable(input.bytes()).unwrap(), bias),
            &owner.dynamic_extension_limits(),
        )
        .unwrap();
        // Cross-owner inputs must not poison another owner's equally numbered
        // in-flight file. This test admits data only; it executes no initializer.
        session(|other, other_scratch| {
            other
                .register_dynamic_module_file(PATH, file.open())
                .unwrap();
            let other_proof = other.prepare_native_source(&block).unwrap();
            let other_request = load_request(other, other_scratch);
            let own_input = read(other, &other_proof, other_request);
            assert!(
                other
                    .publish_dynamic_module(&other_proof, &input, &appended, &block)
                    .is_err()
            );
            assert!(other.validate_pending(&own_input.pending).is_ok());
        });
        let start = owner
            .publish_dynamic_module(&proof, &input, &appended, &block)
            .unwrap();
        assert_eq!(owner.module_load_count(start.module), Some(1));
        assert_eq!(owner.loaded_modules.resolve_handle(2), Some(start.module));
        assert!(start.module > 1); // Generated sources never receive this handle.
        assert!(owner.prepare_native_source(&block).is_err());
        assert!(owner.service_request(request).is_err());
        assert!(owner.service_action(&proof, request).is_err());
        assert!(
            owner
                .publish_dynamic_module(&proof, &input, &appended, &block)
                .is_err()
        );
        assert!(matches!(
            owner.loaded_modules.dynamic.files[0].state,
            FileState::Mapped { call: None, .. }
        ));
        let mut result = [0; 8];
        owner
            .pause()
            .unwrap()
            .read(scratch + 128, &mut result)
            .unwrap();
        assert_eq!(result, sentinel); // Mapping is not initializer completion.
        let active = owner
            .prepare_epoch_native_source(&appended, &block)
            .unwrap();
        let NativeServiceAction::Returned(handle) = owner.service_action(&active, request).unwrap()
        else {
            panic!("in-flight mapped module was initialized twice")
        };
        assert_eq!(handle, 2);
        assert_eq!(owner.module_load_count(start.module), Some(2));
        owner
            .pause()
            .unwrap()
            .read(scratch + 128, &mut result)
            .unwrap();
        assert_eq!(u64::from_le_bytes(result), 0x1122_3344_0000_0000);
    });
}

#[test]
fn dynamic_ticket_read_budget_failure_is_terminal_and_never_publishes_a_mapping_or_result() {
    let file = Input::new();
    session(|owner, scratch| {
        owner
            .register_dynamic_module_file(PATH, file.open())
            .unwrap();
        let proof = owner
            .prepare_native_source(&BlockLimits::default())
            .unwrap();
        let request = load_request(owner, scratch);
        let NativeServiceAction::Load(pending) = owner.service_action(&proof, request).unwrap()
        else {
            panic!("missing file ticket")
        };
        let source = owner.memory().active_source_identity();
        let count = owner.memory().module_count();
        owner.loaded_modules.dynamic.work = MAX_WORK;
        assert!(matches!(
            owner.read_dynamic_module(pending),
            Err(KernelStartupError::Budget(_))
        ));
        assert!(matches!(
            owner.loaded_modules.dynamic.files[0].state,
            FileState::Failed
        ));
        assert_eq!(owner.memory().module_count(), count);
        assert_eq!(owner.memory().active_source_identity(), source);
        assert_eq!(owner.loaded_modules.resolve_handle(2), None);
    });
}
