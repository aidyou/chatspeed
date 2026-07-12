use crate::db::error::StoreError;
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

pub const ENCRYPTED_VALUE_PREFIX: &str = "aes|";
pub const API_KEY_ENCRYPTION_CONFIG_KEY: &str = "api_key_encryption_key";

const MASTER_KEY_WRAPPING_SECRET: &str =
    "ChatSpeed API key master-key wrapper v1: 7d9a4ee251e1b28e3e4d47a6e5c8dbf9";

pub fn encrypt_api_key(conn: &Connection, api_key: &str) -> Result<String, StoreError> {
    let master_key = get_or_create_master_key(conn)?;
    encrypt_value(&master_key, api_key)
}

pub fn decrypt_api_key(conn: &Connection, value: &str) -> Result<String, StoreError> {
    if !value.starts_with(ENCRYPTED_VALUE_PREFIX) {
        return Ok(value.to_string());
    }

    let master_key = get_or_create_master_key(conn)?;
    decrypt_value(&master_key, value)
}

pub fn upgrade_plaintext_api_keys(conn: &Connection) -> Result<(), StoreError> {
    let master_key = get_or_create_master_key(conn)?;
    let mut statement = conn.prepare(
        "SELECT id, api_key FROM ai_model
         WHERE substr(api_key, 1, 4) != 'aes|'",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
    })?;

    let mut plaintext_keys = Vec::new();
    for row in rows {
        plaintext_keys.push(row?);
    }
    drop(statement);

    for (id, api_key) in plaintext_keys {
        let encrypted_api_key = encrypt_value(&master_key, &api_key)?;
        conn.execute(
            "UPDATE ai_model SET api_key = ?1 WHERE id = ?2",
            params![encrypted_api_key, id],
        )?;
    }

    Ok(())
}

fn get_or_create_master_key(conn: &Connection) -> Result<[u8; 32], StoreError> {
    let stored_value = conn
        .query_row(
            "SELECT value FROM config WHERE key = ?1",
            [API_KEY_ENCRYPTION_CONFIG_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;

    if let Some(stored_value) = stored_value {
        let wrapped_key: String = serde_json::from_str(&stored_value).map_err(|_| {
            StoreError::InvalidData("Invalid API key encryption configuration".to_string())
        })?;
        let key_hex = decrypt_value(&wrapping_key(), &wrapped_key)?;
        return decode_master_key(&key_hex);
    }

    let mut random_seed = [0u8; 32];
    OsRng.fill_bytes(&mut random_seed);
    let master_key: [u8; 32] = Sha256::digest(random_seed).into();
    let wrapped_key = encrypt_value(&wrapping_key(), &hex::encode(master_key))?;
    let serialized_value = serde_json::to_string(&wrapped_key)?;

    conn.execute(
        "INSERT INTO config (key, value) VALUES (?1, ?2)",
        params![API_KEY_ENCRYPTION_CONFIG_KEY, serialized_value],
    )?;

    Ok(master_key)
}

fn wrapping_key() -> [u8; 32] {
    Sha256::digest(MASTER_KEY_WRAPPING_SECRET.as_bytes()).into()
}

fn decode_master_key(key_hex: &str) -> Result<[u8; 32], StoreError> {
    let key = hex::decode(key_hex).map_err(|_| {
        StoreError::InvalidData("Invalid API key encryption configuration".to_string())
    })?;
    key.try_into().map_err(|_| {
        StoreError::InvalidData("Invalid API key encryption configuration".to_string())
    })
}

fn encrypt_value(key: &[u8; 32], plaintext: &str) -> Result<String, StoreError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| StoreError::InvalidData("Invalid API key encryption key".to_string()))?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plaintext.as_bytes())
        .map_err(|_| StoreError::InvalidData("Failed to encrypt API key".to_string()))?;

    let mut payload = nonce.to_vec();
    payload.extend(ciphertext);
    Ok(format!(
        "{}{}",
        ENCRYPTED_VALUE_PREFIX,
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decrypt_value(key: &[u8; 32], value: &str) -> Result<String, StoreError> {
    let encoded_payload = value
        .strip_prefix(ENCRYPTED_VALUE_PREFIX)
        .ok_or_else(|| StoreError::InvalidData("Invalid encrypted API key format".to_string()))?;
    let payload = URL_SAFE_NO_PAD
        .decode(encoded_payload)
        .map_err(|_| StoreError::InvalidData("Invalid encrypted API key format".to_string()))?;
    if payload.len() < 12 {
        return Err(StoreError::InvalidData(
            "Invalid encrypted API key format".to_string(),
        ));
    }
    let (nonce, ciphertext) = payload.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| StoreError::InvalidData("Invalid API key encryption key".to_string()))?;
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| StoreError::InvalidData("Unable to decrypt API key".to_string()))?;

    String::from_utf8(plaintext)
        .map_err(|_| StoreError::InvalidData("Invalid decrypted API key".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypts_and_decrypts_api_keys() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute(
            "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .expect("failed to create config table");

        let plaintext = "key-one\nkey-two";
        let encrypted = encrypt_api_key(&conn, plaintext).expect("failed to encrypt API key");

        assert!(encrypted.starts_with(ENCRYPTED_VALUE_PREFIX));
        assert_ne!(encrypted, plaintext);
        assert_eq!(decrypt_api_key(&conn, &encrypted).unwrap(), plaintext);
        assert_eq!(decrypt_api_key(&conn, "legacy-key").unwrap(), "legacy-key");
        assert!(decrypt_api_key(&conn, "aes|not-valid").is_err());
    }

    #[test]
    fn reuses_existing_wrapped_master_key() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute(
            "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .expect("failed to create config table");

        let first = encrypt_api_key(&conn, "first").expect("failed to encrypt first key");
        let stored_key: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                [API_KEY_ENCRYPTION_CONFIG_KEY],
                |row| row.get(0),
            )
            .expect("missing wrapped master key");
        let second = encrypt_api_key(&conn, "second").expect("failed to encrypt second key");
        let stored_key_after: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                [API_KEY_ENCRYPTION_CONFIG_KEY],
                |row| row.get(0),
            )
            .expect("missing wrapped master key");

        assert!(stored_key.starts_with("\"aes|"));
        assert_eq!(stored_key, stored_key_after);
        assert_eq!(decrypt_api_key(&conn, &first).unwrap(), "first");
        assert_eq!(decrypt_api_key(&conn, &second).unwrap(), "second");
    }
}
