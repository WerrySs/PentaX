// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: checked APR identity queries and resolver variants.
// Adapted from pinned libAmpr.cpp; see NOTICE.

use super::{FileState, range};
use crate::budget::Budget;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, Service};

/// Private ABI normalization. ForEach takes an int32 result array, not a
/// callback. Its optional result pointer selects continue-on-error semantics.
pub(super) struct Paths {
    pub list: u64,
    pub count: u32,
    pub ids: u64,
    pub sizes: u64,
    pub error_index: u64,
    pub prefix: u64,
    pub results: u64,
}

impl Paths {
    pub(super) fn sizes(args: CallArgs) -> Self {
        let [list, count, ids, sizes, error_index, _] = args.0;
        Self {
            list,
            count: count as u32,
            ids,
            sizes,
            error_index,
            prefix: 0,
            results: 0,
        }
    }

    fn decode(service: Service, args: CallArgs) -> Result<Self, Failure> {
        use Service::*;
        let [a, b, c, d, e, f] = args.0;
        let (prefix, list, count, ids, sizes, error_index, results) = match service {
            AprResolveFilepathsToIds => (0, a, b, c, 0, d, 0),
            AprResolveFilepathsToIdsAndFileSizes => return Ok(Self::sizes(args)),
            AprResolveFilepathsWithPrefixToIds => (a, b, c, d, 0, e, 0),
            AprResolveFilepathsWithPrefixToIdsAndFileSizes => (a, b, c, d, e, f, 0),
            AprResolveFilepathsToIdsForEach => (0, a, b, c, 0, 0, d),
            AprResolveFilepathsToIdsAndFileSizesForEach => (0, a, b, c, d, 0, e),
            AprResolveFilepathsWithPrefixToIdsForEach => (a, b, c, d, 0, 0, e),
            AprResolveFilepathsWithPrefixToIdsAndFileSizesForEach => (a, b, c, d, e, 0, f),
            _ => return Err(Failure::Unsupported("APR resolver ABI")),
        };
        Ok(Self {
            list,
            count: count as u32,
            ids,
            sizes,
            error_index,
            prefix,
            results,
        })
    }
}

impl FileState {
    pub(crate) fn invoke_apr_import(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        match service {
            Service::AprGetFileStat | Service::AprGetFileSize => {
                self.apr_query(service, args, guest, budget)
            }
            Service::AprResolveFilepathsToIdsAndFileSizes => {
                self.resolve_apr_paths(args, guest, budget)
            }
            _ => self.resolve_apr_common(Paths::decode(service, args)?, guest, budget),
        }
    }

    fn apr_query(
        &self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let id = args.0[0] as u32;
        let output = args.0[1];
        if output == 0 {
            return Err(Errno::Invalid.into());
        }
        let size_only = service == Service::AprGetFileSize;
        // Source size validates its numeric output range before ID lookup;
        // stat looks up the ID first. Actual backing writes remain checked.
        if size_only {
            range(output, 8)?;
        }
        budget.work(1)?;
        let entry = self.apr.identified(id).ok_or(Errno::NoEntry)?;
        if size_only {
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &entry.info.file_size.to_le_bytes(),
                }],
            )?;
        } else {
            range(output, 120)?;
            budget.work(entry.path.len() as u64 + 64)?;
            // The retained guest name is only an identity. Re-open through the
            // same owned mount resolver; no host pathname authority is cached.
            let file = self.selected(&entry.path, budget)?.ok_or(Errno::NoEntry)?;
            let metadata = file.metadata().map_err(|_| Errno::Io)?;
            let bytes = if metadata.is_dir() {
                super::super::directory_stat(0)?
            } else {
                super::super::stat_bytes(&metadata)?
            };
            budget.write(
                guest,
                &[GuestWrite {
                    address: output,
                    bytes: &bytes,
                }],
            )?;
        }
        Ok(0)
    }
}
