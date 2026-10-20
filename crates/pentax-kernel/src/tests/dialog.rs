// SPDX-License-Identifier: GPL-2.0-only

use super::*;

struct NoGuest;
impl GuestAccess for NoGuest {
    fn read(&self, _: u64, _: &mut [u8]) -> Result<(), GuestError> {
        panic!("zero-argument dialog read");
    }
    fn apply(&mut self, _: MemoryEffect, _: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        panic!("zero-argument dialog write");
    }
}

#[test]
fn common_dialog_base_is_process_owned_idempotent_and_admitted_without_guest_access() {
    let mut owner = kernel(Default::default());
    let mut unrelated = kernel(Default::default());
    let garbage = CallArgs([u64::MAX; 6]);
    assert!(!owner.common_dialog_status().initialized);
    assert_eq!(
        owner.invoke_library(Service::CommonDialogIsUsed, garbage, &mut NoGuest),
        Ok(0)
    );
    for _ in 0..2 {
        assert_eq!(
            owner.invoke_library(Service::CommonDialogInitialize, garbage, &mut NoGuest),
            Ok(0)
        );
        assert_eq!(
            owner.common_dialog_status(),
            CommonDialogStatus {
                initialized: true,
                in_use: false
            }
        );
    }
    assert!(!unrelated.common_dialog_status().initialized);
    assert_eq!(
        unrelated.invoke_library(Service::CommonDialogIsUsed, garbage, &mut NoGuest),
        Ok(0)
    );
    let mut limited = kernel(Limits {
        max_attempts: 0,
        ..Default::default()
    });
    assert!(matches!(
        limited.invoke_library(Service::CommonDialogInitialize, garbage, &mut NoGuest),
        Err(Failure::Limit(_))
    ));
    assert!(!limited.common_dialog_status().initialized);
    assert_eq!(owner.usage().bytes_read + owner.usage().bytes_written, 0);
    for module in ["CommonDialog", "libSceCommonDialog"] {
        for library in ["CommonDialog", "libSceCommonDialog"] {
            assert_eq!(
                resolve(module, (1, 1), library, 1, "uoUpLGNkygk"),
                Some(Service::CommonDialogInitialize)
            );
            assert_eq!(resolve(module, (1, 1), library, 2, "uoUpLGNkygk"), None);
        }
    }
}
