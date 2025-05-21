-- This file should undo anything in `up.sql`

-- Remove sui_usd_volume column from volume_data table
ALTER TABLE volume_data DROP COLUMN IF EXISTS sui_usd_volume;
