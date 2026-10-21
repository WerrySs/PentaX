// SPDX-License-Identifier: GPL-2.0-only
//! Fixed diagnostic admission history, not semantic success or in-flight state.
use super::Function;

macro_rules! catalogue {
    ($($name:ident),+ $(,)?) => {
        const FUNCTIONS: &[Function] = &[$(Function::$name),+];
        enum Slot { $($name),+ }
        impl Function {
            fn profile_index(self) -> usize {
                match self { $(Self::$name => Slot::$name as usize),+ }
            }
            fn profile_label(self) -> &'static str {
                match self { $(Self::$name => stringify!($name)),+ }
            }
        }
    };
}
// Both matches are exhaustive: a new Function requires an explicit closed slot
// and label. No registry identity, guest text or numeric address enters here.
catalogue!(
    CommandBuffer,
    MultiCommandBuffers,
    Acb,
    MultiAcbs,
    SuspendPoint,
    AgcInit,
    AgcCreateShader,
    AgcGetFusedShaderSize,
    AgcFuseShaderHalves,
    AgcFuseShaderHalvesLegacy,
    AgcCreateInterpolantMapping,
    AgcCreateInterpolantMapping2,
    AgcAddEqEvent,
    AgcDeleteEqEvent,
    AgcGetEqEventType,
    AgcGetEqContextId,
    AgcGetIsTrinityMode,
    AgcRegistryUnavailable,
    AgcGetRegisterDefaults,
    AgcGetRegisterDefaultsInternal,
    VideoOpen,
    VideoClose,
    VideoSetFlipMaster,
    VideoSetFlipRate,
    VideoInitializeOptions,
    VideoIsOutputSupported,
    VideoConfigureOutput,
    VideoGetOutputStatus,
    VideoSetBufferAttribute2,
    VideoAddBufferAttributeOption,
    VideoRegisterBuffers2,
    VideoChangeBufferAttribute2,
    VideoUnregisterBuffers,
    VideoSubmitFlip,
    VideoGetFlipStatus,
    VideoIsFlipPending,
    AgcDcbSetFlip,
    VideoAddFlipEvent,
    VideoDeleteFlipEvent,
    VideoAddVblankEvent,
    VideoDeleteVblankEvent,
    VideoGetEventId,
    VideoGetEventData,
    VideoGetEventCount,
);

/// Historical admitted calls, including normal rejected return values. This
/// never identifies a currently executing function or proves semantic success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionProfile {
    counts: [u64; FUNCTIONS.len()],
    last: Option<Function>,
    shader_create: [u64; 3],
}
impl Default for FunctionProfile {
    fn default() -> Self {
        Self {
            counts: [0; FUNCTIONS.len()],
            last: None,
            shader_create: [0; 3],
        }
    }
}
impl FunctionProfile {
    pub(super) fn record_shader_create(&mut self, result: &Result<u64, super::Failure>) {
        let index = match result {
            Ok(0) => 0,
            Ok(_) => 1,
            Err(_) => 2,
        };
        self.shader_create[index] = self.shader_create[index].saturating_add(1);
    }
    /// Completed CreateShader helper outcomes: zero status, guest rejection,
    /// and typed host-side refusal. None proves that the shader can render.
    /// Calls refused before driver admission are absent from all three counts.
    pub fn shader_create_outcomes(&self) -> (u64, u64, u64) {
        let [ok, rejected, failed] = self.shader_create;
        (ok, rejected, failed)
    }
    pub(super) fn record(&mut self, function: Function) {
        let counter = &mut self.counts[function.profile_index()];
        *counter = counter.saturating_add(1);
        self.last = Some(function);
    }
    pub fn count(&self, function: Function) -> u64 {
        self.counts[function.profile_index()]
    }
    pub fn last(&self) -> &'static str {
        self.last.map_or("none", Function::profile_label)
    }
    /// At most four nonzero counters. Ties use the fixed catalogue order.
    /// The four fixed scans allocate nothing and expose only static labels.
    pub fn top(&self) -> [Option<(&'static str, u64)>; 4] {
        let mut selected = [None; 4];
        let mut result = [None; 4];
        for rank in 0..4 {
            let mut best = None;
            let mut largest = 0;
            for (index, count) in self.counts.iter().copied().enumerate() {
                if count > largest && !selected[..rank].contains(&Some(index)) {
                    largest = count;
                    best = Some(index);
                }
            }
            selected[rank] = best;
            if let Some(index) = best {
                result[rank] = Some((FUNCTIONS[index].profile_label(), largest));
            }
        }
        result
    }
}
