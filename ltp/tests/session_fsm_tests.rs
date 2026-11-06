//! Exhaustive Unit & Property-Based Tests for LTP Session FSM, Handshake,
//! Claim Management, and Cancellation (RFC 5326 §§3-7).

use core::num::NonZeroU64;
use std::collections::BTreeSet;
use std::vec::Vec;

use bytes::Bytes;
use hardy_ltp::engine::session::RecvSession;
use hardy_ltp::engine::{Engine, EngineConfig, Notice};
use hardy_ltp::segment::{
    Checkpoint, DataSegKind, DataSegment, ReceptionClaim, ReportSegment, Segment, SegmentBody,
};
use hardy_ltp::types::{CancelReason, EngineId, SessionId};
use proptest::prelude::*;

// =========================================================================
// Helper utilities
// =========================================================================

fn make_engine_pair() -> (Engine, Engine) {
    let cfg_a = EngineConfig {
        engine_id: EngineId(1),
        max_segment_payload_len: 1000,
        checkpoint_rtx_limit: 3,
        report_rtx_limit: 3,
        session_inactivity_timeout_micros: 60_000_000, // 60s
        ..EngineConfig::default()
    };
    let cfg_b = EngineConfig {
        engine_id: EngineId(2),
        max_segment_payload_len: 1000,
        checkpoint_rtx_limit: 3,
        report_rtx_limit: 3,
        session_inactivity_timeout_micros: 60_000_000,
        ..EngineConfig::default()
    };
    (Engine::new(cfg_a), Engine::new(cfg_b))
}

fn drain_outbound(engine: &mut Engine) -> Vec<(EngineId, Vec<u8>)> {
    let mut out = Vec::new();
    while let Some(item) = engine.poll_output() {
        out.push(item);
    }
    out
}

// =========================================================================
// 3. Claim Scope Management & Out-of-Order Reception (Receiver RS Engine)
// =========================================================================

#[test]
fn test_contiguous_reception_produces_single_claim() {
    let (mut engine_a, mut engine_b) = make_engine_pair();
    let now = 1_000_000u64;

    let payload: Vec<u8> = (0..3000).map(|i| (i % 251) as u8).collect();
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 3000, now)
        .expect("request ok");

    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 3); // 3 segments of 1000 bytes

    // Deliver all segments in contiguous order
    for (_, bytes) in outbound {
        engine_b
            .handle_incoming_segment(EngineId(1), &bytes, now)
            .expect("deliver ok");
    }

    // Receiver should have generated a Report Segment (RS)
    let rs_out = drain_outbound(&mut engine_b);
    assert_eq!(rs_out.len(), 1);
    let (seg, _) = Segment::parse(&rs_out[0].1).expect("parse RS");
    assert_eq!(seg.session, session);

    match seg.body {
        SegmentBody::Report(rs) => {
            assert_eq!(rs.lower_bound, 0);
            assert_eq!(rs.upper_bound, 3000);
            // Single contiguous claim covering the full 3000 bytes
            assert_eq!(rs.claims.len(), 1);
            assert_eq!(
                rs.claims[0],
                ReceptionClaim {
                    offset: 0,
                    length: 3000,
                }
            );
        }
        other => panic!("expected ReportSegment, got {other:?}"),
    }

    // Assert complete Red block is delivered to upper layer
    let notice = engine_b.poll_notice().expect("block delivered notice");
    match notice {
        Notice::RedPartReceived {
            session: s,
            source,
            client_service,
            data,
            eob,
        } => {
            assert_eq!(s, session);
            assert_eq!(source, EngineId(1));
            assert_eq!(client_service, 1);
            assert_eq!(data, payload);
            assert!(eob);
        }
        other => panic!("expected RedPartReceived, got {other:?}"),
    }
}

