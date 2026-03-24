//! Tests for r2-transport.

use crate::format::WireFormat;
use crate::framing;
use crate::tcp;
use crate::transport::{LinkQuality, TransportId};
use crate::udp;

// ── WireFormat ─────────────────────────────────────────────────────

#[test]
fn format_selection_by_transport() {
    assert_eq!(WireFormat::for_transport("ble"), WireFormat::Compact);
    assert_eq!(WireFormat::for_transport("lora"), WireFormat::Compact);
    assert_eq!(WireFormat::for_transport("tcp"), WireFormat::Extended);
    assert_eq!(WireFormat::for_transport("udp"), WireFormat::Extended);
    assert_eq!(WireFormat::for_transport("websocket"), WireFormat::Extended);
    assert_eq!(WireFormat::for_transport("unknown"), WireFormat::Extended);
}

#[test]
fn format_header_sizes() {
    assert_eq!(WireFormat::Compact.header_size(), 12);
    assert_eq!(WireFormat::Extended.header_size(), 22);
}

#[test]
fn format_hmac_tag_sizes() {
    assert_eq!(WireFormat::Compact.hmac_tag_size(), 8);
    assert_eq!(WireFormat::Extended.hmac_tag_size(), 32);
}

// ── TCP framing ────────────────────────────────────────────────────

#[test]
fn tcp_length_prefix_roundtrip() {
    let mut buf = [0u8; 4];
    framing::write_length_prefix(&mut buf, 42).unwrap();
    assert_eq!(buf, [0, 0, 0, 42]);
    assert_eq!(framing::read_length_prefix(&buf).unwrap(), 42);
}

#[test]
fn tcp_length_prefix_large_value() {
    let mut buf = [0u8; 4];
    framing::write_length_prefix(&mut buf, 65535).unwrap();
    assert_eq!(framing::read_length_prefix(&buf).unwrap(), 65535);
}

#[test]
fn tcp_length_prefix_too_large() {
    let buf = [0xFF, 0xFF, 0xFF, 0xFF]; // 4 GB — exceeds MAX_FRAME_SIZE
    assert_eq!(
        framing::read_length_prefix(&buf),
        Err(framing::FrameError::PayloadTooLarge)
    );
}

#[test]
fn tcp_length_prefix_buffer_too_small() {
    let mut buf = [0u8; 2];
    assert_eq!(
        framing::write_length_prefix(&mut buf, 1),
        Err(framing::FrameError::BufferTooSmall)
    );
}

#[test]
fn tcp_length_prefix_incomplete() {
    let buf = [0u8; 3];
    assert_eq!(
        framing::read_length_prefix(&buf),
        Err(framing::FrameError::Incomplete)
    );
}

#[test]
fn tcp_encode_decode_roundtrip() {
    // R2-WIRE test vector 1 (R2-WIRE §14.1): minimal EVENT
    let wire_frame = [
        0x00, 0x53, 0xA1, 0xB2, 0x42, 0x4D, 0x3E, 0x4C, 0x1A, 0x2B, 0x3C, 0x4D, 0xA1, 0x00, 0x18,
        0xEA,
    ];

    // Encode for TCP.
    let mut buf = [0u8; 256];
    let n = tcp::encode_tcp_frame(&wire_frame, &mut buf).unwrap();
    assert_eq!(n, 4 + 16);
    assert_eq!(&buf[..4], &[0, 0, 0, 16]); // length = 16

    // Decode from TCP.
    let (decoded, consumed) = tcp::decode_tcp_frame(&buf[..n]).unwrap().unwrap();
    assert_eq!(decoded, &wire_frame);
    assert_eq!(consumed, 20);
}

#[test]
fn tcp_decode_incomplete_prefix() {
    let buf = [0, 0];
    assert_eq!(tcp::decode_tcp_frame(&buf).unwrap(), None);
}

#[test]
fn tcp_decode_incomplete_payload() {
    let buf = [0, 0, 0, 10, 0x00, 0x53]; // declares 10 bytes but only 2 available
    assert_eq!(tcp::decode_tcp_frame(&buf).unwrap(), None);
}

