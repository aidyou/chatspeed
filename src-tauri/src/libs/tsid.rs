//! TSID (Time-Sorted Unique Identifier) generator implementation
//!
//! This module provides a thread-safe, synchronous, blocking TSID generator.
//! It is optimized for raw performance in single- and multi-threaded contexts.

use std::sync::Mutex;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// Custom Epoch: 2020-01-01T00:00:00Z in milliseconds
const TSID_EPOCH: u64 = 1577836800000;

/// A thread-safe, synchronous, blocking TSID generator.
/// This version is optimized for raw performance.
pub struct TsidGenerator {
    mu: Mutex<GeneratorState>,
    node_id: u64, // Node ID (0-1023)
}

/// Internal state of the generator
struct GeneratorState {
    last_time_ms: i64, // Last timestamp (milliseconds since custom epoch)
    counter: u32,      // Counter (0-4095)
}

/// Constants for bit allocation
const NODE_BITS: u8 = 10; // Node ID bits (up to 1024 nodes)
const COUNTER_BITS: u8 = 12; // Counter bits (4096 IDs per millisecond)
const TIME_SHIFT: u8 = NODE_BITS + COUNTER_BITS; // Timestamp left shift (22)
const MAX_COUNTER: u32 = (1 << COUNTER_BITS) - 1; // 4095
const MAX_RETRIES: usize = 1000;

impl TsidGenerator {
    /// Creates a new synchronous generator.
    pub fn new(node_id: u64) -> Result<Self, String> {
        if node_id >= (1 << NODE_BITS) {
            return Err(format!(
                "nodeID must be between 0-{}",
                (1 << NODE_BITS) - 1
            ));
        }

        let current_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .map_err(|e| format!("System time is before UNIX EPOCH: {}", e))?;

        // Initialize with current time and a random counter
        let random_bytes = Uuid::new_v4().to_bytes_le();
        let initial_counter =
            u32::from_le_bytes(random_bytes[0..4].try_into().unwrap()) % (MAX_COUNTER + 1);

        let state = GeneratorState {
            last_time_ms: current_ms - TSID_EPOCH as i64,
            counter: initial_counter,
        };

        Ok(TsidGenerator {
            mu: Mutex::new(state),
            node_id,
        })
    }

    /// Generates a TSID. This is a blocking operation.
    pub fn generate(&self) -> Result<String, String> {
        let mut state = self.mu.lock().unwrap(); // Lock once at the beginning

        for _ in 0..MAX_RETRIES {
            let now = SystemTime::now();
            let current_ms = now
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as i64
                - TSID_EPOCH as i64;

            if current_ms > state.last_time_ms {
                state.last_time_ms = current_ms;
                state.counter = 0;
            } else {
                state.counter += 1;
            }

            if state.counter <= MAX_COUNTER {
                let tsid_value = ((state.last_time_ms as u64) << TIME_SHIFT)
                    | (self.node_id << COUNTER_BITS)
                    | (state.counter as u64);

                let tsid_bytes = tsid_value.to_be_bytes();
                return Ok(crockford_base32_encode(&tsid_bytes));
            }

            // Counter overflowed, sleep while holding the lock, like the Go version.
            let now_duration = now.duration_since(UNIX_EPOCH).unwrap();
            let nanos_in_ms = now_duration.as_nanos() % 1_000_000;
            let sleep_nanos = 1_000_000 - nanos_in_ms;
            thread::sleep(std::time::Duration::from_nanos(sleep_nanos as u64));
            // continue loop
        }

        Err("Max retries exceeded".to_string())
    }
}


/// Crockford Base32 character table
/// Excludes easily confused characters (I, L, O, U)
const CROCKFORD_BASE32_TABLE: [u8; 32] = [
    b'0', b'1', b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'a', b'b', b'c', b'd', b'e', b'f',
    b'g', b'h', b'j', b'k', b'm', b'n', b'p', b'q', b'r', b's', b't', b'v', b'w', b'x', b'y', b'z',
];