#[test]
fn test_single_hole_middle_loss_produces_two_claims() {
    let mut engine_b = Engine::new(EngineConfig {
        engine_id: EngineId(2),
        max_segment_payload_len: 1000,
        ..EngineConfig::default()
    });
    let now = 1_000_000u64;
    let session = SessionId {
        originator: EngineId(1),
        number: 42,
    };

    let p1 = vec![0x11; 1000];
    let p3 = vec![0x33; 1000];

    // Segment 1: [0, 1000) Red
    let seg1 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 0,
            kind: DataSegKind::Red,
            data: &p1,
        }),
    };

    // Segment 3: [2000, 3000) Red Checkpoint & EORP & EOB (Segment 2 at [1000, 2000) was dropped!)
    let seg3 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 2000,
            kind: DataSegKind::RedCpEorpEob(Checkpoint {
                serial: NonZeroU64::new(1).unwrap(),
                report_serial: 0,
            }),
            data: &p3,
        }),
    };

    engine_b
        .handle_incoming_segment(EngineId(1), &seg1.encode_to_vec(), now)
        .expect("seg1 ok");
    engine_b
        .handle_incoming_segment(EngineId(1), &seg3.encode_to_vec(), now)
        .expect("seg3 ok");

    // Engine B must emit a Report Segment with explicitly TWO separate claims:
    // (offset: 0, length: 1000) and (offset: 2000, length: 1000)
    let outbound = drain_outbound(&mut engine_b);
    assert_eq!(outbound.len(), 1);

    let (parsed, _) = Segment::parse(&outbound[0].1).expect("parse RS");
    match parsed.body {
        SegmentBody::Report(rs) => {
            assert_eq!(rs.lower_bound, 0);
            assert_eq!(rs.upper_bound, 3000);
            assert_eq!(
                rs.claims.len(),
                2,
                "RS must have exactly two claims for the two received runs"
            );
            assert_eq!(
                rs.claims[0],
                ReceptionClaim {
                    offset: 0,
                    length: 1000,
                }
            );
            assert_eq!(
                rs.claims[1],
                ReceptionClaim {
                    offset: 2000,
                    length: 1000,
                }
            );
        }
        other => panic!("expected ReportSegment, got {other:?}"),
    }

    // Red block must NOT be delivered because hole [1000, 2000) is missing
    assert_eq!(engine_b.poll_notice(), None);
}

#[test]
fn test_overlapping_and_duplicate_segments() {
    let mut engine_b = Engine::new(EngineConfig {
        engine_id: EngineId(2),
        max_segment_payload_len: 2000,
        ..EngineConfig::default()
    });
    let now = 1_000_000u64;
    let session = SessionId {
        originator: EngineId(1),
        number: 77,
    };

    let p1 = vec![0xAA; 1000];
    let p_overlap = vec![0xBB; 1000]; // offset 500..1500

    // Send [0, 1000)
    let seg1 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 0,
            kind: DataSegKind::Red,
            data: &p1,
        }),
    };
    engine_b
        .handle_incoming_segment(EngineId(1), &seg1.encode_to_vec(), now)
        .unwrap();

    // Send overlapping segment [500, 1500) with Checkpoint
    let seg_overlap = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 500,
            kind: DataSegKind::RedCp(Checkpoint {
                serial: NonZeroU64::new(1).unwrap(),
                report_serial: 0,
            }),
            data: &p_overlap,
        }),
    };
    engine_b
        .handle_incoming_segment(EngineId(1), &seg_overlap.encode_to_vec(), now)
        .unwrap();

    // The RS should merge [0, 1000) and [500, 1500) into a single claim [0, 1500)
    let outbound = drain_outbound(&mut engine_b);
    assert_eq!(outbound.len(), 1);
    let (parsed, _) = Segment::parse(&outbound[0].1).unwrap();
    match parsed.body {
        SegmentBody::Report(rs) => {
            assert_eq!(rs.claims.len(), 1);
            assert_eq!(
                rs.claims[0],
                ReceptionClaim {
                    offset: 0,
                    length: 1500,
                }
            );
        }
        other => panic!("expected RS, got {other:?}"),
    }

    // Now inject exact duplicate of seg1 [0, 1000)
    engine_b
        .handle_incoming_segment(EngineId(1), &seg1.encode_to_vec(), now)
        .unwrap();
    // No error, state is maintained cleanly
}

