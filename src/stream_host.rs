//! Embedded `durable-streams-server` instance.
//!
//! Fireline embeds the durable-streams server reference implementation
//! in the same process as the conductor so there is one axum listener
//! for the whole runtime. [`build_stream_router`] returns an
//! [`axum::Router`] ready to be `.merge()`d into the binary's
//! top-level Router.
//!
//! # Routes contributed
//!
//! - `GET /healthz` — server health check
//! - `/v1/stream/{name}` — the durable streams protocol surface
//!   (GET/PUT/HEAD/POST/DELETE per the spec)
//!
//! Those paths are namespaced enough not to collide with Fireline's
//! own routes (`/acp`, `/api/v1/files/*`), so merging (rather than
//! nesting) keeps client URLs aligned with the upstream convention
//! and with every off-the-shelf `durable-streams` client library.
//!
//! # Configuration
//!
//! Storage, limits, CORS, long-poll timeout, and SSE reconnect interval
//! come from the upstream `DS_*` environment variables via
//! [`durable_streams_server::config::Config::from_env`]. The DS config
//! `port` field is intentionally ignored — with Option A embedding,
//! Fireline's own axum listener owns the socket, not the DS server.
//!
//! The default storage mode is in-memory, matching the upstream
//! dev-mode posture documented at
//! <https://thesampaton.github.io/durable-streams-rust-server/deployment/dev-mode.html>.
//! For persistent runs, set `DS_STORAGE__MODE=file-durable` (or `acid`)
//! and `DS_STORAGE__DATA_DIR=/path/to/data`.

use anyhow::Result;
use axum::Router;
use durable_streams_server::{
    config::{Config as DsConfig, StorageMode},
    router,
    storage::{acid::AcidStorage, file::FileStorage, memory::InMemoryStorage},
};
use std::sync::Arc;

/// Build the embedded stream server router.
///
/// Reads storage and limit settings from `DS_*` environment variables,
/// validates the resulting config, and dispatches to the appropriate
/// [`durable_streams_server::storage`] backend. The returned Router is
/// ready to be `.merge()`d into the Fireline top-level Router.
///
/// # Errors
///
/// Returns an error if:
/// - a `DS_*` env var is set but invalid
/// - TLS or CORS config fails validation
/// - a file-backed storage backend cannot open its data directory
pub fn build_stream_router() -> Result<Router> {
    let ds_config = DsConfig::from_env()
        .map_err(|e| anyhow::anyhow!("load durable-streams config from env: {e}"))?;
    ds_config
        .validate()
        .map_err(|e| anyhow::anyhow!("validate durable-streams config: {e}"))?;

    tracing::info!(
        storage_mode = ds_config.storage_mode.as_str(),
        max_memory_bytes = ds_config.max_memory_bytes,
        max_stream_bytes = ds_config.max_stream_bytes,
        "embedding durable-streams-server"
    );

    let router = match ds_config.storage_mode {
        StorageMode::Memory => {
            let storage = Arc::new(InMemoryStorage::new(
                ds_config.max_memory_bytes,
                ds_config.max_stream_bytes,
            ));
            router::build_router(storage, &ds_config)
        }
        StorageMode::FileFast | StorageMode::FileDurable => {
            tracing::info!(
                data_dir = %ds_config.data_dir,
                sync_on_append = ds_config.storage_mode.sync_on_append(),
                "durable-streams file storage"
            );
            let storage = Arc::new(
                FileStorage::new(
                    &ds_config.data_dir,
                    ds_config.max_memory_bytes,
                    ds_config.max_stream_bytes,
                    ds_config.storage_mode.sync_on_append(),
                )
                .map_err(|e| anyhow::anyhow!("initialize file-backed stream storage: {e}"))?,
            );
            router::build_router(storage, &ds_config)
        }
        StorageMode::Acid => {
            tracing::info!(
                data_dir = %ds_config.data_dir,
                shard_count = ds_config.acid_shard_count,
                "durable-streams acid storage"
            );
            let storage = Arc::new(
                AcidStorage::new(
                    &ds_config.data_dir,
                    ds_config.acid_shard_count,
                    ds_config.max_memory_bytes,
                    ds_config.max_stream_bytes,
                )
                .map_err(|e| anyhow::anyhow!("initialize acid stream storage: {e}"))?,
            );
            router::build_router(storage, &ds_config)
        }
    };

    Ok(router)
}
