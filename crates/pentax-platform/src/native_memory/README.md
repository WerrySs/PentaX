# Native kernel memory transactions

This is a bounded Rust port of the mapping publication and rollback ordering in
the pinned Kyty performance fork, commit
`209dffe9032d72749257ce18680d506b891284de`, `src/kernel/memory.cpp`:
`PhysicalMemory::Map`, `PhysicalMemory::ProtectMapping`, `MapDirectMemory`, and
the flexible/virtual-range publication paths. Copyright (c) 2021 InoriRus;
modified 2026 for PentaX. The retained upstream MIT notice is
[Kyty-MIT.txt](../../../pentax-kernel/Kyty-MIT.txt); PentaX is GPL-2.0-only.
No C++ implementation or upstream byte fixture is embedded.

The supported CPU-backing subset is `None`, `MapZeroed`, `Protect`, `Unmap`,
`AllocatePhysical`, `MapPhysical` and `ReleasePhysical` plus one
bounded batch of nonoverlapping output writes. The exact exclusive native owner
is preflighted against prospective permissions before allocating metadata or
changing OS state. New pages are real zero-initialized Windows commitments at
the checked guest address. Reservations use `VirtualAlloc2` placeholders split
into exact 16 KiB owned slots before publication. Private allocations and shared
views replace these slots without releasing their addresses. Existing owner
budgets cover both total virtual mapped bytes and unique admitted backing, and
64 KiB reservations; 16 KiB guest holes remain owned, uncommitted `NOACCESS`.
All physical protections remain NX, including logical executable mappings.

`NativeAddressSpace::reserve_within` additionally reserves one OS-selected
placeholder wholly inside an explicit allocation-granularity-aligned window
with an exclusive end. Size, overflow, native47-bit range and reservation quota
are checked before allocation; the requested window must also fit the actual
Windows application range. One
[VirtualAlloc2](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc2)
call uses NULL BaseAddress, `MEM_RESERVE | MEM_RESERVE_PLACEHOLDER` and
`PAGE_NOACCESS`, with a call-live
[MEM_ADDRESS_REQUIREMENTS](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-mem_address_requirements)
in one typed extended parameter. Windows receives inclusive highest address
`end - 1`; returned alignment and the complete extent are checked again before
owner publication. Reservation commits no backing. Unsupported geometry or
allocation failure is explicit, with no scan, replacement or broader fallback.
`reserve_any` retains its general placement policy. This initial-placement
constraint does not grant payload access or constrain later explicit mappings
beyond their existing owner/permission/budget checks.

Live span checks locate the first sorted page once and then verify every expected
page address and permission in order. Missing pages remain holes; later pages
cannot supply their rights. The non-executable span check validates logical NX
separately from physical NX and grants no read/write access. Stack admission still
checks the complete live RW/NX span and both owned unmapped guards each time.

Only after the complete backing effect succeeds do output copies commit.
Outputs may lie in newly created writable pages (including the original
pthread guest prefix). A recoverable OS failure rolls back before returning.
Unmap preflights the whole span and every output against its prospective holes.
It restores placeholders while retaining reservations. Removing private backing
discards its contents; a fresh private map produces zeroes without overwriting
neighbors. Shared view removal preserves its section's contents. A first injected pre-operation
failure is recoverable. Any partial decommit or actual OS decommit failure
poisons the owner, because discarded contents cannot be reconstructed safely.
No output copies occur after a failed effect. An unexpected post-effect output
failure also poisons the owner and must never be retried as an ordinary error.
If transaction rollback fails, pending reservations whose release failed stay
in the poisoned address-space owner: pause/read/write/fetch/map/protect all
fail. No retry or native resume is valid after that fatal result. Diagnostic
counters may still be inspected. Separately, an OS reservation result rejected
inside its constructor is released there; failed cleanup returns `Poisoned`
and its local destructor retries release without publishing a usable owner.
Final destructor cleanup cannot guarantee reclamation after an OS release
failure; no live memory capability survives that constructor/destructor path.

