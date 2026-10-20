// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Public registration names from KytyPS5-Performance revision
// 209dffe9032d72749257ce18680d506b891284de, libs/libKernel.cpp and libs/libC.cpp.
// Modified 2026 for PentaX: sorted diagnostic-only lookup, never a resolver.

use pentax_linker::MissingFunctionImport;

mod extended;
mod name_hashes;
mod sharp_http2;

// The function hint and independent library category each scan the bounded
// catalogue. Public references add their own function-only comparisons. The
// name/hash binary search is conservatively charged as a full bounded scan.
pub(super) const DIAGNOSTIC_WORK: u64 = (2 * (NAMES.len() as u64 + extended::COUNT as u64)
    + sharp_http2::COUNT as u64
    + name_hashes::COUNT as u64)
    * 256;

pub(super) fn reference_library(import: &MissingFunctionImport) -> Option<&'static str> {
    library_category(import.library_name())
}

fn library_category(library: &[u8]) -> Option<&'static str> {
    let library = library.strip_prefix(b"libSce").unwrap_or(library);
    NAMES
        .iter()
        .map(|entry| entry.1)
        .chain(extended::identities().map(|entry| entry.1))
        .find_map(|known| {
            // Normalize only for this diagnostic category. The returned slice
            // always belongs to a static public entry, never to the input.
            let category = known.strip_prefix("libSce").unwrap_or(known);
            (category.as_bytes() == library).then_some(category)
        })
}

// A reference hint is distinct from an exact identity. It cannot supply a
// binding, relax resolver versions or turn a missing service into a success.
pub(super) fn reference_hint(
    import: &MissingFunctionImport,
) -> (Option<&'static str>, &'static str) {
    hint(
        import.module_name(),
        import.library_name(),
        import.nid().as_bytes(),
    )
}

fn hint(module: &[u8], library: &[u8], nid: &[u8]) -> (Option<&'static str>, &'static str) {
    hint_from(
        module,
        library,
        nid,
        NAMES.iter().copied().chain(extended::identities()),
    )
}

fn hint_from(
    module: &[u8],
    library: &[u8],
    nid: &[u8],
    identities: impl Iterator<Item = (&'static str, &'static str, &'static str, &'static str)>,
) -> (Option<&'static str>, &'static str) {
    let library_reference = sharp_http2::lookup(library, nid);
    let hash_reference = name_hashes::lookup(nid);
    let mut scoped = None;
    let mut unscoped = None;
    let mut scoped_conflict = false;
    let mut unscoped_conflict = false;
    let mut reference_conflict = library_reference
        .zip(hash_reference)
        .is_some_and(|(library_name, hash_name)| library_name != hash_name);
    for (m, l, n, name) in identities {
        if n.as_bytes() != nid {
            continue;
        }
        reference_conflict |= library_reference.is_some_and(|reference| reference != name);
        reference_conflict |= hash_reference.is_some_and(|reference| reference != name);
        if unscoped.is_some_and(|previous| previous != name) {
            unscoped_conflict = true;
        }
        unscoped.get_or_insert(name);
        if m.as_bytes() == module && l.as_bytes() == library {
            if scoped.is_some_and(|previous| previous != name) {
                scoped_conflict = true;
            }
            scoped.get_or_insert(name);
        }
    }
    if reference_conflict {
        (None, "ambiguous-reference")
    } else if scoped.is_some() {
        if scoped_conflict {
            (None, "ambiguous-reference")
        } else {
            (scoped, "descriptor-version")
        }
    } else if unscoped_conflict {
        (None, "ambiguous-reference")
    } else if unscoped.is_some() {
        (unscoped, "different-scope")
    } else if let Some(reference) = library_reference {
        (Some(reference), "library-reference")
    } else if let Some(reference) = hash_reference {
        (Some(reference), "public-name-hash-reference")
    } else {
        (None, "unlisted-reference")
    }
}

pub(super) fn name(import: &MissingFunctionImport) -> Option<&'static str> {
    let versions = import.versions();
    let versions = (
        versions.module_major,
        versions.module_minor,
        versions.library,
    );
    exact_name(
        import.module_name(),
        import.library_name(),
        import.nid().as_bytes(),
        versions,
    )
}

fn exact_name(
    module: &[u8],
    library: &[u8],
    nid: &[u8],
    versions: (u8, u8, u16),
) -> Option<&'static str> {
    if versions == (1, 1, 1)
        && let Some(name) = lookup(module, library, nid)
    {
        return Some(name);
    }
    extended::lookup(module, library, nid, versions)
}

fn lookup(module: &[u8], library: &[u8], nid: &[u8]) -> Option<&'static str> {
    NAMES
        .binary_search_by(|(m, l, n, _)| {
            (m.as_bytes(), l.as_bytes(), n.as_bytes()).cmp(&(module, library, nid))
        })
        .ok()
        .map(|index| NAMES[index].3)
}

