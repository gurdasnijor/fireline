#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use fireline_harness::WakeTimerRuntime;
use tokio::sync::Notify;

#[derive(Clone)]
pub struct RecordingWakeTimerRuntime {
    now_ms: Arc<Mutex<i64>>,
    sleeps: Arc<Mutex<Vec<Duration>>>,
}

impl RecordingWakeTimerRuntime {
    pub fn new(now_ms: i64) -> Self {
        Self {
            now_ms: Arc::new(Mutex::new(now_ms)),
            sleeps: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().expect("lock sleeps").clone()
    }
}

#[async_trait]
impl WakeTimerRuntime for RecordingWakeTimerRuntime {
    fn now_ms(&self) -> i64 {
        *self.now_ms.lock().expect("lock now_ms")
    }

    async fn sleep(&self, duration: Duration) {
        self.sleeps.lock().expect("lock sleeps").push(duration);
        *self.now_ms.lock().expect("lock now_ms") += duration.as_millis() as i64;
    }
}

#[derive(Clone)]
pub struct ControlledWakeTimerRuntime {
    now_ms: Arc<Mutex<i64>>,
    sleeps: Arc<Mutex<Vec<Duration>>>,
    sleep_started: Arc<Notify>,
    release_sleep: Arc<Notify>,
}

impl ControlledWakeTimerRuntime {
    pub fn new(now_ms: i64) -> Self {
        Self {
            now_ms: Arc::new(Mutex::new(now_ms)),
            sleeps: Arc::new(Mutex::new(Vec::new())),
            sleep_started: Arc::new(Notify::new()),
            release_sleep: Arc::new(Notify::new()),
        }
    }

    pub fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().expect("lock sleeps").clone()
    }

    pub async fn wait_for_sleep_started(&self) {
        self.sleep_started.notified().await;
    }

    pub fn release_sleep(&self) {
        self.release_sleep.notify_one();
    }
}

#[async_trait]
impl WakeTimerRuntime for ControlledWakeTimerRuntime {
    fn now_ms(&self) -> i64 {
        *self.now_ms.lock().expect("lock now_ms")
    }

    async fn sleep(&self, duration: Duration) {
        self.sleeps.lock().expect("lock sleeps").push(duration);
        self.sleep_started.notify_one();
        self.release_sleep.notified().await;
        *self.now_ms.lock().expect("lock now_ms") += duration.as_millis() as i64;
    }
}