Physical allocations own unnamed, noninheritable, data-only pagefile sections.
Direct views use `MapViewOfFile3(MEM_REPLACE_PLACEHOLDER)` at exact 16 KiB guest
addresses and offsets, including sub-64 KiB offsets. Each view initially admits
RW/NX access and is restricted to its requested protection before publication;
this preserves later protection upgrades without copy-on-write. Removing a view
uses `UnmapViewOfFile2(MEM_PRESERVE_PLACEHOLDER)`. No address-release window or
copied anonymous fallback is used. Physical release admits a completely allocated
range, stages split physical records and rollback data for every affected alias,
then withdraws just those alias pages while retaining their placeholders.
Surviving pieces retain their exact section offsets. Direct maps can traverse
adjacent physical pieces backed by different sections. Reallocation of a freed
hole creates fresh zeroed backing. CPU and GPU permission bits remain independent on each alias. Host cache
policies remain unsupported; the guest allocation type does not select a host
cache mode. Memory pools remain outside this profile.

Recoverable detach failure or an injected failure before the first section close
restores the original alias views, bytes and permissions before returning.
Failed restoration or failure after any section has closed poisons the owner;
closed handles are never used to attempt rollback. Kernel metadata commits only
after the complete backing transaction. Release output preflight also refuses
outputs into any affected alias, including distant virtual addresses.

Physical pieces refer to retained section slots and offsets; empty slots are
reused only after every referring piece and view has gone. A partially used
Windows pagefile section retains its committed offsets, so its entire capacity
stays charged to the backing budget until the last piece is released and the
section closes. Logical physical holes become reusable immediately; neither
backing capacity nor actual commitment is credited prematurely. Temporary split/rollback metadata is bounded
by `NativeTransactionLimits::max_scratch_bytes` before any OS mutation.

Output preflight checks canonical physical byte ranges as well as virtual
addresses, including prospective new views and self-aliasing within one output.
Conflicting alias outputs refuse before effects. Their conservative quadratic
chunk-comparison work is charged first. Section allocations and private pages
share the unique-backing cap; every virtual alias separately consumes mapped
capacity. `backing_bytes` counts private pages plus whole retained section
capacity; `committed_bytes` counts pagefile-backed private pages plus unique
committed pagefile-section offsets, including offsets without current views.
`file_backed_bytes` separately counts whole retained data-file sections;
their mapped `MEM_COMMIT` state does not represent pagefile commitment.
`mapped_bytes` counts live virtual views/private pages.
Neither is an RSS measurement. This Windows path requires the placeholder APIs
available with Windows 10 version 1803 or later.

Compact owners allocate sections eagerly. The explicit runtime profile opts
into `SEC_RESERVE` sections before allocating physical memory. A first 16 KiB
view commits only its own section offset with `VirtualAlloc(MEM_COMMIT)` before
permissions or outputs publish. A preallocated bitmap prevents duplicate charges
for aliases and remaps. This defers commitment until mapping, not first CPU
access; mapped pages are fully committed. All section capacity remains admitted
at allocation, so deferred commitment cannot bypass the backing ceiling.
Windows cannot decommit these offsets independently: failed later mapping steps
roll back views but retain/charge successful section commits until final close.
Partial release preserves the same rule and newly allocated physical holes get
fresh zeroed sections. No host settings, files or automatic retries are involved.

An explicit `NativeFileBacking` selects a different physical-section backend
before any physical allocation. It never retries a failed pagefile allocation.
An existing local fixed-drive directory must support sparse files and have
quota-visible free space for the entire admitted capacity plus 2 GiB. This is a
snapshot, not a reservation against other disk users. The directory is pinned;
fresh files use relative capability-based `create_new`, exclusive noninheritable
handles, protected owner/SYSTEM access and delete-on-close. No existing file is
opened, truncated or removed. Sparse extents are explicitly zeroed before an
unnamed RW/NX data section is created. Views retain the same offset, alias,
permission and rollback rules. The retained file closes only after its section;
partial physical release cannot credit the remaining section's capacity.

