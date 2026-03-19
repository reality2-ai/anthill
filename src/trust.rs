//! Colony trust — device provisioning and authentication via R2-TRUST.
//!
//! Wraps `r2_trust::TrustGroup` (Ed25519 certificates, HKDF-derived keys,
//! X25519 join encryption) with filesystem persistence and human-readable
//! join codes.  Replaces the earlier HMAC-based implementation.

use ed25519_dalek::{Signer, SigningKey, Verifier};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use r2_trust::lifecycle::{MemberInfo, TrustGroup, DEFAULT_CERT_TTL_SECS, DEFAULT_JOIN_CODE_TTL_SECS};
use r2_trust::revocation::{RevocationReason, RevocationSet};

/// Maximum age of a signed message (seconds) before it's considered stale.
const MAX_MESSAGE_AGE_SECS: u64 = 60;

/// A provisioned device (external view for JSON serialization).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Device identifier (hex-encoded Ed25519 public key).
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Credential token (hex-encoded Ed25519 private key seed).
    pub credential: String,
    /// When this device joined (unix timestamp).
    pub joined_at: u64,
    /// Last seen (unix timestamp).
    pub last_seen: u64,
}

/// Persistent device record for TOML storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDevice {
    /// Hex-encoded Ed25519 public key.
    public_key: String,
    /// Human-readable name.
    name: String,
    /// Certificate bytes (hex-encoded).
    certificate: String,
    /// When this device joined (unix timestamp).
    joined_at: u64,
    /// Last seen (unix timestamp).
    last_seen: u64,
}

/// Persistent device registry.
#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceRegistry {
    #[serde(default)]
    devices: HashMap<String, StoredDevice>,
}

/// Colony trust state — wraps `r2_trust::TrustGroup`.
pub struct ColonyTrust {
    /// The R2-TRUST lifecycle group (key holder side).
    group: TrustGroup,
    /// Path to devices file.
    devices_path: PathBuf,
    /// Path to join codes file (shared between CLI and server).
    join_codes_path: PathBuf,
    /// Last-seen timestamps (not tracked by r2-trust).
    last_seen: HashMap<String, u64>,
}

impl ColonyTrust {
    /// Load or initialise colony trust from a config directory.
    ///
    /// Follows the load-or-create pattern: if `colony.key` exists, restore
    /// the trust group; otherwise generate a new one.
    pub fn load(config_dir: &Path) -> anyhow::Result<Self> {
        let key_path = config_dir.join("colony.key");
        let devices_path = config_dir.join("devices.toml");
        let join_codes_path = config_dir.join("join-codes.toml");
        let now = now_secs();

        let (group, last_seen) = if key_path.exists() {
            // Load existing signing key.
            let hex = std::fs::read_to_string(&key_path)?;
            let seed_bytes = hex_decode(hex.trim())?;
            if seed_bytes.len() != 32 {
                anyhow::bail!("colony.key must be 32 bytes (got {})", seed_bytes.len());
            }
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&seed_bytes);
            let signing_key = SigningKey::from_bytes(&seed);

            // Load members from devices.toml.
            let (members, revocations, last_seen) = load_members(&devices_path);

            // Build key holder self-cert for restore.
            let self_cert = r2_trust::DeviceCertificate::issue(
                &signing_key,
                *signing_key.verifying_key().as_bytes(),
                *signing_key.verifying_key().as_bytes(),
                r2_trust::DeviceRole::KeyHolder,
                now,
                now + DEFAULT_CERT_TTL_SECS,
            );

            let group = TrustGroup::restore(
                signing_key,
                self_cert,
                members,
                revocations,
                0,
                r2_trust::MinCryptoLevel::Classical,
            ).map_err(|e| anyhow::anyhow!("restore trust group: {:?}", e))?;

            (group, last_seen)
        } else {
            // Generate new trust group.
            let mut rng = OsRng;
            let group = TrustGroup::create(&mut rng, now)
                .map_err(|e| anyhow::anyhow!("create trust group: {:?}", e))?;

            // Persist the signing key.
            std::fs::create_dir_all(config_dir)?;
            let hex = hex_encode(group.signing_key().to_bytes().as_ref());
            std::fs::write(&key_path, &hex)?;
            log::info!("Generated colony trust group key at {}", key_path.display());

            (group, HashMap::new())
        };

