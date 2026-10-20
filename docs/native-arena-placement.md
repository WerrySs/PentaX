# Native arena placement

The initial image/TLS/thread/kernel arena is reserved wholly within
`[4GiB, 1TiB)`, matching the admitted source VideoOut tracker's 40-bit surface
range. The OS chooses a free address inside that window. A valid general
47-bit allocation could otherwise exceed the surface tracker's range even
when its backing and permissions are correct. This is a software placement
profile, not a claim about host or console physical memory.

The 4GiB lower bound also leaves addressable room below the arena for the
separately owned native code cache. The prepared stepper's current 4096 bounded
attempts alternate 64KiB steps above/below the entry-point anchor, reaching at
most 128MiB in each direction. A large arena placed at 64KiB could occupy every
positive candidate while leaving insufficient space below for the runtime's
2MiB cache. The prefix is neither reserved nor committed: unrelated host owners
remain untouched, and every actual cache placement still requires successful
independent reservation. This removes that bottom-of-address-space trap; it
does not guarantee a free candidate or cache reach from an entry deep inside
an unusually large arena. Existing bounded refusal remains explicit.

`NativeAddressSpace::reserve_within` validates size, alignment, reservation
quota, exclusive window end and overflow before asking Windows for one owned
NOACCESS placeholder. It checks actual application-address bounds and verifies
the returned complete extent again before publishing the owner. No backing is
committed by reservation. An incompatible host or unavailable window refuses;
there is no broader fallback or address-space scan. Generic `reserve_any`
owners retain their 47-bit policy, and later explicit runtime mappings keep
their existing independent bounds. VideoOut registration still checks every
surface's actual complete range, layout and permissions.

The implementation follows Microsoft's
[VirtualAlloc2](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-virtualalloc2),
[MEM_ADDRESS_REQUIREMENTS](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-mem_address_requirements)
and [MEM_EXTENDED_PARAMETER](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-mem_extended_parameter)
contracts. NULL BaseAddress enables the address requirement; Windows receives
an inclusive highest address of `exclusive_end - 1`, allocation-granularity
alignment, `MEM_RESERVE | MEM_RESERVE_PLACEHOLDER` and `PAGE_NOACCESS`.

Original nonvisual fixtures check pure invalid bounds, unsupported-host
refusal and actual constrained reservation/owner/map/rights/release behavior.
The retained VideoOut event fixture continues through the production image
arena and actual registered pixels. These checks do not establish a rendered
or displayed commercial frame.
