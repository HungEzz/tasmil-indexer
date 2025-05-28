// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * CETUS INDEXER MODULE
 * 
 * This module contains the core logic for indexing Cetus DEX data from Sui blockchain.
 * It processes checkpoints to extract swap and liquidity events, calculates metrics,
 * and manages database storage.
 * 
 * Key components:
 * - Event extraction from blockchain transactions
 * - Volume, TVL, and fees calculation
 * - Database interaction for persistence
 * - Price fetching from external APIs
 */

use std::time::{Duration, SystemTime};
use serde::{Deserialize, Serialize};
use sui_types::full_checkpoint_content::{CheckpointData, CheckpointTransaction};
use sui_types::effects::TransactionEffectsAPI;
use tracing::{info, error, debug};
use anyhow::Error;
use tokio::sync::Mutex;
use std::sync::Arc;
use reqwest;
use crate::PgConnectionPool;
use crate::database::DatabaseManager;
use crate::config::get_config;

/// DATA STRUCTURES FOR EVENTS AND METRICS

/**
 * SwapEvent represents a token swap transaction on Cetus DEX
 * Contains details about the trade including amounts, direction, and fees
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwapEvent {
    pub pool_id: String,           // Unique identifier for the liquidity pool
    pub amount_in: u64,            // Amount of tokens being sold
    pub amount_out: u64,           // Amount of tokens being bought
    pub atob: bool,                // Trade direction: true = token A to B, false = B to A
    pub fee_amount: u64,           // Trading fee charged for this swap
    pub timestamp: SystemTime,     // When the swap occurred
    pub transaction_digest: String, // Unique transaction identifier
}

/**
 * AddLiquidityEvent represents liquidity provision to a pool
 * Tracks when users add liquidity and the amounts provided
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddLiquidityEvent {
    pub pool_id: String,           // Pool receiving the liquidity
    pub amount_a: u64,             // Amount of first token (USDC)
    pub amount_b: u64,             // Amount of second token (SUI)
    pub timestamp: SystemTime,     // When liquidity was added
    pub transaction_digest: String, // Unique transaction identifier
}

/**
 * VolumeData tracks 24-hour trading volume metrics
 * Used for monitoring trading activity and calculating fees
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeData {
    pub sui_usd_volume: f64,      // Total USD volume traded in 24h
    pub last_update: SystemTime,  // Last time this data was updated
}

/**
 * TvlData tracks Total Value Locked (TVL) in the pool
 * Represents the total value of assets available for trading
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TvlData {
    pub total_usd_tvl: f64,       // Total USD value locked in pool
    pub last_update: SystemTime,  // Last time this data was updated
}

/**
 * FeeData tracks trading fees collected over 24 hours
 * Important metric for understanding protocol revenue
 */
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeeData {
    pub fees_24h: f64,            // Total fees collected in 24h (USD)
    pub last_update: SystemTime,  // Last time this data was updated
}

impl Default for FeeData {
    fn default() -> Self {
        Self {
            fees_24h: 0.0,
            last_update: SystemTime::now(),
        }
    }
}

/// MAIN INDEXER STRUCT

/**
 * CetusIndexer is the main struct that processes blockchain data
 * It maintains state about events, metrics, and provides methods for processing
 */
#[derive(Clone)]
pub struct CetusIndexer {
    pub swap_events: Vec<SwapEvent>,              // All processed swap events
    pub liquidity_events: Vec<AddLiquidityEvent>, // All processed liquidity events
    pub volume_24h: VolumeData,                   // Current 24h volume metrics
    pub tvl_24h: TvlData,                        // Current TVL metrics
    pub fee_24h: FeeData,                        // Current 24h fee metrics
    pub last_processed_checkpoint: u64,           // Last checkpoint number processed
}

impl CetusIndexer {
    /// Creates a new CetusIndexer instance with default values
    /// All metrics start at zero and will be calculated as events are processed
    pub fn new() -> Self {
        Self {
            swap_events: Vec::new(),
            liquidity_events: Vec::new(),
            volume_24h: VolumeData {
                sui_usd_volume: 0.0,
                last_update: SystemTime::now(),
            },
            tvl_24h: TvlData {
                total_usd_tvl: 0.0,
                last_update: SystemTime::now(),
            },
            fee_24h: FeeData::default(),
            last_processed_checkpoint: 0,
        }
    }
    
