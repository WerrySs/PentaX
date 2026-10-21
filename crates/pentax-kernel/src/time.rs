// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: std-backed realtime and runtime-epoch monotonic
// clocks, native TSC process counters, checked guest timespec layout and bounded
// sleeps. CPU-time clocks remain unsupported. See NOTICE and Kyty-MIT.txt.

//! Port of the clock and sleep subset of pinned `kernel/pthread.cpp`.
//!
//! Realtime clocks use [`SystemTime`] relative to the Unix epoch. Monotonic
//! clocks use [`Instant`] relative to a runtime-owned epoch captured when this
//! state is created, so their absolute value is not the source boot-relative
//! performance counter, only its monotonic relation. Process counters and
//! microsecond process time share one kernel-owned native TSC epoch, initialized
//! on first use, following the pinned source. Raw TSC reads retain that domain.
//!
//! CPU-time clocks and clock resolutions remain explicitly unsupported. Native
//! TSC counter services require a bound platform adapter with a stable frequency.
//!
//! Raw synchronous sleep invocation blocks the calling host thread, valid only
//! in its admitted single-thread profile. Native Usleep dispatch instead uses
//! the owner-bound cooperative continuation below, in the same process clock
//! domain and up to `Limits::max_sleep_nanoseconds`.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::budget::Budget;
use crate::catalog::Service;
use crate::{CallArgs, Errno, Failure, GuestAccess, GuestWrite, TimeCounterSample};

mod wait;
pub use wait::{TimeWait, TimeWaitAction};

#[cfg(test)]
#[path = "time/tests.rs"]
mod tests;

const NANOS_PER_SECOND: u64 = 1_000_000_000;

const CLOCK_REALTIME: i32 = 0;
const CLOCK_VIRTUAL: i32 = 1;
const CLOCK_PROF: i32 = 2;
const CLOCK_MONOTONIC: i32 = 4;
const CLOCK_UPTIME: i32 = 5;
const CLOCK_UPTIME_PRECISE: i32 = 7;
const CLOCK_UPTIME_FAST: i32 = 8;
const CLOCK_REALTIME_PRECISE: i32 = 9;
const CLOCK_REALTIME_FAST: i32 = 10;
const CLOCK_MONOTONIC_PRECISE: i32 = 11;
const CLOCK_MONOTONIC_FAST: i32 = 12;
const CLOCK_SECOND: i32 = 13;
const CLOCK_THREAD_CPUTIME_ID: i32 = 14;
const CLOCK_PROCTIME: i32 = 15;
const CLOCK_EXT_NETWORK: i32 = 16;
const CLOCK_EXT_DEBUG_NETWORK: i32 = 17;
const CLOCK_EXT_AD_NETWORK: i32 = 18;
const CLOCK_EXT_RAW_NETWORK: i32 = 19;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Domain {
    Realtime,
    RealtimeSecond,
    Monotonic,
}

/// Source clock aliases from `GetPosixClockId` and `KernelClockGettimeSpecial`.
fn clock_domain(clock_id: i32) -> Result<Domain, Failure> {
    match clock_id {
        CLOCK_REALTIME | CLOCK_REALTIME_PRECISE | CLOCK_REALTIME_FAST => Ok(Domain::Realtime),
        CLOCK_SECOND => Ok(Domain::RealtimeSecond),
        CLOCK_MONOTONIC
        | CLOCK_UPTIME
        | CLOCK_UPTIME_PRECISE
        | CLOCK_UPTIME_FAST
        | CLOCK_MONOTONIC_PRECISE
        | CLOCK_MONOTONIC_FAST
        | CLOCK_EXT_NETWORK
        | CLOCK_EXT_DEBUG_NETWORK
        | CLOCK_EXT_AD_NETWORK
        | CLOCK_EXT_RAW_NETWORK => Ok(Domain::Monotonic),
        CLOCK_VIRTUAL | CLOCK_PROF | CLOCK_THREAD_CPUTIME_ID | CLOCK_PROCTIME => {
            Err(Failure::Unsupported("CPU-time or TSC-domain clock"))
        }
        _ => Err(Errno::Invalid.into()),
    }
}

/// `KernelTimespec` and `KernelTimeval`: two little-endian `i64` fields.
fn encode_pair(first: i64, second: i64) -> [u8; 16] {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&first.to_le_bytes());
    bytes[8..].copy_from_slice(&second.to_le_bytes());
    bytes
}

