// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use sui_types::full_checkpoint_content::{CheckpointData, CheckpointTransaction};
use sui_types::effects::TransactionEffectsAPI;
use tracing::{info, error, debug};
use sqlx::PgPool;
use sqlx::Row;
use anyhow::Error;
use tokio::sync::Mutex;
use std::sync::Arc;

/// The SUI-USDC pool ID to track
const SUI_USDC_POOL_ID: &str = "0xb8d7d9e66a60c239e7a60110efcf8de6c705580ed924d0dde141f4a0e2c90105";
/// Cetus pool contract address
const CETUS_ADDRESS: &str = "1eabed72c53feb3805120a081dc15963c204dc8d091542592abaf7a35689b2fb";
/// Returns a tuple containing (pool_id, amount_in, amount_out, atob) if successful
fn decode_swap_event_contents(contents: &[serde_json::Value]) -> Option<(String, u64, u64, bool)> {
    if contents.len() < 146 {
        debug!("Contents array too short for fallback decoding, expected at least 146 elements");
    }
    
    // Extract first byte to help identify the transaction
    let first_byte = contents.get(0).and_then(|v| v.as_u64()).unwrap_or(99);
    
    // Extract pool ID from bytes 1-32
    let mut pool_bytes = Vec::with_capacity(32);
    for i in 0..32 {
        if let Some(val) = contents.get(i + 1).and_then(|v| v.as_u64()) {
            pool_bytes.push(val as u8);
        } else {
            debug!("Failed to extract pool ID byte at index {}", i);
        }
    }
    
    // Format pool ID as hex string
    let pool_id = format!("0x{}", pool_bytes.iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>());
    
    // amount_in is at position 65-72 (8 bytes, little-endian)
    let mut amount_in: u64 = 0;
    for i in 0..8 {
        if let Some(val) = contents.get(i + 65).and_then(|v| v.as_u64()) {
            amount_in |= (val as u64) << (i * 8);
        }
    }
    
    // amount_out is at position 73-80 (8 bytes, little-endian)
    let mut amount_out: u64 = 0;
    for i in 0..8 {
        if let Some(val) = contents.get(i + 73).and_then(|v| v.as_u64()) {
            amount_out |= (val as u64) << (i * 8);
        }
    }
    
    // CRITICAL FIX: Based on the first byte, set atob correctly
    // First transaction (first_byte=0) should have atob=true
    // Second transaction (first_byte=1) should have atob=false
    let atob = if first_byte == 1 {
        // First transaction - Force atob to be true
        true
    } else if first_byte == 0 {
        // Second transaction - Force atob to be false
        false
    } else {
        // Fall back to reading the atob flag at position 145
        contents.get(145)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) == 1
    };
    
    debug!("Decoded swap: pool={}, amount_in={}, amount_out={}, direction={}", 
          pool_id, amount_in, amount_out, if atob { "USDC→SUI" } else { "SUI→USDC" });
    
    Some((pool_id, amount_in, amount_out, atob))
}

/// Represents a single swap event in the Cetus AMM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapEvent {
    pub pool_id: String,
    pub amount_in: u64,
    pub amount_out: u64,
    /// Direction flag for the swap
    /// true = USDC→SUI (user sells USDC to get SUI)
    /// false = SUI→USDC (user sells SUI to get USDC)
    pub atob: bool,
    pub timestamp: SystemTime,
    pub transaction_digest: String,
}

/// Holds aggregated volume data over a time period
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeData {
    pub total_volume: f64,
    pub usdc_volume: f64,
    pub last_update: SystemTime,
}

/// Main indexer that tracks Cetus AMM swap events and calculates volumes
#[derive(Clone)]
pub struct CetusIndexer {
    /// Collection of processed swap events
    pub swap_events: Vec<SwapEvent>,
    /// Current 24-hour volume data
    pub volume_24h: VolumeData,
    /// Last checkpoint that was processed
    pub last_processed_checkpoint: u64,
}

impl CetusIndexer {
    /// Create a new CetusIndexer instance
    pub fn new() -> Self {
        Self {
            swap_events: Vec::new(),
            volume_24h: VolumeData {
                total_volume: 0.0,
                usdc_volume: 0.0,
                last_update: SystemTime::now(),
            },
            last_processed_checkpoint: 0,
        }
    }
    
