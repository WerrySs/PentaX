// SPDX-License-Identifier: GPL-2.0-only
// Copyright (c) 2021 InoriRus
// Modified 2026 for PentaX: bounded default-coalescing VideoOut subscriptions.
// Pinned videoOut.cpp and eventQueueFilters.h; see kernel NOTICE.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VideoEventKind {
    Flip,
    Vblank,
}
impl VideoEventKind {
    fn ident(self) -> u64 {
        match self {
            Self::Flip => 0,
            Self::Vblank => 1,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct Subscription {
    handle: i32,
    generation: u64,
    kind: VideoEventKind,
}

/// Exclusive, allocation-free preadmission of real consumer notification.
/// Dropping this token changes no pending event or delivery order.
pub struct VideoInterrupt<'a> {
    queues: &'a mut [Queue],
    subscription: Subscription,
    payload: u64,
    tsc: u64,
}
impl VideoInterrupt<'_> {
    pub fn complete(self) {
        for queue in self.queues {
            for event in &mut queue.events {
                if event.video != Some(self.subscription) {
                    continue;
                }
                if event.active == 0 {
                    event.active = queue.next_active;
                    queue.next_active += 1;
                }
                event.fflags = (event.fflags + u32::from(event.fflags < 15)).min(15);
                let count = ((event.data >> 12) & 15).saturating_add(1).min(15);
                event.data = (self.tsc & 0xfff)
                    | (count << 12)
                    | ((self.payload & 0x0000_ffff_ffff_ffff) << 16);
            }
        }
    }
}

/// Coalesced real display boundaries, admitted together before publication.
#[derive(Clone, Copy, Debug)]
pub struct VideoBoundary {
    pub handle: i32,
    pub generation: u64,
    pub count: u64,
    pub tsc: u64,
    pub occurrences: u64,
}
pub struct VideoBoundaryBatch<'a> {
    queues: &'a mut [Queue],
    boundaries: [Option<VideoBoundary>; 3],
    close: Option<(i32, u64)>,
    budget: &'a mut Budget,
}
impl VideoBoundaryBatch<'_> {
    pub fn complete(self) {
        for boundary in self.boundaries.into_iter().flatten() {
            let subscription = Subscription {
                handle: boundary.handle,
                generation: boundary.generation,
                kind: VideoEventKind::Vblank,
            };
            let add = boundary.occurrences.min(15);
            for queue in &mut *self.queues {
                for event in &mut queue.events {
                    if event.video != Some(subscription) {
                        continue;
                    }
                    if event.active == 0 {
                        event.active = queue.next_active;
                        queue.next_active += 1;
                    }
                    event.fflags = (u64::from(event.fflags).saturating_add(add)).min(15) as u32;
                    let count = (((event.data >> 12) & 15) + add).min(15);
                    event.data = (boundary.tsc & 0xfff)
                        | (count << 12)
                        | ((boundary.count & 0x0000_ffff_ffff_ffff) << 16);
                }
            }
        }
        if let Some((handle, generation)) = self.close {
            for queue in &mut *self.queues {
                queue.events.retain(|event| {
                    let remove = event
                        .video
                        .is_some_and(|s| s.handle == handle && s.generation == generation);
                    if remove {
                        self.budget.removed_handle();
                    }
                    !remove
                });
            }
        }
    }
}

pub struct VideoClose<'a> {
    queues: &'a mut [Queue],
    budget: &'a mut Budget,
    handle: i32,
    generation: u64,
}
impl VideoClose<'_> {
    pub fn complete(self) {
        for queue in self.queues {
            queue.events.retain(|event| {
                let remove = event
                    .video
                    .is_some_and(|s| s.handle == self.handle && s.generation == self.generation);
                if remove {
                    self.budget.removed_handle();
                }
                !remove
            });
        }
    }
}
impl crate::Kernel {
    /// Retained registrations, including subscriptions not yet signaled. The
    /// caller qualifies the port generation; this does not inspect readiness.
    pub fn video_vblank_subscription_count(&self, handle: i32, generation: u64) -> u64 {
        let subscription = Subscription {
            handle,
            generation,
            kind: VideoEventKind::Vblank,
        };
        self.event_queues
            .queues
            .iter()
            .flat_map(|queue| &queue.events)
            .filter(|event| event.video == Some(subscription))
            .count() as u64
    }