    /// MAIN PROCESSING FUNCTION
    /// Process a checkpoint and extract Cetus-related events
    /// This is called for each blockchain checkpoint to find relevant transactions
    /// 
    /// # Arguments
    /// * `data` - The checkpoint data containing all transactions
    /// * `pool` - Optional database connection for storing results
    /// 
    /// # Returns
    /// * Tuple of (new_swap_events, new_liquidity_events) found in this checkpoint
    pub async fn process_checkpoint(
        &mut self, 
        data: &CheckpointData, 
        pool: Option<&PgConnectionPool>
    ) -> (Vec<SwapEvent>, Vec<AddLiquidityEvent>) {
        let checkpoint_seq_number = data.checkpoint_summary.sequence_number;
        let mut new_swap_events = Vec::new();
        let mut new_liquidity_events = Vec::new();
        let checkpoint_timestamp = data.checkpoint_summary.timestamp();

        info!("Processing checkpoint {} with {} transactions", 
             checkpoint_seq_number, data.transactions.len());

        // Loop through all transactions in this checkpoint
        for transaction in &data.transactions {
            let tx_digest = transaction.effects.transaction_digest().to_string();
            
            // Extract Cetus events from this transaction
            let (swap_events, liquidity_events) = self.extract_events(transaction, checkpoint_timestamp, tx_digest);
            
            // Add any found swap events to our collections
            if !swap_events.is_empty() {
                debug!("Found {} swap events", swap_events.len());
                new_swap_events.extend(swap_events.clone());
                self.swap_events.extend(swap_events);
            }

            // Add any found liquidity events to our collections
            if !liquidity_events.is_empty() {
                debug!("Found {} liquidity events", liquidity_events.len());
                new_liquidity_events.extend(liquidity_events.clone());
                self.liquidity_events.extend(liquidity_events);
            }
        }

        // Update our 24h metrics if we found any new events
        if !new_swap_events.is_empty() || !new_liquidity_events.is_empty() {
            self.update_metrics().await;
            
            // Log a summary of what we found and current metrics
            info!("📊 Checkpoint {}: {} swap events, {} liquidity events | Volume: ${:.2} | TVL: ${:.2} | Fees: ${:.2}", 
                 checkpoint_seq_number, new_swap_events.len(), new_liquidity_events.len(), 
                 self.volume_24h.sui_usd_volume / 1_000_000.0,
                 self.tvl_24h.total_usd_tvl / 1_000_000.0,
                 self.fee_24h.fees_24h / 1_000_000.0);
        }

        // Remember this checkpoint number as processed
        self.last_processed_checkpoint = checkpoint_seq_number;

        // Save to database periodically or when we have new events
        if let Some(pool) = pool {
            let config = get_config();
            if checkpoint_seq_number % config.checkpoint_batch_size == 0 || !new_swap_events.is_empty() || !new_liquidity_events.is_empty() {
                if let Err(err) = self.update_data_in_database(pool).await {
                    error!("Failed to update database: {}", err);
                }
            }
        }

        (new_swap_events, new_liquidity_events)
    }
    
    /// EVENT EXTRACTION FUNCTION
    /// Extract both swap and liquidity events from a single transaction
    /// Parses the transaction's events to find Cetus-specific activities
    /// 
    /// # Arguments
    /// * `transaction` - The transaction to analyze
    /// * `timestamp` - When this transaction occurred
    /// * `tx_digest` - Unique identifier for this transaction
    /// 
    /// # Returns
    /// * Tuple of (swap_events, liquidity_events) found in this transaction
    fn extract_events(&self, transaction: &CheckpointTransaction, timestamp: SystemTime, tx_digest: String) -> (Vec<SwapEvent>, Vec<AddLiquidityEvent>) {
        let mut swap_events = Vec::new();
        let mut liquidity_events = Vec::new();
        let config = get_config();
        
        // Convert transaction events to JSON for easier parsing
        if let Ok(events_json) = serde_json::to_value(&transaction.events) {
            if let Some(data) = events_json.get("data").and_then(|d| d.as_array()) {    
                // Loop through each event in this transaction
                for event in data.iter() {
                    // Skip empty or null events
                    if event.is_null() || (event.is_object() && event.as_object().unwrap().is_empty()) {
                        continue;
                    }
                    
                    // Check if this event is from Cetus protocol
                    if let Some(type_obj) = event.get("type_") {
                        let address = type_obj.get("address").and_then(|a| a.as_str()).unwrap_or("");
                        let module = type_obj.get("module").and_then(|m| m.as_str()).unwrap_or("");
                        let name = type_obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        
                        // Only process events from the Cetus pool module
                        if address == config.cetus_address && module == "pool" {
                        if let Some(contents) = event.get("contents").and_then(|c| c.as_array()) {
                                match name {
                                    // Handle swap events (token trades)
                                    "SwapEvent" => {
                                        if let Some(swap_event) = self.decode_swap_event(contents, timestamp, tx_digest.clone()) {
                                            swap_events.push(swap_event);
                                        }
                                    },
                                    // Handle liquidity provision events
                                    "AddLiquidityEvent" => {
                                        if let Some(liquidity_event) = self.decode_liquidity_event(contents, timestamp, tx_digest.clone()) {
                                            liquidity_events.push(liquidity_event);
                                        }
                                    },
                                    _ => {} // Ignore other event types
                            }
                        }
                    }
                }
            }
        }
        }
        
        (swap_events, liquidity_events)
    }

