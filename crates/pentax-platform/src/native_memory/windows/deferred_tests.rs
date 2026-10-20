// SPDX-License-Identifier: GPL-2.0-only

//! Original reserved-section fixtures: actual OS state, aliases and rollback.
use super::*;
use windows_sys::Win32::System::Memory::{
    FILE_MAP_READ, MEM_COMMIT, MEMORY_BASIC_INFORMATION, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    UnmapViewOfFile, VirtualQuery,
};

const PAGE: u64 = NATIVE_GUEST_PAGE;

struct ObservedView(MEMORY_MAPPED_VIEW_ADDRESS);
impl ObservedView {
    fn new(section: &Section) -> Self {
        // SAFETY: Live original fixture section, read-only inspection view,
        // no chosen address and no source/executable mapping. Drop unmaps it.
        let view = unsafe { MapViewOfFile(section.handle, FILE_MAP_READ, 0, 0, 0) };
        assert!(!view.Value.is_null());
        Self(view)
    }
    fn state(&self, offset: u64) -> u32 {
        state(self.0.Value as u64 + offset).State
    }
}
impl Drop for ObservedView {
    fn drop(&mut self) {
        // SAFETY: The view belongs only to this original fixture wrapper.
        let _ = unsafe { UnmapViewOfFile(self.0) };
    }
}

fn owner() -> (NativeAddressSpace, u64) {
    let mut space = NativeAddressSpace::reserve_any(Default::default(), 0x30000).unwrap();
    space.defer_physical_commit().unwrap();
    let base = space.reserved_ranges().next().unwrap().address;
    (space, base)
}
fn apply(
    space: &mut NativeAddressSpace,
    effect: NativeMemoryEffect,
) -> Result<(), NativeMemoryError> {
    space.pause().unwrap().apply_transaction(
        NativeMemoryTransaction {
            effect,
            writes: &[],
        },
        &Default::default(),
    )
}
fn state(address: u64) -> MEMORY_BASIC_INFORMATION {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    assert_ne!(
        // SAFETY: VirtualQuery only inspects this original owner's live address.
        // The writable output has the exact requested size and lifetime.
        unsafe {
            VirtualQuery(
                address as usize as *const c_void,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        },
        0
    );
    info
}

#[test]
fn deferred_physical_commit_tracks_first_views_across_bitmap_words_and_alias_lifetimes() {
    let (mut space, base) = owner();
    const CAPACITY: u64 = 130 * PAGE;
    apply(
        &mut space,
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: CAPACITY,
        },
    )
    .unwrap();
    assert_eq!(space.committed_bytes(), 0);
    assert_eq!(space.backing_bytes(), CAPACITY);
    {
        let view = ObservedView::new(space.physical_sections[0].as_ref().unwrap());
        for offset in [0, 64 * PAGE, CAPACITY - PAGE] {
            assert_eq!(view.state(offset), MEM_RESERVE);
        }
    }
    assert_eq!(
        space.defer_physical_commit(),
        Err(NativeMemoryError::InvalidRange)
    );
    for (address, offset) in [
        (base, 0),
        (base + PAGE, CAPACITY - PAGE),
        (base + 2 * PAGE, 0),
    ] {
        space
            .map_physical(address, PAGE, offset, NativePermissions::READ_WRITE)
            .unwrap();
        let info = state(address);
        assert_eq!((info.State, info.Protect), (MEM_COMMIT, PAGE_READWRITE));
        let mut zeros = vec![1; PAGE as usize];
        space.pause().unwrap().read(address, &mut zeros).unwrap();
        assert!(zeros.iter().all(|v| *v == 0));
    }
    assert_eq!(space.committed_bytes(), 2 * PAGE);
    assert_eq!(space.mapped_bytes(), 3 * PAGE);
    {
        let view = ObservedView::new(space.physical_sections[0].as_ref().unwrap());
        assert_eq!(view.state(0), MEM_COMMIT);
        assert_eq!(view.state(64 * PAGE), MEM_RESERVE);
        assert_eq!(view.state(CAPACITY - PAGE), MEM_COMMIT);
    }
    space
        .pause()
        .unwrap()
        .write(base + 2 * PAGE, b"shared")
        .unwrap();
    let mut actual = [0; 6];
    space.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"shared");
    space.unmap(base, 3 * PAGE).unwrap();
    assert_eq!(space.committed_bytes(), 2 * PAGE);
    assert_eq!(space.mapped_bytes(), 0);
    space
        .map_physical(base, PAGE, 0, NativePermissions::new(false, false, false))
        .unwrap();
    assert_eq!(state(base).Protect, PAGE_NOACCESS);
    assert!(space.pause().unwrap().read(base, &mut actual).is_err());
    space.protect(base, PAGE, NativePermissions::READ).unwrap();
    space.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"shared");
    assert_eq!(space.committed_bytes(), 2 * PAGE);
    apply(
        &mut space,
        NativeMemoryEffect::ReleasePhysical {
            physical: 0,
            size: CAPACITY,
        },
    )
    .unwrap();
    assert_eq!(
        (
            space.mapped_bytes(),
            space.committed_bytes(),
            space.backing_bytes()
        ),
        (0, 0, 0)
    );
    apply(
        &mut space,
        NativeMemoryEffect::AllocatePhysical {
            physical: 0,
            size: CAPACITY,
        },
    )
    .unwrap();
    space
        .map_physical(base, PAGE, 0, NativePermissions::READ)
        .unwrap();
    space.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(actual, [0; 6]);
    assert_eq!(space.committed_bytes(), PAGE);
}

