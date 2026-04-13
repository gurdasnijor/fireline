use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::{
    Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer, StreamError,
};
use fireline_acp_ids::{RequestId, SessionId};
use reqwest::{
    StatusCode,
    header::{HeaderName, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::durable_subscriber::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, RetryPolicy,
    StreamEnvelope, TraceContext,
};

const TELEGRAM_SUBSCRIBER_NAME: &str = "telegram";
const TELEGRAM_APPROVE_CALLBACK_DATA: &str = "approve";
const TELEGRAM_DENY_CALLBACK_DATA: &str = "deny";
const TELEGRAM_CURSOR_TYPE: &str = "telegram_cursor";
const TELEGRAM_DEAD_LETTER_TYPE: &str = "telegram_dead_letter";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramScope {
    ToolCalls,
}

impl Default for TelegramScope {
    fn default() -> Self {
        Self::ToolCalls
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramParseMode {
    Html,
    MarkdownV2,
}

impl Default for TelegramParseMode {
    fn default() -> Self {
        Self::Html
    }
}

impl TelegramParseMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Html => "HTML",
            Self::MarkdownV2 => "MarkdownV2",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramSubscriberConfig {
    pub bot_token: String,
    pub api_base_url: String,
    pub chat_id: Option<String>,
    pub allowed_user_ids: BTreeSet<String>,
    pub approval_timeout: Option<Duration>,
    pub poll_interval: Duration,
    pub poll_timeout: Duration,
    pub parse_mode: TelegramParseMode,
    pub scope: TelegramScope,
    /// DSV-03/04 infrastructure-plane cursor stream used to persist Telegram
    /// polling offsets and the current in-flight approval card checkpoint.
    pub cursor_stream: Option<String>,
    /// DSV-04 infrastructure-plane dead-letter stream used for terminal
    /// Telegram delivery failures.
    pub dead_letter_stream: Option<String>,
    /// DSV-03 bounded retry policy for transient Telegram API failures.
    pub retry_policy: Option<RetryPolicy>,
}

impl TelegramSubscriberConfig {
    #[must_use]
    pub fn normalized_api_base_url(&self) -> String {
        self.api_base_url.trim_end_matches('/').to_string()
    }

    #[must_use]
    pub fn retry_policy(&self) -> RetryPolicy {
        self.retry_policy.unwrap_or(RetryPolicy {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(5),
        })
    }
}

#[derive(Clone)]
pub struct TelegramSubscriber {
    config: TelegramSubscriberConfig,
    http_client: reqwest::Client,
    state: Arc<Mutex<TelegramCursorRecord>>,
    poll_serial: Arc<Mutex<()>>,
}

impl TelegramSubscriber {
    #[must_use]
    pub fn new(config: TelegramSubscriberConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
            state: Arc::new(Mutex::new(TelegramCursorRecord::default())),
            poll_serial: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn config(&self) -> &TelegramSubscriberConfig {
        &self.config
    }

    /// DSV-01/03/04 durable dispatch path for Telegram approvals.
    ///
    /// It uses the canonical completion key to suppress duplicate sends on
    /// replay, persists the polling cursor in infra storage, and transitions to
    /// a dead-letter record after the bounded retry budget is exhausted.
    async fn dispatch_with_retry(
        &self,
        request: TelegramPermissionRequest,
        agent_log: &[StreamEnvelope],
        cursor_store: &dyn TelegramCursorStore,
        dead_letters: Option<&dyn TelegramDeadLetterSink>,
    ) -> Result<TelegramDispatchResult> {
        if self.is_completed(&request, agent_log) {
            return Ok(TelegramDispatchResult::Skipped(
                TelegramSkipReason::AlreadyCompleted,
            ));
        }

        let completion_key = request.completion_key();
        if let Some(dead_letters) = dead_letters {
            if dead_letters
                .contains(&completion_key)
                .await
                .context("check Telegram dead-letter state")?
            {
                return Ok(TelegramDispatchResult::Skipped(
                    TelegramSkipReason::AlreadyDeadLettered,
                ));
            }
        }

        let mut attempts = 0;
        loop {
            attempts += 1;
            match self.dispatch_once(request.clone(), cursor_store).await {
                HandlerOutcome::Completed(completion) => {
                    return Ok(TelegramDispatchResult::Delivered(TelegramDispatchOutcome {
                        completion,
                        attempts,
                    }));
                }
                HandlerOutcome::RetryTransient(error) => {
                    if let Some(backoff) = self.retry_policy().backoff_for_attempt(attempts) {
                        tokio::time::sleep(backoff).await;
                        continue;
                    }

                    let record = TelegramDeadLetterRecord::from_error(&request, attempts, error)?;
                    if let Some(dead_letters) = dead_letters {
                        dead_letters
                            .record(&record)
                            .await
                            .context("record Telegram dead-letter")?;
                    }
                    self.clear_in_flight(request.completion_key(), cursor_store)
                        .await?;
                    return Ok(TelegramDispatchResult::DeadLettered(record));
                }
                HandlerOutcome::Failed(error) => {
                    let record = TelegramDeadLetterRecord::from_error(&request, attempts, error)?;
                    if let Some(dead_letters) = dead_letters {
                        dead_letters
                            .record(&record)
                            .await
                            .context("record Telegram dead-letter")?;
                    }
                    self.clear_in_flight(request.completion_key(), cursor_store)
                        .await?;
                    return Ok(TelegramDispatchResult::DeadLettered(record));
                }
            }
        }
    }

    async fn dispatch_once(
        &self,
        request: TelegramPermissionRequest,
        cursor_store: &dyn TelegramCursorStore,
    ) -> HandlerOutcome<TelegramApprovalResolution> {
        match self.wait_for_resolution(request, cursor_store).await {
            Ok(completion) => HandlerOutcome::Completed(completion),
            Err(TelegramDispatchError::Transient(error)) => HandlerOutcome::RetryTransient(error),
            Err(TelegramDispatchError::Terminal(error)) => HandlerOutcome::Failed(error),
        }
    }

    async fn wait_for_resolution(
        &self,
        request: TelegramPermissionRequest,
        cursor_store: &dyn TelegramCursorStore,
    ) -> std::result::Result<TelegramApprovalResolution, TelegramDispatchError> {
        let _poll_guard = self.poll_serial.lock().await;
        let trace = request.meta.clone().unwrap_or_default();
        let mut cursor = self.load_cursor(cursor_store).await?;
        let chat_id = self
            .resolve_chat_id(&trace, cursor_store, &mut cursor)
            .await?;

        let completion_key = request.completion_key();
        let message_id = if let Some(in_flight) = cursor
            .in_flight
            .as_ref()
            .filter(|record| record.completion_key == completion_key)
        {
            in_flight.message_id
        } else {
            let message = self.post_approval_card(&chat_id, &request, &trace).await?;
            cursor.last_chat_id = Some(chat_id.clone());
            cursor.in_flight = Some(TelegramInFlightApproval::new(
                completion_key,
                chat_id.clone(),
                message.message_id,
                request.source_offset.clone(),
            ));
            self.store_cursor(cursor_store, &cursor).await?;
            message.message_id
        };

        let deadline = self
            .config
            .approval_timeout
            .map(|timeout| tokio::time::Instant::now() + timeout);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(TelegramDispatchError::terminal(anyhow!(
                        "timed out waiting for Telegram approval for session '{}' request '{}'",
                        request.session_id,
                        request_id_key(&request.request_id)
                    )));
                }
            }

            let batch = self.fetch_updates(&trace, cursor.next_update_id).await?;
            if let Some(chat_id) = batch.observed_chat_id.clone() {
                cursor.last_chat_id = Some(chat_id);
            }
            cursor.next_update_id = max_update_id(cursor.next_update_id, batch.next_update_id);

            if let Some(resolution) = self
                .match_resolution_update(&chat_id, message_id, &request, &batch.updates, &trace)
                .await?
            {
                cursor.in_flight = None;
                self.store_cursor(cursor_store, &cursor).await?;
                return Ok(resolution);
            }

            if batch.next_update_id.is_some() || batch.observed_chat_id.is_some() {
                self.store_cursor(cursor_store, &cursor).await?;
            }

            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    async fn resolve_chat_id(
        &self,
        trace: &TraceContext,
        cursor_store: &dyn TelegramCursorStore,
        cursor: &mut TelegramCursorRecord,
    ) -> std::result::Result<String, TelegramDispatchError> {
        if let Some(chat_id) = &self.config.chat_id {
            return Ok(chat_id.clone());
        }

        if let Some(in_flight) = &cursor.in_flight {
            return Ok(in_flight.chat_id.clone());
        }

        if let Some(chat_id) = &cursor.last_chat_id {
            return Ok(chat_id.clone());
        }

        let batch = self.fetch_updates(trace, cursor.next_update_id).await?;
        cursor.next_update_id = max_update_id(cursor.next_update_id, batch.next_update_id);
        if let Some(chat_id) = batch.observed_chat_id {
            cursor.last_chat_id = Some(chat_id.clone());
            self.store_cursor(cursor_store, cursor).await?;
            return Ok(chat_id);
        }

        if batch.next_update_id.is_some() {
            self.store_cursor(cursor_store, cursor).await?;
        }

        Err(TelegramDispatchError::terminal(anyhow!(
            "telegram subscriber requires config.chat_id or a prior Telegram chat update"
        )))
    }

    async fn post_approval_card(
        &self,
        chat_id: &str,
        request: &TelegramPermissionRequest,
        trace: &TraceContext,
    ) -> std::result::Result<TelegramMessage, TelegramDispatchError> {
        let reason = request
            .reason
            .as_deref()
            .unwrap_or("Fireline approval policy matched this prompt.");
        let payload = json!({
            "chat_id": chat_id,
            "text": render_approval_prompt(request, reason),
            "parse_mode": self.config.parse_mode.as_str(),
            "reply_markup": {
                "inline_keyboard": [[
                    { "text": "Approve", "callback_data": TELEGRAM_APPROVE_CALLBACK_DATA },
                    { "text": "Deny", "callback_data": TELEGRAM_DENY_CALLBACK_DATA }
                ]]
            }
        });
        self.telegram_api(trace, "sendMessage", &payload)
            .await
            .map_err(|error| error.context("send Telegram approval card"))
    }

    async fn fetch_updates(
        &self,
        trace: &TraceContext,
        next_update_id: Option<i64>,
    ) -> std::result::Result<TelegramUpdateBatch, TelegramDispatchError> {
        let timeout = self.config.poll_timeout.as_secs().min(i64::MAX as u64) as i64;
        let payload = json!({
            "allowed_updates": ["message", "callback_query"],
            "offset": next_update_id,
            "timeout": timeout,
        });
        let updates: Vec<TelegramUpdate> =
            self.telegram_api(trace, "getUpdates", &payload)
                .await
                .map_err(|error| error.context("poll Telegram updates"))?;

        let observed_chat_id = updates.iter().rev().find_map(TelegramUpdate::chat_id);
        let next_update_id = updates.last().map(|update| update.update_id + 1);
        Ok(TelegramUpdateBatch {
            updates,
            next_update_id,
            observed_chat_id,
        })
    }

    async fn match_resolution_update(
        &self,
        chat_id: &str,
        message_id: i64,
        request: &TelegramPermissionRequest,
        updates: &[TelegramUpdate],
        trace: &TraceContext,
    ) -> std::result::Result<Option<TelegramApprovalResolution>, TelegramDispatchError> {
        for update in updates {
            let Some(callback) = &update.callback_query else {
                continue;
            };
            let Some(callback_message) = &callback.message else {
                continue;
            };
            if callback_message.chat.id != chat_id || callback_message.message_id != message_id {
                continue;
            }

            if !self.user_is_allowed(&callback.from.id) {
                self.answer_callback_query(
                    &callback.id,
                    "This approval button is not assigned to you.",
                    trace,
                )
                .await?;
                continue;
            }

            let Some(allow) = decision_from_callback(callback.data.as_deref()) else {
                self.answer_callback_query(
                    &callback.id,
                    "Unsupported Telegram approval action.",
                    trace,
                )
                .await?;
                continue;
            };

            let resolved_by = resolver_identity(&callback.from);
            let answer_text = if allow { "Approved" } else { "Denied" };
            self.answer_callback_query(&callback.id, answer_text, trace)
                .await?;
            self.edit_approval_card(chat_id, message_id, request, allow, &resolved_by, trace)
                .await?;
            return Ok(Some(TelegramApprovalResolution::from_request(
                request,
                allow,
                resolved_by,
            )));
        }

        Ok(None)
    }

    async fn answer_callback_query(
        &self,
        callback_query_id: &str,
        text: &str,
        trace: &TraceContext,
    ) -> std::result::Result<(), TelegramDispatchError> {
        let payload = json!({
            "callback_query_id": callback_query_id,
            "text": text,
        });
        let _: bool = self
            .telegram_api(trace, "answerCallbackQuery", &payload)
            .await
            .map_err(|error| error.context("acknowledge Telegram callback query"))?;
        Ok(())
    }

    async fn edit_approval_card(
        &self,
        chat_id: &str,
        message_id: i64,
        request: &TelegramPermissionRequest,
        allow: bool,
        resolved_by: &str,
        trace: &TraceContext,
    ) -> std::result::Result<(), TelegramDispatchError> {
        let verdict = if allow { "Approved" } else { "Denied" };
        let payload = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": render_resolution_text(request, verdict, resolved_by),
            "parse_mode": self.config.parse_mode.as_str(),
        });
        let _: Value = self
            .telegram_api(trace, "editMessageText", &payload)
            .await
            .map_err(|error| error.context("edit Telegram approval card"))?;
        Ok(())
    }

    fn user_is_allowed(&self, user_id: &str) -> bool {
        self.config.allowed_user_ids.is_empty() || self.config.allowed_user_ids.contains(user_id)
    }

    async fn telegram_api<T>(
        &self,
        trace: &TraceContext,
        method: &str,
        payload: &Value,
    ) -> std::result::Result<T, TelegramDispatchError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!(
            "{}/bot{}/{}",
            self.config.normalized_api_base_url(),
            self.config.bot_token,
            method
        );
        let mut request = self
            .http_client
            .post(url)
            .json(payload)
            .header("content-type", "application/json");
        request = apply_trace_headers(request, trace)
            .map_err(TelegramDispatchError::terminal)
            .map_err(|error| error.context(format!("encode Telegram headers for '{method}'")))?;

        let response = request
            .send()
            .await
            .map_err(|error| TelegramDispatchError::transient(anyhow!(error)))
            .map_err(|error| error.context(format!("call Telegram Bot API method '{method}'")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|error| TelegramDispatchError::transient(anyhow!(error)))
            .map_err(|error| {
                error.context(format!("read Telegram Bot API response for '{method}'"))
            })?;

        if !status.is_success() {
            let description = telegram_error_description(&body);
            let error = anyhow!("Telegram {method} failed with HTTP {status}: {description}");
            return Err(if status_is_transient(status) {
                TelegramDispatchError::Transient(error)
            } else {
                TelegramDispatchError::Terminal(error)
            });
        }

        let envelope = serde_json::from_str::<TelegramApiEnvelope<T>>(&body)
            .map_err(anyhow::Error::from)
            .map_err(TelegramDispatchError::terminal)
            .map_err(|error| {
                error.context(format!("decode Telegram Bot API response for '{method}'"))
            })?;
        if !envelope.ok {
            return Err(TelegramDispatchError::terminal(anyhow!(
                "Telegram {method} returned ok=false: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_string())
            )));
        }
        envelope.result.ok_or_else(|| {
            TelegramDispatchError::terminal(anyhow!(
                "Telegram {method} returned no result: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "missing result".to_string())
            ))
        })
    }

    async fn load_cursor(
        &self,
        cursor_store: &dyn TelegramCursorStore,
    ) -> std::result::Result<TelegramCursorRecord, TelegramDispatchError> {
        cursor_store
            .load_cursor()
            .await
            .map_err(TelegramDispatchError::terminal)
            .map_err(|error| error.context("load Telegram cursor state"))
    }

    async fn store_cursor(
        &self,
        cursor_store: &dyn TelegramCursorStore,
        cursor: &TelegramCursorRecord,
    ) -> std::result::Result<(), TelegramDispatchError> {
        let mut snapshot = cursor.clone();
        snapshot.updated_at_ms = now_ms();
        cursor_store
            .store_cursor(&snapshot)
            .await
            .map_err(TelegramDispatchError::terminal)
            .map_err(|error| error.context("persist Telegram cursor state"))?;
        Ok(())
    }

    async fn clear_in_flight(
        &self,
        completion_key: CompletionKey,
        cursor_store: &dyn TelegramCursorStore,
    ) -> Result<()> {
        let mut cursor = cursor_store
            .load_cursor()
            .await
            .context("load Telegram cursor before clearing in-flight state")?;
        if cursor
            .in_flight
            .as_ref()
            .is_some_and(|record| record.completion_key == completion_key)
        {
            cursor.in_flight = None;
            cursor.updated_at_ms = now_ms();
            cursor_store
                .store_cursor(&cursor)
                .await
                .context("clear Telegram in-flight checkpoint")?;
        }
        Ok(())
    }
}

impl DurableSubscriber for TelegramSubscriber {
    type Event = TelegramPermissionRequest;
    type Completion = TelegramApprovalResolution;

    fn name(&self) -> &str {
        TELEGRAM_SUBSCRIBER_NAME
    }

    fn matches(&self, envelope: &StreamEnvelope) -> Option<Self::Event> {
        let event = decode_permission_event(envelope)?;
        (event.kind == "permission_request")
            .then_some(event.into_request(envelope.source_offset.clone()))
            .flatten()
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        event.completion_key()
    }

    fn is_completed(&self, event: &Self::Event, log: &[StreamEnvelope]) -> bool {
        log.iter().any(|envelope| {
            decode_permission_event(envelope)
                .and_then(TelegramPermissionEvent::into_resolution)
                .is_some_and(|resolution| {
                    resolution.session_id == event.session_id
                        && resolution.request_id == event.request_id
                })
        })
    }
}

#[async_trait]
impl ActiveSubscriber for TelegramSubscriber {
    async fn handle(&self, event: Self::Event) -> HandlerOutcome<Self::Completion> {
        let span = tracing::info_span!(
            "fireline.subscriber.handle",
            fireline.subscriber_name = TELEGRAM_SUBSCRIBER_NAME,
            fireline.completion_key_variant = "prompt",
            fireline.session_id = %event.session_id,
            fireline.request_id = %request_id_key(&event.request_id),
        );
        let _entered = span.enter();

        let durable_cursor_store;
        let memory_cursor_store;
        let cursor_store: &dyn TelegramCursorStore =
            if let Some(stream_url) = &self.config.cursor_stream {
                durable_cursor_store = DurableTelegramCursorStore::new(stream_url.clone());
                &durable_cursor_store
            } else {
                memory_cursor_store = MemoryTelegramCursorStore::new(self.state.clone());
                &memory_cursor_store
            };

        let durable_dead_letters;
        let dead_letters: Option<&dyn TelegramDeadLetterSink> =
            if let Some(stream_url) = &self.config.dead_letter_stream {
                durable_dead_letters = DurableTelegramDeadLetterSink::new(stream_url.clone());
                Some(&durable_dead_letters)
            } else {
                None
            };

        match self
            .dispatch_with_retry(event, &[], cursor_store, dead_letters)
            .await
        {
            Ok(TelegramDispatchResult::Delivered(outcome)) => {
                HandlerOutcome::Completed(outcome.completion)
            }
            Ok(TelegramDispatchResult::Skipped(TelegramSkipReason::AlreadyCompleted)) => {
                HandlerOutcome::Failed(anyhow!(
                    "telegram dispatch skipped because approval_resolved already exists for the canonical completion key"
                ))
            }
            Ok(TelegramDispatchResult::Skipped(TelegramSkipReason::AlreadyDeadLettered)) => {
                HandlerOutcome::Failed(anyhow!(
                    "telegram dispatch skipped because the canonical completion key is already dead-lettered"
                ))
            }
            Ok(TelegramDispatchResult::DeadLettered(record)) => HandlerOutcome::Failed(anyhow!(
                "telegram dispatch dead-lettered canonical key '{}' after {} attempts: {}",
                record.completion_key.storage_key(),
                record.attempts,
                record.last_error
            )),
            Err(error) => HandlerOutcome::Failed(error),
        }
    }

    fn retry_policy(&self) -> RetryPolicy {
        self.config.retry_policy()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramApprovalResolution {
    #[serde(default = "approval_resolved_kind")]
    pub kind: String,
    pub session_id: SessionId,
    pub request_id: RequestId,
    pub allow: bool,
    pub resolved_by: String,
    pub created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
}

impl TelegramApprovalResolution {
    fn from_request(request: &TelegramPermissionRequest, allow: bool, resolved_by: String) -> Self {
        Self {
            kind: approval_resolved_kind(),
            session_id: request.session_id.clone(),
            request_id: request.request_id.clone(),
            allow,
            resolved_by,
            created_at_ms: now_ms(),
            meta: request.meta.clone(),
        }
    }

    #[must_use]
    pub fn completion_key(&self) -> CompletionKey {
        CompletionKey::prompt(self.session_id.clone(), self.request_id.clone())
    }
}

pub async fn append_telegram_approval_resolution(
    producer: &Producer,
    completion: &TelegramApprovalResolution,
) -> Result<()> {
    producer.append_json(&json!({
        "type": "permission",
        "key": format!(
            "{}:{}:resolved",
            completion.session_id,
            request_id_key(&completion.request_id)
        ),
        "headers": insert_headers(),
        "value": completion,
    }));
    producer
        .flush()
        .await
        .context("flush Telegram approval resolution")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramPermissionRequest {
    pub kind: String,
    pub session_id: SessionId,
    pub request_id: RequestId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset: Option<String>,
}

impl TelegramPermissionRequest {
    #[must_use]
    pub fn completion_key(&self) -> CompletionKey {
        CompletionKey::prompt(self.session_id.clone(), self.request_id.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramInFlightApproval {
    pub completion_key: CompletionKey,
    pub chat_id: String,
    pub message_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset: Option<String>,
}

impl TelegramInFlightApproval {
    #[must_use]
    pub fn new(
        completion_key: CompletionKey,
        chat_id: String,
        message_id: i64,
        source_offset: Option<String>,
    ) -> Self {
        Self {
            completion_key,
            chat_id,
            message_id,
            source_offset,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramCursorRecord {
    pub subscriber: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_update_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_flight: Option<TelegramInFlightApproval>,
    pub updated_at_ms: i64,
}

impl Default for TelegramCursorRecord {
    fn default() -> Self {
        Self {
            subscriber: TELEGRAM_SUBSCRIBER_NAME.to_string(),
            next_update_id: None,
            last_chat_id: None,
            in_flight: None,
            updated_at_ms: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TelegramDeadLetterRecord {
    pub completion_key: CompletionKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_offset: Option<String>,
    pub attempts: u32,
    pub last_error: String,
    pub created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<TraceContext>,
}

impl TelegramDeadLetterRecord {
    fn from_error(
        request: &TelegramPermissionRequest,
        attempts: u32,
        error: anyhow::Error,
    ) -> Result<Self> {
        Ok(Self {
            completion_key: request.completion_key(),
            source_offset: request.source_offset.clone(),
            attempts,
            last_error: error.to_string(),
            created_at_ms: now_ms(),
            meta: request.meta.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramDispatchOutcome {
    pub completion: TelegramApprovalResolution,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TelegramDispatchResult {
    Skipped(TelegramSkipReason),
    Delivered(TelegramDispatchOutcome),
    DeadLettered(TelegramDeadLetterRecord),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelegramSkipReason {
    AlreadyCompleted,
    AlreadyDeadLettered,
}

#[async_trait]
trait TelegramCursorStore: Send + Sync {
    async fn load_cursor(&self) -> Result<TelegramCursorRecord>;
    async fn store_cursor(&self, record: &TelegramCursorRecord) -> Result<bool>;
}

#[async_trait]
trait TelegramDeadLetterSink: Send + Sync {
    async fn contains(&self, completion_key: &CompletionKey) -> Result<bool>;
    async fn record(&self, record: &TelegramDeadLetterRecord) -> Result<()>;
}

#[derive(Debug)]
pub struct DurableTelegramCursorStore {
    stream_url: String,
    cached: Mutex<Option<TelegramCursorRecord>>,
}

impl DurableTelegramCursorStore {
    #[must_use]
    pub fn new(stream_url: impl Into<String>) -> Self {
        Self {
            stream_url: stream_url.into(),
            cached: Mutex::new(None),
        }
    }

    async fn replay_cursor(&self) -> Result<TelegramCursorRecord> {
        let Some(rows) = read_all_json_rows(&self.stream_url).await? else {
            return Ok(TelegramCursorRecord::default());
        };

        let mut cursor = TelegramCursorRecord::default();
        for row in rows {
            let Ok(envelope) = serde_json::from_value::<StateEnvelope<TelegramCursorRecord>>(row)
            else {
                continue;
            };
            if envelope.entity_type != TELEGRAM_CURSOR_TYPE {
                continue;
            }
            cursor = merge_cursor_record(Some(&cursor), &envelope.value);
        }
        Ok(cursor)
    }
}

#[async_trait]
impl TelegramCursorStore for DurableTelegramCursorStore {
    async fn load_cursor(&self) -> Result<TelegramCursorRecord> {
        let mut cached = self.cached.lock().await;
        if cached.is_none() {
            *cached = Some(self.replay_cursor().await?);
        }
        Ok(cached.clone().unwrap_or_default())
    }

    async fn store_cursor(&self, record: &TelegramCursorRecord) -> Result<bool> {
        let mut cached = self.cached.lock().await;
        let merged = merge_cursor_record(cached.as_ref(), record);
        if cached.as_ref() == Some(&merged) {
            return Ok(false);
        }

        ensure_json_stream_exists(&self.stream_url)
            .await
            .with_context(|| format!("ensure Telegram cursor stream '{}'", self.stream_url))?;
        let producer = json_producer(&self.stream_url, "telegram-cursor");
        producer.append_json(&StateEnvelope {
            entity_type: TELEGRAM_CURSOR_TYPE.to_string(),
            key: TELEGRAM_SUBSCRIBER_NAME.to_string(),
            headers: StateHeaders {
                operation: "insert".to_string(),
            },
            value: merged.clone(),
        });
        producer
            .flush()
            .await
            .context("flush Telegram cursor state")?;

        *cached = Some(merged);
        Ok(true)
    }
}

#[derive(Debug)]
pub struct DurableTelegramDeadLetterSink {
    stream_url: String,
    known_keys: Mutex<Option<HashSet<String>>>,
}

impl DurableTelegramDeadLetterSink {
    #[must_use]
    pub fn new(stream_url: impl Into<String>) -> Self {
        Self {
            stream_url: stream_url.into(),
            known_keys: Mutex::new(None),
        }
    }

    async fn replay_keys(&self) -> Result<HashSet<String>> {
        let Some(rows) = read_all_json_rows(&self.stream_url).await? else {
            return Ok(HashSet::new());
        };

        let mut known = HashSet::new();
        for row in rows {
            let Ok(envelope) =
                serde_json::from_value::<StateEnvelope<TelegramDeadLetterRecord>>(row)
            else {
                continue;
            };
            if envelope.entity_type != TELEGRAM_DEAD_LETTER_TYPE {
                continue;
            }
            known.insert(dead_letter_storage_key(&envelope.value.completion_key));
        }
        Ok(known)
    }
}

#[async_trait]
impl TelegramDeadLetterSink for DurableTelegramDeadLetterSink {
    async fn contains(&self, completion_key: &CompletionKey) -> Result<bool> {
        let mut known_keys = self.known_keys.lock().await;
        if known_keys.is_none() {
            *known_keys = Some(self.replay_keys().await?);
        }
        Ok(known_keys
            .as_ref()
            .is_some_and(|keys| keys.contains(&dead_letter_storage_key(completion_key))))
    }

    async fn record(&self, record: &TelegramDeadLetterRecord) -> Result<()> {
        ensure_json_stream_exists(&self.stream_url)
            .await
            .with_context(|| format!("ensure Telegram dead-letter stream '{}'", self.stream_url))?;
        let producer = json_producer(&self.stream_url, "telegram-dead-letter");
        producer.append_json(&StateEnvelope {
            entity_type: TELEGRAM_DEAD_LETTER_TYPE.to_string(),
            key: dead_letter_storage_key(&record.completion_key),
            headers: StateHeaders {
                operation: "insert".to_string(),
            },
            value: record.clone(),
        });
        producer
            .flush()
            .await
            .context("flush Telegram dead-letter state")?;

        let mut known_keys = self.known_keys.lock().await;
        let keys = known_keys.get_or_insert_with(HashSet::new);
        keys.insert(dead_letter_storage_key(&record.completion_key));
        Ok(())
    }
}

#[derive(Debug)]
struct MemoryTelegramCursorStore {
    state: Arc<Mutex<TelegramCursorRecord>>,
}

impl MemoryTelegramCursorStore {
    fn new(state: Arc<Mutex<TelegramCursorRecord>>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl TelegramCursorStore for MemoryTelegramCursorStore {
    async fn load_cursor(&self) -> Result<TelegramCursorRecord> {
        Ok(self.state.lock().await.clone())
    }

    async fn store_cursor(&self, record: &TelegramCursorRecord) -> Result<bool> {
        let mut state = self.state.lock().await;
        let merged = merge_cursor_record(Some(&state), record);
        if *state == merged {
            return Ok(false);
        }
        *state = merged;
        Ok(true)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TelegramPermissionEvent {
    kind: String,
    session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allow: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    created_at_ms: i64,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    meta: Option<TraceContext>,
}

impl TelegramPermissionEvent {
    fn into_request(self, source_offset: Option<String>) -> Option<TelegramPermissionRequest> {
        Some(TelegramPermissionRequest {
            kind: self.kind,
            session_id: self.session_id,
            request_id: self.request_id?,
            reason: self.reason,
            created_at_ms: self.created_at_ms,
            meta: self.meta,
            source_offset,
        })
    }

    fn into_resolution(self) -> Option<TelegramApprovalResolution> {
        Some(TelegramApprovalResolution {
            kind: self.kind,
            session_id: self.session_id,
            request_id: self.request_id?,
            allow: self.allow?,
            resolved_by: self.resolved_by?,
            created_at_ms: self.created_at_ms,
            meta: self.meta,
        })
    }
}

fn decode_permission_event(envelope: &StreamEnvelope) -> Option<TelegramPermissionEvent> {
    (envelope.entity_type == "permission")
        .then(|| envelope.value_as::<TelegramPermissionEvent>())
        .flatten()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TelegramUpdateBatch {
    updates: Vec<TelegramUpdate>,
    next_update_id: Option<i64>,
    observed_chat_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramApiEnvelope<T> {
    ok: bool,
    result: Option<T>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TelegramUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<TelegramMessage>,
    #[serde(default)]
    callback_query: Option<TelegramCallbackQuery>,
}

impl TelegramUpdate {
    fn chat_id(&self) -> Option<String> {
        self.message
            .as_ref()
            .map(|message| message.chat.id.clone())
            .or_else(|| {
                self.callback_query
                    .as_ref()
                    .and_then(|callback| callback.message.as_ref())
                    .map(|message| message.chat.id.clone())
            })
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TelegramCallbackQuery {
    id: String,
    from: TelegramUser,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TelegramMessage {
    message_id: i64,
    chat: TelegramChat,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TelegramChat {
    #[serde(deserialize_with = "deserialize_string_like")]
    id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct TelegramUser {
    #[serde(deserialize_with = "deserialize_string_like")]
    id: String,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Debug)]
enum TelegramDispatchError {
    Transient(anyhow::Error),
    Terminal(anyhow::Error),
}

impl TelegramDispatchError {
    fn transient(error: impl Into<anyhow::Error>) -> Self {
        Self::Transient(error.into())
    }

    fn terminal(error: impl Into<anyhow::Error>) -> Self {
        Self::Terminal(error.into())
    }

    fn context(self, context: impl Into<String>) -> Self {
        let context = context.into();
        match self {
            Self::Transient(error) => Self::Transient(error.context(context)),
            Self::Terminal(error) => Self::Terminal(error.context(context)),
        }
    }
}

fn apply_trace_headers(
    mut request: reqwest::RequestBuilder,
    trace: &TraceContext,
) -> Result<reqwest::RequestBuilder> {
    if let Some(traceparent) = trace.traceparent.as_deref() {
        request = request.header(
            HeaderName::from_static("traceparent"),
            HeaderValue::from_str(traceparent).context("encode traceparent header")?,
        );
    }
    if let Some(tracestate) = trace.tracestate.as_deref() {
        request = request.header(
            HeaderName::from_static("tracestate"),
            HeaderValue::from_str(tracestate).context("encode tracestate header")?,
        );
    }
    if let Some(baggage) = trace.baggage.as_deref() {
        request = request.header(
            HeaderName::from_static("baggage"),
            HeaderValue::from_str(baggage).context("encode baggage header")?,
        );
    }
    Ok(request)
}

fn render_approval_prompt(request: &TelegramPermissionRequest, reason: &str) -> String {
    format!(
        "<b>Fireline approval required</b>\n\nReason: {reason}\nSession: <code>{}</code>\nRequest: <code>{}</code>",
        request.session_id,
        request_id_key(&request.request_id)
    )
}

fn render_resolution_text(
    request: &TelegramPermissionRequest,
    verdict: &str,
    resolved_by: &str,
) -> String {
    format!(
        "<b>{verdict}</b> by <code>{resolved_by}</code>\n\nSession: <code>{}</code>\nRequest: <code>{}</code>",
        request.session_id,
        request_id_key(&request.request_id)
    )
}

fn resolver_identity(user: &TelegramUser) -> String {
    user.username
        .as_deref()
        .map(|username| format!("telegram:@{username}"))
        .unwrap_or_else(|| format!("telegram:user-{}", user.id))
}

fn decision_from_callback(data: Option<&str>) -> Option<bool> {
    match data {
        Some(TELEGRAM_APPROVE_CALLBACK_DATA) => Some(true),
        Some(TELEGRAM_DENY_CALLBACK_DATA) => Some(false),
        _ => None,
    }
}

fn insert_headers() -> Map<String, Value> {
    let mut headers = Map::new();
    headers.insert("operation".to_string(), Value::String("insert".to_string()));
    headers
}

fn merge_cursor_record(
    current: Option<&TelegramCursorRecord>,
    candidate: &TelegramCursorRecord,
) -> TelegramCursorRecord {
    TelegramCursorRecord {
        subscriber: candidate.subscriber.clone(),
        next_update_id: max_update_id(
            current.and_then(|record| record.next_update_id),
            candidate.next_update_id,
        ),
        last_chat_id: candidate
            .last_chat_id
            .clone()
            .or_else(|| current.and_then(|record| record.last_chat_id.clone())),
        in_flight: candidate.in_flight.clone(),
        updated_at_ms: candidate.updated_at_ms,
    }
}

fn max_update_id(current: Option<i64>, candidate: Option<i64>) -> Option<i64> {
    match (current, candidate) {
        (Some(current), Some(candidate)) => Some(current.max(candidate)),
        (Some(current), None) => Some(current),
        (None, Some(candidate)) => Some(candidate),
        (None, None) => None,
    }
}

fn dead_letter_storage_key(completion_key: &CompletionKey) -> String {
    format!(
        "{TELEGRAM_SUBSCRIBER_NAME}:{}",
        completion_key.storage_key()
    )
}

fn telegram_error_description(body: &str) -> String {
    serde_json::from_str::<TelegramApiEnvelope<Value>>(body)
        .ok()
        .and_then(|envelope| envelope.description)
        .or_else(|| {
            let trimmed = body.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .unwrap_or_else(|| "unknown Telegram error".to_string())
}

fn status_is_transient(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

fn approval_resolved_kind() -> String {
    "approval_resolved".to_string()
}

fn request_id_key(request_id: &RequestId) -> String {
    match request_id {
        RequestId::Null => "null".to_string(),
        RequestId::Number(number) => number.to_string(),
        RequestId::Str(text) => text.clone(),
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}

fn json_producer(stream_url: &str, producer_name: &str) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("{producer_name}-{}", uuid::Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) | Err(StreamError::Conflict) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                tracing::debug!(?error, stream_url, "retrying Telegram stream creation");
            }
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("create Telegram stream '{stream_url}'"));
            }
        }
    }
}

async fn read_all_json_rows(stream_url: &str) -> Result<Option<Vec<Value>>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let mut reader = match stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
    {
        Ok(reader) => reader,
        Err(error) => {
            return Err(anyhow::Error::from(error)).context("build Telegram stream reader");
        }
    };

    let mut rows = Vec::new();
    loop {
        match reader.next_chunk().await {
            Ok(Some(chunk)) => {
                if !chunk.data.is_empty() {
                    let chunk_rows: Vec<Value> = serde_json::from_slice(&chunk.data)
                        .context("decode Telegram stream chunk as JSON array")?;
                    rows.extend(chunk_rows);
                }
                if chunk.up_to_date {
                    break;
                }
            }
            Ok(None) => break,
            Err(StreamError::NotFound { .. }) => return Ok(None),
            Err(error) => {
                return Err(anyhow::Error::from(error))
                    .with_context(|| format!("read Telegram stream '{stream_url}'"));
            }
        }
    }
    Ok(Some(rows))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: String,
    key: String,
    headers: StateHeaders,
    value: T,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct StateHeaders {
    operation: String,
}

fn deserialize_string_like<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    match value {
        Value::String(text) => Ok(text),
        Value::Number(number) => Ok(number.to_string()),
        other => Err(serde::de::Error::custom(format!(
            "expected string-like Telegram id, got {other}"
        ))),
    }
}