        let mut trust = Self {
            group,
            devices_path,
            join_codes_path,
            last_seen,
        };
        // Load any pending join codes from disk (CLI may have written them).
        trust.load_join_codes();
        Ok(trust)
    }

    /// Returns true if no devices have been provisioned yet.
    pub fn is_empty_colony(&self) -> bool {
        self.group.is_empty()
    }

    /// Generate a join code (valid for 5 minutes).
    ///
    /// Returns a short human-readable hex string (e.g. "a1b2-c3d4-e5f6").
    /// Persists to disk so the running server can see CLI-generated codes.
    pub fn generate_join_code(&mut self) -> String {
        let mut rng = OsRng;
        let now = now_secs();

        // Generate a short code: 6 random bytes + 10 zero bytes.
        // This gives 48 bits of entropy — plenty for a 5-minute single-use code.
        let mut value = [0u8; 16];
        rng.fill_bytes(&mut value[..6]);
        let code = r2_trust::join::JoinCode::from_raw(value, now + DEFAULT_JOIN_CODE_TTL_SECS);
        let formatted = format_join_code(code.value());
        self.group.inject_join_code(code);
        self.save_join_codes();
        formatted
    }

    /// Verify a join code is valid (without consuming it).
    #[allow(dead_code)]
    pub fn verify_join_code(&mut self, code: &str) -> bool {
        // Reload from disk in case CLI wrote new codes.
        self.load_join_codes();
        let raw = match parse_join_code(code) {
            Some(r) => r,
            None => return false,
        };
        let now = now_secs();
        self.group.validate_join_code(&raw, now)
    }

    /// Join the colony using a user-provided join code.
    /// Validates and consumes the code, provisions a device, returns credentials.
    pub fn join_with_code(&mut self, code: &str, name: &str) -> Option<Device> {
        // Reload from disk in case CLI wrote the code.
        self.load_join_codes();
        let raw = parse_join_code(code)?;
        let mut rng = OsRng;
        let device_key = SigningKey::generate(&mut rng);
        let device_pub = device_key.verifying_key();
        let id = hex_encode(device_pub.as_bytes());
        let credential = hex_encode(device_key.to_bytes().as_ref());
        let now = now_secs();

        // Validate and consume the join code via process_join_request.
        match self.group.process_join_request(
            &mut rng,
            now,
            &raw,
            &device_pub,
            String::from(name),
            DEFAULT_CERT_TTL_SECS,
        ) {
            Ok(_) => {
                self.last_seen.insert(id.clone(), now);
                self.save_devices();
                self.save_join_codes(); // Remove consumed code from disk.
                Some(Device {
                    id,
                    name: name.to_string(),
                    credential,
                    joined_at: now,
                    last_seen: now,
                })
            }
            Err(e) => {
                log::warn!("Join failed: {:?}", e);
                None
            }
        }
    }

    /// Provision a new device directly (no join code required).
    /// Used internally for bootstrap / programmatic provisioning.
    #[allow(dead_code)]
    pub fn provision_device(&mut self, name: &str) -> Device {
        let mut rng = OsRng;
        let device_key = SigningKey::generate(&mut rng);
        let device_pub = device_key.verifying_key();
        let id = hex_encode(device_pub.as_bytes());
        let credential = hex_encode(device_key.to_bytes().as_ref());
        let now = now_secs();

        // Generate an internal join code and consume it via process_join_request.
        let code = self.group.generate_join_code(&mut rng, now, 60);
        let code_value = *code.value();

        let _ = self.group.process_join_request(
            &mut rng,
            now,
            &code_value,
            &device_pub,
            String::from(name),
            DEFAULT_CERT_TTL_SECS,
        );

        self.last_seen.insert(id.clone(), now);
        self.save_devices();

        Device {
            id,
            name: name.to_string(),
            credential,
            joined_at: now,
            last_seen: now,
        }
    }

    /// Authenticate a device by its credential (hex-encoded Ed25519 seed).
    ///
    /// Derives the public key from the seed and looks up the member.
    pub fn authenticate(&mut self, credential: &str) -> Option<Device> {
        let credential = credential.trim();
        if credential.len() != 64 {
            return None;
        }

        let bytes = hex_decode(credential).ok()?;
        if bytes.len() != 32 {
            return None;
        }

        let mut seed = [0u8; 32];
        seed.copy_from_slice(&bytes);

        // Try as private key seed — derive public key.
        let signing_key = SigningKey::from_bytes(&seed);
        let pub_key = signing_key.verifying_key();
        let pub_hex = hex_encode(pub_key.as_bytes());

        if let Some(member) = self.group.find_member(pub_key.as_bytes()) {
            let now = now_secs();
            self.last_seen.insert(pub_hex.clone(), now);
            return Some(Device {
                id: pub_hex.clone(),
                name: member.name.clone(),
                credential: credential.to_string(),
                joined_at: member.certificate.issued_at,
                last_seen: now,
            });
        }

        // Try as direct public key lookup.
        if let Some(member) = self.group.find_member(&seed) {
            let now = now_secs();
            let id = hex_encode(&seed);
            self.last_seen.insert(id.clone(), now);
            return Some(Device {
                id: id.clone(),
                name: member.name.clone(),
                credential: credential.to_string(),
                joined_at: member.certificate.issued_at,
                last_seen: now,
            });
        }

        None
    }

    /// List all provisioned devices.
    pub fn list_devices(&self) -> Vec<Device> {
        let mut devices: Vec<Device> = self.group.members().iter().map(|m| {
            let id = hex_encode(&m.certificate.device_public_key);
            Device {
                id: id.clone(),
                name: m.name.clone(),
                credential: id.clone(),
                joined_at: m.certificate.issued_at,
                last_seen: self.last_seen.get(&id).copied().unwrap_or(m.certificate.issued_at),
            }
        }).collect();
        devices.sort_by_key(|d| d.joined_at);
        devices
    }

    /// Revoke a device by its hex public key id.
    pub fn revoke_device(&mut self, device_id: &str) -> bool {
        let bytes = match hex_decode(device_id) {
            Ok(b) if b.len() == 32 => b,
            _ => return false,
        };
        let mut pk = [0u8; 32];
        pk.copy_from_slice(&bytes);

        let now = now_secs();
        match self.group.revoke_device(now, &pk, RevocationReason::ForcedRemoval) {
            Ok(_) => {
                self.last_seen.remove(device_id);
                self.save_devices();
                true
            }
            Err(_) => false,
        }
    }

    /// Load join codes from disk (CLI may have written them).
    fn load_join_codes(&mut self) {
        let now = now_secs();
        if let Ok(contents) = std::fs::read_to_string(&self.join_codes_path) {
            for line in contents.lines() {
                let mut parts = line.split_whitespace();
                let hex_code = match parts.next() {
                    Some(h) => h,
                    None => continue,
                };
                let expiry: u64 = match parts.next().and_then(|s| s.parse().ok()) {
                    Some(e) => e,
                    None => continue,
                };
                if expiry <= now { continue; }
                if let Some(raw) = parse_join_code(hex_code) {
                    // Only inject if not already known.
                    if !self.group.validate_join_code(&raw, now) {
                        self.group.inject_join_code(
                            r2_trust::join::JoinCode::from_raw(raw, expiry)
                        );
                    }
                }
            }
        }
    }

    /// Save active join codes to disk (for CLI↔server handoff).
    fn save_join_codes(&self) {
        let now = now_secs();
        let lines: Vec<String> = self.group.join_codes().iter()
            .filter(|c| c.expires_at() > now)
            .map(|c| format!("{} {}", format_join_code(c.value()), c.expires_at()))
            .collect();
        let _ = std::fs::write(&self.join_codes_path, lines.join("\n"));
    }

    /// Persist devices to disk.
    fn save_devices(&self) {
        let mut registry = DeviceRegistry::default();
        for member in self.group.members() {
            let pk_hex = hex_encode(&member.certificate.device_public_key);
            let cert_hex = hex_encode(&member.certificate.to_bytes());
            registry.devices.insert(pk_hex.clone(), StoredDevice {
                public_key: pk_hex.clone(),
                name: member.name.clone(),
                certificate: cert_hex,
                joined_at: member.certificate.issued_at,
                last_seen: self.last_seen.get(&pk_hex).copied()
                    .unwrap_or(member.certificate.issued_at),
            });
        }
        if let Ok(toml) = toml::to_string_pretty(&registry) {
            let _ = std::fs::write(&self.devices_path, toml);
        }
    }
}

