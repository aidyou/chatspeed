use crate::db::error::StoreError;
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, KeyInit};
use rust_i18n::t;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

/// Constant part of the encryption key
/// DO NOT MODIFY THIS VALUE
const BACKUP_ENCRYPTION_CONSTANT: &str = "acb168bd7dc06e86a99d34c4539c9a4e40224b84170c9b617fa0a893c37f74bc84fa2556012a4704756f806e007938482db74cc26b695c2bf84169cfe727d9bf";
const MAGIC: &[u8] = b"CSDB";

/// Generate encryption key
pub fn generate_encryption_key(backup_dir_name: &str, random_key: &[u8; 8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(BACKUP_ENCRYPTION_CONSTANT.as_bytes());
    hasher.update(backup_dir_name.as_bytes());
    hasher.update(random_key);

    // Use first 32 bytes as AES-256 key
    let result = hasher.finalize();
    hex::encode(&result[..32])
}

/// Encrypt database file using streaming approach
pub fn encrypt_database_streaming(
    input_path: &Path,
    output_path: &Path,
    backup_dir_name: &str,
) -> Result<(), StoreError> {
    const BUFFER_SIZE: usize = 64 * 1024; // 64KB buffer

    // Open input file
    let input_file = File::open(input_path).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_open_database_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;
    let mut reader = BufReader::new(input_file);

    // Create output file
    let output_file = File::create(output_path).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_create_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;
    let mut writer = BufWriter::new(output_file);

    // Generate random components
    let mut random_key = [0u8; 8];
    OsRng.fill_bytes(&mut random_key);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    // Generate key and cipher
    let key_str = generate_encryption_key(backup_dir_name, &random_key);
    let key_bytes = hex::decode(&key_str).map_err(|e| {
        StoreError::IoError(t!("db.backup.failed_to_decode_key", error = e.to_string()).to_string())
    })?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| {
        StoreError::IoError(
            t!("db.backup.failed_to_create_cipher", error = e.to_string()).to_string(),
        )
    })?;

    // Write header to output file
    write_encrypted_file_header(&mut writer, &random_key, &nonce_bytes)?;

    // Process file in chunks
    let mut buffer = vec![0u8; BUFFER_SIZE];
    let mut chunk_index = 0u32;

    loop {
        let bytes_read = reader.read(&mut buffer).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_database_file",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        if bytes_read == 0 {
            break; // EOF
        }

        let chunk = &buffer[..bytes_read];

        // Create nonce for this chunk (base nonce + chunk index)
        let mut chunk_nonce = nonce_bytes;
        // Add chunk index to nonce to ensure uniqueness for each chunk
        for i in 0..4 {
            chunk_nonce[i] ^= ((chunk_index >> (i * 8)) & 0xFF) as u8;
        }

        // Encrypt chunk
        let ciphertext = cipher.encrypt(&chunk_nonce.into(), chunk).map_err(|e| {
            StoreError::IoError(
                t!("db.backup.failed_to_encrypt_data", error = e.to_string()).to_string(),
            )
        })?;

        // Write chunk size (4 bytes) followed by encrypted data
        writer
            .write_all(&(ciphertext.len() as u32).to_le_bytes())
            .map_err(|e| {
                StoreError::IoError(
                    t!(
                        "db.backup.failed_to_write_encrypted_file",
                        error = e.to_string()
                    )
                    .to_string(),
                )
            })?;
        writer.write_all(&ciphertext).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_write_encrypted_file",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        chunk_index += 1;
    }

    // Write end marker (0 size)
    writer.write_all(&0u32.to_le_bytes()).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    writer.flush().map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    Ok(())
}

