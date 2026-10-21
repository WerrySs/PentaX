<div align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="crates/pentax-desktop/assets/brand/logo-dark.svg">
    <img src="crates/pentax-desktop/assets/brand/logo-light.svg" alt="PentaX" width="144">
  </picture>

  <h1>PentaX</h1>
  <p>An experimental PlayStation 5 emulator written in Rust.</p>

  <p>
    <img src="https://img.shields.io/badge/status-early_development-8B5CF6?style=flat-square" alt="Status: early development">
    <a href="rust-toolchain.toml"><img src="https://img.shields.io/badge/language-Rust-DEA584?style=flat-square" alt="Language: Rust"></a>
    <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--2.0--only-8B5CF6?style=flat-square" alt="License: GPL-2.0-only"></a>
  </p>

  <p>
    <a href="#getting-started">Getting started</a> &nbsp;·&nbsp;
    <a href="#development-status">Status</a> &nbsp;·&nbsp;
    <a href="docs/architecture.md">Architecture</a> &nbsp;·&nbsp;
    <a href="https://github.com/WerrySs/PentaX/issues">Issue tracker</a>
  </p>
</div>

---

PentaX brings together a Rust execution runtime, a Vulkan shader pipeline and a
native desktop interface. Development focuses on checked guest memory, explicit
system-service contracts and reusable runtime components.

> [!IMPORTANT]
> **Commercial games do not boot yet.** The project can exercise native CPU,
> kernel-service and GPU paths through original diagnostics. These results do
> not establish game compatibility, a rendered game frame or an FPS result.

## Development status

| Area | Current scope |
| --- | --- |
| Loading and linking | Checked ELF64/SCE metadata, explicit module sets, supported relocations and static TLS preparation. |
| CPU execution | A bounded Windows x86-64 native worker with mediated instructions, checked memory access and explicit unsupported operations. |
| System services | Stateful kernel/libc subsets, read-only game-data mounts and selected module-initialization paths. |
| Memory | Owned guest mappings, shared physical backing, independent CPU/GPU access rights and bounded resource policies. |
| Graphics | Shader compilation to SPIR-V, guest compute and bounded triangle-list draws with automatic or explicit 16/32-bit indices to linear or RenderTarget64KB RGBA8 targets, with LOAD preservation, fenced readback and atomic guest writeback. |
| Video output | Owned CPU/GPU flip queues, retained flip/vblank events and a supervised Vulkan window with exact presentation acknowledgement. |
| Desktop | A native library with supervised Play/Stop sessions, global/per-title preferences, and keyboard, mouse and controller navigation. |

A verified original 1920×1080 native program draws through Vulkan into a tiled
target, queues its GPU flip and presents one frame through the worker-owned
window. It checks drawn and preserved pixels, padding, guard bytes and exactly
one presentation acknowledgement. Earlier original cases also cover two draws
with changed SGPR inputs and retained LOAD contents. These results use original
programs and shaders. Manual launcher interaction and commercial game boot
remain unverified.

The work ahead includes broader game startup, guest-thread scheduling and
system-service coverage, additional draw/resource modes, sustained presentation
and physical audio.
Performance work needs a
working game path before meaningful frame-rate measurements are possible.

The [architecture and milestones](docs/architecture.md) describe component
boundaries and the evidence behind each stage. The
[issue tracker](https://github.com/WerrySs/PentaX/issues) records active work.

## Getting started

**Windows x86-64 is the locally tested development platform.** Install the Rust
toolchain pinned in [rust-toolchain.toml](rust-toolchain.toml), Visual Studio C++
Build Tools and the Windows SDK. Vulkan diagnostics also need a Vulkan-capable
GPU and driver.

Build the desktop and both sibling executables together, then launch the interface:

```powershell
cargo build --locked --release -p pentax-desktop -p pentax-cli -p pentax-worker
.\target\release\pentax.exe
```

Run the original reference diagnostic or inspect a selected executable:

```powershell
cargo run --locked -p pentax-cli -- self-test
cargo run --locked -p pentax-cli -- inspect "C:\Games\Example\eboot.bin"
```

`inspect` opens the selected executable read-only and reads bounded header data;
it does not execute it. The desktop interface is an early development tool.
Play starts a supervised interactive session with bounded resident resources;
commercial game boot and playability remain unverified. Keep the desktop, CLI
and worker together.
No games, firmware, keys or proprietary SDK files are included.

Native execution currently requires Windows x86-64. Build prerequisites for
other hosts are documented in [CONTRIBUTING.md](CONTRIBUTING.md); previous build
checks do not establish equivalent runtime support. Packaged builds, when
available, list their tested platforms and limitations in the
[release notes](https://github.com/WerrySs/PentaX/releases).

## Technical documentation

| Document | Contents |
| --- | --- |
| [Architecture](docs/architecture.md) | Runtime boundaries, milestones, memory policy and research references. |
| [Native worker](crates/pentax-worker/README.md) | Explicit image probes, resource limits and supported diagnostic paths. |
| [Linker](crates/pentax-linker/README.md) | Metadata validation, provider matching and relocation scope. |
| [Kernel](crates/pentax-kernel/README.md) | Implemented service contracts and remaining limitations. |
| [Shader compiler](crates/pentax-shader-compiler/README.md) | Supported shader subset, SPIR-V resources and host requirements. |
| [Upstream provenance](docs/upstream-provenance.md) | Pinned source revisions, Rust adaptations, modifications and retained notices. |

## Contributing

Keep changes focused and describe the behavior they implement. Contributions
should include the local verification appropriate to the change, preserve
upstream attribution and distinguish original diagnostic results from game
compatibility. Local checks cover runtime behavior; GitHub Actions runs portable
checks and Windows builds using original CPU diagnostics. GPU, desktop interaction
and commercial-title validation require appropriate local hardware and evidence.

Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, review and release practices.
Report vulnerabilities through [SECURITY.md](SECURITY.md).

## License and acknowledgements

PentaX's combined distribution is licensed under [GPL-2.0-only](LICENSE).
File-specific broader grants and dependency licenses remain applicable.

The project includes attributed Rust adaptations of KytyPS5-Performance,
derived from [KytyPS5](https://github.com/KytyPS5/KytyPS5) and
[Kyty](https://github.com/InoriRus/Kyty). Exact source revisions and retained
copyright and license notices are documented in
[upstream provenance](docs/upstream-provenance.md).

PentaX is independent and is not affiliated with or endorsed by Sony Interactive
Entertainment. PlayStation is a trademark of its respective owner.

---

<div align="center">
  <strong>Follow development</strong>
  <p>
    <a href="https://github.com/WerrySs/PentaX/commits"><img src="https://img.shields.io/badge/commits-browse_history-8B5CF6?style=flat-square" alt="Browse commit history"></a>
    <a href="https://github.com/WerrySs/PentaX/pulls"><img src="https://img.shields.io/badge/pull_requests-view_changes-8B5CF6?style=flat-square" alt="View pull requests"></a>
    <a href="https://github.com/WerrySs/PentaX/stargazers"><img src="https://img.shields.io/badge/stars-view_on_GitHub-8B5CF6?style=flat-square" alt="View stars on GitHub"></a>
  </p>
</div>
