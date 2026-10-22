// SPDX-License-Identifier: GPL-2.0-only
//! Original detach/Join interaction checks; native fixtures separately prove RET.
use super::super::join::JoinAction;
use super::*;
use pentax_kernel::{GuestAccess, GuestError, GuestWrite, MemoryEffect, ProcessIdentity};
const BASE: u64 = 0x10000;
struct Guest {
    bytes: [u8; 4096],
    refuse: bool,
    bound: bool,
}
impl GuestAccess for Guest {
    fn preflight_pthread(&self) -> Result<(), Failure> {
        if self.bound {
            Ok(())
        } else {
            Err(Failure::Unsupported("unbound test pthread"))
        }
    }
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        let at = address.checked_sub(BASE).ok_or(GuestError::ReadDenied)? as usize;
        let bytes = self
            .bytes
            .get(at..at + output.len())
            .ok_or(GuestError::ReadDenied)?;
        output.copy_from_slice(bytes);
        Ok(())
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        if self.refuse {
            return Err(GuestError::WriteDenied);
        }
        if effect != MemoryEffect::None {
            return Err(GuestError::UnsupportedMapping);
        }
        for (index, write) in writes.iter().enumerate() {
            let end = write
                .address
                .checked_add(write.bytes.len() as u64)
                .ok_or(GuestError::WriteDenied)?;
            let at = write
                .address
                .checked_sub(BASE)
                .ok_or(GuestError::WriteDenied)? as usize;
            self.bytes
                .get(at..at + write.bytes.len())
                .ok_or(GuestError::WriteDenied)?;
            for other in &writes[..index] {
                let other_end = other
                    .address
                    .checked_add(other.bytes.len() as u64)
                    .ok_or(GuestError::WriteDenied)?;
                if write.address < other_end && other.address < end {
                    return Err(GuestError::WriteDenied);
                }
            }
        }
        for write in writes {
            let at = (write.address - BASE) as usize;
            self.bytes[at..at + write.bytes.len()].copy_from_slice(write.bytes);
        }
        Ok(())
    }
}
fn setup() -> (Kernel, ThreadRuntime, Guest) {
    let mut kernel = Kernel::new(
        Default::default(),
        ProcessIdentity {
            process_id: 1,
            initial_thread_id: 7,
        },
        vec![],
    )
    .unwrap();
    let mut guest = Guest {
        bytes: [0xa5; 4096],
        refuse: false,
        bound: true,
    };
    kernel.activate_thread_context(BASE + 16).unwrap();
    let mut runtime = ThreadRuntime::new(4).unwrap();
    for i in 0..2 {
        let thread_pointer = BASE + 0x100 + i * 0x40;
        let pointer = BASE + 0x200 + i * 0x40;
        kernel
            .register_thread_context(thread_pointer, pointer, 700, 0x7f, &mut guest)
            .unwrap();
        runtime.records.push(Record {
            tls_index: i as usize + 1,
            thread_pointer,
            joined: false,
            join_claim: None,
            waiting_for: None,
            layout: ThreadEntryLayout {
                entry: 0x2000,
                argument: 0,
                stack: NativeRange {
                    address: 0x3000 + i * 0x4000,
                    size: 0x4000,
                },
                guards: None,
                return_slot: 0x6ff8 + i * 0x4000,
                return_marker: 0x7000,
                pthread_pointer: pointer,
            },
            attributes: Default::default(),
            published: true,
            returned: None,
            execution: None,
            exit: None,
        });
    }
    (kernel, runtime, guest)
}

#[test]
fn detach_active_and_completed_handles_prevents_join_without_output_or_retirement() {
    let (mut kernel, mut runtime, guest) = setup();
    let before = guest.bytes;
    for index in 0..2 {
        runtime.records[index].returned = (index == 1).then_some(77);
        let pointer = runtime.records[index].layout.pthread_pointer;
        let args = CallArgs([pointer, BASE + 0x300, u64::MAX, 0, 0, 0]);
        for service in [
            Service::PthreadDetach,
            Service::PosixPthreadDetach,
            Service::PosixPthreadDetach,
        ] {
            assert_eq!(kernel.admit_pthread_detach(service, args, &guest), Ok(None));
            assert_eq!(runtime.detach(&mut kernel, service, args, &guest), Ok(0));
        }
        assert!(runtime.records[index].attributes.detached);
        assert!(!runtime.records[index].joined);
        assert_eq!(runtime.records[index].returned, (index == 1).then_some(77));
        assert!(matches!(
            runtime.begin_join(&mut kernel, args, BASE + 16),
            Ok(JoinAction::Returned(0x8002_0016))
        ));
    }
    assert_eq!(runtime.records.len(), 2);
    assert_eq!(guest.bytes, before);
}

