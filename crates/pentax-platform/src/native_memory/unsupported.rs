// SPDX-License-Identifier: GPL-2.0-only
use super::*;
#[derive(Debug)]
pub struct NativeFileBacking {
    unavailable: std::convert::Infallible,
}
impl NativeFileBacking {
    pub fn open(_: &std::path::Path, _: u64) -> Result<Self, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub const fn capacity_bytes(&self) -> u64 {
        match self.unavailable {}
    }
}
#[derive(Debug)]
pub struct NativeAddressSpace {
    unavailable: std::convert::Infallible,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
pub struct PausedNativeMemory<'a> {
    _owner: &'a mut NativeAddressSpace,
}
impl NativeAddressSpace {
    pub fn compare_replace_words(
        &mut self,
        _: &[NativeWordReplacement],
        _: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn statistics(&self) -> NativeMemoryStatistics {
        NativeMemoryStatistics {
            mapped_bytes: 0,
            committed_bytes: 0,
            backing_bytes: 0,
            file_backed_bytes: 0,
            private_commit_limit_bytes: 0,
            host_failure: None,
            reserved_bytes: 0,
            limits: NativeMemoryLimits {
                mapped_bytes: 0,
                reserved_bytes: 0,
            },
        }
    }
    /// Unsupported hosts cannot construct an owner or fabricate its identity.
    pub const fn owner_id(&self) -> NativeOwnerId {
        match self.unavailable {}
    }
    pub fn restrict_limits(&mut self, _: NativeMemoryLimits) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn defer_physical_commit(&mut self) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn use_file_backing(
        &mut self,
        _: NativeFileBacking,
        _: u64,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub const fn backing_bytes(&self) -> u64 {
        0
    }
    pub fn new(_: NativeMemoryLimits) -> Result<Self, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn reserve_any(_: NativeMemoryLimits, _: u64) -> Result<Self, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn reserve_within(
        _: NativeMemoryLimits,
        _: u64,
        _: NativeRange,
    ) -> Result<Self, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn reserved_ranges(&self) -> impl Iterator<Item = NativeRange> {
        std::iter::empty()
    }
    pub const fn mapped_bytes(&self) -> u64 {
        0
    }
    pub const fn committed_bytes(&self) -> u64 {
        0
    }
    pub const fn reserved_bytes(&self) -> u64 {
        0
    }
    pub const fn allocation_granularity(&self) -> u64 {
        0
    }
    pub fn map_at(
        &mut self,
        _: u64,
        _: u64,
        _: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn protect(
        &mut self,
        _: u64,
        _: u64,
        _: NativePermissions,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn pause(&mut self) -> Result<PausedNativeMemory<'_>, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn unmap(&mut self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn abandon(&mut self) {
        match self.unavailable {}
    }
}
impl PausedNativeMemory<'_> {
    pub fn preflight_mapped(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn set_virtual_range_name(
        &mut self,
        _: u64,
        _: u64,
        _: &[u8],
        _: &NativeNameLimits,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn virtual_range_name(
        &mut self,
        _: u64,
        _: NativeRange,
        _: &[u8],
        _: &NativeNameLimits,
    ) -> Result<NativeRangeName, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_gpu_read(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_gpu_write(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn read_gpu(&self, _: u64, _: &mut [u8]) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn snapshot_gpu_buffer(
        &self,
        _: u64,
        _: &mut [u8],
        _: bool,
        _: bool,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn apply_gpu_writes(
        &mut self,
        _: &[NativeMemoryWrite<'_>],
        _: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_disjoint_backing(
        &self,
        _: NativeRange,
        _: NativeRange,
        _: u64,
    ) -> Result<u64, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_distinct_backing(
        &self,
        _: u64,
        _: u64,
        _: u64,
    ) -> Result<u64, NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub const fn owner_id(&self) -> NativeOwnerId {
        self._owner.owner_id()
    }
    pub fn preflight_fetch_source(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_non_executable(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_reserved_unmapped(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn apply_transaction(
        &mut self,
        _: NativeMemoryTransaction<'_>,
        _: &NativeTransactionLimits,
    ) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_read(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn preflight_write(&self, _: u64, _: u64) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn read(&self, _: u64, _: &mut [u8]) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn fetch_source(&self, _: u64, _: &mut [u8]) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
    pub fn write(&mut self, _: u64, _: &[u8]) -> Result<(), NativeMemoryError> {
        Err(NativeMemoryError::UnsupportedHost)
    }
}