File-backed mode independently bounds private pagefile commitment through
`private_commit_limit_bytes`; private image, TLS and anonymous mappings still
consume that allowance. Both backends share the overall backing and mapped-view
limits. File-backed data can page to the selected disk and may incur I/O latency
or later storage faults; it is not a claim of lower RSS or higher frame rate.
Preparation failures, owner cleanup and poisoned transaction cleanup close the
owned files. The containing directory is never recursively deleted.

Memory-transaction OS errors also retain a closed operation category and numeric
Win32 code. The record clears at the next transaction and excludes addresses,
paths, raw error strings and guest bytes. It is diagnostic, not access authority.

Microsoft API contracts used by this port:

- [VirtualAlloc](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc):
  reservation granularity and zero-initialized commitment.
- [VirtualAlloc2](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc2)
  and [VirtualFree](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualfree):
  placeholder reservation, exact splitting, private replacement and restoration.
- [VirtualProtect](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualprotect):
  committed pages and separate reservation boundaries. Protection is applied
  one owned 16 KiB guest page at a time.
- [MapViewOfFile3](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-mapviewoffile3):
  shared sections and exact placeholder replacement, including 16 KiB offsets.
- [UnmapViewOfFile2](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-unmapviewoffile2):
  removal of a shared view while retaining its placeholder.
- [CreateFileMappingW](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-createfilemappingw):
  file-section lifetime and shared aliases; extending a file alone does not promise zero bytes.
- [Sparse files](https://learn.microsoft.com/en-us/windows/win32/fileio/sparse-files)
  and [FSCTL_SET_ZERO_DATA](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_set_zero_data):
  zero-filled sparse extents without eagerly writing the full logical capacity.
- [SetSecurityInfo](https://learn.microsoft.com/en-us/windows/win32/api/aclapi/nf-aclapi-setsecurityinfo):
  protected access on each newly created file before guest data is stored.
- [CreateFileMappingW](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-createfilemappingw):
  eager/reserved pagefile sections, retained commitment, view references and final reclamation.

This memory API contains no executor, running state, public host pointer,
firmware TCB/DTV, raw syscall forwarding or global allocator mutation.

`NativeOwnerId` identifies one process-local owner independently of its Rust or
guest addresses. Construction claims a checked monotonically increasing token
before any host operation; exhausted counters fail without wrap. Moves preserve
the token, and failed constructions or dropped owners never return one for reuse.
The opaque copyable token is a comparison aid, not liveness or execution proof.
Unsupported hosts cannot construct an owner or fabricate a usable token.

An exclusive pause exposes `preflight_fetch_source` for complete logical-X spans
and `preflight_reserved_unmapped` for nonempty 16 KiB aligned guard ranges. The
latter walks the actual reservation union and refuses every logical mapped page,
including no-permission mappings. Foreign reservations or gaps between owned
reservations do not become guards. Both checks reject poisoned owners first;
they do not allocate, change protections, copy bytes or grant execution access.

`unmap` preflights a complete aligned owned mapping before restoring its exact
private/shared slots to placeholders. Reservations remain owned for later admission and
guards. A partial/OS decommit failure poisons the owner; `abandon` also permanently
revokes access after failed rollback. Drop still owns reservation cleanup. These
operations do not free unrelated memory or provide running-memory access.


CPU read/write/source-fetch and GPU read/write use separate logical bits on the
same owned pages. Physical Windows protection admits the union needed by those
operations while remaining NX. Ordinary CPU stepping still checks CPU rights;
GPU permission cannot authorize a CPU operand. Mapping/protection transactions
preserve all bits, and guest outputs from kernel calls still require CPU writes.

GPU reads, compiler-derived buffer snapshots and bounded atomic GPU writeback
use explicit paused-owner methods. Write-only buffer staging preserves untouched
bytes for full-buffer fenced writeback without admitting shader reads. GPU outputs
retain complete-span permission, overlap, physical-alias, work and byte checks.
Two aliases share contents while retaining their own access rights. CPU and GPU
are serialized by the exclusive pause through upload, fence and writeback; this
is synchronous visibility, not concurrent cache emulation or asynchronous queues.