/// Sign a message with Ed25519: sign(signing_key, device_id + ":" + timestamp + ":" + payload).
/// Returns (signature_hex, timestamp).
pub fn sign_message(credential: &str, device_id: &str, payload: &str) -> (String, u64) {
    let timestamp = now_secs();
    let data = format!("{}:{}:{}", device_id, timestamp, payload);

    if let Ok(bytes) = hex_decode(credential) {
        if bytes.len() == 32 {
            let mut seed = [0u8; 32];
            seed.copy_from_slice(&bytes);
            let key = SigningKey::from_bytes(&seed);
            let sig = key.sign(data.as_bytes());
            return (hex_encode(&sig.to_bytes()), timestamp);
        }
    }

    (String::new(), timestamp)
}

/// Verify a signed message. Returns true if valid and fresh.
pub fn verify_signature(
    credential: &str,
    device_id: &str,
    timestamp: u64,
    payload: &str,
    signature: &str,
) -> bool {
    let now = now_secs();
    if now.abs_diff(timestamp) > MAX_MESSAGE_AGE_SECS {
        return false;
    }

    let data = format!("{}:{}:{}", device_id, timestamp, payload);

    let bytes = match hex_decode(credential) {
        Ok(b) if b.len() == 32 => b,
        _ => return false,
    };
    let mut seed = [0u8; 32];
    seed.copy_from_slice(&bytes);
    let key = SigningKey::from_bytes(&seed);
    let pub_key = key.verifying_key();

    let sig_bytes = match hex_decode(signature) {
        Ok(b) if b.len() == 64 => b,
        _ => return false,
    };
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(&sig_bytes);
    let sig = ed25519_dalek::Signature::from_bytes(&sig_arr);

    pub_key.verify(data.as_bytes(), &sig).is_ok()
}

