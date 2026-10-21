// SPDX-License-Identifier: GPL-2.0-only

//! Final-only queue snapshot. Pending waits are not readiness or deadlock proof.

use super::Scheduling;
use pentax_core::gpu::{
    driver::{FunctionProfile, Usage},
    video::FlipObservations,
};
use std::fmt::Write;

#[derive(Debug, Default)]
pub(in crate::native_session) struct Observations {
    threads: u64,
    waiting: u64,
    waiting_equeue: u64,
    returned: u64,
    video_open_ports: u64,
    video_registered_buffers: u64,
    video_vblank_subscriptions: u64,
    // Calls include VideoOut and AGC/shader helpers; command ranges are charged
    // before copying/parsing. Draws/dispatches are attempts, not completions.
    // Written bytes include committed PM4 memory/label stores as well as
    // draw/compute writeback, so they do not prove a completed draw.
    graphics: Usage,
    graphics_functions: FunctionProfile,
    flips: FlipObservations,
    main_state: Option<&'static str>,
    main_wait: Option<&'static str>,
    active_role: Option<&'static str>,
}

impl Observations {
    pub(in crate::native_session) fn protocol_fields(&self) -> String {
        let mut fields = format!(
            "\tscheduling_threads={}\tscheduling_pending_waits={}\tscheduling_pending_equeue_waits={}\tscheduling_returned={}\tscheduling_main_state={}\tscheduling_main_wait={}\tscheduling_active_role={}\tvideo_open_ports={}\tvideo_registered_buffers={}\tvideo_vblank_subscriptions={}\tgraphics_calls={}\tgraphics_command_buffers={}\tgraphics_draw_attempts={}\tgraphics_dispatch_attempts={}\tgraphics_written_bytes={}\tvideo_flip_reserved={}\tvideo_flip_recording_with_frame={}\tvideo_flip_ready_with_frame={}\tvideo_flip_presenting={}\tvideo_flip_acknowledged={}",
            self.threads,
            self.waiting,
            self.waiting_equeue,
            self.returned,
            self.main_state.unwrap_or("unobserved"),
            self.main_wait.unwrap_or("none"),
            self.active_role.unwrap_or("unobserved"),
            self.video_open_ports,
            self.video_registered_buffers,
            self.video_vblank_subscriptions,
            self.graphics.calls,
            self.graphics.command_buffers,
            self.graphics.draws,
            self.graphics.dispatches,
            self.graphics.written_bytes,
            self.flips.reserved,
            self.flips.recording_with_frame,
            self.flips.ready_with_frame,
            self.flips.presenting,
            self.flips.acknowledged,
        );
        // Final-only bounded rendering: four closed labels/counts and one
        // historical last-admitted label, never arguments or runtime identity.
        fields.push_str("\tgraphics_function_top=");
        let mut separator = "";
        for (label, count) in self.graphics_functions.top().into_iter().flatten() {
            let _ = write!(fields, "{separator}{label}:{count}");
            separator = ",";
        }
        if separator.is_empty() {
            fields.push_str("none");
        }
        let _ = write!(
            fields,
            "\tgraphics_function_last={}",
            self.graphics_functions.last()
        );
        let (ok, rejected, failed) = self.graphics_functions.shader_create_outcomes();
        let _ = write!(
            fields,
            "\tgraphics_shader_create_ok={ok}\tgraphics_shader_create_rejected={rejected}\tgraphics_shader_create_failed={failed}"
        );
        let (ok, rejected, failed) = self.graphics_functions.shader_fuse_size_outcomes();
        let _ = write!(
            fields,
            "\tgraphics_shader_fuse_size_ok={ok}\tgraphics_shader_fuse_size_rejected={rejected}\tgraphics_shader_fuse_size_failed={failed}"
        );
        let (ok, rejected, failed) = self.graphics_functions.shader_fuse_outcomes();
        let _ = write!(
            fields,
            "\tgraphics_shader_fuse_ok={ok}\tgraphics_shader_fuse_rejected={rejected}\tgraphics_shader_fuse_failed={failed}"
        );
        fields
    }
}

impl Scheduling<'_, '_> {
    /// Visit the bounded queue once after an epoch stops, outside native entry.
    /// Do not call readiness checks: those can advance retained wait state.
    pub(in crate::native_session) fn observations(
        &self,
        video: (u64, u64, u64),
        graphics: Usage,
        flips: FlipObservations,
        graphics_functions: FunctionProfile,
    ) -> Observations {
        let mut snapshot = Observations {
            threads: self.threads.len() as u64,
            active_role: Some(if self.is_main() { "main" } else { "child" }),
            video_open_ports: video.0,
            video_registered_buffers: video.1,
            video_vblank_subscriptions: video.2,
            graphics,
            graphics_functions,
            flips,
            ..Default::default()
        };
        for thread in &self.threads {
            snapshot.waiting += u64::from(thread.waiting.is_some());
            snapshot.waiting_equeue += u64::from(
                thread
                    .waiting
                    .as_ref()
                    .is_some_and(|(wait, _)| wait.is_event_queue()),
            );
            snapshot.returned += u64::from(thread.returned);
        }
        if let Some(main) = self.threads.first() {
            snapshot.main_state = Some(if main.returned {
                "returned"
            } else if main.waiting.is_some() {
                "pending-wait"
            } else {
                "unparked"
            });
            // The label comes exclusively from the admitted public catalogue,
            // never a guest name, argument, identifier or decoded source byte.
            snapshot.main_wait = main.waiting_service;
        }
        snapshot
    }
}
