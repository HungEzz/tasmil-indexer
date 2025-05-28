// @generated automatically by Diesel CLI.

diesel::table! {
    volume_data (id) {
        id -> Int4,
        period -> Varchar,
        sui_usd_volume -> Numeric,
        total_usd_tvl -> Numeric,
        fees_24h -> Numeric,
        last_update -> Timestamp,
        last_processed_checkpoint -> Int8,
    }
}

diesel::table! {
    swap_events (id) {
        id -> Int4,
        pool_id -> Varchar,
        amount_in -> Numeric,
        amount_out -> Numeric,
        atob -> Bool,
        fee_amount -> Numeric,
        timestamp -> Timestamp,
        transaction_digest -> Varchar,
    }
}

diesel::table! {
    liquidity_events (id) {
        id -> Int4,
        pool_id -> Varchar,
        amount_a -> Numeric,
        amount_b -> Numeric,
        timestamp -> Timestamp,
        transaction_digest -> Varchar,
    }
}

diesel::allow_tables_to_appear_in_same_query!(
    volume_data,
    swap_events,
    liquidity_events,
);