#[test]
fn test_reverse_order_reception() {
    let mut engine_b = Engine::new(EngineConfig {
        engine_id: EngineId(2),
        max_segment_payload_len: 1000,
        ..EngineConfig::default()
    });
    let now = 1_000_000u64;
    let session = SessionId {
        originator: EngineId(1),
        number: 88,
    };

    let p1 = vec![0x10; 1000];
    let p2 = vec![0x20; 1000];
    let p3 = vec![0x30; 1000];

    let seg1 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 0,
            kind: DataSegKind::Red,
            data: &p1,
        }),
    };
    let seg2 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 1000,
            kind: DataSegKind::Red,
            data: &p2,
        }),
    };
    let seg3 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 2000,
            kind: DataSegKind::RedCpEorpEob(Checkpoint {
                serial: NonZeroU64::new(1).unwrap(),
                report_serial: 0,
            }),
            data: &p3,
        }),
    };

    // Feed in reverse order: 3, then 2, then 1
    engine_b
        .handle_incoming_segment(EngineId(1), &seg3.encode_to_vec(), now)
        .unwrap();
    engine_b
        .handle_incoming_segment(EngineId(1), &seg2.encode_to_vec(), now)
        .unwrap();
    engine_b
        .handle_incoming_segment(EngineId(1), &seg1.encode_to_vec(), now)
        .unwrap();

    // Once segment 1 arrives, contiguous block [0, 3000) is complete and delivered
    let notice = engine_b
        .poll_notice()
        .expect("must deliver complete red part");
    match notice {
        Notice::RedPartReceived { data, .. } => {
            assert_eq!(data.len(), 3000);
            assert_eq!(&data[0..1000], &p1[..]);
            assert_eq!(&data[1000..2000], &p2[..]);
            assert_eq!(&data[2000..3000], &p3[..]);
        }
        other => panic!("expected RedPartReceived, got {other:?}"),
    }
}

// Proptest for Out-of-Order sparse reassembly
proptest! {
    #![proptest_config(ProptestConfig::with_cases(50))]
    #[test]
    fn prop_sparse_reconstruction_invariant(
        data in proptest::collection::vec(any::<u8>(), 1..=4096),
        cuts in proptest::collection::vec(1usize..4096, 1..=8),
        seed in any::<u64>(),
    ) {
        let block_len = data.len();
        let mut cut_points: BTreeSet<usize> = cuts.into_iter().filter(|&c| c < block_len).collect();
        cut_points.insert(0);
        cut_points.insert(block_len);

        let points: Vec<usize> = cut_points.into_iter().collect();
        let mut slices: Vec<(usize, usize)> = Vec::new();
        for i in 0..points.len() - 1 {
            slices.push((points[i], points[i + 1]));
        }

        // Permute slice arrival using a deterministic pseudo-random shuffle
        let mut permuted = slices.clone();
        let mut rng = seed;
        for i in (1..permuted.len()).rev() {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            let j = (rng as usize) % (i + 1);
            permuted.swap(i, j);
        }

        let mut recv_session = RecvSession::new(
            SessionId { originator: EngineId(1), number: 1 },
            EngineId(1),
            1,
            0,
            1_000_000,
        );
        recv_session.red_part_len = Some(block_len as u64);

        for (start, end) in permuted {
            let slice = &data[start..end];
            let ok = recv_session.insert_red(start as u64, slice, 10_000_000);
            prop_assert!(ok);
        }

        prop_assert!(recv_session.is_red_complete());
        prop_assert_eq!(&recv_session.red_buf[..block_len], &data[..]);
    }
}

// =========================================================================
// 4. Checkpoint (CP) & Report Segment (RS) Handshake Machine
// =========================================================================

