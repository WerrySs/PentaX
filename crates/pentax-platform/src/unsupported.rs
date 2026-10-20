// SPDX-License-Identifier: GPL-2.0-or-later

use crate::{HostLayout, OriginalFixtureResult, PlatformError};

pub(super) fn execute_trapped_sum42() -> Result<OriginalFixtureResult, PlatformError> {
    Err(PlatformError::UnsupportedHost)
}

pub(super) fn execute_guest_stack_entry42() -> Result<OriginalFixtureResult, PlatformError> {
    Err(PlatformError::UnsupportedHost)
}

pub(super) fn execute_native_block_dispatch() -> Result<OriginalFixtureResult, PlatformError> {
    Err(PlatformError::UnsupportedHost)
}

pub(super) fn host_layout() -> Result<HostLayout, PlatformError> {
    Err(PlatformError::UnsupportedHost)
}

pub(super) fn configure_worker_fault_policy() -> Result<(), PlatformError> {
    Err(PlatformError::UnsupportedHost)
}

// No value of this type exists on unsupported hosts. This keeps the complete
// workspace buildable without claiming that a native x86 guest backend exists.
pub(super) enum Region {}

impl Region {
    pub(super) fn reserve(_length: usize) -> Result<Self, PlatformError> {
        Err(PlatformError::UnsupportedHost)
    }
    pub(super) fn len(&self) -> usize {
        match *self {}
    }
    pub(super) fn address(&self) -> u64 {
        match *self {}
    }
    pub(super) fn commit_writable(&mut self) -> Result<(), PlatformError> {
        match *self {}
    }
    pub(super) fn write(&mut self, _offset: usize, _bytes: &[u8]) -> Result<(), PlatformError> {
        match *self {}
    }
    pub(super) fn seal_executable(&mut self) -> Result<(), PlatformError> {
        match *self {}
    }
    pub(super) unsafe fn call_original_fixture(&self) -> u64 {
        match *self {}
    }
}
