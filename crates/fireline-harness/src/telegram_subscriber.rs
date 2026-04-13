use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::Producer;
use fireline_acp_ids::{RequestId, SessionId};
use reqwest::header::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::durable_subscriber::{
    ActiveSubscriber, CompletionKey, DurableSubscriber, HandlerOutcome, StreamEnvelope,
    TraceContext,
};

const TELEGRAM_SUBSCRIBER_NAME: &str = "telegram";
const TELEGRAM_APPROVE_CALLBACK_DATA: &str = "approve";
const TELEGRAM_DENY_CALLBACK_DATA: &str = "deny";

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
}

impl TelegramSubscriberConfig {
    #[must_use]
    pub fn normalized_api_base_url(&self) -> String {
        self.api_base_url.trim_end_matches('/').to_string()
    }
}

#[derive(Clone)]
pub struct TelegramSubscriber {
    config: TelegramSubscriberConfig,
    http_client: reqwest::Client,
    state: Arc<Mutex<TelegramSubscriberState>>,
    poll_serial: Arc<Mutex<()>>,
}

impl TelegramSubscriber {
    #[must_use]
    pub fn new(config: TelegramSubscriberConfig) -> Self {
        Self {
            config,
            http_client: reqwest::Client::new(),
            state: Arc::new(Mutex::new(TelegramSubscriberState::default())),
            poll_serial: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn config(&self) -> &TelegramSubscriberConfig {
        &self.config
    }

    async fn wait_for_resolution(
        &self,
        request: TelegramPermissionRequest,
    ) -> Result<TelegramApprovalResolution> {
        let _poll_guard = self.poll_serial.lock().await;
        let trace = request.meta.clone().unwrap_or_default();
        let chat_id = self.resolve_chat_id(&trace).await?;
        let message = self.post_approval_card(&chat_id, &request, &trace).await?;
        self.remember_chat_id(chat_id.clone()).await;

        let deadline = self
            .config
            .approval_timeout
            .map(|timeout| tokio::time::Instant::now() + timeout);

        loop {
            if let Some(deadline) = deadline {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow!(
                        "timed out waiting for Telegram approval for session '{}' request '{}'",
                        request.session_id,
                        request_id_key(&request.request_id)
                    ));
                }
            }

            match self.fetch_updates(&trace).await {
                Ok(updates) => {
                    if let Some(resolution) = self
                        .match_resolution_update(
                            &chat_id,
                            message.message_id,
                            &request,
                            &updates,
                            &trace,
                        )
                        .await?
                    {
                        return Ok(resolution);
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        session_id = %request.session_id,
                        request_id = %request_id_key(&request.request_id),
                        "telegram subscriber polling failed; retrying within active wait"
                    );
                }
            }

            tokio::time::sleep(self.config.poll_interval).await;
        }
    }

    async fn resolve_chat_id(&self, trace: &TraceContext) -> Result<String> {
        if let Some(chat_id) = &self.config.chat_id {
            return Ok(chat_id.clone());
        }

        if let Some(chat_id) = self.state.lock().await.last_chat_id.clone() {
            return Ok(chat_id);
        }

        let updates = self.fetch_updates(trace).await?;
        if let Some(chat_id) = updates.iter().rev().find_map(TelegramUpdate::chat_id) {
            self.remember_chat_id(chat_id.clone()).await;
            return Ok(chat_id);
        }

        Err(anyhow!(
            "telegram subscriber requires config.chat_id or a prior Telegram chat update"
        ))
    }

    async fn post_approval_card(
        &self,
        chat_id: &str,
        request: &TelegramPermissionRequest,
        trace: &TraceContext,
    ) -> Result<TelegramMessage> {
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
            .context("send Telegram approval card")
    }

    async fn fetch_updates(&self, trace: &TraceContext) -> Result<Vec<TelegramUpdate>> {
        let offset = self.state.lock().await.next_update_id;
        let timeout = self.config.poll_timeout.as_secs().min(i64::MAX as u64) as i64;
        let payload = json!({
            "allowed_updates": ["message", "callback_query"],
            "offset": offset,
            "timeout": timeout,
        });
        let updates: Vec<TelegramUpdate> = self
            .telegram_api(trace, "getUpdates", &payload)
            .await
            .context("poll Telegram updates")?;

        let mut state = self.state.lock().await;
        for update in &updates {
            state.next_update_id = Some(update.update_id + 1);
            if let Some(chat_id) = update.chat_id() {
                state.last_chat_id = Some(chat_id);
            }
        }
        Ok(updates)
    }

    async fn match_resolution_update(
        &self,
        chat_id: &str,
        message_id: i64,
        request: &TelegramPermissionRequest,
        updates: &[TelegramUpdate],
        trace: &TraceContext,
    ) -> Result<Option<TelegramApprovalResolution>> {
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
    ) -> Result<()> {
        let payload = json!({
            "callback_query_id": callback_query_id,
            "text": text,
        });
        let _: bool = self
            .telegram_api(trace, "answerCallbackQuery", &payload)
            .await
            .context("acknowledge Telegram callback query")?;
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
    ) -> Result<()> {
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
            .context("edit Telegram approval card")?;
        Ok(())
    }

    async fn remember_chat_id(&self, chat_id: String) {
        self.state.lock().await.last_chat_id = Some(chat_id);
    }

    fn user_is_allowed(&self, user_id: &str) -> bool {
        self.config.allowed_user_ids.is_empty() || self.config.allowed_user_ids.contains(user_id)
    }

    async fn telegram_api<T>(
        &self,
        trace: &TraceContext,
        method: &str,
        payload: &Value,
    ) -> Result<T>
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
        request = apply_trace_headers(request, trace)?;
        let response = request
            .send()
            .await
            .with_context(|| format!("call Telegram Bot API method '{method}'"))?;
        let status = response.status();
        let envelope = response
            .json::<TelegramApiEnvelope<T>>()
            .await
            .with_context(|| format!("decode Telegram Bot API response for '{method}'"))?;
        if !status.is_success() {
            return Err(anyhow!(
                "Telegram {method} failed with HTTP {status}: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_string())
            ));
        }
        if !envelope.ok {
            return Err(anyhow!(
                "Telegram {method} returned ok=false: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "unknown Telegram error".to_string())
            ));
        }
        envelope.result.ok_or_else(|| {
            anyhow!(
                "Telegram {method} returned no result: {}",
                envelope
                    .description
                    .unwrap_or_else(|| "missing result".to_string())
            )
        })
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
            .then_some(event.into_request())
            .flatten()
    }

    fn completion_key(&self, event: &Self::Event) -> CompletionKey {
        CompletionKey::prompt(event.session_id.clone(), event.request_id.clone())
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
        match self.wait_for_resolution(event).await {
            Ok(completion) => HandlerOutcome::Completed(completion),
            Err(error) => HandlerOutcome::Failed(error),
        }
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
}

#[derive(Debug, Default)]
struct TelegramSubscriberState {
    last_chat_id: Option<String>,
    next_update_id: Option<i64>,
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
    fn into_request(self) -> Option<TelegramPermissionRequest> {
        Some(TelegramPermissionRequest {
            kind: self.kind,
            session_id: self.session_id,
            request_id: self.request_id?,
            reason: self.reason,
            created_at_ms: self.created_at_ms,
            meta: self.meta,
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