// Only pinned public identities and labels occur in this table. A miss returns
// no label; input strings are never formatted into a diagnostic name.
const NAMES: &[(&str, &str, &str, &str)] = &[
    (
        "Coredump",
        "Coredump",
        "8zLSfEfW5AU",
        "Coredump::sceCoredumpRegisterCoredumpHandler",
    ),
    (
        "Coredump",
        "Coredump",
        "fFkhOgztiCA",
        "Coredump::sceCoredumpUnregisterCoredumpHandler",
    ),
    (
        "Fiber",
        "Fiber",
        "0dy4JtMUcMQ",
        "Fiber::FiberGetThreadFramePointerAddress",
    ),
    (
        "Fiber",
        "Fiber",
        "7+OJIpko9RY",
        "Fiber::FiberInitializeInternal",
    ),
    (
        "Fiber",
        "Fiber",
        "B0ZX2hx9DMw",
        "Fiber::FiberReturnToThread",
    ),
    ("Fiber", "Fiber", "JeNX5F-NzQU", "Fiber::FiberFinalize"),
    ("Fiber", "Fiber", "JzyT91ucGDc", "Fiber::FiberRename"),
    (
        "Fiber",
        "Fiber",
        "Kj4nXMpnM8Y",
        "Fiber::FiberStopContextSizeCheck",
    ),
    (
        "Fiber",
        "Fiber",
        "Lcqty+QNWFc",
        "Fiber::FiberStartContextSizeCheck",
    ),
    ("Fiber", "Fiber", "PFT2S-tJ7Uk", "Fiber::FiberSwitch"),
    ("Fiber", "Fiber", "ZqhZFuzKT6U", "Fiber::FiberSwitch"),
    ("Fiber", "Fiber", "a0LLrZWac0M", "Fiber::FiberRun"),
    (
        "Fiber",
        "Fiber",
        "asjUJJ+aa8s",
        "Fiber::FiberOptParamInitialize",
    ),
    ("Fiber", "Fiber", "avfGJ94g36Q", "Fiber::FiberRun"),
    ("Fiber", "Fiber", "hVYD7Ou2pCQ", "Fiber::FiberInitialize"),
    ("Fiber", "Fiber", "p+zLIOg27zU", "Fiber::FiberGetSelf"),
    ("Fiber", "Fiber", "uq2Y5BFz0PE", "Fiber::FiberGetInfo"),
    (
        "LibcInternal",
        "LibcInternal",
        "+P6FRGH4LfA",
        "LibcInternal::memmove",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "-P6FNMzk2Kc",
        "LibcInternal::cosf",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "2WE3BTYVwKM",
        "LibcInternal::cos",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "5Xa2ACNECdo",
        "LibcInternal::strcpy",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "6sJWiWSRuqk",
        "LibcInternal::strncpy",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "8zTFvBIAIN8",
        "LibcInternal::memset",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "9yDWMxEFdJU",
        "LibcInternal::strrchr",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "DfivPArhucg",
        "LibcInternal::memcmp",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "DiGVep5yB5w",
        "LibC::std_execute_once",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "GMpvxPFW924",
        "LibcInternal::vprintf",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "H2e8t5ScQGc",
        "LibC::cxa_finalize",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "H8ya2H00jbI",
        "LibcInternal::sin",
    ),
    ("LibcInternal", "LibcInternal", "L1SBTkC+Cvw", "LibC::abort"),
    (
        "LibcInternal",
        "LibcInternal",
        "Ls4tzzhimqQ",
        "LibcInternal::strcat",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "MUjC4lbHrK4",
        "LibcInternal::fflush",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "Ovb2dSJOAuE",
        "LibcInternal::strncmp",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "Q3VBxCXhUHs",
        "LibcInternal::memcpy",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "Q4rRL34CEeE",
        "LibcInternal::sinf",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "QxmSHBCuKTk",
        "LibcInternal::strtoul",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "aesyjrHVWy4",
        "LibcInternal::strcmp",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "al3JzFI9MQ0",
        "LibcInternal::LibcHeapErrorReportForGame",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "eLdDw6l0-bU",
        "LibcInternal::snprintf",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "fPxypibz2MY",
        "LibcInternal::atoi",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "j4ViWNHEgww",
        "LibcInternal::strlen",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "jMB7EFyu30Y",
        "LibcInternal::sincos",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "mXlxhmLNMPg",
        "LibcInternal::strtol",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "ob5xAW4ln-0",
        "LibcInternal::strchr",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "pztV4AF18iI",
        "LibcInternal::sincosf",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "tsvEmnenz48",
        "LibC::cxa_atexit",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "viiwFMaNamA",
        "LibcInternal::strstr",
    ),
    (
        "LibcInternal",
        "LibcInternal",
        "zlfEH8FmyUA",
        "LibcInternal::strtoul",
    ),
    (
        "LibcInternal",
        "LibcInternalExt",
        "NWtTN10cJzE",
        "LibcInternalExt::LibcHeapGetTraceInfo",
    ),
    (
        "LibcInternal",
        "LibcInternalExt",
        "qBS714-Jr3g",
        "LibcInternalExt::LibcInternalExtCxaThreadAtexit",
    ),
    ("libc", "libc", "-VVn74ZyhEs", "LibC::libc_difftime"),
    ("libc", "libc", "1mecP7RgI2A", "LibC::libc_gmtime"),
    ("libc", "libc", "8G2LB+A3rzg", "LibC::atexit"),
    ("libc", "libc", "9BcDykPmo1I", "LibC::libc_error"),
    ("libc", "libc", "Av3zjWi64Kw", "LibC::libc_strftime"),
    ("libc", "libc", "DiGVep5yB5w", "LibC::std_execute_once"),
    ("libc", "libc", "H2e8t5ScQGc", "LibC::cxa_finalize"),
    ("libc", "libc", "L1SBTkC+Cvw", "LibC::abort"),
    ("libc", "libc", "M4YYbSFfJ8g", "LibC::setenv"),
    ("libc", "libc", "XKRegsFpEpk", "LibC::catchReturnFromMain"),
    ("libc", "libc", "YQ0navp+YIc", "LibC::puts"),
    ("libc", "libc", "bzQExy189ZI", "LibC::init_env"),
    ("libc", "libc", "efhK-YSUYYQ", "LibC::libc_localtime"),
    ("libc", "libc", "hcuQgD53UxM", "LibC::libc_printf"),
    ("libc", "libc", "n7AepwR0s34", "LibC::libc_mktime"),
    ("libc", "libc", "tsvEmnenz48", "LibC::cxa_atexit"),
    ("libc", "libc", "uMei1W9uyNo", "LibC::exit"),
    ("libc", "libc", "wLlFkwG9UcQ", "LibC::libc_time"),
    ("libkernel", "Posix", "+U1R4WtXvoc", "Posix::pthread_detach"),
    (
        "libkernel",
        "Posix",
        "-SrbXpGR1f0",
        "Posix::pthread_attr_setstack",
    ),
    (
        "libkernel",
        "Posix",
        "-quPa4SEJUw",
        "Posix::pthread_getstack",
    ),
    (
        "libkernel",
        "Posix",
        "0-KXaS70xy4",
        "Posix::pthread_getspecific",
    ),
    (
        "libkernel",
        "Posix",
        "0TyVk4MSLt0",
        "Posix::pthread_cond_init",
    ),
    (
        "libkernel",
        "Posix",
        "0qOtCR-ZHck",
        "Posix::pthread_attr_getstacksize",
    ),
    (
        "libkernel",
        "Posix",
        "1471ajPzxh0",
        "Posix::pthread_rwlock_destroy",
    ),
    (
        "libkernel",
        "Posix",
        "2MOy+rUfuhQ",
        "Posix::pthread_cond_signal",
    ),
    (
        "libkernel",
        "Posix",
        "2Q0z6rnBrTE",
        "Posix::pthread_attr_setstacksize",
    ),
    (
        "libkernel",
        "Posix",
        "2Z+PpY6CaJg",
        "Posix::pthread_mutex_unlock",
    ),
    ("libkernel", "Posix", "3e+4Iv7IJ8U", "Posix::accept"),
    (
        "libkernel",
        "Posix",
        "4SbrhCozqQU",
        "Posix::sem_reltimedwait_np",
    ),
    ("libkernel", "Posix", "4n51s0zEf0c", "Posix::inet_pton"),
    ("libkernel", "Posix", "5jRCs2axtr4", "Posix::inet_ntop"),
    (
        "libkernel",
        "Posix",
        "5txKfcMUAok",
        "Posix::pthread_mutexattr_setprotocol",
    ),
    (
        "libkernel",
        "Posix",
        "6BpEZuDT7YI",
        "Posix::pthread_key_delete",
    ),
    ("libkernel", "Posix", "6O8EwYOgH9Y", "Posix::getsockopt"),
    (
        "libkernel",
        "Posix",
        "7H0iTOciTLo",
        "Posix::pthread_mutex_lock",
    ),
    (
        "libkernel",
        "Posix",
        "7Xl257M4VNI",
        "LibKernel::PthreadEqual",
    ),
    (
        "libkernel",
        "Posix",
        "7ZlAakEf0Qg",
        "Posix::pthread_attr_setinheritsched",
    ),
    ("libkernel", "Posix", "8nY19bKoiZk", "Posix::fcntl"),
    (
        "libkernel",
        "Posix",
        "9vyP6Z7bqzc",
        "Posix::pthread_rename_np",
    ),
    ("libkernel", "Posix", "AqBioC2vF3I", "LibKernel::read"),
    ("libkernel", "Posix", "B5GmVDKwpn0", "Posix::pthread_yield"),
    ("libkernel", "Posix", "Bq+LRV-N6Hk", "Posix::sem_getvalue"),
    (
        "libkernel",
        "Posix",
        "E+tyo3lp5Lw",
        "Posix::pthread_attr_setdetachstate",
    ),
    ("libkernel", "Posix", "E6ao34wPw+U", "stat"),
    ("libkernel", "Posix", "EotR8a3ASf4", "Posix::pthread_self"),
    ("libkernel", "Posix", "Ez8xjo9UF4E", "Posix::recv"),
    (
        "libkernel",
        "Posix",
        "FIs3-UQT9sg",
        "Posix::pthread_getschedparam",
    ),
    ("libkernel", "Posix", "FJrT5LuUBAU", "Posix::pthread_exit"),
    (
        "libkernel",
        "Posix",
        "FXPWHNk8Of0",
        "Posix::pthread_attr_getschedparam",
    ),
    (
        "libkernel",
        "Posix",
        "HF7lK46xzjY",
        "Posix::pthread_mutexattr_destroy",
    ),
    ("libkernel", "Posix", "IKP8typ0QUk", "Posix::sem_post"),
    ("libkernel", "Posix", "JGMio+21L4c", "mkdir"),
    (
        "libkernel",
        "Posix",
        "JKyG3SWyA10",
        "Posix::pthread_attr_setguardsize",
    ),
    (
        "libkernel",
        "Posix",
        "JNkVVsVDmOk",
        "Posix::pthread_attr_getguardsize",
    ),
    (
        "libkernel",
        "Posix",
        "JarMIy8kKEY",
        "Posix::pthread_attr_setschedpolicy",
    ),
    (
        "libkernel",
        "Posix",
        "Jmi+9w9u0E4",
        "Posix::pthread_create_name_np",
    ),
    (
        "libkernel",
        "Posix",
        "K-jXhbt2gn4",
        "Posix::pthread_mutex_trylock",
    ),
    ("libkernel", "Posix", "KuOmgKoqCdY", "Posix::bind"),
    ("libkernel", "Posix", "NhpspxdjEKU", "nanosleep"),
    (
        "libkernel",
        "Posix",
        "Op8TBGY5KHg",
        "Posix::pthread_cond_wait",
    ),
    ("libkernel", "Posix", "OxhIB8LB-PQ", "Posix::pthread_create"),
    (
        "libkernel",
        "Posix",
        "P41kTWUS3EI",
        "Posix::pthread_getschedparam",
    ),
    ("libkernel", "Posix", "RenI1lL1WFk", "Posix::getsockname"),
    (
        "libkernel",
        "Posix",
        "RtLRV-pBTTY",
        "Posix::pthread_attr_getschedpolicy",
    ),
    (
        "libkernel",
        "Posix",
        "SFxTMOfuCkE",
        "LibKernel::PthreadRwlockTryrdlock",
    ),
    ("libkernel", "Posix", "T8fER+tIGgk", "Posix::select"),
    ("libkernel", "Posix", "TU-d9PfIHPM", "Posix::socket"),
    (
        "libkernel",
        "Posix",
        "Ucsu-OK+els",
        "Posix::pthread_attr_get_np",
    ),
    (
        "libkernel",
        "Posix",
        "UqDGjXA5yUM",
        "LibKernel::Memory::KernelMunmap",
    ),
    (
        "libkernel",
        "Posix",
        "VUT1ZSrHT0I",
        "Posix::pthread_attr_getdetachstate",
    ),
    ("libkernel", "Posix", "WBWzsRifCEA", "Posix::sem_trywait"),
    (
        "libkernel",
        "Posix",
        "WrOLvHU0yQM",
        "Posix::pthread_setspecific",
    ),
    ("libkernel", "Posix", "XVL8So3QJUk", "Posix::connect"),
    (
        "libkernel",
        "Posix",
        "XhWHn6P5R7U",
        "LibKernel::PthreadRwlockTrywrlock",
    ),
    (
        "libkernel",
        "Posix",
        "Xs9hdiD7sAA",
        "Posix::pthread_setschedparam",
    ),
    ("libkernel", "Posix", "YCV5dGGBcCo", "Posix::sem_wait"),
    (
        "libkernel",
        "Posix",
        "YQOfxL4QfeU",
        "LibKernel::Memory::KernelMprotect",
    ),
    ("libkernel", "Posix", "Z4QosVuAsA0", "Posix::pthread_once"),
    (
        "libkernel",
        "Posix",
        "a2P9wYGeZvc",
        "Posix::pthread_setprio",
    ),
    ("libkernel", "Posix", "cDW233RAwWo", "Posix::sem_destroy"),
    ("libkernel", "Posix", "cfwBSQyr5Ys", "cfwBSQyr5Ys"),
    (
        "libkernel",
        "Posix",
        "dQHWEsJtoE4",
        "Posix::pthread_mutexattr_init",
    ),
    (
        "libkernel",
        "Posix",
        "euKRgm0Vn2M",
        "Posix::pthread_attr_setschedparam",
    ),
    ("libkernel", "Posix", "fFxGkxF2bVo", "Posix::setsockopt"),
    ("libkernel", "Posix", "fZOeZIOEmLw", "Posix::send"),
    ("libkernel", "Posix", "h9CcP3J0oVM", "Posix::pthread_join"),
    ("libkernel", "Posix", "ih4CD9-gghM", "Posix::ftruncate"),
    ("libkernel", "Posix", "k+AXqu2-eBc", "getpagesize"),
    ("libkernel", "Posix", "lLMT9vJAck0", "clock_gettime"),
    ("libkernel", "Posix", "lUk6wrGXyMw", "Posix::recvfrom"),
    (
        "libkernel",
        "Posix",
        "lZzFeSxPl08",
        "Posix::pthread_setcancelstate",
    ),
    (
        "libkernel",
        "Posix",
        "ltCfaGr2JGE",
        "Posix::pthread_mutex_destroy",
    ),
    (
        "libkernel",
        "Posix",
        "mDmgMOGVUqg",
        "Posix::pthread_mutexattr_settype",
    ),
    (
        "libkernel",
        "Posix",
        "mkx2fVhNMsg",
        "Posix::pthread_cond_broadcast",
    ),
    (
        "libkernel",
        "Posix",
        "mqULNdimTn0",
        "Posix::pthread_key_create",
    ),
    ("libkernel", "Posix", "n88vx3C5nW8", "gettimeofday"),
    ("libkernel", "Posix", "oBr313PppNE", "Posix::sendto"),
    (
        "libkernel",
        "Posix",
        "oIRFTjoILbg",
        "Posix::pthread_setschedparam",
    ),
    ("libkernel", "Posix", "pDuPEf3m4fI", "Posix::sem_init"),
    ("libkernel", "Posix", "pxnCmagrtao", "Posix::listen"),
    (
        "libkernel",
        "Posix",
        "qlk9pSLsUmM",
        "Posix::pthread_attr_getschedparam",
    ),
    (
        "libkernel",
        "Posix",
        "sIlRvQqsN2Y",
        "Posix::pthread_rwlock_wrlock",
    ),
    ("libkernel", "Posix", "smIj7eqzZE8", "clock_getres"),
    (
        "libkernel",
        "Posix",
        "ttHNfU+qDBU",
        "Posix::pthread_mutex_init",
    ),
    (
        "libkernel",
        "Posix",
        "vQm4fDEsWi8",
        "Posix::pthread_attr_getstack",
    ),
    ("libkernel", "Posix", "w5IHyvahg-o", "Posix::sem_timedwait"),
    (
        "libkernel",
        "Posix",
        "wtkt-teR1so",
        "Posix::pthread_attr_init",
    ),
    ("libkernel", "Posix", "yS8U2TGCe1A", "nanosleep"),
    (
        "libkernel",
        "Posix",
        "ytQULN-nhL4",
        "Posix::pthread_rwlock_init",
    ),
    (
        "libkernel",
        "Posix",
        "zHchY8ft5pk",
        "Posix::pthread_attr_destroy",
    ),
    (
        "libkernel",
        "libkernel",
        "+BzXYkqYeLE",
        "LibKernel::PthreadSetspecific",
    ),
    (
        "libkernel",
        "libkernel",
        "+L98PIbGttk",
        "LibKernel::PthreadRwlockUnlock",
    ),
    (
        "libkernel",
        "libkernel",
        "+r3rMFwItV4",
        "FileSystem::KernelPread",
    ),
    (
        "libkernel",
        "libkernel",
        "-2IRUCO--PM",
        "LibKernel::KernelReadTsc",
    ),
    (
        "libkernel",
        "libkernel",
        "-Wreprtu0Qs",
        "LibKernel::PthreadAttrSetdetachstate",
    ),
    (
        "libkernel",
        "libkernel",
        "-ZR+hG7aDHw",
        "LibKernel::KernelSleep",
    ),
    (
        "libkernel",
        "libkernel",
        "-fA+7ZlGDQs",
        "LibKernel::PthreadAttrGetstacksize",
    ),
    (
        "libkernel",
        "libkernel",
        "-o5uEDpN+oY",
        "LibKernel::KernelConvertUtcToLocaltime",
    ),
    (
        "libkernel",
        "libkernel",
        "-quPa4SEJUw",
        "LibKernel::PthreadAttrGetstack",
    ),
    ("libkernel", "libkernel", "04AjkP0jO9U", "Posix::UmtxOp"),
    (
        "libkernel",
        "libkernel",
        "0NTHN1NKONI",
        "LibKernel::KernelConvertLocaltimeToUtc",
    ),
    (
        "libkernel",
        "libkernel",
        "1-LFLmRFxxM",
        "FileSystem::KernelMkdir",
    ),
    (
        "libkernel",
        "libkernel",
        "12wOHk8ywb0",
        "Semaphore::KernelPollSema",
    ),
    (
        "libkernel",
        "libkernel",
        "14bOACANTBo",
        "Posix::pthread_once",
    ),
    (
        "libkernel",
        "libkernel",
        "188x57JYp0g",
        "Semaphore::KernelCreateSema",
    ),
    (
        "libkernel",
        "libkernel",
        "1FGvU0i9saQ",
        "LibKernel::PthreadMutexattrSetprotocol",
    ),
    (
        "libkernel",
        "libkernel",
        "1G3lF1Gg1k8",
        "FileSystem::KernelOpen",
    ),
    (
        "libkernel",
        "libkernel",
        "1j3S3n-tTW4",
        "LibKernel::KernelGetTscFrequency",
    ),
    (
        "libkernel",
        "libkernel",
        "1jfXLRVzisc",
        "LibKernel::KernelUsleep",
    ),
    (
        "libkernel",
        "libkernel",
        "1tKyG7RlMJo",
        "LibKernel::PthreadGetprio",
    ),
    (
        "libkernel",
        "libkernel",
        "23CPPI1tyBY",
        "EventQueue::KernelGetEventFilter",
    ),
    (
        "libkernel",
        "libkernel",
        "27bAgiJmOh0",
        "Posix::pthread_cond_timedwait",
    ),
    (
        "libkernel",
        "libkernel",
        "2MOy+rUfuhQ",
        "LibKernel::PthreadCondSignal",
    ),
    (
        "libkernel",
        "libkernel",
        "2Of0f+3mhhE",
        "LibKernel::PthreadMutexDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "2Q0z6rnBrTE",
        "LibKernel::PthreadAttrSetstacksize",
    ),
    (
        "libkernel",
        "libkernel",
        "2SKEx6bSq-4",
        "Memory::KernelBatchMap",
    ),
    (
        "libkernel",
        "libkernel",
        "2Tb92quprl0",
        "LibKernel::PthreadCondInit",
    ),
    (
        "libkernel",
        "libkernel",
        "2Z+PpY6CaJg",
        "Posix::pthread_mutex_unlock",
    ),
    (
        "libkernel",
        "libkernel",
        "2pOuoWoCxdk",
        "KernelAioPollRequest",
    ),
    (
        "libkernel",
        "libkernel",
        "3PtV6p3QNX4",
        "LibKernel::PthreadEqual",
    ),
    (
        "libkernel",
        "libkernel",
        "3kg7rT0NQIs",
        "LibKernel::PthreadExit",
    ),
    (
        "libkernel",
        "libkernel",
        "3qxgM4ezETA",
        "LibKernel::PthreadAttrSetaffinity",
    ),
    (
        "libkernel",
        "libkernel",
        "4+h9EzwKF4I",
        "LibKernel::PthreadAttrSetschedpolicy",
    ),
    (
        "libkernel",
        "libkernel",
        "4DM06U2BNEY",
        "Semaphore::KernelCancelSema",
    ),
    (
        "libkernel",
        "libkernel",
        "4J2sUJmuHZQ",
        "LibKernel::KernelGetProcessTime",
    ),
    (
        "libkernel",
        "libkernel",
        "4R6-OvI2cEA",
        "EventQueue::KernelAddUserEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "4czppHBiriw",
        "Semaphore::KernelSignalSema",
    ),
    (
        "libkernel",
        "libkernel",
        "4h6F1LLbTiw",
        "Memory::KernelMapFlexibleMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "4oXYe9Xmk0Q",
        "LibKernel::KernelGetGPI",
    ),
    (
        "libkernel",
        "libkernel",
        "4qGrR6eoP9Y",
        "LibKernel::PthreadDetach",
    ),
    (
        "libkernel",
        "libkernel",
        "4wSze92BhLI",
        "FileSystem::KernelWrite",
    ),
    (
        "libkernel",
        "libkernel",
        "52NcYU9+lEo",
        "FileSystem::KernelRename",
    ),
    (
        "libkernel",
        "libkernel",
        "57ZK+ODEXWY",
        "EventQueue::KernelAddTimerEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "5TgME6AYty4",
        "KernelAioDeleteRequest",
    ),
    (
        "libkernel",
        "libkernel",
        "62KCwEMmzcM",
        "LibKernel::PthreadAttrDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "6ULAa0fq4jA",
        "LibKernel::PthreadRwlockInit",
    ),
    (
        "libkernel",
        "libkernel",
        "6UgtwV+0zb4",
        "LibKernel::PthreadCreate",
    ),
    (
        "libkernel",
        "libkernel",
        "6XG4B33N09g",
        "LibKernel::PthreadYield",
    ),
    ("libkernel", "libkernel", "6Z83sYWFlA8", "LibKernel::exit"),
    ("libkernel", "libkernel", "6c3rCVE-fTU", "LibKernel::open"),
    (
        "libkernel",
        "libkernel",
        "6xVpy0Fdq+I",
        "LibKernel::sigprocmask",
    ),
    (
        "libkernel",
        "libkernel",
        "7H0iTOciTLo",
        "Posix::pthread_mutex_lock",
    ),
    (
        "libkernel",
        "libkernel",
        "7oxv3PPCumo",
        "Memory::KernelReserveVirtualRange",
    ),
    (
        "libkernel",
        "libkernel",
        "7uhBFWRAS60",
        "EventFlag::KernelClearEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "8+s5BzZjxSg",
        "LibKernel::PthreadAttrGetaffinity",
    ),
    (
        "libkernel",
        "libkernel",
        "8OnWXlgQlvo",
        "LibKernel::KernelRtldThreadAtexitDecrement",
    ),
    (
        "libkernel",
        "libkernel",
        "8mql9OcQnd4",
        "EventFlag::KernelDeleteEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "959qrazPIrg",
        "LibKernel::KernelGetProcParam",
    ),
    (
        "libkernel",
        "libkernel",
        "9BcDykPmo1I",
        "LibKernel::get_error_addr",
    ),
    (
        "libkernel",
        "libkernel",
        "9RnL-m0+diQ",
        "LibKernel::PthreadAttrGetsolosched",
    ),
    (
        "libkernel",
        "libkernel",
        "9UK1vLZQft4",
        "LibKernel::PthreadMutexLock",
    ),
    (
        "libkernel",
        "libkernel",
        "9bfdLIyuwCY",
        "Memory::KernelMtypeprotect",
    ),
    ("libkernel", "libkernel", "9eMlfusH4sU", "Posix::flock"),
    (
        "libkernel",
        "libkernel",
        "9lvj5DjHZiA",
        "EventFlag::KernelPollEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "AUXVxWeJU-A",
        "FileSystem::KernelUnlink",
    ),
    ("libkernel", "libkernel", "AqBioC2vF3I", "LibKernel::read"),
    (
        "libkernel",
        "libkernel",
        "B+vc2AO2Zrc",
        "Memory::KernelAllocateMainDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "B2n8aDorSH4",
        "Posix::KernelSyncOnAddressWait32",
    ),
    (
        "libkernel",
        "libkernel",
        "BB+kb08Tl9A",
        "LibKernel::PthreadRwlockDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "BHouLQzh0X0",
        "Memory::KernelDirectMemoryQuery",
    ),
    (
        "libkernel",
        "libkernel",
        "BNowx2l588E",
        "LibKernel::KernelGetProcessTimeCounterFrequency",
    ),
    (
        "libkernel",
        "libkernel",
        "BQQniolj9tQ",
        "Memory::KernelMapDirectMemory2",
    ),
    (
        "libkernel",
        "libkernel",
        "BmMjYxmew1w",
        "LibKernel::PthreadCondTimedwait",
    ),
    (
        "libkernel",
        "libkernel",
        "BohYr-F7-is",
        "Memory::KernelSetPrtAperture",
    ),
    (
        "libkernel",
        "libkernel",
        "BpFoboUJoZU",
        "EventFlag::KernelCreateEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "Bvn74vj6oLo",
        "LibKernel::PthreadAttrSetstack",
    ),
    (
        "libkernel",
        "libkernel",
        "C0f7TJcbfac",
        "Memory::KernelAvailableDirectMemorySize",
    ),
    ("libkernel", "libkernel", "C2kJ-byS5rM", "Posix::pwrite"),
    (
        "libkernel",
        "libkernel",
        "C36iRE0F5sE",
        "Semaphore::PthreadSemWait",
    ),
    (
        "libkernel",
        "libkernel",
        "CBNtXOoef-E",
        "Posix::sched_get_priority_max",
    ),
    (
        "libkernel",
        "libkernel",
        "Cg4srZ6TKbU",
        "FileSystem::KernelRead",
    ),
    (
        "libkernel",
        "libkernel",
        "D0OdFMjp46I",
        "EventQueue::KernelCreateEqueue",
    ),
    (
        "libkernel",
        "libkernel",
        "DGMG3JshrZU",
        "Memory::KernelSetVirtualRangeName",
    ),
    (
        "libkernel",
        "libkernel",
        "DLORcroUqbc",
        "LibKernel::KernelGetOpenPsId",
    ),
    ("libkernel", "libkernel", "DRuBt2pvICk", "LibKernel::read"),
    (
        "libkernel",
        "libkernel",
        "DjpBvGlaWbQ",
        "Semaphore::PthreadSemGetvalue",
    ),
    (
        "libkernel",
        "libkernel",
        "Dk6FC-TI+7Q",
        "LibKernel::PthreadAttrSetsolosched",
    ),
    (
        "libkernel",
        "libkernel",
        "DzES9hQF4f4",
        "LibKernel::PthreadAttrSetschedparam",
    ),
    (
        "libkernel",
        "libkernel",
        "EI-5-jlq2dE",
        "LibKernel::PthreadGetthreadid",
    ),
    (
        "libkernel",
        "libkernel",
        "EgmLo6EWgso",
        "LibKernel::PthreadRwlockUnlock",
    ),
    (
        "libkernel",
        "libkernel",
        "EjllaAqAPZo",
        "Posix::pthread_condattr_setclock",
    ),
    (
        "libkernel",
        "libkernel",
        "El+cQ20DynU",
        "LibKernel::PthreadAttrSetguardsize",
    ),
    (
        "libkernel",
        "libkernel",
        "EotR8a3ASf4",
        "LibKernel::PthreadSelf",
    ),
    (
        "libkernel",
        "libkernel",
        "F+yfmduIBB8",
        "LibKernel::PthreadAttrSetstackaddr",
    ),
    (
        "libkernel",
        "libkernel",
        "F6e0kwo4cnk",
        "EventQueue::KernelTriggerUserEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "F8bUHwAG284",
        "LibKernel::PthreadMutexattrInit",
    ),
    (
        "libkernel",
        "libkernel",
        "FJmglmTMdr4",
        "LibKernel::getargv",
    ),
    (
        "libkernel",
        "libkernel",
        "FN4gaPmuFV8",
        "FileSystem::KernelWrite",
    ),
    (
        "libkernel",
        "libkernel",
        "FXPWHNk8Of0",
        "LibKernel::PthreadAttrGetschedparam",
    ),
    (
        "libkernel",
        "libkernel",
        "Fjc4-n1+y2g",
        "LibKernel::elf_phdr_match_addr",
    ),
    (
        "libkernel",
        "libkernel",
        "Ft3EtsZzAoY",
        "KernelAioDeleteRequests",
    ),
    ("libkernel", "libkernel", "FxVZqBAA7ks", "LibKernel::write"),
    (
        "libkernel",
        "libkernel",
        "GBUY7ywdULE",
        "LibKernel::PthreadRename",
    ),
    (
        "libkernel",
        "libkernel",
        "GEnUkDZoUwY",
        "Semaphore::PthreadSemInit",
    ),
    (
        "libkernel",
        "libkernel",
        "H2a+IN9TP0E",
        "Semaphore::PthreadSemTrywait",
    ),
    (
        "libkernel",
        "libkernel",
        "Hc4CaR6JBL0",
        "Posix::KernelSyncOnAddressWait",
    ),
    (
        "libkernel",
        "libkernel",
        "HgX7+AORI58",
        "KernelAioSubmitReadCommands",
    ),
    ("libkernel", "libkernel", "HoLVWNanBBc", "LibKernel::getpid"),
    (
        "libkernel",
        "libkernel",
        "How7B8Oet6k",
        "LibKernel::PthreadGetname",
    ),
    (
        "libkernel",
        "libkernel",
        "IOnSvHzqu6A",
        "EventFlag::KernelSetEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "IWIBBdTHit4",
        "Memory::KernelMapFlexibleMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "IafI2PxcPnQ",
        "LibKernel::PthreadMutexTimedlock",
    ),
    (
        "libkernel",
        "libkernel",
        "Ij+ryuEClXQ",
        "EventQueue::KernelDeleteAmprSystemEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "Io9+nTKXZtA",
        "Posix::pthread_mutex_timedlock",
    ),
    (
        "libkernel",
        "libkernel",
        "J+LF6LwObXU",
        "EventQueue::KernelDeleteHRTimerEvent",
    ),
    ("libkernel", "libkernel", "JGMio+21L4c", "Posix::mkdir"),
    (
        "libkernel",
        "libkernel",
        "JGgj7Uvrl+A",
        "LibKernel::PthreadCondBroadcast",
    ),
    (
        "libkernel",
        "libkernel",
        "JTvBflhYazQ",
        "EventFlag::KernelWaitEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "JaRMy+QcpeU",
        "LibKernel::PthreadAttrGetdetachstate",
    ),
    (
        "libkernel",
        "libkernel",
        "KOF-oJbQVvc",
        "KernelAioWaitRequest",
    ),
    (
        "libkernel",
        "libkernel",
        "L-Q3LEjIbgA",
        "Memory::KernelMapDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "L0v2Go5jOuM",
        "Memory::KernelGetPrtAperture",
    ),
    (
        "libkernel",
        "libkernel",
        "LJDwdSNTnDg",
        "EventQueue::KernelDeleteUserEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "LXo1tpFqJGs",
        "Memory::KernelMemoryPoolDecommit",
    ),
    (
        "libkernel",
        "libkernel",
        "La9uyZv4Kvw",
        "LibKernel::KernelGetAvailableCpumask",
    ),
    (
        "libkernel",
        "libkernel",
        "LwG8g3niqwA",
        "LibKernel::KernelDlsym",
    ),
    (
        "libkernel",
        "libkernel",
        "MBuItvba6z8",
        "Memory::KernelReleaseDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "NH6xARDOVv8",
        "LibKernel::KernelGetOperationMode",
    ),
    ("libkernel", "libkernel", "NNtFaKJbPt0", "LibKernel::close"),
    (
        "libkernel",
        "libkernel",
        "NcaWUxfMNIQ",
        "Memory::KernelMapNamedDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "OAmWq+OHSjw",
        "LibKernel::PthreadSetcancelstate",
    ),
    (
        "libkernel",
        "libkernel",
        "OMDRKKAZ8I4",
        "LibKernel::KernelDebugRaiseException",
    ),
    (
        "libkernel",
        "libkernel",
        "Op8TBGY5KHg",
        "Posix::pthread_cond_wait",
    ),
    (
        "libkernel",
        "libkernel",
        "Ou3iL1abvng",
        "LibKernel::stack_chk_fail",
    ),
    (
        "libkernel",
        "libkernel",
        "Ox9i0c7L5w0",
        "LibKernel::PthreadRwlockRdlock",
    ),
    ("libkernel", "libkernel", "Oy6IpwgtYOk", "Posix::lseek"),
    (
        "libkernel",
        "libkernel",
        "PZQhiiLXRFs",
        "Posix::KernelSyncOnAddressWait64",
    ),
    (
        "libkernel",
        "libkernel",
        "PZku4ZrXJqg",
        "EventFlag::KernelCancelEventFlag",
    ),
    (
        "libkernel",
        "libkernel",
        "PrdHuuDekhY",
        "LibKernel::PthreadKeyDelete",
    ),
    (
        "libkernel",
        "libkernel",
        "Q0qr9AyqJSk",
        "EventQueue::KernelGetEventFflags",
    ),
    (
        "libkernel",
        "libkernel",
        "QBi7HCK03hw",
        "LibKernel::KernelClockGettime",
    ),
    (
        "libkernel",
        "libkernel",
        "QKd0qM58Qes",
        "LibKernel::KernelStopUnloadModule",
    ),
    (
        "libkernel",
        "libkernel",
        "QcteRwbsnV0",
        "LibKernel::KernelUsleep",
    ),
    (
        "libkernel",
        "libkernel",
        "Qhv5ARAoOEc",
        "LibKernel::KernelRemoveExceptionHandler",
    ),
    (
        "libkernel",
        "libkernel",
        "QvsZxomvUHs",
        "LibKernel::KernelNanosleep",
    ),
    (
        "libkernel",
        "libkernel",
        "R1Jvn8bSCW8",
        "Semaphore::KernelDeleteSema",
    ),
    (
        "libkernel",
        "libkernel",
        "R74tt43xP6k",
        "EventQueue::KernelAddHRTimerEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "RXXqi4CtF8w",
        "LibKernel::PthreadCondDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "RpQJJVKTiFM",
        "LibKernel::KernelGetModuleInfoForUnwind",
    ),
    (
        "libkernel",
        "libkernel",
        "Ru36fiTtJzA",
        "LibKernel::PthreadAttrGetstackaddr",
    ),
    (
        "libkernel",
        "libkernel",
        "T72hz6ffq08",
        "LibKernel::PthreadYield",
    ),
    (
        "libkernel",
        "libkernel",
        "Tz4RNUCBbGI",
        "LibKernel::KernelRtldThreadAtexitIncrement",
    ),
    (
        "libkernel",
        "libkernel",
        "UK2Tl2DWUns",
        "FileSystem::KernelClose",
    ),
    (
        "libkernel",
        "libkernel",
        "UTXzJbWhhTE",
        "LibKernel::PthreadAttrSetstacksize",
    ),
    (
        "libkernel",
        "libkernel",
        "Uu-iDFC9aUc",
        "EventQueue::KernelGetEventError",
    ),
    (
        "libkernel",
        "libkernel",
        "VAzswvTOCzI",
        "FileSystem::KernelUnlink",
    ),
    (
        "libkernel",
        "libkernel",
        "Vwc+L05e6oE",
        "Semaphore::PthreadSemDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "Vzl66WmfLvk",
        "Memory::KernelMemoryPoolCommit",
    ),
    (
        "libkernel",
        "libkernel",
        "W0Hpm2X0uPE",
        "LibKernel::PthreadSetprio",
    ),
    (
        "libkernel",
        "libkernel",
        "WDszmSbWuDk",
        "EventQueue::KernelAddUserEventEdge",
    ),
    (
        "libkernel",
        "libkernel",
        "WFcfL2lzido",
        "Memory::KernelQueryMemoryProtection",
    ),
    (
        "libkernel",
        "libkernel",
        "WKAXJ4XBPQ4",
        "LibKernel::PthreadCondWait",
    ),
    (
        "libkernel",
        "libkernel",
        "WhCc1w3EhSI",
        "LibKernel::KernelSetThreadAtexitReport",
    ),
    (
        "libkernel",
        "libkernel",
        "WkwEd3N7w0Y",
        "LibKernel::KernelInstallExceptionHandler",
    ),
    (
        "libkernel",
        "libkernel",
        "XD3mDeybCnk",
        "LibKernel::PthreadRwlockTryrdlock",
    ),
    (
        "libkernel",
        "libkernel",
        "XQ8C8y+de+E",
        "KernelAioSubmitWriteCommands",
    ),
    (
        "libkernel",
        "libkernel",
        "Xjoosiw+XPI",
        "LibKernel::KernelUuidCreate",
    ),
    (
        "libkernel",
        "libkernel",
        "YN878uKRBbE",
        "Memory::KernelMemoryPoolBatch",
    ),
    (
        "libkernel",
        "libkernel",
        "Z4QosVuAsA0",
        "Posix::pthread_once",
    ),
    (
        "libkernel",
        "libkernel",
        "Zxa0VhQVTsk",
        "Semaphore::KernelWaitSema",
    ),
    (
        "libkernel",
        "libkernel",
        "aI+OeCz8xrQ",
        "LibKernel::PthreadSelf",
    ),
    (
        "libkernel",
        "libkernel",
        "aNz11fnnzi4",
        "Memory::KernelAvailableFlexibleMemorySize",
    ),
    (
        "libkernel",
        "libkernel",
        "adh--6nIqTk",
        "LibKernel::PthreadRwlockTimedwrlock",
    ),
    (
        "libkernel",
        "libkernel",
        "aishVAiFaYM",
        "Semaphore::PthreadSemPost",
    ),
    (
        "libkernel",
        "libkernel",
        "bBfz7kMF2Ho",
        "EventQueue::KernelAddAmprEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "bIHoZCTomsI",
        "LibKernel::PthreadRwlockTrywrlock",
    ),
    (
        "libkernel",
        "libkernel",
        "bMmid3pfyjo",
        "EventQueue::KernelDeleteAmprEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "bY-PO6JhzhQ",
        "FileSystem::KernelClose",
    ),
    (
        "libkernel",
        "libkernel",
        "bnZxYgAFeA0",
        "LibKernel::KernelGetSanitizerNewReplaceExternal",
    ),
    (
        "libkernel",
        "libkernel",
        "bt3CTBKmGyI",
        "LibKernel::PthreadSetaffinity",
    ),
    (
        "libkernel",
        "libkernel",
        "bvD+95Q6asU",
        "Memory::KernelMemoryPoolGetBlockStats",
    ),
    (
        "libkernel",
        "libkernel",
        "cQke9UuBQOk",
        "Memory::KernelMunmap",
    ),
    (
        "libkernel",
        "libkernel",
        "ca7v6Cxulzs",
        "LibKernel::KernelSetGPO",
    ),
    (
        "libkernel",
        "libkernel",
        "cmo1RIYva9o",
        "LibKernel::PthreadMutexInit",
    ),
    (
        "libkernel",
        "libkernel",
        "crb5j7mkk1c",
        "LibKernel::KernelIsSignalReturn",
    ),
    (
        "libkernel",
        "libkernel",
        "dJcuQVn6-Iw",
        "Posix::pthread_condattr_destroy",
    ),
    (
        "libkernel",
        "libkernel",
        "eV9wAD2riIA",
        "FileSystem::KernelStat",
    ),
    (
        "libkernel",
        "libkernel",
        "eXbUSpEaTsA",
        "LibKernel::PthreadAttrSetinheritsched",
    ),
    (
        "libkernel",
        "libkernel",
        "ejekcaNQNq0",
        "LibKernel::KernelGettimeofday",
    ),
    (
        "libkernel",
        "libkernel",
        "eoht7mQOCmo",
        "LibKernel::PthreadGetspecific",
    ),
    ("libkernel", "libkernel", "ezv-RSBNKqI", "Posix::pread"),
    (
        "libkernel",
        "libkernel",
        "f7KBOafysXo",
        "LibKernel::KernelGetModuleInfoFromAddr",
    ),
    (
        "libkernel",
        "libkernel",
        "fTx66l5iWIA",
        "LibKernel::KernelFsync",
    ),
    (
        "libkernel",
        "libkernel",
        "fgxnMeTNUtY",
        "LibKernel::KernelGetProcessTimeCounter",
    ),
    (
        "libkernel",
        "libkernel",
        "fjN6NQHhK8k",
        "Semaphore::PthreadSemTimedwait",
    ),
    (
        "libkernel",
        "libkernel",
        "fzyMKs9kim0",
        "EventQueue::KernelWaitEqueue",
    ),
    (
        "libkernel",
        "libkernel",
        "g+PZd2hiacg",
        "LibKernel::PthreadCondDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "g0VTBxfJyu0",
        "LibKernel::KernelGetCurrentCpu",
    ),
    (
        "libkernel",
        "libkernel",
        "geDaqgH9lTg",
        "LibKernel::PthreadKeyCreate",
    ),
    (
        "libkernel",
        "libkernel",
        "h-OifiouBd8",
        "LibKernel::PthreadRwlockattrSettype",
    ),
    (
        "libkernel",
        "libkernel",
        "hwVSPCmp5tM",
        "Memory::KernelCheckedReleaseDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "i2ifZ3fS2fo",
        "LibKernel::PthreadRwlockattrDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "iGjsr1WAtI0",
        "LibKernel::PthreadRwlockRdlock",
    ),
    (
        "libkernel",
        "libkernel",
        "iKJMWrAumPE",
        "LibKernel::getargc",
    ),
    (
        "libkernel",
        "libkernel",
        "iMp8QpE+XO4",
        "LibKernel::PthreadMutexattrSettype",
    ),
    (
        "libkernel",
        "libkernel",
        "iPtZRWICjrM",
        "LibKernel::PthreadRwlockTimedrdlock",
    ),
    (
        "libkernel",
        "libkernel",
        "il03nluKfMk",
        "LibKernel::KernelRaiseException",
    ),
    (
        "libkernel",
        "libkernel",
        "j2AIqSqJP0w",
        "FileSystem::KernelGetdents",
    ),
    (
        "libkernel",
        "libkernel",
        "jh+8XiK4LeE",
        "Memory::KernelIsAddressSanitizerEnabled",
    ),
    (
        "libkernel",
        "libkernel",
        "jpFjmgAC5AE",
        "EventQueue::KernelDeleteEqueue",
    ),
    (
        "libkernel",
        "libkernel",
        "k+AXqu2-eBc",
        "Posix::getpagesize",
    ),
    (
        "libkernel",
        "libkernel",
        "kBJzF8x4SyE",
        "Memory::KernelBatchMap2",
    ),
    (
        "libkernel",
        "libkernel",
        "kBwCPsYX-m4",
        "FileSystem::KernelFstat",
    ),
    (
        "libkernel",
        "libkernel",
        "kDh-NfxgMtE",
        "LibKernel::PthreadCondSignal",
    ),
    (
        "libkernel",
        "libkernel",
        "kOcnerypnQA",
        "LibKernel::KernelGettimezone",
    ),
    (
        "libkernel",
        "libkernel",
        "kbw4UHHSYy0",
        "LibKernel::pthread_cxa_finalize",
    ),
    (
        "libkernel",
        "libkernel",
        "kwGyyjohI50",
        "EventQueue::KernelGetEventData",
    ),
    (
        "libkernel",
        "libkernel",
        "lLMT9vJAck0",
        "LibKernel::clock_gettime",
    ),
    (
        "libkernel",
        "libkernel",
        "lgK+oIWkJyA",
        "KernelAioWaitRequests",
    ),
    (
        "libkernel",
        "libkernel",
        "m0iS6jNsXds",
        "Posix::sched_get_priority_min",
    ),
    (
        "libkernel",
        "libkernel",
        "m5-2bsNfv7s",
        "LibKernel::PthreadCondattrInit",
    ),
    (
        "libkernel",
        "libkernel",
        "mBd4AfLP+u8",
        "FileSystem::KernelPwritev",
    ),
    (
        "libkernel",
        "libkernel",
        "mJ7aghmgvfc",
        "EventQueue::KernelGetEventId",
    ),
    (
        "libkernel",
        "libkernel",
        "mKoTx03HRWA",
        "Posix::pthread_condattr_init",
    ),
    (
        "libkernel",
        "libkernel",
        "mL8NDH86iQI",
        "Memory::KernelMapNamedFlexibleMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "mkgXxsoxWHg",
        "Memory::KernelClearVirtualRangeName",
    ),
    (
        "libkernel",
        "libkernel",
        "mkx2fVhNMsg",
        "Posix::pthread_cond_broadcast",
    ),
    ("libkernel", "libkernel", "mqQMh1zPPT8", "Posix::fstat"),
    (
        "libkernel",
        "libkernel",
        "mqdNorrB+gI",
        "LibKernel::PthreadRwlockWrlock",
    ),
    (
        "libkernel",
        "libkernel",
        "n1-v6FgU7MQ",
        "Memory::KernelConfiguredFlexibleMemorySize",
    ),
    (
        "libkernel",
        "libkernel",
        "n88vx3C5nW8",
        "Posix::gettimeofday",
    ),
    (
        "libkernel",
        "libkernel",
        "nKWi-N2HBV4",
        "FileSystem::KernelPwrite",
    ),
    (
        "libkernel",
        "libkernel",
        "naInUjYt3so",
        "FileSystem::KernelRmdir",
    ),
    (
        "libkernel",
        "libkernel",
        "nsYoNRywwNg",
        "LibKernel::PthreadAttrInit",
    ),
    (
        "libkernel",
        "libkernel",
        "nu4a0-arQis",
        "KernelAioInitializeParam",
    ),
    (
        "libkernel",
        "libkernel",
        "o69RpYO-Mu0",
        "LibKernel::PthreadCondSignalto",
    ),
    (
        "libkernel",
        "libkernel",
        "o7O4z3jwKzo",
        "KernelAioPollRequests",
    ),
    (
        "libkernel",
        "libkernel",
        "oib76F-12fk",
        "FileSystem::KernelLseek",
    ),
    (
        "libkernel",
        "libkernel",
        "onNY9Byn-W8",
        "LibKernel::PthreadJoin",
    ),
    (
        "libkernel",
        "libkernel",
        "p5EcQeEeJAE",
        "LibKernel::KernelRtldSetApplicationHeapAPI",
    ),
    (
        "libkernel",
        "libkernel",
        "pB-yGZ2nQ9o",
        "LibKernel::KernelSetThreadAtexitCount",
    ),
    (
        "libkernel",
        "libkernel",
        "pO96TwzOm5E",
        "Memory::KernelGetDirectMemorySize",
    ),
    (
        "libkernel",
        "libkernel",
        "pU-QydtGcGY",
        "Memory::KernelMemoryPoolReserve",
    ),
    (
        "libkernel",
        "libkernel",
        "py6L8jiVAN8",
        "LibKernel::KernelGetSanitizerMallocReplaceExternal",
    ),
    (
        "libkernel",
        "libkernel",
        "q2y-wDIVWZA",
        "Posix::KernelSyncOnAddressWake",
    ),
    (
        "libkernel",
        "libkernel",
        "qBDmpCyGssE",
        "LibKernel::PthreadCancel",
    ),
    (
        "libkernel",
        "libkernel",
        "qCSfqDILlns",
        "Memory::KernelMemoryPoolExpand",
    ),
    (
        "libkernel",
        "libkernel",
        "qsdmgXjqSgk",
        "LibKernel::PthreadRwlockattrDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "rNhWz+lvOMU",
        "LibKernel::KernelSetThreadDtors",
    ),
    (
        "libkernel",
        "libkernel",
        "rTXw65xmLIA",
        "Memory::KernelAllocateDirectMemory",
    ),
    (
        "libkernel",
        "libkernel",
        "rVjRvHJ0X6c",
        "Memory::KernelVirtualQuery",
    ),
    (
        "libkernel",
        "libkernel",
        "rcrVFJsQWRY",
        "LibKernel::PthreadGetaffinity",
    ),
    (
        "libkernel",
        "libkernel",
        "sCJd99Phct0",
        "LibKernel::PthreadSetcanceltype",
    ),
    (
        "libkernel",
        "libkernel",
        "sIlRvQqsN2Y",
        "LibKernel::PthreadRwlockWrlock",
    ),
    (
        "libkernel",
        "libkernel",
        "smWEktiyyG0",
        "LibKernel::PthreadMutexattrDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "tU5e3f9gSiU",
        "LibKernel::KernelIsTrinityMode",
    ),
    (
        "libkernel",
        "libkernel",
        "tZ2yplY8MBY",
        "Memory::KernelGetPageTableStats",
    ),
    (
        "libkernel",
        "libkernel",
        "taRWhTJFTgE",
        "FileSystem::KernelGetdirentries",
    ),
    (
        "libkernel",
        "libkernel",
        "tn3VlD0hG60",
        "LibKernel::PthreadMutexUnlock",
    ),
    (
        "libkernel",
        "libkernel",
        "txHtngJ+eyc",
        "LibKernel::PthreadAttrGetguardsize",
    ),
    (
        "libkernel",
        "libkernel",
        "uWyW3v98sU4",
        "FileSystem::KernelCheckReachability",
    ),
    (
        "libkernel",
        "libkernel",
        "upoVrzMHFeE",
        "LibKernel::PthreadMutexTrylock",
    ),
    (
        "libkernel",
        "libkernel",
        "uvT2iYBBnkY",
        "LibKernel::KernelSync",
    ),
    (
        "libkernel",
        "libkernel",
        "vNe1w4diLCs",
        "LibKernel::tls_get_addr",
    ),
    (
        "libkernel",
        "libkernel",
        "vSMAm3cxYTY",
        "Memory::KernelMprotect",
    ),
    (
        "libkernel",
        "libkernel",
        "vYU8P9Td2Zo",
        "KernelAioInitializeImpl",
    ),
    (
        "libkernel",
        "libkernel",
        "vuae5JPNt9A",
        "EventQueue::KernelAddAmprSystemEvent",
    ),
    (
        "libkernel",
        "libkernel",
        "vz+pg2zdopI",
        "EventQueue::KernelGetEventUserData",
    ),
    (
        "libkernel",
        "libkernel",
        "wRYVA5Zolso",
        "LibKernel::KernelClockGetres",
    ),
    (
        "libkernel",
        "libkernel",
        "waPcxYiR3WA",
        "LibKernel::PthreadCondattrDestroy",
    ),
    (
        "libkernel",
        "libkernel",
        "wuCroIGjt2g",
        "FileSystem::KernelOpen",
    ),
    (
        "libkernel",
        "libkernel",
        "wzvqT4UqKX8",
        "LibKernel::KernelLoadStartModule",
    ),
    (
        "libkernel",
        "libkernel",
        "x1X76arYMxU",
        "LibKernel::PthreadAttrGet",
    ),
    (
        "libkernel",
        "libkernel",
        "xFebsA4YsFI",
        "LibKernel::PthreadRwlockattrInit",
    ),
    (
        "libkernel",
        "libkernel",
        "yDBwVAolDgg",
        "Memory::KernelIsStack",
    ),
    (
        "libkernel",
        "libkernel",
        "yOfGg-I1ZII",
        "LibKernel::PthreadRwlockattrInit",
    ),
    (
        "libkernel",
        "libkernel",
        "yTj62I7kw4s",
        "FileSystem::KernelPreadv",
    ),
    ("libkernel", "libkernel", "z0dtnPxYgtg", "chmod"),
    (
        "libkernel",
        "libkernel",
        "zE-wXIZjLoM",
        "LibKernel::KernelDebugRaiseExceptionOnReleaseMode",
    ),
    (
        "libkernel",
        "libkernel_write_throttling",
        "YFC3dBBipj8",
        "WriteThrottlingStub",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_library_categories_are_independent_and_accept_only_one_exact_prefix() {
        for library in [b"Http2".as_slice(), b"libSceHttp2"] {
            assert_eq!(library_category(library), Some("Http2"));
            assert_eq!(
                hint(b"original_module", library, b"AAAAAAAAAAA"),
                (None, "unlisted-reference")
            );
            assert_eq!(
                exact_name(b"original_module", library, b"AAAAAAAAAAA", (1, 1, 1)),
                None
            );
        }
        for library in [
            b"http2".as_slice(),
            b"libsceHttp2",
            b"libSCEHttp2",
            b"Http2_private",
            b"libSceHttp2_private",
            b"libScelibSceHttp2",
            b"original-private-library",
            b"libSce",
            b"",
            b"libSceHttp2\tprivate",
            b"\xffHttp2",
        ] {
            assert_eq!(library_category(library), None);
        }
        assert_eq!(library_category(b"libkernel"), Some("libkernel"));
    }

    #[test]
    fn public_library_categories_return_protocol_safe_static_catalogue_storage() {
        for (_, library, _, _) in NAMES.iter().copied().chain(extended::identities()) {
            // The input allocation is dropped before inspecting the returned
            // category. Its static lifetime must not depend on those bytes.
            let category = {
                let input = library.as_bytes().to_vec();
                library_category(&input).unwrap()
            };
            assert_eq!(category, library.strip_prefix("libSce").unwrap_or(library));
            assert!(!category.is_empty() && category.len() <= 128);
            assert!(category.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-' | b'.')
            }));
            assert!(
                NAMES
                    .iter()
                    .copied()
                    .chain(extended::identities())
                    .any(|(_, known, _, _)| {
                        let known = known.strip_prefix("libSce").unwrap_or(known);
                        std::ptr::eq(category, known)
                    })
            );
        }
    }

    #[test]
    fn sharp_http2_references_require_the_public_library_and_never_become_exact_names() {
        for (nid, name) in [
            (b"09tk+kIA1Ns".as_slice(), "sceHttp2SetMinSslVersion"),
            (b"N4UfjvWJsMw".as_slice(), "sceHttp2CreateCookieBox"),
            (b"jrVHsKCXA0g".as_slice(), "sceHttp2SetCookieBox"),
        ] {
            assert_eq!(
                hint(b"original_module", b"libSceHttp2", nid),
                (Some(name), "library-reference")
            );
            // No normalization of the source's library or invented module identity.
            for library in [b"Http2".as_slice(), b"libscehttp2", b"unknown"] {
                assert_eq!(
                    hint(b"original_module", library, nid),
                    (None, "unlisted-reference")
                );
            }
            for module in [b"Http2".as_slice(), b"libSceHttp2", b"original_module"] {
                for versions in [(1, 1, 1), (2, 1, 1), (1, 1, 2)] {
                    assert_eq!(exact_name(module, b"libSceHttp2", nid, versions), None);
                }
            }
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            );
        }
        assert_eq!(
            hint(b"original_module", b"libSceHttp2", b"AAAAAAAAAAA"),
            (None, "unlisted-reference")
        );
    }

    #[test]
    fn sharp_http2_conflicts_refuse_and_existing_catalogue_labels_keep_their_qualification() {
        let conflicting = [(
            "original_module",
            "libSceHttp2",
            "09tk+kIA1Ns",
            "OriginalConflictingLabel",
        )];
        assert_eq!(
            hint_from(
                b"original_module",
                b"libSceHttp2",
                b"09tk+kIA1Ns",
                conflicting.into_iter(),
            ),
            (None, "ambiguous-reference")
        );
        let same = [(
            "original_module",
            "libSceHttp2",
            "09tk+kIA1Ns",
            "sceHttp2SetMinSslVersion",
        )];
        assert_eq!(
            hint_from(
                b"original_module",
                b"libSceHttp2",
                b"09tk+kIA1Ns",
                same.into_iter(),
            ),
            (Some("sceHttp2SetMinSslVersion"), "descriptor-version")
        );
        assert_eq!(
            hint_from(
                b"other_module",
                b"libSceHttp2",
                b"09tk+kIA1Ns",
                same.into_iter(),
            ),
            (Some("sceHttp2SetMinSslVersion"), "different-scope")
        );
        assert_eq!(
            DIAGNOSTIC_WORK,
            (2 * (NAMES.len() as u64 + extended::COUNT as u64) + 3 + name_hashes::COUNT as u64)
                * 256
        );
    }

    #[test]
    fn public_name_hashes_never_claim_identity_and_conflicting_labels_refuse() {
        let nid = b"+KDWny9Y-6k";
        let name = "sceContentExportTerm";
        for (module, library) in [
            (
                b"original-module".as_slice(),
                b"original-library".as_slice(),
            ),
            (b"libSceContentExport", b"libSceContentExport"),
        ] {
            assert_eq!(
                hint(module, library, nid),
                (Some(name), "public-name-hash-reference")
            );
            for versions in [(1, 1, 1), (2, 1, 1), (1, 1, 2)] {
                assert_eq!(exact_name(module, library, nid, versions), None);
            }
        }
        let conflicting = [(
            "original-module",
            "original-library",
            "+KDWny9Y-6k",
            "ConflictingPublicLabel",
        )];
        assert_eq!(
            hint_from(
                b"original-module",
                b"original-library",
                nid,
                conflicting.into_iter()
            ),
            (None, "ambiguous-reference")
        );
        let same = [("original-module", "original-library", "+KDWny9Y-6k", name)];
        assert_eq!(
            hint_from(
                b"original-module",
                b"original-library",
                nid,
                same.into_iter()
            ),
            (Some(name), "descriptor-version")
        );
        assert_eq!(
            hint_from(b"another-module", b"another-library", nid, same.into_iter()),
            (Some(name), "different-scope")
        );
    }

    #[test]
    fn reference_hints_do_not_claim_exact_scope_or_choose_ambiguous_names() {
        assert_eq!(
            hint(b"libkernel", b"libkernel", b"sIlRvQqsN2Y"),
            (Some("LibKernel::PthreadRwlockWrlock"), "descriptor-version")
        );
        assert_eq!(
            hint(b"unknown", b"unknown", b"sIlRvQqsN2Y"),
            (None, "ambiguous-reference")
        );
        assert_eq!(
            hint(b"libkernel", b"libkernel", b"AAAAAAAAAAA"),
            (None, "unlisted-reference")
        );
        let mut unique = false;
        for (m, l, n, name) in NAMES {
            if hint(b"unknown", b"unknown", n.as_bytes()) == (Some(*name), "different-scope") {
                assert_eq!(
                    hint(m.as_bytes(), l.as_bytes(), n.as_bytes()).0,
                    Some(*name)
                );
                unique = true;
                break;
            }
        }
        assert!(unique);
    }

    #[test]
    fn public_names_are_sorted_unique_scoped_and_protocol_safe() {
        for pair in NAMES.windows(2) {
            assert!((pair[0].0, pair[0].1, pair[0].2) < (pair[1].0, pair[1].1, pair[1].2));
        }
        for (module, library, nid, name) in NAMES {
            assert!(
                name.len() <= 128
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b':')
            );
            assert_eq!(
                lookup(module.as_bytes(), library.as_bytes(), nid.as_bytes()),
                Some(*name)
            );
        }
        assert_eq!(
            lookup(b"libkernel", b"libkernel", b"sIlRvQqsN2Y"),
            Some("LibKernel::PthreadRwlockWrlock")
        );
        assert_eq!(
            lookup(b"libkernel", b"Posix", b"sIlRvQqsN2Y"),
            Some("Posix::pthread_rwlock_wrlock")
        );
        assert_eq!(lookup(b"libkernel", b"unlisted", b"sIlRvQqsN2Y"), None);
        assert_eq!(lookup(b"unlisted", b"libkernel", b"sIlRvQqsN2Y"), None);
        assert_eq!(lookup(b"libkernel", b"libkernel", b"AAAAAAAAAAA"), None);
    }
}
