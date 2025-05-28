-- This file should undo anything in `up.sql`

-- Rollback SuiNS Indexer Schema Migration
-- This migration drops all tables created by the up migration

-- Drop indexes first
DROP INDEX IF EXISTS idx_liquidity_events_tx_digest;
DROP INDEX IF EXISTS idx_liquidity_events_timestamp;
DROP INDEX IF EXISTS idx_liquidity_events_pool_id;

DROP INDEX IF EXISTS idx_swap_events_tx_digest;
DROP INDEX IF EXISTS idx_swap_events_timestamp;
DROP INDEX IF EXISTS idx_swap_events_pool_id;

DROP INDEX IF EXISTS idx_volume_data_checkpoint;
DROP INDEX IF EXISTS idx_volume_data_last_update;
DROP INDEX IF EXISTS idx_volume_data_period;

-- Drop tables
DROP TABLE IF EXISTS liquidity_events;
DROP TABLE IF EXISTS swap_events;
DROP TABLE IF EXISTS volume_data;
