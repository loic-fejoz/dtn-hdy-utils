//! Exhaustive Unit & Property-Based Tests for LTP Segment Header Sizing & Dynamic MTU (RFC 5326 §3).

use std::num::NonZeroU64;

use hardy_ltp::sdnv;
use hardy_ltp::segment::{
    Checkpoint, DataSegKind, DataSegment, Extension, MtuTooSmallError, Segment, SegmentBody,
};
use hardy_ltp::types::{EngineId, SessionId};
use proptest::prelude::*;

#[test]
fn test_max_payload_formula_and_exact_fit() {
    let session = SessionId {
        originator: EngineId(1),
        number: 1,
    };
    let client_service = 1;
    let offset = 0;
    let kind = DataSegKind::Red;

    // Fixed overhead without extensions:
    // Control: 1
    // Session originator: 1 (sdnv(1))
    // Session number: 1 (sdnv(1))
    // Extension counts: 1
    // Client service: 1 (sdnv(1))
    // Offset: 1 (sdnv(0))
    // Total fixed = 6 bytes.
    let fixed = DataSegment::fixed_header_overhead(session, client_service, offset, kind, &[], &[]);
    assert_eq!(fixed, 6);

    let mtu = 100;
    let max_p =
        DataSegment::max_payload_for_mtu(mtu, session, client_service, offset, kind, &[], &[])
            .expect("mtu 100 is plenty for 6-byte header");

    // With available = 100 - 6 = 94:
    // If P = 93: sdnv(93) = 1 byte -> 93 + 1 = 94. Wire len = 6 + 94 = 100 == MTU.
    assert_eq!(max_p, 93);

    let payload = vec![0xAB; max_p];
    let seg = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service,
            offset,
            kind,
            data: &payload,
        }),
    };
    let encoded = seg.encode_to_vec();
    assert_eq!(encoded.len(), mtu, "Segment must exactly fill MTU");

    // Now test with Header & Trailer Extensions:
    // Header ext: tag=1, value=[0xAA; 4] -> 1 (tag) + 1 (len SDNV) + 4 = 6 bytes
    let h_val = [0xAA; 4];
    let h_ext = [Extension {
        tag: 1,
        value: &h_val,
    }];
    // Trailer ext: tag=2, value=[0xBB; 3] -> 1 (tag) + 1 (len SDNV) + 3 = 5 bytes
    let t_val = [0xBB; 3];
    let t_ext = [Extension {
        tag: 2,
        value: &t_val,
    }];

    let fixed_with_ext =
        DataSegment::fixed_header_overhead(session, client_service, offset, kind, &h_ext, &t_ext);
    assert_eq!(fixed_with_ext, 6 + 6 + 5); // 17 bytes

    let max_p_ext = DataSegment::max_payload_for_mtu(
        mtu,
        session,
        client_service,
        offset,
        kind,
        &h_ext,
        &t_ext,
    )
    .expect("mtu 100 with extensions fits");

    // available = 100 - 17 = 83 -> max_p_ext = 82 (since 82 + 1 = 83)
    assert_eq!(max_p_ext, 82);

    let payload_ext = vec![0xCD; max_p_ext];
    let seg_ext = Segment {
        session,
        header_ext: h_ext.to_vec(),
        trailer_ext: t_ext.to_vec(),
        body: SegmentBody::Data(DataSegment {
            client_service,
            offset,
            kind,
            data: &payload_ext,
        }),
    };
    let encoded_ext = seg_ext.encode_to_vec();
    assert_eq!(
        encoded_ext.len(),
        mtu,
        "Segment with extensions must exactly fill MTU"
    );
}

