use alloc::vec::Vec;

use chacha20poly1305::{aead::Aead, Key, KeyInit, XChaCha20Poly1305, XNonce};
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::montgomery::MontgomeryPoint;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rand_core::{CryptoRng, RngCore};
use subtle::ConstantTimeEq;
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};

use crate::cert::DeviceCertificate;
use crate::error::{Error, Result};
use crate::hkdf::hkdf_expand;
use crate::types::{
    KemAlgo, MinCryptoLevel, DEVICE_CERT_LEN, JOIN_CODE_LEN, JOIN_NONCE_LEN,
    JOIN_RESPONSE_BUNDLE_LEN, JOIN_RESPONSE_NONCE_LEN, KEY_LEN,
};

/// Join code used during provisioning (R2-TRUST §5.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinCode {
    value: [u8; JOIN_CODE_LEN],
    expires_at: u64,
    used: bool,
}

impl JoinCode {
    /// Generate a new join code with a given expiration time.
    pub fn generate<R: RngCore + CryptoRng>(rng: &mut R, expires_at: u64) -> Self {
        let mut value = [0u8; JOIN_CODE_LEN];
        rng.fill_bytes(&mut value);
        JoinCode {
            value,
            expires_at,
            used: false,
        }
    }

    /// Restore a join code from persisted state.
    pub fn from_raw(value: [u8; JOIN_CODE_LEN], expires_at: u64) -> Self {
        JoinCode {
            value,
            expires_at,
            used: false,
        }
    }

    /// Validate a candidate join code.
    pub fn validate(&self, candidate: &[u8; JOIN_CODE_LEN], now: u64) -> Result<()> {
        if now >= self.expires_at {
            return Err(Error::JoinCodeExpired);
        }
        if self.used {
            return Err(Error::InvalidJoinCode);
        }
        if self.value.ct_eq(candidate).unwrap_u8() == 1 {
            Ok(())
        } else {
            Err(Error::InvalidJoinCode)
        }
    }

    /// Mark the join code as used.
    pub fn mark_used(&mut self) {
        self.used = true;
    }

    /// Access the raw value.
    pub fn value(&self) -> &[u8; JOIN_CODE_LEN] {
        &self.value
    }

    /// Expiration timestamp.
    pub fn expires_at(&self) -> u64 {
        self.expires_at
    }
}

/// Join request payload (kem_algo + join code + anti-replay nonce).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequestPayload {
    /// Key encapsulation mechanism to use.
    pub kem_algo: KemAlgo,
    /// The join code (must match key holder's code).
    pub join_code: [u8; JOIN_CODE_LEN],
    /// Anti-replay nonce from the joining device.
    pub nonce: [u8; JOIN_NONCE_LEN],
}

impl JoinRequestPayload {
    /// Create a new join request with classical KEM.
    pub fn new(join_code: [u8; JOIN_CODE_LEN], nonce: [u8; JOIN_NONCE_LEN]) -> Self {
        JoinRequestPayload {
            kem_algo: KemAlgo::Classical,
            join_code,
            nonce,
        }
    }

    /// Serialize to wire format.
    pub fn encode(&self) -> [u8; 1 + JOIN_CODE_LEN + JOIN_NONCE_LEN] {
        let mut out = [0u8; 1 + JOIN_CODE_LEN + JOIN_NONCE_LEN];
        out[0] = self.kem_algo.into();
        out[1..1 + JOIN_CODE_LEN].copy_from_slice(&self.join_code);
        out[1 + JOIN_CODE_LEN..].copy_from_slice(&self.nonce);
        out
    }

    /// Parse from wire format.
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 1 + JOIN_CODE_LEN + JOIN_NONCE_LEN {
            return Err(Error::PayloadTooShort);
        }
        let kem_algo = KemAlgo::try_from(bytes[0])?;
        let mut join_code = [0u8; JOIN_CODE_LEN];
        join_code.copy_from_slice(&bytes[1..1 + JOIN_CODE_LEN]);
        let mut nonce = [0u8; JOIN_NONCE_LEN];
        nonce.copy_from_slice(&bytes[1 + JOIN_CODE_LEN..]);
        Ok(JoinRequestPayload {
            kem_algo,
            join_code,
            nonce,
        })
    }
}

/// Bundle delivered in a join response (certificate + DEK + HK + min_crypto_level).
///
/// Per R2-TRUST §5.2, the encrypted join response includes the trust group's
/// minimum cryptographic level so the joining device can enforce it during
/// entanglement negotiation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinResponseBundle {
    /// The issued device certificate.
    pub certificate: DeviceCertificate,
    /// Data encryption key for the trust group.
    pub dek: [u8; 32],
    /// HMAC key for the trust group.
    pub hk: [u8; 32],
    /// Minimum cryptographic level required by this trust group.
    pub min_crypto_level: MinCryptoLevel,
}

impl JoinResponseBundle {
    /// Create a new bundle.
    pub fn new(
        certificate: DeviceCertificate,
        dek: [u8; 32],
        hk: [u8; 32],
        min_crypto_level: MinCryptoLevel,
    ) -> Self {
        JoinResponseBundle {
            certificate,
            dek,
            hk,
            min_crypto_level,
        }
    }

    /// Serialize to wire format.
    pub fn to_bytes(&self) -> [u8; JOIN_RESPONSE_BUNDLE_LEN] {
        let mut out = [0u8; JOIN_RESPONSE_BUNDLE_LEN];
        out[..DEVICE_CERT_LEN].copy_from_slice(&self.certificate.to_bytes());
        out[DEVICE_CERT_LEN..DEVICE_CERT_LEN + 32].copy_from_slice(&self.dek);
        out[DEVICE_CERT_LEN + 32..DEVICE_CERT_LEN + 64].copy_from_slice(&self.hk);
        out[DEVICE_CERT_LEN + 64] = self.min_crypto_level.into();
        out
    }