    /// SWAP EVENT DECODER
    /// Decode swap event from raw blockchain event contents
    /// Parses the binary data to extract swap details like amounts and direction
    /// 
    /// # Arguments
    /// * `contents` - Raw event data as JSON array
    /// * `timestamp` - When the swap occurred
    /// * `tx_digest` - Transaction identifier
    /// 
    /// # Returns
    /// * Some(SwapEvent) if successfully decoded, None if parsing failed
    fn decode_swap_event(&self, contents: &[serde_json::Value], timestamp: SystemTime, tx_digest: String) -> Option<SwapEvent> {
        // Ensure we have enough data to parse (146 elements expected)
        if contents.len() < 146 {
            return None;
        }
        
        let config = get_config();
        let first_byte = contents.get(0).and_then(|v| v.as_u64()).unwrap_or(99);
        
        // Extract pool ID
        let mut pool_bytes = Vec::with_capacity(32);
        for i in 0..32 {
            if let Some(val) = contents.get(i + 1).and_then(|v| v.as_u64()) {
                pool_bytes.push(val as u8);
            } else {
                return None;
            }
        }
        
        let pool_id = format!("0x{}", pool_bytes.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>());
        
        if pool_id != config.sui_usdc_pool_id {
            return None;
        }
        
        // Extract amounts
        let amount_in = self.extract_u64_from_bytes(contents, 65)?;
        let amount_out = self.extract_u64_from_bytes(contents, 73)?;
        let fee_amount = self.extract_u64_from_bytes(contents, 89)?;
        
        if amount_in == 0 || amount_out == 0 {
            return None;
        }
        
        let atob = match first_byte {
            1 => true,
            0 => false,
            _ => contents.get(145).and_then(|v| v.as_u64()).unwrap_or(0) == 1,
        };
        
        Some(SwapEvent {
            pool_id: config.sui_usdc_pool_id.clone(),
            amount_in,
            amount_out,
            atob,
            fee_amount,
            timestamp,
            transaction_digest: tx_digest,
        })
    }

    /// LIQUIDITY EVENT DECODER
    /// Decode liquidity provision event from raw blockchain event contents
    /// Parses the binary data to extract liquidity amounts for both tokens
    /// 
    /// # Arguments
    /// * `contents` - Raw event data as JSON array
    /// * `timestamp` - When the liquidity was added
    /// * `tx_digest` - Transaction identifier
    /// 
    /// # Returns
    /// * Some(AddLiquidityEvent) if successfully decoded, None if parsing failed
    fn decode_liquidity_event(&self, contents: &[serde_json::Value], timestamp: SystemTime, tx_digest: String) -> Option<AddLiquidityEvent> {
        // Ensure we have enough data to parse (120 elements expected)
        if contents.len() < 120 {
            return None;
        }
        
        let config = get_config();
        
        // Extract pool ID from the first 32 bytes
        let mut pool_bytes = Vec::with_capacity(32);
        for i in 0..32 {
            if let Some(val) = contents.get(i).and_then(|v| v.as_u64()) {
                pool_bytes.push(val as u8);
        } else {
                return None;
            }
        }
        
        // Convert bytes to hex string format
        let pool_id = format!("0x{}", pool_bytes.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>());
        
        // Only process events from our target pool
        if pool_id != config.sui_usdc_pool_id {
            return None;
        }
        
        // Extract token amounts from specific byte positions
        let amount_a = self.extract_u64_from_bytes(contents, 104)?; // USDC amount
        let amount_b = self.extract_u64_from_bytes(contents, 112)?; // SUI amount
        
        // Skip empty liquidity events
                                if amount_a == 0 && amount_b == 0 {
            return None;
        }
        
        Some(AddLiquidityEvent {
            pool_id: config.sui_usdc_pool_id.clone(),
                                        amount_a,
                                        amount_b,
                                        timestamp,
            transaction_digest: tx_digest,
        })
    }

