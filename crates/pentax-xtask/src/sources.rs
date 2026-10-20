// SPDX-License-Identifier: GPL-2.0-or-later

//! Version-bound repairs for missing upstream notice harvesting. These retain
//! the report's SPDX expressions; supplemental asset notices do not relabel code.
//! See ../notices/README.md for exact source revisions and extraction provenance.

use super::{License, Package, Report, UsedBy};
use sha2::{Digest, Sha256};

mod resources;

struct Source {
    text: &'static str,
    checksum: &'static str,
    url: &'static str,
}

macro_rules! source {
    ($file:literal, $checksum:literal, $url:literal) => {
        Source {
            text: include_str!(concat!("../notices/", $file)),
            checksum: $checksum,
            url: $url,
        }
    };
}

const EGUI: Source = source!(
    "egui-MIT.txt",
    "95ca92f5f8ea5231f1580b3a2a799e8260af3114b900e1def5355a7f44bcf60c",
    "https://raw.githubusercontent.com/emilk/egui/0.37.0/LICENSE-MIT"
);
const OBJC: Source = source!(
    "objc2-MIT.txt",
    "e353f37b12aefbb9f9b29490e837cfee05d9bda70804b3562839a3285c1df1e5",
    "https://raw.githubusercontent.com/madsmtm/objc2/4fc083f1c6d6784577e38b0ee8dbd344481e2fd2/LICENSE.txt"
);
const OBJC_CURRENT: Source = source!(
    "objc2-LICENSE.md",
    "7f976f7e9cb2d87df7230606feb932c3f21ac0e664045a775b600046ff850c54",
    "https://raw.githubusercontent.com/madsmtm/objc2/d7d2fa23ceaa5e6096c923b081040e5d81b3b9df/LICENSE.md"
);
const GILRS: Source = source!(
    "gilrs-MIT.txt",
    "74497fa3c93ebb5ec32d85de117469cb3fab3276f7c02426f14c0f19f31f4424",
    "https://raw.githubusercontent.com/Arvamer/gilrs/07e286e24b046cf39e5c367daa2770b805a64692/LICENSE-MIT"
);
const ICED: Source = source!(
    "iced-x86-MIT.txt",
    "93a47598376fd3f276b631e75149340087153c6ff1635a53c5790249e9124e2d",
    "https://raw.githubusercontent.com/icedland/iced/c50f29b7bc305696895c075f3fc7719751426b12/LICENSE.txt"
);
const SDL: Source = source!(
    "SDL_GameControllerDB-LICENSE.txt",
    "97f35b302b361680ec1e891e95d2d52097bb95abff361434916d99dc1305f127",
    "https://crates.io/crates/gilrs/0.11.2"
);
const UBUNTU_COPYRIGHT: Source = source!(
    "ubuntu-light-copyright.txt",
    "8acacc7c5ad4da2f69c6dda72000cd483c7f1816cd2b20bf15f0da1e89f48b21",
    "https://crates.io/crates/epaint_default_fonts/0.37.0"
);
const DISPATCH_MANIFEST: Source = source!(
    "dispatch-Cargo.toml.txt",
    "05c60b829931ef7f1f130735843c11f5cf9cc69546eca5fc941d014781c68e31",
    "https://raw.githubusercontent.com/SSheldon/rust-dispatch/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml"
);
const SPDX_MIT: Source = source!(
    "SPDX-MIT.txt",
    "b05785f9f18e6716bab63424b11454513b9943a222595b70411009202fc592b5",
    "https://spdx.org/licenses/MIT.html"
);
const PLASMA_MIT: Source = source!(
    "plasma-MIT.txt",
    "8f25018489d6fe0dec34a352314c38dc146247b7de65735790f4398a92afa84b",
    "https://crates.io/crates/wayland-protocols-plasma/0.3.12"
);
const PLASMA_COPYRIGHTS: Source = source!(
    "plasma-copyrights.txt",
    "0a27bdd6a195a9347105af60889f4065062286a5ddfcb9869a0db364281505f4",
    "https://crates.io/crates/wayland-protocols-plasma/0.3.12"
);
const PLASMA_BSD: Source = source!(
    "plasma-BSD-3-Clause.txt",
    "e11af671c491efac2f94c1d752bea92f4308ca2450c7d11e582e4b48c40e4de1",
    "https://crates.io/crates/wayland-protocols-plasma/0.3.12"
);
const PLASMA_CMU: Source = source!(
    "plasma-MIT-CMU.txt",
    "577bcdce29392adc2d1eb2233d999589e2950bd0ea5aa5e6525568794571fead",
    "https://crates.io/crates/wayland-protocols-plasma/0.3.12"
);
const PLASMA_LGPL: Source = source!(
    "plasma-LGPL-2.1-or-later.txt",
    "1ccf09bf2f598308df4bed9cd8e9657dc5cd0973d2800318f2e241486e2edf3f",
    "https://crates.io/crates/wayland-protocols-plasma/0.3.12"
);

