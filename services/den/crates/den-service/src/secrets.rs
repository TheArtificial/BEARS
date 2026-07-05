use aes_gcm::{
    aead::{rand_core::RngCore, Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest, Sha256};

use den_core::DenError;

const SECRET_PREFIX: &str = "v1";
const NONCE_LEN: usize = 12;

fn cipher(secret_key: &str) -> Result<Aes256Gcm, DenError> {
    let secret_key = secret_key.trim();
    if secret_key.len() < 16 {
        return Err(DenError::ValidationError(
            "DEN_SECRET_ENCRYPTION_KEY must be set to at least 16 characters to store encrypted secrets".to_string(),
        ));
    }
    let digest = Sha256::digest(secret_key.as_bytes());
    Ok(Aes256Gcm::new_from_slice(&digest)
        .map_err(|err| DenError::System(format!("initialize Den secret cipher: {err}")))?)
}

pub fn encrypt_secret(plaintext: &str, secret_key: &str) -> Result<String, DenError> {
    let plaintext = plaintext.trim();
    if plaintext.is_empty() {
        return Err(DenError::ValidationError(
            "secret value is empty".to_string(),
        ));
    }
    let cipher = cipher(secret_key)?;
    let mut nonce_bytes = [0_u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .map_err(|err| DenError::System(format!("encrypt Den secret: {err}")))?;
    Ok(format!(
        "{SECRET_PREFIX}:{}:{}",
        URL_SAFE_NO_PAD.encode(nonce_bytes),
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

pub fn decrypt_secret(encoded: &str, secret_key: &str) -> Result<String, DenError> {
    let mut parts = encoded.split(':');
    let Some(prefix) = parts.next() else {
        return Err(DenError::ValidationError(
            "encrypted secret is malformed".to_string(),
        ));
    };
    if prefix != SECRET_PREFIX {
        return Err(DenError::ValidationError(
            "encrypted secret has unsupported version".to_string(),
        ));
    }
    let Some(nonce_part) = parts.next() else {
        return Err(DenError::ValidationError(
            "encrypted secret nonce is missing".to_string(),
        ));
    };
    let Some(ciphertext_part) = parts.next() else {
        return Err(DenError::ValidationError(
            "encrypted secret ciphertext is missing".to_string(),
        ));
    };
    if parts.next().is_some() {
        return Err(DenError::ValidationError(
            "encrypted secret is malformed".to_string(),
        ));
    }
    let nonce = URL_SAFE_NO_PAD.decode(nonce_part).map_err(|err| {
        DenError::ValidationError(format!("encrypted secret nonce is invalid: {err}"))
    })?;
    if nonce.len() != NONCE_LEN {
        return Err(DenError::ValidationError(
            "encrypted secret nonce length is invalid".to_string(),
        ));
    }
    let ciphertext = URL_SAFE_NO_PAD.decode(ciphertext_part).map_err(|err| {
        DenError::ValidationError(format!("encrypted secret ciphertext is invalid: {err}"))
    })?;
    let cipher = cipher(secret_key)?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
        .map_err(|err| DenError::System(format!("decrypt Den secret: {err}")))?;
    String::from_utf8(plaintext)
        .map_err(|err| DenError::ValidationError(format!("decrypted secret is not UTF-8: {err}")))
}

#[cfg(test)]
mod tests;
