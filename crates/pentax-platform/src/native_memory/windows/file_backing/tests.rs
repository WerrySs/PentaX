// SPDX-License-Identifier: GPL-2.0-only

//! Original data-only file-section ownership, privacy and cleanup fixtures.
use super::*;
use windows_sys::Win32::{
    Foundation::{GetHandleInformation, HANDLE_FLAG_INHERIT, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE, ACE_HEADER,
        Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
        DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl, IsWellKnownSid,
        PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED, WinCreatorOwnerRightsSid, WinLocalSystemSid,
    },
    System::Memory::{MEMORY_BASIC_INFORMATION, PAGE_NOACCESS, VirtualQuery},
};

const PAGE: u64 = NATIVE_GUEST_PAGE;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);
struct Area(std::path::PathBuf);
impl Area {
    fn new() -> Self {
        let id = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("pentax-file-backing-{}-{id}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn files(&self) -> usize {
        std::fs::read_dir(&self.0).unwrap().count()
    }
    fn owner(&self, capacity: u64, private: u64) -> (NativeAddressSpace, u64) {
        let mut space = NativeAddressSpace::reserve_any(
            NativeMemoryLimits {
                mapped_bytes: 16 * PAGE,
                reserved_bytes: 0x30000,
            },
            0x30000,
        )
        .unwrap();
        space
            .use_file_backing(NativeFileBacking::open(&self.0, capacity).unwrap(), private)
            .unwrap();
        let base = space.reserved_ranges().next().unwrap().address;
        (space, base)
    }
}
impl Drop for Area {
    fn drop(&mut self) {
        // Remove only this fixture's empty directory. Never recursively delete.
        let _ = std::fs::remove_dir(&self.0);
    }
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
fn query(address: u64) -> MEMORY_BASIC_INFORMATION {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    // SAFETY: Query only; the original fixture owns the numeric address and
    // supplies exact writable output storage. No guest pointer is dereferenced.
    let result = unsafe {
        VirtualQuery(
            address as usize as *const c_void,
            &mut info,
            std::mem::size_of_val(&info),
        )
    };
    assert_ne!(result, 0);
    info
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: This exact descriptor was allocated by GetSecurityInfo.
        let _ = unsafe { LocalFree(self.0) };
    }
}
fn private_file(file: &File) {
    let handle = file.as_raw_handle();
    let mut flags = 0;
    // SAFETY: A live fixture-owned file handle and exact writable output.
    let result = unsafe { GetHandleInformation(handle, &mut flags) };
    assert_ne!(result, 0);
    assert_eq!(flags & HANDLE_FLAG_INHERIT, 0);
    let mut descriptor = std::ptr::null_mut();
    let mut acl = std::ptr::null_mut();
    // SAFETY: Live owned file; request only its DACL and LocalAlloc descriptor.
    // Scalar/pointer outputs stay live and Descriptor releases the allocation.
    let code = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut acl,
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(code, 0);
    let descriptor = Descriptor(descriptor);
    assert!(!acl.is_null());
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: Valid live Windows-created descriptor and exact scalar outputs.
    let result = unsafe { GetSecurityDescriptorControl(descriptor.0, &mut control, &mut revision) };
    assert_ne!(result, 0);
    assert_ne!(control & SE_DACL_PROTECTED, 0);
    // SAFETY: GetSecurityInfo returned a valid ACL inside the live descriptor.
    let count = unsafe { (*acl).AceCount };
    assert_eq!(count, 2);
    let mut seen = [false; 2];
    for index in 0..2 {
        let mut raw = std::ptr::null_mut();
        // SAFETY: Two ACEs were validated above; Windows returns live ACE data.
        let result = unsafe { GetAce(acl, index, &mut raw) };
        assert_ne!(result, 0);
        // SAFETY: GetAce succeeded and returns at least its documented header.
        let header = unsafe { &*raw.cast::<ACE_HEADER>() };
        assert_eq!(header.AceType, 0);
        assert_eq!(header.AceFlags, 0);
        assert!(usize::from(header.AceSize) >= std::mem::size_of::<ACCESS_ALLOWED_ACE>());
        // SAFETY: The validated type/size prove this access-allowed ACE layout.
        let ace = unsafe { &*raw.cast::<ACCESS_ALLOWED_ACE>() };
        let sid = std::ptr::addr_of!(ace.SidStart).cast_mut().cast::<c_void>();
        // SAFETY: GetSecurityInfo's validated access-allowed ACE has a live SID.
        let system = unsafe { IsWellKnownSid(sid, WinLocalSystemSid) } != 0;
        // SAFETY: Same validated SID; this is a read-only classification query.
        let owner = unsafe { IsWellKnownSid(sid, WinCreatorOwnerRightsSid) } != 0;
        assert!(system || owner);
        seen[usize::from(owner)] = true;
    }
    assert_eq!(seen, [true; 2]);
}

#[test]
fn file_backing_is_private_nx_zeroed_shared_and_independently_bounded() {
    let area = Area::new();
    let (mut space, base) = area.owner(4 * PAGE, PAGE);
    space
        .map_at(base, PAGE, NativePermissions::READ_WRITE)
        .unwrap();
    let file = space.file_backing.as_ref().unwrap().create(PAGE).unwrap();
    private_file(&file);
    let name = std::fs::read_dir(&area.0)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name();
    assert!(
        space
            .file_backing
            .as_ref()
            .unwrap()
            .directory
            .open(name)
            .is_err()
    );
    drop(file);
    assert_eq!(area.files(), 0);
    space.allocate_physical(0, 3 * PAGE).unwrap();
    assert_eq!(area.files(), 1);
    assert_eq!(
        (space.committed_bytes(), space.backing_bytes()),
        (PAGE, 4 * PAGE)
    );
    for address in [base + PAGE, base + 5 * PAGE] {
        space
            .map_physical(address, 3 * PAGE, 0, NativePermissions::READ_WRITE)
            .unwrap();
        assert_eq!(query(address).Protect, PAGE_READWRITE);
        let mut zero = vec![1; 3 * PAGE as usize];
        space.pause().unwrap().read(address, &mut zero).unwrap();
        assert!(zero.iter().all(|v| *v == 0));
    }
    let first = base + PAGE;
    let alias = base + 5 * PAGE;
    space
        .pause()
        .unwrap()
        .write(first + PAGE - 3, b"shared-boundary")
        .unwrap();
    let mut actual = [0; 15];
    space
        .pause()
        .unwrap()
        .read(alias + PAGE - 3, &mut actual)
        .unwrap();
    assert_eq!(&actual, b"shared-boundary");
    space
        .protect(alias, PAGE, NativePermissions::new(false, false, false))
        .unwrap();
    assert_eq!(query(alias).Protect, PAGE_NOACCESS);
    assert!(space.pause().unwrap().read(alias, &mut [0]).is_err());
    space.protect(alias, PAGE, NativePermissions::READ).unwrap();
    space.unmap(first + PAGE, PAGE).unwrap();
    space
        .map_physical(first + PAGE, PAGE, PAGE, NativePermissions::READ)
        .unwrap();
    space
        .pause()
        .unwrap()
        .read(first + PAGE, &mut actual[..12])
        .unwrap();
    assert_eq!(&actual[..12], b"red-boundary");
    let before = space.operations;
    assert_eq!(
        space.map_at(base + 9 * PAGE, PAGE, NativePermissions::READ),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(
        space.allocate_physical(3 * PAGE, 2 * PAGE),
        Err(NativeMemoryError::BudgetExceeded)
    );
    assert_eq!(space.operations, before);
    let statistics = space.statistics();
    assert_eq!(
        (
            statistics.file_backed_bytes,
            statistics.private_commit_limit_bytes
        ),
        (3 * PAGE, PAGE)
    );
    assert_eq!(statistics.committed_bytes, PAGE);
    apply(
        &mut space,
        NativeMemoryEffect::ReleasePhysical {
            physical: 0,
            size: 3 * PAGE,
        },
    )
    .unwrap();
    assert_eq!(
        (
            space.mapped_bytes(),
            space.committed_bytes(),
            space.backing_bytes()
        ),
        (PAGE, PAGE, PAGE)
    );
    assert_eq!(area.files(), 0);
}

#[test]
fn file_backing_partial_release_preserves_neighbors_and_reallocated_holes_are_fresh() {
    let area = Area::new();
    let (mut space, base) = area.owner(4 * PAGE, PAGE);
    space.allocate_physical(0, 3 * PAGE).unwrap();
    for address in [base, base + 5 * PAGE] {
        space
            .map_physical(address, 3 * PAGE, 0, NativePermissions::READ_WRITE)
            .unwrap();
    }
    for (offset, bytes) in [
        (0, b"left".as_slice()),
        (PAGE, b"old".as_slice()),
        (2 * PAGE, b"right".as_slice()),
    ] {
        space.pause().unwrap().write(base + offset, bytes).unwrap();
    }
    apply(
        &mut space,
        NativeMemoryEffect::ReleasePhysical {
            physical: PAGE,
            size: PAGE,
        },
    )
    .unwrap();
    assert_eq!(
        (area.files(), space.backing_bytes(), space.committed_bytes()),
        (1, 3 * PAGE, 0)
    );
    space.allocate_physical(PAGE, PAGE).unwrap();
    for address in [base + PAGE, base + 6 * PAGE] {
        space
            .map_physical(address, PAGE, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
    }
    let mut actual = [1; 5];
    space
        .pause()
        .unwrap()
        .read(base + PAGE, &mut actual)
        .unwrap();
    assert_eq!(actual, [0; 5]);
    space.pause().unwrap().write(base + PAGE, b"fresh").unwrap();
    space
        .pause()
        .unwrap()
        .read(base + 6 * PAGE, &mut actual)
        .unwrap();
    assert_eq!(&actual, b"fresh");
    space.pause().unwrap().read(base, &mut actual[..4]).unwrap();
    assert_eq!(&actual[..4], b"left");
    space
        .pause()
        .unwrap()
        .read(base + 2 * PAGE, &mut actual)
        .unwrap();
    assert_eq!(&actual, b"right");
    assert_eq!(area.files(), 2);
    apply(
        &mut space,
        NativeMemoryEffect::ReleasePhysical {
            physical: 0,
            size: 3 * PAGE,
        },
    )
    .unwrap();
    assert_eq!(
        (area.files(), space.backing_bytes(), space.committed_bytes()),
        (0, 0, 0)
    );
}

#[test]
fn file_backing_preparation_failures_leave_no_files_metadata_or_transaction_outputs() {
    for stage in 0..6 {
        let area = Area::new();
        let (mut space, base) = area.owner(4 * PAGE, PAGE);
        space
            .map_at(base, PAGE, NativePermissions::READ_WRITE)
            .unwrap();
        space.pause().unwrap().write(base, b"keep").unwrap();
        space.file_backing.as_mut().unwrap().fail_stage = Some(stage);
        let result = space.pause().unwrap().apply_transaction(
            NativeMemoryTransaction {
                effect: NativeMemoryEffect::AllocatePhysical {
                    physical: 0,
                    size: PAGE,
                },
                writes: &[NativeMemoryWrite {
                    address: base,
                    bytes: b"lost",
                }],
            },
            &Default::default(),
        );
        assert!(result.is_err());
        assert!(!space.poisoned);
        assert!(space.physical_allocations.is_empty());
        assert!(space.physical_sections.is_empty());
        let mut actual = [0; 4];
        space.pause().unwrap().read(base, &mut actual).unwrap();
        assert_eq!(&actual, b"keep");
        assert_eq!(area.files(), 0);
        assert_eq!(
            space.statistics().host_failure,
            Some(NativeHostMemoryFailure {
                operation: NativeHostMemoryOperation::BackingFile,
                code: 8
            })
        );
        space.file_backing.as_mut().unwrap().fail_stage = None;
        space.allocate_physical(0, PAGE).unwrap();
        assert_eq!(area.files(), 1);
        drop(space);
        assert_eq!(area.files(), 0);
    }
}

#[test]
fn file_backing_drop_cleans_sections_after_failed_mapping_and_poisoned_rollback() {
    for poisoned in [false, true] {
        let area = Area::new();
        let (mut space, base) = area.owner(4 * PAGE, PAGE);
        space.allocate_physical(0, 2 * PAGE).unwrap();
        space.gate = Some(space.operations + 1);
        if poisoned {
            space.rollback_gate = Some(0);
        }
        assert!(
            apply(
                &mut space,
                NativeMemoryEffect::MapPhysical {
                    address: base,
                    size: 2 * PAGE,
                    physical: 0,
                    permissions: NativePermissions::READ_WRITE
                }
            )
            .is_err()
        );
        assert_eq!(space.poisoned, poisoned);
        assert_eq!(area.files(), 1);
        assert_eq!(space.committed_bytes(), 0);
        drop(space);
        assert_eq!(area.files(), 0);
    }
}