/// Encrypt a payload using XChaCha20-Poly1305 with a key derived from the credential.
/// Returns base64(nonce + ciphertext).
pub fn encrypt_payload(credential: &str, plaintext: &[u8]) -> Result<String, String> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    use sha2::{Sha256, Digest};

    let key_bytes = Sha256::digest(credential.as_bytes());
    let cipher = XChaCha20Poly1305::new_from_slice(&key_bytes)
        .map_err(|e| format!("key error: {}", e))?;

    let mut nonce_bytes = [0u8; 24];
    rand::RngCore::fill_bytes(&mut OsRng, &mut nonce_bytes);
    let nonce = XNonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encrypt error: {}", e))?;

    let mut output = nonce_bytes.to_vec();
    output.extend_from_slice(&ciphertext);
    Ok(base64_encode(&output))
}

/// Decrypt a XChaCha20-Poly1305 payload.
/// Input is base64(nonce + ciphertext).
#[allow(dead_code)]
pub fn decrypt_payload(credential: &str, encrypted: &str) -> Result<Vec<u8>, String> {
    use chacha20poly1305::{aead::Aead, KeyInit, XChaCha20Poly1305, XNonce};
    use sha2::{Sha256, Digest};

    let key_bytes = Sha256::digest(credential.as_bytes());
    let cipher = XChaCha20Poly1305::new_from_slice(&key_bytes)
        .map_err(|e| format!("key error: {}", e))?;

    let data = base64_decode(encrypted)?;
    if data.len() < 24 {
        return Err("ciphertext too short".into());
    }

    let nonce = XNonce::from_slice(&data[..24]);
    let ciphertext = &data[24..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt error: {}", e))
}

/// Thread-safe wrapper.
pub type SharedTrust = Arc<Mutex<ColonyTrust>>;

pub fn load_colony_trust(config_dir: &Path) -> anyhow::Result<SharedTrust> {
    Ok(Arc::new(Mutex::new(ColonyTrust::load(config_dir)?)))
}

// --- Utilities ---

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Format 16-byte join code as short human-readable "xxxx-xxxx-xxxx".
/// Only shows the first 6 bytes (48 bits) — plenty for a 5-minute single-use code.
fn format_join_code(raw: &[u8; 16]) -> String {
    let hex = hex_encode(&raw[..6]);
    format!("{}-{}-{}", &hex[0..4], &hex[4..8], &hex[8..12])
}

/// Parse human-readable join code back to 16 bytes.
/// Supports both short (xxxx-xxxx-xxxx) and full (xxxx-xxxx-xxxx-xxxx) formats.
fn parse_join_code(code: &str) -> Option<[u8; 16]> {
    let stripped: String = code.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    let bytes = hex_decode(&stripped).ok()?;
    let mut out = [0u8; 16];
    if bytes.len() == 6 {
        // Short format — pad remaining bytes with zeros.
        out[..6].copy_from_slice(&bytes);
    } else if bytes.len() == 16 {
        // Full format (backwards compat).
        out.copy_from_slice(&bytes);
    } else {
        return None;
    }
    Some(out)
}

/// Load members from devices.toml.
fn load_members(path: &Path) -> (Vec<MemberInfo>, RevocationSet, HashMap<String, u64>) {
    let mut members = Vec::new();
    let mut last_seen = HashMap::new();

    if let Ok(contents) = std::fs::read_to_string(path) {
        if let Ok(registry) = toml::from_str::<DeviceRegistry>(&contents) {
            for stored in registry.devices.values() {
                if let Ok(cert_bytes) = hex_decode(&stored.certificate) {
                    if let Ok(cert) = r2_trust::DeviceCertificate::from_bytes(&cert_bytes) {
                        members.push(MemberInfo {
                            certificate: cert,
                            name: stored.name.clone(),
                        });
                        last_seen.insert(stored.public_key.clone(), stored.last_seen);
                    }
                }
            }
        }
    }

    (members, RevocationSet::new(), last_seen)
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, anyhow::Error> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        anyhow::bail!("odd hex length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| anyhow::anyhow!("bad hex: {}", e))
        })
        .collect()
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 63) as usize] as char);
        result.push(CHARS[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 { result.push(CHARS[((n >> 6) & 63) as usize] as char); }
        else { result.push('='); }
        if chunk.len() > 2 { result.push(CHARS[(n & 63) as usize] as char); }
        else { result.push('='); }
    }
    result
}

