// SPDX-License-Identifier: GPL-2.0-only

use super::*;
use crate::tests::{TestGuest, kernel};
use crate::{GuestError, Limits};

const ARG: u64 = 0x1_0000;
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
fn word(guest: &TestGuest, at: u64) -> u32 {
    u32::from_le_bytes(guest.get(at, 4).try_into().unwrap())
}

#[test]
fn network_errno_is_thread_owned_guest_editable_and_independent_from_posix_and_backend_receipts() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    owner.activate_thread_context(11).unwrap();
    let posix = call(&mut owner, &mut guest, Service::ErrnoLocation, &[]).unwrap();
    let net = call(&mut owner, &mut guest, Service::NetGetErrorAddr, &[]).unwrap();
    assert_ne!(posix, net);
    guest.put(posix - ARG, &55_u32.to_le_bytes());
    guest.put(net - ARG, &77_u32.to_le_bytes());
    assert_eq!(call(&mut owner, &mut guest, Service::NetInit, &[]), Ok(0));
    assert_eq!(guest.network_calls, 1); // The source ignores an actual false result.
    guest.network_backend_result = true;
    assert_eq!(call(&mut owner, &mut guest, Service::NetInit, &[]), Ok(0));
    assert_eq!(guest.network_calls, 2);
    assert_eq!(word(&guest, net), 77);
    assert_eq!(
        call(&mut owner, &mut guest, Service::NetPoolDestroy, &[u64::MAX]),
        Ok(u64::from(EBADF))
    );
    assert_eq!(word(&guest, net), 9);
    assert_eq!(word(&guest, posix), 55);
    owner.activate_thread_context(22).unwrap();
    let other = call(&mut owner, &mut guest, Service::NetGetErrorAddr, &[]).unwrap();
    assert_ne!(net, other);
    assert_eq!(word(&guest, other), 0);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetResolverCreate,
            &[0, u64::MAX, 0]
        ),
        Ok(u64::from(EINVAL))
    );
    assert_eq!(word(&guest, other), 22);
    owner.activate_thread_context(11).unwrap();
    assert_eq!(
        call(&mut owner, &mut guest, Service::NetGetErrorAddr, &[]),
        Ok(net)
    );
    assert_eq!(word(&guest, net), 9);
}

#[test]
fn network_startup_ids_reuse_without_parent_cascade_and_ca_outputs_precede_not_found() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"\xffpool\0");
    let net = call(&mut owner, &mut guest, Service::NetGetErrorAddr, &[]).unwrap();
    guest.put(net - ARG, &77_u32.to_le_bytes());
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetPoolCreate,
            &[ARG, u64::MAX, 1 << 40]
        ),
        Ok(1)
    );
    assert_eq!(owner.network.pools[1].as_ref().unwrap()._size, -1);
    assert_eq!(owner.network.pools[1].as_ref().unwrap()._name, b"\xffpool");
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetResolverCreate,
            &[ARG, 1, 0]
        ),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::SslInit, &[u64::MAX]),
        Ok(0x2_0000)
    );
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::HttpInit,
            &[1, 0x2_0000, u64::MAX]
        ),
        Ok(0x1_0000)
    );
    for service in [Service::SslGetCaCerts, Service::SslFreeCaCerts] {
        guest.put(64, &[0xa5; 40]);
        assert_eq!(
            call(&mut owner, &mut guest, service, &[u64::MAX, 0]),
            Ok(u64::from(SSL_ARGUMENT))
        );
        assert_eq!(
            call(&mut owner, &mut guest, service, &[u64::MAX, ARG + 72]),
            Ok(u64::from(SSL_INVALID))
        );
        assert_eq!(guest.get(ARG + 64, 40), [0xa5; 40]);
        assert_eq!(
            call(&mut owner, &mut guest, service, &[0x2_0000, ARG + 72]),
            Ok(if service == Service::SslGetCaCerts {
                u64::from(SSL_NOT_FOUND)
            } else {
                0
            })
        );
        assert_eq!(guest.get(ARG + 64, 8), [0xa5; 8]);
        assert_eq!(guest.get(ARG + 72, 24), [0; 24]);
        assert_eq!(guest.get(ARG + 96, 8), [0xa5; 8]);
        assert_eq!(word(&guest, net), 77); // SSL does not use FinishNetCall.
    }
    assert_eq!(
        call(&mut owner, &mut guest, Service::NetPoolDestroy, &[1]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::NetResolverDestroy, &[0]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::SslTerm, &[0x2_0000]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::HttpTerm, &[0x1_0000]),
        Ok(0)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::HttpTerm, &[0x1_0000]),
        Ok(u64::from(HTTP_INVALID))
    );
    assert_eq!(word(&guest, net), 77);
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetResolverAbort,
            &[u64::MAX, u64::MAX]
        ),
        Ok(0)
    );
    assert_eq!(owner.handle_usage().network, 0);
    assert_eq!(owner.handle_usage().live, owner.handle_usage().accounted());
    assert_eq!(
        call(&mut owner, &mut guest, Service::NetPoolCreate, &[ARG, 1, 0]),
        Ok(1)
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::SslInit, &[1]),
        Ok(0x2_0000)
    );
    assert_eq!(kernel(Default::default()).handle_usage().network, 0);
}