    /// Parse from wire format.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != JOIN_RESPONSE_BUNDLE_LEN {
            return Err(Error::PayloadTooShort);
        }
        let certificate = DeviceCertificate::from_bytes(&bytes[..DEVICE_CERT_LEN])?;
        let mut dek = [0u8; 32];
        dek.copy_from_slice(&bytes[DEVICE_CERT_LEN..DEVICE_CERT_LEN + 32]);
        let mut hk = [0u8; 32];
        hk.copy_from_slice(&bytes[DEVICE_CERT_LEN + 32..DEVICE_CERT_LEN + 64]);
        let min_crypto_level = MinCryptoLevel::try_from(bytes[DEVICE_CERT_LEN + 64])?;
        Ok(JoinResponseBundle {
            certificate,
            dek,
            hk,
            min_crypto_level,
        })
    }
}

/// Encrypted join response container.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncryptedJoinResponse {
    /// XChaCha20-Poly1305 nonce (24 bytes).
    pub nonce: [u8; JOIN_RESPONSE_NONCE_LEN],
    /// Encrypted bundle (certificate + DEK + HK) with authentication tag.
    pub ciphertext: Vec<u8>,
}

/// Encrypt the join response bundle for a joining device.
pub fn encrypt_join_response<R: RngCore + CryptoRng>(
    rng: &mut R,
    trust_group_key: &SigningKey,
    device_public: &VerifyingKey,
    bundle: &JoinResponseBundle,
) -> Result<EncryptedJoinResponse> {
    let shared = derive_shared_secret(trust_group_key, device_public)?;
    let key = derive_join_key(shared.as_bytes(), trust_group_key, device_public)?;
    let cipher = XChaCha20Poly1305::new(&key);

    let mut nonce = [0u8; JOIN_RESPONSE_NONCE_LEN];
    rng.fill_bytes(&mut nonce);
    let plaintext = bundle.to_bytes();
    let ciphertext = cipher
        .encrypt(XNonce::from_slice(&nonce), plaintext.as_ref())
        .map_err(|_| Error::Encryption)?;

    Ok(EncryptedJoinResponse { nonce, ciphertext })
}

/// Decrypt a join response bundle using the joining device's key.
pub fn decrypt_join_response(
    device_secret: &SigningKey,
    trust_group_public: &VerifyingKey,
    encrypted: &EncryptedJoinResponse,
) -> Result<JoinResponseBundle> {
    let shared = derive_shared_secret_device(device_secret, trust_group_public)?;
    let key = derive_join_key(shared.as_bytes(), trust_group_public, device_secret)?;
    let cipher = XChaCha20Poly1305::new(&key);
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&encrypted.nonce),
            encrypted.ciphertext.as_ref(),
        )
        .map_err(|_| Error::Decryption)?;
    JoinResponseBundle::from_bytes(&plaintext)
}

fn derive_join_key(
    shared_secret: &[u8; 32],
    tg_key: &impl PublicKeyBytes,
    device_key: &impl PublicKeyBytes,
) -> Result<Key> {
    let tg_bytes = tg_key.public_bytes();
    let dev_bytes = device_key.public_bytes();
    let mut salt = Vec::with_capacity(KEY_LEN * 2);
    salt.extend_from_slice(&tg_bytes);
    salt.extend_from_slice(&dev_bytes);
    let okm = hkdf_expand(shared_secret, &salt, b"R2-TRUST-v0.1-JOIN")?;
    Ok(*Key::from_slice(&okm))
}

fn derive_shared_secret(
    trust_group_key: &SigningKey,
    device_public: &VerifyingKey,
) -> Result<SharedSecret> {
    let tg_secret = ed25519_secret_to_x25519(trust_group_key);
    let device_public = ed25519_public_to_x25519(device_public)?;
    Ok(tg_secret.diffie_hellman(&device_public))
}

fn derive_shared_secret_device(
    device_secret: &SigningKey,
    trust_group_public: &VerifyingKey,
) -> Result<SharedSecret> {
    let device_secret = ed25519_secret_to_x25519(device_secret);
    let trust_group_public = ed25519_public_to_x25519(trust_group_public)?;
    Ok(device_secret.diffie_hellman(&trust_group_public))
}

fn ed25519_secret_to_x25519(secret: &SigningKey) -> StaticSecret {
    use sha2::Digest;

    let hash = sha2::Sha512::digest(secret.to_bytes());
    let mut clamped = [0u8; 32];
    clamped.copy_from_slice(&hash[..32]);
    clamped[0] &= 248;
    clamped[31] &= 127;
    clamped[31] |= 64;
    StaticSecret::from(clamped)
}

fn ed25519_public_to_x25519(public: &VerifyingKey) -> Result<PublicKey> {
    let compressed = CompressedEdwardsY(*public.as_bytes());
    let edwards = compressed.decompress().ok_or(Error::InvalidPublicKey)?;
    let montgomery: MontgomeryPoint = edwards.to_montgomery();
    Ok(PublicKey::from(montgomery.to_bytes()))
}

trait PublicKeyBytes {
    fn public_bytes(&self) -> [u8; KEY_LEN];
}

impl PublicKeyBytes for SigningKey {
    fn public_bytes(&self) -> [u8; KEY_LEN] {
        self.verifying_key().to_bytes()
    }
}

impl PublicKeyBytes for VerifyingKey {
    fn public_bytes(&self) -> [u8; KEY_LEN] {
        self.to_bytes()
    }
}
