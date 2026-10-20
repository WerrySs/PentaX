// SPDX-License-Identifier: GPL-2.0-or-later

//! Render cargo-about 0.9.2 JSON without distributing host paths or Cargo metadata.

use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::process::ExitCode;

mod sources;

const MAX_REPORT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Deserialize)]
struct Report {
    licenses: Vec<License>,
    crates: Vec<CrateLicense>,
}

#[derive(Clone, Deserialize)]
struct License {
    name: String,
    id: String,
    text: String,
    used_by: Vec<UsedBy>,
}

#[derive(Clone, Deserialize)]
struct UsedBy {
    #[serde(rename = "crate")]
    package: Package,
}

#[derive(Deserialize)]
struct CrateLicense {
    package: Package,
    license: String,
}

#[derive(Clone, Deserialize)]
struct Package {
    name: String,
    version: String,
    repository: Option<String>,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Notice generation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if arguments.len() != 3 || arguments[0] != "notices" {
        return Err(
            "Usage: pentax-xtask notices <cargo-about-report.json> <THIRD-PARTY-NOTICES.txt>"
                .into(),
        );
    }
    let mut bytes = Vec::new();
    File::open(&arguments[1])
        .map_err(|error| error.to_string())?
        .take(MAX_REPORT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_REPORT_BYTES {
        return Err("The license report exceeds the 64 MiB limit.".into());
    }
    let report: Report = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Invalid cargo-about report: {error}"))?;
    let notices = render(&report)?;
    let output = Path::new(&arguments[2]);
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(output, notices).map_err(|error| error.to_string())?;
    Ok(())
}

fn render(report: &Report) -> Result<String, String> {
    if report.crates.is_empty() || report.licenses.is_empty() {
        return Err("The license report is empty.".into());
    }
    // Supplemental asset notices must never mask an absent original code notice.
    let original_coverage: BTreeSet<_> = report
        .licenses
        .iter()
        .flat_map(|license| {
            license
                .used_by
                .iter()
                .map(|usage| (usage.package.name.as_str(), usage.package.version.as_str()))
        })
        .collect();
    for krate in &report.crates {
        if !original_coverage
            .contains(&(krate.package.name.as_str(), krate.package.version.as_str()))
        {
            return Err(format!(
                "Missing original resolved license text for {} {}.",
                krate.package.name, krate.package.version
            ));
        }
    }
    let licenses = sources::complete_notices(report)?;
    let mut covered = BTreeSet::new();
    for license in &licenses {
        if license.id.trim().is_empty()
            || license.name.trim().is_empty()
            || license.text.trim().is_empty()
            || license.used_by.is_empty()
        {
            return Err(
                "A license section has missing identifiers, text, or component attribution.".into(),
            );
        }
        for usage in &license.used_by {
            validate_package(&usage.package)?;
            covered.insert((usage.package.name.as_str(), usage.package.version.as_str()));
        }
    }
    for krate in &report.crates {
        validate_package(&krate.package)?;
        if krate.license.trim().is_empty()
            || matches!(krate.license.as_str(), "Unknown" | "Ignore")
            || !covered.contains(&(krate.package.name.as_str(), krate.package.version.as_str()))
        {
            return Err(format!(
                "Missing resolved license text for {} {}.",
                krate.package.name, krate.package.version
            ));
        }
    }
    validate_bundled_fonts(&licenses)?;
    let mut sections: Vec<_> = licenses.iter().collect();
    sections.sort_by(|left, right| (&left.id, &left.text).cmp(&(&right.id, &right.text)));
    let mut output = String::from(
        "PentaX - Third-party notices\n\nGenerated from the locked dependency graph with cargo-about 0.9.2.\nCovers Windows x64, Linux x64, and macOS Apple silicon dependencies.\nDependencies and embedded fonts retain the licenses and notices below.\nPentaX's own license is provided separately in LICENSE.\n",
    );
    output.push_str("\n================================================================\nAdapted Kyty components\n================================================================\n\n");
    output.push_str(include_str!("../../../docs/upstream-provenance.md"));
    output.push_str("\n\nRetained original Kyty MIT notice:\n\n");
    output.push_str(include_str!("../../../LICENSES/Kyty-MIT.txt"));
    output.push_str("\n\nKernel port provenance:\n\n");
    output.push_str(include_str!("../../pentax-kernel/NOTICE"));
    output.push_str("\n\nCore graphics submit provenance:\n\n");
    output.push_str(include_str!("../../pentax-core/NOTICE"));
    output.push_str("\n\nGPU compute preparation provenance:\n\n");
    output.push_str(include_str!("../../pentax-gpu/NOTICE"));
    output.push_str("\n\nShader compiler provenance:\n\n");
    output.push_str(include_str!("../../pentax-shader-compiler/NOTICE"));
    output.push_str("\n\nRetained Khronos SPIR-V header terms:\n\n");
    output.push_str(include_str!("../../pentax-shader-compiler/Khronos-MIT.txt"));
    output.push_str("\n\nVulkan backend provenance:\n\n");
    output.push_str(include_str!("../../pentax-platform/src/vulkan/NOTICE"));
    writeln!(output, "\nComponents covered: {}\n", report.crates.len())
        .map_err(|error| error.to_string())?;
    for license in sections {
        writeln!(output, "================================================================\n{} ({})\n================================================================\n\nUsed by:", license.name, license.id).map_err(|error| error.to_string())?;
        let components: BTreeSet<_> = license
            .used_by
            .iter()
            .map(|usage| {
                (
                    &usage.package.name,
                    &usage.package.version,
                    &usage.package.repository,
                )
            })
            .collect();
        for (name, version, repository) in components {
            write!(output, "- {name} {version}").map_err(|error| error.to_string())?;
            if let Some(repository) = repository {
                write!(output, " - {repository}").map_err(|error| error.to_string())?;
            }
            output.push('\n');
        }
        output.push('\n');
        output.push_str(&license.text);
        output.push_str("\n\n");
    }
    Ok(output)
}

fn validate_package(package: &Package) -> Result<(), String> {
    if package.name.is_empty()
        || package.version.is_empty()
        || package.name.chars().any(char::is_control)
        || package.version.chars().any(char::is_control)
        || package
            .repository
            .as_ref()
            .is_some_and(|value| value.chars().any(char::is_control))
    {
        return Err("A component has missing or invalid attribution metadata.".into());
    }
    Ok(())
}

fn validate_bundled_fonts(licenses: &[License]) -> Result<(), String> {
    let sections: Vec<_> = licenses
        .iter()
        .filter(|license| {
            license
                .used_by
                .iter()
                .any(|usage| usage.package.name == "epaint_default_fonts")
        })
        .collect();
    for required in ["OFL-1.1", "Ubuntu-font-1.0"] {
        if !sections.iter().any(|license| license.id == required) {
            return Err(format!("The bundled-font report is missing {required}."));
        }
    }
    if !sections
        .iter()
        .any(|license| matches!(license.id.as_str(), "MIT" | "Apache-2.0"))
    {
        return Err("The bundled-font report is missing its code license.".into());
    }
    for required in ["Bitstream", "Source Foundry"] {
        if !sections
            .iter()
            .any(|license| license.text.contains(required))
        {
            return Err(format!(
                "The bundled-font report is missing the {required} attribution from Hack-Regular.txt."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let package = || Package {
            name: "epaint_default_fonts".into(),
            version: "0.37.0".into(),
            repository: Some("https://github.com/emilk/egui".into()),
        };
        Report {
            licenses: ["MIT", "OFL-1.1", "Ubuntu-font-1.0"]
                .into_iter()
                .map(|id| License {
                    name: id.into(),
                    id: id.into(),
                    text: "Synthetic fixture: Source Foundry and Bitstream attribution.".into(),
                    used_by: vec![UsedBy { package: package() }],
                })
                .collect(),
            crates: vec![CrateLicense {
                package: package(),
                license: "MIT AND OFL-1.1 AND Ubuntu-font-1.0".into(),
            }],
        }
    }

    #[test]
    fn retains_all_license_texts_and_omits_unselected_cargo_metadata() {
        let licenses = ["MIT", "OFL-1.1", "Ubuntu-font-1.0"].map(|id| serde_json::json!({ "id": id, "name": id, "text": format!("{id} synthetic license text with Source Foundry and Bitstream."), "source_path": "C:/private/registry/LICENSE", "used_by": [{ "crate": { "name": "epaint_default_fonts", "version": "0.37.0", "repository": "https://github.com/emilk/egui", "manifest_path": "C:/private/registry/Cargo.toml" } }] }));
        let json = serde_json::json!({
            "crates": [{ "package": { "name": "epaint_default_fonts", "version": "0.37.0", "repository": "https://github.com/emilk/egui", "manifest_path": "C:/private/workspace/Cargo.toml" }, "license": "MIT AND OFL-1.1 AND Ubuntu-font-1.0" }],
            "licenses": licenses
        });
        let report: Report = serde_json::from_value(json).unwrap();
        let output = render(&report).unwrap();
        assert!(output.contains("epaint_default_fonts 0.37.0"));
        assert!(output.contains("https://github.com/emilk/egui"));
        assert!(!output.contains("C:/private"));
        assert!(output.contains(include_str!("../../../LICENSES/Kyty-MIT.txt")));
        assert!(output.contains(include_str!("../../../docs/upstream-provenance.md")));
        for license in report.licenses {
            assert!(output.contains(&license.text));
        }
    }

    #[test]
    fn refuses_a_report_missing_required_font_notices() {
        let mut report = report();
        report.licenses.retain(|license| license.id != "OFL-1.1");
        assert!(render(&report).unwrap_err().contains("OFL-1.1"));
    }

    #[test]
    fn refuses_an_unresolved_component_without_silently_dropping_it() {
        let mut report = report();
        report.crates.push(CrateLicense {
            package: Package {
                name: "unresolved".into(),
                version: "1.0.0".into(),
                repository: None,
            },
            license: "Unknown".into(),
        });
        assert!(render(&report).unwrap_err().contains("unresolved 1.0.0"));
    }

    #[test]
    fn repairs_reviewed_missing_notices_and_blocks_an_unreviewed_update() {
        let mut report = report();
        let package = Package {
            name: "egui".into(),
            version: "0.37.0".into(),
            repository: Some("https://github.com/emilk/egui".into()),
        };
        report.licenses.push(License {
            name: "MIT License".into(),
            id: "MIT".into(),
            text: include_str!("../notices/SPDX-MIT.txt").into(),
            used_by: vec![UsedBy {
                package: package.clone(),
            }],
        });
        report.crates.push(CrateLicense {
            package,
            license: "MIT OR Apache-2.0".into(),
        });
        let output = render(&report).unwrap();
        assert!(output.contains("Copyright (c) 2018-2021 Emil Ernerfeldt"));
        assert!(!output.contains("<copyright holders>"));
        report.licenses.last_mut().unwrap().used_by[0]
            .package
            .version = "0.38.0".into();
        report.crates.last_mut().unwrap().package.version = "0.38.0".into();
        assert!(render(&report).unwrap_err().contains("egui 0.38.0"));
    }

    #[test]
    fn supplemental_asset_notice_cannot_mask_a_missing_original_code_notice() {
        let mut report = report();
        report.crates.push(CrateLicense {
            package: Package {
                name: "gilrs".into(),
                version: "0.11.2".into(),
                repository: None,
            },
            license: "MIT OR Apache-2.0".into(),
        });
        assert!(
            render(&report)
                .unwrap_err()
                .contains("Missing original resolved license text for gilrs")
        );
    }
}