#[test]
fn network_startup_fixed_source_capacities_refuse_without_reading_unused_names() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"pool\0");
    for id in 1..32 {
        assert_eq!(
            call(&mut owner, &mut guest, Service::NetPoolCreate, &[ARG, 1, 0]),
            Ok(id)
        );
    }
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetPoolCreate,
            &[u64::MAX, 1, 0]
        ),
        Ok(u64::from(ENFILE))
    );
    for id in 0..32 {
        assert_eq!(
            call(
                &mut owner,
                &mut guest,
                Service::NetResolverCreate,
                &[ARG, 1, 0]
            ),
            Ok(id)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::SslInit, &[1]),
            Ok(0x2_0000 | id)
        );
        assert_eq!(
            call(&mut owner, &mut guest, Service::HttpInit, &[1, 0x2_0000, 1]),
            Ok(0x1_0000 | id)
        );
    }
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::NetResolverCreate,
            &[u64::MAX, 1, 0]
        ),
        Ok(u64::from(ENOSPACE))
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::SslInit, &[1]),
        Ok(u64::from(SSL_FULL))
    );
    assert_eq!(
        call(&mut owner, &mut guest, Service::HttpInit, &[1, 0x2_0000, 1]),
        Ok(u64::from(HTTP_MEMORY))
    );
    let counts = owner.handle_usage();
    assert_eq!(counts.network, 127);
    assert_eq!(counts.live, counts.accounted());
}

#[test]
fn network_source_abort_and_guest_or_policy_failures_do_not_publish_metadata_or_ca_output() {
    let mut owner = kernel(Default::default());
    let mut guest = TestGuest::new();
    guest.put(0, b"pool\0");
    for (service, args) in [
        (Service::NetPoolCreate, vec![ARG, 0, 0]),
        (Service::NetPoolCreate, vec![ARG, 1, 1]),
        (Service::SslInit, vec![0]),
        (Service::HttpInit, vec![u64::MAX, u64::MAX, 0]),
    ] {
        assert!(matches!(
            call(&mut owner, &mut guest, service, &args),
            Err(Failure::Unsupported(_))
        ));
        assert_eq!(owner.handle_usage().network, 0);
    }
    assert!(matches!(
        call(
            &mut owner,
            &mut guest,
            Service::NetPoolCreate,
            &[u64::MAX, 1, 0]
        ),
        Err(Failure::Guest(_))
    ));
    assert_eq!(owner.handle_usage().network, 0);
    assert_eq!(
        call(&mut owner, &mut guest, Service::SslInit, &[1]),
        Ok(0x2_0000)
    );
    guest.put(64, &[0xa5; 32]);
    guest.refuse = true;
    assert_eq!(
        call(
            &mut owner,
            &mut guest,
            Service::SslGetCaCerts,
            &[0x2_0000, ARG + 64]
        ),
        Err(Failure::Guest(GuestError::WriteDenied))
    );
    assert_eq!(guest.get(ARG + 64, 32), [0xa5; 32]);
    guest.refuse = false;
    let mut limited = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    assert_eq!(
        call(
            &mut limited,
            &mut guest,
            Service::NetPoolCreate,
            &[ARG, 1, 0]
        ),
        Ok(1)
    );
    assert_eq!(
        call(&mut limited, &mut guest, Service::SslInit, &[1]),
        Err(Failure::Limit("handles"))
    );
    assert_eq!(limited.handle_usage().network, 1);
    assert_eq!(
        limited.handle_usage().live,
        limited.handle_usage().accounted()
    );
}