#[test]
fn tcp_decode_zero_length() {
    let buf = [0, 0, 0, 0]; // zero-length keepalive
    let (frame, consumed) = tcp::decode_tcp_frame(&buf).unwrap().unwrap();
    assert!(frame.is_empty());
    assert_eq!(consumed, 4);
}

// ── BLE framing ────────────────────────────────────────────────────

#[test]
fn ble_length_prefix_roundtrip() {
    let mut buf = [0u8; 2];
    framing::write_ble_length_prefix(&mut buf, 300).unwrap();
    assert_eq!(framing::read_ble_length_prefix(&buf).unwrap(), 300);
}

#[test]
fn ble_length_prefix_little_endian() {
    let mut buf = [0u8; 2];
    framing::write_ble_length_prefix(&mut buf, 0x0102).unwrap();
    assert_eq!(buf, [0x02, 0x01]); // LE: low byte first
}

#[test]
fn ble_frame_extraction() {
    let mut buf = [0u8; 10];
    framing::write_ble_length_prefix(&mut buf, 4).unwrap();
    buf[2..6].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);

    let (start, len) = framing::try_extract_ble_frame(&buf[..6]).unwrap().unwrap();
    assert_eq!(start, 2);
    assert_eq!(len, 4);
    assert_eq!(&buf[start..start + len], &[0x01, 0x02, 0x03, 0x04]);
}

// ── UDP validation ─────────────────────────────────────────────────

#[test]
fn udp_validate_good_extended() {
    // Byte 0: version=0, type=EVENT(0), flags=0 → 0x00
    let data = [0x00; 22];
    assert!(udp::validate_udp_datagram(&data, WireFormat::Extended).is_ok());
}

#[test]
fn udp_validate_good_compact() {
    let data = [0x00; 12];
    assert!(udp::validate_udp_datagram(&data, WireFormat::Compact).is_ok());
}

#[test]
fn udp_validate_too_short() {
    let data = [0x00; 8]; // less than compact header (12)
    assert!(udp::validate_udp_datagram(&data, WireFormat::Compact).is_err());
}

#[test]
fn udp_validate_bad_version() {
    // Version 1 in bits 7:6 → 0b01_000_000 = 0x40
    let mut data = [0x00; 22];
    data[0] = 0x40;
    assert!(udp::validate_udp_datagram(&data, WireFormat::Extended).is_err());
}

#[test]
fn udp_message_type_extraction() {
    // Type = EVENT (0) → bits 5:3 = 000 → byte 0 = 0x00
    assert_eq!(udp::message_type(&[0x00]), Some(0));
    // Type = HEARTBEAT (5) → bits 5:3 = 101 → 0b00_101_000 = 0x28
    assert_eq!(udp::message_type(&[0x28]), Some(5));
    // Type = GROUP_MGMT (4) → bits 5:3 = 100 → 0b00_100_000 = 0x20
    assert_eq!(udp::message_type(&[0x20]), Some(4));
    // Empty
    assert_eq!(udp::message_type(&[]), None);
}

// ── Port constants ─────────────────────────────────────────────────

#[test]
fn port_constants() {
    assert_eq!(tcp::R2_PORT, 21042);
    assert_eq!(tcp::R2_PORT, 0x5232); // "R2" in ASCII
    assert_eq!(tcp::R2_OTA_PORT, 21043);
    assert_eq!(tcp::R2_PRESENCE_PORT, 21044);
    assert_eq!(tcp::R2_CONSOLE_PORT, 21045);
}

// ── R2-WIRE test vectors (R2-WIRE §14) ─────────────────────────────

#[test]
fn wire_test_vector_1_tcp_roundtrip() {
    // Test Vector 1: Minimal EVENT (Compact, no route, no HMAC)
    // This is compact format, but TCP carries it as-is.
    let tv1 = [
        0x00, 0x53, 0xA1, 0xB2, 0x42, 0x4D, 0x3E, 0x4C, 0x1A, 0x2B, 0x3C, 0x4D, 0xA1, 0x00, 0x18,
        0xEA,
    ];

    let mut buf = [0u8; 256];
    let n = tcp::encode_tcp_frame(&tv1, &mut buf).unwrap();
    let (decoded, _) = tcp::decode_tcp_frame(&buf[..n]).unwrap().unwrap();
    assert_eq!(
        decoded, &tv1,
        "R2-WIRE frame must survive TCP encode/decode unchanged"
    );
}

