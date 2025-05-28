// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * CETUS CHECKPOINT PROCESSOR
 * 
 * This binary is the main entry point for processing Sui blockchain checkpoints
 * to extract and index Cetus DEX transaction data including swaps and liquidity events.
 * 
 * Key functionalities:
 * - Processes Sui blockchain checkpoints sequentially
 * - Extracts Cetus swap and liquidity events
 * - Calculates 24h volume, TVL, and fees metrics
 * - Stores data in PostgreSQL database
 * - Provides real-time monitoring via logging
 */

use dotenvy::dotenv;
use mysten_service::metrics::start_basic_prometheus_server;
use prometheus::Registry;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use sui_data_ingestion_core::{
    DataIngestionMetrics, FileProgressStore, IndexerExecutor, ReaderOptions, Worker, WorkerPool,
};
use sui_types::full_checkpoint_content::CheckpointData;
use tokio::sync::{oneshot, Mutex};
use tracing::{info, Level, error};
use async_trait::async_trait;
use anyhow::Result;
use suins_indexer::cetus_indexer::{
    CetusIndexer,
    start_volume_update_job,
    create_volume_data_table,
};
use suins_indexer::{PgConnectionPool, PgConnectionPoolExt, init_config, get_config};

/**
 * CetusIndexerWorker is the main worker that processes each checkpoint
 * It implements the Worker trait to handle checkpoint data processing
 */
struct CetusIndexerWorker {
    // Thread-safe reference to the Cetus indexer instance
    indexer: Arc<Mutex<CetusIndexer>>,
    // Database connection pool for storing processed data
    pg_pool: PgConnectionPool,
}

impl CetusIndexerWorker {
    /// Creates a new CetusIndexerWorker instance
    /// 
    /// # Arguments
    /// * `indexer` - Arc<Mutex<CetusIndexer>> for thread-safe access to the indexer
    /// * `pg_pool` - Database connection pool
    fn new(indexer: Arc<Mutex<CetusIndexer>>, pg_pool: PgConnectionPool) -> Self {
        Self {
            indexer,
            pg_pool,
        }
    }
}

/**
 * Implementation of the Worker trait for processing checkpoints
 * This is called for each checkpoint that needs to be processed
 */
#[async_trait]
impl Worker for CetusIndexerWorker {
    type Result = ();
    
    /// Process a single checkpoint and extract Cetus events
    /// 
    /// # Arguments
    /// * `checkpoint` - The checkpoint data containing all transactions
    /// 
    /// # Returns
    /// * `Result<()>` - Success or error result
    async fn process_checkpoint(&self, checkpoint: &CheckpointData) -> Result<()> {
        // Acquire exclusive access to the indexer (thread-safe)
        let mut indexer = self.indexer.lock().await;
        
        // Process the checkpoint and extract swap/liquidity events
        let (swap_events, liquidity_events) = indexer.process_checkpoint(checkpoint, Some(&self.pg_pool)).await;
        
        // Log detailed information if any events were found
        if !swap_events.is_empty() || !liquidity_events.is_empty() {
            info!("------------------------------------");
            info!("CHECKPOINT: {}", checkpoint.checkpoint_summary.sequence_number);
            info!("Timestamp: {}", checkpoint.checkpoint_summary.timestamp_ms);
            
            // Log detailed information about each swap event
            if !swap_events.is_empty() {
                info!("Found {} Cetus swap events", swap_events.len());
                for (idx, event) in swap_events.iter().enumerate() {
                    info!("SWAP EVENT #{}", idx + 1);
                    info!("Transaction: {}", event.transaction_digest);
                    info!("Pool ID: {}", event.pool_id);
                    info!("Amount In: {}", event.amount_in);
                    info!("Amount Out: {}", event.amount_out);
                    info!("Fee Amount: {}", event.fee_amount);
                    info!("Direction: {}", if event.atob { "USDC -> SUI" } else { "SUI -> USDC" });
                    info!("SUI Amount: {}", if event.atob { event.amount_out } else { event.amount_in });
                }
            }
            
            // Log detailed information about each liquidity event
            if !liquidity_events.is_empty() {
                info!("Found {} Cetus liquidity events", liquidity_events.len());
                for (idx, event) in liquidity_events.iter().enumerate() {
                    info!("LIQUIDITY EVENT #{}", idx + 1);
                    info!("Transaction: {}", event.transaction_digest);
                    info!("Pool ID: {}", event.pool_id);
                    info!("USDC Amount: {}", event.amount_a);
                    info!("SUI Amount: {}", event.amount_b);
                }
            }
            
            // Display current 24-hour metrics in millions for readability
            info!("💰 Current 24h Volume: ${:.2}", indexer.volume_24h.sui_usd_volume / 1_000_000.0);
            info!("💎 Current 24h TVL: ${:.2}", indexer.tvl_24h.total_usd_tvl / 1_000_000.0);
            info!("💵 Current 24h Fees: ${:.2}", indexer.fee_24h.fees_24h / 1_000_000.0);
            info!("------------------------------------");
        }
        
        Ok(())
    }
}

/**
 * Main function - Entry point of the application
 * Sets up logging, configuration, database, and starts checkpoint processing
 */
