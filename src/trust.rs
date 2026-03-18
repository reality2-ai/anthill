//! Colony trust — device provisioning and authentication.
//!
//! Implements R2-TRUST §4 provisioning for the Anthill colony:
//!   1. Colony root secret (generated once, stored on server)
//!   2. Join codes (short-lived, derived from root)
//!   3. Device credentials (permanent, derived at join time)
//!   4. Authentication (HMAC verification on every connect)

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Compute HMAC-SHA256.
fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key)
        .expect("HMAC key length should be valid");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Maximum age of a signed message (seconds) before it's considered stale.
const MAX_MESSAGE_AGE_SECS: u64 = 60;

/// Sign a message: HMAC-SHA256(credential, device_id + timestamp + payload).
/// Returns (signature_hex, timestamp).
pub fn sign_message(credential: &str, device_id: &str, payload: &str) -> (String, u64) {
    let timestamp = now_secs();
    let data = format!("{}:{}:{}", device_id, timestamp, payload);
    let key = hex::decode(credential).unwrap_or_default();
    let sig = hmac_sha256(&key, data.as_bytes());
    (hex::encode(&sig), timestamp)
}

/// Verify a signed message. Returns true if valid and fresh.
pub fn verify_signature(
    credential: &str,
    device_id: &str,
    timestamp: u64,
    payload: &str,
    signature: &str,
) -> bool {
    // Check freshness.
    let now = now_secs();
    if now.abs_diff(timestamp) > MAX_MESSAGE_AGE_SECS {
        return false;
    }

    let data = format!("{}:{}:{}", device_id, timestamp, payload);
    let key = hex::decode(credential).unwrap_or_default();
    let expected = hmac_sha256(&key, data.as_bytes());
    let expected_hex = hex::encode(&expected);

    // Constant-time comparison.
    if signature.len() != expected_hex.len() {
        return false;
    }
    signature
        .bytes()
        .zip(expected_hex.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Derive an encryption key from a credential (32 bytes for AES-256).
#[allow(dead_code)]
pub fn derive_encryption_key(credential: &str) -> Vec<u8> {
    let key = hex::decode(credential).unwrap_or_default();
    hmac_sha256(&key, b"anthill:encrypt:v1")
}

/// Encrypt a payload with AES-256-GCM.
/// Returns base64(nonce + ciphertext).
#[allow(dead_code)]
pub fn encrypt_payload(credential: &str, plaintext: &[u8]) -> Result<String, String> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    use aes_gcm::aead::Aead;

    let key_bytes = derive_encryption_key(credential);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| format!("key error: {}", e))?;

    // Generate random 12-byte nonce.
    let nonce_bytes = generate_random_bytes(12);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("encrypt error: {}", e))?;

    // Prepend nonce to ciphertext, base64 encode.
    let mut output = nonce_bytes;
    output.extend_from_slice(&ciphertext);
    Ok(base64_encode(&output))
}

/// Decrypt an AES-256-GCM payload.
/// Input is base64(nonce + ciphertext).
#[allow(dead_code)]
pub fn decrypt_payload(credential: &str, encrypted: &str) -> Result<Vec<u8>, String> {
    use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
    use aes_gcm::aead::Aead;

    let key_bytes = derive_encryption_key(credential);
    let cipher = Aes256Gcm::new_from_slice(&key_bytes)
        .map_err(|e| format!("key error: {}", e))?;

    let data = base64_decode(encrypted)?;
    if data.len() < 12 {
        return Err("ciphertext too short".into());
    }

    let nonce = Nonce::from_slice(&data[..12]);
    let ciphertext = &data[12..];

    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt error: {}", e))
}

#[allow(dead_code)]
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
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 63) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 63) as usize] as char);
        } else {
            result.push('=');
        }
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

/// A provisioned device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// Device identifier (random, assigned at join).
    pub id: String,
    /// Human-readable name (set by user after joining).
    pub name: String,
    /// Hex-encoded credential (HMAC key for this device).
    pub credential: String,
    /// When this device joined (unix timestamp).
    pub joined_at: u64,
    /// Last seen (unix timestamp).
    pub last_seen: u64,
}

/// Persistent device registry.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DeviceRegistry {
    pub devices: HashMap<String, Device>,
}

/// Colony trust state.
pub struct ColonyTrust {
    /// Colony root secret (32 bytes).
    root_secret: Vec<u8>,
    /// Provisioned devices.
    devices: DeviceRegistry,
    /// Path to devices file.
    devices_path: PathBuf,
    /// Path to join codes file (shared between CLI and server).
    join_codes_path: PathBuf,
    /// Currently active join codes (code → expiry timestamp).
    join_codes: HashMap<String, u64>,
}

impl ColonyTrust {
    /// Load or initialise colony trust from a config directory.
    pub fn load(config_dir: &Path) -> anyhow::Result<Self> {
        let key_path = config_dir.join("colony.key");
        let devices_path = config_dir.join("devices.toml");

        // Load or generate root secret.
        let root_secret = if key_path.exists() {
            let hex = std::fs::read_to_string(&key_path)?;
            hex::decode(hex.trim())?
        } else {
            let secret = generate_random_bytes(32);
            let hex = hex::encode(&secret);
            std::fs::write(&key_path, &hex)?;
            log::info!("Generated colony root secret at {}", key_path.display());
            secret
        };

        // Load devices.
        let devices = if devices_path.exists() {
            let contents = std::fs::read_to_string(&devices_path)?;
            toml::from_str(&contents).unwrap_or_default()
        } else {
            DeviceRegistry::default()
        };

        let join_codes_path = config_dir.join("join-codes.toml");
        let join_codes = Self::load_join_codes(&join_codes_path);

        Ok(Self {
            root_secret,
            devices,
            devices_path,
            join_codes_path,
            join_codes,
        })
    }

