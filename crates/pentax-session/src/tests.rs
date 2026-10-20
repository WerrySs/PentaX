// SPDX-License-Identifier: GPL-2.0-only
//! Original bounded messages. These fixtures confer no display capability.
use super::*;

#[test]
fn configuration_opens_before_ticks_without_allocating_pixels_or_consuming_a_frame() {
    let configuration = Header::configure(nonce(), 7, 2, 1, flags(), limits()).unwrap();
    assert_eq!(configuration.payload_len(), 0);
    assert_eq!(configuration.id().sequence(), 0);
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    let mut consumer = session();
    let feed = decoder
        .feed_checked(&configuration.encode(), HEADER_BYTES, |header| {
            consumer.check_incoming_header(header)
        })
        .unwrap();
    let message = feed.message.unwrap();
    assert_eq!(message.capacity(), 0);
    assert_eq!(
        consumer.receive_control(&message),
        Ok(ControlEvent::Configured(configuration))
    );
    assert_eq!(consumer.last_vblank(), 0);
    assert!(consumer.pending().is_none());
    let ready = Header::ready(configuration).unwrap();
    assert!(ready.matches_configuration(configuration));
    assert_eq!(ready.frame_flags(), configuration.frame_flags());
    let mut worker = session();
    assert_eq!(
        worker.receive_control(&decoded(ready, &[])),
        Ok(ControlEvent::Ready(ready))
    );
    assert_eq!(worker.last_vblank(), 0);
    assert!(worker.pending().is_none());
    assert_eq!(
        worker.begin_frame(2, 1, flags()).unwrap().id().sequence(),
        1
    );
    assert_eq!(
        Header::ready(Header::close(nonce(), 7).unwrap()),
        Err(Error::InvalidHeader)
    );
}

#[test]
fn configuration_echo_and_bounds_reject_stale_mismatched_or_pixel_bearing_handshakes() {
    let configuration = Header::configure(nonce(), 7, 2, 1, flags(), limits()).unwrap();
    for other in [
        Header::configure(nonce(), 7, 1, 1, flags(), limits()).unwrap(),
        Header::configure(nonce(), 8, 2, 1, flags(), limits()).unwrap(),
        Header::configure(Nonce::new([0x22; 16]).unwrap(), 7, 2, 1, flags(), limits()).unwrap(),
        Header::configure(
            nonce(),
            7,
            2,
            1,
            FrameFlags::new(AlphaMode::Straight, ColorEncoding::Srgb),
            limits(),
        )
        .unwrap(),
    ] {
        assert!(
            !Header::ready(other)
                .unwrap()
                .matches_configuration(configuration)
        );
    }
    for (offset, value, expected) in [
        (32, 1, Error::InvalidIdentity),
        (48, 0, Error::InvalidDimensions),
        (56, 1, Error::InvalidLength),
        (14, 4, Error::InvalidFlags),
    ] {
        let mut wire = configuration.encode();
        wire[offset] = value;
        let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
        assert_eq!(decoder.feed(&wire, HEADER_BYTES).unwrap_err(), expected);
        assert_eq!(decoder.retained_bytes(), 0);
    }
    let mut worker = session();
    let frame = worker.begin_frame(1, 1, flags()).unwrap();
    assert_eq!(
        worker.receive_control(&decoded(Header::ready(configuration).unwrap(), &[])),
        Err(Error::Backpressure)
    );
    assert_eq!(worker.pending().unwrap().header(), frame);
    assert!(!worker.pending().unwrap().receipt_observed());
    assert_eq!(
        Header::configure(nonce(), 7, 5, 1, flags(), limits()),
        Err(Error::InvalidDimensions)
    );
}

fn limits() -> Limits {
    Limits {
        max_dimension: 4,
        max_frame_bytes: 64,
        max_staging_bytes: 512,
    }
}
fn nonce() -> Nonce {
    Nonce::new([0x11; 16]).unwrap()
}
fn flags() -> FrameFlags {
    FrameFlags::new(AlphaMode::Premultiplied, ColorEncoding::Srgb)
}
fn session() -> Session {
    Session::new(nonce(), 7, limits()).unwrap()
}
fn bytes(header: Header, payload: &[u8]) -> Vec<u8> {
    let mut out = header.encode().to_vec();
    out.extend_from_slice(payload);
    out
}
fn decoded(header: Header, payload: &[u8]) -> Decoded {
    let wire = bytes(header, payload);
    Decoder::new(header.id().nonce(), header.id().generation(), limits())
        .unwrap()
        .feed(&wire, wire.capacity())
        .unwrap()
        .message
        .unwrap()
}