/// Encodes 8 bytes of data into a 13-character Crockford Base32 string.
fn crockford_base32_encode(data: &[u8]) -> String {
    if data.len() != 8 {
        panic!("data must be 8 bytes");
    }

    let value = u64::from_be_bytes(data.try_into().unwrap());
    let mut result = [0u8; 13];

    // Encode the first character from the top 4 bits (64 % 5 = 4)
    result[0] = CROCKFORD_BASE32_TABLE[((value >> 60) & 0x0F) as usize];

    // Encode the remaining 12 characters from the other 60 bits
    for i in 0..12 {
        let shift = 55 - (i * 5);
        result[i + 1] = CROCKFORD_BASE32_TABLE[((value >> shift) & 0x1F) as usize];
    }

    String::from_utf8(result.to_vec()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_tsid_generator_creation() {
        let generator = TsidGenerator::new(1);
        assert!(generator.is_ok());

        let generator = TsidGenerator::new(1023);
        assert!(generator.is_ok());

        let generator = TsidGenerator::new(1024); // Should fail (>= 1024)
        assert!(generator.is_err());
    }

    #[test]
    fn test_crockford_base32_encode() {
        // For 600001314817234, it should be "000h1parkr16j"
        let value: u64 = 600001314817234;
        let encoded = crockford_base32_encode(&value.to_be_bytes());
        assert_eq!(encoded, "000h1parkr16j");

        let data1 = [0, 0, 0, 0, 0, 0, 0, 1]; // Value 1
        let encoded1 = crockford_base32_encode(&data1);
        assert_eq!(encoded1, "0000000000001");

        let value2 = u64::MAX;
        let encoded2 = crockford_base32_encode(&value2.to_be_bytes());
        assert_eq!(encoded2, "fzzzzzzzzzzzz");
    }

    #[test]
    #[should_panic(expected = "data must be 8 bytes")]
    fn test_crockford_base32_encode_panic() {
        let data = [0, 0, 0, 0, 0, 0, 1]; // 7 bytes
        crockford_base32_encode(&data);
    }

    #[test]
    fn test_lexicographical_order_perf() {
        let generator = TsidGenerator::new(1).unwrap();
        let count = 2_000_000;

        let mut generated_ids = Vec::with_capacity(count);
        let mut id_map_for_uniqueness_check = HashSet::with_capacity(count);

        let start = std::time::Instant::now();
        for i in 0..count {
            let current_id = generator.generate().unwrap();

            if !id_map_for_uniqueness_check.insert(current_id.clone()) {
                panic!("Duplicate ID {} generated at iteration {}", current_id, i);
            }
            generated_ids.push(current_id);
        }
        println!(
            "Sync generator: {}M IDs generation + uniqueness check in {:?}",
            count / 1_000_000,
            start.elapsed()
        );

        let sort_start = std::time::Instant::now();
        let mut sorted_ids = generated_ids.clone();
        sorted_ids.sort_unstable();
        println!("Sorting {}M IDs in {:?}", count / 1_000_000, sort_start.elapsed());

        let check_start = std::time::Instant::now();
        for i in 0..count {
            if generated_ids[i] != sorted_ids[i] {
                panic!(
                    "Lexicographical order violated at index {}.\nGenerated: {}.\nExpected (Sorted): {}",
                    i, generated_ids[i], sorted_ids[i]
                );
            }
        }
        println!("Verifying order of {}M IDs in {:?}", count / 1_000_000, check_start.elapsed());
    }

    #[test]
    fn test_uniqueness_concurrent() {
        let generator = Arc::new(TsidGenerator::new(2).unwrap());
        let mut handles = vec![];
        let num_ids_per_thread = 100_000;
        let num_threads = 10;

        let start = std::time::Instant::now();
        for _ in 0..num_threads {
            let gen_clone = generator.clone();
            handles.push(thread::spawn(move || {
                let mut ids = Vec::with_capacity(num_ids_per_thread);
                for _ in 0..num_ids_per_thread {
                    ids.push(gen_clone.generate().unwrap());
                }
                ids
            }));
        }

        let mut all_ids = HashSet::new();
        for handle in handles {
            let ids = handle.join().unwrap();
            for id in ids {
                all_ids.insert(id);
            }
        }
        println!(
            "Sync generator ({} threads, {} total): finished in {:?}",
            num_threads,
            num_ids_per_thread * num_threads,
            start.elapsed()
        );
        assert_eq!(all_ids.len(), num_ids_per_thread * num_threads);
    }
}