// SPDX-License-Identifier: GPL-2.0-only
// KytyPS5 f186b357 libAudio2.cpp; modified for checked owned headless state.
use super::state::*;
use super::*;

impl AudioState {
    pub(super) fn lifecycle_with<G: GuestAccess>(
        &mut self,
        service: Service,
        args: CallArgs,
        guest: &mut G,
        budget: &mut Budget,
        mut clock: impl FnMut(&G) -> Result<u64, Failure>,
    ) -> Result<u64, Failure> {
        let a = args.0;
        match service {
            Service::AudioOut2ContextCreate => {
                if a[0] == 0 {
                    return Err(Failure::Unsupported("null AudioOut2 context parameter"));
                }
                if a[3] == 0 {
                    return Err(Failure::Unsupported("null AudioOut2 context output"));
                }
                let grains = budget.u32(guest, at(a[0], 16)?)?;
                let grains = if grains == 0 { 512 } else { grains };
                if grains > MAX_GRAINS {
                    return Err(Failure::Limit("audio grains"));
                }
                let objects = budget.u32(guest, at(a[0], 4)?)?;
                let sink = if objects != 0 {
                    match self.free_sink() {
                        Some(s) => Some(s),
                        None => return Ok(RESOURCE),
                    }
                } else {
                    None
                };
                let Some(slot) = self.contexts.iter().position(Option::is_none) else {
                    return Ok(RESOURCE);
                };
                budget.can_add_handle()?;
                let next = next(self.next_context)?;
                let id = self.next_context.to_le_bytes();
                let write = GuestWrite {
                    address: a[3],
                    bytes: &id,
                };
                // Source writes the handle BEFORE reading queue_depth. Read
                // through the staged output, including partial virtual aliases.
                let mut depth = [0; 4];
                budget.read_staged(guest, at(a[0], 12)?, &mut depth, &[write])?;
                let depth = u32::from_le_bytes(depth);
                let now = clock(guest)?;
                budget.write(guest, &[write])?;
                self.contexts[slot] = Some(Context {
                    id: self.next_context,
                    queue: Queue {
                        depth: if depth == 0 { 4 } else { depth },
                        queued: 0,
                        grains,
                        last: now,
                    },
                    sink,
                    mix: Vec::new(),
                });
                if let Some(s) = sink {
                    self.sinks[s] = Some(Sink {
                        frames: grains,
                        rate: 48_000,
                        ..Default::default()
                    });
                }
                self.next_context = next;
                budget.added_handle();
                Ok(0)
            }
            Service::AudioOut2ContextDestroy => {
                if let Some(index) = self.context(a[0]) {
                    for p in 0..self.ports.len() {
                        if self.ports[p].as_ref().is_some_and(|p| p.context == a[0]) {
                            self.destroy_port(p, budget);
                        }
                    }
                    let c = self.contexts[index].take().expect("owned context");
                    if let Some(s) = c.sink {
                        self.sinks[s] = None;
                    }
                    budget.removed_handle();
                }
                Ok(0)
            }
            Service::AudioOut2ContextAdvance => {
                if let Some(index) = self.context(a[0]) {
                    let now = clock(guest)?;
                    self.contexts[index]
                        .as_mut()
                        .expect("owned context")
                        .queue
                        .update(now);
                    for port in self
                        .ports
                        .iter_mut()
                        .flatten()
                        .filter(|p| p.context == a[0])
                    {
                        port.locked = false;
                    }
                }
                Ok(0)
            }
            Service::AudioOut2ContextGetQueueLevel => {
                let index = self.context(a[0]);
                let mut queue =
                    index.map(|i| self.contexts[i].as_ref().expect("owned context").queue);
                if let Some(q) = &mut queue {
                    q.update(clock(guest)?);
                }
                let q = queue.map_or(0, |q| q.queued);
                let available = queue.map_or(4, |q| q.depth.saturating_sub(q.queued));
                write_levels(guest, budget, a[1], q, a[2], available)?;
                if let (Some(i), Some(q)) = (index, queue) {
                    self.contexts[i].as_mut().expect("owned context").queue = q;
                }
                Ok(0)
            }
            Service::AudioOut2UserCreate => {
                if a[1] == 0 {
                    return Ok(INVALID);
                }
                if self.users.len() >= budget.limits.max_handles.min(4096) {
                    return Err(Failure::Limit("audio users"));
                }
                budget.can_add_handle()?;
                let next = next(self.next_user)?;
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &self.next_user.to_le_bytes(),
                    }],
                )?;
                self.users.push(self.next_user);
                self.next_user = next;
                budget.added_handle();
                Ok(0)
            }
            Service::AudioOut2UserDestroy => {
                let Some(i) = self.users.iter().position(|u| *u == a[0]) else {
                    return Ok(INVALID);
                };
                if self.ports.iter().flatten().any(|p| p.user == a[0]) {
                    return Ok(BUSY);
                }
                self.users.remove(i);
                budget.removed_handle();
                Ok(0)
            }
            Service::AudioOut2PortCreate => self.create_port(args, guest, budget),
            Service::AudioOut2PortDestroy => {
                if let Some(i) = self.port(a[0]) {
                    self.destroy_port(i, budget);
                }
                Ok(0)
            }
            Service::AudioOut2PortGetState => {
                if a[1] == 0 {
                    return Err(Failure::Unsupported("null AudioOut2 port state"));
                }
                let mut bytes = [0; 64];
                bytes[0..2].copy_from_slice(&1u16.to_le_bytes());
                bytes[2] = self.port(a[0]).map_or(2, |i| {
                    channels(self.ports[i].as_ref().expect("owned port").format) as u8
                });
                bytes[4..6].copy_from_slice(&127i16.to_le_bytes());
                budget.write(
                    guest,
                    &[GuestWrite {
                        address: a[1],
                        bytes: &bytes,
                    }],
                )?;
                Ok(0)
            }
            Service::AudioOut2PortSetAttributes => self.attributes(args, guest, budget),
            _ => Err(Failure::Unsupported("AudioOut2 lifecycle service")),
        }
    }
    fn port_error(&mut self, next: u64, value: u64) -> Result<u64, Failure> {
        // Upstream increments before source validation, even for these errors.
        // Controlled guest/policy/allocation failures never reach this commit.
        self.next_port = next;
        Ok(value)
    }
    fn create_port(
        &mut self,
        args: CallArgs,
        guest: &mut impl GuestAccess,
        budget: &mut Budget,
    ) -> Result<u64, Failure> {
        let a = args.0;
        if a[1] == 0 {
            return Err(Failure::Unsupported("null AudioOut2 port parameter"));
        }
        if a[2] == 0 {
            return Err(Failure::Unsupported("null AudioOut2 port output"));
        }
        let next = next(self.next_port)?;
        let format = budget.u32(guest, at(a[1], 4)?)?;
        let mut kind = [0; 2];
        budget.read(guest, a[1], &mut kind)?;
        let kind = u16::from_le_bytes(kind);
        let Some(c) = self.context(a[0]) else {
            return self.port_error(next, INVALID);
        };
        let context = self.contexts[c].as_ref().expect("owned context");
        let grains = context.queue.grains;
        let object = kind & 0xff00 == 0x100;
        let mut frequency = None;
        if object {
            if kind != 0x100 || context.sink.is_none() {
                return self.port_error(next, INVALID);
            }
            let freq = budget.u32(guest, at(a[1], 8)?)?;
            frequency = Some(freq);
            if freq != 48_000 || !recognized(format) || channels(format) != 1 {
                return self.port_error(next, INVALID);
            }
        }
        let user = budget.u64(guest, at(a[1], 16)?)?;
        if !self.users.contains(&user) {
            return self.port_error(next, INVALID);
        }
        let Some(slot) = self.ports.iter().position(Option::is_none) else {
            return self.port_error(next, PORT_FULL);
        };
        let freq = match frequency {
            Some(f) => f,
            None => budget.u32(guest, at(a[1], 8)?)?,
        };
        let sink = if !object && freq != 0 && recognized(format) {
            self.free_sink()
        } else {
            None
        };
        budget.can_add_handle()?;
        budget.write(
            guest,
            &[GuestWrite {
                address: a[2],
                bytes: &self.next_port.to_le_bytes(),
            }],
        )?;
        self.ports[slot] = Some(Port {
            id: self.next_port,
            context: a[0],
            user,
            kind,
            format,
            grains,
            sink,
            gains: [1.0; 16],
            ambisonics: u32::MAX,
            locked: false,
            pcm: Vec::new(),
        });
        if let Some(s) = sink {
            self.sinks[s] = Some(Sink {
                frames: grains,
                rate: freq,
                ..Default::default()
            });
        }
        self.next_port = next;
        budget.added_handle();
        Ok(0)
    }
}

