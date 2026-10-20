// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use Service::*;
fn call(
    owner: &mut Kernel,
    guest: &mut TestGuest,
    service: Service,
    values: &[u64],
) -> Result<u64, Failure> {
    let mut args = [0; 6];
    args[..values.len()].copy_from_slice(values);
    owner.invoke_library(service, CallArgs(args), guest)
}
fn commit(owner: &mut Kernel, guest: &mut TestGuest, mode: u32) -> Result<u64, Failure> {
    guest.put(0, &(-7_i32).to_le_bytes());
    guest.put(4, &mode.to_le_bytes());
    call(owner, guest, SaveCommit, &[ARG])
}

#[test]
fn save_initialization_and_transaction_receipts_do_not_claim_storage_or_reset_identifiers() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert!(!owner.save_data_initialized());
    assert_eq!(
        call(&mut owner, &mut guest, SaveCreateTransaction, &[u64::MAX]),
        Ok(1)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveDeleteTransaction, &[u64::MAX]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveInitialize, &[u64::MAX; 6]),
        Ok(0)
    );
    assert!(owner.save_data_initialized());
    assert_eq!(
        call(&mut owner, &mut guest, SaveCreateTransaction, &[0]),
        Ok(2)
    );
    assert_eq!(call(&mut owner, &mut guest, SaveTerminate, &[]), Ok(0));
    assert!(!owner.save_data_initialized());
    assert_eq!(call(&mut owner, &mut guest, SaveInitialize, &[0]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, SaveCreateTransaction, &[1]),
        Ok(3)
    );
    assert_eq!(
        call(
            &mut kernel(Default::default()),
            &mut guest,
            SaveCreateTransaction,
            &[1]
        ),
        Ok(1)
    );
    assert_eq!(owner.usage().mapped_bytes, 0);
    assert_eq!(owner.usage().handles_live, 0);
    assert_eq!(guest.effects, 0);
}

#[test]
fn save_commit_events_have_exact_source_layout_retry_and_lifecycle_behavior() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[u64::MAX, 0]),
        Ok(0x809f_0000)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[u64::MAX, u64::MAX]),
        Ok(0x809f_0008)
    );
    assert_eq!(commit(&mut owner, &mut guest, 2), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 128]),
        Ok(0x809f_0008)
    );
    assert_eq!(commit(&mut owner, &mut guest, 3), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, SaveInitialize, &[u64::MAX]),
        Ok(0)
    );
    guest.put(120, &[0xa5; 120]);
    guest.refuse = true;
    assert!(matches!(
        call(&mut owner, &mut guest, SaveGetEvent, &[u64::MAX, ARG + 128]),
        Err(Failure::Guest(_))
    ));
    guest.refuse = false;
    assert_eq!(guest.get(ARG + 120, 120), [0xa5; 120]);
    assert!(matches!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + PAGE - 100]),
        Err(Failure::Guest(_))
    ));
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[u64::MAX, ARG + 128]),
        Ok(0)
    );
    assert_eq!(guest.get(ARG + 128, 4), 4_u32.to_le_bytes());
    assert_eq!(guest.get(ARG + 132, 100), [0; 100]);
    assert_eq!(guest.get(ARG + 120, 8), [0xa5; 8]);
    assert_eq!(guest.get(ARG + 232, 8), [0xa5; 8]);
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 128]),
        Ok(0x809f_0008)
    );
    assert_eq!(commit(&mut owner, &mut guest, 1), Ok(0));
    assert_eq!(call(&mut owner, &mut guest, SaveTerminate, &[]), Ok(0));
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 128]),
        Ok(0x809f_0008)
    );
}

#[test]
fn save_queue_retains_twenty_events_and_refuses_invalid_parameters_before_publication() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    assert_eq!(
        call(&mut owner, &mut guest, SavePrepare, &[0, ARG]),
        Ok(0x809f_0000)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SavePrepare, &[ARG, 0]),
        Ok(0x809f_0000)
    );
    assert_eq!(
        call(&mut owner, &mut guest, SaveCommit, &[0]),
        Ok(0x809f_0000)
    );
    assert!(matches!(
        call(&mut owner, &mut guest, SavePrepare, &[u64::MAX, ARG]),
        Err(Failure::Guest(_))
    ));
    assert!(matches!(
        call(&mut owner, &mut guest, SaveCommit, &[ARG + PAGE - 4]),
        Err(Failure::Guest(_))
    ));
    guest.put(64, b"/savedata0\0\0\0\0\0\0");
    assert_eq!(
        call(&mut owner, &mut guest, SavePrepare, &[ARG + 64, ARG]),
        Ok(0)
    );
    for _ in 0..23 {
        assert_eq!(commit(&mut owner, &mut guest, 1), Ok(0));
    }
    for _ in 0..20 {
        assert_eq!(
            call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 128]),
            Ok(0)
        );
    }
    assert_eq!(
        call(&mut owner, &mut guest, SaveGetEvent, &[0, ARG + 128]),
        Ok(0x809f_0008)
    );
    let mut limited = kernel(Limits {
        max_scratch_bytes: 64,
        max_io_bytes: 64,
        max_name_bytes: 64,
        ..Default::default()
    });
    assert_eq!(
        commit(&mut limited, &mut guest, 1),
        Err(Failure::Limit("scratch"))
    );
    assert_eq!(
        call(&mut limited, &mut guest, SaveGetEvent, &[0, ARG + 128]),
        Ok(0x809f_0008)
    );
    let mut exhausted = kernel(Limits {
        max_work: 0,
        ..Default::default()
    });
    assert!(matches!(
        call(&mut exhausted, &mut guest, SaveInitialize, &[0]),
        Err(Failure::Limit(_))
    ));
    assert!(!exhausted.save_data_initialized());
}
