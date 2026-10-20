// SPDX-License-Identifier: GPL-2.0-only
//! Original headless contracts. No device, host sleep or proprietary input.
use super::state::*;
use super::*;
use crate::tests::{ARG, PAGE, TestGuest, kernel};
use crate::{Kernel, Limits, MemoryEffect, TimeCounterSample};
use Service::*;
use std::cell::Cell;

struct Guest {
    memory: TestGuest,
    micros: Cell<u64>,
}
impl Guest {
    fn new() -> Self {
        Self {
            memory: TestGuest::new(),
            micros: Cell::new(0),
        }
    }
    fn put(&mut self, offset: u64, bytes: &[u8]) {
        self.memory.put(offset, bytes);
    }
    fn word(&mut self, offset: u64, word: u32) {
        self.put(offset, &word.to_le_bytes());
    }
    fn value(&self, offset: u64) -> u64 {
        u64::from_le_bytes(self.memory.get(ARG + offset, 8).try_into().unwrap())
    }
}
impl GuestAccess for Guest {
    fn read(&self, address: u64, bytes: &mut [u8]) -> Result<(), GuestError> {
        self.memory.read(address, bytes)
    }
    fn apply(&mut self, effect: MemoryEffect, writes: &[GuestWrite<'_>]) -> Result<(), GuestError> {
        self.memory.apply(effect, writes)
    }
    fn read_after_writes(
        &self,
        address: u64,
        bytes: &mut [u8],
        writes: &[GuestWrite<'_>],
    ) -> Result<(), GuestError> {
        self.memory.read_after_writes(address, bytes, writes)
    }
    fn preflight_pthread(&self) -> Result<(), Failure> {
        Ok(())
    }
    fn time_counter(&self) -> Result<TimeCounterSample, Failure> {
        Ok(TimeCounterSample {
            ticks: 10_000 + self.micros.get(),
            frequency: 1_000_000,
        })
    }
}
fn args(values: &[u64]) -> CallArgs {
    let mut a = [0; 6];
    a[..values.len()].copy_from_slice(values);
    CallArgs(a)
}
fn call(k: &mut Kernel, g: &mut Guest, service: Service, values: &[u64]) -> Result<u64, Failure> {
    k.invoke_library(service, args(values), g)
}
fn immediate(action: Result<AudioOut2WaitAction, Failure>) -> u64 {
    match action.unwrap() {
        AudioOut2WaitAction::Returned(value) => value,
        action => panic!("not immediate: {action:?}"),
    }
}
fn ticket(action: Result<AudioOut2WaitAction, Failure>) -> AudioOut2Wait {
    match action.unwrap() {
        AudioOut2WaitAction::Waiting(ticket) => ticket,
        action => panic!("not waiting: {action:?}"),
    }
}
fn context(k: &mut Kernel, g: &mut Guest, grains: u32, depth: u32, objects: bool) -> u64 {
    g.put(0, &[0; 64]);
    g.word(4, u32::from(objects));
    g.word(12, depth);
    g.word(16, grains);
    assert_eq!(
        call(
            k,
            g,
            AudioOut2ContextCreate,
            &[ARG, u64::MAX, u64::MAX, ARG + 64]
        ),
        Ok(0)
    );
    g.value(64)
}
fn user(k: &mut Kernel, g: &mut Guest) -> u64 {
    assert_eq!(
        call(k, g, AudioOut2UserCreate, &[u64::MAX, ARG + 72]),
        Ok(0)
    );
    g.value(72)
}
fn port(
    k: &mut Kernel,
    g: &mut Guest,
    context: u64,
    user: u64,
    object: bool,
    format: u32,
    rate: u32,
) -> u64 {
    g.put(128, &[0; 64]);
    g.put(128, &(if object { 0x100u16 } else { 0 }).to_le_bytes());
    g.word(132, format);
    g.word(136, rate);
    g.put(144, &user.to_le_bytes());
    assert_eq!(
        call(k, g, AudioOut2PortCreate, &[context, ARG + 128, ARG + 80]),
        Ok(0)
    );
    g.value(80)
}
fn attribute(g: &mut Guest, row: u64, id: u32, value: u64, length: u64) {
    g.word(row, id);
    g.put(row + 8, &value.to_le_bytes());
    g.put(row + 16, &length.to_le_bytes());
}
fn set_pcm(k: &mut Kernel, g: &mut Guest, p: u64, bytes: &[u8]) {
    g.put(1024, bytes);
    g.put(512, &(ARG + 1024).to_le_bytes());
    attribute(g, 256, 0, ARG + 512, 8);
    assert_eq!(
        call(k, g, AudioOut2PortSetAttributes, &[p, ARG + 256, 1]),
        Ok(0)
    );
}

#[test]
fn owned_lifecycle_preserves_create_overlay_error_id_consumption_and_atomic_census() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    g.word(4, 1);
    g.word(12, 9);
    g.word(16, 4);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextCreate,
            &[ARG, 0, 0, ARG + 12]
        ),
        Ok(0)
    );
    let c = k.audio_out2.contexts[0].as_ref().unwrap();
    assert_eq!((c.id, c.queue.depth, c.queue.grains), (1, 1, 4));
    let count = k.handle_usage();
    g.memory.refuse = true;
    assert!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextCreate,
            &[ARG, 0, 0, ARG + 64]
        )
        .is_err()
    );
    assert_eq!(k.handle_usage(), count);
    assert_eq!(k.audio_out2.next_context, 2);
    g.memory.refuse = false;
    let u = user(&mut k, &mut g);
    g.put(128, &[0; 64]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortCreate,
            &[999, ARG + 128, ARG + 80]
        ),
        Ok(INVALID)
    );
    assert_eq!(k.audio_out2.next_port, 2);
    let p = port(&mut k, &mut g, 1, u, true, 0x100, 48_000);
    assert_eq!(p, 2);
    assert_eq!(call(&mut k, &mut g, AudioOut2UserDestroy, &[u]), Ok(BUSY));
    g.memory.refuse = true;
    assert!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortCreate,
            &[1, ARG + 128, ARG + 80]
        )
        .is_err()
    );
    assert_eq!(k.audio_out2.next_port, 3);
    g.memory.refuse = false;
    g.put(2048, &[0x5a; 66]);
    assert_eq!(
        call(&mut k, &mut g, AudioOut2PortGetState, &[p, ARG + 2049]),
        Ok(0)
    );
    assert_eq!(g.memory.get(ARG + 2048, 4), [0x5a, 1, 0, 1]);
    assert_eq!(g.memory.get(ARG + 2113, 1), [0x5a]);
    assert_eq!(call(&mut k, &mut g, AudioOut2ContextDestroy, &[1]), Ok(0));
    assert_eq!(call(&mut k, &mut g, AudioOut2UserDestroy, &[u]), Ok(0));
    assert_eq!(
        (
            k.handle_usage().audio2,
            k.handle_usage().live,
            k.handle_usage().accounted()
        ),
        (0, 0, 0)
    );
}