#[test]
fn test_sender_marks_last_red_segment_with_checkpoint() {
    let (mut engine_a, _) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x42; 2500]; // 1000 + 1000 + 500 = 3 segments
    engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 2500, now)
        .unwrap();

    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 3);

    // Segment 1: Red data (type 0)
    let (s1, _) = Segment::parse(&outbound[0].1).unwrap();
    assert_eq!(s1.type_code(), 0);

    // Segment 2: Red data (type 0)
    let (s2, _) = Segment::parse(&outbound[1].1).unwrap();
    assert_eq!(s2.type_code(), 0);

    // Segment 3: Red Checkpoint + EORP + EOB (type 3)
    let (s3, _) = Segment::parse(&outbound[2].1).unwrap();
    assert_eq!(
        s3.type_code(),
        3,
        "Last red segment of all-red block must be type 3"
    );
    match s3.body {
        SegmentBody::Data(ds) => {
            assert!(
                ds.kind.checkpoint().is_some(),
                "Must carry checkpoint serial"
            );
            assert!(ds.kind.is_eorp());
            assert!(ds.kind.is_eob());
        }
        other => panic!("expected DataSegment, got {other:?}"),
    }
}

#[test]
fn test_checkpoint_retransmission_timer_and_cancellation() {
    let (mut engine_a, _) = make_engine_pair();
    let mut now = 1_000_000u64;

    let payload = vec![0x42; 500]; // Single CP segment
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 500, now)
        .unwrap();

    // Drain initial transmission
    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 1);

    // CP is in flight, RS is lost (never arrives).
    // Advance time and trigger timeouts up to checkpoint_rtx_limit (3)
    for _retry in 1..=3 {
        let deadline = engine_a.next_timer_deadline().expect("deadline armed");
        assert!(deadline > now);
        now = deadline;
        engine_a.handle_timer_tick(now);

        // Sender retransmits the checkpoint
        let rtx = drain_outbound(&mut engine_a);
        assert_eq!(rtx.len(), 1, "Must retransmit checkpoint");
        let (s, _) = Segment::parse(&rtx[0].1).unwrap();
        assert_eq!(s.type_code(), 3);
    }

    // Now advance time once more beyond retry limit:
    let final_deadline = engine_a.next_timer_deadline().expect("cancel deadline");
    now = final_deadline;
    engine_a.handle_timer_tick(now);

    // Sender must emit CancelSender (CS, type 12) with reason code Rlexc (retry limit exceeded)
    let cancel_out = drain_outbound(&mut engine_a);
    assert_eq!(cancel_out.len(), 1);
    let (s_cancel, _) = Segment::parse(&cancel_out[0].1).unwrap();
    assert_eq!(s_cancel.session, session);
    assert_eq!(s_cancel.type_code(), 12);
    match s_cancel.body {
        SegmentBody::CancelSender { reason } => {
            assert_eq!(reason, CancelReason::Rlexc);
        }
        other => panic!("expected CancelSender, got {other:?}"),
    }

    // TransmissionSessionCancelled notice emitted
    let notice = engine_a.poll_notice().expect("notice cancelled");
    assert_eq!(
        notice,
        Notice::TransmissionSessionCancelled {
            session,
            reason: CancelReason::Rlexc
        }
    );
}

