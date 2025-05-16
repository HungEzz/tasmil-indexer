// @generated automatically by Diesel CLI.

diesel::table! {
    cetus_custom_indexer (pool_id) {
        pool_id -> Varchar,
        total_volume_24h -> Varchar,
    }
}

