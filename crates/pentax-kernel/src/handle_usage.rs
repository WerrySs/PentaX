// SPDX-License-Identifier: GPL-2.0-only

use crate::Kernel;

/// Closed aggregate census of retained resources, independent of the budget's
/// running count. No guest identifiers, addresses or file names are returned.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandleUsage {
    pub live: usize,
    pub limit: usize,
    pub memory: usize,
    /// Subset of `memory`; never add this a second time to the total.
    pub pthread_objects: usize,
    pub files: usize,
    pub modules: usize,
    pub synchronization: usize,
    pub events: usize,
    pub network: usize,
    pub ampr: usize,
    pub keys: usize,
    pub semaphores: usize,
    pub fibers: usize,
    /// Caller-backed libc arenas; their chunks are independently bounded.
    pub libc_heaps: usize,
    /// Trophy contexts, cached packages and per-user package state records.
    pub trophies: usize,
    /// Subset of `trophies`; never add this again to `accounted()`.
    pub trophy_contexts: usize,
    /// Subset of `trophy_contexts` with a completed package registration.
    pub trophy_registered_contexts: usize,
    /// Subset of `trophies`, populated only after real package admission.
    pub trophy_packages: usize,
    /// Subset of `trophies`, containing actual per-user package states.
    pub trophy_users: usize,
    /// UDS contexts and owned opaque objects; their mappings count in memory.
    pub uds: usize,
    /// Logical font, text, glyph and parser records. Their real owned mappings
    /// are independently counted in `memory`, as for UDS objects.
    pub fonts: usize,
    /// IME sessions and pending event dispatches. Temporary event mappings are
    /// independently included in memory, never added twice here.
    pub ime: usize,
    /// Creation-only ACM contexts; no batch or audio execution is implied.
    pub acm: usize,
}
impl HandleUsage {
    #[must_use]
    pub const fn accounted(&self) -> usize {
        self.memory
            + self.files
            + self.modules
            + self.synchronization
            + self.events
            + self.network
            + self.ampr
            + self.keys
            + self.semaphores
            + self.fibers
            + self.libc_heaps
            + self.trophies
            + self.uds
            + self.fonts
            + self.ime
            + self.acm
    }
}
impl Kernel {
    /// Allocation-free observation of bounded owner tables. This does not
    /// change admission, free objects or claim that guest cleanup took place.
    #[must_use]
    pub fn handle_usage(&self) -> HandleUsage {
        let (trophy_contexts, trophy_packages, trophy_users, trophy_registered_contexts) =
            self.trophies.retained_record_counts();
        HandleUsage {
            live: self.usage().handles_live,
            limit: self.budget.limits.max_handles,
            memory: self.memory.retained_handle_count(),
            pthread_objects: self.memory.retained_pthread_object_count(),
            files: self.files.retained_handle_count(),
            modules: self.sysmodules.retained_handle_count(),
            synchronization: self.sync.retained_handle_count(),
            events: self.event_queues.retained_handle_count(),
            network: self.np.retained_handle_count()
                + self.network.retained_handle_count()
                + self.http2.retained_handle_count()
                + self.np_webapi2.retained_handle_count(),
            ampr: self.ampr.retained_handle_count(),
            keys: self.pthread_keys.retained_handle_count(),
            semaphores: self.pthread_semaphores.retained_handle_count(),
            fibers: self.fibers.retained_handle_count(),
            libc_heaps: self.mspaces.retained_handle_count(),
            trophies: self.trophies.retained_handle_count(),
            trophy_contexts,
            trophy_registered_contexts,
            trophy_packages,
            trophy_users,
            uds: self.uds.retained_handle_count(),
            fonts: self.fonts.retained_handle_count(),
            ime: self.ime.retained_handle_count(),
            acm: self.acm.retained_handle_count(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TestGuest, kernel};
    use crate::{CallArgs, Failure, Limits, Service};

    #[test]
    fn trophy2_contexts_share_global_quota_and_keep_failed_publication_out_of_census() {
        let mut owner = kernel(Limits {
            max_handles: 2,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let mut call = |owner: &mut Kernel, service, values: &[u64]| {
            let mut args = [0; 6];
            args[..values.len()].copy_from_slice(values);
            owner.invoke_library(service, CallArgs(args), &mut guest)
        };
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[7]), Ok(0));
        assert_eq!(
            call(
                &mut owner,
                Service::Trophy2CreateContext,
                &[0x1_0000, 7, 0, 0]
            ),
            Ok(0)
        );
        let full = owner.handle_usage();
        assert_eq!(
            (full.live, full.accounted(), full.trophies, full.modules),
            (2, 2, 1, 1)
        );
        assert_eq!(
            (
                full.trophy_contexts,
                full.trophy_packages,
                full.trophy_users
            ),
            (1, 0, 0)
        );
        assert_eq!(full.trophy_registered_contexts, 0);
        assert!(matches!(
            call(
                &mut owner,
                Service::Trophy2CreateContext,
                &[0x1_0000, 7, 1, 0]
            ),
            Err(Failure::Limit(_))
        ));
        assert_eq!(owner.handle_usage(), full);
        assert_eq!(
            call(&mut owner, Service::Trophy2DestroyContext, &[1]),
            Ok(0)
        );
        assert_eq!(
            call(
                &mut owner,
                Service::Trophy2CreateContext,
                &[0x1_0000, 7, 1, 0]
            ),
            Ok(0)
        );
        assert_eq!(
            call(&mut owner, Service::Trophy2DestroyContext, &[2]),
            Ok(0)
        );
        assert_eq!(call(&mut owner, Service::SysmoduleUnload, &[7]), Ok(0));
        let empty = owner.handle_usage();
        assert_eq!((empty.live, empty.accounted(), empty.trophies), (0, 0, 0));
        assert_eq!(
            (
                empty.trophy_contexts,
                empty.trophy_packages,
                empty.trophy_users
            ),
            (0, 0, 0)
        );
        assert_eq!(empty.trophy_registered_contexts, 0);
    }

    #[test]
    fn mspace_arenas_share_global_handle_quota_and_have_a_separate_census() {
        let mut owner = kernel(Limits {
            max_handles: 2,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let mut call = |owner: &mut Kernel, service, values: &[u64]| {
            let mut args = [0; 6];
            args[..values.len()].copy_from_slice(values);
            owner.invoke_library(service, CallArgs(args), &mut guest)
        };
        assert_eq!(
            call(&mut owner, Service::MspaceCreate, &[0, 0x1_0000, 0x1000, 0]),
            Ok(0x1_0000)
        );
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[7]), Ok(0));
        let full = owner.handle_usage();
        assert_eq!(
            (full.live, full.accounted(), full.libc_heaps, full.modules),
            (2, 2, 1, 1)
        );
        assert_eq!(
            call(&mut owner, Service::MspaceCreate, &[0, 0x1_1000, 0x1000, 0]),
            Err(Failure::Limit("handles"))
        );
        assert_eq!(owner.handle_usage(), full);
        assert_eq!(
            call(&mut owner, Service::MspaceMalloc, &[0x1_0000, 64]),
            Ok(0x1_0040)
        );
        assert_eq!(owner.handle_usage(), full);
        assert_eq!(call(&mut owner, Service::MspaceDestroy, &[0x1_0000]), Ok(0));
        assert_eq!(
            call(&mut owner, Service::MspaceCreate, &[0, 0x1_1000, 0x1000, 0]),
            Ok(0x1_1000)
        );
        assert_eq!(call(&mut owner, Service::MspaceDestroy, &[0x1_1000]), Ok(0));
        assert_eq!(call(&mut owner, Service::SysmoduleUnload, &[7]), Ok(0));
        let empty = owner.handle_usage();
        assert_eq!((empty.live, empty.accounted(), empty.libc_heaps), (0, 0, 0));
    }

    #[test]
    fn np_webapi2_shares_global_quota_and_reports_owned_records_in_network_census() {
        let mut owner = kernel(Limits {
            max_handles: 2,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let output = 0x1_0000;
        let mut call = |owner: &mut Kernel, service, values: &[u64]| {
            let mut args = [0; 6];
            args[..values.len()].copy_from_slice(values);
            owner.invoke_library(service, CallArgs(args), &mut guest)
        };
        assert_eq!(
            call(&mut owner, Service::NpWebApi2Initialize, &[0, 0x4000]),
            Ok(1)
        );
        assert_eq!(
            call(
                &mut owner,
                Service::NpWebApi2CreateRequest,
                &[0, 0, 0, 0, 0, output]
            ),
            Ok(0)
        );
        let full = owner.handle_usage();
        assert_eq!((full.live, full.accounted(), full.network), (2, 2, 2));
        assert_eq!(
            call(&mut owner, Service::SysmoduleLoad, &[7]),
            Err(Failure::Limit("handles"))
        );
        assert_eq!(owner.handle_usage(), full);
        assert_eq!(
            call(&mut owner, Service::NpWebApi2DeleteRequest, &[1000]),
            Ok(0)
        );
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[7]), Ok(0));
        let mixed = owner.handle_usage();
        assert_eq!(
            (mixed.live, mixed.accounted(), mixed.network, mixed.modules),
            (2, 2, 1, 1)
        );
        assert_eq!(call(&mut owner, Service::NpWebApi2Terminate, &[1]), Ok(0));
        assert_eq!(call(&mut owner, Service::SysmoduleUnload, &[7]), Ok(0));
        let empty = owner.handle_usage();
        assert_eq!((empty.live, empty.accounted(), empty.network), (0, 0, 0));
    }

    #[test]
    fn handle_census_balances_shared_owner_quota_reuse_refusal_and_final_cleanup() {
        let mut owner = kernel(Limits {
            max_handles: 6,
            ..Default::default()
        });
        let mut guest = TestGuest::new();
        let mut call = |owner: &mut Kernel, service, values: &[u64]| {
            let mut args = [0; 6];
            args[..values.len()].copy_from_slice(values);
            owner.invoke_library(service, CallArgs(args), &mut guest)
        };
        let slot = 0x1_0000;
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[7]), Ok(0));
        assert_eq!(call(&mut owner, Service::MutexLock, &[slot]), Ok(0));
        let request = call(&mut owner, Service::NpCreateRequest, &[0]).unwrap();
        assert_eq!(call(&mut owner, Service::KeyCreate, &[slot + 16, 0]), Ok(0));
        assert_eq!(
            call(&mut owner, Service::PthreadSemInit, &[slot + 64, 0, 2]),
            Ok(0)
        );
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[8]), Ok(0));
        let full = owner.handle_usage();
        assert_eq!((full.live, full.accounted(), full.limit), (6, 6, 6));
        assert_eq!(
            (
                full.modules,
                full.memory,
                full.pthread_objects,
                full.network,
                full.keys,
                full.semaphores
            ),
            (2, 1, 1, 1, 1, 1)
        );
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[7]), Ok(0));
        assert_eq!(owner.handle_usage(), full); // A reference is not another live object.
        assert_eq!(
            call(&mut owner, Service::SysmoduleLoad, &[9]),
            Err(Failure::Limit("handles"))
        );
        assert_eq!(owner.handle_usage(), full);
        assert_eq!(call(&mut owner, Service::KeyDelete, &[0]), Ok(0));
        assert_eq!(owner.handle_usage().keys, 0); // A free generation slot is not live.
        assert_eq!(call(&mut owner, Service::SysmoduleLoad, &[9]), Ok(0));
        for id in [7, 7, 8, 9] {
            assert_eq!(call(&mut owner, Service::SysmoduleUnload, &[id]), Ok(0));
        }
        assert_eq!(call(&mut owner, Service::MutexUnlock, &[slot]), Ok(0));
        assert_eq!(call(&mut owner, Service::MutexDestroy, &[slot]), Ok(0));
        assert_eq!(
            call(&mut owner, Service::NpDeleteRequest, &[request]),
            Ok(0)
        );
        assert_eq!(
            call(&mut owner, Service::PthreadSemDestroy, &[slot + 64]),
            Ok(0)
        );
        let empty = owner.handle_usage();
        assert_eq!(
            (empty.live, empty.accounted(), empty.pthread_objects),
            (0, 0, 0)
        );
        assert_eq!(kernel(Default::default()).handle_usage().live, 0);
    }
}
