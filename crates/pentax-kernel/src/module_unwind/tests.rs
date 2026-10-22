// SPDX-License-Identifier: GPL-2.0-only
use super::*;
use crate::tests::{ARG, TestGuest, kernel};
use crate::{GuestError, MemoryEffect, Service};

struct Guest {
    bytes: TestGuest,
    queries: usize,
    present: bool,
}
impl GuestAccess for Guest {
    fn read(&self, address: u64, output: &mut [u8]) -> Result<(), GuestError> {
        self.bytes.read(address, output)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.bytes.apply(effect, writes)
    }
    fn module_unwind_info(&mut self, address: u64) -> Result<Option<ModuleUnwindInfo>, Failure> {
        self.queries += 1;
        if !self.present || address != 0x1234 {
            return Ok(None);
        }
        let mut name = [0; 256];
        name[..11].copy_from_slice(b"original.so");
        Ok(Some(ModuleUnwindInfo {
            name,
            eh_frame_header: 0x1800,
            eh_frame: 0x1400,
            eh_frame_size: 0x400,
            image_address: 0x1000,
            image_size: 0x1000,
        }))
    }
}
fn guest() -> Guest {
    Guest {
        bytes: TestGuest::new(),
        queries: 0,
        present: true,
    }
}
fn invoke(
    owner: &mut crate::Kernel,
    guest: &mut Guest,
    address: u64,
    flags: u64,
    output: u64,
) -> Result<u64, Failure> {
    owner.invoke_library(
        Service::ModuleInfoForUnwind,
        CallArgs([address, flags, output, 0, 0, 0]),
        guest,
    )
}

#[test]
fn unwind_abi_encodes_actual_304_bytes_and_only_exact_source_scopes() {
    let mut owner = kernel(Default::default());
    let mut guest = guest();
    guest.bytes.put(0, &u64::MAX.to_le_bytes());
    guest.bytes.put(304, &[0xa5; 8]);
    assert_eq!(invoke(&mut owner, &mut guest, 0x1234, u64::MAX, ARG), Ok(0));
    assert_eq!(guest.bytes.u64(0), 304);
    assert_eq!(guest.bytes.get(ARG + 8, 12), b"original.so\0");
    for (index, expected) in [0x1800, 0x1400, 0x400, 0x1000, 0x1000]
        .into_iter()
        .enumerate()
    {
        assert_eq!(guest.bytes.u64(264 + index as u64 * 8), expected);
    }
    assert_eq!(guest.bytes.get(ARG + 304, 8), [0xa5; 8]);
    let entries: Vec<_> = Service::registrations()
        .iter()
        .filter(|r| r.service == Service::ModuleInfoForUnwind)
        .collect();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries
            .iter()
            .map(|r| (r.module, r.library, r.nid))
            .collect::<Vec<_>>(),
        [
            ("libkernel", "libkernel", "RpQJJVKTiFM"),
            ("Sysmodule", "Sysmodule", "4fU5yvOkVG4"),
            ("libSceSysmodule", "libSceSysmodule", "4fU5yvOkVG4"),
        ]
    );
}

#[test]
fn unwind_source_error_order_and_int32_errno_do_not_fabricate_a_host_boundary() {
    let mut owner = kernel(Default::default());
    let mut guest = guest();
    assert_eq!(invoke(&mut owner, &mut guest, 1, 3, 0), Ok(0x8002_0016));
    assert_eq!(invoke(&mut owner, &mut guest, 1, 0, 0), Ok(0x8002_000e));
    guest.bytes.put(0, &[0xa5; 304]);
    assert_eq!(invoke(&mut owner, &mut guest, 1, 3, ARG), Ok(0x8002_0016));
    assert_eq!(guest.bytes.get(ARG, 304), [0; 304]);
    guest.bytes.put(0, &303u64.to_le_bytes());
    assert_eq!(invoke(&mut owner, &mut guest, 1, 0, ARG), Ok(0x8002_0016));
    assert_eq!(guest.queries, 0);
    guest.bytes.put(0, &[0xa5; 304]);
    guest.bytes.put(0, &304u64.to_le_bytes());
    let before = guest.bytes.get(ARG, 304).to_vec();
    assert_eq!(invoke(&mut owner, &mut guest, 1, 0, ARG), Ok(0x8002_0003));
    assert_eq!(guest.bytes.get(ARG, 304), before);
    assert_eq!(guest.queries, 1);
}

#[test]
fn unwind_refuses_output_permission_overflow_and_budget_without_partial_store() {
    let mut owner = kernel(Default::default());
    let mut guest = guest();
    guest.bytes.put(0, &[0xa5; 304]);
    guest.bytes.put(0, &304u64.to_le_bytes());
    guest.bytes.refuse = true;
    let before = guest.bytes.get(ARG, 304).to_vec();
    assert_eq!(
        invoke(&mut owner, &mut guest, 0x1234, 0, ARG),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.bytes.get(ARG, 304), before);
    assert_eq!(
        invoke(&mut owner, &mut guest, 0x1234, 3, u64::MAX - 10),
        Err(Failure::Guest(GuestError::AddressOverflow))
    );
    let limits = crate::Limits {
        max_scratch_bytes: 615,
        max_name_bytes: 32,
        max_io_bytes: 64,
        ..Default::default()
    };
    let mut owner = kernel(limits);
    guest.queries = 0;
    assert!(matches!(
        invoke(&mut owner, &mut guest, 0x1234, 0, ARG),
        Err(Failure::Limit(_))
    ));
    assert_eq!(guest.queries, 0);
}
