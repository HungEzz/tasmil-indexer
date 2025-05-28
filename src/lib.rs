// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

//! SuiNS Indexer - A high-performance indexer for Sui blockchain data
//! 
//! This crate provides indexing capabilities for Sui blockchain events,
//! specifically focusing on Cetus AMM protocol data.

// Core modules
pub mod cetus_indexer;
pub mod config;
pub mod database;
pub mod models;
pub mod schema;

// Re-exports for convenience
pub use cetus_indexer::{CetusIndexer, SwapEvent, AddLiquidityEvent};
pub use config::{Config, init_config, get_config};
pub use database::DatabaseManager;
pub use models::*;

// Type aliases for better ergonomics
pub type PgConnectionPool = diesel_async::pooled_connection::bb8::Pool<diesel_async::AsyncPgConnection>;
pub type PgPoolConnection<'a> = diesel_async::pooled_connection::bb8::PooledConnection<'a, diesel_async::AsyncPgConnection>;

// Re-export commonly used types from dependencies
pub use sui_types::full_checkpoint_content::{CheckpointData, CheckpointTransaction};
pub use sui_types::effects::{TransactionEffects as CheckpointTransactionEffect};
pub use sui_types::transaction::TransactionDataAPI;
pub use sui_types::event::Event as CheckpointTransactionEffectEvent;

use dotenvy::dotenv;
use std::env;

use diesel::{ConnectionError, ConnectionResult};
use diesel_async::pooled_connection::bb8::Pool;
use diesel_async::pooled_connection::AsyncDieselConnectionManager;
use diesel_async::pooled_connection::ManagerConfig;
use diesel_async::AsyncPgConnection;
use futures_util::future::BoxFuture;
use futures_util::FutureExt;
use std::time::Duration;

// Extension trait to add new() method to PgConnectionPool
pub trait PgConnectionPoolExt {
    fn new(database_url: &str) -> Self;
}

impl PgConnectionPoolExt for PgConnectionPool {
    fn new(database_url: &str) -> Self {
        // Initialize rustls CryptoProvider
        let _ = rustls::crypto::ring::default_provider().install_default();
        
        let mut config = ManagerConfig::default();
        config.custom_setup = Box::new(establish_connection);

        let manager = AsyncDieselConnectionManager::<diesel_async::AsyncPgConnection>::new_with_config(
            database_url.to_string(),
            config,
        );

        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                Pool::builder()
                    .connection_timeout(Duration::from_secs(30))
                    .build(manager)
                    .await
                    .expect("Could not build Postgres DB connection pool")
            })
        })
    }
}

pub async fn get_connection_pool() -> PgConnectionPool {
    // Initialize rustls CryptoProvider
    rustls::crypto::ring::default_provider().install_default().unwrap();
    
    dotenv().ok();

    let database_url = env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let mut config = ManagerConfig::default();
    config.custom_setup = Box::new(establish_connection);

    let manager = AsyncDieselConnectionManager::<diesel_async::AsyncPgConnection>::new_with_config(
        database_url,
        config,
    );

    Pool::builder()
        .connection_timeout(Duration::from_secs(30))
        .build(manager)
        .await
        .expect("Could not build Postgres DB connection pool")
}

fn establish_connection(config: &str) -> BoxFuture<ConnectionResult<AsyncPgConnection>> {
    let fut = async {
        // We first set up the way we want rustls to work.
        let rustls_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(SkipServerCertCheck))
            .with_no_client_auth();
        let tls = tokio_postgres_rustls::MakeRustlsConnect::new(rustls_config);
        let (client, conn) = tokio_postgres::connect(config, tls)
            .await
            .map_err(|e| ConnectionError::BadConnection(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                eprintln!("Database connection: {e}");
            }
        });
        AsyncPgConnection::try_from(client).await
    };
    fut.boxed()
}

fn root_certs() -> rustls::RootCertStore {
    rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    }
}

/// Skip performing any verification of a server's cert in order to temporarily match the default
/// behavior of libpq
#[derive(Debug)]
struct SkipServerCertCheck;

impl rustls::client::danger::ServerCertVerifier for SkipServerCertCheck {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::client::WebPkiServerVerifier::builder(std::sync::Arc::new(root_certs()))
            .build()
            .unwrap()
            .supported_verify_schemes()
    }
}