// Source output order is queue then available. Merge the at-most-eight ordinary
// virtual bytes into one span when overlapping; GuestAccess still rejects
// distinct physical aliases atomically. No background/padding read is needed.
fn write_levels(
    guest: &mut impl GuestAccess,
    budget: &mut Budget,
    qa: u64,
    q: u32,
    aa: u64,
    a: u32,
) -> Result<(), Failure> {
    let qb = q.to_le_bytes();
    let ab = a.to_le_bytes();
    if qa == 0 && aa == 0 {
        return Ok(());
    }
    if qa == 0 {
        return budget.write(
            guest,
            &[GuestWrite {
                address: aa,
                bytes: &ab,
            }],
        );
    }
    if aa == 0 {
        return budget.write(
            guest,
            &[GuestWrite {
                address: qa,
                bytes: &qb,
            }],
        );
    }
    let qe = at(qa, 4)?;
    let ae = at(aa, 4)?;
    if qa < ae && aa < qe {
        let start = qa.min(aa);
        let end = qe.max(ae);
        let mut bytes = [0; 8];
        let qi = (qa - start) as usize;
        let ai = (aa - start) as usize;
        bytes[qi..qi + 4].copy_from_slice(&qb);
        bytes[ai..ai + 4].copy_from_slice(&ab);
        budget.write(
            guest,
            &[GuestWrite {
                address: start,
                bytes: &bytes[..(end - start) as usize],
            }],
        )
    } else {
        budget.write(
            guest,
            &[
                GuestWrite {
                    address: qa,
                    bytes: &qb,
                },
                GuestWrite {
                    address: aa,
                    bytes: &ab,
                },
            ],
        )
    }
}