#[test]
fn test_delayed_checkpoint_retains_state_and_re_emits_cached_rs() {
    let (mut engine_a, mut engine_b) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x99; 500];
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 500, now)
        .unwrap();

    let outbound = drain_outbound(&mut engine_a);
    let cp_bytes = outbound[0].1.clone();

    // Receiver B receives CP
    engine_b
        .handle_incoming_segment(EngineId(1), &cp_bytes, now)
        .unwrap();

    // B emits RS and delivers block
    let b_rs = drain_outbound(&mut engine_b);
    assert_eq!(b_rs.len(), 1);
    assert!(engine_b.poll_notice().is_some());

    // Deliver RS to A so A acknowledges it with RA
    engine_a
        .handle_incoming_segment(EngineId(2), &b_rs[0].1, now)
        .unwrap();
    let a_ra = drain_outbound(&mut engine_a);
    assert_eq!(a_ra.len(), 1);

    // Deliver RA to B -> B closes and retains session state
    engine_b
        .handle_incoming_segment(EngineId(1), &a_ra[0].1, now)
        .unwrap();

    // Now simulate sender retransmitting an old duplicate CP (e.g. delayed in network)
    engine_b
        .handle_incoming_segment(EngineId(1), &cp_bytes, now + 1000)
        .unwrap();

    // Assert: B retains session and re-emits cached RS instead of CR or error!
    let b_re_rs = drain_outbound(&mut engine_b);
    assert_eq!(
        b_re_rs.len(),
        1,
        "Must re-emit RS for duplicate delayed checkpoint"
    );
    let (re_seg, _) = Segment::parse(&b_re_rs[0].1).unwrap();
    assert_eq!(re_seg.session, session);
    assert_eq!(re_seg.type_code(), 8); // Report Segment
}

#[test]
fn test_report_acknowledgment_rasp_clears_receiver_retained_state() {
    let (mut engine_a, mut engine_b) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x77; 500];
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 500, now)
        .unwrap();

    let outbound = drain_outbound(&mut engine_a);
    engine_b
        .handle_incoming_segment(EngineId(1), &outbound[0].1, now)
        .unwrap();

    // Receiver generates RS
    let rs_out = drain_outbound(&mut engine_b);
    let (rs_seg, _) = Segment::parse(&rs_out[0].1).unwrap();
    let rs_serial = match rs_seg.body {
        SegmentBody::Report(rs) => rs.report_serial.get(),
        _ => panic!("not RS"),
    };

    // Sender receives RS, must emit RA (type 9)
    engine_a
        .handle_incoming_segment(EngineId(2), &rs_out[0].1, now)
        .unwrap();
    let ra_out = drain_outbound(&mut engine_a);
    assert_eq!(ra_out.len(), 1);
    let (ra_seg, _) = Segment::parse(&ra_out[0].1).unwrap();
    assert_eq!(ra_seg.session, session);
    assert_eq!(ra_seg.type_code(), 9); // RA
    match ra_seg.body {
        SegmentBody::ReportAck { report_serial } => {
            assert_eq!(report_serial, rs_serial);
        }
        other => panic!("expected ReportAck, got {other:?}"),
    }

    // Deliver RA to receiver B -> clears RS timer and closes session
    engine_b
        .handle_incoming_segment(EngineId(1), &ra_out[0].1, now)
        .unwrap();
    assert_eq!(engine_b.active_recv_sessions(), 0);
}

// =========================================================================
// 5. Mixed Red/Green & All-Green Data Sessions
// =========================================================================

#[test]
fn test_all_green_data_session() {
    let (mut engine_a, mut engine_b) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x55; 2500]; // 3 green segments: 1000, 1000, 500
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 0, now)
        .expect("all green request ok");

    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 3);

    // Segment 1: Green (type 4)
    let (s1, _) = Segment::parse(&outbound[0].1).unwrap();
    assert_eq!(s1.type_code(), 4);

    // Segment 2: Green (type 4)
    let (s2, _) = Segment::parse(&outbound[1].1).unwrap();
    assert_eq!(s2.type_code(), 4);

    // Segment 3: Green EOB (type 7)
    let (s3, _) = Segment::parse(&outbound[2].1).unwrap();
    assert_eq!(s3.type_code(), 7);

    // Deliver to Receiver B
    for (_, bytes) in outbound {
        engine_b
            .handle_incoming_segment(EngineId(1), &bytes, now)
            .unwrap();
    }

    // Receiver must NOT generate any RS (outbound is empty)
    assert_eq!(drain_outbound(&mut engine_b).len(), 0);

    // Receiver delivers 3 GreenSegmentArrived notices
    let mut received_bytes = Vec::new();
    let mut seen_eob = false;
    while let Some(notice) = engine_b.poll_notice() {
        match notice {
            Notice::GreenSegmentArrived {
                session: s,
                data,
                eob,
                ..
            } => {
                assert_eq!(s, session);
                received_bytes.extend_from_slice(&data);
                if eob {
                    seen_eob = true;
                }
            }
            other => panic!("unexpected notice: {other:?}"),
        }
    }
    assert_eq!(received_bytes, payload);
    assert!(seen_eob);
}

