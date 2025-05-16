// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::schema::cetus_custom_indexer;
use diesel::prelude::*;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug)]
#[diesel(table_name = cetus_custom_indexer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CetusCustomIndexer {
    pub pool_id: String,
    pub total_volume_24h: String,
}
