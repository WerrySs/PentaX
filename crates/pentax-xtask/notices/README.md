# Release notice provenance

These files repair missing license harvesting in the locked dependency graph.
The Rust renderer verifies every vendored notice's SHA-256 before using it, binds
repairs to the reviewed package versions, and rejects unreviewed copyright
templates. Upstream SPDX alternatives remain unchanged. Additional resource
notices do not assert a different license for an entire crate.

`about.toml` resolves the full published Microsoft `license-mit`, dpi/libm,
AccessKit and profiling notices directly using checksum-backed clarifications.
AccessKit's upstream MIT file supplies complete terms without a holder line; no
holder name is inferred. The sources below cover the remaining harvesting gaps.

| Notice | Published source and scope |
| --- | --- |
| `egui-MIT.txt` | `emilk/egui` official `0.37.0` release tag, root `LICENSE-MIT`; SHA-256 `95ca92f5f8ea5231f1580b3a2a799e8260af3114b900e1def5355a7f44bcf60c`. It is byte-identical at eframe's published commit `3c6eb8261c3293ad9c7ea1ed58487e038860ebef`. Other egui 0.37.0 packages record commit `7c0b40402271e8d40cda69f87cf3c2617c2e4382`, which was unavailable upstream during review. The version-matching official release tag is used explicitly; the unavailable revision is not silently overridden. This retains Emil Ernerfeldt's actual notice, including for the font crate's own Rust code. |
| `objc2-MIT.txt` | `madsmtm/objc2` root `LICENSE.txt` at published commits `4fc083f1c6d6784577e38b0ee8dbd344481e2fd2` (block2 0.5.1, objc-sys 0.3.5), `5237cbf081d04ef75a1f6a207b4018e7c1ae7438` (objc2 0.5.3), and `e282618be4c3a3b9542957e0c8540e9588472ce8` (framework 0.2.2). All three contain the byte-identical complete Steven Sheldon MIT notice. |
| `objc2-LICENSE.md` | Current published source at `d7d2fa23ceaa5e6096c923b081040e5d81b3b9df` (objc2 0.6.5), `b4167b582b2f75f9a1be75495c41b765344fd03c` (block2 0.6.2), `8852b424193ca41602281b3d7540d7c8ed51e49a` (dispatch2 0.3.1), `8d214f5477365ffcbcbb7de058c86ed9a518efb7` (encode 4.1.0), and `7b1abfd750a2cacaea71d6a56ecfb83cb7de560b` (framework 0.3.2). Every root document is byte-identical. It links to MIT terms and supplies no replacement holder notice. The historical complete MIT notice is retained conservatively beside this current document; it is not a claim that the historical holder owns all later contributions. |
| `gilrs-MIT.txt` | Official GitHub mirror `Arvamer/gilrs`, exact published revision `07e286e24b046cf39e5c367daa2770b805a64692`, root `LICENSE-MIT`; also verified through the official GitLab repository API. Applies to gilrs 0.11.2 and gilrs-core 0.6.8. This avoids cargo-about's unavailable GitLab CDN route. |
| `iced-x86-MIT.txt` | Complete root `LICENSE.txt` from `icedland/iced` commit `c50f29b7bc305696895c075f3fc7719751426b12`, pinned by the published iced-x86 1.21.0 `.cargo_vcs_info.json`. The crate archive omits this file. SHA-256 `93a47598376fd3f276b631e75149340087153c6ff1635a53c5790249e9124e2d` retains the actual iced project and contributors notice and full MIT terms; repair is restricted to that package/version. |
| `SDL_GameControllerDB-LICENSE.txt` | Complete `gilrs 0.11.2/SDL_GameControllerDB/LICENSE` from the published crate. `build.rs` embeds this controller mapping database. The original Sam Lantinga Zlib notice is retained as an additional bundled-data notice. |
| `ubuntu-light-copyright.txt` | Exact copyright string from the bundled Ubuntu-Light.ttf name table, retaining Canonical's actual notice beside the complete UFL terms. `ubuntu-light-copyright-provenance.json` records the original font hash, raw UTF-16BE record location/hash and decoding rule. No holder is inferred from font or crate metadata. |
| `dispatch-Cargo.toml.txt` | Unmodified published `dispatch 0.2.0/Cargo.toml.orig`, byte-identical to `SSheldon/rust-dispatch` commit `82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml`. It declares MIT; that revision and the release tags contain no full license or holder notice. Upstream records the omission in [issue 18](https://github.com/SSheldon/rust-dispatch/issues/18). No holder attribution is fabricated. |
| `SPDX-MIT.txt` | Standard MIT terms emitted by cargo-about 0.9.2's SPDX fallback. For dispatch only, the unfilled copyright field is removed and the absence of an upstream holder notice is stated. Every permission and warranty term is retained. |
| `plasma-*.txt` | Published `wayland-protocols-plasma 0.3.12`, repository revision `a3d7927d87799b2955bf491b51c7c2a3a82da661`. License files are unmodified copies of `plasma-wayland-protocols/LICENSES/{MIT,MIT-CMU,BSD-3-Clause,LGPL-2.1-or-later}.txt`. `plasma-copyrights.txt` copies every XML `<copyright>` block verbatim after removing only the XML/CDATA wrapper and outer whitespace, with its original filename and complete-source SHA-256. The crate's modules reference these XML protocols for generated bindings. Scope of derivation is not inferred solely from source inclusion; the original headers and complete terms are retained conservatively as additional resource notices. The MIT, BSD and MIT-CMU unfilled template fields are replaced by those actual source notices; all license terms remain. |

`khronos-resource-provenance.json` records exact upstream URLs, revisions, full
source hashes, extracted notice hashes and resource paths for the seven source
headers/notices alongside it. Packaged Khronos XML/IDL sources match those
upstream bytes; `khronos_api` includes them with `include_bytes!`. ANGLE's BSD
license and Khronos's MIT-style resource terms are retained in full. Original
Apache source headers supplement the separately included complete Apache terms;
none of the reviewed pinned AccessKit or gl-rs trees contains a separate NOTICE
file.

`accesskit-source-provenance.json` records 13 original leading source-header
variants across 64 files in the six pinned AccessKit packages, with exact source
URLs and hashes. The headers retain the actual AccessKit, Chromium, kurbo and
Zeeshan Ali Khan notices. The complete upstream `LICENSE.chromium` is included
beside those source-specific Chromium dates. Source headers supplement the
separately retained complete MIT and Apache terms; the crates' declared license
alternatives remain intact.

The complete original dependency sources are retrievable from the versions and
checksums in `Cargo.lock`, using `cargo fetch --locked` for the three release
targets described in CONTRIBUTING.md. These provenance files, renderer and
configuration are included in the versioned PentaX source. Keep them together
when updating a dependency; a version update must review its asset notices too.
