# Owned module information for unwinding

`KernelGetModuleInfoForUnwind` and `SysmoduleGetModuleInfoForUnwind` share the
304-byte source ABI. The normal kernel and Sysmodule registrations are explicit;
the additional paired `libSceSysmodule` spelling has its own exact binding.
No other scope or version combination is inferred.

The operation selects an actual guest module in the current retained source
epoch, excludes generated providers, and checks its current owned mappings.
Unknown addresses and unmapped module extents return the source ESRCH error.
Guest addresses never become host pointers. There is no synthetic host-boundary
module or success based on the magnitude of an unknown address.

The loader's opt-in metadata query validates a unique initialized
`PT_GNU_EH_FRAME` header and a checked fixed-width absolute or signed/unsigned
PC-relative frame pointer. A plain ELF can additionally supply the exact size
from an allocated `.eh_frame` section whose file extent is the same initialized
source slice. Section counts are capped at 4096 and string tables at one MiB.
SELF section offsets are not interpreted as container offsets; SELF uses its
initialized GNU header. Without an explicit frame size, the range ends at the
following header or initialized containing segment boundary. Malformed,
ambiguous, indirect and unsupported encodings remain typed refusals.

The module name comes from bounded source `DT_SONAME` bytes. A synthetic input
without that field has an empty name; no unavailable filename or ambient host
path is invented. The native adapter checks live header bytes before publishing
their pointers. A changed header requires review rather than stale success.
This operation supplies metadata to a guest unwinder; it does not implement an
exception unwinder, interpret CFI, throw host exceptions or execute guest code.

The source's low int32 flag, null pointer, size validation and invalid-flag zero
store order are retained. A successful result is one checked 304-byte guest
transaction. Library errors use the zero-extended int32 kernel domain. Existing
guest-byte, staging, provider-work, mapping and ownership bounds remain active.
Original loader, kernel and native-provider-request fixtures are source only
until separately compiled and run; provider-request tests execute no guest CPU.

Behavior is adapted from `src/libs/libSysmodule.cpp` and
`src/libs/libKernel.cpp` at the pinned
[KytyPS5-Performance revision](https://github.com/WerrySs/KytyPS5-Performance/tree/209dffe9032d72749257ce18680d506b891284de).
The header encoding is independently described by the
[Linux Standard Base](https://refspecs.linuxfoundation.org/LSB_2.0.0/LSB-Core/LSB-Core/ehframehdr.html).
Upstream attribution and licenses are retained in the affected crates' notices.