#[test]
fn test_dynamic_expansion_engine_and_session_id() {
    // 1-byte Session ID components
    let small_session = SessionId {
        originator: EngineId(1), // sdnv len 1
        number: 1,               // sdnv len 1
    };

    // Large Session ID components: 9-byte originator and 10-byte session number
    let large_session = SessionId {
        originator: EngineId(1 << 56), // 9-byte SDNV
        number: u64::MAX,              // 10-byte SDNV
    };

    let client_service = 1;
    let offset = 0;
    let kind = DataSegKind::Red;
    let mtu = 100;

    let small_fixed =
        DataSegment::fixed_header_overhead(small_session, client_service, offset, kind, &[], &[]);
    let large_fixed =
        DataSegment::fixed_header_overhead(large_session, client_service, offset, kind, &[], &[]);

    // Small: 1 + 1 + 1 + 1 + 1 + 1 = 6
    assert_eq!(small_fixed, 6);
    // Large: 1 + 9 + 10 + 1 + 1 + 1 = 23
    assert_eq!(large_fixed, 23);
    assert_eq!(large_fixed - small_fixed, 17);

    let small_max_p = DataSegment::max_payload_for_mtu(
        mtu,
        small_session,
        client_service,
        offset,
        kind,
        &[],
        &[],
    )
    .unwrap();
    let large_max_p = DataSegment::max_payload_for_mtu(
        mtu,
        large_session,
        client_service,
        offset,
        kind,
        &[],
        &[],
    )
    .unwrap();

    assert_eq!(small_max_p, 93);
    // available = 100 - 23 = 77 -> max_p = 76 (76 + 1 = 77)
    assert_eq!(large_max_p, 76);
    assert_eq!(small_max_p - large_max_p, 17);

    // Verify encoded large segment does not exceed MTU
    let large_payload = vec![0xEF; large_max_p];
    let large_seg = Segment {
        session: large_session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service,
            offset,
            kind,
            data: &large_payload,
        }),
    };
    let large_encoded = large_seg.encode_to_vec();
    assert_eq!(large_encoded.len(), mtu);
}

#[test]
fn test_zero_payload_and_mtu_too_small_edge_cases() {
    let session = SessionId {
        originator: EngineId(42),
        number: 100,
    };
    let client_service = 1;
    let offset = 0;
    let kind = DataSegKind::RedCp(Checkpoint {
        serial: NonZeroU64::new(1).unwrap(),
        report_serial: 0,
    });

    // Control: 1
    // Originator (42): 1
    // Session number (100): 1
    // Extension counts: 1
    // Client service: 1
    // Offset: 1
    // CP serial (1): 1
    // CP report serial (0): 1
    // Total fixed = 8 bytes.
    let fixed = DataSegment::fixed_header_overhead(session, client_service, offset, kind, &[], &[]);
    assert_eq!(fixed, 8);

    // Minimum MTU required for 0-byte payload: fixed + 1 (sdnv(0) = 1) = 9
    let min_mtu = fixed + 1;

    // When MTU < min_mtu: must return MtuTooSmallError
    for small_mtu in 0..min_mtu {
        let err = DataSegment::max_payload_for_mtu(
            small_mtu,
            session,
            client_service,
            offset,
            kind,
            &[],
            &[],
        )
        .unwrap_err();
        assert_eq!(
            err,
            MtuTooSmallError {
                mtu: small_mtu,
                header_len: min_mtu,
            }
        );
    }

    // When MTU == min_mtu: max payload is exactly 0
    let zero_p =
        DataSegment::max_payload_for_mtu(min_mtu, session, client_service, offset, kind, &[], &[])
            .expect("MTU == min_mtu allows 0 payload");
    assert_eq!(zero_p, 0);

    let seg = Segment {
        session,
        header_ext: Vec::new(),
        trailer_ext: Vec::new(),
        body: SegmentBody::Data(DataSegment {
            client_service,
            offset,
            kind,
            data: &[],
        }),
    };
    let encoded = seg.encode_to_vec();
    assert_eq!(encoded.len(), min_mtu);
}