#[test]
fn wire_test_vector_2_tcp_roundtrip() {
    // Test Vector 2: EVENT with route stack and HMAC (Compact)
    let tv2 = [
        0x06, 0x42, 0x00, 0x42, 0xC9, 0x89, 0x50, 0xFB, 0xDE, 0xAD, 0x12, 0x34, 0x01, 0xCA, 0xFE,
        0xA3, 0x00, 0x18, 0xFF, 0x01, 0x00, 0x02, 0x18, 0x80, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
        0x77, 0x88,
    ];

    let mut buf = [0u8; 256];
    let n = tcp::encode_tcp_frame(&tv2, &mut buf).unwrap();
    let (decoded, _) = tcp::decode_tcp_frame(&buf[..n]).unwrap().unwrap();
    assert_eq!(
        decoded, &tv2,
        "R2-WIRE frame must survive TCP encode/decode unchanged"
    );
}

#[test]
fn wire_test_vector_3_udp_validate() {
    // Test Vector 3: MCU-Originated EVENT (Compact, LoRa)
    let tv3 = [
        0x01, 0x21, 0x00, 0x07, 0x49, 0xB4, 0x74, 0x65, 0x00, 0x00, 0x00, 0x00, 0xA1, 0x00, 0x18,
        0xBB,
    ];

    // Compact format validation should pass.
    assert!(udp::validate_udp_datagram(&tv3, WireFormat::Compact).is_ok());

    // MCU flag (bit 0) should be set.
    assert_eq!(tv3[0] & 0x01, 1);

    // Message type should be EVENT (0).
    assert_eq!(udp::message_type(&tv3), Some(0));
}

// ── Transport types ────────────────────────────────────────────────

#[test]
fn transport_id_bitmasks() {
    assert_eq!(TransportId::Ble.bitmask(), 0x01);
    assert_eq!(TransportId::Wifi.bitmask(), 0x02);
    assert_eq!(TransportId::Lora.bitmask(), 0x04);
    assert_eq!(TransportId::Internet.bitmask(), 0x08);
    // All four fit in one byte without overlap.
    let all = TransportId::Ble.bitmask()
        | TransportId::Wifi.bitmask()
        | TransportId::Lora.bitmask()
        | TransportId::Internet.bitmask();
    assert_eq!(all, 0x0F);
}

#[test]
fn transport_id_wire_format() {
    // R2-WIRE §4.3.5: BLE and LoRa → compact; WiFi and Internet → extended.
    assert_eq!(TransportId::Ble.wire_format(), WireFormat::Compact);
    assert_eq!(TransportId::Lora.wire_format(), WireFormat::Compact);
    assert_eq!(TransportId::Wifi.wire_format(), WireFormat::Extended);
    assert_eq!(TransportId::Internet.wire_format(), WireFormat::Extended);
}

#[test]
fn transport_id_power_cost() {
    // R2-ROUTE §5.2 defaults: BLE cheapest, WiFi most expensive.
    assert!(TransportId::Ble.default_power_cost() < TransportId::Lora.default_power_cost());
    assert!(TransportId::Lora.default_power_cost() < TransportId::Wifi.default_power_cost());
}

#[test]
fn transport_id_max_payload() {
    // Constrained transports have small MTU.
    assert!(TransportId::Ble.max_payload() <= 254);
    assert!(TransportId::Lora.max_payload() <= 222);
    // IP transports support large payloads.
    assert!(TransportId::Wifi.max_payload() >= 65535);
    assert!(TransportId::Internet.max_payload() >= 65535);
}

#[test]
fn link_quality_defaults() {
    let lq = LinkQuality::default();
    assert_eq!(lq.quality, 0.0);
    assert_eq!(lq.rssi, 0);
    assert_eq!(lq.snr, 0);
    assert_eq!(lq.latency_ms, 0);
}
