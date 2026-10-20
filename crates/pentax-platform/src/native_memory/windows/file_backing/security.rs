// SPDX-License-Identifier: GPL-2.0-only

//! Protect a new empty, exclusive file before any guest data can reach it.
use super::*;
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        ACL,
        Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
            SetSecurityInfo,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR,
    },
};

struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: Exactly the LocalAlloc-owned descriptor returned by conversion.
        let _ = unsafe { LocalFree(self.0) };
    }
}
pub(super) fn protect(handle: HANDLE) -> Result<(), NativeMemoryError> {
    // Protected DACL: SYSTEM and the object's owner only. No inherited ACEs.
    let text: Vec<u16> = "D:P(A;;FA;;;SY)(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut raw = std::ptr::null_mut();
    // SAFETY: NUL-terminated fixed SDDL, writable descriptor-pointer output;
    // Windows allocates it and Descriptor always releases it with LocalFree.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut raw,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(os_error("backing file ACL descriptor", 0));
    }
    let descriptor = Descriptor(raw);
    let mut present = 0;
    let mut defaulted = 0;
    let mut acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: Valid live converted descriptor; all outputs are exact scalars.
    if unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted) }
        == 0
    {
        return Err(os_error("backing file ACL query", 0));
    }
    if present == 0 || acl.is_null() {
        return Err(NativeMemoryError::PermissionDenied(0));
    }
    // SAFETY: This is the exclusively opened new empty file with WRITE_DAC;
    // the nonnull DACL remains live for the synchronous call. No owner, group
    // or SACL mutation is requested. No data/section exists until this succeeds.
    let code = unsafe {
        SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            acl,
            std::ptr::null(),
        )
    };
    if code != 0 {
        return Err(NativeMemoryError::OperatingSystem {
            operation: "backing file ACL apply",
            address: 0,
            code,
        });
    }
    Ok(())
}