#[test]
fn source_limits_and_controlled_output_failures_do_not_publish_handles_or_ids() {
    let mut k = kernel(Limits {
        max_handles: 1,
        ..Default::default()
    });
    let mut g = Guest::new();
    assert!(matches!(
        call(&mut k, &mut g, AudioOut2ContextCreate, &[0, 0, 0, 0]),
        Err(Failure::Unsupported(_))
    ));
    g.word(16, MAX_GRAINS + 1);
    assert!(matches!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextCreate,
            &[ARG, 0, 0, ARG + 64]
        ),
        Err(Failure::Limit(_))
    ));
    g.word(16, 4);
    assert!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextCreate,
            &[ARG, 0, 0, ARG + PAGE - 7]
        )
        .is_err()
    );
    assert_eq!(k.audio_out2.next_context, 1);
    assert_eq!(k.handle_usage().audio2, 0);
    assert_eq!(
        call(&mut k, &mut g, AudioOut2UserCreate, &[1, ARG + 72]),
        Ok(0)
    );
    assert!(matches!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextCreate,
            &[ARG, 0, 0, ARG + 64]
        ),
        Err(Failure::Limit(_))
    ));
    assert_eq!(k.audio_out2.next_context, 1);
    assert_eq!(k.handle_usage().audio2, 1);
    let mut tiny = kernel(Limits {
        max_scratch_bytes: 1024,
        max_io_bytes: 512,
        max_name_bytes: 64,
        ..Default::default()
    });
    assert!(matches!(
        call(&mut tiny, &mut g, AudioOut2UserCreate, &[0, ARG + 72]),
        Err(Failure::Limit(_))
    ));
    assert!(tiny.audio_out2.contexts.is_empty());
    assert_eq!(tiny.handle_usage().audio2, 0);
}