#[test]
fn test_sdnv_multibyte_length_boundary_in_mtu() {
    let session = SessionId {
        originator: EngineId(1),
        number: 1,
    };
    let client_service = 1;
    let offset = 0;
    let kind = DataSegKind::Red;
    let fixed = DataSegment::fixed_header_overhead(session, client_service, offset, kind, &[], &[]);
    assert_eq!(fixed, 6);

    // At MTU = fixed + 128 = 134:
    // available = 128.
    // If P = 127, sdnv(127) = 1 byte -> 127 + 1 = 128 <= 128. Fits!
    // If P = 128, sdnv(128) = 2 bytes -> 128 + 2 = 130 > 128. Does not fit!
    // So max payload must be 127.
    let mtu_134 = fixed + 128;
    let p_134 =
        DataSegment::max_payload_for_mtu(mtu_134, session, client_service, offset, kind, &[], &[])
            .unwrap();
    assert_eq!(p_134, 127);

    // At MTU = fixed + 129 = 135:
    // available = 129.
    // If P = 127, 127 + 1 = 128 <= 129. Fits.
    // If P = 128, 128 + 2 = 130 > 129. Does not fit!
    // So max payload is still 127!
    let mtu_135 = fixed + 129;
    let p_135 =
        DataSegment::max_payload_for_mtu(mtu_135, session, client_service, offset, kind, &[], &[])
            .unwrap();
    assert_eq!(p_135, 127);

    // At MTU = fixed + 130 = 136:
    // available = 130.
    // If P = 128, 128 + 2 = 130 <= 130. Fits!
    // So max payload becomes 128!
    let mtu_136 = fixed + 130;
    let p_136 =
        DataSegment::max_payload_for_mtu(mtu_136, session, client_service, offset, kind, &[], &[])
            .unwrap();
    assert_eq!(p_136, 128);

    // Verify encoded lengths for both cases:
    for (m, p) in [(mtu_134, p_134), (mtu_135, p_135), (mtu_136, p_136)] {
        let payload = vec![0; p];
        let seg = Segment {
            session,
            header_ext: Vec::new(),
            trailer_ext: Vec::new(),
            body: SegmentBody::Data(DataSegment {
                client_service,
                offset,
                kind,
                data: &payload,
            }),
        };
        assert!(seg.exact_encoded_len(&[], &[]) <= m);
        assert_eq!(seg.encode_to_vec().len(), seg.exact_encoded_len(&[], &[]));
    }
}

proptest! {
    /// Property: For any arbitrary MTU, SessionId, ClientService, Offset, and Extensions,
    /// max_payload_for_mtu never panics, and the resulting segment wire length <= MTU,
    /// while (max_payload + 1) strictly exceeds MTU.
    #[test]
    fn prop_dynamic_mtu_calculation(
        originator in any::<u64>(),
        session_num in any::<u64>(),
        client_service in any::<u64>(),
        offset in any::<u64>(),
        mtu in 0usize..4096,
    ) {
        let session = SessionId {
            originator: EngineId(originator),
            number: session_num,
        };
        let kind = DataSegKind::Red;

        match DataSegment::max_payload_for_mtu(mtu, session, client_service, offset, kind, &[], &[]) {
            Err(err) => {
                let fixed = DataSegment::fixed_header_overhead(session, client_service, offset, kind, &[], &[]);
                prop_assert_eq!(err.header_len, fixed + 1);
                prop_assert!(mtu < fixed + 1);
            }
            Ok(max_p) => {
                let fixed = DataSegment::fixed_header_overhead(session, client_service, offset, kind, &[], &[]);
                let wire_len = fixed + sdnv::encoded_len(max_p as u64) + max_p;
                prop_assert!(wire_len <= mtu, "Calculated wire len {} must be <= MTU {}", wire_len, mtu);

                // Check that max_p + 1 would strictly exceed MTU
                let overflow_p = max_p + 1;
                let overflow_wire_len = fixed + sdnv::encoded_len(overflow_p as u64) + overflow_p;
                prop_assert!(overflow_wire_len > mtu, "max_p + 1 wire len {} must exceed MTU {}", overflow_wire_len, mtu);
            }
        }
    }
}
