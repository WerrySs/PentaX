// SPDX-License-Identifier: GPL-2.0-or-later

//! Original resource and source headers supplement the separately resolved
//! crate licenses. No inference from Cargo package authors is made.

use super::{License, Package, Report, supplement};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Provenance {
    records: Vec<Record>,
}

#[derive(Deserialize)]
struct Record {
    file: String,
    sha256: String,
    source_url: String,
    source_sha256: String,
    extraction: String,
    packages: Vec<Scope>,
}

#[derive(Deserialize)]
struct Scope {
    name: String,
    version: String,
    resources: Vec<String>,
}

pub(super) fn append(report: &Report, licenses: &mut Vec<License>) -> Result<(), String> {
    let provenance: Provenance = serde_json::from_str(include_str!(
        "../../notices/khronos-resource-provenance.json"
    ))
    .map_err(|error| format!("Invalid resource notice provenance: {error}"))?;
    let accesskit: Provenance = serde_json::from_str(include_str!(
        "../../notices/accesskit-source-provenance.json"
    ))
    .map_err(|error| format!("Invalid AccessKit notice provenance: {error}"))?;
    for record in provenance.records.into_iter().chain(accesskit.records) {
        let (id, text) = match record.file.as_str() {
            "khronos-egl-mit-header.txt" => (
                "MIT",
                include_str!("../../notices/khronos-egl-mit-header.txt"),
            ),
            "khronos-webgl-mit-header.txt" => (
                "MIT",
                include_str!("../../notices/khronos-webgl-mit-header.txt"),
            ),
            "khronos-opengl-apache-header.txt" => (
                "Apache-2.0",
                include_str!("../../notices/khronos-opengl-apache-header.txt"),
            ),
            "angle-bsd-license.txt" => (
                "BSD-3-Clause",
                include_str!("../../notices/angle-bsd-license.txt"),
            ),
            "accesskit-winit-apache-header.txt" => (
                "Apache-2.0",
                include_str!("../../notices/accesskit-winit-apache-header.txt"),
            ),
            "gl-rs-2015-apache-header.txt" => (
                "Apache-2.0",
                include_str!("../../notices/gl-rs-2015-apache-header.txt"),
            ),
            "gl-rs-2015-2016-apache-header.txt" => (
                "Apache-2.0",
                include_str!("../../notices/gl-rs-2015-2016-apache-header.txt"),
            ),
            "accesskit-source-header-01.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-01.txt"),
            ),
            "accesskit-source-header-02.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-02.txt"),
            ),
            "accesskit-source-header-03.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-03.txt"),
            ),
            "accesskit-source-header-04.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-04.txt"),
            ),
            "accesskit-source-header-05.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-05.txt"),
            ),
            "accesskit-source-header-06.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-06.txt"),
            ),
            "accesskit-source-header-07.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-07.txt"),
            ),
            "accesskit-source-header-08.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-08.txt"),
            ),
            "accesskit-source-header-09.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-09.txt"),
            ),
            "accesskit-source-header-10.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-10.txt"),
            ),
            "accesskit-source-header-11.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-11.txt"),
            ),
            "accesskit-source-header-12.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-12.txt"),
            ),
            "accesskit-source-header-13.txt" => (
                "MIT",
                include_str!("../../notices/accesskit-source-header-13.txt"),
            ),
            "accesskit-chromium-BSD.txt" => (
                "BSD-3-Clause",
                include_str!("../../notices/accesskit-chromium-BSD.txt"),
            ),
            _ => return Err(format!("Unreviewed resource notice: {}.", record.file)),
        };
        if format!("{:x}", Sha256::digest(text.as_bytes())) != record.sha256 {
            return Err(format!(
                "Resource notice checksum mismatch: {}.",
                record.file
            ));
        }
        for scope in &record.packages {
            for krate in report
                .crates
                .iter()
                .filter(|krate| krate.package.name == scope.name)
            {
                let package: &Package = &krate.package;
                if package.version != scope.version {
                    return Err(format!(
                        "Review bundled resource notices for {} {} before release.",
                        package.name, package.version
                    ));
                }
                supplement(
                    licenses,
                    package,
                    id,
                    format!(
                        "Additional original resource/source notice; crate license expression unchanged.\nResource scope: {}\nSource: {}\nFull source SHA256: {}\nExtraction: {}\nExtracted notice SHA256: {}\n\n{text}",
                        scope.resources.join(", "),
                        record.source_url,
                        record.source_sha256,
                        record.extraction,
                        record.sha256,
                    ),
                );
            }
        }
    }
    Ok(())
}