#[test]
fn test_mixed_red_green_session() {
    let (mut engine_a, mut engine_b) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x88; 2000]; // 1000 red + 1000 green
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 1000, now)
        .unwrap();

    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 2);

    // First segment is Red Checkpoint + EORP, not EOB (type 2)
    let (s1, _) = Segment::parse(&outbound[0].1).unwrap();
    assert_eq!(s1.type_code(), 2);

    // Second segment is Green EOB (type 7)
    let (s2, _) = Segment::parse(&outbound[1].1).unwrap();
    assert_eq!(s2.type_code(), 7);

    // Deliver to B
    for (_, bytes) in outbound {
        engine_b
            .handle_incoming_segment(EngineId(1), &bytes, now)
            .unwrap();
    }

    // B emitted RS for the red part
    let b_rs = drain_outbound(&mut engine_b);
    assert_eq!(b_rs.len(), 1);
    let (rs_seg, _) = Segment::parse(&b_rs[0].1).unwrap();
    assert_eq!(rs_seg.type_code(), 8);

    // B emits RedPartReceived once the complete red part is received
    let red_notice = engine_b.poll_notice().expect("red notice");
    match red_notice {
        Notice::RedPartReceived {
            session: s,
            data,
            eob,
            ..
        } => {
            assert_eq!(s, session);
            assert_eq!(data, &payload[0..1000]);
            assert!(!eob, "Red part was not the end of the block");
        }
        other => panic!("expected red notice, got {other:?}"),
    }

    // B emits GreenSegmentArrived immediately upon green segment ingestion
    let green_notice = engine_b.poll_notice().expect("green notice");
    match green_notice {
        Notice::GreenSegmentArrived {
            session: s,
            offset,
            data,
            eob,
            ..
        } => {
            assert_eq!(s, session);
            assert_eq!(offset, 1000);
            assert_eq!(data, &payload[1000..2000]);
            assert!(eob);
        }
        other => panic!("expected green notice, got {other:?}"),
    }

    // Deliver RS to A, then RA to B
    engine_a
        .handle_incoming_segment(EngineId(2), &b_rs[0].1, now)
        .unwrap();
    let a_ra = drain_outbound(&mut engine_a);
    assert_eq!(a_ra.len(), 1);

    engine_b
        .handle_incoming_segment(EngineId(1), &a_ra[0].1, now)
        .unwrap();

    // Sender transmission completes
    let a_notice = engine_a.poll_notice().expect("transmission completed");
    assert_eq!(a_notice, Notice::TransmissionSessionCompleted { session });
}

// =========================================================================
// 6. Cancellation State Machine (CR / CAS / CAR)
// =========================================================================

#[test]
fn test_sender_initiated_cancellation_and_cas_reply() {
    let mut engine_b = Engine::new(EngineConfig {
        engine_id: EngineId(2),
        ..EngineConfig::default()
    });
    let now = 1_000_000u64;
    let session = SessionId {
        originator: EngineId(1),
        number: 99,
    };

    // Receiver starts receiving some data
    let p = vec![0x12; 500];
    let seg = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 0,
            kind: DataSegKind::Red,
            data: &p,
        }),
    };
    engine_b
        .handle_incoming_segment(EngineId(1), &seg.encode_to_vec(), now)
        .unwrap();
    assert_eq!(engine_b.active_recv_sessions(), 1);

    // Inject CancelSender (CS, type 12)
    let cs = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::CancelSender {
            reason: CancelReason::UsrCncld,
        },
    };
    engine_b
        .handle_incoming_segment(EngineId(1), &cs.encode_to_vec(), now)
        .unwrap();

    // Receiver must halt assembly and clean buffer (active_recv_sessions == 0)
    assert_eq!(engine_b.active_recv_sessions(), 0);

    // Receiver emits TransmissionSessionCancelled notice
    let notice = engine_b.poll_notice().expect("cancelled notice");
    assert_eq!(
        notice,
        Notice::TransmissionSessionCancelled {
            session,
            reason: CancelReason::UsrCncld
        }
    );

    // Receiver replies with CancelAckSender (CAS, type 13)
    let replies = drain_outbound(&mut engine_b);
    assert_eq!(replies.len(), 1);
    let (reply_seg, _) = Segment::parse(&replies[0].1).unwrap();
    assert_eq!(reply_seg.session, session);
    assert_eq!(reply_seg.type_code(), 13);
    assert_eq!(reply_seg.body, SegmentBody::CancelAckSender);
}

