//! Extended message encode/decode (SPEC.md §4).

use crate::types::*;

/// Extended fixed header size (SPEC.md §4).
const EXT_HEADER: usize = 22;
/// Extended HMAC tag size (SPEC.md §4).
const EXT_HMAC: usize = 32;

/// Encode an extended message into `buf` (SPEC.md §4).
///
/// Returns the number of bytes written.
pub fn encode_extended(msg: &ExtendedMessage<'_>, buf: &mut [u8]) -> Result<usize, WireError> {
    let route_size = match &msg.route {
        Some(r) => {
            if r.len > 8 {
                return Err(WireError::InvalidRouteLen);
            }
            1 + (r.len as usize) * 4
        }
        None => 0,
    };
    let hmac_size = if msg.hmac_tag.is_some() { EXT_HMAC } else { 0 };
    let total = EXT_HEADER + route_size + msg.payload.len() + hmac_size;
    if buf.len() < total {
        return Err(WireError::BufferTooSmall);
    }

    let h = &msg.header;
    buf[0] = encode_byte0(h.version, h.msg_type, h.flags);
    buf[1] = encode_byte1(h.ttl, h.k);
    buf[2..6].copy_from_slice(&h.msg_id.to_be_bytes());
    buf[6..10].copy_from_slice(&h.event_hash.to_be_bytes());
    buf[10..14].copy_from_slice(&h.payload_len.to_be_bytes());
    buf[14..18].copy_from_slice(&h.target_group.to_be_bytes());
    buf[18..22].copy_from_slice(&h.target_hive.to_be_bytes());

    let mut pos = EXT_HEADER;

    if let Some(r) = &msg.route {
        buf[pos] = r.len;
        pos += 1;
        for i in 0..r.len as usize {
            buf[pos..pos + 4].copy_from_slice(&r.entries[i].to_be_bytes());
            pos += 4;
        }
    }

    buf[pos..pos + msg.payload.len()].copy_from_slice(msg.payload);
    pos += msg.payload.len();

    if let Some(tag) = &msg.hmac_tag {
        buf[pos..pos + EXT_HMAC].copy_from_slice(tag);
        pos += EXT_HMAC;
    }

    Ok(pos)
}

/// Decode an extended message from bytes.
/// Decode an extended message from bytes (SPEC.md §4).
///
/// Payload is borrowed from the input slice (zero-copy).
pub fn decode_extended(data: &[u8]) -> Result<ExtendedMessage<'_>, WireError> {
    if data.len() < EXT_HEADER {
        return Err(WireError::TruncatedMessage);
    }

    let (version, msg_type, flags) = decode_byte0(data[0])?;
    let (ttl, k) = decode_byte1(data[1]);

    let msg_id = u32::from_be_bytes([data[2], data[3], data[4], data[5]]);
    let event_hash = u32::from_be_bytes([data[6], data[7], data[8], data[9]]);
    let payload_len = u32::from_be_bytes([data[10], data[11], data[12], data[13]]);
    let target_group = u32::from_be_bytes([data[14], data[15], data[16], data[17]]);
    let target_hive = u32::from_be_bytes([data[18], data[19], data[20], data[21]]);

    let header = ExtendedHeader {
        version,
        msg_type,
        flags,
        ttl,
        k,
        msg_id,
        event_hash,
        payload_len,
        target_group,
        target_hive,
    };

    let mut pos = EXT_HEADER;

    let route = if flags.has_route {
        if pos >= data.len() {
            return Err(WireError::TruncatedMessage);
        }
        let rlen = data[pos] as usize;
        pos += 1;
        if rlen == 0 || rlen > 8 {
            return Err(WireError::InvalidRouteLen);
        }
        if pos + rlen * 4 > data.len() {
            return Err(WireError::TruncatedMessage);
        }
        let mut rs = ExtendedRouteStack::new();
        rs.len = rlen as u8;
        for i in 0..rlen {
            rs.entries[i] =
                u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
            pos += 4;
        }
        Some(rs)
    } else {
        None
    };

    let hmac_tag = if flags.has_hmac {
        if data.len() < pos + EXT_HMAC {
            return Err(WireError::TruncatedMessage);
        }
        let mut tag = [0u8; EXT_HMAC];
        tag.copy_from_slice(&data[data.len() - EXT_HMAC..]);
        Some(tag)
    } else {
        None
    };

    let payload_end = if flags.has_hmac {
        data.len() - EXT_HMAC
    } else {
        data.len()
    };
    let payload = &data[pos..payload_end];

    // R2-WIRE §4.3.2: payload_len is authoritative — reject if mismatch.
    if payload.len() as u32 != payload_len {
        return Err(WireError::PayloadLenMismatch);
    }

    Ok(ExtendedMessage {
        header,
        route,
        payload,
        hmac_tag,
    })
}
