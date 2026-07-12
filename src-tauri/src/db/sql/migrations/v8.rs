use super::common::MigrationDefinition;
use crate::db::api_key_crypto::upgrade_plaintext_api_keys;
use crate::db::StoreError;
use rusqlite::Connection;

pub const MIGRATION_SQL: &[(&str, &str)] = &[];

fn upgrade_api_key_encryption(conn: &Connection) -> Result<(), StoreError> {
    upgrade_plaintext_api_keys(conn)
}

pub const MIGRATION: MigrationDefinition = MigrationDefinition {
    version: 8,
    description: "v8 migration: Encrypt provider API keys",
    sql: MIGRATION_SQL,
    ensure: Some(upgrade_api_key_encryption),
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::api_key_crypto::{
        decrypt_api_key, API_KEY_ENCRYPTION_CONFIG_KEY, ENCRYPTED_VALUE_PREFIX,
    };

    #[test]
    fn upgrades_plaintext_provider_keys_without_replacing_the_master_key() {
        let conn = Connection::open_in_memory().expect("failed to open database");
        conn.execute(
            "CREATE TABLE config (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .expect("failed to create config table");
        conn.execute(
            "CREATE TABLE ai_model (id INTEGER PRIMARY KEY, api_key TEXT NOT NULL)",
            [],
        )
        .expect("failed to create model table");
        conn.execute(
            "INSERT INTO ai_model (id, api_key) VALUES (1, 'legacy-key')",
            [],
        )
        .expect("failed to insert legacy key");
        conn.execute(
            "INSERT INTO ai_model (id, api_key) VALUES (2, 'AES|legacy-key')",
            [],
        )
        .expect("failed to insert uppercase-prefixed legacy key");

        upgrade_api_key_encryption(&conn).expect("failed to upgrade provider key");
        let wrapped_master_key: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                [API_KEY_ENCRYPTION_CONFIG_KEY],
                |row| row.get(0),
            )
            .expect("missing wrapped master key");
        let encrypted_key: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing encrypted provider key");
        let encrypted_uppercase_prefixed_key: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 2", [], |row| {
                row.get(0)
            })
            .expect("missing encrypted uppercase-prefixed provider key");

        assert!(encrypted_key.starts_with(ENCRYPTED_VALUE_PREFIX));
        assert!(encrypted_uppercase_prefixed_key.starts_with(ENCRYPTED_VALUE_PREFIX));
        assert_eq!(
            decrypt_api_key(&conn, &encrypted_key).unwrap(),
            "legacy-key"
        );
        assert_eq!(
            decrypt_api_key(&conn, &encrypted_uppercase_prefixed_key).unwrap(),
            "AES|legacy-key"
        );

        upgrade_api_key_encryption(&conn).expect("repeat upgrade should succeed");
        let encrypted_key_after: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 1", [], |row| {
                row.get(0)
            })
            .expect("missing encrypted provider key after repeat upgrade");
        let encrypted_uppercase_prefixed_key_after: String = conn
            .query_row("SELECT api_key FROM ai_model WHERE id = 2", [], |row| {
                row.get(0)
            })
            .expect("missing uppercase-prefixed provider key after repeat upgrade");
        let wrapped_master_key_after: String = conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                [API_KEY_ENCRYPTION_CONFIG_KEY],
                |row| row.get(0),
            )
            .expect("missing wrapped master key");
        assert_eq!(encrypted_key, encrypted_key_after);
        assert_eq!(
            encrypted_uppercase_prefixed_key,
            encrypted_uppercase_prefixed_key_after
        );
        assert_eq!(wrapped_master_key, wrapped_master_key_after);
    }
}