    /// Returns true if no devices have been provisioned yet (queen bootstrap).
    pub fn is_empty_colony(&self) -> bool {
        self.devices.devices.is_empty()
    }

    /// Generate a join code (valid for 5 minutes). Persisted to disk.
    pub fn generate_join_code(&mut self) -> String {
        let now = now_secs();
        let expiry = now + 300; // 5 minutes

        // Derive code from root secret + timestamp + random.
        let random = generate_random_bytes(8);
        let mut data = Vec::new();
        data.extend_from_slice(b"join:");
        data.extend_from_slice(&now.to_le_bytes());
        data.extend_from_slice(&random);

        let mac = hmac_sha256(&self.root_secret, &data);
        let code = format!(
            "{:03x}-{:03x}",
            u16::from_le_bytes([mac[0], mac[1]]) & 0xFFF,
            u16::from_le_bytes([mac[2], mac[3]]) & 0xFFF,
        );

        self.join_codes.insert(code.clone(), expiry);

        // Clean expired codes.
        self.join_codes.retain(|_, exp| *exp > now);

        // Persist to disk so the server process can see codes from the CLI.
        self.save_join_codes();

        code
    }

    /// Verify and consume a join code. Returns true if valid.
    pub fn verify_join_code(&mut self, code: &str) -> bool {
        // Reload from disk (CLI may have written new codes).
        self.join_codes = Self::load_join_codes(&self.join_codes_path);

        let now = now_secs();
        if let Some(&expiry) = self.join_codes.get(code) {
            if expiry > now {
                self.join_codes.remove(code);
                self.save_join_codes();
                return true;
            }
        }
        // Clean expired.
        self.join_codes.retain(|_, exp| *exp > now);
        self.save_join_codes();
        false
    }

    fn load_join_codes(path: &Path) -> HashMap<String, u64> {
        if let Ok(contents) = std::fs::read_to_string(path) {
            let now = now_secs();
            // Simple format: one line per code, "code expiry_timestamp"
            contents
                .lines()
                .filter_map(|line| {
                    let mut parts = line.split_whitespace();
                    let code = parts.next()?.to_string();
                    let expiry: u64 = parts.next()?.parse().ok()?;
                    if expiry > now { Some((code, expiry)) } else { None }
                })
                .collect()
        } else {
            HashMap::new()
        }
    }

    fn save_join_codes(&self) {
        let contents: String = self
            .join_codes
            .iter()
            .map(|(code, expiry)| format!("{} {}", code, expiry))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(&self.join_codes_path, contents);
    }

    /// Provision a new device. Returns the device credential.
    pub fn provision_device(&mut self, name: &str) -> Device {
        let id = hex::encode(&generate_random_bytes(16));
        let credential = hex::encode(&hmac_sha256(
            &self.root_secret,
            format!("device:{}", id).as_bytes(),
        ));

        let device = Device {
            id: id.clone(),
            name: name.to_string(),
            credential,
            joined_at: now_secs(),
            last_seen: now_secs(),
        };

        self.devices.devices.insert(id, device.clone());
        self.save_devices();
        device
    }

    /// Authenticate a device by its credential. Returns the device if valid.
    pub fn authenticate(&mut self, credential: &str) -> Option<Device> {
        for device in self.devices.devices.values_mut() {
            if device.credential == credential {
                device.last_seen = now_secs();
                let d = device.clone();
                self.save_devices();
                return Some(d);
            }
        }
        None
    }

    /// List all provisioned devices.
    pub fn list_devices(&self) -> Vec<&Device> {
        let mut devices: Vec<_> = self.devices.devices.values().collect();
        devices.sort_by_key(|d| d.joined_at);
        devices
    }

    /// Revoke a device.
    pub fn revoke_device(&mut self, device_id: &str) -> bool {
        let removed = self.devices.devices.remove(device_id).is_some();
        if removed {
            self.save_devices();
        }
        removed
    }

    /// Save devices to disk.
    fn save_devices(&self) {
        if let Ok(toml) = toml::to_string_pretty(&self.devices) {
            let _ = std::fs::write(&self.devices_path, toml);
        }
    }
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

fn generate_random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut bytes = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        if f.read_exact(&mut bytes).is_ok() {
            return bytes;
        }
    }
    // Fallback: time + pid mixing.
    let seed = now_secs() ^ (std::process::id() as u64);
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = ((seed.wrapping_mul(6364136223846793005).wrapping_add(i as u64)) >> 33) as u8;
    }
    bytes
}

/// Hex encoding/decoding.
mod hex {
    pub fn encode(data: &[u8]) -> String {
        data.iter().map(|b| format!("{:02x}", b)).collect()
    }

    pub fn decode(s: &str) -> Result<Vec<u8>, anyhow::Error> {
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
}
