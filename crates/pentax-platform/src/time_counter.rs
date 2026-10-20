// SPDX-License-Identifier: GPL-2.0-only

//! Native invariant TSC and its frequency for the Windows x86-64 worker.
//! CPUID supplies the frequency when available; otherwise a single bounded
//! calibration uses the host monotonic clock, as in the pinned Kyty reference.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CounterSample {
    pub ticks: u64,
    pub frequency: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterError {
    Unsupported,
    Calibration,
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
pub use backend::sample;

#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
pub fn sample() -> Result<CounterSample, CounterError> {
    Err(CounterError::Unsupported)
}

#[cfg(all(target_os = "windows", target_arch = "x86_64"))]
mod backend {
    use super::*;
    use std::arch::x86_64::{__cpuid, __cpuid_count, _mm_lfence, _rdtsc};
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    static FREQUENCY: OnceLock<Result<u64, CounterError>> = OnceLock::new();

    // Only called after TSC/invariance admission; no guest pointer or code is used.
    fn read_admitted() -> u64 {
        // SAFETY: x86-64 guarantees SSE2. Callers admit invariant TSC with CPUID;
        // the Windows worker uses the normal user-mode TSC facility. Fixed fences
        // order this host observation without touching guest or host control state.
        unsafe {
            _mm_lfence();
            let ticks = _rdtsc();
            _mm_lfence();
            ticks
        }
    }

    fn frequency() -> Result<u64, CounterError> {
        // CPUID is available in x86-64. Only supported leaves are queried;
        // the calls inspect fixed host feature numbers and accept no guest input.
        let enumerated = {
            let maximum = __cpuid(0).eax;
            if maximum < 1
                || __cpuid(1).edx & (1 << 4) == 0
                || __cpuid(0x8000_0000).eax < 0x8000_0007
                || __cpuid(0x8000_0007).edx & (1 << 8) == 0
            {
                return Err(CounterError::Unsupported);
            }
            if maximum >= 0x15 {
                let ratio = __cpuid_count(0x15, 0);
                if ratio.eax != 0 && ratio.ebx != 0 && ratio.ecx != 0 {
                    Some(u64::from(ratio.ecx) * u64::from(ratio.ebx) / u64::from(ratio.eax))
                } else {
                    None
                }
            } else {
                None
            }
        };
        if let Some(hz) = enumerated.filter(|hz| *hz != 0) {
            return Ok(hz);
        }
        // Initialization is once per worker, independent of guest sleep budgets.
        let start = Instant::now();
        let initial = read_admitted();
        std::thread::sleep(Duration::from_millis(100));
        let final_ticks = read_admitted();
        let nanos = start.elapsed().as_nanos();
        let ticks = final_ticks
            .checked_sub(initial)
            .filter(|n| *n != 0)
            .ok_or(CounterError::Calibration)?;
        if nanos == 0 {
            return Err(CounterError::Calibration);
        }
        let hz = u128::from(ticks) * 1_000_000_000 / nanos;
        // Match the source's 100 kHz rounding without floating-point conversion.
        let hz = u64::try_from((hz + 50_000) / 100_000 * 100_000)
            .map_err(|_| CounterError::Calibration)?;
        if hz == 0 {
            Err(CounterError::Calibration)
        } else {
            Ok(hz)
        }
    }

    pub fn sample() -> Result<CounterSample, CounterError> {
        let frequency = (*FREQUENCY.get_or_init(frequency))?;
        Ok(CounterSample {
            ticks: read_admitted(),
            frequency,
        })
    }
}