#[test]
fn pcm_snapshots_attributes_locks_and_actual_stereo_mix_are_atomic_and_source_ordered() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    let c = context(&mut k, &mut g, 2, 8, true);
    let u = user(&mut k, &mut g);
    let p = port(&mut k, &mut g, c, u, true, 0x101, 48_000);
    // Set gain + ambisonics before PCM. S16 [-32768,16384], gain2, X
    // ambisonics produces exact [-sqrt2,+sqrt2,+sqrt(1/2),-sqrt(1/2)].
    g.put(520, &2f32.to_le_bytes());
    g.word(524, 2);
    attribute(&mut g, 256, 1, ARG + 520, 4);
    attribute(&mut g, 280, 8, ARG + 524, 4);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 2]
        ),
        Ok(0)
    );
    set_pcm(&mut k, &mut g, p, &[0, 128, 0, 64]);
    g.put(1024, &[0; 4]); // Owned copy must survive changed guest bytes.
    attribute(&mut g, 256, 1, 0, u64::MAX);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 1]
        ),
        Ok(0)
    );
    attribute(&mut g, 280, 1, 0, 0);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 2]
        ),
        Ok(INVALID)
    );
    assert_eq!(call(&mut k, &mut g, AudioOut2ContextPush, &[c, 0]), Ok(0));
    let bytes = &k.audio_out2.contexts[0].as_ref().unwrap().mix;
    let actual: Vec<_> = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .collect();
    assert_eq!(actual, [0xbfb5_04f3, 0x3fb5_04f3, 0x3f35_04f3, 0xbf35_04f3]);
    assert_eq!(
        k.audio_out2_usage(),
        AudioOut2Usage {
            mix_frames: 2,
            logical_output_batches: 1
        }
    );
    assert_eq!(
        k.audio_out2.ports[0].as_ref().unwrap().pcm.len() + bytes.len(),
        20
    );
    assert_eq!(k.usage().audio_bytes, k.audio_out2.pcm_bytes());
    assert_eq!(call(&mut k, &mut g, AudioOut2ContextAdvance, &[c]), Ok(0));
    g.put(520, &f32::NAN.to_le_bytes());
    attribute(&mut g, 256, 1, ARG + 520, 4);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 1]
        ),
        Ok(INVALID)
    );
    assert_eq!(k.audio_out2.ports[0].as_ref().unwrap().gains[0], 2.0);
    // An unknown ID needs no pointer/size access even at the page tail.
    g.word(PAGE - 4, 999);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + PAGE - 4, 1]
        ),
        Ok(0)
    );
    let old = k.audio_out2.ports[0].as_ref().unwrap().pcm.clone();
    attribute(&mut g, 256, 0, ARG + 512, 8);
    g.put(512, &(ARG + PAGE - 1).to_le_bytes());
    assert!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 1]
        )
        .is_err()
    );
    assert_eq!(k.audio_out2.ports[0].as_ref().unwrap().pcm, old);
}

#[test]
fn queue_backpressure_levels_and_fractional_sink_clock_have_distinct_domains() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    let c = context(&mut k, &mut g, 1, 1, false);
    g.micros.set(1);
    assert_eq!(call(&mut k, &mut g, AudioOut2ContextPush, &[c, 0]), Ok(0));
    assert_eq!(
        call(&mut k, &mut g, AudioOut2ContextPush, &[c, 0]),
        Ok(NOT_READY)
    );
    let w = ticket(k.begin_audio_out2_wait(AudioOut2ContextPush, args(&[c, 1]), &mut g));
    assert!(!k.audio_out2_wait_ready(&w, &g).unwrap());
    assert_eq!(
        k.audio_out2_wait_delay(&w, &g).unwrap(),
        Some(std::time::Duration::from_micros(1000))
    );
    g.micros.set(1001);
    assert_eq!(k.complete_audio_out2_wait(&w, &mut g), Ok(Some(0)));
    assert!(k.complete_audio_out2_wait(&w, &mut g).is_err());
    g.put(2000, &[0x5a; 8]);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2ContextGetQueueLevel,
            &[c, ARG + 2000, ARG + 2002]
        ),
        Ok(0)
    );
    assert_eq!(g.memory.get(ARG + 2000, 8), [1, 0, 0, 0, 0, 0, 0x5a, 0x5a]);
    let mut sink = Sink {
        frames: 1,
        rate: 48_000,
        ..Default::default()
    };
    assert_eq!(sink.submit(1, true), Ok(1));
    assert_eq!((sink.next, sink.remainder), (21, 40_000));
    assert_eq!(sink.submit(1, true), Ok(21));
    assert_eq!((sink.next, sink.remainder), (42, 32_000));
    assert_eq!(sink.submit(1, true), Ok(42));
    assert_eq!((sink.next, sink.remainder), (63, 24_000));
    assert_eq!(sink.submit(200_000, true), Ok(200_000));
    assert_eq!(sink.submit(200_001, false), Ok(200_001));
    assert_eq!((sink.next, sink.remainder), (0, 0));
}

