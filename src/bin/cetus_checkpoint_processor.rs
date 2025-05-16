// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use dotenvy::dotenv;
use mysten_service::metrics::start_basic_prometheus_server;
use prometheus::Registry;
use diesel::{dsl::sql, ExpressionMethods};
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
use sqlx::{Pool, Postgres};
use anyhow::Result;
use diesel_async::AsyncConnection;
use suins_indexer::cetus_indexer::{
     CetusIndexer,
     start_volume_update_job,
     create_volume_data_table,
     SwapEvent,
};
use suins_indexer::models::CetusCustomIndexer;
use suins_indexer::schema::cetus_custom_indexer;
use suins_indexer::{PgConnectionPool, PgConnectionPoolExt};
use diesel_async::RunQueryDsl;
use diesel_async::scoped_futures::ScopedFutureExt;
struct CetusIndexerWorker {
    indexer: Arc<Mutex<CetusIndexer>>,
    pg_pool: PgConnectionPool,
    sqlx_pool: Option<Pool<Postgres>>,
}

impl CetusIndexerWorker {
    fn new(indexer: Arc<Mutex<CetusIndexer>>, pg_pool: PgConnectionPool, sqlx_pool: Option<Pool<Postgres>>) -> Self {
        Self {
            indexer,
            pg_pool,
            sqlx_pool,
        }
    }

    async fn commit_to_db(
        &self,
        swap_events: &[SwapEvent],
    ) -> Result<()> {
        let mut connection = self.pg_pool.get().await.unwrap();

        // Convert SwapEvent to CetusCustomIndexer
        let cetus_custom_events: Vec<CetusCustomIndexer> = swap_events
            .iter()
            .map(|event| {
                let usdc_amount = if event.atob { event.amount_in } else { event.amount_out };
                CetusCustomIndexer {
                    pool_id: event.pool_id.clone(),
                    total_volume_24h: usdc_amount.to_string(),
                }
            })
            .collect();

        connection
            .transaction::<_, anyhow::Error, _>(|conn| {
                async move {
                    if !cetus_custom_events.is_empty() {
                        diesel_async::RunQueryDsl::execute(
                            diesel::insert_into(cetus_custom_indexer::table)
                                .values(&cetus_custom_events)
                                .on_conflict(cetus_custom_indexer::pool_id)
                                .do_update()
                                .set((
                                    cetus_custom_indexer::total_volume_24h.eq(sql("excluded.total_volume_24h")),
                                )),
                            conn
                        )
                        .await
                        .unwrap_or_else(|_| {
                            panic!("Failed to process swap events: {:?}", swap_events)
                        });
                    }

                    Ok(())
                }
                .scope_boxed()
            })
            .await
        }
}


#[async_trait]
impl Worker for CetusIndexerWorker {
    type Result = ();
    async fn process_checkpoint(&self, checkpoint: &CheckpointData) -> Result<()> {
        // Get a mutable reference to the indexer
        let mut indexer = self.indexer.lock().await;
        
        // Process the checkpoint - pass sqlx_pool as Option<&Pool<Postgres>>
        let swap_events = indexer.process_checkpoint(checkpoint, self.sqlx_pool.as_ref()).await;
        
        if !swap_events.is_empty() {
            info!("------------------------------------");
            info!("CHECKPOINT: {}", checkpoint.checkpoint_summary.sequence_number);
            info!("Timestamp: {}", checkpoint.checkpoint_summary.timestamp_ms);
            info!("Found {} Cetus swap events", swap_events.len());
            for (idx, event) in swap_events.iter().enumerate() {
                info!("SWAP EVENT #{}", idx + 1);
                info!("Transaction: {}", event.transaction_digest);
                info!("Pool ID: {}", event.pool_id);
                info!("Amount In: {}", event.amount_in);
                info!("Amount Out: {}", event.amount_out);
                info!("Direction: {}", if event.atob { "USDC -> SUI" } else { "SUI -> USDC" });
                info!("USDC Amount: {}", if event.atob { event.amount_in } else { event.amount_out });
                info!("------------------------------------");
            }
            
            // Log current 24h volume
            info!("💰 Current 24h USDC Volume: ${:.2}", indexer.get_usdc_volume_24h());
        }
        self.commit_to_db(&swap_events).await?;
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

    info!("🚀 Starting SUI-USDC Pool Volume Indexer"); 
    info!("📁 Checkpoints dir: {}", checkpoints_dir);
    info!("💾 Database enabled: {}", use_database);
    info!("🎯 Target pool: 0xb8d7d9e66a60c239e7a60110efcf8de6c705580ed924d0dde141f4a0e2c90105");

    let (_exit_sender, exit_receiver) = oneshot::channel();
    let progress_store = FileProgressStore::new(PathBuf::from(backfill_progress_file_path));

    let registry: Registry = start_basic_prometheus_server();
    let metrics = DataIngestionMetrics::new(&registry);
    let mut executor = IndexerExecutor::new(progress_store, 1, metrics);

    // Create a new CetusIndexer instance
    let indexer = Arc::new(Mutex::new(CetusIndexer::new()));
    
    // Setup database connection pools
    let diesel_pool = PgConnectionPool::new(&database_url);
    let sqlx_pool = if use_database {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await?;
            
        // Create volume_data table if it doesn't exist
        if let Err(err) = create_volume_data_table(&pool).await {
            error!("❌ Failed to create volume_data table: {}", err);
        }
        
        // Initialize volume from database if it exists
        let mut indexer_locked = indexer.lock().await;
        match CetusIndexer::get_last_processed_checkpoint(&pool).await {
            Ok(checkpoint) => {
                indexer_locked.last_processed_checkpoint = checkpoint;
                info!("✅ Loaded last processed checkpoint: {}", checkpoint);
            }
            Err(err) => {
                error!("❌ Failed to load last processed checkpoint: {}", err);
            }
        }
        
        match CetusIndexer::get_volume_24h_from_database(&pool).await {
            Ok(volume_data) => {
                let usdc_volume = volume_data.usdc_volume;
                indexer_locked.volume_24h = volume_data;
                info!("✅ Loaded 24h volume from database: ${:.2}", usdc_volume);
            }
            Err(err) => {
                error!("❌ Failed to load 24h volume from database: {}", err);
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
        CetusIndexerWorker::new(indexer.clone(), diesel_pool, sqlx_pool),
        "cetus_indexing".to_string(),
        25, // lower concurrency since we're just logging
    );
    executor.register(worker_pool).await?;

    info!("⏳ Starting checkpoint processing...");
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