    /// Fixed three-port readiness mask for the paused display scheduler. The
    /// complete queue/event traversal is charged before inspecting membership.
    pub fn video_vblank_mask(
        &mut self,
        generations: [Option<u64>; 3],
    ) -> Result<[bool; 3], Failure> {
        let cost = (self.usage().handles_live as u64)
            .checked_mul(5)
            .and_then(|n| n.checked_add(3))
            .ok_or(Failure::Limit("video subscription work"))?;
        self.budget.work(cost)?;
        let mut mask = [false; 3];
        for queue in &self.event_queues.queues {
            for event in &queue.events {
                for (slot, generation) in generations.iter().enumerate() {
                    if generation.is_some_and(|generation| {
                        event.video
                            == Some(Subscription {
                                handle: slot as i32 + 1,
                                generation,
                                kind: VideoEventKind::Vblank,
                            })
                    }) {
                        mask[slot] = true;
                    }
                }
            }
        }
        Ok(mask)
    }

    /// A display deadline may keep only its actual subscribed equeue waiter
    /// wakeable; another queue's subscription cannot hide an unrelated deadlock.
    pub fn event_queue_wait_has_vblank(
        &mut self,
        ticket: &EventQueueWait,
        generations: [Option<u64>; 3],
    ) -> Result<bool, Failure> {
        let id = self.event_queues.waiter_queue(ticket, &mut self.budget)?;
        let at = match self.event_queues.index(id, &mut self.budget) {
            Ok(at) => at,
            Err(Failure::Errno(Errno::BadFd)) => return Ok(false),
            Err(error) => return Err(error),
        };
        let queue = &self.event_queues.queues[at];
        let cost = (queue.events.len() as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(1))
            .ok_or(Failure::Limit("video wait membership"))?;
        self.budget.work(cost)?;
        Ok(queue.events.iter().any(|event| {
            event.video.is_some_and(|s| {
                s.kind == VideoEventKind::Vblank
                    && s.handle
                        .checked_sub(1)
                        .and_then(|handle| usize::try_from(handle).ok())
                        .filter(|slot| *slot < 3)
                        .is_some_and(|slot| generations[slot] == Some(s.generation))
            })
        }))
    }