#[test]
fn fixed_wire_and_every_fragment_boundary_preserve_pixels_flags_and_message_boundaries() {
    let mut sender = session();
    let header = sender.begin_frame(2, 1, flags()).unwrap();
    let expected = [
        80, 78, 84, 88, 83, 69, 83, 0, 1, 0, 1, 0, 64, 0, 3, 0, 17, 17, 17, 17, 17, 17, 17, 17, 17,
        17, 17, 17, 17, 17, 17, 17, 1, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1,
        0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(header.encode(), expected);
    assert_eq!(header.stride(), Some(8));
    assert_eq!(header.frame_flags(), Some(flags()));
    assert_eq!(flags().alpha(), AlphaMode::Premultiplied);
    assert_eq!(flags().color(), ColorEncoding::Srgb);
    let pixels = [0, 0xff, 0x17, 0, 3, 5, 7, 0x80];
    let wire = bytes(header, &pixels);
    for split in 0..=wire.len() {
        let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
        let first = decoder.feed(&wire[..split], wire.capacity()).unwrap();
        assert_eq!(first.consumed, split);
        let message = if let Some(message) = first.message {
            message
        } else {
            let second = decoder.feed(&wire[split..], wire.capacity()).unwrap();
            assert_eq!(second.consumed, wire.len() - split);
            second.message.unwrap()
        };
        assert_eq!(message.header(), header);
        assert_eq!(message.payload(), pixels);
        assert!(wire.capacity() + message.capacity() <= limits().max_staging_bytes);
        assert_eq!(decoder.retained_bytes(), 0);
    }
    let vblank = bytes(Header::vblank(nonce(), 7).unwrap(), &9u64.to_le_bytes());
    let mut joined = wire.clone();
    joined.extend_from_slice(&vblank);
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    let first = decoder.feed(&joined, joined.capacity()).unwrap();
    assert_eq!(first.consumed, wire.len());
    assert_eq!(first.message.unwrap().payload(), pixels);
    let next = decoder
        .feed(&joined[first.consumed..], joined.capacity())
        .unwrap();
    assert_eq!(next.consumed, vblank.len());
    assert_eq!(next.message.unwrap().payload(), 9u64.to_le_bytes());
}

#[test]
fn malformed_fixed_fields_and_oversize_products_refuse_before_any_payload_allocation() {
    let frame = session().begin_frame(2, 2, flags()).unwrap().encode();
    let mut mutations = Vec::new();
    for (at, value, expected) in [
        (0, 0, Error::InvalidHeader),
        (8, 2, Error::UnsupportedVersion),
        (10, 7, Error::UnknownKind),
        (12, 63, Error::InvalidHeader),
        (14, 4, Error::InvalidFlags),
        (32, 0, Error::InvalidIdentity),
        (16, 0x22, Error::InvalidIdentity),
        (40, 0, Error::InvalidIdentity),
        (40, 8, Error::StaleGeneration),
        (48, 0, Error::InvalidDimensions),
        (48, 5, Error::InvalidDimensions),
        (56, 15, Error::InvalidLength),
        (60, 1, Error::InvalidHeader),
    ] {
        let mut value_bytes = frame;
        value_bytes[at] = value;
        mutations.push((value_bytes, expected));
    }
    let mut zero_nonce = frame;
    zero_nonce[16..32].fill(0);
    mutations.push((zero_nonce, Error::InvalidIdentity));
    let presented = Header::presented(Header::decode(&frame, limits()).unwrap().id())
        .unwrap()
        .encode();
    let mut dimensions = presented;
    dimensions[48] = 1;
    mutations.push((dimensions, Error::InvalidDimensions));
    let mut control_flags = presented;
    control_flags[14] = 1;
    mutations.push((control_flags, Error::InvalidFlags));
    let mut control_payload = presented;
    control_payload[56] = 8;
    mutations.push((control_payload, Error::InvalidLength));
    for (wire, expected) in mutations {
        let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
        assert_eq!(decoder.feed(&wire, wire.len()).unwrap_err(), expected);
        assert_eq!(decoder.retained_bytes(), 0);
        assert!(decoder.is_closed());
        assert_eq!(decoder.feed(&[], 0).unwrap_err(), Error::Closed);
    }
    let mut huge = frame;
    huge[48..52].copy_from_slice(&4096u32.to_le_bytes());
    huge[52..56].copy_from_slice(&1025u32.to_le_bytes());
    huge[56..60].copy_from_slice(&(4096u32 * 1025 * 4).to_le_bytes());
    let mut decoder = Decoder::new(nonce(), 7, Limits::default()).unwrap();
    assert_eq!(decoder.feed(&huge, huge.len()).unwrap_err(), Error::Limit);
    assert_eq!(decoder.retained_bytes(), 0);
    assert_eq!(Nonce::new([0; 16]), Err(Error::InvalidIdentity));
    assert!(Session::new(nonce(), 0, limits()).is_err());
    assert!(
        Decoder::new(
            nonce(),
            7,
            Limits {
                max_staging_bytes: MAX_STAGING_BYTES + 1,
                ..limits()
            }
        )
        .is_err()
    );
}

#[test]
fn simultaneous_actual_capacities_and_eof_release_partial_storage_without_implicit_completion() {
    let header = session().begin_frame(4, 4, flags()).unwrap();
    let wire = bytes(header, &[0xab; 64]);
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    assert_eq!(decoder.feed(&wire[..64], 449).unwrap_err(), Error::Limit);
    assert_eq!(decoder.retained_bytes(), 0);
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    assert!(
        decoder
            .feed(&wire[..65], wire.capacity())
            .unwrap()
            .message
            .is_none()
    );
    let capacity = decoder.retained_bytes();
    assert!(capacity >= 64);
    assert_eq!(decoder.feed(&[], 513 - capacity).unwrap_err(), Error::Limit);
    assert_eq!(decoder.retained_bytes(), 0);
    for prefix in [1, 63, 64, 70, 127] {
        let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
        assert!(
            decoder
                .feed(&wire[..prefix], wire.capacity())
                .unwrap()
                .message
                .is_none()
        );
        assert_eq!(decoder.end_of_stream(), Error::Truncated);
        assert!(decoder.is_closed());
        assert_eq!(decoder.retained_bytes(), 0);
    }
    let mut clean = Decoder::new(nonce(), 7, limits()).unwrap();
    assert_eq!(clean.end_of_stream(), Error::EndOfStream);
    let mut partial = Decoder::new(nonce(), 7, limits()).unwrap();
    partial.feed(&wire[..4], wire.capacity()).unwrap();
    assert_eq!(partial.advance_generation(8), Err(Error::Backpressure));
    let mut replacement = Decoder::new(nonce(), 7, limits()).unwrap();
    replacement.advance_generation(8).unwrap();
    assert_eq!(
        replacement.feed(&wire[..64], wire.capacity()).unwrap_err(),
        Error::StaleGeneration
    );
    assert_eq!(replacement.retained_bytes(), 0);
    let mut owner = session();
    let pending = owner.begin_frame(1, 1, flags()).unwrap();
    owner.terminate();
    assert_eq!(owner.pending().unwrap().header(), pending);
    assert!(!owner.pending().unwrap().receipt_observed());
    assert_eq!(owner.complete_frame(pending.id()), Err(Error::Closed));
}

#[test]
fn one_inflight_receipt_retry_and_generation_transition_do_not_reuse_identity() {
    let mut worker = session();
    let mut consumer = session();
    let frame = worker.begin_frame(2, 1, flags()).unwrap();
    consumer.accept_frame(frame).unwrap();
    assert_eq!(worker.begin_frame(1, 1, flags()), Err(Error::Backpressure));
    assert_eq!(worker.advance_generation(8), Err(Error::Backpressure));
    assert_eq!(
        worker.complete_frame(frame.id()),
        Err(Error::InvalidReceipt)
    );
    let receipt = consumer.receipt_for_presented(frame.id()).unwrap();
    assert_eq!(
        consumer.receipt_for_presented(frame.id()),
        Err(Error::DuplicateReceipt)
    );
    let receipt = decoded(receipt, &[]);
    assert_eq!(
        worker.receive_control(&receipt),
        Ok(ControlEvent::Presented(frame.id()))
    );
    // A simulated external clock failure performs no complete_frame call.
    assert!(worker.pending().unwrap().receipt_observed());
    assert_eq!(worker.begin_frame(1, 1, flags()), Err(Error::Backpressure));
    worker.complete_frame(frame.id()).unwrap();
    consumer.complete_frame(frame.id()).unwrap();
    assert!(worker.pending().is_none());
    worker.advance_generation(8).unwrap();
    consumer.advance_generation(8).unwrap();
    let next = worker
        .begin_frame(
            1,
            1,
            FrameFlags::new(AlphaMode::Straight, ColorEncoding::Unspecified),
        )
        .unwrap();
    assert_eq!((next.id().sequence(), next.id().generation()), (2, 8));
    consumer.accept_frame(next).unwrap();
    assert_eq!(
        next.frame_flags().unwrap().color(),
        ColorEncoding::Unspecified
    );
    assert_eq!(consumer.pending().unwrap().header(), next);
}

#[test]
fn foreign_reordered_duplicate_and_stale_controls_terminate_without_releasing_frame_ownership() {
    let mut worker = session();
    let frame = worker.begin_frame(1, 1, flags()).unwrap();
    let receipt = decoded(Header::presented(frame.id()).unwrap(), &[]);
    worker.receive_control(&receipt).unwrap();
    assert_eq!(
        worker.receive_control(&receipt),
        Err(Error::DuplicateReceipt)
    );
    assert!(worker.is_terminal());
    assert_eq!(worker.pending().unwrap().header(), frame);
    for (at, value, expected) in [
        (16, 0x22, Error::InvalidIdentity),
        (32, 2, Error::InvalidReceipt),
        (40, 8, Error::StaleGeneration),
    ] {
        let mut worker = session();
        let frame = worker.begin_frame(1, 1, flags()).unwrap();
        let mut wire = Header::presented(frame.id()).unwrap().encode();
        wire[at] = value;
        let header = Header::decode(&wire, limits()).unwrap();
        let message = decoded(header, &[]);
        assert_eq!(worker.receive_control(&message), Err(expected));
        assert_eq!(worker.pending().unwrap().header(), frame);
        assert!(worker.is_terminal());
    }
    let mut sender = session();
    let first = sender.begin_frame(1, 1, flags()).unwrap();
    sender.receipt_for_presented(first.id()).unwrap();
    sender.complete_frame(first.id()).unwrap();
    let second = sender.begin_frame(1, 1, flags()).unwrap();
    let mut consumer = session();
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    assert_eq!(
        decoder
            .feed_checked(&second.encode(), HEADER_BYTES, |header| consumer
                .check_incoming_header(header))
            .unwrap_err(),
        Error::UnexpectedSequence
    );
    assert_eq!(decoder.retained_bytes(), 0);
    assert!(!consumer.is_terminal());
    assert_eq!(
        consumer.accept_frame(second),
        Err(Error::UnexpectedSequence)
    );
    assert!(consumer.is_terminal());
    let mut consumer = session();
    consumer.accept_frame(first).unwrap();
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    assert_eq!(
        decoder
            .feed_checked(&second.encode(), HEADER_BYTES, |header| consumer
                .check_incoming_header(header))
            .unwrap_err(),
        Error::Backpressure
    );
    assert_eq!(decoder.retained_bytes(), 0);
    assert_eq!(consumer.accept_frame(second), Err(Error::Backpressure));
    assert_eq!(consumer.pending().unwrap().header(), first);
}

#[test]
fn vblank_is_independent_monotonic_and_close_or_sequence_exhaustion_never_acknowledges() {
    let mut worker = session();
    let frame = worker.begin_frame(1, 1, flags()).unwrap();
    let tick = decoded(Header::vblank(nonce(), 7).unwrap(), &99u64.to_le_bytes());
    assert_eq!(worker.receive_control(&tick), Ok(ControlEvent::Vblank(99)));
    assert_eq!(worker.last_vblank(), 99);
    assert!(!worker.pending().unwrap().receipt_observed());
    assert_eq!(worker.receive_control(&tick), Err(Error::InvalidVblank));
    assert_eq!(worker.pending().unwrap().header(), frame);
    let zero = bytes(Header::vblank(nonce(), 7).unwrap(), &[0; 8]);
    assert_eq!(
        Decoder::new(nonce(), 7, limits())
            .unwrap()
            .feed(&zero, zero.capacity())
            .unwrap_err(),
        Error::InvalidVblank
    );
    let mut worker = session();
    let frame = worker.begin_frame(1, 1, flags()).unwrap();
    let close_wire = bytes(Header::close(nonce(), 7).unwrap(), &[]);
    let mut decoder = Decoder::new(nonce(), 7, limits()).unwrap();
    let close = decoder
        .feed(&close_wire, close_wire.capacity())
        .unwrap()
        .message
        .unwrap();
    assert!(decoder.is_closed());
    assert_eq!(decoder.feed(&[], 0).unwrap_err(), Error::Closed);
    assert_eq!(worker.receive_control(&close), Ok(ControlEvent::Closed));
    assert_eq!(worker.pending().unwrap().header(), frame);
    assert_eq!(worker.complete_frame(frame.id()), Err(Error::Closed));
    let mut worker = session();
    crate::session::exhaust_sequence(&mut worker);
    assert_eq!(
        worker.begin_frame(1, 1, flags()),
        Err(Error::SequenceExhausted)
    );
    assert!(worker.pending().is_none());
    worker.advance_generation(8).unwrap();
    assert_eq!(
        worker.begin_frame(1, 1, flags()),
        Err(Error::SequenceExhausted)
    );
}
