// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * CONFIGURATION MANAGEMENT MODULE
 * 
 * This module handles all configuration settings for the SuiNS Indexer.
 * It loads settings from environment variables, validates them, and provides
 * a global configuration instance that can be safely accessed from anywhere
 * in the application.
 * 
 * Key features:
 * - Environment variable loading with validation
 * - Default values for optional settings
 * - Thread-safe global configuration instance
 * - Comprehensive validation of addresses and IDs
 * - Security-conscious logging (hides sensitive data)
 */

use std::env;
use std::time::Duration;
use anyhow::{Result, Context};
use dotenvy::dotenv;
use std::sync::OnceLock;

/**
 * Configuration structure for the SuiNS Indexer
 * 
 * This struct contains all the settings needed to run the indexer,
 * including database connections, blockchain addresses, API endpoints,
 * and operational parameters.
 */
#[derive(Debug, Clone)]
pub struct Config {
    // DATABASE CONFIGURATION
    /// PostgreSQL database connection string
    /// Format: "postgresql://username:password@localhost/database_name"
    pub database_url: String,
    
    // CETUS PROTOCOL CONFIGURATION
    /// The specific SUI/USDC pool ID we're monitoring on Cetus DEX
    /// This is a 32-byte address in hex format (66 chars with 0x prefix)
    pub sui_usdc_pool_id: String,
    
    /// Cetus protocol contract address on Sui blockchain
    /// Used to filter events to only those from Cetus protocol
    pub cetus_address: String,
    
    // PRICE FEED CONFIGURATION
    /// Pyth Network price feed ID for USDC/USD price
    /// Used to convert USDC amounts to USD for calculations
    pub usdc_price_feed_id: String,
    
    /// Pyth Network price feed ID for SUI/USD price
    /// Critical for converting SUI amounts to USD for volume calculations
    pub sui_price_feed_id: String,
    
    /// Pyth Network API endpoint for fetching price data
    /// Default: "https://hermes.pyth.network/api/latest_price_feeds"
    pub pyth_api_url: String,
    
    // OPERATIONAL SETTINGS
    /// How often to update metrics and save to database (in seconds)
    /// Default: 600 seconds (10 minutes)
    pub update_interval: Duration,
    
    /// How long to keep events in memory for calculations (in hours)
    /// Events older than this are pruned to save memory
    /// Default: 25 hours (to ensure 24h calculations work properly)
    pub event_retention_hours: u64,
    
    /// How many checkpoints to process before forcing a database update
    /// Helps ensure data persistence even during long processing runs
    /// Default: 10 checkpoints
    pub checkpoint_batch_size: u64,
    
    // SUI NETWORK CONFIGURATION (Optional)
    /// Remote storage URL for downloading checkpoints
    /// If not provided, assumes checkpoints are available locally
    pub remote_storage: Option<String>,
    
    /// Path to the file tracking backfill progress
    /// Used to resume processing from the last checkpoint
    pub backfill_progress_file_path: Option<String>,
}

