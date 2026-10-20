// SPDX-License-Identifier: GPL-2.0-only

//! Pin explicit backing authority outside read-only data and save directories.
use super::*;
use std::path::PathBuf;

pub(super) struct BackingDirectory {
    pub(super) path: PathBuf,
    _pin: File,
}
impl BackingDirectory {
    pub(super) fn open(args: &[OsString], requested: &OsString) -> Result<Self, ProbeError> {
        let path = std::fs::canonicalize(requested)
            .map_err(|_| ProbeError::Input("backing-directory-path"))?;
        for pair in args.chunks_exact(2) {
            let directory = pair[0] == "--data-directory" || pair[0] == "--save-directory";
            let bound = pair[0] == "--data-directory"
                || pair[0] == "--data-file"
                || pair[0] == "--module-file"
                || pair[0] == "--preload-libc-as";
            let selected;
            let host = if bound {
                let (_, host) = pair[1]
                    .to_str()
                    .and_then(|v| v.split_once('='))
                    .filter(|(_, host)| !host.is_empty())
                    .ok_or(ProbeError::Input("backing-input-binding"))?;
                selected = OsString::from(host);
                &selected
            } else if pair[0] == "--save-directory"
                || pair[0] == "--image"
                || pair[0] == "--preload-libc"
                || pair[0] == "--config"
            {
                &pair[1]
            } else {
                continue;
            };
            let other =
                std::fs::canonicalize(host).map_err(|_| ProbeError::Input("backing-input-path"))?;
            if other.starts_with(&path) || (directory && path.starts_with(&other)) {
                return Err(ProbeError::Input("backing-input-overlap"));
            }
        }
        let pin = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0200_0000)
            .open(&path)
            .map_err(|_| ProbeError::Input("backing-directory-open"))?;
        if !pin
            .metadata()
            .map_err(|_| ProbeError::Input("backing-directory-metadata"))?
            .is_dir()
        {
            return Err(ProbeError::Input("backing-directory-not-directory"));
        }
        Ok(Self { path, _pin: pin })
    }
}
