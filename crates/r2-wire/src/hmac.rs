//! HMAC envelope for authenticated messaging (R2-WIRE §10, R2-TRUST §6).
//!
//! The wire protocol authenticates only **immutable** fields — TTL, K,
//! msg_id, and route_stack are mutable (relay nodes change them) and
//! explicitly excluded from the HMAC input.
//!
//! ## Authenticated bytes
//!
//! **Compact:** `type(1) || event_hash(4) || target(4) || payload(N)`
//!
//! **Extended:** `type(1) || event_hash(4) || target_group(4) || target_hive(4) || payload(N)`
//!
//! ## Tag sizes
//!
//! - Compact: 8 bytes (truncated HMAC-SHA256)
//! - Extended: 32 bytes (full HMAC-SHA256)
//!
//! ## Usage
//!
//! The [`HmacProvider`] trait is crypto-agnostic — `r2-wire` defines *what*
//! to authenticate, the caller supplies *how*. `r2-trust` provides the
//! concrete implementation using HKDF-derived keys.

use crate::types::{CompactMessage, ExtendedMessage, Flags};

/// Compact HMAC tag size — truncated to first 8 bytes of HMAC-SHA256.
pub const COMPACT_TAG_LEN: usize = 8;
/// Extended HMAC tag size — full 32-byte HMAC-SHA256 output.
pub const EXTENDED_TAG_LEN: usize = 32;

/// Maximum authenticated-bytes buffer for compact messages.
///
/// 1 (type) + 4 (event_hash) + 4 (target) + 180 (max CBOR compact) = 189.
const COMPACT_AUTH_MAX: usize = 1 + 4 + 4 + 180;

/// Crypto-agnostic HMAC provider (R2-WIRE §10.3).
///
/// Implementors compute HMAC-SHA256 over the authenticated bytes and return
/// the tag. The trait has no dependencies on any crypto crate — `r2-wire`
/// only defines the interface.
///
/// # Constant-time requirement
///
/// [`verify_compact`] and [`verify_extended`] use the provider's output
/// and perform constant-time comparison. Implementations SHOULD also use
/// constant-time MAC finalization internally.
pub trait HmacProvider {
    /// Compute truncated 8-byte HMAC tag for compact frames.
    fn mac_compact(&self, authenticated_bytes: &[u8]) -> [u8; COMPACT_TAG_LEN];

    /// Compute full 32-byte HMAC tag for extended frames.
    fn mac_extended(&self, authenticated_bytes: &[u8]) -> [u8; EXTENDED_TAG_LEN];
}

// ---------------------------------------------------------------------------
// Authenticated bytes extraction
// ---------------------------------------------------------------------------

/// Build the authenticated byte sequence for a compact message (R2-WIRE §10.2).
///
/// Returns the number of bytes written into `buf`.
///
/// Layout: `type(1) || event_hash(4) || target(4) || payload(N)`
pub fn authenticated_bytes_compact(msg: &CompactMessage<'_>, buf: &mut [u8]) -> usize {
    let payload_len = msg.payload.len();
    let total = 1 + 4 + 4 + payload_len;
    debug_assert!(buf.len() >= total);

    buf[0] = msg.header.msg_type as u8;
    buf[1..5].copy_from_slice(&msg.header.event_hash.to_be_bytes());
    buf[5..9].copy_from_slice(&msg.header.target.to_be_bytes());
    buf[9..9 + payload_len].copy_from_slice(msg.payload);
    total
}

/// Build the authenticated byte sequence for an extended message (R2-WIRE §10.2).
///
/// Returns the number of bytes written into `buf`.
///
/// Layout: `type(1) || event_hash(4) || target_group(4) || target_hive(4) || payload(N)`
pub fn authenticated_bytes_extended(msg: &ExtendedMessage<'_>, buf: &mut [u8]) -> usize {
    let payload_len = msg.payload.len();
    let total = 1 + 4 + 4 + 4 + payload_len;
    debug_assert!(buf.len() >= total);

    buf[0] = msg.header.msg_type as u8;
    buf[1..5].copy_from_slice(&msg.header.event_hash.to_be_bytes());
    buf[5..9].copy_from_slice(&msg.header.target_group.to_be_bytes());
    buf[9..13].copy_from_slice(&msg.header.target_hive.to_be_bytes());
    buf[13..13 + payload_len].copy_from_slice(msg.payload);
    total
}

