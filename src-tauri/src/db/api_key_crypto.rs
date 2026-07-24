use crate::db::error::StoreError;
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rusqlite::{params, Connection, OptionalExtension};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const LEGACY_ENCRYPTED_VALUE_PREFIX: &str = "aes|";
pub const V2_ENCRYPTED_VALUE_PREFIX: &str = "aes_2|";
pub const API_KEY_ENCRYPTION_CONFIG_KEY: &str = "api_key_encryption_key";
pub const API_KEY_FILE_CONFIG_KEY: &str = "api_key_file";

const KEY_FILE_TYPE: &str = "chatspeed-api-key";
const KEY_FILE_VERSION: u32 = 1;
const V2_AAD_CONTEXT: &str = "chatspeed:ai_model.api_key:aes_2";
const MASTER_KEY_WRAPPING_SECRET: &str =
    "ChatSpeed API key master-key wrapper v1: 7d9a4ee251e1b28e3e4d47a6e5c8dbf9";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyEncryptionState {
    Legacy,
    Ready,
    Locked,
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiKeyEncryptionStatus {
    pub state: ApiKeyEncryptionState,
    pub key_file: Option<String>,
    pub key_id: Option<String>,
    pub required_key_ids: Vec<String>,
    pub reason: Option<String>,
}

impl ApiKeyEncryptionStatus {
    pub fn is_locked(&self) -> bool {
        matches!(
            self.state,
            ApiKeyEncryptionState::Locked | ApiKeyEncryptionState::Unsupported
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApiKeyFile {
    r#type: String,
    version: u32,
    id: String,
    key: String,
}

#[derive(Clone)]
struct KeyMaterial {
    id: String,
    key: [u8; 32],
}

enum StoredValue<'a> {
    Plaintext,
    LegacyV1,
    V2 { key_id: &'a str, payload: &'a str },
    Unsupported,
}

pub fn encrypt_api_key(conn: &Connection, api_key: &str) -> Result<String, StoreError> {
    match configured_key(conn) {
        Ok(Some(key)) => {
            if inspect_encryption_status(conn)?.is_locked() {
                return Err(api_keys_locked_error());
            }
            encrypt_v2(&key, api_key)
        }
        Ok(None) | Err(_) => {
            if inspect_encryption_status(conn)?.is_locked() {
                return Err(api_keys_locked_error());
            }
            let master_key = get_or_create_master_key(conn)?;
            encrypt_legacy_value(&master_key, api_key)
        }
    }
}

pub fn decrypt_api_key(conn: &Connection, value: &str) -> Result<String, StoreError> {
    match classify_value(value) {
        StoredValue::V2 { .. } => {
            decrypt_with_keys(conn, value, configured_key(conn)?.as_ref(), None)
        }
        StoredValue::Plaintext | StoredValue::LegacyV1 | StoredValue::Unsupported => {
            decrypt_with_keys(conn, value, None, None)
        }
    }
}

pub fn inspect_encryption_status(conn: &Connection) -> Result<ApiKeyEncryptionStatus, StoreError> {
    let values = stored_api_key_values(conn)?;
    let mut required_key_ids = Vec::new();

    for value in &values {
        match classify_value(value) {
            StoredValue::V2 { key_id, .. } => {
                if !required_key_ids.iter().any(|id| id == key_id) {
                    required_key_ids.push(key_id.to_string());
                }
            }
            StoredValue::Unsupported => {
                return Ok(ApiKeyEncryptionStatus {
                    state: ApiKeyEncryptionState::Unsupported,
                    key_file: configured_key_path(conn)?.map(|path| path.display().to_string()),
                    key_id: None,
                    required_key_ids,
                    reason: Some("unsupported_version".to_string()),
                });
            }
            StoredValue::Plaintext | StoredValue::LegacyV1 => {}
        }
    }

    let key_path = configured_key_path(conn)?;
    let key_file = key_path.as_ref().map(|path| path.display().to_string());
    let key = match key_path.as_ref() {
        Some(path) => match read_key_file(path) {
            Ok(key) => Some(key),
            Err(error) => {
                return Ok(ApiKeyEncryptionStatus {
                    state: if required_key_ids.is_empty() {
                        ApiKeyEncryptionState::Legacy
                    } else {
                        ApiKeyEncryptionState::Locked
                    },
                    key_file,
                    key_id: None,
                    required_key_ids,
                    reason: Some(key_file_error_reason(&error).to_string()),
                });
            }
        },
        None => None,
    };

    if required_key_ids.is_empty() {
        return Ok(ApiKeyEncryptionStatus {
            state: if key.is_some() {
                ApiKeyEncryptionState::Ready
            } else {
                ApiKeyEncryptionState::Legacy
            },
            key_file,
            key_id: key.map(|key| key.id),
            required_key_ids,
            reason: None,
        });
    }

    let Some(key) = key else {
        return Ok(ApiKeyEncryptionStatus {
            state: ApiKeyEncryptionState::Locked,
            key_file,
            key_id: None,
            required_key_ids,
            reason: Some("key_file_not_configured".to_string()),
        });
    };

    if required_key_ids.len() != 1 || required_key_ids[0] != key.id {
        return Ok(ApiKeyEncryptionStatus {
            state: ApiKeyEncryptionState::Locked,
            key_file,
            key_id: Some(key.id),
            required_key_ids,
            reason: Some("wrong_key_file".to_string()),
        });
    }

    if values.iter().any(|value| {
        matches!(classify_value(value), StoredValue::V2 { .. }) && decrypt_v2(&key, value).is_err()
    }) {
        return Ok(ApiKeyEncryptionStatus {
            state: ApiKeyEncryptionState::Locked,
            key_file,
            key_id: Some(key.id),
            required_key_ids,
            reason: Some("decryption_failed".to_string()),
        });
    }

    Ok(ApiKeyEncryptionStatus {
        state: ApiKeyEncryptionState::Ready,
        key_file,
        key_id: Some(key.id),
        required_key_ids,
        reason: None,
    })
}

pub fn generate_key_file(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        return Err(StoreError::AlreadyExists(format!(
            "API key file already exists: {}",
            path.display()
        )));
    }

    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    let key_file = ApiKeyFile {
        r#type: KEY_FILE_TYPE.to_string(),
        version: KEY_FILE_VERSION,
        id: key_id(&key),
        key: hex::encode(key),
    };
    let serialized = serde_json::to_vec_pretty(&key_file)?;

    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    let mut file = options.open(path)?;
    if let Err(error) = file
        .write_all(&serialized)
        .and_then(|_| file.write_all(b"\n"))
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(path);
        return Err(StoreError::IoError(error.to_string()));
    }

    Ok(())
}

pub fn activate_key_file(conn: &mut Connection, path: &Path) -> Result<(), StoreError> {
    let canonical_path = fs::canonicalize(path)?;
    let target_key = read_key_file(&canonical_path)?;
    let current_key = configured_key(conn).ok().flatten();
    let rows = stored_api_key_rows(conn)?;
    let mut encrypted_rows = Vec::with_capacity(rows.len());

    for (id, value) in rows {
        let plaintext = decrypt_with_keys(conn, &value, current_key.as_ref(), Some(&target_key))?;
        encrypted_rows.push((id, encrypt_v2(&target_key, &plaintext)?));
    }

    let tx = conn.transaction()?;
    for (id, encrypted_value) in encrypted_rows {
        tx.execute(
            "UPDATE ai_model SET api_key = ?1 WHERE id = ?2",
            params![encrypted_value, id],
        )?;
    }
    tx.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
        params![
            API_KEY_FILE_CONFIG_KEY,
            serde_json::to_string(&canonical_path.display().to_string())?
        ],
    )?;
    tx.execute(
        "DELETE FROM config WHERE key = ?1",
        [API_KEY_ENCRYPTION_CONFIG_KEY],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn migrate_to_configured_key_if_available(conn: &mut Connection) -> Result<bool, StoreError> {
    let Some(path) = configured_key_path(conn)? else {
        return Ok(false);
    };
    if !path.is_file() {
        return Ok(false);
    }
    let needs_migration = stored_api_key_values(conn)?.iter().any(|value| {
        matches!(
            classify_value(value),
            StoredValue::Plaintext | StoredValue::LegacyV1
        )
    });
    if !needs_migration {
        let status = inspect_encryption_status(conn)?;
        if matches!(status.state, ApiKeyEncryptionState::Ready) {
            return delete_legacy_master_key(conn);
        }
        return Ok(false);
    }
    let status = inspect_encryption_status(conn)?;
    if status.is_locked() {
        return Ok(false);
    }
    activate_key_file(conn, &path)?;
    Ok(true)
}

fn delete_legacy_master_key(conn: &Connection) -> Result<bool, StoreError> {
    Ok(conn.execute(
        "DELETE FROM config WHERE key = ?1",
        [API_KEY_ENCRYPTION_CONFIG_KEY],
    )? > 0)
}

pub fn upgrade_plaintext_api_keys(conn: &Connection) -> Result<(), StoreError> {
    let rows = stored_api_key_rows(conn)?;
    let mut plaintext_keys = Vec::new();
    for (id, api_key) in rows {
        match classify_value(&api_key) {
            StoredValue::Plaintext => plaintext_keys.push((id, api_key)),
            StoredValue::LegacyV1 | StoredValue::V2 { .. } => {}
            StoredValue::Unsupported => {}
        }
    }

    if plaintext_keys.is_empty() {
        return Ok(());
    }

    let master_key = get_or_create_master_key(conn)?;
    for (id, api_key) in plaintext_keys {
        let encrypted_api_key = encrypt_legacy_value(&master_key, &api_key)?;
        conn.execute(
            "UPDATE ai_model SET api_key = ?1 WHERE id = ?2",
            params![encrypted_api_key, id],
        )?;
    }
    Ok(())
}

fn classify_value(value: &str) -> StoredValue<'_> {
    if value.starts_with(LEGACY_ENCRYPTED_VALUE_PREFIX) {
        return StoredValue::LegacyV1;
    }
    if let Some(rest) = value.strip_prefix(V2_ENCRYPTED_VALUE_PREFIX) {
        if let Some((key_id, payload)) = rest.split_once('|') {
            if !key_id.is_empty() && !payload.is_empty() {
                return StoredValue::V2 { key_id, payload };
            }
        }
        return StoredValue::Unsupported;
    }
    if value.starts_with("aes_") && value.contains('|') {
        return StoredValue::Unsupported;
    }
    StoredValue::Plaintext
}

fn decrypt_with_keys(
    conn: &Connection,
    value: &str,
    configured: Option<&KeyMaterial>,
    candidate: Option<&KeyMaterial>,
) -> Result<String, StoreError> {
    match classify_value(value) {
        StoredValue::Plaintext => Ok(value.to_string()),
        StoredValue::LegacyV1 => {
            let master_key = get_or_create_master_key(conn)?;
            decrypt_legacy_value(&master_key, value)
        }
        StoredValue::V2 { key_id, .. } => {
            let key = candidate
                .filter(|key| key.id == key_id)
                .or_else(|| configured.filter(|key| key.id == key_id))
                .ok_or_else(api_keys_locked_error)?;
            decrypt_v2(key, value)
        }
        StoredValue::Unsupported => Err(StoreError::InvalidData(
            "Unsupported or malformed encrypted API key value".to_string(),
        )),
    }
}

fn encrypt_v2(key: &KeyMaterial, plaintext: &str) -> Result<String, StoreError> {
    let cipher = Aes256Gcm::new_from_slice(&key.key)
        .map_err(|_| StoreError::InvalidData("Invalid API key encryption key".to_string()))?;
    let mut nonce = [0u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let aad = format!("{}:{}", V2_AAD_CONTEXT, key.id);
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| StoreError::InvalidData("Failed to encrypt API key".to_string()))?;
    let mut payload = nonce.to_vec();
    payload.extend(ciphertext);
    Ok(format!(
        "{}{}|{}",
        V2_ENCRYPTED_VALUE_PREFIX,
        key.id,
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decrypt_v2(key: &KeyMaterial, value: &str) -> Result<String, StoreError> {
    let StoredValue::V2 { key_id, payload } = classify_value(value) else {
        return Err(StoreError::InvalidData(
            "Invalid encrypted API key format".to_string(),
        ));
    };
    if key_id != key.id {
        return Err(api_keys_locked_error());
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| StoreError::InvalidData("Invalid encrypted API key format".to_string()))?;
    if payload.len() < 12 {
        return Err(StoreError::InvalidData(
            "Invalid encrypted API key format".to_string(),
        ));
    }
    let (nonce, ciphertext) = payload.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(&key.key)
        .map_err(|_| StoreError::InvalidData("Invalid API key encryption key".to_string()))?;
    let aad = format!("{}:{}", V2_AAD_CONTEXT, key.id);
    let plaintext = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| StoreError::InvalidData("Unable to decrypt API key".to_string()))?;
    String::from_utf8(plaintext)
        .map_err(|_| StoreError::InvalidData("Invalid decrypted API key".to_string()))
}

fn configured_key(conn: &Connection) -> Result<Option<KeyMaterial>, StoreError> {
    configured_key_path(conn)?
        .map(|path| read_key_file(&path))
        .transpose()
}

fn configured_key_path(conn: &Connection) -> Result<Option<PathBuf>, StoreError> {
    let stored_value = conn
        .query_row(
            "SELECT value FROM config WHERE key = ?1",
            [API_KEY_FILE_CONFIG_KEY],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    stored_value
        .map(|value| {
            serde_json::from_str::<String>(&value)
                .map(PathBuf::from)
                .map_err(|_| {
                    StoreError::InvalidData("Invalid API key file configuration".to_string())
                })
        })
        .transpose()
}

fn read_key_file(path: &Path) -> Result<KeyMaterial, StoreError> {
    let contents = fs::read(path)?;
    let key_file: ApiKeyFile = serde_json::from_slice(&contents)
        .map_err(|_| StoreError::InvalidData("Invalid ChatSpeed API key file".to_string()))?;
    if key_file.r#type != KEY_FILE_TYPE || key_file.version != KEY_FILE_VERSION {
        return Err(StoreError::InvalidData(
            "Unsupported ChatSpeed API key file".to_string(),
        ));
    }
    let key_bytes = hex::decode(&key_file.key)
        .map_err(|_| StoreError::InvalidData("Invalid ChatSpeed API key file".to_string()))?;
    let key: [u8; 32] = key_bytes
        .try_into()
        .map_err(|_| StoreError::InvalidData("Invalid ChatSpeed API key file".to_string()))?;
    let id = key_id(&key);
    if key_file.id != id {
        return Err(StoreError::InvalidData(
            "Invalid ChatSpeed API key file fingerprint".to_string(),
        ));
    }
    Ok(KeyMaterial { id, key })
}

fn key_id(key: &[u8; 32]) -> String {
    hex::encode(Sha256::digest(key))[..32].to_string()
}

fn key_file_error_reason(error: &StoreError) -> &'static str {
    match error {
        StoreError::IoError(_) => "key_file_missing",
        _ => "invalid_key_file",
    }
}

fn api_keys_locked_error() -> StoreError {
    StoreError::InvalidData(t!("db.api_keys_locked").to_string())
}

fn stored_api_key_values(conn: &Connection) -> Result<Vec<String>, StoreError> {
    Ok(stored_api_key_rows(conn)?
        .into_iter()
        .map(|(_, value)| value)
        .collect())
}

fn stored_api_key_rows(conn: &Connection) -> Result<Vec<(i64, String)>, StoreError> {
    let mut statement = conn.prepare("SELECT id, api_key FROM ai_model ORDER BY id")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(StoreError::from)
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
        let key_hex = decrypt_legacy_value(&wrapping_key(), &wrapped_key)?;
        return decode_master_key(&key_hex);
    }

    let mut master_key = [0u8; 32];
    OsRng.fill_bytes(&mut master_key);
    let wrapped_key = encrypt_legacy_value(&wrapping_key(), &hex::encode(master_key))?;
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

fn encrypt_legacy_value(key: &[u8; 32], plaintext: &str) -> Result<String, StoreError> {
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
        LEGACY_ENCRYPTED_VALUE_PREFIX,
        URL_SAFE_NO_PAD.encode(payload)
    ))
}