    /// BYTE EXTRACTION HELPER
    /// Helper function to extract u64 value from byte array in little-endian format
    /// This is needed because blockchain data is stored as individual bytes
    /// 
    /// # Arguments
    /// * `contents` - The raw byte array as JSON values
    /// * `start_pos` - Starting position to read 8 bytes from
    /// 
    /// # Returns
    /// * Some(u64) if successfully parsed, None if data is incomplete
    fn extract_u64_from_bytes(&self, contents: &[serde_json::Value], start_pos: usize) -> Option<u64> {
        let mut value: u64 = 0;
        // Read 8 bytes and combine them into a u64 (little-endian)
        for i in 0..8 {
            if let Some(val) = contents.get(start_pos + i).and_then(|v| v.as_u64()) {
                value |= (val as u64) << (i * 8); // Shift each byte to its position
            } else {
                return None;
            }
        }
        Some(value)
    }

    /// METRICS UPDATE COORDINATOR
    /// Update all metrics (volume, TVL, and fees) and clean up old events
    /// This is called after processing each checkpoint with new events
    async fn update_metrics(&mut self) {
        // Calculate fresh 24h metrics
        let volume = self.calculate_volume_24h().await;
        let tvl = self.calculate_tvl_24h().await;
        let fees = self.calculate_fees_24h().await;
        
        // Update our stored metrics
        self.volume_24h.sui_usd_volume = volume;
        self.tvl_24h.total_usd_tvl = tvl;
        self.fee_24h.fees_24h = fees;
        
        // Update timestamps
        let now = SystemTime::now();
        self.volume_24h.last_update = now;
        self.tvl_24h.last_update = now;
        self.fee_24h.last_update = now;
        
        // Remove events older than 24 hours to keep memory usage reasonable
        self.prune_old_events(now);
    }

    /// VOLUME CALCULATION
    /// Calculate total trading volume in USD over the last 24 hours
    /// Sums up all swap events and converts SUI amounts to USD using current price
    /// 
    /// # Returns
    /// * f64 representing total 24h volume in USD
    async fn calculate_volume_24h(&mut self) -> f64 {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now - Duration::from_secs(24 * 60 * 60);
        
        // Get current SUI price in USD from Pyth oracle
        let sui_price = match get_price_from_pyth("0x23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744").await {
            Ok(price) => price,
            Err(_) => {
                error!("Failed to get SUI price, using fallback");
                1.0 // Fallback price if API fails
            }
        };

        let mut total_volume = 0.0;
        
        // Process all swap events from the last 24 hours
        for event in &self.swap_events {
            if event.timestamp >= twenty_four_hours_ago {
                let sui_amount = if event.atob {
                    // USDC -> SUI: output is SUI
                    event.amount_out as f64 / 1_000_000_000.0 // Convert from smallest unit
                } else {
                    // SUI -> USDC: input is SUI  
                    event.amount_in as f64 / 1_000_000_000.0
                };
                
                // Convert SUI amount to USD value
                total_volume += sui_amount * sui_price;
            }
        }

        total_volume
    }

    /// TVL CALCULATION
    /// Calculate Total Value Locked in USD by analyzing current liquidity in the pool
    /// TVL represents the total value of assets available for trading
    /// 
    /// # Returns
    /// * f64 representing current TVL in USD
    async fn calculate_tvl_24h(&mut self) -> f64 {
        // Get current SUI price in USD
        let sui_price = match get_price_from_pyth("0x23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744").await {
            Ok(price) => price,
            Err(_) => 1.0 // Fallback price
        };

        let mut total_usdc = 0.0;
        let mut total_sui = 0.0;

        // Sum up all liquidity additions (simplified approach)
        // In reality, you'd need to track removals too
        for event in &self.liquidity_events {
            total_usdc += event.amount_a as f64 / 1_000_000.0; // USDC has 6 decimals
            total_sui += event.amount_b as f64 / 1_000_000_000.0; // SUI has 9 decimals
        }

        // Convert both to USD and sum
        let usdc_value = total_usdc; // USDC is approximately $1
        let sui_value = total_sui * sui_price;
        
        usdc_value + sui_value
    }

