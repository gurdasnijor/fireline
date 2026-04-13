use std::future::Future;
use std::pin::Pin;

use anyhow::{Result, anyhow};
use serde::de::DeserializeOwned;

use crate::awakeable::{AwakeableFuture, AwakeableKey, AwakeableResolution};
use crate::durable_subscriber::TraceContext;

/// First resolved branch from a race over multiple awakeables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwakeableRaceWinner<T> {
    pub winner_index: usize,
    pub winner_key: AwakeableKey,
    pub value: T,
    pub trace_context: Option<TraceContext>,
}

/// Promise.race-style sugar over multiple awakeables.
///
/// This is strictly additive: each branch is still an ordinary
/// `AwakeableFuture<T>`, and dropping the losing branches does not append any
/// extra stream state.
pub async fn race_awakeables<T, I>(awakeables: I) -> Result<AwakeableRaceWinner<T>>
where
    I: IntoIterator<Item = AwakeableFuture<T>>,
    T: DeserializeOwned + Send + 'static,
{
    let branches: Vec<
        Pin<Box<dyn Future<Output = (usize, Result<AwakeableResolution<T>>)> + Send>>,
    > = awakeables
        .into_iter()
        .enumerate()
        .map(|(index, awakeable)| {
            Box::pin(async move { (index, awakeable.into_resolution().await) })
                as Pin<Box<dyn Future<Output = (usize, Result<AwakeableResolution<T>>)> + Send>>
        })
        .collect();

    if branches.is_empty() {
        return Err(anyhow!("awakeable race requires at least one branch"));
    }

    let ((winner_index, resolution), _, _) = futures::future::select_all(branches).await;
    let resolution = resolution?;
    Ok(AwakeableRaceWinner {
        winner_index,
        winner_key: resolution.key,
        value: resolution.value,
        trace_context: resolution.trace_context,
    })
}
