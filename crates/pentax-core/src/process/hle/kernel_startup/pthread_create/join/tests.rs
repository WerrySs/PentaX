// SPDX-License-Identifier: GPL-2.0-only
//! Original data-only state/backing checks. Native fixtures separately prove RET.
use super::*;
use pentax_kernel::{GuestAccess, GuestError, GuestWrite, MemoryEffect, ProcessIdentity};
const BASE: u64 = 0x10000;
struct Guest {
    bytes: [u8; 4096],
    refuse: bool,
}
impl GuestAccess for Guest {
    fn preflight_pthread(&self) -> Result<(), Failure> {
        Ok(())
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
fn ticket(action: JoinAction) -> JoinTicket {
    match action {
        JoinAction::Wait(t) => t,
        _ => panic!("expected join wait"),
    }
}
fn error(action: JoinAction) -> u64 {
    match action {
        JoinAction::Returned(code) => code,
        _ => panic!("expected source error"),
    }
}
#[test]
fn join_record_readiness_failed_output_and_reap_preserve_single_atomic_result() {
    let (mut kernel, mut runtime, mut guest) = setup();
    let waiter = BASE + 16;
    let args = CallArgs([BASE + 0x200, BASE + 0x300, 0, 0, 0, 0]);
    assert_eq!(kernel.admit_pthread_join(args, &guest), Ok(None));
    let join = ticket(runtime.begin_join(&mut kernel, args, waiter).unwrap());
    assert!(!runtime.join_ready(&join).unwrap());
    assert_eq!(
        runtime.complete_join(&mut kernel, &join, waiter, &mut guest),
        Ok(None)
    );
    // Only the production post-cleanup path sets this retained field; the
    // controlled unit transition tests publication, not an actual guest RET.
    runtime.records[0].returned = Some(0xfedc_ba98_7654_3210);
    assert!(runtime.join_ready(&join).unwrap());
    assert!(
        runtime
            .complete_join(&mut kernel, &join, waiter + 8, &mut guest)
            .is_err()
    );
    guest.refuse = true;
    assert!(
        runtime
            .complete_join(&mut kernel, &join, waiter, &mut guest)
            .is_err()
    );
    assert_eq!(guest.bytes[0x300..0x308], [0xa5; 8]);
    assert!(!runtime.records[0].joined);
    assert!(runtime.records[0].join_claim.is_some());
    guest.refuse = false;
    assert_eq!(
        runtime.complete_join(&mut kernel, &join, waiter, &mut guest),
        Ok(Some(0))
    );
    assert_eq!(
        guest.bytes[0x300..0x308],
        0xfedc_ba98_7654_3210_u64.to_le_bytes()
    );
    assert!(runtime.records[0].joined);
    assert!(runtime.records[0].join_claim.is_none());
    assert!(
        runtime
            .complete_join(&mut kernel, &join, waiter, &mut guest)
            .is_err()
    );
    assert_eq!(
        error(runtime.begin_join(&mut kernel, args, waiter).unwrap()),
        0x8002_0003
    );
}
#[test]
fn join_detached_unknown_self_and_competing_claims_keep_outputs_unchanged() {
    let (mut kernel, mut runtime, guest) = setup();
    let args = CallArgs([BASE + 0x200, BASE + 0x300, 0, 0, 0, 0]);
    runtime.records[0].attributes.detached = true;
    assert_eq!(
        error(runtime.begin_join(&mut kernel, args, BASE + 16).unwrap()),
        0x8002_0016
    );
    runtime.records[0].attributes.detached = false;
    assert_eq!(
        error(
            runtime
                .begin_join(
                    &mut kernel,
                    CallArgs([99, BASE + 0x300, 0, 0, 0, 0]),
                    BASE + 16
                )
                .unwrap()
        ),
        0x8002_0003
    );
    assert_eq!(
        error(runtime.begin_join(&mut kernel, args, BASE + 0x100).unwrap()),
        0x8002_000b
    );
    let join = ticket(runtime.begin_join(&mut kernel, args, BASE + 16).unwrap());
    kernel.activate_thread_context(BASE + 0x140).unwrap();
    assert_eq!(
        error(runtime.begin_join(&mut kernel, args, BASE + 0x140).unwrap()),
        0x8002_0016
    );
    assert!(!runtime.join_ready(&join).unwrap());
    assert_eq!(guest.bytes[0x300..0x308], [0xa5; 8]);
    assert_eq!(
        kernel.admit_pthread_join(CallArgs([0; 6]), &guest),
        Ok(Some(0x8002_0016))
    );
}
#[test]
fn join_cycle_is_rejected_before_claiming_or_parking_the_second_waiter() {
    let (mut kernel, mut runtime, guest) = setup();
    kernel.activate_thread_context(BASE + 0x100).unwrap();
    let first = ticket(
        runtime
            .begin_join(
                &mut kernel,
                CallArgs([BASE + 0x240, 0, 0, 0, 0, 0]),
                BASE + 0x100,
            )
            .unwrap(),
    );
    kernel.activate_thread_context(BASE + 0x140).unwrap();
    assert_eq!(
        error(
            runtime
                .begin_join(
                    &mut kernel,
                    CallArgs([BASE + 0x200, 0, 0, 0, 0, 0]),
                    BASE + 0x140
                )
                .unwrap()
        ),
        0x8002_000b
    );
    assert!(runtime.records[0].join_claim.is_none());
    assert_eq!(runtime.records[0].waiting_for, Some(1));
    assert!(runtime.records[1].waiting_for.is_none());
    assert!(!runtime.join_ready(&first).unwrap());
    assert_eq!(guest.bytes[0x300..0x308], [0xa5; 8]);
}
