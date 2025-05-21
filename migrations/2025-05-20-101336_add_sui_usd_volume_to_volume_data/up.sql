-- Your SQL goes here

-- Add sui_usd_volume column to volume_data table
ALTER TABLE volume_data ADD COLUMN IF NOT EXISTS sui_usd_volume NUMERIC(30, 8) NOT NULL DEFAULT 0;
