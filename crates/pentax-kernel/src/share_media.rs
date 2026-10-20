// SPDX-License-Identifier: GPL-2.0-only
// Adapted from authorized KytyPS5-Performance@209dffe libShare.cpp.
// Blob a38efbeefac2742da3dbe8248651a983150abb07; see NOTICE/provenance.
//! Silent source Share contracts. Capture is explicitly unsupported; callback
//! registration neither retains nor invokes a handler in this reference.

use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Service, budget::Budget};
const INVALID: u64 = 0x8196_0002;
const NOT_SUPPORTED: u64 = 0x8196_0007;

pub(crate) fn invoke(
    service: Service,
    args: CallArgs,
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    use Service::*;
    budget.work(1)?;
    let a = args.0;
    match service {
        ShareCaptureVideoClip
        | ShareCaptureVideoClipExtended
        | ShareCaptureScreenshot
        | ShareCaptureScreenshotExtended => {
            if a[1] != 0 {
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &u32::MAX.to_le_bytes(),
                    }],
                )?;
            }
            Ok(NOT_SUPPORTED)
        }
        ShareOpenMenuForContent => Ok(NOT_SUPPORTED),
        ShareFeaturePermit | ShareFeatureProhibit | ShareSetCaptureSource => {
            Ok(if a[0] as u32 == 0 { INVALID } else { 0 })
        }
        ShareSetContentParam | ShareSetContentParamForApplicationTitle => {
            Ok(if a[0] == 0 { INVALID } else { 0 })
        }
        ShareRegisterContentEventCallback
        | ShareUnregisterContentEventCallback
        | ShareSetScreenshotOverlayImage => Ok(0),
        ShareGetCurrentStatus => {
            if a[0] as u32 == 0 || a[1] == 0 {
                return Ok(INVALID);
            }
            budget.write(
                guest,
                &[GuestWrite {
                    address: a[1],
                    bytes: &[0; 16],
                }],
            )?;
            Ok(0)
        }
        ShareGetRunningStatus => {
            if a[0] == 0 {
                return Ok(INVALID);
            }
            budget.write(
                guest,
                &[GuestWrite {
                    address: a[0],
                    bytes: &[0; 4],
                }],
            )?;
            Ok(0)
        }
        _ => Err(Failure::Unsupported("Share media service")),
    }
}

#[cfg(test)]
mod tests;