/// Decrypt database file using streaming approach
pub fn decrypt_database_streaming(
    input_path: &Path,
    output_path: &Path,
    backup_dir_name: &str,
) -> Result<(), StoreError> {
    // Open input file
    let input_file = File::open(input_path).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_open_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;
    let mut reader = BufReader::new(input_file);

    // Create output file
    let output_file = File::create(output_path).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_open_dest_db",
                path = output_path.display(),
                error = e.to_string()
            )
            .to_string(),
        )
    })?;
    let mut writer = BufWriter::new(output_file);

    // Read header from input file
    let (random_key, nonce_bytes) = read_encrypted_file_header(&mut reader)?;

    // Generate key and cipher
    let key_str = generate_encryption_key(backup_dir_name, &random_key);
    let key_bytes = hex::decode(&key_str).map_err(|e| {
        StoreError::IoError(t!("db.backup.failed_to_decode_key", error = e.to_string()).to_string())
    })?;
    let cipher = Aes256Gcm::new_from_slice(&key_bytes).map_err(|e| {
        StoreError::IoError(
            t!("db.backup.failed_to_create_cipher", error = e.to_string()).to_string(),
        )
    })?;

    // Process chunks
    let mut chunk_index = 0u32;

    loop {
        // Read chunk size
        let mut size_bytes = [0u8; 4];
        reader.read_exact(&mut size_bytes).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_encrypted_file",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        let chunk_size = u32::from_le_bytes(size_bytes);

        // Check for end marker
        if chunk_size == 0 {
            break;
        }

        // Read encrypted chunk
        let mut encrypted_chunk = vec![0u8; chunk_size as usize];
        reader.read_exact(&mut encrypted_chunk).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_read_encrypted_file",
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        // Create nonce for this chunk (same as during encryption)
        let mut chunk_nonce = nonce_bytes;
        for i in 0..4 {
            chunk_nonce[i] ^= ((chunk_index >> (i * 8)) & 0xFF) as u8;
        }

        // Decrypt chunk
        let decrypted_chunk = cipher
            .decrypt(&chunk_nonce.into(), encrypted_chunk.as_ref())
            .map_err(|e| {
                StoreError::IoError(
                    t!("db.backup.failed_to_decrypt_data", error = e.to_string()).to_string(),
                )
            })?;

        // Write decrypted data
        writer.write_all(&decrypted_chunk).map_err(|e| {
            StoreError::IoError(
                t!(
                    "db.backup.failed_to_write_temp_db",
                    path = output_path.display(),
                    error = e.to_string()
                )
                .to_string(),
            )
        })?;

        chunk_index += 1;
    }

    writer.flush().map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_temp_db",
                path = output_path.display(),
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    Ok(())
}

/// Write encrypted file header
fn write_encrypted_file_header<W: Write>(
    writer: &mut W,
    random_key: &[u8; 8],
    nonce: &[u8; 12],
) -> Result<(), StoreError> {
    // Write magic number
    writer.write_all(MAGIC).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    // Write version
    writer.write_all(&1u32.to_le_bytes()).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    // Write random key
    writer.write_all(random_key).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    // Write nonce
    writer.write_all(nonce).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_write_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    Ok(())
}