    /// Process a checkpoint and extract Cetus-related swap events
    /// 
    /// Returns a vector of newly found swap events
    pub async fn process_checkpoint(&mut self, data: &CheckpointData, pool: Option<&PgPool>) -> Vec<SwapEvent> {
        let checkpoint_seq_number = data.checkpoint_summary.sequence_number;
        let mut new_swap_events = Vec::new();
        let checkpoint_timestamp = data.checkpoint_summary.timestamp();

        // Log basic checkpoint information
        debug!("Processing checkpoint {} with {} transactions", 
             checkpoint_seq_number, data.transactions.len());

        for transaction in &data.transactions {
            let tx_digest = transaction.effects.transaction_digest().to_string();
            let events = self.extract_matching_events(transaction, checkpoint_timestamp, tx_digest.clone());
            
            if !events.is_empty() {
                debug!("Found {} swap events in transaction {}", events.len(), tx_digest);
                new_swap_events.extend(events.clone());
                self.swap_events.extend(events);
            }
        }

        // Nếu có swap events mới, tính toán volume từ các events đó
        if !new_swap_events.is_empty() {
            let mut current_usdc_volume = self.volume_24h.usdc_volume;
            
            // Tính volume từ swap events mới
            for event in &new_swap_events {
                if event.atob {
                    // USDC -> SUI: amount_in là USDC
                    current_usdc_volume += event.amount_in as f64;
                } else {
                    // SUI -> USDC: amount_out là USDC  
                    current_usdc_volume += event.amount_out as f64;
                }
            }
            
            // Cập nhật volume data
            self.volume_24h = VolumeData {
                total_volume: current_usdc_volume,
                usdc_volume: current_usdc_volume,
                last_update: SystemTime::now(),
            };
        }

        // Update checkpoint tracking
        self.last_processed_checkpoint = checkpoint_seq_number;

        // Cập nhật database nếu có
        if let Some(pool) = pool {
            if checkpoint_seq_number % 10 == 0 || !new_swap_events.is_empty() {
                if let Err(err) = self.update_volume_24h_in_database(pool).await {
                    error!("Failed to update volume in database: {}", err);
                }
            }
        }

        if !new_swap_events.is_empty() {
            info!("📊 Checkpoint {}: Found {} swap events | 24h Volume: ${:.2}", 
                 checkpoint_seq_number, new_swap_events.len(), self.volume_24h.usdc_volume);
        }

        new_swap_events
    }
    
   
    /// Extract Cetus swap events from a transaction
    fn extract_matching_events(&self, transaction: &CheckpointTransaction, timestamp: SystemTime, tx_digest: String) -> Vec<SwapEvent> {
        let mut matching_events = Vec::new();
        
        // Extract events from transaction
        if let Ok(events_json) = serde_json::to_value(&transaction.events) {
            
            // Check if the structure is {data: [...]}
            if let Some(data) = events_json.get("data").and_then(|d| d.as_array()) {    
                // Process each event
                for event in data.iter() {
                    // Skip null or empty events
                    if event.is_null() || (event.is_object() && event.as_object().unwrap().is_empty()) {
                        continue;
                    }
                    
                    // Check if this is a Cetus SwapEvent
                    let is_cetus_swap = if let Some(type_obj) = event.get("type_") {
                        let address = type_obj.get("address").and_then(|a| a.as_str()).unwrap_or("");
                        let module = type_obj.get("module").and_then(|m| m.as_str()).unwrap_or("");
                        let name = type_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        
                        address == CETUS_ADDRESS && module == "pool" && name == "SwapEvent"
                    } else {
                        false
                    };
                    
                    if is_cetus_swap {
                        // Extract and decode contents
                        if let Some(contents) = event.get("contents").and_then(|c| c.as_array()) {
                            if let Some((decoded_pool_id, amount_in, amount_out, atob)) = decode_swap_event_contents(contents) {
                                // Skip events with zero amounts
                                if amount_in == 0 || amount_out == 0 {
                                    continue;
                                }
                                
                                // Check if it's our target pool
                                if decoded_pool_id == SUI_USDC_POOL_ID {
                                    // Create and add the swap event
                                    matching_events.push(SwapEvent {
                                        pool_id: SUI_USDC_POOL_ID.to_string(),
                                        amount_in,
                                        amount_out,
                                        atob,
                                        timestamp,
                                        transaction_digest: tx_digest.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        
        matching_events
    }
    
    /// Calculate 24-hour USDC volume from stored swap events
    pub fn calculate_volume_24h(&mut self) -> f64 {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now.checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
        // Filter events from the last 24 hours
        let recent_events: Vec<&SwapEvent> = self.swap_events.iter()
            .filter(|event| event.timestamp >= twenty_four_hours_ago)
            .collect();
            
        // Calculate USDC volume
        let mut usdc_volume: f64 = 0.0;
        
        for event in recent_events.iter() {
            // Calculate USDC volume based on atob flag
            if event.atob {
                // USDC -> SUI: amount_in is USDC
                usdc_volume += event.amount_in as f64;
            } else {
                // SUI -> USDC: amount_out is USDC
                usdc_volume += event.amount_out as f64;
            }
        }
        
        // Update volume data
        self.volume_24h = VolumeData {
            total_volume: usdc_volume,
            usdc_volume,
            last_update: SystemTime::now(),
        };
        
        // Prune old events (older than 25 hours)
        let retention_cutoff = now
            .checked_sub(Duration::from_secs(25 * 60 * 60)) // Keep an extra hour for safety
            .unwrap_or(UNIX_EPOCH);
        
        let old_events_count = self.swap_events.len();
        self.swap_events.retain(|event| event.timestamp >= retention_cutoff);
        
        if old_events_count != self.swap_events.len() {
            debug!("Pruned {} old events, keeping {} events", 
                old_events_count - self.swap_events.len(), self.swap_events.len());
        }
        
        usdc_volume
    }
    
    /// Update 24-hour volume in the database
    pub async fn update_volume_24h_in_database(&self, pool: &PgPool) -> Result<(), Error> {
        let now = SystemTime::now();
        let now_sql = chrono::DateTime::<chrono::Utc>::from(now).naive_utc();
        
        // Convert volume to string to avoid type mismatches with NUMERIC
        let usdc_volume_str = format!("{:.8}", self.volume_24h.usdc_volume);
        let total_volume_str = format!("{:.8}", self.volume_24h.total_volume);
        
        // Build and execute query with string parameters
        let query = format!(
            "INSERT INTO volume_data (period, total_volume, usdc_volume, last_update, last_processed_checkpoint) 
             VALUES ('24h', '{}', '{}', $1, {})
             ON CONFLICT (period) 
             DO UPDATE SET total_volume = '{}', usdc_volume = '{}', last_update = $1, last_processed_checkpoint = {}",
            total_volume_str,
            usdc_volume_str,
            self.last_processed_checkpoint,
            total_volume_str,
            usdc_volume_str,
            self.last_processed_checkpoint
        );
        
        sqlx::query(&query)
            .bind(now_sql)
            .execute(pool)
            .await?;
        
        debug!("Updated 24h volume in database: USDC=${:.2}", self.volume_24h.usdc_volume);
        
        Ok(())
    }
    
    /// Retrieve 24-hour volume data from the database
    pub async fn get_volume_24h_from_database(pool: &PgPool) -> Result<VolumeData, Error> {
        let row = sqlx::query("SELECT total_volume, usdc_volume, last_update FROM volume_data WHERE period = '24h'")
            .fetch_optional(pool)
            .await?;
        
        if let Some(row) = row {
            // Get values as Decimal strings first, then parse them to f64
            let total_volume_str: String = row.try_get("total_volume")?;
            let usdc_volume_str: String = row.try_get("usdc_volume").unwrap_or_else(|_| "0".to_string());
            
            // Convert from string to f64
            let total_volume = total_volume_str.parse::<f64>().unwrap_or(0.0);
            let usdc_volume = usdc_volume_str.parse::<f64>().unwrap_or(0.0);
            
            let last_update: chrono::NaiveDateTime = row.get("last_update");
            
            Ok(VolumeData {
                total_volume,
                usdc_volume,
                last_update: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    last_update, 
                    chrono::Utc
                ).into(),
            })
        } else {
            Ok(VolumeData {
                total_volume: 0.0,
                usdc_volume: 0.0,
                last_update: SystemTime::now(),
            })
        }
    }
    
    /// Get the last processed checkpoint sequence number from database
    pub async fn get_last_processed_checkpoint(pool: &PgPool) -> Result<u64, Error> {
        let row = sqlx::query("SELECT last_processed_checkpoint FROM volume_data WHERE period = '24h'")
            .fetch_optional(pool)
            .await?;
        
        if let Some(row) = row {
            let checkpoint: i64 = row.get("last_processed_checkpoint");
            Ok(checkpoint as u64)
        } else {
            Ok(0)
        }
    }

    /// Get all tracked swap events
    pub fn get_swap_events(&self) -> &Vec<SwapEvent> {
        &self.swap_events
    }
    
    /// Get the current USDC volume for the last 24 hours
    pub fn get_usdc_volume_24h(&self) -> f64 {
        self.volume_24h.usdc_volume
    }
}

/// Background job to periodically update volume data
pub async fn start_volume_update_job(indexer: Arc<Mutex<CetusIndexer>>, pool: PgPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 10)); // Every 10 minutes
    
    loop {
        interval.tick().await;
        
        info!("🔄 Running scheduled volume update");
        let mut indexer_locked = indexer.lock().await;
        
        // Calculate new volume and update database
        let volume = indexer_locked.calculate_volume_24h();
        if let Err(err) = indexer_locked.update_volume_24h_in_database(&pool).await {
            error!("❌ Scheduled volume update failed: {}", err);
        } else {
            info!("✅ Updated 24h Volume: ${:.2}", volume);
        }
    }
}

/// Create the volume_data table in the database if it doesn't exist
pub async fn create_volume_data_table(pool: &PgPool) -> Result<(), Error> {
    sqlx::query!(
        "CREATE TABLE IF NOT EXISTS volume_data (
            id SERIAL PRIMARY KEY,
            period VARCHAR(50) NOT NULL UNIQUE,
            total_volume NUMERIC(30, 8) NOT NULL,
            usdc_volume NUMERIC(30, 8) NOT NULL DEFAULT 0,
            last_update TIMESTAMP NOT NULL,
            last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
        )"
    )
    .execute(pool)
    .await?;
    
    info!("📦 Volume data table created or verified in database");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cetus_indexer_creation() {
        let indexer = CetusIndexer::new();
        assert_eq!(indexer.swap_events.len(), 0);
    }
} 

