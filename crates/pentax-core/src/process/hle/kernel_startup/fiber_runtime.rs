// SPDX-License-Identifier: GPL-2.0-only
//! Source-bound fiber actions. Pure service tickets never prove CPU exchange.
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod native;
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use native::{FiberEntryLayout, PreparedFiberAction};
#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub(super) use native::{FiberRuntime, prepare};
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
#[derive(Debug)]
pub struct PreparedFiberAction {
    _private: (),
}
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub(super) struct FiberRuntime;
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
impl FiberRuntime {
    pub(super) fn new(_: usize, _: usize) -> Result<Self, super::KernelStartupError> {
        Ok(Self)
    }
    pub(super) fn preflight_thread_stack(
        &self,
        _: &pentax_platform::native_memory::PausedNativeMemory<'_>,
        _: pentax_platform::native_memory::NativeRange,
        _: &pentax_kernel::Kernel,
        _: &mut u64,
        _: u64,
    ) -> Result<(), super::KernelStartupError> {
        Ok(())
    }
}
#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare(
    _: &mut super::KernelMemory<'_, '_, '_>,
    _: &mut pentax_kernel::Kernel,
    _: &mut FiberRuntime,
    _: &mut super::pthread_create::ThreadRuntime,
    action: pentax_kernel::FiberAction,
    _: u64,
    _: u64,
    _: [u64; 6],
    _: Option<u64>,
    _: usize,
    _: Option<super::NativeEntryLayout>,
) -> Result<super::NativeServiceAction, super::KernelStartupError> {
    match action {
        pentax_kernel::FiberAction::Returned(v) => Ok(super::NativeServiceAction::Returned(v)),
        _ => Err(super::KernelStartupError::Kernel(
            pentax_kernel::Failure::Unsupported("native fiber host"),
        )),
    }
}