#[test]
fn deferred_physical_commit_keeps_rolled_back_and_partially_released_offsets_charged() {
    let (mut space, base) = owner();
    space.allocate_physical(0, 3 * PAGE).unwrap();
    let before = space.operations;
    space.gate = Some(before + 1); // First view committed; second never mapped.
    assert!(
        apply(
            &mut space,
            NativeMemoryEffect::MapPhysical {
                address: base,
                size: 2 * PAGE,
                physical: 0,
                permissions: NativePermissions::READ_WRITE,
            }
        )
        .is_err()
    );
    assert!(!space.poisoned);
    assert_eq!(
        (
            space.mapped_bytes(),
            space.committed_bytes(),
            space.backing_bytes()
        ),
        (0, PAGE, 3 * PAGE)
    );
    assert_eq!(state(base).State, MEM_RESERVE);
    assert_eq!(
        space.statistics().host_failure,
        Some(NativeHostMemoryFailure {
            operation: NativeHostMemoryOperation::Other,
            code: 8,
        })
    );
    space.gate = None;
    for (address, offset) in [(base, 0), (base + PAGE, PAGE), (base + 2 * PAGE, PAGE)] {
        space
            .map_physical(address, PAGE, offset, NativePermissions::READ_WRITE)
            .unwrap();
    }
    space.pause().unwrap().write(base, b"kept").unwrap();
    let failure = NativeMemoryEffect::ReleasePhysical {
        physical: PAGE,
        size: PAGE,
    };
    // Fail before closing anything and restore the detached section view.
    space.gate = Some(space.operations + 1);
    assert!(apply(&mut space, failure).is_err());
    assert!(!space.poisoned);
    assert_eq!(space.mapped_bytes(), 3 * PAGE);
    space.gate = None;
    apply(&mut space, failure).unwrap();
    assert_eq!(space.statistics().host_failure, None);
    assert_eq!(
        (
            space.mapped_bytes(),
            space.committed_bytes(),
            space.backing_bytes()
        ),
        (PAGE, 2 * PAGE, 3 * PAGE)
    );
    // Reusing a physical hole creates a fresh section, not old retained bytes.
    apply(
        &mut space,
        NativeMemoryEffect::AllocatePhysical {
            physical: PAGE,
            size: PAGE,
        },
    )
    .unwrap();
    assert_eq!(
        (space.committed_bytes(), space.backing_bytes()),
        (2 * PAGE, 4 * PAGE)
    );
    space
        .map_physical(base + PAGE, PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    let mut actual = [1; 4];
    space
        .pause()
        .unwrap()
        .read(base + PAGE, &mut actual)
        .unwrap();
    assert_eq!(actual, [0; 4]);
    space.pause().unwrap().read(base, &mut actual).unwrap();
    assert_eq!(&actual, b"kept");
    apply(
        &mut space,
        NativeMemoryEffect::ReleasePhysical {
            physical: 0,
            size: 3 * PAGE,
        },
    )
    .unwrap();
    assert_eq!((space.committed_bytes(), space.backing_bytes()), (0, 0));
}

#[test]
fn deferred_physical_commit_cannot_bypass_whole_backing_or_alias_mapping_budgets() {
    let (mut space, base) = owner();
    space
        .restrict_limits(NativeMemoryLimits {
            mapped_bytes: 2 * PAGE,
            reserved_bytes: space.reserved_bytes(),
        })
        .unwrap();
    space.allocate_physical(0, 2 * PAGE).unwrap();
    assert_eq!(space.committed_bytes(), 0);
    let before = space.operations;
    assert_eq!(
        space.allocate_physical(2 * PAGE, PAGE),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        space.map_at(base, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        space.restrict_limits(NativeMemoryLimits {
            mapped_bytes: PAGE,
            reserved_bytes: space.reserved_bytes()
        }),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(space.operations, before);
    for address in [base, base + PAGE] {
        space
            .map_physical(address, PAGE, 0, NativePermissions::READ)
            .unwrap();
    }
    assert_eq!(space.committed_bytes(), PAGE);
    assert_eq!(
        space.map_physical(base + 2 * PAGE, PAGE, 0, NativePermissions::READ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(space.statistics().backing_bytes, 2 * PAGE);
}
