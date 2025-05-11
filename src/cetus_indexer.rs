// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use sui_types::full_checkpoint_content::{CheckpointData, CheckpointTransaction};
use sui_types::object::Object;
use sui_types::effects::TransactionEffectsAPI;
use tracing::{info, error};
use sqlx::PgPool;
use sqlx::Row;
use anyhow::Error;
use tokio::sync::Mutex;
use std::sync::Arc;

/// The SUI-USDC pool ID
const SUI_USDC_POOL_ID: &str = "0xb8d7d9e66a60c239e7a60110efcf8de6c705580ed924d0dde141f4a0e2c90105";

/// Helper function to extract a u64 value from an array of JSON numbers
/// Assumes little-endian encoding (least significant byte first)
fn extract_u64_from_array(array: &[serde_json::Value], start_index: usize) -> Option<u64> {
    if start_index + 8 > array.len() {
        return None; // Not enough elements
    }
    
    let mut result: u64 = 0;
    for i in 0..8 {
        // Extract byte value, default to 0 if not a number
        let byte_value = array.get(start_index + i)
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u8;
            
        // Shift into position (little-endian)
        result |= (byte_value as u64) << (i * 8);
    }
    
    Some(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CetusTransactionData {
    pub transaction_digest: String,
    pub checkpoint_sequence_number: u64,
    pub transaction_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapEvent {
    pub pool_id: String,
    pub amount_in: u64,
    pub amount_out: u64,
    pub atob: bool,
    pub by_amount_in: bool,
    pub partner_id: String,
    pub coin_a: String,
    pub coin_b: String,
    pub timestamp: SystemTime,
    pub transaction_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeData {
    pub total_volume: f64,
    pub usdc_volume: f64,
    pub sui_usdc_pool_volume: f64,
    pub last_update: SystemTime,
}

#[derive(Clone)]
pub struct CetusIndexer {
    // Map to store accumulated data
    pub transaction_data: HashMap<String, CetusTransactionData>,
    pub swap_events: Vec<SwapEvent>,
    pub volume_24h: VolumeData,
    pub last_processed_checkpoint: u64,
}

impl CetusIndexer {
    pub fn new() -> Self {
        Self {
            transaction_data: HashMap::new(),
            swap_events: Vec::new(),
            volume_24h: VolumeData {
                total_volume: 0.0,
                usdc_volume: 0.0,
                sui_usdc_pool_volume: 0.0,
                last_update: SystemTime::now(),
            },
            last_processed_checkpoint: 0,
        }
    }
    
    /// Extract swap events from a transaction
    fn extract_swap_events(&self, transaction: &CheckpointTransaction, timestamp: SystemTime, tx_digest: String) -> Vec<SwapEvent> {
        let mut swap_events = Vec::new();
        
        // Log transaction digest
        info!("Extracting swap events from transaction: {}", tx_digest);
        // Extract events directly from the transaction
        if let Ok(all_events_json) = serde_json::to_value(&transaction.events) {
            // Check if the structure is {data: [...]}
            if let Some(data) = all_events_json.get("data") {
                if let Some(events_array) = data.as_array() {
                    
                    for (event_idx, event_json) in events_array.iter().enumerate() {
                        info!("Event #{}: {}", event_idx, event_json);
                        // Try to extract pool_id directly from the event contents
                        if let Some(contents) = event_json.get("contents") {
                            info!("Contents: {:?}", contents);
                            // contents is an array (binary data that might need decoding)
                             if let Some(contents_array) = contents.as_array() { 
                                info!("Contents array: {:?}", contents_array);
                                // Check if this is a meaningful array (might be binary data)
                                if contents_array.len() >= 16 {  // Assuming we need at least 2 values (amount_in, amount_out)
                                    // These might be encoded values for amount_in and amount_out
                                    if let (Some(amount_in), Some(amount_out)) = (
                                        extract_u64_from_array(contents_array, 0),
                                        extract_u64_from_array(contents_array, 8)
                                    ) {
                                        info!("Possible swap values extracted: amount_in={}, amount_out={}", amount_in, amount_out);
                                        
                                        // Check event's type to determine if it's swap-related
                                        if let Some(type_info) = event_json.get("type_") {
                                            let type_str = serde_json::to_string(type_info).unwrap_or_default();
                                            info!("Event type: {}", type_str);
                                            
                                            // If it's a swap event or related to Cetus
                                            if type_str.contains("SwapEvent") || type_str.contains("cetus") || type_str.contains("pool") {
                                                info!("💰 BINARY DATA SWAP DETECTED");
                                                info!("🔄 Transaction: {}", tx_digest);
                                                
                                                // The pool ID could be from other fields or assume SUI_USDC_POOL_ID
                                                // We need to find a better way to extract pool ID, but for now assume it's for our target pool
                                                let event_pool_id = SUI_USDC_POOL_ID;
                                                
                                                // Basic checks to validate the amounts
                                                if amount_in > 0 && amount_out > 0 {
                                                    info!("🏦 Assuming Pool ID: {}", event_pool_id);
                                                    info!("💱 Binary data - Amount in: {}, Amount out: {}", amount_in, amount_out);
                                                    
                                                    // Default values for fields we can't determine
                                                    let atob = true;  // Default direction
                                                    let by_amount_in = true;
                                                    let partner_id = "unknown".to_string();
                                                    
                                                    // For SUI-USDC pool we know the coins, but direction is uncertain
                                                    let coin_a = "usdc::USDC".to_string();
                                                    let coin_b = "sui::SUI".to_string();
                                                    
                                                    swap_events.push(SwapEvent {
                                                        pool_id: event_pool_id.to_string(),
                                                        amount_in,
                                                        amount_out,
                                                        atob,
                                                        by_amount_in,
                                                        partner_id,
                                                        coin_a,
                                                        coin_b,
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
                    }
                } else {
                    info!("Events data is not an array, but: {}", data);
                }
            } 
        } else {
            info!("❌ Failed to serialize transaction events to JSON");
        }
        
        info!("Extracted {} swap events from transaction {}", swap_events.len(), tx_digest);
        swap_events
    }
    
    // /// Enrich swap events with pool information (token_a and token_b)
    // fn enrich_swap_events_with_pool_info(&self, swap_events: &mut Vec<SwapEvent>, transaction: &CheckpointTransaction) {
    //     // Look for Pool objects in the transaction output
    //     for object in &transaction.output_objects {
    //         if let Some(struct_tag) = object.struct_tag() {
    //             if struct_tag.to_string().contains("pool::Pool") {
    //                 // This is a pool object, extract its ID and token types
    //                 let pool_id = object.id().to_string();
                    
    //                 // Extract token types from struct_tag
    //                 // The format should be like: 0x...::pool::Pool<0x...::coin::COIN_A, 0x...::coin::COIN_B>
    //                 let type_params = struct_tag.type_params.clone();
    //                 if type_params.len() >= 2 {
    //                     let token_a = type_params[0].to_string();
    //                     let token_b = type_params[1].to_string();
                        
    //                     // Update coin_a and coin_b for any matching pool IDs if they're still unknown
    //                     for event in swap_events.iter_mut() {
    //                         if event.pool_id == pool_id && (event.coin_a == "unknown" || event.coin_b == "unknown") {
    //                             event.coin_a = token_a.clone();
    //                             event.coin_b = token_b.clone();
    //                         }
    //                     }
    //                 }
    //             }
    //         }
    //     }
    // }
    
    /// Calculate 24h volume with specific focus on USDC
    pub fn calculate_volume_24h(&mut self) -> (f64, f64, f64) {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now.checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
            
        // Filter events from the last 24 hours
        let recent_events: Vec<&SwapEvent> = self.swap_events.iter()
            .filter(|event| event.timestamp >= twenty_four_hours_ago)
            .collect();
            
        // Calculate total volume (all tokens)
        let mut total_volume: f64 = 0.0;
        
        // Calculate USDC volume specifically
        let mut usdc_volume: f64 = 0.0;
        
        // USDC type identifier - this would need to be configured correctly
        const USDC_TYPE_IDENTIFIER: &str = "usdc::USDC";
        
        info!("📊📊📊 Starting 24h volume calculation with {} events", recent_events.len());
        for event in recent_events.iter() {
            // Add to total volume (in native units)
            let event_total = (event.amount_in as f64) + (event.amount_out as f64);
            total_volume += event_total;
            
            // Check if this swap involves USDC
            let token_in;
            let token_out;
            
            if event.atob {
                // A→B direction
                token_in = &event.coin_a;
                token_out = &event.coin_b;
            } else {
                // B→A direction
                token_in = &event.coin_b;
                token_out = &event.coin_a;
            }
            
            // Add to USDC volume if applicable
            if token_in.contains(USDC_TYPE_IDENTIFIER) {
                usdc_volume += event.amount_in as f64;
            }
            
            if token_out.contains(USDC_TYPE_IDENTIFIER) {
                usdc_volume += event.amount_out as f64;
            }
        }
        
        // Calculate SUI-USDC pool volume
        let sui_usdc_pool_volume = self.calculate_sui_usdc_pool_volume_24h();
        
        // Update volume data
        self.volume_24h = VolumeData {
            total_volume,
            usdc_volume,
            sui_usdc_pool_volume,
            last_update: now,
        };
        
        // Log results
        let recent_events_count = recent_events.len();
        info!("📊 24h Volume Summary:");
        info!("📊 Total Events: {}", recent_events_count);
        info!("📊 Total Volume: {:.2}", total_volume);
        info!("📊 USDC Volume: {:.2}", usdc_volume);
        info!("📊 SUI-USDC Pool Volume: {:.2}", sui_usdc_pool_volume);
        
        // Prune old events
        let retention_cutoff = now
            .checked_sub(Duration::from_secs(25 * 60 * 60)) // Keep an extra hour for safety
            .unwrap_or(UNIX_EPOCH);
        
        let old_events_count = self.swap_events.len();
        self.swap_events.retain(|event| event.timestamp >= retention_cutoff);
        info!("📊 Pruned {} old events, keeping {} events", old_events_count - self.swap_events.len(), self.swap_events.len());
        
        (total_volume, usdc_volume, sui_usdc_pool_volume)
    }
    
    /// Calculate 24h volume for specific SUI-USDC pool
    pub fn calculate_sui_usdc_pool_volume_24h(&self) -> f64 {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now.checked_sub(Duration::from_secs(24 * 60 * 60))
            .unwrap_or(UNIX_EPOCH);
            
        // Filter events from the last 24 hours for specific pool
        let recent_pool_events: Vec<&SwapEvent> = self.swap_events.iter()
            .filter(|event| event.timestamp >= twenty_four_hours_ago && event.pool_id == SUI_USDC_POOL_ID)
            .collect();
            
        // Calculate volume for SUI-USDC pool
        let mut sui_usdc_volume: f64 = 0.0;
        
        // Type identifiers for SUI and USDC
        const SUI_TYPE_IDENTIFIER: &str = "sui::SUI";
        const USDC_TYPE_IDENTIFIER: &str = "usdc::USDC";
        
        info!("📊 Calculating SUI-USDC pool volume for pool: {} (found {} events)", SUI_USDC_POOL_ID, recent_pool_events.len());
        
        for event in recent_pool_events.iter() {
            // Determine which tokens are involved
            let is_sui_a = event.coin_a.contains(SUI_TYPE_IDENTIFIER);
            let is_sui_b = event.coin_b.contains(SUI_TYPE_IDENTIFIER);
            let is_usdc_a = event.coin_a.contains(USDC_TYPE_IDENTIFIER);
            let is_usdc_b = event.coin_b.contains(USDC_TYPE_IDENTIFIER);
            
            // Check if this is a SUI-USDC swap
            if (is_sui_a && is_usdc_b) || (is_usdc_a && is_sui_b) {
                // Determine the direction and token amounts
                if event.atob {
                    // A→B direction
                    if is_usdc_a {
                        // USDC→SUI: add USDC amount
                        sui_usdc_volume += event.amount_in as f64;
                    } else if is_usdc_b {
                        // SUI→USDC: add USDC amount
                        sui_usdc_volume += event.amount_out as f64;
                    }
                } else {
                    // B→A direction
                    if is_usdc_b {
                        // USDC→SUI: add USDC amount
                        sui_usdc_volume += event.amount_in as f64;
                    } else if is_usdc_a {
                        // SUI→USDC: add USDC amount
                        sui_usdc_volume += event.amount_out as f64;
                    }
                }
            }
        }
        
        info!("📊 24h SUI-USDC Pool Volume: {:.2} USDC (from {} events)", 
            sui_usdc_volume, recent_pool_events.len());
        
        sui_usdc_volume
    }
    
    /// Update database schema to include USDC volume
    pub async fn update_volume_24h_in_database(&self, pool: &PgPool) -> Result<(), Error> {
        let now = SystemTime::now();
        
        // Calculate SUI-USDC pool volume
        let sui_usdc_pool_volume = self.calculate_sui_usdc_pool_volume_24h();
        
        // Convert SystemTime to SQL DateTime
        let now_sql = chrono::DateTime::<chrono::Utc>::from(now).naive_utc();
        
        // Use query_as instead of query! macro to avoid compile-time checking
        let query = format!(
            "INSERT INTO volume_data (period, total_volume, usdc_volume, sui_usdc_pool_volume, last_update, last_processed_checkpoint) 
             VALUES ('24h', {}, {}, {}, '{}', {})
             ON CONFLICT (period) 
             DO UPDATE SET total_volume = {}, usdc_volume = {}, sui_usdc_pool_volume = {}, last_update = '{}', last_processed_checkpoint = {}",
            self.volume_24h.total_volume,
            self.volume_24h.usdc_volume,
            sui_usdc_pool_volume,
            now_sql,
            self.last_processed_checkpoint,
            self.volume_24h.total_volume,
            self.volume_24h.usdc_volume,
            sui_usdc_pool_volume,
            now_sql,
            self.last_processed_checkpoint
        );
        
        sqlx::query(&query)
            .execute(pool)
            .await?;
        
        info!("📊 Updated 24h volumes in database: Total=${:.2}, USDC=${:.2}, SUI-USDC Pool=${:.2}", 
              self.volume_24h.total_volume, 
              self.volume_24h.usdc_volume,
              sui_usdc_pool_volume);
        
        Ok(())
    }
    
    /// Get volume data from database
    pub async fn get_volume_24h_from_database(pool: &PgPool) -> Result<VolumeData, Error> {
        // Use query instead of query! macro to avoid compile-time checking
        let row = sqlx::query("SELECT total_volume, usdc_volume, sui_usdc_pool_volume, last_update FROM volume_data WHERE period = '24h'")
            .fetch_optional(pool)
            .await?;
        
        if let Some(row) = row {
            let total_volume: f64 = row.get("total_volume");
            let usdc_volume: f64 = row.try_get("usdc_volume").unwrap_or(0.0);
            let sui_usdc_pool_volume: f64 = row.try_get("sui_usdc_pool_volume").unwrap_or(0.0);
            let last_update: chrono::NaiveDateTime = row.get("last_update");
            
            Ok(VolumeData {
                total_volume,
                usdc_volume,
                sui_usdc_pool_volume,
                last_update: chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(
                    last_update, 
                    chrono::Utc
                ).into(),
            })
        } else {
            Ok(VolumeData {
                total_volume: 0.0,
                usdc_volume: 0.0,
                sui_usdc_pool_volume: 0.0,
                last_update: SystemTime::now(),
            })
        }
    }
    
    /// Đọc last_processed_checkpoint từ database
    pub async fn get_last_processed_checkpoint(pool: &PgPool) -> Result<u64, Error> {
        // Sử dụng query thay vì query! macro để tránh check compile-time
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

    /// Check if a transaction is Cetus-related
    fn is_cetus_transaction(&self, transaction: &CheckpointTransaction) -> bool {
        // Get transaction digest for logging if needed
        let _tx_digest = transaction.effects.transaction_digest();
        
        // Check if any events are related to our target pool
        if let Ok(all_events_json) = serde_json::to_value(&transaction.events) {
            // Check if the structure is {data: [...]}
            if let Some(data) = all_events_json.get("data") {
                if let Some(events_array) = data.as_array() {
                    for event in events_array {
                        // Check for pool_id in event contents
                        if let Some(contents) = event.get("contents") {
                            if let Some(fields) = contents.as_object() {
                                if let Some(pool_id) = fields.get("pool").and_then(|p| p.as_str()) {
                                    if pool_id == SUI_USDC_POOL_ID {
                                        info!("✅ Found event with target pool ID: {}", pool_id);
                                        return true;
                                    }
                                }
                            } else if let Some(contents_array) = contents.as_array() {
                                // If contents is an array, convert to string and check if it contains pool ID
                                let contents_str = serde_json::to_string(contents_array).unwrap_or_default();
                                if contents_str.contains(SUI_USDC_POOL_ID) {
                                    info!("✅ Found target pool ID in event contents array: {}", SUI_USDC_POOL_ID);
                                    return true;
                                }
                                
                                // Check if this might be a swap event based on type
                                if let Some(type_field) = event.get("type_") {
                                    let type_str = serde_json::to_string(type_field).unwrap_or_default();
                                    if (type_str.contains("SwapEvent") || type_str.contains("cetus")) &&
                                       contents_array.len() >= 8 {
                                        info!("✅ Found potential Cetus swap event with binary data");
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            } else if let Some(events_array) = all_events_json.as_array() {
                // Try direct approach if there's no "data" field
                for event in events_array {
                    // Check for pool_id in event contents
                    if let Some(contents) = event.get("contents") {
                        if let Some(fields) = contents.as_object() {
                            if let Some(pool_id) = fields.get("pool").and_then(|p| p.as_str()) {
                                if pool_id == SUI_USDC_POOL_ID {
                                    info!("✅ Found event with target pool ID: {}", pool_id);
                                    return true;
                                }
                            }
                        } else if let Some(contents_array) = contents.as_array() {
                            // If contents is an array, convert to string and check if it contains pool ID
                            let contents_str = serde_json::to_string(contents_array).unwrap_or_default();
                            if contents_str.contains(SUI_USDC_POOL_ID) {
                                info!("✅ Found target pool ID in event contents array: {}", SUI_USDC_POOL_ID);
                                return true;
                            }
                            
                            // Check if this might be a swap event based on type
                            if let Some(type_field) = event.get("type_") {
                                let type_str = serde_json::to_string(type_field).unwrap_or_default();
                                if (type_str.contains("SwapEvent") || type_str.contains("cetus")) &&
                                   contents_array.len() >= 8 {
                                    info!("✅ Found potential Cetus swap event with binary data");
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }

        // Serialize to JSON and check if contains the pool ID
        let tx_json = serde_json::to_string(&transaction).unwrap_or_default();
        if tx_json.contains(SUI_USDC_POOL_ID) {
            info!("✅ Found target pool ID in transaction JSON: {}", SUI_USDC_POOL_ID);
            return true;
        }

        // Don't log anything for non-relevant transactions
        false
    }

    /// Extract relevant data from a transaction
    fn extract_transaction_data(
        &self, 
        transaction: &CheckpointTransaction,
        checkpoint_sequence_number: u64
    ) -> CetusTransactionData {
        // Get the transaction digest directly from the effects
        let transaction_digest = transaction.effects.transaction_digest().to_string();
        
        // Serialize all transaction data to JSON for inspection
        let transaction_data = serde_json::to_value(transaction).unwrap_or_default();
        
        CetusTransactionData {
            transaction_digest,
            transaction_data,
            checkpoint_sequence_number,
        }
    }

    /// Get all accumulated transaction data
    pub fn get_all_transaction_data(&self) -> Vec<&CetusTransactionData> {
        self.transaction_data.values().collect()
    }

    /// Clear accumulated data
    pub fn clear_data(&mut self) {
        self.transaction_data.clear();
        // Không xóa swap_events vì chúng cần cho việc tính volume 24h
    }

    /// Process a checkpoint and collect all Cetus-related transactions
    pub async fn process_checkpoint(&mut self, data: &CheckpointData, pool: Option<&PgPool>) -> Vec<CetusTransactionData> {
        let checkpoint_seq_number = data.checkpoint_summary.sequence_number;
        let mut cetus_transactions = Vec::new();
        let checkpoint_timestamp = data.checkpoint_summary.timestamp();
        let checkpoint_digest = data.checkpoint_summary.digest();
        
        // Log checkpoint information
        info!("╔══════════════════════════════════════");
        info!("║ Processing checkpoint {}", checkpoint_seq_number);
        info!("║ Checkpoint Digest: {}", checkpoint_digest);
        info!("║ Timestamp: {:?}", checkpoint_timestamp);
        info!("║ Transaction count: {}", data.transactions.len());
        info!("║ Filtering for SUI-USDC Pool: {}", SUI_USDC_POOL_ID);
        info!("╚══════════════════════════════════════");

        for (tx_index, transaction) in data.transactions.iter().enumerate() {
            // Only check if the transaction is Cetus-related before logging
            if self.is_cetus_transaction(transaction) {
                // Log transaction events for Cetus-related transactions only
                if let Ok(all_events_json) = serde_json::to_value(&transaction.events) {
                    // Check if events is an array and log each event type
                    if let Some(events_array) = all_events_json.as_array() {
                        for (event_index, event) in events_array.iter().enumerate() {
                            if let Some(type_str) = event.get("type").and_then(|t| t.as_str()) {
                                info!("  Event #{}: Type = {}", event_index, type_str);
                                
                                // If it looks like a Cetus event, log more details
                                if type_str.contains("cetus") {
                                    info!("  🔍 FOUND CETUS EVENT: {}", type_str);
                                    info!("  Event details: {}", event);
                                }
                            }
                        }
                    }
                }
                
                let tx_data = self.extract_transaction_data(transaction, checkpoint_seq_number);
            
                // Extract swap events from transaction
                let swap_events = self.extract_swap_events(transaction, checkpoint_timestamp, tx_data.transaction_digest.clone());
                if !swap_events.is_empty() {
                    info!("Found {} swap events in transaction {}", swap_events.len(), tx_data.transaction_digest);
                    
                    // Log details of each swap event
                    for (idx, event) in swap_events.iter().enumerate() {
                        info!("  Swap Event #{}: pool_id={}, coin_a={}, coin_b={}, amount_in={}, amount_out={}, a2b={}",
                              idx, event.pool_id, event.coin_a, event.coin_b, event.amount_in, event.amount_out, event.atob);
                    }
                    
                    self.swap_events.extend(swap_events);
                }
                
                // Format log data nicely
                info!("╔══════════════════════════════════════");
                info!("║ Found Cetus Transaction");
                info!("║ Transaction Digest: {}", tx_data.transaction_digest);
                info!("║ Checkpoint: {}", checkpoint_seq_number);
                info!("║ Checkpoint Digest: {}", checkpoint_digest);
                info!("║ Timestamp: {:?}", checkpoint_timestamp);
                info!("╚══════════════════════════════════════");
                
                self.transaction_data.insert(tx_data.transaction_digest.clone(), tx_data.clone());
                cetus_transactions.push(tx_data);
            }
        }

        // Update last_processed_checkpoint
        self.last_processed_checkpoint = checkpoint_seq_number;
        
        // Update 24h volume after processing checkpoint
        self.calculate_volume_24h();
        
        // Save to database if we have a pool connection and it's checkpoint 10, 20, 30...
        // or if we found swap events
        if let Some(pool) = pool {
            if checkpoint_seq_number % 10 == 0 || !self.swap_events.is_empty() {
                if let Err(err) = self.update_volume_24h_in_database(pool).await {
                    error!("Failed to update volume in database: {}", err);
                }
            }
        }

        if !cetus_transactions.is_empty() {
            info!("✅ Found {} Cetus transactions in checkpoint {}", 
                 cetus_transactions.len(), checkpoint_seq_number);
        }

        cetus_transactions
    }

    /// Get swap events for SUI-USDC pool
    pub fn get_sui_usdc_pool_events(&self) -> Vec<&SwapEvent> {
        self.swap_events.iter()
            .filter(|event| event.pool_id == SUI_USDC_POOL_ID)
            .collect()
    }
    
    /// Get transaction digests for SUI-USDC pool events
    pub fn get_sui_usdc_pool_transaction_digests(&self) -> Vec<String> {
        self.swap_events.iter()
            .filter(|event| event.pool_id == SUI_USDC_POOL_ID)
            .map(|event| event.transaction_digest.clone())
            .collect()
    }
}

/// Job để cập nhật volume định kỳ
pub async fn start_volume_update_job(indexer: Arc<Mutex<CetusIndexer>>, pool: PgPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(60 * 10)); // 10 phút
    
    loop {
        interval.tick().await;
        
        let mut indexer_locked = indexer.lock().await;
        // Tính toán volume mới
        indexer_locked.calculate_volume_24h();
        // Cập nhật vào database
        if let Err(err) = indexer_locked.update_volume_24h_in_database(&pool).await {
            error!("Scheduled volume update failed: {}", err);
        }
    }
}

/// Tạo bảng volume_data trong database
pub async fn create_volume_data_table(pool: &PgPool) -> Result<(), Error> {
    sqlx::query!(
        "CREATE TABLE IF NOT EXISTS volume_data (
            id SERIAL PRIMARY KEY,
            period VARCHAR(50) NOT NULL UNIQUE,
            total_volume DECIMAL(30, 8) NOT NULL,
            usdc_volume DECIMAL(30, 8) NOT NULL DEFAULT 0,
            sui_usdc_pool_volume DECIMAL(30, 8) NOT NULL DEFAULT 0,
            last_update TIMESTAMP NOT NULL,
            last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
        )"
    )
    .execute(pool)
    .await?;
    
    info!("✅ Volume data table created or already exists");
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cetus_indexer_creation() {
        let indexer = CetusIndexer::new();
        assert_eq!(indexer.transaction_data.len(), 0);
    }
} 