    /// Core validates the actual live VideoOut port before this internal call.
    /// Membership is retained by the queue; queue deletion cannot revive it.
    pub fn add_video_event(
        &mut self,
        queue: u64,
        handle: i32,
        generation: u64,
        kind: VideoEventKind,
        user_data: u64,
    ) -> Result<u64, Failure> {
        let at = self.event_queues.index(queue, &mut self.budget)?;
        self.budget
            .work(self.event_queues.queues[at].events.len() as u64 + 1)?;
        if self.event_queues.queues[at].events.iter().any(|event| {
            event.filter == VIDEO_OUT
                && event.ident == kind.ident()
                && event.video
                    != Some(Subscription {
                        handle,
                        generation,
                        kind,
                    })
        }) {
            return Err(Failure::Unsupported("video event queue port collision"));
        }
        let now = self.event_queues.nanos(Instant::now());
        self.event_queues.add(
            at,
            Event {
                ident: kind.ident(),
                filter: VIDEO_OUT,
                flags: 0,
                fflags: 0,
                data: 0,
                udata: user_data,
                deadline: 0,
                interval: 0,
                active: 0,
                video: Some(Subscription {
                    handle,
                    generation,
                    kind,
                }),
            },
            &mut self.budget,
            now,
        )
    }
    pub fn delete_video_event(&mut self, queue: u64, kind: VideoEventKind) -> Result<u64, Failure> {
        let at = self.event_queues.index(queue, &mut self.budget)?;
        let queue = &mut self.event_queues.queues[at];
        let event = match EventQueues::event_index(queue, kind.ident(), VIDEO_OUT, &mut self.budget)
        {
            Ok(index) => index,
            Err(Failure::Errno(Errno::NoEntry)) => return Ok(0),
            Err(error) => return Err(error),
        };
        self.budget.work(queue.events.len() as u64)?;
        queue.events.remove(event);
        self.budget.removed_handle();
        Ok(0)
    }
    pub fn prepare_video_interrupt(
        &mut self,
        handle: i32,
        generation: u64,
        kind: VideoEventKind,
        payload: u64,
        tsc: u64,
    ) -> Result<VideoInterrupt<'_>, Failure> {
        let subscription = Subscription {
            handle,
            generation,
            kind,
        };
        let cost = (self.usage().handles_live as u64)
            .checked_mul(3)
            .and_then(|n| n.checked_add(1))
            .ok_or(Failure::Limit("video event work"))?;
        self.budget.work(cost)?;
        for queue in &self.event_queues.queues {
            // There is at most one ident/filter per queue. The entire batch
            // must admit its next active ordinal before any notification.
            if queue
                .events
                .iter()
                .any(|e| e.video == Some(subscription) && e.active == 0)
            {
                queue
                    .next_active
                    .checked_add(1)
                    .ok_or(Failure::Limit("event ordering"))?;
            }
        }
        Ok(VideoInterrupt {
            queues: &mut self.event_queues.queues,
            subscription,
            payload,
            tsc,
        })
    }
    pub fn prepare_video_boundaries(
        &mut self,
        boundaries: [Option<VideoBoundary>; 3],
    ) -> Result<VideoBoundaryBatch<'_>, Failure> {
        self.prepare_video_boundary_close(boundaries, None)
    }
    pub fn prepare_video_boundary_close(
        &mut self,
        boundaries: [Option<VideoBoundary>; 3],
        close: Option<(i32, u64)>,
    ) -> Result<VideoBoundaryBatch<'_>, Failure> {
        // Three fixed ports; neither elapsed periods nor missed notifications
        // introduce a loop. Admit all ordinals and work before any event changes.
        let cost = (self.usage().handles_live as u64)
            .checked_mul(15)
            .and_then(|n| n.checked_add(3))
            .ok_or(Failure::Limit("video event work"))?;
        self.budget.work(cost)?;
        for queue in &self.event_queues.queues {
            let mut activations = 0_u64;
            for boundary in boundaries.iter().flatten() {
                if boundary.occurrences == 0 {
                    return Err(Failure::Unsupported("empty video boundary"));
                }
                let subscription = Subscription {
                    handle: boundary.handle,
                    generation: boundary.generation,
                    kind: VideoEventKind::Vblank,
                };
                activations += u64::from(
                    queue
                        .events
                        .iter()
                        .any(|event| event.video == Some(subscription) && event.active == 0),
                );
            }
            queue
                .next_active
                .checked_add(activations)
                .ok_or(Failure::Limit("event ordering"))?;
        }
        Ok(VideoBoundaryBatch {
            queues: &mut self.event_queues.queues,
            boundaries,
            close,
            budget: &mut self.budget,
        })
    }

    pub fn prepare_video_close(
        &mut self,
        handle: i32,
        generation: u64,
    ) -> Result<VideoClose<'_>, Failure> {
        let cost = (self.usage().handles_live as u64)
            .checked_mul(3)
            .and_then(|n| n.checked_add(1))
            .ok_or(Failure::Limit("video event work"))?;
        self.budget.work(cost)?;
        Ok(VideoClose {
            queues: &mut self.event_queues.queues,
            budget: &mut self.budget,
            handle,
            generation,
        })
    }
}

#[cfg(test)]
mod tests;