#[allow(dead_code)]
fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=');
    let mut result = Vec::new();
    let chars: Vec<u8> = s.bytes().map(|b| match b {
        b'A'..=b'Z' => b - b'A',
        b'a'..=b'z' => b - b'a' + 26,
        b'0'..=b'9' => b - b'0' + 52,
        b'+' => 62,
        b'/' => 63,
        _ => 255,
    }).collect();

    for chunk in chars.chunks(4) {
        let len = chunk.len();
        if chunk.iter().any(|&b| b == 255) {
            return Err("invalid base64".into());
        }
        let n = (chunk[0] as u32) << 18
            | (if len > 1 { chunk[1] as u32 } else { 0 }) << 12
            | (if len > 2 { chunk[2] as u32 } else { 0 }) << 6
            | (if len > 3 { chunk[3] as u32 } else { 0 });
        result.push((n >> 16) as u8);
        if len > 2 { result.push((n >> 8) as u8); }
        if len > 3 { result.push(n as u8); }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_code_format_roundtrip() {
        // Short format: first 6 bytes displayed, rest zeroed.
        let mut raw = [0u8; 16];
        raw[..6].copy_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab]);
        let formatted = format_join_code(&raw);
        assert_eq!(formatted, "0123-4567-89ab");
        let parsed = parse_join_code(&formatted).expect("parse");
        assert_eq!(parsed, raw);
    }

    #[test]
    fn join_code_full_format_compat() {
        // Full 16-byte format still parses (backwards compat).
        let raw = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
                    0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10];
        let full = "01234567-89abcdef-fedcba98-76543210";
        let parsed = parse_join_code(full).expect("parse full");
        assert_eq!(parsed, raw);
    }

    #[test]
    fn sign_and_verify_roundtrip() {
        let key = SigningKey::generate(&mut OsRng);
        let credential = hex_encode(key.to_bytes().as_ref());
        let (signature, timestamp) = sign_message(&credential, "dev", "hello");
        assert!(verify_signature(&credential, "dev", timestamp, "hello", &signature));
    }

    #[test]
    fn verify_rejects_wrong_credential() {
        let key1 = SigningKey::generate(&mut OsRng);
        let key2 = SigningKey::generate(&mut OsRng);
        let cred1 = hex_encode(key1.to_bytes().as_ref());
        let cred2 = hex_encode(key2.to_bytes().as_ref());
        let (signature, timestamp) = sign_message(&cred1, "dev1", "msg");
        assert!(!verify_signature(&cred2, "dev1", timestamp, "msg", &signature));
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let cred = hex_encode(&[0x42u8; 32]);
        let plaintext = b"secret data for testing";
        let encrypted = encrypt_payload(&cred, plaintext).unwrap();
        let decrypted = decrypt_payload(&cred, &encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn colony_trust_lifecycle() {
        let dir = std::env::temp_dir().join("anthill-test-trust-r2");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut trust = ColonyTrust::load(&dir).unwrap();
        assert!(trust.is_empty_colony());

        let device = trust.provision_device("My Laptop");
        assert!(!trust.is_empty_colony());
        assert_eq!(device.name, "My Laptop");

        let authed = trust.authenticate(&device.credential);
        assert!(authed.is_some());
        assert_eq!(authed.unwrap().name, "My Laptop");

        assert!(trust.authenticate("deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef").is_none());

        let code = trust.generate_join_code();
        assert!(trust.verify_join_code(&code));

        assert!(trust.revoke_device(&device.id));
        assert!(trust.authenticate(&device.credential).is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn colony_trust_persists_across_loads() {
        let dir = std::env::temp_dir().join("anthill-test-trust-r2-persist");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let device_cred;
        {
            let mut trust = ColonyTrust::load(&dir).unwrap();
            let device = trust.provision_device("Persistent Device");
            device_cred = device.credential.clone();
        }

        let mut trust2 = ColonyTrust::load(&dir).unwrap();
        assert!(!trust2.is_empty_colony());
        let authed = trust2.authenticate(&device_cred);
        assert!(authed.is_some());

        std::fs::remove_dir_all(&dir).ok();
    }
}
