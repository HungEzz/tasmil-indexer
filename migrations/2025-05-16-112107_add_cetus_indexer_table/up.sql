-- Your SQL goes here

-- Create cetus_indexer table
CREATE TABLE cetus_indexer (
    id SERIAL PRIMARY KEY,
    total_volume_24h DECIMAL(30, 8) NOT NULL DEFAULT 0,
    last_update TIMESTAMP NOT NULL DEFAULT NOW(),
    last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
);

-- Add a unique index on last_update to ensure we have only the latest record
CREATE UNIQUE INDEX cetus_indexer_last_update_idx ON cetus_indexer (last_update);
