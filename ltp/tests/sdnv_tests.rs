//! Exhaustive Unit & Property-Based Tests for LTP SDNV Codec (RFC 5326 §2, item 20).

use hardy_ltp::sdnv::{Error, MAX_LEN, decode, encode, encode_to_array, encoded_len};
use proptest::prelude::*;

#[test]
fn test_boundary_values_deterministic() {
    let test_cases: &[(u64, &[u8])] = &[
        (0, &[0x00]),
        (1, &[0x01]),
        (127, &[0x7F]),
        (128, &[0x81, 0x00]),
        (129, &[0x81, 0x01]),
        (255, &[0x81, 0x7F]),
        (256, &[0x82, 0x00]),
        (16_383, &[0xFF, 0x7F]),
        (16_384, &[0x81, 0x80, 0x00]),
        ((1 << 21) - 1, &[0xFF, 0xFF, 0x7F]),
        (1 << 21, &[0x81, 0x80, 0x80, 0x00]),
        ((1 << 28) - 1, &[0xFF, 0xFF, 0xFF, 0x7F]),
        (1 << 28, &[0x81, 0x80, 0x80, 0x80, 0x00]),
        ((1 << 35) - 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0x7F]),
        (1 << 35, &[0x81, 0x80, 0x80, 0x80, 0x80, 0x00]),
        ((1 << 42) - 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]),
        (1 << 42, &[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]),
        ((1 << 49) - 1, &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F]),
        (1 << 49, &[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00]),
        (
            (1 << 56) - 1,
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F],
        ),
        (
            1 << 56,
            &[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00],
        ),
        (
            (1 << 63) - 1,
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F],
        ),
        (
            1 << 63,
            &[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00],
        ),
        (
            u64::MAX - 1,
            &[0x81, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7E],
        ),
        (
            u64::MAX,
            &[0x81, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F],
        ),
    ];

    for &(val, expected_bytes) in test_cases {
        let mut buf = Vec::new();
        encode(val, &mut buf);
        assert_eq!(buf, expected_bytes, "Encoding failed for value {val}");
        assert_eq!(
            encoded_len(val),
            expected_bytes.len(),
            "encoded_len mismatch for {val}"
        );

        let (arr, arr_len) = encode_to_array(val);
        assert_eq!(
            &arr[..arr_len],
            expected_bytes,
            "encode_to_array mismatch for {val}"
        );

        let (decoded, consumed) = decode(&buf).expect("Decode failed for valid encoding");
        assert_eq!(decoded, val, "Decoded value mismatch for {val}");
        assert_eq!(
            consumed,
            expected_bytes.len(),
            "Consumed bytes mismatch for {val}"
        );
    }
}

#[test]
fn test_sdnv_overflow_checks() {
    // 10-octet encoding with bit 1 set in first byte -> represents 2^64, which exceeds u64::MAX
    let overflow_10_bytes = [0x82, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00];
    assert_eq!(decode(&overflow_10_bytes), Err(Error::Overflow));

    // 10-octet encoding with all continuation bits set and value 2^70 - 1
    let overflow_10_all_ff = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F];
    assert_eq!(decode(&overflow_10_all_ff), Err(Error::Overflow));

    // 11-octet encoding (exceeds MAX_LEN = 10)
    let eleven_bytes = [
        0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
    ];
    assert_eq!(decode(&eleven_bytes), Err(Error::Overflow));

    // 11-octet with all continuation bits
    let eleven_continuations = [0x80; 11];
    assert_eq!(decode(&eleven_continuations), Err(Error::Overflow));

    // 12-octet sequence
    let twelve_bytes = [0x80; 12];
    assert_eq!(decode(&twelve_bytes), Err(Error::Overflow));
}

#[test]
fn test_sdnv_truncated_inputs() {
    // Empty buffer
    assert_eq!(decode(&[]), Err(Error::Truncated));

    // Single continuation byte
    assert_eq!(decode(&[0x80]), Err(Error::Truncated));
    assert_eq!(decode(&[0x81]), Err(Error::Truncated));
    assert_eq!(decode(&[0xFF]), Err(Error::Truncated));

    // Multi-byte continuation prefixes without terminal octet (len < 10)
    assert_eq!(decode(&[0x81, 0x80]), Err(Error::Truncated));
    assert_eq!(decode(&[0x81, 0x80, 0x80]), Err(Error::Truncated));
    assert_eq!(
        decode(&[0x81, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80]),
        Err(Error::Truncated)
    );
}

#[test]
fn test_sdnv_canonical_and_zero_padding() {
    // Non-canonical zero padding within MAX_LEN (10 octets)
    // 2 octets for 0: [0x80, 0x00]
    assert_eq!(decode(&[0x80, 0x00]), Ok((0, 2)));

    // 3 octets for 42: [0x80, 0x80, 0x2A]
    assert_eq!(decode(&[0x80, 0x80, 0x2A]), Ok((42, 3)));

    // Exactly 10 octets zero-padded for value 5:
    let mut padded_10 = [0x80; 10];
    padded_10[9] = 0x05;
    assert_eq!(decode(&padded_10), Ok((5, 10)));

    // 11 octets zero-padded (exceeds MAX_LEN) -> must be rejected as Overflow
    let mut padded_11 = [0x80; 11];
    padded_11[10] = 0x05;
    assert_eq!(decode(&padded_11), Err(Error::Overflow));
}

proptest! {
    /// Property: Any u64 value roundtrips correctly via encode and decode.
    #[test]
    fn prop_sdnv_roundtrip(v in any::<u64>()) {
        let mut buf = Vec::new();
        encode(v, &mut buf);

        prop_assert_eq!(buf.len(), encoded_len(v));
        prop_assert!(buf.len() <= MAX_LEN);

        let (decoded, consumed) = decode(&buf).expect("valid encoding must decode");
        prop_assert_eq!(decoded, v);
        prop_assert_eq!(consumed, buf.len());

        let (arr, arr_len) = encode_to_array(v);
        prop_assert_eq!(&arr[..arr_len], &buf[..]);
    }

    /// Property: Any truncated prefix of a valid multi-byte encoding returns Err(Truncated).
    #[test]
    fn prop_sdnv_truncated_prefixes(v in 128u64..=u64::MAX) {
        let mut buf = Vec::new();
        encode(v, &mut buf);
        prop_assert!(buf.len() >= 2);

        for len in 1..buf.len() {
            prop_assert_eq!(decode(&buf[..len]), Err(Error::Truncated));
        }
    }

    /// Property: Arbitrary byte streams never panic when passed to decode.
    #[test]
    fn prop_sdnv_no_panic_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..64)) {
        let _ = decode(&bytes);
    }

    /// Property: Valid SDNV followed by arbitrary trailing garbage consumes only the SDNV octets.
    #[test]
    fn prop_sdnv_trailing_garbage(v in any::<u64>(), garbage in proptest::collection::vec(any::<u8>(), 0..32)) {
        let mut buf = Vec::new();
        encode(v, &mut buf);
        let expected_len = buf.len();
        buf.extend_from_slice(&garbage);

        let (decoded, consumed) = decode(&buf).expect("must decode valid prefix");
        prop_assert_eq!(decoded, v);
        prop_assert_eq!(consumed, expected_len);
    }
}
