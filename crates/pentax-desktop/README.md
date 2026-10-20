# Desktop sessions

The selected library entry has explicit Play and Stop actions. Existing theme,
layout, keyboard focus and controller activation remain shared with the launcher.
F5 starts the selected inspected executable; Shift+F5 stops the current session.
A global Stop action remains available when browsing another title or page.
Selecting or inspecting an entry never starts it automatically.

On Windows x64, Play prepares a bounded plan in a background job, then invokes
only the application-adjacent `pentax-cli.exe` through the safe platform
supervisor. It requests the runtime memory profile, finite long-startup execution
profile and actual `--present-window yes` worker consumer. The CLI's current
long-startup deadline remains 300 seconds; this is an experimental startup session,
not an unlimited-play or title-compatibility promise. Install the desktop, CLI
and worker together. Unsupported hosts keep Play disabled.

Preparation checks the selected executable and only two optional named files:
`sce_module/libc.prx` for explicit preload and
`sce_module/libSceNpCppWebApi.prx` for deferred loading. It does not enumerate the
game directory, read executable payloads or discover arbitrary modules. The
selected executable's containing directory becomes the read-only `/app0` mount.
The worker independently admits the inputs and requested guest file operations.

Session saves and physical backing live below the application configuration
directory, in `runtime/title-<validated-title-id>/save` and `backing`. Entries
without an assigned valid title ID use `local-<full-SHA256>` of the canonical
selected path. These keys are local storage identities, not title discovery or
security secrets. Existing preferences and per-title overrides are never
rewritten by Play. Saved global preferences are forwarded when present; this
does not claim that every graphics or runtime tuning preference is implemented.

Canonical existing ancestors and read-only filesystem identity handles check
source/storage separation, including aliases whose path spelling differs.
Managed runtime/title/save/backing components refuse existing symlinks or Windows
reparse points. Storage is checked again after creation; save and backing cannot
overlap each other or the data root. This is not an atomic defense against a
concurrent external filesystem editor. The worker retains its independent
capability, overlap and backing admission checks. No title file is modified by
preparation.

The UI owns one supervised process and polls fixed output buffers every 33 ms.
The supervisor owns the private kill-on-close job and all descendants. Stop and
launcher close request cancellation and continue nonblocking cleanup; a new
session is refused until the previous job has actually exited and its pipes
have drained. Unresolved shutdown latches RestartRequired even if subsequent
quarantine cleanup observes exit. It does not permit another launch in that
desktop process. There is no shell command, ambient executable lookup, raw
handle adoption, desktop unsafe code or private operational automation.

Only bounded protocol fields map to closed user-facing summaries. Raw stdout,
stderr, filesystem errors, addresses and module diagnostics are not displayed
or logged by this adapter. A live process is reported as an active session;
it is not proof of a guest frame, successful presentation or FPS. Actual window
creation and presentation remain the worker's independently verified consumer
lifecycle.

Original source tests cover launch arguments and exact named module selection,
storage overlap and preserved preference files, stable local keys, and fragmented
or malformed bounded protocol input. They launch no child, UI or game. Execution
results are recorded separately by the coordinating validation.
