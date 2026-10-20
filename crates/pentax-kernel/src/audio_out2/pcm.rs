// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5 f186b357 libAudio2.cpp; checked PCM snapshots and actual stereo mixing.
use super::state::*;
use super::*;

type MixedBatch = (Vec<u8>, [Option<usize>; SINKS], u64);

impl AudioState {
    pub(super) fn attributes(
        &mut self,
        args: CallArgs,
        guest: &impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let [id, attributes, count, ..] = args.0;
        let count = count as u32;
        if count != 0 && attributes == 0 {
            return Ok(INVALID);
        }
        let Some(index) = self.port(id) else {
            return Ok(INVALID_PORT);
        };
        // Charge the entire bounded header loop before its first read. Unknown
        // attributes read only their ID; locked non-PCM fields read no value.
        budget.work(
            u64::from(count)
                .checked_mul(4)
                .ok_or(Failure::Limit("audio attributes"))?,
        )?;
        let port = self.ports[index].as_ref().expect("owned port");
        let mut gains = port.gains;
        let mut ambisonics = port.ambisonics;
        let mut seen = 0u32;
        let mut pcm_pointer = 0;
        for i in 0..count {
            let address = at(attributes, u64::from(i) * 24)?;
            let id = budget.u32(guest, address)?;
            if !matches!(id, 0 | 1 | 8) {
                continue;
            }
            if seen & (1 << id) != 0 {
                return Ok(INVALID);
            }
            seen |= 1 << id;
            if id != 0 && port.locked {
                continue;
            }
            let value = budget.u64(guest, at(address, 8)?)?;
            if value == 0 {
                return Ok(INVALID);
            }
            let size = budget.u64(guest, at(address, 16)?)?;
            match id {
                0 => {
                    if size < 8 {
                        return Ok(INVALID);
                    }
                    pcm_pointer = budget.u64(guest, value)?;
                }
                1 => {
                    let count = channels(port.format);
                    if size != (count * 4) as u64 {
                        return Ok(INVALID);
                    }
                    let mut bytes = [0; 64];
                    budget.read(guest, value, &mut bytes[..count * 4])?;
                    for (n, g) in gains.iter_mut().take(count).enumerate() {
                        *g = f32::from_le_bytes(
                            bytes[n * 4..n * 4 + 4].try_into().expect("float width"),
                        );
                        if !g.is_finite() || *g < 0.0 {
                            return Ok(INVALID);
                        }
                    }
                }
                8 => {
                    if !port.object() || size != 4 {
                        return Ok(INVALID);
                    }
                    ambisonics = budget.u32(guest, value)?;
                    if ambisonics != u32::MAX && ambisonics > 15 && !(64..=99).contains(&ambisonics)
                    {
                        return Ok(INVALID);
                    }
                }
                _ => unreachable!(),
            }
        }
        let pcm = if seen & 1 != 0 {
            if pcm_pointer == 0 || !recognized(port.format) {
                Some(Vec::new())
            } else {
                let length = (port.grains as usize)
                    .checked_mul(channels(port.format))
                    .and_then(|n| n.checked_mul(if port.format & 127 == 1 { 2 } else { 4 }))
                    .ok_or(Failure::Limit("audio PCM bytes"))?;
                self.admit_payload(port.pcm.capacity(), length)?;
                let mut bytes = budget.buffer_held(length, self.storage())?;
                self.admit_payload(port.pcm.capacity(), bytes.capacity())?;
                budget.read(guest, pcm_pointer, &mut bytes)?;
                Some(bytes)
            }
        } else {
            None
        };
        let port = self.ports[index].as_mut().expect("owned port");
        port.gains = gains;
        port.ambisonics = ambisonics;
        if let Some(pcm) = pcm {
            port.pcm = pcm;
            port.locked = port.object();
        }
        Ok(0)
    }

    pub(super) fn mix(&self, context: usize, budget: &mut Budget) -> Result<MixedBatch, Failure> {
        let context = self.contexts[context].as_ref().expect("owned context");
        let has_object = self
            .ports
            .iter()
            .flatten()
            .any(|p| p.context == context.id && p.object() && !p.pcm.is_empty());
        let mut mix = if has_object {
            let size = context.queue.grains as usize * 8;
            self.admit_payload(context.mix.capacity(), size)?;
            let bytes = budget.buffer_held(size, self.storage())?;
            self.admit_payload(context.mix.capacity(), bytes.capacity())?;
            bytes
        } else {
            Vec::new()
        };
        let mut sinks = [None; SINKS];
        let mut count = 0;
        let mut frames = 0u64;
        let ports = self
            .ports
            .iter()
            .flatten()
            .filter(|p| p.context == context.id && !p.pcm.is_empty());
        let work = ports.clone().filter(|p| p.object()).count() as u64
            * u64::from(context.queue.grains)
            * 12;
        budget.work(work)?;
        for p in ports {
            if !p.object() {
                if count < SINKS
                    && let Some(s) = p.sink
                {
                    sinks[count] = Some(s);
                    count += 1;
                }
                continue;
            }
            let (left, right) = match p.ambisonics {
                u32::MAX | 64 => (f32::from_bits(0x3f35_04f3), f32::from_bits(0x3f35_04f3)),
                0 => (1.0, 1.0),
                2 | 65 => (f32::from_bits(0x3f35_04f3), -f32::from_bits(0x3f35_04f3)),
                _ => continue,
            };
            let left = left * p.gains[0];
            let right = right * p.gains[0];
            frames = frames
                .checked_add(u64::from(context.queue.grains))
                .ok_or(Failure::Limit("audio mix counter"))?;
            for frame in 0..context.queue.grains as usize {
                let sample = if p.format & 127 == 0 {
                    f32::from_le_bytes(
                        p.pcm[frame * 4..frame * 4 + 4]
                            .try_into()
                            .expect("float width"),
                    )
                } else {
                    f32::from(i16::from_le_bytes(
                        p.pcm[frame * 2..frame * 2 + 2]
                            .try_into()
                            .expect("sample width"),
                    )) / 32768.0
                };
                for (channel, gain) in [left, right].into_iter().enumerate() {
                    let offset = frame * 8 + channel * 4;
                    let old = f32::from_le_bytes(
                        mix[offset..offset + 4].try_into().expect("float width"),
                    );
                    mix[offset..offset + 4].copy_from_slice(&(old + sample * gain).to_le_bytes());
                }
            }
        }
        // Unsupported-but-source-accepted ambisonics still initialized mix,
        // hence submits an actual zero object batch and participates in pacing.
        if !mix.is_empty() && count < SINKS {
            sinks[count] = context.sink;
        }
        Ok((mix, sinks, frames))
    }
}