#[test]
fn test_receiver_initiated_cancellation_and_car_reply() {
    let (mut engine_a, _) = make_engine_pair();
    let now = 1_000_000u64;

    let payload = vec![0x34; 1000];
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 1000, now)
        .unwrap();
    assert_eq!(engine_a.active_send_sessions(), 1);

    // Drain initial data segment
    drain_outbound(&mut engine_a);

    // Inject CancelReceiver (CR, type 14) from peer
    let cr = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::CancelReceiver {
            reason: CancelReason::Rlexc,
        },
    };
    engine_a
        .handle_incoming_segment(EngineId(2), &cr.encode_to_vec(), now)
        .unwrap();

    // Sender halts transmission and cleans session
    assert_eq!(engine_a.active_send_sessions(), 0);

    // Sender emits TransmissionSessionCancelled notice
    let notice = engine_a.poll_notice().expect("cancelled notice");
    assert_eq!(
        notice,
        Notice::TransmissionSessionCancelled {
            session,
            reason: CancelReason::Rlexc
        }
    );

    // Sender replies with CancelAckReceiver (CAR, type 15)
    let replies = drain_outbound(&mut engine_a);
    assert_eq!(replies.len(), 1);
    let (reply_seg, _) = Segment::parse(&replies[0].1).unwrap();
    assert_eq!(reply_seg.session, session);
    assert_eq!(reply_seg.type_code(), 15);
    assert_eq!(reply_seg.body, SegmentBody::CancelAckReceiver);
}

#[test]
fn test_inflight_data_discard_after_cancellation() {
    let mut engine_b = Engine::new(EngineConfig {
        engine_id: EngineId(2),
        ..EngineConfig::default()
    });
    let now = 1_000_000u64;
    let session = SessionId {
        originator: EngineId(1),
        number: 101,
    };

    // Inject CancelSender (CS, type 12)
    let cs = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::CancelSender {
            reason: CancelReason::UsrCncld,
        },
    };
    engine_b
        .handle_incoming_segment(EngineId(1), &cs.encode_to_vec(), now)
        .unwrap();

    // Drain CAS reply and notice
    drain_outbound(&mut engine_b);
    let _ = engine_b.poll_notice();

    // Now inject in-flight data segments arriving late for this cancelled session
    let late_data = vec![0xFF; 500];
    let late_seg = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service: 1,
            offset: 0,
            kind: DataSegKind::RedCpEorpEob(Checkpoint {
                serial: NonZeroU64::new(1).unwrap(),
                report_serial: 0,
            }),
            data: &late_data,
        }),
    };

    // Late segment must be silently discarded
    engine_b
        .handle_incoming_segment(EngineId(1), &late_seg.encode_to_vec(), now + 10)
        .unwrap();

    // Assert no session was created, no outbound RS was emitted, no notice
    assert_eq!(engine_b.active_recv_sessions(), 0);
    assert_eq!(drain_outbound(&mut engine_b).len(), 0);
    assert_eq!(engine_b.poll_notice(), None);
}