fn decrypt_legacy_value(key: &[u8; 32], value: &str) -> Result<String, StoreError> {
    let encoded_payload = value
        .strip_prefix(LEGACY_ENCRYPTED_VALUE_PREFIX)
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

    fn create_tables(conn: &Connection) {
        conn.execute(
            "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .expect("failed to create config table");
        conn.execute(
            "CREATE TABLE ai_model (id INTEGER PRIMARY KEY, api_key TEXT NOT NULL)",
            [],
        )
        .expect("failed to create ai_model table");
    }

    #[test]
    fn legacy_encryption_remains_compatible() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        let plaintext = "key-one\nkey-two";
        let encrypted = encrypt_api_key(&conn, plaintext).expect("failed to encrypt API key");
        assert!(encrypted.starts_with(LEGACY_ENCRYPTED_VALUE_PREFIX));
        assert_eq!(decrypt_api_key(&conn, &encrypted).unwrap(), plaintext);
        assert_eq!(decrypt_api_key(&conn, "legacy-key").unwrap(), "legacy-key");
    }

    #[test]
    fn key_file_migrates_legacy_values_to_v2() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        let legacy = encrypt_api_key(&conn, "secret").expect("failed to encrypt legacy key");
        conn.execute(
            "INSERT INTO ai_model (id, api_key) VALUES (1, ?1)",
            [legacy],
        )
        .expect("failed to insert model");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = temp_dir.path().join("test.csk");
        generate_key_file(&key_path).expect("failed to generate key file");
        activate_key_file(&mut conn, &key_path).expect("failed to activate key file");

        let encrypted: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing encrypted key");
        assert!(encrypted.starts_with(V2_ENCRYPTED_VALUE_PREFIX));
        assert_eq!(decrypt_api_key(&conn, &encrypted).unwrap(), "secret");
        assert!(matches!(
            inspect_encryption_status(&conn).unwrap().state,
            ApiKeyEncryptionState::Ready
        ));
        assert!(!config_key_exists(&conn, API_KEY_ENCRYPTION_CONFIG_KEY));
    }

    #[test]
    fn ready_v2_values_remove_a_stale_legacy_master_key() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        conn.execute("INSERT INTO ai_model (id, api_key) VALUES (1, 'plain')", [])
            .expect("failed to insert model");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = temp_dir.path().join("test.csk");
        generate_key_file(&key_path).expect("failed to generate key file");
        activate_key_file(&mut conn, &key_path).expect("failed to activate key file");

        get_or_create_master_key(&conn).expect("failed to recreate stale legacy master key");
        assert!(config_key_exists(&conn, API_KEY_ENCRYPTION_CONFIG_KEY));

        assert!(migrate_to_configured_key_if_available(&mut conn)
            .expect("failed to clean stale legacy master key"));
        assert!(!config_key_exists(&conn, API_KEY_ENCRYPTION_CONFIG_KEY));
        let encrypted: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing encrypted key");
        assert_eq!(decrypt_api_key(&conn, &encrypted).unwrap(), "plain");
    }

    #[test]
    fn failed_key_activation_keeps_the_legacy_master_key() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        get_or_create_master_key(&conn).expect("failed to create legacy master key");
        conn.execute(
            "INSERT INTO ai_model (id, api_key) VALUES (1, 'aes|invalid')",
            [],
        )
        .expect("failed to insert invalid legacy value");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = temp_dir.path().join("test.csk");
        generate_key_file(&key_path).expect("failed to generate key file");

        assert!(activate_key_file(&mut conn, &key_path).is_err());
        assert!(config_key_exists(&conn, API_KEY_ENCRYPTION_CONFIG_KEY));
        assert!(!config_key_exists(&conn, API_KEY_FILE_CONFIG_KEY));
    }

    #[test]
    fn missing_key_file_locks_v2_values() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        conn.execute("INSERT INTO ai_model (id, api_key) VALUES (1, 'plain')", [])
            .expect("failed to insert model");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = temp_dir.path().join("test.csk");
        generate_key_file(&key_path).expect("failed to generate key file");
        activate_key_file(&mut conn, &key_path).expect("failed to activate key file");
        fs::remove_file(&key_path).expect("failed to remove key file");

        let status = inspect_encryption_status(&conn).expect("failed to inspect status");
        assert!(matches!(status.state, ApiKeyEncryptionState::Locked));
        assert_eq!(status.reason.as_deref(), Some("key_file_missing"));
    }

    #[test]
    fn missing_configured_key_file_does_not_block_legacy_values() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        let legacy = encrypt_api_key(&conn, "legacy-secret").expect("failed to encrypt legacy key");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let missing_path = temp_dir.path().join("missing.csk");
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            params![
                API_KEY_FILE_CONFIG_KEY,
                serde_json::to_string(&missing_path.display().to_string()).unwrap()
            ],
        )
        .expect("failed to configure missing key file");

        assert_eq!(decrypt_api_key(&conn, &legacy).unwrap(), "legacy-secret");
        assert!(encrypt_api_key(&conn, "new-secret")
            .unwrap()
            .starts_with(LEGACY_ENCRYPTED_VALUE_PREFIX));
        let status = inspect_encryption_status(&conn).unwrap();
        assert!(matches!(status.state, ApiKeyEncryptionState::Legacy));
        assert_eq!(status.reason.as_deref(), Some("key_file_missing"));
    }

    #[test]
    fn plaintext_upgrade_does_not_rewrap_v2_values() {
        let mut conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        conn.execute("INSERT INTO ai_model (id, api_key) VALUES (1, 'plain')", [])
            .expect("failed to insert plaintext model");
        let temp_dir = tempfile::tempdir().expect("failed to create temp dir");
        let key_path = temp_dir.path().join("test.csk");
        generate_key_file(&key_path).expect("failed to generate key file");
        activate_key_file(&mut conn, &key_path).expect("failed to activate key file");
        let before: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing v2 value");

        upgrade_plaintext_api_keys(&conn).expect("plaintext upgrade should succeed");
        let after: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing v2 value after upgrade");
        assert_eq!(before, after);
    }

    #[test]
    fn unsupported_encryption_versions_remain_locked() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        create_tables(&conn);
        conn.execute(
            "INSERT INTO ai_model (id, api_key) VALUES (1, 'aes_3|future-value')",
            [],
        )
        .expect("failed to insert future encrypted value");

        upgrade_plaintext_api_keys(&conn).expect("legacy ensure should leave future values alone");
        let status = inspect_encryption_status(&conn).expect("failed to inspect status");
        assert!(matches!(status.state, ApiKeyEncryptionState::Unsupported));
        assert!(status.is_locked());
    }

    fn config_key_exists(conn: &Connection, key: &str) -> bool {
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM config WHERE key = ?1)",
            [key],
            |row| row.get(0),
        )
        .expect("failed to inspect config key")
    }
}