    /// FEES CALCULATION
    /// Calculate total trading fees collected over the last 24 hours in USD
    /// Fees are charged on each swap and represent protocol revenue
    /// 
    /// # Returns
    /// * f64 representing total 24h fees in USD
    async fn calculate_fees_24h(&mut self) -> f64 {
        let now = SystemTime::now();
        let twenty_four_hours_ago = now - Duration::from_secs(24 * 60 * 60);
        
        // Get current SUI price for conversion
        let sui_price = match get_price_from_pyth("0x23d7315113f5b1d3ba7a83604c44b94d79f4fd69af77f804fc7f920a6dc65744").await {
            Ok(price) => price,
            Err(_) => 1.0
        };

        let mut total_fees = 0.0;
        
        // Sum fees from all swaps in the last 24 hours
        for event in &self.swap_events {
            if event.timestamp >= twenty_four_hours_ago {
                // Fee amount is typically in the same token as amount_in
            let fee_in_usd = if event.atob {
                    // USDC -> SUI: fee is in USDC
                    event.fee_amount as f64 / 1_000_000.0 // USDC has 6 decimals
            } else {
                    // SUI -> USDC: fee is in SUI
                    let fee_in_sui = event.fee_amount as f64 / 1_000_000_000.0; // SUI has 9 decimals
                    fee_in_sui * sui_price // Convert to USD
            };
            
                total_fees += fee_in_usd;
            }
        }
        
        total_fees
    }
    
    /// EVENT CLEANUP
    /// Remove events older than 24 hours to prevent memory growth
    /// Only keeps recent events needed for 24h calculations
    /// 
    /// # Arguments
    /// * `now` - Current time for comparison
    fn prune_old_events(&mut self, now: SystemTime) {
        let twenty_four_hours_ago = now - Duration::from_secs(24 * 60 * 60);
        
        // Keep only recent swap events
        self.swap_events.retain(|event| event.timestamp >= twenty_four_hours_ago);
        
        // Keep only recent liquidity events  
        self.liquidity_events.retain(|event| event.timestamp >= twenty_four_hours_ago);
        
        debug!("Pruned old events. Remaining: {} swaps, {} liquidity events", 
               self.swap_events.len(), self.liquidity_events.len());
    }

    /// DATABASE UPDATE
    /// Save current metrics and checkpoint progress to the database
    /// This ensures data persistence across restarts
    /// 
    /// # Arguments
    /// * `pool` - Database connection pool
    /// 
    /// # Returns
    /// * Result indicating success or failure
    pub async fn update_data_in_database(&self, pool: &PgConnectionPool) -> Result<(), Error> {
        let mut conn = pool.get().await
            .map_err(|e| anyhow::anyhow!("Failed to get database connection: {}", e))?;
        
        // Save current volume, TVL, and fee metrics
        DatabaseManager::update_volume_data(
            &mut conn,
            self.volume_24h.sui_usd_volume,
            self.tvl_24h.total_usd_tvl,
            self.fee_24h.fees_24h,
            self.last_processed_checkpoint as i64,
        ).await?;
        
        Ok(())
    }

    /// DATABASE LOAD
    /// Load existing metrics and checkpoint progress from the database
    /// Called at startup to restore previous state
    /// 
    /// # Arguments
    /// * `pool` - Database connection pool
    /// 
    /// # Returns
    /// * Result indicating success or failure
    pub async fn get_data_from_database(&mut self, pool: &PgConnectionPool) -> Result<(), Error> {
        let mut conn = pool.get().await
            .map_err(|e| anyhow::anyhow!("Failed to get database connection: {}", e))?;
        
        // Load the most recent volume data
        if let Some(data) = DatabaseManager::get_volume_data(&mut conn).await? {
            self.volume_24h.sui_usd_volume = data.sui_usd_volume.to_string().parse().unwrap_or(0.0);
            self.tvl_24h.total_usd_tvl = data.total_usd_tvl.to_string().parse().unwrap_or(0.0);
            self.fee_24h.fees_24h = data.fees_24h.to_string().parse().unwrap_or(0.0);
            self.last_processed_checkpoint = data.last_processed_checkpoint as u64;
            
            info!("Loaded data from database - Volume: ${:.2}, TVL: ${:.2}, Fees: ${:.2}", 
                  self.volume_24h.sui_usd_volume / 1_000_000.0,
                  self.tvl_24h.total_usd_tvl / 1_000_000.0,
                  self.fee_24h.fees_24h / 1_000_000.0);
        }
        
        Ok(())
    }