// ---------------------------------------------------------------------------
// Sign (apply HMAC tag to a message)
// ---------------------------------------------------------------------------

/// Compute and attach the HMAC tag to a compact message.
///
/// Returns a new `Flags` with `has_hmac = true` and the 8-byte tag.
/// The caller should set `msg.header.flags = flags` and `msg.hmac_tag = Some(tag)`
/// before encoding, or use the returned pair directly.
pub fn sign_compact(
    msg: &CompactMessage<'_>,
    hmac: &impl HmacProvider,
) -> (Flags, [u8; COMPACT_TAG_LEN]) {
    let mut auth_buf = [0u8; COMPACT_AUTH_MAX];
    let len = authenticated_bytes_compact(msg, &mut auth_buf);
    let tag = hmac.mac_compact(&auth_buf[..len]);
    let flags = Flags {
        has_hmac: true,
        ..msg.header.flags
    };
    (flags, tag)
}

/// Compute and attach the HMAC tag to an extended message.
///
/// Returns a new `Flags` with `has_hmac = true` and the 32-byte tag.
pub fn sign_extended(
    msg: &ExtendedMessage<'_>,
    hmac: &impl HmacProvider,
) -> (Flags, [u8; EXTENDED_TAG_LEN]) {
    // Extended payloads can be up to 2^32. Use a stack buffer for typical sizes,
    // but we need to handle the auth bytes directly for large payloads.
    // For correctness over all sizes, compute in two parts isn't possible with
    // the trait as-is. We'll assemble the auth bytes contiguously.
    //
    // For the initial implementation, use a reasonable stack buffer.
    // The maximum extended payload is 65535 (STANDARD_MAX), so 65548 total.
    let payload_len = msg.payload.len();
    let total = 1 + 4 + 4 + 4 + payload_len;

    // Use a stack array for small messages, assert for large ones.
    // In practice, extended messages on constrained devices are < 4KB.
    let mut auth_buf = [0u8; 1 + 4 + 4 + 4]; // header portion
    auth_buf[0] = msg.header.msg_type as u8;
    auth_buf[1..5].copy_from_slice(&msg.header.event_hash.to_be_bytes());
    auth_buf[5..9].copy_from_slice(&msg.header.target_group.to_be_bytes());
    auth_buf[9..13].copy_from_slice(&msg.header.target_hive.to_be_bytes());

    // For extended, we need to MAC header_auth || payload. Since we can't
    // concatenate without allocation on no_std, we provide the full buffer
    // approach for messages that fit, and document the limitation.
    //
    // Practical limit: stack-allocate up to 4KB. Beyond that, callers should
    // use the streaming HmacProvider variant (future extension).
    const EXT_AUTH_MAX: usize = 13 + 4096;
    debug_assert!(total <= EXT_AUTH_MAX, "extended payload too large for stack HMAC");
    let mut full_buf = [0u8; EXT_AUTH_MAX];
    full_buf[..13].copy_from_slice(&auth_buf);
    full_buf[13..total].copy_from_slice(msg.payload);

    let tag = hmac.mac_extended(&full_buf[..total]);
    let flags = Flags {
        has_hmac: true,
        ..msg.header.flags
    };
    (flags, tag)
}

// ---------------------------------------------------------------------------
// Verify (check HMAC tag on a received message)
// ---------------------------------------------------------------------------

