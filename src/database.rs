// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use diesel::prelude::*;
use diesel_async::{RunQueryDsl, AsyncPgConnection};
use diesel::result::Error;
use diesel::upsert::excluded;
use chrono::Utc;
use bigdecimal::{BigDecimal, FromPrimitive};
use crate::models::{VolumeDataRecord, NewVolumeDataRecord, NewSwapEventRecord, NewLiquidityEventRecord};
use crate::schema::volume_data;

pub type PgPoolConnection<'a> = bb8::PooledConnection<'a, diesel_async::pooled_connection::AsyncDieselConnectionManager<AsyncPgConnection>>;

pub struct DatabaseManager;

impl DatabaseManager {
    /// Create database tables
    pub async fn create_tables(conn: &mut PgPoolConnection<'_>) -> Result<(), Error> {
        // Create volume_data table
        diesel::sql_query(r#"
            CREATE TABLE IF NOT EXISTS volume_data (
                id SERIAL PRIMARY KEY,
                period VARCHAR NOT NULL UNIQUE,
                sui_usd_volume NUMERIC NOT NULL,
                total_usd_tvl NUMERIC NOT NULL,
                fees_24h NUMERIC NOT NULL,
                last_update TIMESTAMP NOT NULL,
                last_processed_checkpoint BIGINT NOT NULL
            )
        "#).execute(conn).await?;

        // Create swap_events table
        diesel::sql_query(r#"
            CREATE TABLE IF NOT EXISTS swap_events (
                id SERIAL PRIMARY KEY,
                pool_id VARCHAR NOT NULL,
                amount_in NUMERIC NOT NULL,
                amount_out NUMERIC NOT NULL,
                atob BOOLEAN NOT NULL,
                fee_amount NUMERIC NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                transaction_digest VARCHAR NOT NULL UNIQUE
            )
        "#).execute(conn).await?;

        // Create liquidity_events table
        diesel::sql_query(r#"
            CREATE TABLE IF NOT EXISTS liquidity_events (
                id SERIAL PRIMARY KEY,
                pool_id VARCHAR NOT NULL,
                amount_a NUMERIC NOT NULL,
                amount_b NUMERIC NOT NULL,
                timestamp TIMESTAMP NOT NULL,
                transaction_digest VARCHAR NOT NULL UNIQUE
            )
        "#).execute(conn).await?;

        // Create indexes for better performance - Execute each index creation separately
        diesel::sql_query(r#"
            CREATE INDEX IF NOT EXISTS idx_swap_events_pool_id ON swap_events(pool_id)
        "#).execute(conn).await?;

        diesel::sql_query(r#"
            CREATE INDEX IF NOT EXISTS idx_swap_events_timestamp ON swap_events(timestamp)
        "#).execute(conn).await?;

        diesel::sql_query(r#"
            CREATE INDEX IF NOT EXISTS idx_liquidity_events_pool_id ON liquidity_events(pool_id)
        "#).execute(conn).await?;

        diesel::sql_query(r#"
            CREATE INDEX IF NOT EXISTS idx_liquidity_events_timestamp ON liquidity_events(timestamp)
        "#).execute(conn).await?;

        Ok(())
    }

    /// Update volume data in database
    pub async fn update_volume_data(
        conn: &mut PgPoolConnection<'_>,
        sui_usd_volume: f64,
        total_usd_tvl: f64,
        fees_24h: f64,
        last_processed_checkpoint: i64,
    ) -> Result<(), anyhow::Error> {
        let now = Utc::now().naive_utc();
        let record = NewVolumeDataRecord {
            period: "24h".to_string(),
            sui_usd_volume: BigDecimal::from_f64(sui_usd_volume).unwrap_or_default(),
            total_usd_tvl: BigDecimal::from_f64(total_usd_tvl).unwrap_or_default(),
            fees_24h: BigDecimal::from_f64(fees_24h).unwrap_or_default(),
            last_update: now,
            last_processed_checkpoint,
        };

        diesel::insert_into(volume_data::table)
            .values(&record)
            .on_conflict(volume_data::period)
            .do_update()
            .set((
                volume_data::sui_usd_volume.eq(excluded(volume_data::sui_usd_volume)),
                volume_data::total_usd_tvl.eq(excluded(volume_data::total_usd_tvl)),
                volume_data::fees_24h.eq(excluded(volume_data::fees_24h)),
                volume_data::last_processed_checkpoint.eq(excluded(volume_data::last_processed_checkpoint)),
            ))
            .execute(conn)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to update volume data: {}", e))?;

        Ok(())
    }

    /// Get volume data from database
    pub async fn get_volume_data(conn: &mut PgPoolConnection<'_>) -> Result<Option<VolumeDataRecord>, Error> {
        use crate::schema::volume_data::dsl::*;

        volume_data
            .filter(period.eq("24h"))
            .first::<VolumeDataRecord>(conn)
            .await
            .optional()
    }

    /// Get last processed checkpoint
    pub async fn get_last_processed_checkpoint(conn: &mut PgPoolConnection<'_>) -> Result<u64, Error> {
        use crate::schema::volume_data::dsl::*;

        let result = volume_data
            .filter(period.eq("24h"))
            .select(last_processed_checkpoint)
            .first::<i64>(conn)
            .await
            .optional()?;

        Ok(result.unwrap_or(0) as u64)
    }

    /// Insert multiple swap events
    pub async fn insert_swap_events(
        conn: &mut PgPoolConnection<'_>,
        events: &[crate::cetus_indexer::SwapEvent],
    ) -> Result<(), Error> {
        if events.is_empty() {
            return Ok(());
        }

        use crate::schema::swap_events::dsl::*;

        let records: Vec<NewSwapEventRecord> = events
            .iter()
            .map(|event| {
                let timestamp_naive = chrono::DateTime::<chrono::Utc>::from(event.timestamp).naive_utc();
                NewSwapEventRecord {
                    pool_id: event.pool_id.clone(),
                    amount_in: BigDecimal::from(event.amount_in),
                    amount_out: BigDecimal::from(event.amount_out),
                    atob: event.atob,
                    fee_amount: BigDecimal::from(event.fee_amount),
                    timestamp: timestamp_naive,
                    transaction_digest: event.transaction_digest.clone(),
                }
            })
            .collect();

        diesel::insert_into(swap_events)
            .values(&records)
            .on_conflict(transaction_digest)
            .do_nothing()
            .execute(conn)
            .await?;

        Ok(())
    }

    /// Insert multiple liquidity events
    pub async fn insert_liquidity_events(
        conn: &mut PgPoolConnection<'_>,
        events: &[crate::cetus_indexer::AddLiquidityEvent],
    ) -> Result<(), Error> {
        if events.is_empty() {
            return Ok(());
        }

        use crate::schema::liquidity_events::dsl::*;

        let records: Vec<NewLiquidityEventRecord> = events
            .iter()
            .map(|event| {
                let timestamp_naive = chrono::DateTime::<chrono::Utc>::from(event.timestamp).naive_utc();
                NewLiquidityEventRecord {
                    pool_id: event.pool_id.clone(),
                    amount_a: BigDecimal::from(event.amount_a),
                    amount_b: BigDecimal::from(event.amount_b),
                    timestamp: timestamp_naive,
                    transaction_digest: event.transaction_digest.clone(),
                }
            })
            .collect();

        diesel::insert_into(liquidity_events)
            .values(&records)
            .on_conflict(transaction_digest)
            .do_nothing()
            .execute(conn)
            .await?;

        Ok(())
    }
} 