/// Read encrypted file header
fn read_encrypted_file_header<R: Read>(reader: &mut R) -> Result<([u8; 8], [u8; 12]), StoreError> {
    // Read magic number
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_read_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    if magic != *MAGIC {
        return Err(StoreError::IoError(
            t!("db.backup.invalid_magic_number").to_string(),
        ));
    }

    // Read version
    let mut version_bytes = [0u8; 4];
    reader.read_exact(&mut version_bytes).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_read_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    let version = u32::from_le_bytes(version_bytes);
    if version != 1 {
        return Err(StoreError::IoError(
            t!("db.backup.unsupported_version").to_string(),
        ));
    }

    // Read random key
    let mut random_key = [0u8; 8];
    reader.read_exact(&mut random_key).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_read_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    // Read nonce
    let mut nonce = [0u8; 12];
    reader.read_exact(&mut nonce).map_err(|e| {
        StoreError::IoError(
            t!(
                "db.backup.failed_to_read_encrypted_file",
                error = e.to_string()
            )
            .to_string(),
        )
    })?;

    Ok((random_key, nonce))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_generate_encryption_key_consistency() {
        let backup_dir_name = "consistent_test";
        let random_key = [1u8, 2, 3, 4, 5, 6, 7, 8];

        // Generate the same key twice
        let key1 = generate_encryption_key(backup_dir_name, &random_key);
        let key2 = generate_encryption_key(backup_dir_name, &random_key);

        // Verify they are identical
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_generate_encryption_key_uniqueness() {
        let backup_dir_name = "unique_test";
        let random_key1 = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let random_key2 = [8u8, 7, 6, 5, 4, 3, 2, 1];

        let key1 = generate_encryption_key(backup_dir_name, &random_key1);
        let key2 = generate_encryption_key(backup_dir_name, &random_key2);

        // Verify they are different
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_streaming_encryption_decryption_roundtrip() {
        // Create temporary files
        let temp_dir = TempDir::new().unwrap();
        let input_path = temp_dir.path().join("test.db");
        let encrypted_path = temp_dir.path().join("test.db.enc");
        let output_path = temp_dir.path().join("test_restored.db");
        let backup_dir_name = "test_backup";

        // Create a large test file (larger than buffer size to test multiple chunks)
        let mut test_data = vec![0u8; 128 * 1024]; // 128KB of zeros
        for (i, byte) in test_data.iter_mut().enumerate() {
            *byte = (i % 256) as u8; // Fill with pattern
        }

        // Write test data to input file
        {
            let mut file = File::create(&input_path).unwrap();
            file.write_all(&test_data).unwrap();
            file.flush().unwrap();
        }

        // Encrypt using streaming approach
        encrypt_database_streaming(&input_path, &encrypted_path, backup_dir_name).unwrap();

        // Decrypt using streaming approach
        decrypt_database_streaming(&encrypted_path, &output_path, backup_dir_name).unwrap();

        // Read decrypted data
        let mut decrypted_data = Vec::new();
        {
            let mut file = File::open(&output_path).unwrap();
            file.read_to_end(&mut decrypted_data).unwrap();
        }

        // Verify that the decrypted data matches the original
        assert_eq!(decrypted_data, test_data);
    }

    #[test]
    fn test_streaming_encryption_with_different_backup_dir_names() {
        // Create temporary files
        let temp_dir = TempDir::new().unwrap();
        let input_path = temp_dir.path().join("test.db");
        let encrypted_path1 = temp_dir.path().join("test1.db.enc");
        let encrypted_path2 = temp_dir.path().join("test2.db.enc");
        let output_path1 = temp_dir.path().join("restored1.db");
        let output_path2 = temp_dir.path().join("restored2.db");

        // Create test data
        let test_data = b"Different backup directory name test for streaming".to_vec();

        // Write test data to input file
        {
            let mut file = File::create(&input_path).unwrap();
            file.write_all(&test_data).unwrap();
            file.flush().unwrap();
        }

        // Encrypt with different backup directory names
        encrypt_database_streaming(&input_path, &encrypted_path1, "backup1").unwrap();
        encrypt_database_streaming(&input_path, &encrypted_path2, "backup2").unwrap();

        // Verify that the encrypted files are different
        let mut data1 = Vec::new();
        let mut data2 = Vec::new();
        {
            let mut file1 = File::open(&encrypted_path1).unwrap();
            let mut file2 = File::open(&encrypted_path2).unwrap();
            file1.read_to_end(&mut data1).unwrap();
            file2.read_to_end(&mut data2).unwrap();
        }
        assert_ne!(data1, data2);

        // Verify that each can be decrypted correctly with the corresponding backup name
        decrypt_database_streaming(&encrypted_path1, &output_path1, "backup1").unwrap();
        decrypt_database_streaming(&encrypted_path2, &output_path2, "backup2").unwrap();

        // Read decrypted data
        let mut decrypted_data1 = Vec::new();
        let mut decrypted_data2 = Vec::new();
        {
            let mut file1 = File::open(&output_path1).unwrap();
            let mut file2 = File::open(&output_path2).unwrap();
            file1.read_to_end(&mut decrypted_data1).unwrap();
            file2.read_to_end(&mut decrypted_data2).unwrap();
        }

        assert_eq!(decrypted_data1, test_data);
        assert_eq!(decrypted_data2, test_data);

        // Verify that using wrong backup name fails to decrypt
        let wrong_output_path = temp_dir.path().join("wrong.db");
        let result = decrypt_database_streaming(&encrypted_path1, &wrong_output_path, "backup2");
        assert!(result.is_err());
    }
}