impl Source {
    fn checked(&self) -> Result<&'static str, String> {
        let actual = format!("{:x}", Sha256::digest(self.text.as_bytes()));
        if actual != self.checksum {
            return Err(format!("Vendored notice checksum mismatch: {}.", self.url));
        }
        Ok(self.text)
    }

    fn notice(&self) -> Result<String, String> {
        Ok(format!(
            "Source: {}\nSource SHA256: {}\n\n{}",
            self.url,
            self.checksum,
            self.checked()?
        ))
    }
}

pub(super) fn complete_notices(report: &Report) -> Result<Vec<License>, String> {
    let mut licenses = Vec::new();
    for license in &report.licenses {
        if has_placeholder(&license.text) {
            if license.id != "MIT" {
                return Err(format!(
                    "Unreviewed copyright placeholder in {}.",
                    license.id
                ));
            }
            if license.text != SPDX_MIT.checked()? && license.text != PLASMA_MIT.checked()? {
                return Err(
                    "The reviewed MIT template changed; review its complete terms before release."
                        .into(),
                );
            }
            for usage in &license.used_by {
                licenses.push(License {
                    name: license.name.clone(),
                    id: license.id.clone(),
                    text: replace_mit_template(&usage.package)?,
                    used_by: vec![usage.clone()],
                });
            }
        } else {
            licenses.push(license.clone());
        }
    }
    for krate in &report.crates {
        let package = &krate.package;
        match (package.name.as_str(), package.version.as_str()) {
            ("epaint_default_fonts", "0.37.0") => {
                supplement(&mut licenses, package, "MIT", EGUI.notice()?);
                supplement(
                    &mut licenses,
                    package,
                    "Ubuntu-font-1.0",
                    format!(
                        "Original font name-table copyright, accompanying the complete Ubuntu Font Licence in these notices.\nResource: fonts/Ubuntu-Light.ttf\nFull font SHA256: 80307b8da7649aa4ee4d484b232140e3ce1ec0ca093073d3c53c8f5a5ced7a70\nExtraction: nameID 0, platform 3, encoding 1, language 1033; UTF-16BE decoded verbatim.\n\n{}",
                        UBUNTU_COPYRIGHT.notice()?
                    ),
                );
            }
            ("gilrs", "0.11.2") => {
                supplement(&mut licenses, package, "Zlib", SDL.notice()?);
            }
            ("wayland-protocols-plasma", "0.3.12") => {
                let copyright = PLASMA_COPYRIGHTS.notice()?;
                let bsd = PLASMA_BSD
                    .checked()?
                    .strip_prefix("Copyright (c) <year> <owner>. All rights reserved.\n")
                    .ok_or("The reviewed Plasma BSD template changed.")?;
                let cmu = PLASMA_CMU
                    .checked()?
                    .strip_prefix("<copyright notice> ")
                    .ok_or("The reviewed Plasma MIT-CMU template changed.")?;
                supplement(
                    &mut licenses,
                    package,
                    "BSD-3-Clause",
                    format!(
                        "Additional bundled protocol notices; code license expression unchanged.\n\n{copyright}\n\nBSD-3-Clause terms from {} (SHA256 {}):\n{bsd}",
                        PLASMA_BSD.url, PLASMA_BSD.checksum
                    ),
                );
                supplement(
                    &mut licenses,
                    package,
                    "MIT-CMU",
                    format!(
                        "Additional bundled protocol notices; code license expression unchanged.\n\n{copyright}\n\nMIT-CMU terms from {} (SHA256 {}):\n{cmu}",
                        PLASMA_CMU.url, PLASMA_CMU.checksum
                    ),
                );
                supplement(
                    &mut licenses,
                    package,
                    "LGPL-2.1-or-later",
                    format!(
                        "Additional bundled protocol notices; code license expression unchanged.\n\n{copyright}\n\n{}",
                        PLASMA_LGPL.notice()?
                    ),
                );
            }
            ("epaint_default_fonts" | "gilrs" | "wayland-protocols-plasma", _) => {
                return Err(format!(
                    "Review bundled asset notices for {} {} before release.",
                    package.name, package.version
                ));
            }
            _ => {}
        }
    }
    resources::append(report, &mut licenses)?;
    for license in &licenses {
        // The unmodified LGPL application appendix legitimately shows an example
        // copyright field. Exempt only this exact checksum-verified full text;
        // keep checking every surrounding notice and all unknown license text.
        let checked = if license.id == "LGPL-2.1-or-later" {
            license.text.replace(PLASMA_LGPL.checked()?, "")
        } else {
            license.text.clone()
        };
        if has_placeholder(&checked) {
            return Err(format!(
                "Unresolved copyright placeholder in {}.",
                license.id
            ));
        }
    }
    Ok(licenses)
}

