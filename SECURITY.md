# Security

PentaX is experimental. Security fixes target `main` and the most recent release;
older snapshots are not maintained. Guest executable files are untrusted input.

Report suspected vulnerabilities privately to [@WerrySs](https://github.com/WerrySs)
through an existing private channel. Include the affected commit or version,
platform, impact, and minimal reproduction steps using an original fixture. Avoid
public disclosure of an unpatched vulnerability and do not attach firmware,
keys, game content, or private memory dumps.

Local release checks audit the locked dependency graph against the current
RustSec database. Dependabot opens dependency update pull requests. GitHub
Actions runs bounded build and original contract checks using standard hosted
runners with read-only repository permissions. CI, local checks and review do
not establish that guest execution or a release is free
of vulnerabilities.
