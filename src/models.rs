// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::schema::{volume_data, swap_events, liquidity_events};
use diesel::prelude::*;
use chrono::NaiveDateTime;
use bigdecimal::BigDecimal;
use serde::{Deserialize, Serialize};

// Volume Data Models
#[derive(Queryable, Selectable, Debug, Serialize, Deserialize)]
#[diesel(table_name = volume_data)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VolumeDataRecord {
    pub id: i32,
    pub period: String,
    pub sui_usd_volume: BigDecimal,
    pub total_usd_tvl: BigDecimal,
    pub fees_24h: BigDecimal,
    pub last_update: NaiveDateTime,
    pub last_processed_checkpoint: i64,
}

#[derive(Insertable, AsChangeset, Debug)]
#[diesel(table_name = volume_data)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewVolumeDataRecord {
    pub period: String,
    pub sui_usd_volume: BigDecimal,
    pub total_usd_tvl: BigDecimal,
    pub fees_24h: BigDecimal,
    pub last_update: NaiveDateTime,
    pub last_processed_checkpoint: i64,
}

// Swap Event Models
#[derive(Queryable, Selectable, Debug, Serialize, Deserialize)]
#[diesel(table_name = swap_events)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct SwapEventRecord {
    pub id: i32,
    pub pool_id: String,
    pub amount_in: BigDecimal,
    pub amount_out: BigDecimal,
    pub atob: bool,
    pub fee_amount: BigDecimal,
    pub timestamp: NaiveDateTime,
    pub transaction_digest: String,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = swap_events)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewSwapEventRecord {
    pub pool_id: String,
    pub amount_in: BigDecimal,
    pub amount_out: BigDecimal,
    pub atob: bool,
    pub fee_amount: BigDecimal,
    pub timestamp: NaiveDateTime,
    pub transaction_digest: String,
}

// Liquidity Event Models
#[derive(Queryable, Selectable, Debug, Serialize, Deserialize)]
#[diesel(table_name = liquidity_events)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LiquidityEventRecord {
    pub id: i32,
    pub pool_id: String,
    pub amount_a: BigDecimal,
    pub amount_b: BigDecimal,
    pub timestamp: NaiveDateTime,
    pub transaction_digest: String,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = liquidity_events)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct NewLiquidityEventRecord {
    pub pool_id: String,
    pub amount_a: BigDecimal,
    pub amount_b: BigDecimal,
    pub timestamp: NaiveDateTime,
    pub transaction_digest: String,
}