fn supplement(licenses: &mut Vec<License>, package: &Package, id: &str, text: String) {
    licenses.push(License {
        name: format!("Additional upstream notice: {id}"),
        id: id.into(),
        text,
        used_by: vec![UsedBy {
            package: package.clone(),
        }],
    });
}

fn replace_mit_template(package: &Package) -> Result<String, String> {
    let old_objc = matches!(
        (package.name.as_str(), package.version.as_str()),
        ("block2", "0.5.1")
            | ("objc-sys", "0.3.5")
            | ("objc2", "0.5.3")
            | ("objc2-app-kit" | "objc2-foundation", "0.2.2")
    );
    let new_objc = matches!(
        (package.name.as_str(), package.version.as_str()),
        ("block2", "0.6.2")
            | ("dispatch2", "0.3.1")
            | ("objc2-encode", "4.1.0")
            | (
                "objc2-app-kit" | "objc2-core-foundation" | "objc2-foundation" | "objc2-io-kit",
                "0.3.2"
            )
            | ("objc2", "0.6.5")
    );
    if old_objc || new_objc {
        let mut text = String::from(
            "The historical complete upstream MIT notice is retained below. Exact published-revision provenance for each component is recorded in the source tree's pentax-xtask/notices/README.md.\n\n",
        );
        text.push_str(&OBJC.notice()?);
        if new_objc {
            text.push_str("\n\nThe newer upstream license document links to MIT rather than repeating its terms:\n\n");
            text.push_str(&OBJC_CURRENT.notice()?);
        }
        return Ok(text);
    }
    match (package.name.as_str(), package.version.as_str()) {
        (
            "ecolor" | "eframe" | "egui-winit" | "egui" | "egui_glow" | "emath" | "epaint",
            "0.37.0",
        ) => EGUI.notice(),
        ("gilrs", "0.11.2") | ("gilrs-core", "0.6.8") => GILRS.notice(),
        ("iced-x86", "1.21.0") => ICED.notice(),
        ("dispatch", "0.2.0") => {
            DISPATCH_MANIFEST.checked()?;
            let terms = SPDX_MIT
                .checked()?
                .replace("Copyright (c) <year> <copyright holders>\n", "");
            Ok(format!(
                "Upstream declares MIT in its published Cargo.toml but supplies no copyright notice or full license file. No holder attribution has been inferred.\nDeclaration: {}\nDeclaration SHA256: {}\nUpstream missing-license issue: https://github.com/SSheldon/rust-dispatch/issues/18\nMIT permission and warranty terms (SPDX template; unfilled copyright field omitted):\n\n{terms}",
                DISPATCH_MANIFEST.url, DISPATCH_MANIFEST.checksum
            ))
        }
        ("wayland-protocols-plasma", "0.3.12") => {
            let terms = PLASMA_MIT
                .checked()?
                .strip_prefix("MIT License Copyright (c) <year> <copyright holders>\n")
                .ok_or("The reviewed Plasma MIT template changed.")?;
            Ok(format!(
                "The protocol source headers supply the copyright notices; the unfilled template field is omitted.\n\n{}\n\nMIT terms from {} (SHA256 {}):\n{terms}",
                PLASMA_COPYRIGHTS.notice()?,
                PLASMA_MIT.url,
                PLASMA_MIT.checksum
            ))
        }
        _ => Err(format!(
            "Unreviewed MIT copyright placeholder for {} {}.",
            package.name, package.version
        )),
    }
}

