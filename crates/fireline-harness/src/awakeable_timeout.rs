use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::awakeable::AwakeableFuture;

/// Timeout ergonomics are intentionally signature-only in Phase 5.
///
/// Real timer append/consume behavior belongs to DS Phase 6's wake-timer
/// substrate; publishing that write path early would create write-only stream
/// traffic that nothing can complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwakeableTimeoutError {
    RequiresWakeTimerSubscriber,
}

impl fmt::Display for AwakeableTimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RequiresWakeTimerSubscriber => write!(
                f,
                "awakeable timeout requires DS Phase 6 WakeTimerSubscriber; Phase 5 only publishes the API signature"
            ),
        }
    }
}

impl std::error::Error for AwakeableTimeoutError {}

impl<T> AwakeableFuture<T>
where
    T: DeserializeOwned + Send + 'static,
{
    pub async fn with_timeout(
        self,
        _duration: Duration,
    ) -> std::result::Result<T, AwakeableTimeoutError> {
        let _ = self;
        Err(AwakeableTimeoutError::RequiresWakeTimerSubscriber)
    }
}
