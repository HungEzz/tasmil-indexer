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
use reqwest;

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
    pub sui_usd_volume: f64,
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
                sui_usd_volume: 0.0,
                last_update: SystemTime::now(),
            },
            last_processed_checkpoint: 0,
        }
    }
    
    /// Process a checkpoint and extract Cetus-related swap events
    /// 
    /// Returns a vector of newly found swap events
    pub async fn process_checkpoint(
        &mut self, 
        data: &CheckpointData, 
        pool: Option<&PgPool>
    ) -> Vec<SwapEvent> {
        let checkpoint_seq_number = data.checkpoint_summary.sequence_number;
        let mut new_swap_events = Vec::new();
        let checkpoint_timestamp = data.checkpoint_summary.timestamp();

        // Log basic checkpoint information
        info!("aaaaProcessing checkpoint {} with {} transactions", 
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

        // If there are new swap events, calculate volume with Pyth price
        if !new_swap_events.is_empty() {
            let volume = self.calculate_volume_24h().await;
            
            // Convert from microdollar to dollar for display
            let volume_in_dollars = volume / 1_000_000.0;
            
            info!("📊 Checkpoint {}: Found {} swap events | 24h Volume: ${:.2}", 
                 checkpoint_seq_number, new_swap_events.len(), volume_in_dollars);
        }

        // Update checkpoint tracking
        self.last_processed_checkpoint = checkpoint_seq_number;

        // Update database if available
        if let Some(pool) = pool {
            if checkpoint_seq_number % 10 == 0 || !new_swap_events.is_empty() {
                if let Err(err) = self.update_volume_24h_in_database(pool).await {
                    error!("Failed to update volume in database: {}", err);
                }
            }
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
    
    /// Calculate 24-hour volume in USD from stored swap events
    pub async fn calculate_volume_24h(&mut self) -> f64 {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now.checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
        
        // Pruning old events (older than 25 hours)
        let retention_cutoff = now
            .checked_sub(Duration::from_secs(24 * 60 * 60 + 1)) // Keep an extra hour for safety
            .unwrap_or(UNIX_EPOCH);
        
        let old_events_count = self.swap_events.len();
        self.swap_events.retain(|event| event.timestamp >= retention_cutoff);
        
        if old_events_count != self.swap_events.len() {
            debug!("Pruned {} old events, keeping {} events", 
                old_events_count - self.swap_events.len(), self.swap_events.len());
        }
        
        // Filter events from the last 24 hours
        let recent_events: Vec<&SwapEvent> = self.swap_events.iter()
            .filter(|event| event.timestamp >= twenty_four_hours_ago)
            .collect();
        
        // Get SUI price in USD
        let sui_price_usd = match get_sui_price_from_pyth().await {
            Ok(price) => price,
            Err(e) => {
                error!("Failed to get SUI price from Pyth: {}", e);
                // Use a default price or fetch from an alternative source
                // For now, we'll use a simple estimation based on USDC (1:1 with USD)
                1.0
            }
        };
        
        // Calculate USD volume - sửa lại phương pháp tính toán
        let mut sui_usd_volume: f64 = 0.0;
        
        for event in recent_events.iter() {
            // Bỏ qua các sự kiện có giá trị không hợp lệ
            if event.amount_in == 0 || event.amount_out == 0 {
                continue;
            }
            
            // Kiểm tra timestamp hợp lệ (không trong tương lai)
            if event.timestamp > now {
                error!("Skipping event with future timestamp: {:?}", event);
                continue;
            }
            
            // Tính toán khối lượng cho từng sự kiện riêng lẻ
            let sui_amount = if event.atob {
                // USDC → SUI: amount_out is SUI
                event.amount_out as f64 / 1_000_000_000.0 // Convert from 1e9 (SUI decimals)
            } else {
                // SUI → USDC: amount_in is SUI
                event.amount_in as f64 / 1_000_000_000.0 // Convert from 1e9 (SUI decimals)
            };
            
            // Tính giá trị USD cho lượng SUI của sự kiện này
            let event_value_in_usd = sui_amount * sui_price_usd * 1_000_000.0; // Convert to microdollars
            sui_usd_volume += event_value_in_usd;
        }
        
        // Update volume data
        self.volume_24h = VolumeData {
            sui_usd_volume,
            last_update: SystemTime::now(),
        };
        
        info!("Volume calculation: USD=${:.2}, SUI price=${:.4}", 
             sui_usd_volume / 1_000_000.0, sui_price_usd);
        
        sui_usd_volume
    }
    
    /// Update 24-hour volume in the database
    pub async fn update_volume_24h_in_database(&self, pool: &PgPool) -> Result<(), Error> {
        let now = SystemTime::now();
        let now_sql = chrono::DateTime::<chrono::Utc>::from(now).naive_utc();
        
        // Convert volume to string to avoid type mismatches with NUMERIC
        let sui_usd_volume_str = format!("{:.8}", self.volume_24h.sui_usd_volume);
        
        // Build and execute query with string parameters
        let query = format!(
            "INSERT INTO volume_data (period, sui_usd_volume, last_update, last_processed_checkpoint) 
             VALUES ('24h', '{}', $1, {})
             ON CONFLICT (period) 
             DO UPDATE SET sui_usd_volume = '{}', last_update = $1, last_processed_checkpoint = {}",
            sui_usd_volume_str,
            self.last_processed_checkpoint,
            sui_usd_volume_str,
            self.last_processed_checkpoint
        );
        
        sqlx::query(&query)
            .bind(now_sql)
            .execute(pool)
            .await?;
        
        debug!("Updated 24h volume in database: USD=${:.2}", self.volume_24h.sui_usd_volume / 1_000_000.0);
        
        Ok(())
    }
    
    /// Retrieve 24-hour volume data from the database
    pub async fn get_volume_24h_from_database(pool: &PgPool) -> Result<VolumeData, Error> {
        let row = sqlx::query("SELECT sui_usd_volume, last_update FROM volume_data WHERE period = '24h'")
            .fetch_optional(pool)
            .await?;
        
        if let Some(row) = row {
            // Get values as Decimal strings first, then parse them to f64
            let sui_usd_volume_str: String = row.try_get("sui_usd_volume")?;
            
            // Convert from string to f64
            let sui_usd_volume = sui_usd_volume_str.parse::<f64>().unwrap_or(0.0);
            
            let last_update: chrono::NaiveDateTime = row.get("last_update");
            
            Ok(VolumeData {
                sui_usd_volume,
                last_update: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    last_update, 
                    chrono::Utc
                ).into(),
            })
        } else {
            Ok(VolumeData {
                sui_usd_volume: 0.0,
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
}

/// Background job to periodically update volume data
pub async fn start_volume_update_job(indexer: Arc<Mutex<CetusIndexer>>, pool: PgPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 10)); // Every 10 minutes
    
    loop {
        interval.tick().await;
        
        info!("🔄 Running scheduled volume update");
        let mut indexer_locked = indexer.lock().await;
        
        // Calculate new volume and update database
        let volume = indexer_locked.calculate_volume_24h().await;
        let volume_in_dollars = volume / 1_000_000.0;

        if let Err(err) = indexer_locked.update_volume_24h_in_database(&pool).await {
            error!("❌ Scheduled volume update failed: {}", err);
        } else {
            info!("✅ Updated 24h Volume: ${:.2}", volume_in_dollars);
        }
    }
}

/// Create the volume_data table in the database if it doesn't exist
pub async fn create_volume_data_table(pool: &PgPool) -> Result<(), Error> {
    // Drop the existing table if it exists
    sqlx::query!("DROP TABLE IF EXISTS volume_data")
        .execute(pool)
        .await?;
    
    // Create a new table with only the required fields
    sqlx::query!(
        "CREATE TABLE volume_data (
            id SERIAL PRIMARY KEY,
            period VARCHAR(50) NOT NULL UNIQUE,
            sui_usd_volume NUMERIC(30, 8) NOT NULL DEFAULT 0,
            last_update TIMESTAMP NOT NULL,
            last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
        )"
    )
    .execute(pool)
    .await?;
    
    info!("📦 Volume data table recreated with only sui_usd_volume field");
    
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

/// Get SUI price in USD from Pyth Network
async fn get_sui_price_from_pyth() -> Result<f64, Error> {
    // Create a reqwest client to interact with the Pyth Network API
    let client = reqwest::Client::new();

    // SUI/USD price feed ID (without 0x prefix)
    let price_id = "23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744";

    // Fetch the price feed using the REST API
    let url = format!("https://hermes.pyth.network/api/latest_price_feeds?ids[]={}", price_id);
    let response = client.get(&url).send().await?;
    let price_feed_data: serde_json::Value = response.json().await?;
    
    debug!("Got price feed data from Pyth Network");
    
    // Parse the price from the response
    if let Some(price_data) = price_feed_data.as_array().and_then(|arr| arr.first()) {
        if let Some(price_obj) = price_data.get("price") {
            // Try to parse the price values
            let price = price_obj["price"].as_str()
                .ok_or_else(|| anyhow::anyhow!("Price value not found or not a string"))?
                .parse::<i64>()?;
                
            let expo = price_obj["expo"].as_i64()
                .ok_or_else(|| anyhow::anyhow!("Expo value not found"))?;
            
            // Calculate the actual price value with exponent
            let sui_price_usd = (price as f64) * 10f64.powi(expo as i32);

            info!("SUI/USD Price: ${:.4}", sui_price_usd);
            return Ok(sui_price_usd);
        }
    }
    
    Err(anyhow::anyhow!("Failed to parse price feed data from Pyth Network"))
}

