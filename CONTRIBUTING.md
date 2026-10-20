# Contributing to PentaX

PentaX is an experimental PS5 emulator written in Rust. The current reference
guest is an original diagnostic fixture; passing it does not establish commercial
game compatibility. Keep changes focused on behavior that can be inspected and
verified.

## Development

Use Rust 1.97.1 with the MSVC toolchain on Windows, Xcode Command Line Tools on
macOS, or a native Linux compiler toolchain. The pinned toolchain includes
`rustfmt` and Clippy. On Ubuntu 24.04, install the desktop build dependencies:

```sh
sudo apt-get install pkg-config libudev-dev libxkbcommon-dev libwayland-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev libgl1-mesa-dev
```

Run these checks locally before requesting review. GitHub Actions covers portable
library checks, strict complete-workspace Windows analysis, Windows builds and
selected original CPU contracts. Native dispatch and callback execution currently
target Windows x64; their other-platform stubs are not the Linux strict-analysis
target. GPU execution,
desktop interaction and commercial-title behavior need separate local evidence:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
cargo run -p pentax-cli --locked -- self-test
cargo run -p pentax-cli --locked -- self-test-startup
cargo run -p pentax-cli --locked -- self-test-gpu
cargo run -p pentax-cli --locked -- self-test-hle
cargo run -p pentax-cli --locked -- self-test-shader
```

Use the complete-workspace commands above on Windows x64. The portable CI subset
on Linux is:

```sh
cargo clippy --locked -p pentax-loader -p pentax-linker -p pentax-codegen -p pentax-shader -p pentax-shader-compiler -p pentax-gpu -p pentax-config -p pentax-kernel --all-targets -- -D warnings
cargo test --locked -p pentax-loader -p pentax-linker -p pentax-codegen -p pentax-shader -p pentax-shader-compiler -p pentax-gpu -p pentax-config -p pentax-kernel --lib
```

For dependency changes, commit the updated `Cargo.lock` and run
`cargo audit --file Cargo.lock` using cargo-audit 0.22.2. Review new advisories and
maintenance warnings; do not hide them with blanket exclusions.

Dependency changes must also pass local license verification for the supported
dependency targets. Use cargo-about 0.9.2 and `about.toml`, then the Rust notice renderer:

```sh
cargo install cargo-about --version 0.9.2 --locked --features cli
mkdir -p target
cargo about generate --workspace --locked --format json --fail --output-file target/third-party-report.json
cargo run -p pentax-xtask --locked -- notices target/third-party-report.json target/THIRD-PARTY-NOTICES.txt
```

Do not suppress unresolved licenses. Any clarification must retain the source
terms and be backed by the published notice files and their SHA-256 checksums.
The Rust renderer also rejects unreviewed copyright templates and verifies the
version-bound original notices in `crates/pentax-xtask/notices`. See that
directory's provenance README when updating dependencies or bundled assets.
To retrieve the exact original dependency sources, run
`cargo fetch --locked --target x86_64-pc-windows-msvc --target x86_64-unknown-linux-gnu --target aarch64-apple-darwin`;
versions and archive checksums are recorded in Cargo.lock.

## Pull requests

Target `main` from a focused branch. Explain the trigger, resulting behavior, and
actual local validation results and the checked source commit. State which
platforms you exercised; compilation does not establish that a desktop or
controller interaction works.

Keep loader, guest memory, CPU, configuration, and frontend responsibilities
separate. Add original, minimal fixtures for guest behavior. Include boundary and
failure cases when changing executable parsing or guest memory. Repository Rust
in the portable crates forbids `unsafe`; changes to the native platform boundary
require a focused safety review and platform tests. Host memory protections
must not depend on a user setting.

Use public references with attribution and a compatible license. Do not commit
console firmware, proprietary SDK files, keys, commercial game data, personal
paths, or private dumps. Describe guest incompatibilities precisely instead of
adding success placeholders. Contributions are licensed under the repository's
GPL-2.0-only distribution license. Preserve file-specific grants and the
source, copyright, permission and modification notices of adapted components;
see [upstream provenance](docs/upstream-provenance.md).

## Releases

Version tags must match the workspace version, for example `v0.1.0`. Run the
development checks, current dependency audit and strict license verification
locally against the exact release commit. Build only packages that can be
verified on the available hosts; do not reuse another version's binaries or
claim untested platforms. For Windows x64:

```sh
cargo build --release --locked --target x86_64-pc-windows-msvc -p pentax-desktop -p pentax-cli -p pentax-worker --bins
```

Run each packaged CLI's version, reference, startup, HLE and GPU/shader syntax checks, plus
the worker fixtures, from the extracted archive. Verify explicit syscall
rejection before execution. Archives
contain the launcher, CLI, isolated worker, `LICENSE`, `THIRD-PARTY-NOTICES.txt`,
and the pinned toolchain's unmodified `RUST-STANDARD-LIBRARY-NOTICES.html`, with
SHA256 checksums and a link to the corresponding source. Generate and verify
notices locally before archiving, then validate the archive's inventory and
checksums before extracting or publishing it.

Create a draft prerelease after these checks. Before publication, the maintainer records
desktop startup, keyboard/mouse and controller checks on each platform claimed as
tested, and writes release notes describing remaining limitations. Build success
alone must not be described as game compatibility or support for other devices.