#[test]
fn test_retransmission_cycles_limit_exceeded_cancels_with_rxmtcycexc() {
    let cfg_a = EngineConfig {
        engine_id: EngineId(1),
        max_segment_payload_len: 1000,
        max_rxmt_cycles: 2, // Allow at most 2 retransmission cycles
        ..EngineConfig::default()
    };
    let mut engine_a = Engine::new(cfg_a);
    let now = 1_000_000u64;

    let payload = vec![0xEE; 1000];
    let session = engine_a
        .request_transmission(EngineId(2), 1, Bytes::copy_from_slice(&payload), 1000, now)
        .unwrap();

    // Drain initial transmission (CP serial 1)
    let outbound = drain_outbound(&mut engine_a);
    assert_eq!(outbound.len(), 1);

    // Simulated adversary / adversarial peer repeatedly claims tiny slices to trigger an infinite loop:
    // Cycle 1: RS claims [0, 100), leaving [100, 1000) missing
    let rs1 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Report(ReportSegment {
            report_serial: NonZeroU64::new(1).unwrap(),
            checkpoint_serial: 1,
            upper_bound: 1000,
            lower_bound: 0,
            claims: vec![ReceptionClaim {
                offset: 0,
                length: 100,
            }],
        }),
    };
    engine_a
        .handle_incoming_segment(EngineId(2), &rs1.encode_to_vec(), now)
        .unwrap();

    // Sender emits RA (type 9) + Retransmission chunk with CP 2 (cycle 1 <= 2)
    let out1 = drain_outbound(&mut engine_a);
    assert_eq!(out1.len(), 2); // 1 RA + 1 retransmission
    assert_eq!(engine_a.active_send_sessions(), 1);

    // Cycle 2: RS claims [0, 200), leaving [200, 1000) missing
    let rs2 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Report(ReportSegment {
            report_serial: NonZeroU64::new(2).unwrap(),
            checkpoint_serial: 2,
            upper_bound: 1000,
            lower_bound: 0,
            claims: vec![ReceptionClaim {
                offset: 0,
                length: 200,
            }],
        }),
    };
    engine_a
        .handle_incoming_segment(EngineId(2), &rs2.encode_to_vec(), now)
        .unwrap();

    // Sender emits RA + Retransmission chunk with CP 3 (cycle 2 <= 2)
    let out2 = drain_outbound(&mut engine_a);
    assert_eq!(out2.len(), 2);
    assert_eq!(engine_a.active_send_sessions(), 1);

    // Cycle 3: RS claims [0, 300). This would exceed max_rxmt_cycles (2).
    let rs3 = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Report(ReportSegment {
            report_serial: NonZeroU64::new(3).unwrap(),
            checkpoint_serial: 3,
            upper_bound: 1000,
            lower_bound: 0,
            claims: vec![ReceptionClaim {
                offset: 0,
                length: 300,
            }],
        }),
    };
    engine_a
        .handle_incoming_segment(EngineId(2), &rs3.encode_to_vec(), now)
        .unwrap();

    // Sender MUST NOT retransmit. It MUST emit RA (unconditional) + CancelSender with RxmtCycExc!
    let out3 = drain_outbound(&mut engine_a);
    assert_eq!(out3.len(), 2); // 1 RA + 1 CS
    let (ra_seg, _) = Segment::parse(&out3[0].1).unwrap();
    assert_eq!(ra_seg.type_code(), 9); // RA

    let (cs_seg, _) = Segment::parse(&out3[1].1).unwrap();
    assert_eq!(cs_seg.type_code(), 12); // CancelSender
    match cs_seg.body {
        SegmentBody::CancelSender { reason } => {
            assert_eq!(
                reason,
                CancelReason::RxmtCycExc,
                "Must cancel with RXMTCYCEXC"
            );
        }
        other => panic!("expected CancelSender, got {other:?}"),
    }

    // Sender emits notice
    let notice = engine_a.poll_notice().expect("cancelled notice");
    assert_eq!(
        notice,
        Notice::TransmissionSessionCancelled {
            session,
            reason: CancelReason::RxmtCycExc,
        }
    );
}