#[test]
fn detach_cannot_invalidate_an_accepted_join_or_consume_its_true_value() {
    let (mut kernel, mut runtime, mut guest) = setup();
    let args = CallArgs([BASE + 0x200, BASE + 0x300, 0, 0, 0, 0]);
    let JoinAction::Wait(ticket) = runtime.begin_join(&mut kernel, args, BASE + 16).unwrap() else {
        panic!("expected accepted join");
    };
    kernel.activate_thread_context(BASE + 0x140).unwrap();
    for (service, code) in [
        (Service::PthreadDetach, 0x8002_0016),
        (Service::PosixPthreadDetach, 22),
    ] {
        assert_eq!(runtime.detach(&mut kernel, service, args, &guest), Ok(code));
        assert!(!runtime.records[0].attributes.detached);
        assert!(!runtime.join_ready(&ticket).unwrap());
    }
    runtime.records[0].returned = Some(0xfedc_ba98_7654_3210);
    kernel.activate_thread_context(BASE + 16).unwrap();
    guest.refuse = true;
    assert!(
        runtime
            .complete_join(&mut kernel, &ticket, BASE + 16, &mut guest)
            .is_err()
    );
    assert_eq!(
        runtime.detach(&mut kernel, Service::PosixPthreadDetach, args, &guest),
        Ok(22)
    );
    assert!(!runtime.records[0].joined);
    guest.refuse = false;
    assert_eq!(
        runtime.complete_join(&mut kernel, &ticket, BASE + 16, &mut guest),
        Ok(Some(0))
    );
    assert_eq!(
        guest.bytes[0x300..0x308],
        0xfedc_ba98_7654_3210_u64.to_le_bytes()
    );
    assert_eq!(
        runtime.detach(&mut kernel, Service::PthreadDetach, args, &guest),
        Ok(0x8002_0003)
    );
}

#[test]
fn detach_authenticates_main_without_created_record_and_refuses_unpublished_or_unbound() {
    let (mut kernel, mut runtime, mut guest) = setup();
    let main_pointer = BASE + 0x280;
    // A directly registered selected context models the actual initial thread:
    // it has a kernel self handle, but no created-thread runtime record.
    kernel
        .register_thread_context(BASE + 0x180, main_pointer, 700, 0x7f, &mut guest)
        .unwrap();
    kernel.activate_thread_context(BASE + 0x180).unwrap();
    let main = CallArgs([main_pointer, u64::MAX, 0, 0, 0, 0]);
    assert_eq!(
        runtime.detach(&mut kernel, Service::PosixPthreadDetach, main, &guest),
        Ok(0)
    );
    assert_eq!(
        runtime.detach(&mut kernel, Service::PthreadDetach, main, &guest),
        Ok(0)
    );
    assert_eq!(runtime.records.len(), 2);
    let args = CallArgs([BASE + 0x200, 0, 0, 0, 0, 0]);
    runtime.records[0].published = false;
    assert_eq!(
        runtime.detach(&mut kernel, Service::PosixPthreadDetach, args, &guest),
        Ok(3)
    );
    assert!(!runtime.records[0].attributes.detached);
    runtime.records[0].published = true;
    guest.bound = false;
    assert_eq!(
        kernel.admit_pthread_detach(Service::PosixPthreadDetach, CallArgs([0; 6]), &guest),
        Ok(Some(22))
    );
    assert!(
        kernel
            .admit_pthread_detach(Service::PosixPthreadDetach, args, &guest)
            .is_err()
    );
    assert!(
        runtime
            .detach(&mut kernel, Service::PosixPthreadDetach, args, &guest)
            .is_err()
    );
    assert!(!runtime.records[0].attributes.detached);
    guest.bound = true;
    assert_eq!(
        runtime.detach(
            &mut kernel,
            Service::PosixPthreadDetach,
            CallArgs([99, 0, 0, 0, 0, 0]),
            &guest
        ),
        Ok(3)
    );
}