fn decode_pair(bytes: &[u8; 16]) -> (i64, i64) {
    let mut first = [0; 8];
    let mut second = [0; 8];
    first.copy_from_slice(&bytes[..8]);
    second.copy_from_slice(&bytes[8..]);
    (i64::from_le_bytes(first), i64::from_le_bytes(second))
}

fn signed_seconds(duration: Duration) -> Result<i64, Failure> {
    i64::try_from(duration.as_secs())
        .map_err(|_| Failure::Unsupported("time value exceeds signed seconds"))
}

/// Source reports a host realtime before the Unix epoch as EFAULT.
pub(crate) fn realtime() -> Result<Duration, Failure> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Failure::Errno(Errno::Fault))
}

/// Validated `KernelNanosleep` request in nanoseconds.
fn request_nanoseconds(seconds: i64, nanoseconds: i64) -> Result<u64, Failure> {
    if seconds < 0 || nanoseconds < 0 || nanoseconds >= NANOS_PER_SECOND as i64 {
        return Err(Errno::Invalid.into());
    }
    (seconds as u64)
        .checked_mul(NANOS_PER_SECOND)
        .and_then(|value| value.checked_add(nanoseconds as u64))
        .ok_or(Failure::Errno(Errno::Invalid))
}

/// One validated observation of the process counter and its microsecond value.
/// Both fields share the same retained epoch and native counter sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcessClock {
    pub counter: u64,
    pub microseconds: u64,
}

pub(crate) struct TimeState {
    epoch: Instant,
    counter_epoch: Option<TimeCounterSample>,
    wait_owner: u64,
    wait_serial: u64,
}

impl TimeState {
    /// Runtime-owned monotonic origin for resolved device snapshots. Unlike
    /// ProcessTime this requires no guest/native counter adapter.
    pub(crate) fn device_microseconds(&self) -> Result<u64, Failure> {
        u64::try_from(self.epoch.elapsed().as_micros())
            .map_err(|_| Failure::Limit("device monotonic time"))
    }

    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            counter_epoch: None,
            wait_owner: 0,
            wait_serial: 0,
        }
    }

    pub(crate) fn invoke(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        match service {
            Service::ClockGettime => {
                self.clock_gettime(args.0[0] as u32 as i32, args.0[1], guest, budget)
            }
            Service::ClockGetres => {
                if args.0[1] == 0 {
                    return Err(Errno::Fault.into());
                }
                clock_domain(args.0[0] as u32 as i32)?;
                Err(Failure::Unsupported("clock resolution"))
            }
            Service::Gettimeofday => {
                let output = args.0[0];
                if output == 0 {
                    return Err(Errno::Fault.into());
                }
                let now = realtime()?;
                let bytes = encode_pair(signed_seconds(now)?, i64::from(now.subsec_micros()));
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: output,
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            Service::ProcessTime | Service::ProcessTimeCounter => {
                self.process_counter(guest, service == Service::ProcessTime)
            }
            Service::ProcessTimeCounterFrequency | Service::TscFrequency => {
                Ok(counter_sample(guest)?.frequency)
            }
            Service::ReadTsc => Ok(counter_sample(guest)?.ticks),
            Service::Usleep => {
                sleep(u64::from(args.0[0] as u32) * 1_000, budget)?;
                Ok(0)
            }
            Service::Sleep => {
                sleep(u64::from(args.0[0] as u32) * NANOS_PER_SECOND, budget)?;
                Ok(0)
            }
            Service::Nanosleep => nanosleep(args.0[0], args.0[1], guest, budget),
            _ => Err(Failure::Unsupported("service is not owned by time state")),
        }
    }

    /// Internal users such as validated video flips share the very same lazy
    /// process epoch as the guest counter services, without invoking a service.
    pub(crate) fn process_time_counter(
        &mut self,
        guest: &impl GuestAccess,
    ) -> Result<u64, Failure> {
        self.process_counter(guest, false)
    }

    pub(crate) fn process_clock(
        &mut self,
        guest: &impl GuestAccess,
    ) -> Result<ProcessClock, Failure> {
        let (initial, elapsed) = self.prepare_process_counter(guest)?;
        let microseconds = process_microseconds(elapsed, initial.frequency)?;
        // Conversion errors must not publish a first-use epoch either.
        self.counter_epoch = Some(initial);
        Ok(ProcessClock {
            counter: elapsed,
            microseconds,
        })
    }

    fn process_counter(
        &mut self,
        guest: &impl GuestAccess,
        microseconds: bool,
    ) -> Result<u64, Failure> {
        let (initial, elapsed) = self.prepare_process_counter(guest)?;
        let result = if microseconds {
            process_microseconds(elapsed, initial.frequency)?
        } else {
            elapsed
        };
        // Keep original service semantics: failed second samples, frequency
        // changes and failed microsecond conversion never initialize an epoch.
        self.counter_epoch = Some(initial);
        Ok(result)
    }

    fn prepare_process_counter(
        &self,
        guest: &impl GuestAccess,
    ) -> Result<(TimeCounterSample, u64), Failure> {
        let sample = counter_sample(guest)?;
        let initial = self.counter_epoch.unwrap_or(sample);
        let current = if self.counter_epoch.is_none() {
            counter_sample(guest)?
        } else {
            sample
        };
        if current.frequency != initial.frequency {
            return Err(Failure::Unsupported("changed native TSC frequency"));
        }
        let elapsed = current.ticks.saturating_sub(initial.ticks);
        Ok((initial, elapsed))
    }

    fn clock_gettime<G: GuestAccess>(
        &self,
        clock_id: i32,
        output: u64,
        guest: &mut G,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        if output == 0 {
            return Err(Errno::Fault.into());
        }
        let bytes = match clock_domain(clock_id)? {
            Domain::Realtime => {
                let now = realtime()?;
                encode_pair(signed_seconds(now)?, i64::from(now.subsec_nanos()))
            }
            Domain::RealtimeSecond => encode_pair(signed_seconds(realtime()?)?, 0),
            Domain::Monotonic => {
                let elapsed = self.epoch.elapsed();
                encode_pair(signed_seconds(elapsed)?, i64::from(elapsed.subsec_nanos()))
            }
        };
        budget.write(
            guest,
            &[GuestWrite {
                address: output,
                bytes: &bytes,
            }],
        )?;
        Ok(0)
    }
}