fn has_placeholder(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "<year>",
        "<copyright holders>",
        "<copyright holder>",
        "<copyright notice>",
        "<owner>",
        "[fullname]",
    ]
    .iter()
    .any(|placeholder| lower.contains(placeholder))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_vendored_sources_and_detects_modified_text() {
        for source in [
            &EGUI,
            &OBJC,
            &OBJC_CURRENT,
            &GILRS,
            &ICED,
            &SDL,
            &UBUNTU_COPYRIGHT,
            &DISPATCH_MANIFEST,
            &SPDX_MIT,
            &PLASMA_MIT,
            &PLASMA_COPYRIGHTS,
            &PLASMA_BSD,
            &PLASMA_CMU,
            &PLASMA_LGPL,
        ] {
            source.checked().unwrap();
        }
        let invalid = Source {
            text: "changed",
            checksum: EGUI.checksum,
            url: EGUI.url,
        };
        assert!(invalid.checked().is_err());
    }

    #[test]
    fn rejects_unknown_or_updated_template_components() {
        let package = Package {
            name: "egui".into(),
            version: "0.38.0".into(),
            repository: None,
        };
        assert!(
            replace_mit_template(&package)
                .unwrap_err()
                .contains("egui 0.38.0")
        );
    }

    #[test]
    fn retains_real_notice_and_does_not_invent_dispatch_copyright() {
        let package = |name: &str| Package {
            name: name.into(),
            version: "0.37.0".into(),
            repository: None,
        };
        let notice = replace_mit_template(&package("egui")).unwrap();
        assert!(notice.contains("Copyright (c) 2018-2021 Emil Ernerfeldt"));
        assert!(!has_placeholder(&notice));
        let dispatch = Package {
            name: "dispatch".into(),
            version: "0.2.0".into(),
            repository: None,
        };
        let notice = replace_mit_template(&dispatch).unwrap();
        assert!(notice.contains("No holder attribution has been inferred"));
        assert!(!notice.contains("Copyright (c) Steven Sheldon"));
        assert!(!has_placeholder(&notice));
    }

    #[test]
    fn retains_exact_iced_notice_and_refuses_an_unreviewed_version() {
        let mut package = Package {
            name: "iced-x86".into(),
            version: "1.21.0".into(),
            repository: None,
        };
        let notice = replace_mit_template(&package).unwrap();
        assert!(notice.ends_with(ICED.checked().unwrap()));
        assert!(notice.contains("Copyright (C) 2018-present iced project and contributors"));
        assert!(!has_placeholder(&notice));
        package.version = "1.22.0".into();
        assert!(replace_mit_template(&package).is_err());
    }

    #[test]
    fn apache_appendix_examples_are_not_copyright_template_notices() {
        assert!(!has_placeholder(
            "Copyright [yyyy] [name of copyright owner]"
        ));
        assert!(has_placeholder("Copyright (c) <year> <owner>"));
    }

    #[test]
    fn refuses_to_replace_a_template_with_unreviewed_additional_terms() {
        let report = Report {
            crates: vec![],
            licenses: vec![License {
                id: "MIT".into(),
                name: "MIT".into(),
                used_by: vec![],
                text: format!("{}\nAdditional unreviewed restriction.", SPDX_MIT.text),
            }],
        };
        assert!(
            complete_notices(&report)
                .err()
                .unwrap()
                .contains("complete terms")
        );
    }
}