/// Verify the HMAC tag on a compact message.
///
/// Returns `true` if the tag matches (constant-time comparison).
/// Returns `false` if no tag is present or the tag doesn't match.
pub fn verify_compact(msg: &CompactMessage<'_>, hmac: &impl HmacProvider) -> bool {
    let received_tag = match msg.hmac_tag {
        Some(tag) => tag,
        None => return false,
    };

    let mut auth_buf = [0u8; COMPACT_AUTH_MAX];
    let len = authenticated_bytes_compact(msg, &mut auth_buf);
    let expected = hmac.mac_compact(&auth_buf[..len]);

    constant_time_eq(&received_tag, &expected)
}

/// Verify the HMAC tag on an extended message.
///
/// Returns `true` if the tag matches (constant-time comparison).
/// Returns `false` if no tag is present or the tag doesn't match.
pub fn verify_extended(msg: &ExtendedMessage<'_>, hmac: &impl HmacProvider) -> bool {
    let received_tag = match msg.hmac_tag {
        Some(tag) => tag,
        None => return false,
    };

    let payload_len = msg.payload.len();
    let total = 1 + 4 + 4 + 4 + payload_len;
    const EXT_AUTH_MAX: usize = 13 + 4096;
    if total > EXT_AUTH_MAX {
        return false; // Too large for stack verification
    }

    let mut full_buf = [0u8; EXT_AUTH_MAX];
    full_buf[0] = msg.header.msg_type as u8;
    full_buf[1..5].copy_from_slice(&msg.header.event_hash.to_be_bytes());
    full_buf[5..9].copy_from_slice(&msg.header.target_group.to_be_bytes());
    full_buf[9..13].copy_from_slice(&msg.header.target_hive.to_be_bytes());
    full_buf[13..total].copy_from_slice(msg.payload);

    let expected = hmac.mac_extended(&full_buf[..total]);

    constant_time_eq(&received_tag, &expected)
}

// ---------------------------------------------------------------------------
// Frame classification (R2-TRUST §6.3)
// ---------------------------------------------------------------------------

/// Inbound frame classification (R2-TRUST §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameClass {
    /// HMAC verified with trust group key — same trust group.
    SameGroup,
    /// HMAC present but no matching key — relay opaquely.
    Relay,
    /// No HMAC tag (H flag = 0) — unauthenticated.
    Unauthenticated,
}

/// Classify an inbound compact frame (R2-TRUST §6.3).
///
/// - `group_hmac`: the trust group's HMAC provider (if this device is a member).
///
/// Returns `None` if the HMAC is present but **invalid** (frame MUST be dropped).
pub fn classify_compact(
    msg: &CompactMessage<'_>,
    group_hmac: Option<&impl HmacProvider>,
) -> Option<FrameClass> {
    if msg.hmac_tag.is_none() {
        return Some(FrameClass::Unauthenticated);
    }

    // HMAC is present. Try to verify.
    match group_hmac {
        Some(hmac) => {
            if verify_compact(msg, hmac) {
                Some(FrameClass::SameGroup)
            } else {
                None // Invalid HMAC — drop frame
            }
        }
        None => {
            // We have no key for this group — forward opaquely.
            Some(FrameClass::Relay)
        }
    }
}

/// Classify an inbound extended frame (R2-TRUST §6.3).
///
/// Same semantics as [`classify_compact`].
pub fn classify_extended(
    msg: &ExtendedMessage<'_>,
    group_hmac: Option<&impl HmacProvider>,
) -> Option<FrameClass> {
    if msg.hmac_tag.is_none() {
        return Some(FrameClass::Unauthenticated);
    }

    match group_hmac {
        Some(hmac) => {
            if verify_extended(msg, hmac) {
                Some(FrameClass::SameGroup)
            } else {
                None
            }
        }
        None => Some(FrameClass::Relay),
    }
}

// ---------------------------------------------------------------------------
// Constant-time comparison (R2-WIRE §10.6 step 3)
// ---------------------------------------------------------------------------

/// Constant-time byte slice equality (no early exit on mismatch).
#[inline]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}