impl Config {
    /// CONFIGURATION LOADER
    /// Load configuration from environment variables with validation
    /// 
    /// This function reads all required and optional environment variables,
    /// applies default values where appropriate, and validates the configuration
    /// before returning it.
    /// 
    /// # Returns
    /// * `Result<Config>` - Successfully loaded and validated configuration or error
    /// 
    /// # Environment Variables Required
    /// - `DATABASE_URL` - PostgreSQL connection string
    /// - `SUI_USDC_POOL_ID` - Target pool address (66 chars with 0x prefix)
    /// - `CETUS_ADDRESS` - Cetus protocol address (64 chars hex)
    /// - `USDC_PRICE_FEED_ID` - Pyth price feed ID for USDC (64 chars hex)
    /// - `SUI_PRICE_FEED_ID` - Pyth price feed ID for SUI (64 chars hex)
    /// 
    /// # Environment Variables Optional
    /// - `PYTH_API_URL` - Pyth API endpoint (default: Pyth hermes)
    /// - `UPDATE_INTERVAL_SECONDS` - Update frequency (default: 600)
    /// - `EVENT_RETENTION_HOURS` - Memory retention time (default: 25)
    /// - `CHECKPOINT_BATCH_SIZE` - DB update frequency (default: 10)
    /// - `REMOTE_STORAGE` - Remote checkpoint storage URL
    /// - `BACKFILL_PROGRESS_FILE_PATH` - Progress tracking file path
    pub fn from_env() -> Result<Self> {
        // Load .env file if it exists (for local development)
        dotenv().ok();
        
        let config = Config {
            // Required: Database connection
            database_url: env::var("DATABASE_URL")
                .context("DATABASE_URL must be set - example: postgresql://user:pass@localhost/suins_indexer")?,
            
            // Required: Target pool to monitor
            sui_usdc_pool_id: env::var("SUI_USDC_POOL_ID")
                .context("SUI_USDC_POOL_ID must be set - the 32-byte pool address with 0x prefix")?,
            
            // Required: Cetus protocol address for event filtering
            cetus_address: env::var("CETUS_ADDRESS")
                .context("CETUS_ADDRESS must be set - the 32-byte Cetus contract address")?,
            
            // Required: Price feed IDs for USD conversion
            usdc_price_feed_id: env::var("USDC_PRICE_FEED_ID")
                .context("USDC_PRICE_FEED_ID must be set - Pyth Network price feed ID for USDC")?,
            
            sui_price_feed_id: env::var("SUI_PRICE_FEED_ID")
                .context("SUI_PRICE_FEED_ID must be set - Pyth Network price feed ID for SUI")?,
            
            // Optional: Pyth API endpoint (has sensible default)
            pyth_api_url: env::var("PYTH_API_URL")
                .unwrap_or_else(|_| "https://hermes.pyth.network/api/latest_price_feeds".to_string()),
            
            // Optional: Update interval with validation
            update_interval: Duration::from_secs(
                env::var("UPDATE_INTERVAL_SECONDS")
                    .unwrap_or_else(|_| "600".to_string()) // Default: 10 minutes
                    .parse::<u64>()
                    .context("UPDATE_INTERVAL_SECONDS must be a valid number of seconds")?
            ),
            
            // Optional: Event retention period
            event_retention_hours: env::var("EVENT_RETENTION_HOURS")
                .unwrap_or_else(|_| "25".to_string()) // Default: 25 hours (buffer over 24h)
                .parse::<u64>()
                .context("EVENT_RETENTION_HOURS must be a valid number of hours")?,
            
            // Optional: Checkpoint batch size for database updates
            checkpoint_batch_size: env::var("CHECKPOINT_BATCH_SIZE")
                .unwrap_or_else(|_| "10".to_string()) // Default: every 10 checkpoints
                .parse::<u64>()
                .context("CHECKPOINT_BATCH_SIZE must be a valid number")?,
            
            // Optional: Sui network configuration
            remote_storage: env::var("REMOTE_STORAGE").ok(),
            backfill_progress_file_path: env::var("BACKFILL_PROGRESS_FILE_PATH").ok(),
        };
        
        // Validate all configuration values before using
        config.validate()?;
        
        Ok(config)
    }
    
    /// CONFIGURATION VALIDATOR
    /// Validate the configuration values to ensure they're in the correct format
    /// 
    /// This function checks:
    /// - Address formats (hex strings with correct lengths)
    /// - URL formats (must be valid HTTP/HTTPS)
    /// - Numeric ranges (reasonable min/max values)
    /// - Required field presence
    /// 
    /// # Returns
    /// * `Result<()>` - Success if all validation passes, detailed error if not
    fn validate(&self) -> Result<()> {
        // Validate SUI pool ID format (0x + 64 hex chars = 66 total)
        if !self.sui_usdc_pool_id.starts_with("0x") || self.sui_usdc_pool_id.len() != 66 {
            return Err(anyhow::anyhow!(
                "SUI_USDC_POOL_ID must be a valid 32-byte hex string starting with 0x (66 characters total)"
            ));
        }
        
        // Validate Cetus address format (64 hex characters, no 0x prefix expected)
        if self.cetus_address.len() != 64 {
            return Err(anyhow::anyhow!(
                "CETUS_ADDRESS must be a valid 32-byte hex string (64 characters, no 0x prefix)"
            ));
        }
        
        // Validate USDC price feed ID format (64 hex characters)
        if self.usdc_price_feed_id.len() != 64 {
            return Err(anyhow::anyhow!(
                "USDC_PRICE_FEED_ID must be a valid 32-byte hex string (64 characters)"
            ));
        }
        
        // Validate SUI price feed ID format (64 hex characters)
        if self.sui_price_feed_id.len() != 64 {
            return Err(anyhow::anyhow!(
                "SUI_PRICE_FEED_ID must be a valid 32-byte hex string (64 characters)"
            ));
        }
        
        // Validate Pyth API URL format
        if !self.pyth_api_url.starts_with("http") {
            return Err(anyhow::anyhow!(
                "PYTH_API_URL must be a valid HTTP/HTTPS URL"
            ));
        }
        
        // Validate update interval (minimum 1 minute to avoid API spam)
        if self.update_interval.as_secs() < 60 {
            return Err(anyhow::anyhow!(
                "UPDATE_INTERVAL_SECONDS must be at least 60 seconds to avoid API rate limits"
            ));
        }
        
        // Validate event retention (minimum 1 hour for meaningful 24h calculations)
        if self.event_retention_hours < 1 {
            return Err(anyhow::anyhow!(
                "EVENT_RETENTION_HOURS must be at least 1 hour"
            ));
        }
        
        // Validate checkpoint batch size (reasonable range to balance performance vs memory)
        if self.checkpoint_batch_size < 1 || self.checkpoint_batch_size > 100 {
            return Err(anyhow::anyhow!(
                "CHECKPOINT_BATCH_SIZE must be between 1 and 100 checkpoints"
            ));
        }
        
        Ok(())
    }
    
