-- SuiNS Indexer Schema Migration
-- This migration creates all necessary tables for the SuiNS Indexer

-- Create volume_data table for storing 24h volume, TVL, and fees data
CREATE TABLE IF NOT EXISTS volume_data (
    id SERIAL PRIMARY KEY,
    period VARCHAR(50) NOT NULL UNIQUE DEFAULT '24h',
    sui_usd_volume NUMERIC(30, 8) NOT NULL DEFAULT 0,
    total_usd_tvl NUMERIC(30, 8) NOT NULL DEFAULT 0,
    fees_24h NUMERIC(30, 8) NOT NULL DEFAULT 0,
    last_update TIMESTAMP NOT NULL DEFAULT NOW(),
    last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
);

-- Create swap_events table for storing Cetus swap events
CREATE TABLE IF NOT EXISTS swap_events (
    id SERIAL PRIMARY KEY,
    pool_id VARCHAR(66) NOT NULL,
    amount_in NUMERIC(30, 8) NOT NULL,
    amount_out NUMERIC(30, 8) NOT NULL,
    atob BOOLEAN NOT NULL,
    fee_amount NUMERIC(30, 8) NOT NULL DEFAULT 0,
    timestamp TIMESTAMP NOT NULL,
    transaction_digest VARCHAR(64) NOT NULL
);

-- Create liquidity_events table for storing Cetus liquidity events
CREATE TABLE IF NOT EXISTS liquidity_events (
    id SERIAL PRIMARY KEY,
    pool_id VARCHAR(66) NOT NULL,
    amount_a NUMERIC(30, 8) NOT NULL,
    amount_b NUMERIC(30, 8) NOT NULL,
    timestamp TIMESTAMP NOT NULL,
    transaction_digest VARCHAR(64) NOT NULL
);

-- Create indexes for better performance
CREATE INDEX IF NOT EXISTS idx_volume_data_period ON volume_data (period);
CREATE INDEX IF NOT EXISTS idx_volume_data_last_update ON volume_data (last_update);
CREATE INDEX IF NOT EXISTS idx_volume_data_checkpoint ON volume_data (last_processed_checkpoint);

CREATE INDEX IF NOT EXISTS idx_swap_events_pool_id ON swap_events (pool_id);
CREATE INDEX IF NOT EXISTS idx_swap_events_timestamp ON swap_events (timestamp);
CREATE INDEX IF NOT EXISTS idx_swap_events_tx_digest ON swap_events (transaction_digest);

CREATE INDEX IF NOT EXISTS idx_liquidity_events_pool_id ON liquidity_events (pool_id);
CREATE INDEX IF NOT EXISTS idx_liquidity_events_timestamp ON liquidity_events (timestamp);
CREATE INDEX IF NOT EXISTS idx_liquidity_events_tx_digest ON liquidity_events (transaction_digest);

-- Insert default volume_data record if not exists
INSERT INTO volume_data (period, sui_usd_volume, total_usd_tvl, fees_24h, last_update, last_processed_checkpoint)
VALUES ('24h', 0, 0, 0, NOW(), 0)
ON CONFLICT (period) DO NOTHING;
