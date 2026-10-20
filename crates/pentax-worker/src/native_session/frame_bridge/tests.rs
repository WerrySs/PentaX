// SPDX-License-Identifier: GPL-2.0-only
//! Original transport-only messages; never claim an OS tick or presentation.
use super::*;
use std::{cell::RefCell, rc::Rc};

struct Bytes {
    input: [u8; 256],
    input_len: usize,
    read: usize,
    output: [u8; 256],
    output_len: usize,
    pending: bool,
    delay: bool,
    fragment: usize,
    eof: bool,
    cancelled: bool,
}
impl Default for Bytes {
    fn default() -> Self {
        Self {
            input: [0; 256],
            input_len: 0,
            read: 0,
            output: [0; 256],
            output_len: 0,
            pending: false,
            delay: false,
            fragment: 1,
            eof: false,
            cancelled: false,
        }
    }
}
struct OriginalTransport(Rc<RefCell<Bytes>>);
impl Transport for OriginalTransport {
    fn retained_bytes(&self) -> usize {
        1024
    }
    fn max_retained_bytes(&self) -> usize {
        1024
    }
    fn queue_write(&mut self, input: &[u8], _: usize) -> Result<bool, Error> {
        let mut bytes = self.0.borrow_mut();
        if bytes.pending {
            return Ok(false);
        }
        let at = bytes.output_len;
        let end = at
            .checked_add(input.len())
            .filter(|n| *n <= 256)
            .ok_or(Error::Limit)?;
        bytes.output[at..end].copy_from_slice(input);
        bytes.output_len = end;
        bytes.pending = true;
        bytes.delay = true;
        Ok(true)
    }
    fn poll_write(&mut self, _: usize) -> Result<bool, Error> {
        let mut bytes = self.0.borrow_mut();
        if bytes.pending && bytes.delay {
            bytes.delay = false;
            return Ok(true);
        }
        bytes.pending = false;
        Ok(false)
    }
    fn read_control(&mut self, target: &mut [u8], _: usize) -> Result<Read, Error> {
        let mut bytes = self.0.borrow_mut();
        if bytes.read == bytes.input_len {
            return Ok(if bytes.eof { Read::Eof } else { Read::Pending });
        }
        let count = (bytes.input_len - bytes.read)
            .min(bytes.fragment)
            .min(target.len());
        target[..count].copy_from_slice(&bytes.input[bytes.read..bytes.read + count]);
        bytes.read += count;
        Ok(Read::Bytes(count))
    }
    fn cancel(&mut self) {
        self.0.borrow_mut().cancelled = true;
    }
}
fn flags() -> FrameFlags {
    FrameFlags::new(
        pentax_session::AlphaMode::Straight,
        pentax_session::ColorEncoding::Srgb,
    )
}
fn append(bytes: &Rc<RefCell<Bytes>>, header: Header, payload: &[u8]) {
    let mut bytes = bytes.borrow_mut();
    let start = bytes.input_len;
    let end = start + 64 + payload.len();
    assert!(end <= 256);
    bytes.input[start..start + 64].copy_from_slice(&header.encode());
    bytes.input[start + 64..end].copy_from_slice(payload);
    bytes.input_len = end;
}

#[test]
fn configure_send_completion_precedes_exact_fragmented_ready_and_independent_ticks() {
    let io = Rc::new(RefCell::new(Bytes::default()));
    let mut transport = OriginalTransport(io.clone());
    let mut bridge = FrameBridge::new(
        &mut transport,
        Nonce::new([0x17; 16]).unwrap(),
        3,
        Limits::default(),
    )
    .unwrap();
    bridge.configure(2, 2, flags(), 7).unwrap();
    let config = bridge.configuration.unwrap();
    bridge.flush().unwrap();
    assert!(!bridge.configuration_sent);
    assert!(bridge.write_queued);
    bridge.flush().unwrap();
    assert!(bridge.configuration_sent);
    assert_eq!(
        &io.borrow().output[..io.borrow().output_len],
        &config.encode()
    );
    bridge.flush().unwrap();
    assert_eq!(io.borrow().output_len, 64); // No duplicated owned write.
    append(&io, Header::ready(config).unwrap(), &[]);
    for _ in 0..4 {
        bridge.read_controls().unwrap();
    }
    assert!(bridge.ready);
    assert_eq!(bridge.wire.last_vblank(), 0);
    assert!(bridge.frame.is_none());
    append(
        &io,
        Header::vblank(bridge.wire.nonce(), 3).unwrap(),
        &91u64.to_le_bytes(),
    );
    for _ in 0..5 {
        bridge.read_controls().unwrap();
    }
    assert_eq!(bridge.wire.last_vblank(), 91);
    assert!(bridge.frame.is_none());
    assert!(bridge.external_wake().unwrap().is_some());
}

#[test]
fn wrong_ready_and_receipt_before_complete_frame_send_refuse_before_payload_allocation() {
    for premature_receipt in [false, true] {
        let io = Rc::new(RefCell::new(Bytes {
            fragment: 256,
            ..Default::default()
        }));
        let mut transport = OriginalTransport(io.clone());
        let mut bridge = FrameBridge::new(
            &mut transport,
            Nonce::new([0x17; 16]).unwrap(),
            3,
            Limits::default(),
        )
        .unwrap();
        bridge.configure(2, 2, flags(), 7).unwrap();
        bridge.flush().unwrap();
        bridge.flush().unwrap();
        if premature_receipt {
            bridge.ready = true;
            let frame = bridge.wire.begin_frame(2, 2, flags()).unwrap();
            append(&io, Header::presented(frame.id()).unwrap(), &[]);
        } else {
            let wrong = Header::configure(
                bridge.wire.nonce(),
                3,
                3,
                2,
                flags(),
                Limits::default().wire,
            )
            .unwrap();
            append(&io, Header::ready(wrong).unwrap(), &[]);
        }
        assert_eq!(bridge.read_controls(), Err(Error::Protocol));
        assert_eq!(bridge.decoder.retained_bytes(), 0);
        assert!(bridge.frame.is_none());
        assert!(
            !bridge
                .wire
                .pending()
                .is_some_and(|frame| frame.receipt_observed())
        );
        bridge.cancel();
        assert!(io.borrow().cancelled);
        assert!(bridge.wire.is_terminal());
    }
}

#[test]
fn eof_cancel_deadline_and_staging_refusal_never_complete_a_wire_identity() {
    let io = Rc::new(RefCell::new(Bytes {
        eof: true,
        ..Default::default()
    }));
    let mut transport = OriginalTransport(io.clone());
    let mut bridge = FrameBridge::new(
        &mut transport,
        Nonce::new([0x17; 16]).unwrap(),
        3,
        Limits::default(),
    )
    .unwrap();
    let header = bridge.wire.begin_frame(2, 2, flags()).unwrap();
    assert_eq!(bridge.read_controls(), Err(Error::Closed));
    assert_eq!(bridge.wire.pending().unwrap().header(), header);
    bridge.deadline = Some(Instant::now());
    assert!(matches!(bridge.external_wake(), Err(Error::Timeout)));
    bridge.cancel();
    assert_eq!(bridge.wire.pending().unwrap().header(), header);
    assert!(!bridge.wire.pending().unwrap().receipt_observed());
    assert!(bridge.completion.is_none());
    drop(bridge);
    let limits = Limits {
        wire: pentax_session::Limits {
            max_staging_bytes: CHUNK,
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(matches!(
        FrameBridge::new(&mut transport, Nonce::new([0x17; 16]).unwrap(), 3, limits),
        Err(Error::Limit)
    ));
}