    /// Get the SUI/USDC pool ID (already includes 0x prefix)
    pub fn sui_usdc_pool_id(&self) -> &str {
        &self.sui_usdc_pool_id
    }
    
    /// Get the Cetus address with 0x prefix added
    /// The config stores the address without prefix, this method adds it
    pub fn cetus_address_with_prefix(&self) -> String {
        format!("0x{}", self.cetus_address)
    }
    
    /// Get the event retention duration as a Duration object
    /// Converts hours to Duration for use with SystemTime calculations
    pub fn event_retention_duration(&self) -> Duration {
        Duration::from_secs(self.event_retention_hours * 3600) // 3600 seconds per hour
    }
    
    /// CONFIGURATION SUMMARY PRINTER
    /// Print configuration summary without exposing sensitive information
    /// 
    /// This method logs the configuration in a readable format while being
    /// careful not to expose full addresses or sensitive data. It truncates
    /// long addresses to show just enough for identification.
    pub fn print_summary(&self) {
        println!("📋 Configuration Summary:");
        println!("  🏊 Pool ID: {}...{}", 
                &self.sui_usdc_pool_id[..10],      // First 10 chars
                &self.sui_usdc_pool_id[62..]);     // Last 4 chars
        println!("  🏢 Cetus Address: {}...{}", 
                &self.cetus_address[..8],          // First 8 chars
                &self.cetus_address[56..]);        // Last 8 chars
        println!("  ⏱️  Update Interval: {}s", self.update_interval.as_secs());
        println!("  🗂️  Event Retention: {}h", self.event_retention_hours);
        println!("  📦 Batch Size: {}", self.checkpoint_batch_size);
        println!("  🌐 Pyth API: {}", self.pyth_api_url);
        
        // Show optional settings if they're configured
        if self.remote_storage.is_some() {
            println!("  ☁️  Remote Storage: Configured");
        }
        if self.backfill_progress_file_path.is_some() {
            println!("  📄 Progress File: Configured");
        }
    }
}

/// GLOBAL CONFIGURATION MANAGEMENT
/// Global configuration instance using OnceLock for thread safety
/// 
/// OnceLock ensures that:
/// - Configuration is initialized exactly once
/// - Multiple threads can safely access the same config
/// - No runtime overhead after initialization
/// - Prevents accidental re-initialization
static CONFIG: OnceLock<Config> = OnceLock::new();

/// CONFIGURATION INITIALIZER
/// Initialize the global configuration from environment variables
/// 
/// This function should be called once at application startup.
/// It loads the configuration, validates it, prints a summary,
/// and stores it in the global CONFIG instance.
/// 
/// # Returns
/// * `Result<()>` - Success if configuration loaded and stored, error otherwise
/// 
/// # Errors
/// - If environment variables are missing or invalid
/// - If validation fails
/// - If configuration was already initialized
pub fn init_config() -> Result<()> {
    // Load and validate configuration from environment
    let config = Config::from_env()?;
    
    // Print summary for operator visibility (non-sensitive data only)
    config.print_summary();
    
    // Store in global instance (this can only happen once)
    CONFIG.set(config).map_err(|_| {
        anyhow::anyhow!("Configuration already initialized - init_config() should only be called once")
    })?;
    
    Ok(())
}

/// CONFIGURATION ACCESSOR
/// Get the global configuration instance
/// 
/// This function provides access to the validated, global configuration.
/// It will panic if called before init_config(), which is intentional
/// to catch programming errors early.
/// 
/// # Returns
/// * `&'static Config` - Reference to the global configuration
/// 
/// # Panics
/// * If called before init_config() - this indicates a programming error
pub fn get_config() -> &'static Config {
    CONFIG.get().expect(
        "Configuration not initialized. Call init_config() first in your main() function."
    )
} 