#[test]
fn paced_global_lease_defers_other_context_mutations_and_keeps_exact_ticket_owner() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    k.activate_thread_context(1).unwrap();
    let c = context(&mut k, &mut g, 2, 8, true);
    let u = user(&mut k, &mut g);
    let p = port(&mut k, &mut g, c, u, true, 0x100, 48_000);
    set_pcm(&mut k, &mut g, p, &[0; 8]);
    g.micros.set(1);
    assert_eq!(
        immediate(k.begin_audio_out2_wait(AudioOut2ContextPush, args(&[c, 1]), &mut g)),
        0
    );
    let usage = k.audio_out2_usage();
    let queue = k.audio_out2.contexts[0].as_ref().unwrap().queue.queued;
    assert!(matches!(
        call(&mut k, &mut g, AudioOut2ContextPush, &[c, 1]),
        Err(Failure::Unsupported(_))
    ));
    assert_eq!(k.audio_out2_usage(), usage);
    assert_eq!(
        k.audio_out2.contexts[0].as_ref().unwrap().queue.queued,
        queue
    );
    let paced = ticket(k.begin_audio_out2_wait(AudioOut2ContextPush, args(&[c, 1]), &mut g));
    k.activate_thread_context(2).unwrap();
    let deferred = ticket(k.begin_audio_out2_wait(AudioOut2ContextDestroy, args(&[c]), &mut g));
    assert!(!k.audio_out2_wait_ready(&deferred, &g).unwrap());
    g.micros.set(42);
    assert!(k.audio_out2_wait_ready(&paced, &g).unwrap());
    assert!(k.complete_audio_out2_wait(&paced, &mut g).is_err());
    k.activate_thread_context(1).unwrap();
    let attempts = k.usage().attempts;
    assert_eq!(k.complete_audio_out2_wait(&paced, &mut g), Ok(Some(0)));
    assert_eq!(k.usage().attempts, attempts);
    assert!(k.audio_out2_wait_ready(&deferred, &g).unwrap());
    k.activate_thread_context(2).unwrap();
    assert_eq!(k.complete_audio_out2_wait(&deferred, &mut g), Ok(Some(0)));
    assert_eq!(k.handle_usage().audio2, 1);
    assert_eq!(k.usage().audio_bytes, 0);
    assert_eq!(
        k.audio_out2_usage(),
        AudioOut2Usage {
            mix_frames: 4,
            logical_output_batches: 2
        }
    );
    assert!(k.audio_out2_wait_ready(&deferred, &g).is_err());
}

#[test]
fn accepted_silent_ambisonics_still_submits_and_counter_overflow_refuses_before_commit() {
    let mut k = kernel(Default::default());
    let mut g = Guest::new();
    let c = context(&mut k, &mut g, 1, 8, true);
    let u = user(&mut k, &mut g);
    let p = port(&mut k, &mut g, c, u, true, 0x100, 48_000);
    g.word(520, 3);
    attribute(&mut g, 256, 8, ARG + 520, 4);
    assert_eq!(
        call(
            &mut k,
            &mut g,
            AudioOut2PortSetAttributes,
            &[p, ARG + 256, 1]
        ),
        Ok(0)
    );
    set_pcm(&mut k, &mut g, p, &1f32.to_le_bytes());
    assert_eq!(call(&mut k, &mut g, AudioOut2ContextPush, &[c, 0]), Ok(0));
    assert_eq!(k.audio_out2.contexts[0].as_ref().unwrap().mix, [0; 8]);
    assert_eq!(
        k.audio_out2_usage(),
        AudioOut2Usage {
            mix_frames: 0,
            logical_output_batches: 1
        }
    );
    k.audio_out2.usage.logical_output_batches = u64::MAX;
    let queued = k.audio_out2.contexts[0].as_ref().unwrap().queue.queued;
    assert!(matches!(
        call(&mut k, &mut g, AudioOut2ContextPush, &[c, 0]),
        Err(Failure::Limit(_))
    ));
    assert_eq!(
        k.audio_out2.contexts[0].as_ref().unwrap().queue.queued,
        queued
    );
    assert_eq!(k.audio_out2.sinks[0].unwrap().next, 0);
}
