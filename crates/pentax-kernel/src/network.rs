// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded owned startup resources and guest-thread errno.
// Adapted from pinned network.cpp/libNet.cpp/errno.h; see NOTICE.

use crate::budget::Budget;
use crate::{CallArgs, Failure, GuestAccess, GuestWrite, Kernel, Service};

const EBADF: u32 = 0x8041_0109;
const EINVAL: u32 = 0x8041_0116;
const ENFILE: u32 = 0x8041_0117;
const ENOSPACE: u32 = 0x8041_01de;
const SSL_INVALID: u32 = 0x8095_f006;
const SSL_FULL: u32 = 0x8095_f008;
const SSL_ARGUMENT: u32 = 0x8095_177a;
const SSL_NOT_FOUND: u32 = 0x8095_f004;
const HTTP_MEMORY: u32 = 0x8043_1022;
const HTTP_INVALID: u32 = 0x8043_1100;

struct Pool {
    _name: Vec<u8>,
    _size: i32,
}
struct Resolver {
    _name: Vec<u8>,
    _pool: i32,
}
struct Http {
    _pool: i32,
    _ssl: usize,
    _size: u64,
}
pub(crate) struct NetworkState {
    pools: [Option<Pool>; 32],
    resolvers: [Option<Resolver>; 32],
    ssl: [Option<u64>; 32],
    http: [Option<Http>; 32],
}
impl Default for NetworkState {
    fn default() -> Self {
        Self {
            pools: std::array::from_fn(|_| None),
            resolvers: std::array::from_fn(|_| None),
            ssl: [None; 32],
            http: std::array::from_fn(|_| None),
        }
    }
}

