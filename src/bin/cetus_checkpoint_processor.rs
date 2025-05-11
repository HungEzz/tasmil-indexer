// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use anyhow::Result;
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
use sqlx::postgres::PgPoolOptions;

use suins_indexer::cetus_indexer::{CetusIndexer, create_volume_data_table, start_volume_update_job};

struct CetusIndexerWorker {
    indexer: Arc<Mutex<CetusIndexer>>,
    pool: Option<sqlx::PgPool>,
}

impl CetusIndexerWorker {
    fn new(indexer: Arc<Mutex<CetusIndexer>>, pool: Option<sqlx::PgPool>) -> Self {
        Self { indexer, pool }
    }
}

#[async_trait]
impl Worker for CetusIndexerWorker {
    type Result = ();
    async fn process_checkpoint(&self, checkpoint: &CheckpointData) -> Result<()> {
        // Get a mutable reference to the indexer
        let mut indexer = self.indexer.lock().await;
        
        // Process the checkpoint
        let cetus_transactions = indexer.process_checkpoint(checkpoint, self.pool.as_ref()).await;
        
        if !cetus_transactions.is_empty() {
            info!("------------------------------------");
            info!("CHECKPOINT: {}", checkpoint.checkpoint_summary.sequence_number);
            info!("Found {} Cetus transactions", cetus_transactions.len());
            
            for (idx, tx) in cetus_transactions.iter().enumerate() {
                info!("TRANSACTION #{}", idx + 1);
                info!("Digest: {}", tx.transaction_digest);
                info!("Checkpoint: {}", tx.checkpoint_sequence_number);
                info!("------------------------------------");
            }
        }
        
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging with timestamps
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_ansi(true)
        .init();
    
    dotenv().ok();
    let checkpoints_dir = env::var("CHECKPOINTS_DIR").unwrap_or("/home/hungez/Documents/suins-indexer-1/checkpoints".to_string());
    let remote_storage = env::var("REMOTE_STORAGE").ok();
    let backfill_progress_file_path = env::var("BACKFILL_PROGRESS_FILE_PATH")
        .unwrap_or("/home/hungez/Documents/suins-indexer-1/backfill_progress/backfill_progress".to_string());
    
    // Database connection string
    let database_url = env::var("DATABASE_URL")
        .unwrap_or("postgres://postgres:postgres@localhost/suins_indexer".to_string());
    
    // Flag to enable/disable database
    let use_database = env::var("USE_DATABASE")
        .unwrap_or("false".to_string())
        .parse::<bool>()
        .unwrap_or(false);

    info!("Starting Cetus indexer with checkpoints dir: {}", checkpoints_dir);
    info!("Database enabled: {}", use_database);

    let (_exit_sender, exit_receiver) = oneshot::channel();
    let progress_store = FileProgressStore::new(PathBuf::from(backfill_progress_file_path));

    let registry: Registry = start_basic_prometheus_server();
    let metrics = DataIngestionMetrics::new(&registry);
    let mut executor = IndexerExecutor::new(progress_store, 1, metrics);

    // Create a new CetusIndexer instance
    let indexer = Arc::new(Mutex::new(CetusIndexer::new()));
    
    // Setup database if enabled
    let pool = if use_database {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
        
        // Create volume_data table if it doesn't exist
        if let Err(err) = create_volume_data_table(&pool).await {
            error!("Failed to create volume_data table: {}", err);
        }
        
        // Initialize volume from database if it exists
        let mut indexer_locked = indexer.lock().await;
        match CetusIndexer::get_last_processed_checkpoint(&pool).await {
            Ok(checkpoint) => {
                indexer_locked.last_processed_checkpoint = checkpoint;
                info!("Loaded last processed checkpoint: {}", checkpoint);
            }
            Err(err) => {
                error!("Failed to load last processed checkpoint: {}", err);
            }
        }
        
        match CetusIndexer::get_volume_24h_from_database(&pool).await {
            Ok(volume_data) => {
                let total_volume = volume_data.total_volume;
                indexer_locked.volume_24h = volume_data;
                info!("Loaded 24h volume from database: ${:.2}", total_volume);
            }
            Err(err) => {
                error!("Failed to load 24h volume from database: {}", err);
            }
        }
        
        // Start background job to update volume every 10 minutes
        let job_indexer = indexer.clone();
        let job_pool = pool.clone();
        tokio::spawn(async move {
            start_volume_update_job(job_indexer, job_pool).await;
        });
        
        Some(pool)
    } else {
        None
    };

    let worker_pool = WorkerPool::new(
        CetusIndexerWorker::new(indexer.clone(), pool),
        "cetus_indexing".to_string(),
        25, // lower concurrency since we're just logging
    );
    executor.register(worker_pool).await?;

    executor
        .run(
            PathBuf::from(checkpoints_dir),
            remote_storage,
            vec![],
            ReaderOptions::default(),
            exit_receiver,
        )
        .await?;
    
    Ok(())
} 