#[tokio::main]
async fn main() -> Result<()> {
    // Initialize structured logging with INFO level and timestamps
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)        
        .with_target(false)  // Don't show module targets
        .with_ansi(true)     // Enable colored output
        .init();
    
    // Load environment variables from .env file
    dotenv().ok();
    
    // Initialize application configuration from environment variables
    if let Err(err) = init_config() {
        error!("❌ Failed to initialize configuration: {}", err);
        std::process::exit(1);
    }
    
    // Get the validated configuration
    let config = get_config();
    
    // Determine checkpoints directory path
    // Priority: config file -> environment variable -> default path
    let checkpoints_dir = config.backfill_progress_file_path
        .as_ref()
        .map(|p| PathBuf::from(p).parent().unwrap().parent().unwrap().join("checkpoints").to_string_lossy().to_string())
        .unwrap_or_else(|| env::var("CHECKPOINTS_DIR").unwrap_or("/home/hungez/Documents/suins-indexer-1/checkpoints".to_string()));
    
    // Get remote storage configuration (for downloading checkpoints)
    let remote_storage = config.remote_storage.clone();
    
    // Get backfill progress file path (tracks last processed checkpoint)
    let backfill_progress_file_path = config.backfill_progress_file_path
        .clone()
        .unwrap_or_else(|| "/home/hungez/Documents/suins-indexer-1/backfill_progress/backfill_progress".to_string());
    
    // Get database connection string from configuration
    let database_url = &config.database_url;
    
    // Check if database functionality should be enabled
    let use_database = env::var("USE_DATABASE")
        .unwrap_or("false".to_string())
        .parse::<bool>()
        .unwrap_or(false);

    // Log startup information
    info!("🚀 Starting SUI-USDC Pool Volume & TVL Indexer"); 
    info!("📁 Checkpoints dir: {}", checkpoints_dir);
    info!("💾 Database enabled: {}", use_database);
    info!("🎯 Target pool: {}", config.sui_usdc_pool_id);

    // Create channels for graceful shutdown
    let (_exit_sender, exit_receiver) = oneshot::channel();
    
    // Set up progress tracking (remembers last processed checkpoint)
    let progress_store = FileProgressStore::new(PathBuf::from(backfill_progress_file_path));

    // Initialize Prometheus metrics server for monitoring
    let registry: Registry = start_basic_prometheus_server();
    let metrics = DataIngestionMetrics::new(&registry);
    
    // Create the main executor with 1 worker thread
    let mut executor = IndexerExecutor::new(progress_store, 1, metrics);

    // Create a new CetusIndexer instance wrapped in Arc<Mutex> for thread safety
    let indexer = Arc::new(Mutex::new(CetusIndexer::new()));
    
    // Setup database connection pool
    let diesel_pool = PgConnectionPool::new(database_url);
    
    // Initialize database and load existing data if database is enabled
    if use_database {
        // Create volume_data table if it doesn't exist
        if let Err(err) = create_volume_data_table(&diesel_pool).await {
            error!("❌ Failed to create volume_data table: {}", err);
        }
        
        // Load the last processed checkpoint from database
        let mut indexer_locked = indexer.lock().await;
        match CetusIndexer::get_last_processed_checkpoint(&diesel_pool).await {
            Ok(checkpoint) => {
                indexer_locked.last_processed_checkpoint = checkpoint;
                info!("✅ Loaded last processed checkpoint: {}", checkpoint);
            }
            Err(err) => {
                error!("❌ Failed to load last processed checkpoint: {}", err);
            }
        }
        
        // Load existing volume, TVL, and fee data from database
        if let Err(err) = indexer_locked.get_data_from_database(&diesel_pool).await {
            error!("❌ Failed to load data from database: {}", err);
        } else {
            info!("✅ Loaded data from database:");
            info!("   24h Volume: ${:.2}", indexer_locked.volume_24h.sui_usd_volume / 1_000_000.0);
            info!("   24h TVL: ${:.2}", indexer_locked.tvl_24h.total_usd_tvl / 1_000_000.0);
            info!("   24h Fees: ${:.2}", indexer_locked.fee_24h.fees_24h / 1_000_000.0);
        }
        
        // Start background job to update volume and TVL metrics every 10 minutes
        let job_indexer = indexer.clone();
        let job_pool = diesel_pool.clone();
        tokio::spawn(async move {
            start_volume_update_job(job_indexer, job_pool).await;
        });
    }

    // Create worker pool with 25 concurrent workers for processing
    let worker_pool = WorkerPool::new(
        CetusIndexerWorker::new(indexer.clone(), diesel_pool),
        "cetus_indexing".to_string(),
        25, // Number of concurrent workers
    );
    
    // Register the worker pool with the executor
    executor.register(worker_pool).await?;

    info!("⏳ Starting checkpoint processing...");
    
    // Start processing checkpoints
    // This will run indefinitely, processing new checkpoints as they arrive
    executor
        .run(
            PathBuf::from(checkpoints_dir),    // Local checkpoint storage
            remote_storage,                     // Remote checkpoint source
            vec![],                            // Additional checkpoint sources (empty)
            ReaderOptions::default(),          // Default reading options
            exit_receiver,                     // Graceful shutdown receiver
        )
        .await?;
    
    Ok(())
} 