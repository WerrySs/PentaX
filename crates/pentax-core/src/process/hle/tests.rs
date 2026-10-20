// SPDX-License-Identifier: GPL-2.0-or-later

//! Original provider, binding, preparation and gateway tests. Every image is
//! generated here or by the factory; no external or game bytes are read.

mod binding;
mod dependency_flag;
mod gateway;
mod heap_trace;
mod strings;

use super::catalog::KnownFunction;
use super::fixture::{CODE, CONSUMER_BIAS, Consumer, PROVIDER_BIAS};
use super::*;
use pentax_linker::ModuleInput;
use pentax_loader::open_executable;

fn providers() -> GeneratedProviders {
    build_hle_providers(
        HleProfile::KytyObservedLibcInternalV1,
        &HleLimits::default(),
    )
    .unwrap()
}

fn minimal_consumer() -> Consumer {
    Consumer::importing(vec![(CODE, vec![0xcc])])
}

fn placement() -> ProviderPlacement {
    ProviderPlacement {
        load_bias: PROVIDER_BIAS,
    }
}

/// Plan `images` (consumer first, at their biases) plus the generated provider.
fn plan_with<'a>(
    images: &[(&'a [u8], u64)],
    providers: &'a GeneratedProviders,
) -> Result<HleSessionPlan<'a>, HleError> {
    let inputs = images
        .iter()
        .map(|&(bytes, bias)| ModuleInput::new(open_executable(bytes).unwrap(), bias))
        .collect::<Vec<_>>();
    plan_hle_session(&inputs, providers, placement(), &HleLimits::default())
}

fn plan_one<'a>(
    consumer: &'a [u8],
    providers: &'a GeneratedProviders,
) -> Result<HleSessionPlan<'a>, HleError> {
    plan_with(&[(consumer, CONSUMER_BIAS)], providers)
}