fn process_microseconds(elapsed: u64, frequency: u64) -> Result<u64, Failure> {
    u64::try_from(u128::from(elapsed) * 1_000_000 / u128::from(frequency))
        .map_err(|_| Failure::Unsupported("process time exceeds 64-bit microseconds"))
}

fn counter_sample(guest: &impl GuestAccess) -> Result<TimeCounterSample, Failure> {
    let sample = guest.time_counter()?;
    if sample.frequency == 0 {
        return Err(Failure::Unsupported("zero native TSC frequency"));
    }
    Ok(sample)
}

/// Zero is a legitimate immediate return; positive durations must fit policy.
fn sleep(nanoseconds: u64, budget: &Budget) -> Result<(), Failure> {
    if nanoseconds == 0 {
        return Ok(());
    }
    if nanoseconds > budget.limits.max_sleep_nanoseconds {
        return Err(Failure::Limit("sleep duration"));
    }
    std::thread::sleep(Duration::from_nanos(nanoseconds));
    Ok(())
}

/// The source always reports a zero remainder. After a positive sleep the
/// remainder destination cannot be preflighted for writability through
/// [`GuestAccess`], and elapsed time cannot be rolled back, so a non-null
/// remainder is only admitted for a zero request.
fn nanosleep<G: GuestAccess>(
    request: u64,
    remainder: u64,
    guest: &mut G,
    budget: &mut Budget,
) -> Result<u64, Failure> {
    if request == 0 {
        return Err(Errno::Fault.into());
    }
    let mut raw = [0; 16];
    budget.read(&*guest, request, &mut raw)?;
    let (seconds, nanoseconds) = decode_pair(&raw);
    let total = request_nanoseconds(seconds, nanoseconds)?;
    if total == 0 {
        if remainder != 0 {
            budget.write(
                guest,
                &[GuestWrite {
                    address: remainder,
                    bytes: &encode_pair(0, 0),
                }],
            )?;
        }
        return Ok(0);
    }
    if total > budget.limits.max_sleep_nanoseconds {
        return Err(Failure::Limit("sleep duration"));
    }
    if remainder != 0 {
        return Err(Failure::Unsupported(
            "nanosleep remainder after positive sleep",
        ));
    }
    sleep(total, budget)?;
    Ok(0)
}
