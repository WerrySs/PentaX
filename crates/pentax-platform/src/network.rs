// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: serialized, success-only host initialization.
// Adapted from EnsureSocketBackend in network.cpp at source pin
// 209dffe9032d72749257ce18680d506b891284de; see NOTICE.

//! Host socket-backend initialization for the dedicated worker process.
//! A successful Windows initialization is retained until process exit, matching
//! the reference's absence of WSACleanup. This opens no sockets or connections.

/// Initialize the host socket backend, retaining only successful initialization.
/// Windows failures are retried on the next call. Other hosts need no explicit
/// initialization in the pinned reference and return `true`.
#[cfg(target_os = "windows")]
pub fn initialize() -> bool {
    use std::sync::Mutex;
    use windows_sys::Win32::Networking::WinSock::{WSADATA, WSAStartup};

    static INITIALIZED: Mutex<bool> = Mutex::new(false);

    // The protected value is only changed after WSAStartup succeeds. Recovering
    // a poisoned lock preserves that invariant and does not suppress retries.
    let mut initialized = match INITIALIZED.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *initialized {
        return true;
    }

    let mut data = WSADATA::default();
    // SAFETY: data is owned, correctly aligned, fully sized writable WSADATA
    // storage. WSAStartup writes it synchronously and retains no buffer pointer.
    // The fixed version word requests WinSock 2.2; no guest address is involved.
    if unsafe { WSAStartup(0x0202, &mut data) } != 0 {
        return false;
    }
    *initialized = true;
    true
}

/// No explicit socket-backend initialization is required on non-Windows hosts.
#[cfg(not(target_os = "windows"))]
pub fn initialize() -> bool {
    true
}