    /// CHECKPOINT RETRIEVAL
    /// Get the last processed checkpoint number from the database
    /// Used to resume processing from where we left off
    /// 
    /// # Arguments
    /// * `pool` - Database connection pool
    /// 
    /// # Returns
    /// * Result containing the last checkpoint number or error
    pub async fn get_last_processed_checkpoint(pool: &PgConnectionPool) -> Result<u64, Error> {
        let mut conn = pool.get().await
            .map_err(|e| anyhow::anyhow!("Failed to get database connection: {}", e))?;
        Ok(DatabaseManager::get_last_processed_checkpoint(&mut conn).await?)
    }

    /// GETTER METHOD
    /// Get reference to all stored swap events
    /// Used for analysis and reporting
    pub fn get_swap_events(&self) -> &Vec<SwapEvent> {
        &self.swap_events
    }
}

/// EXTERNAL PRICE FETCHING
/// Fetch current price from Pyth Network oracle
/// Pyth provides real-time price feeds for various cryptocurrencies
/// 
/// # Arguments
/// * `price_id` - Unique identifier for the price feed (SUI price feed ID)
/// 
/// # Returns
/// * Result containing the current price in USD or error
async fn get_price_from_pyth(price_id: &str) -> Result<f64, Error> {
    // Pyth Network API endpoint for price data
    let url = format!("https://hermes.pyth.network/api/latest_price_feeds?ids[]={}", price_id);
    
    // Make HTTP request to Pyth API
    let response = reqwest::get(&url).await?;
    let json: serde_json::Value = response.json().await?;
    
    // Parse the response to extract price
    if let Some(price_feed) = json.as_array().and_then(|arr| arr.get(0)) {
        if let Some(price_obj) = price_feed.get("price") {
            if let Some(price_str) = price_obj.get("price").and_then(|p| p.as_str()) {
                if let Ok(price_int) = price_str.parse::<i64>() {
                    // Pyth prices come with an exponent for decimal places
                    if let Some(expo) = price_obj.get("expo").and_then(|e| e.as_i64()) {
                        let price = price_int as f64 * 10_f64.powi(expo as i32);
                        return Ok(price);
                    }
                }
            }
        }
    }
    
    Err(anyhow::anyhow!("Failed to parse price from Pyth response"))
}

/// BACKGROUND JOB
/// Start a background task to update volume and TVL data every 10 minutes
/// This ensures metrics stay current even without new transactions
/// 
/// # Arguments
/// * `indexer` - Shared reference to the CetusIndexer
/// * `pool` - Database connection pool
pub async fn start_volume_update_job(indexer: Arc<Mutex<CetusIndexer>>, pool: PgConnectionPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(600)); // 10 minutes
    
    loop {
        interval.tick().await;
        
        // Update metrics and save to database
        {
            let mut indexer_guard = indexer.lock().await;
            indexer_guard.update_metrics().await;
            
            if let Err(err) = indexer_guard.update_data_in_database(&pool).await {
                error!("Failed to update database in background job: {}", err);
        } else {
                info!("🔄 Background update completed - Volume: ${:.2}, TVL: ${:.2}, Fees: ${:.2}",
                      indexer_guard.volume_24h.sui_usd_volume / 1_000_000.0,
                      indexer_guard.tvl_24h.total_usd_tvl / 1_000_000.0,
                      indexer_guard.fee_24h.fees_24h / 1_000_000.0);
            }
        }
    }
}

/// DATABASE TABLE CREATION
/// Create the volume_data table if it doesn't exist
/// This table stores our calculated metrics for persistence
/// 
/// # Arguments
/// * `pool` - Database connection pool
/// 
/// # Returns
/// * Result indicating success or failure
pub async fn create_volume_data_table(pool: &PgConnectionPool) -> Result<(), Error> {
    let mut conn = pool.get().await
        .map_err(|e| anyhow::anyhow!("Failed to get database connection: {}", e))?;
    DatabaseManager::create_tables(&mut conn).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cetus_indexer_creation() {
        let indexer = CetusIndexer::new();
        assert_eq!(indexer.swap_events.len(), 0);
        assert_eq!(indexer.liquidity_events.len(), 0);
        assert_eq!(indexer.last_processed_checkpoint, 0);
    }
}

