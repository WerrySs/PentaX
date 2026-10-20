# Supervised sibling CLI

`SupervisedProcess::spawn_cli` resolves only `pentax-cli.exe` beside the running
application. It quotes a bounded UTF-16 command line and passes an explicit
absolute application name to `CreateProcessW`. It does not invoke a shell, search
PATH, adopt raw handles, or provide a frame transport. Arguments are not retained
after launch and errors contain only closed operation labels and numeric codes.

The root is created suspended. Before resuming it, the owner assigns it to a new
unnamed job with kill-on-close and without either breakaway flag. The job therefore
also owns descendants created by the CLI, including its worker. Only three newly
created standard handles are inherited: read-only NUL input and private stdout
and stderr writers. The attribute list excludes existing application/console
handles. No priority, affinity, memory, pagefile or global host policy changes.

`poll` copies at most 4 KiB from each stream per call. Each stream independently
admits at most 64 KiB in total, or a tighter caller limit. The sole parent reader
uses `PeekNamedPipe` and reads only already buffered bytes. It never waits for
output or process exit. An output-limit error initiates cancellation; shutdown
drains excess bytes into a fixed discard buffer without growing retained output.

`State::Exited` requires all three observations: the actual root process is
signaled, the private job reports zero active processes, and both output pipes
have reached EOF after draining buffered bytes. Exit code 259 is accepted as a
real exit code only after a signaled process observation. A root that exits while
descendants remain initiates termination of its own job. `request_cancel` is
only a checked termination request; it cannot manufacture an exit observation.

The caller must continue `shutdown_poll` on stop/window close. Run time is finite
(default one hour, maximum one day); shutdown grace is at most five seconds.
There is no indefinite join or wait in a pump or destructor. A failed job
assignment retains the exact still-suspended process and initial thread, using
checked `TerminateProcess` rather than claiming the unassigned child belongs to
the job. Any unresolved failure or drop retains one poisoned process/job/pipe
owner. New launches are permanently refused in that application process.
`poll_quarantined_cleanup` uses a try-lock and zero-time polls to release handles
only after actual closure; even successful cleanup does not clear that poison.
The UI must latch the original restart requirement. Process supervision is not
a sandbox and does not establish presentation, game boot, or FPS.

Primary contracts:

- [CreateProcessW](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-createprocessw)
- [Explicit inherited handle lists](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute)
- [Job objects and child inheritance](https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects)
- [Job limits and kill-on-close](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ns-winnt-jobobject_extended_limit_information)
- [PeekNamedPipe](https://learn.microsoft.com/en-us/windows/win32/api/namedpipeapi/nf-namedpipeapi-peeknamedpipe)
- [Zero-time process observation](https://learn.microsoft.com/en-us/windows/win32/api/synchapi/nf-synchapi-waitforsingleobject)

Original source fixtures cover bounded quoting, partial diagnostic capture and
shutdown discard, and real child/descendant job termination with actual EOF.
Compiling or authoring those fixtures is distinct from recording a successful
run; the coordinating validation records which checks actually execute.
