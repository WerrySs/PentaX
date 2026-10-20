// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: retained common-dialog base state; see NOTICE.
// Adapted from pinned src/libs/dialog.cpp and libDialog.cpp.

use crate::{Failure, Service};

/// Base initialization is not evidence of a displayed or completed dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommonDialogStatus {
    pub initialized: bool,
    pub in_use: bool,
}

#[derive(Default)]
pub(crate) struct DialogState {
    status: CommonDialogStatus,
}
impl DialogState {
    pub(crate) const fn status(&self) -> CommonDialogStatus {
        self.status
    }
    pub(crate) fn invoke(&mut self, service: Service) -> Result<u64, Failure> {
        match service {
            Service::CommonDialogInitialize => {
                // Pinned initialization is idempotent. It neither opens a panel
                // nor resets a future active dialog. Retain its actual receipt.
                self.status.initialized = true;
                Ok(0)
            }
            // No dialog-open service is admitted by this profile. A later
            // implementation must update this same process-owned active state.
            Service::CommonDialogIsUsed => Ok(u64::from(self.status.in_use)),
            _ => Err(Failure::Unsupported("common dialog service")),
        }
    }
}

impl crate::Kernel {
    #[must_use]
    pub const fn common_dialog_status(&self) -> CommonDialogStatus {
        self.dialogs.status()
    }
}
