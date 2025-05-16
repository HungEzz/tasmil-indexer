-- Your SQL goes here

CREATE TABLE IF NOT EXISTS volume_data (
    id SERIAL PRIMARY KEY,
    period VARCHAR(50) NOT NULL UNIQUE,
    total_volume DECIMAL(30, 8) NOT NULL,
    usdc_volume DECIMAL(30, 8) NOT NULL DEFAULT 0,
    last_update TIMESTAMP NOT NULL,
    last_processed_checkpoint BIGINT NOT NULL DEFAULT 0
);