fn index(value: u64) -> Option<usize> {
    usize::try_from(value as u32 as i32)
        .ok()
        .filter(|index| *index < 32)
}
fn typed(value: u64, tag: u32) -> Option<usize> {
    let bits = value as u32;
    (bits >> 16 == tag >> 16)
        .then_some((bits & 0xffff) as usize)
        .filter(|index| *index < 32)
}
fn name(address: u64, guest: &impl GuestAccess, budget: &mut Budget) -> Result<Vec<u8>, Failure> {
    if address == 0 {
        return Err(Failure::Unsupported("source-null network pool name"));
    }
    // Names are opaque bytes, not host paths or UTF-8 requirements. Source
    // string construction occurs only after finding a free usable record.
    let mut bytes = budget.buffer(budget.limits.max_name_bytes)?;
    for at in 0..bytes.len() {
        let address = address
            .checked_add(at as u64)
            .ok_or(crate::GuestError::AddressOverflow)?;
        budget.read(guest, address, &mut bytes[at..at + 1])?;
        if bytes[at] == 0 {
            bytes.truncate(at);
            return Ok(bytes);
        }
    }
    Err(Failure::Limit("name bytes"))
}
impl NetworkState {
    pub(crate) fn retained_handle_count(&self) -> usize {
        self.pools.iter().filter(|entry| entry.is_some()).count()
            + self
                .resolvers
                .iter()
                .filter(|entry| entry.is_some())
                .count()
            + self.ssl.iter().filter(|entry| entry.is_some()).count()
            + self.http.iter().filter(|entry| entry.is_some()).count()
    }
    fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        budget.work(129)?; // Four fixed32-slot tables and classification.
        let a = args.0;
        use Service::*;
        match service {
            NetInit => {
                // Source invokes its actual backend and returns0 even when
                // startup fails. This never certifies a socket or connectivity.
                let _ = guest.initialize_network_backend()?;
                Ok(0)
            }
            NetPoolCreate => {
                if a[2] as u32 != 0 || a[1] as u32 == 0 {
                    return Err(Failure::Unsupported(
                        "source-aborting network pool arguments",
                    ));
                }
                let Some(at) = (1..32).find(|at| self.pools[*at].is_none()) else {
                    return Ok(u64::from(ENFILE));
                };
                budget.can_add_handle()?;
                let name = name(a[0], guest, budget)?;
                self.pools[at] = Some(Pool {
                    _name: name,
                    _size: a[1] as u32 as i32,
                });
                budget.added_handle();
                Ok(at as u64)
            }
            NetPoolDestroy => {
                let Some(at) = index(a[0]).filter(|at| self.pools[*at].is_some()) else {
                    return Ok(u64::from(EBADF));
                };
                self.pools[at] = None;
                budget.removed_handle();
                Ok(0)
            }
            NetResolverCreate => {
                if a[0] == 0 || a[2] as u32 != 0 {
                    return Ok(u64::from(EINVAL));
                }
                if index(a[1]).is_none_or(|at| self.pools[at].is_none()) {
                    return Ok(u64::from(ENOSPACE));
                }
                let Some(at) = self.resolvers.iter().position(Option::is_none) else {
                    return Ok(u64::from(ENOSPACE));
                };
                budget.can_add_handle()?;
                let name = name(a[0], guest, budget)?;
                self.resolvers[at] = Some(Resolver {
                    _name: name,
                    _pool: a[1] as u32 as i32,
                });
                budget.added_handle();
                Ok(at as u64) // Source slot0 is valid, not a failure sentinel.
            }
            NetResolverDestroy => {
                let Some(at) = index(a[0]).filter(|at| self.resolvers[*at].is_some()) else {
                    return Ok(u64::from(EBADF));
                };
                self.resolvers[at] = None;
                budget.removed_handle();
                Ok(0)
            }
            NetResolverAbort => Ok(0), // Pinned no-op; no DNS/cancellation claim.
            SslInit => {
                if a[0] == 0 {
                    return Err(Failure::Unsupported("source-aborting SSL pool size"));
                }
                let Some(at) = self.ssl.iter().position(Option::is_none) else {
                    return Ok(u64::from(SSL_FULL));
                };
                budget.can_add_handle()?;
                self.ssl[at] = Some(a[0]);
                budget.added_handle();
                Ok(0x2_0000 | at as u64)
            }
            SslTerm => {
                let Some(at) = typed(a[0], 0x2_0000).filter(|at| self.ssl[*at].is_some()) else {
                    return Ok(u64::from(SSL_INVALID));
                };
                self.ssl[at] = None;
                budget.removed_handle();
                Ok(0)
            }
            SslGetCaCerts | SslFreeCaCerts => {
                if a[1] == 0 {
                    return Ok(u64::from(SSL_ARGUMENT));
                }
                if typed(a[0], 0x2_0000).is_none_or(|at| self.ssl[at].is_none()) {
                    return Ok(u64::from(SSL_INVALID));
                }
                // Get explicitly clears all three fields before NOT_FOUND;
                // Free neither follows nor releases preexisting guest pointers.
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &[0; 24],
                    }],
                )?;
                Ok(if service == SslGetCaCerts {
                    u64::from(SSL_NOT_FOUND)
                } else {
                    0
                })
            }
            HttpInit => {
                if a[2] == 0 {
                    return Err(Failure::Unsupported("source-aborting HTTP pool size"));
                }
                let Some(ssl) = typed(a[1], 0x2_0000).filter(|at| self.ssl[*at].is_some()) else {
                    return Ok(u64::from(HTTP_MEMORY));
                };
                if index(a[0]).is_none_or(|at| self.pools[at].is_none()) {
                    return Ok(u64::from(HTTP_MEMORY));
                }
                let Some(at) = self.http.iter().position(Option::is_none) else {
                    return Ok(u64::from(HTTP_MEMORY));
                };
                budget.can_add_handle()?;
                self.http[at] = Some(Http {
                    _pool: a[0] as u32 as i32,
                    _ssl: ssl,
                    _size: a[2],
                });
                budget.added_handle();
                Ok(0x1_0000 | at as u64)
            }
            HttpTerm => {
                let Some(at) = typed(a[0], 0x1_0000).filter(|at| self.http[*at].is_some()) else {
                    return Ok(u64::from(HTTP_INVALID));
                };
                self.http[at] = None;
                budget.removed_handle();
                Ok(0)
            }
            _ => Err(Failure::Unsupported("network startup service")),
        }
    }
}
impl Kernel {
    fn network_errno_address(&mut self, guest: &mut impl GuestAccess) -> Result<u64, Failure> {
        self.budget.work(12)?;
        guest.preflight_network_errno(None)?;
        let address = if let Some(address) = self.threads.active().net_errno {
            address
        } else {
            let address = self.memory.runtime_word(0, guest, &mut self.budget)?;
            self.threads.active_mut().net_errno = Some(address);
            address
        };
        guest.preflight_network_errno(Some(address))?;
        Ok(address)
    }
    pub(crate) fn invoke_network(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
    ) -> Result<u64, Failure> {
        if service == Service::NetGetErrorAddr {
            return self.network_errno_address(guest);
        }
        let result = self
            .network
            .invoke(service, args, guest, &mut self.budget)?;
        if matches!(
            service,
            Service::NetInit
                | Service::NetPoolCreate
                | Service::NetPoolDestroy
                | Service::NetResolverCreate
                | Service::NetResolverDestroy
                | Service::NetResolverAbort
        ) && (result as u32 as i32) < 0
        {
            let number = (result as u32).wrapping_sub(0x8041_0101).wrapping_add(1);
            let address = self.network_errno_address(guest)?;
            self.budget.write(
                guest,
                &[GuestWrite {
                    address,
                    bytes: &number.to_le_bytes(),
                }],
            )?;
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "network/tests.rs"]
mod tests;
