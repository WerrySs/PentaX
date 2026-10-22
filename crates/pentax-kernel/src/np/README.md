# Offline NP callback profile

`NpRegisterStateCallbackA` retains a process-owned callback function and userdata.
A null function returns `0x80550003`; a repeated function returns `0x80550008`
without replacing its userdata. A successful registration returns the one-based
appended ordinal and consumes the existing shared handle admission. Rows are not
reused or removed by this profile.

`NpCheckCallback` captures all pending rows and clears their pending status before
the first invocation. Immutable rows and a high-water boundary represent that
snapshot exactly. Calls receive the configured guest user ID, signed-out state1
and the original userdata. Their return registers are ignored. New registrations
are excluded from the outer snapshot and can be consumed by a later or reentrant
check. Empty checks return0 without claiming any callback executed.

The native owner must admit the current executable target and exact caller frame,
then supply a live actual return receipt for every snapshot step. Thirty-two
independent batches reuse the existing native callback-frame bound. Resource,
source or execution failures are typed; undelivered admitted batches are revoked
and cannot later appear as successful empty checks. All registered rows remain
bounded by existing handle and scratch limits. No host callback or network call
is made.

Only canonical NpManager1.1/library1 identities and the observed paired
libSceNpManager Check alias are registered. Crossed scopes and other versions
remain unresolved. The legacy registration and unregister APIs remain unavailable:
Kyty's no-argument unregister no-op does not establish the A-specific callback-ID
removal contract corroborated by public shadPS4 source. Supporting removal needs
an independently reviewed generation and snapshot-lifetime implementation.

The contract is adapted from retained KytyPS5-Performance revision209dffe,
network.cpp NpManager and libNet.cpp; see the crate NOTICE. No proprietary SDK,
game code, online state or successful commercial execution is claimed.
