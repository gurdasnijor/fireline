//! Peer wire transport.
//!
//! `prompt_peer` reaches the target peer by opening a normal ACP client
//! connection to the peer's hosted `/acp` endpoint, initializing, starting a
//! session, and sending a prompt. This stays on the SDK's normal live-session
//! path rather than introducing a second Fireline-specific peer protocol.

use anyhow::{Context, Result};
use fireline_acp_ids::SessionId;
use futures::{SinkExt, StreamExt};
use opentelemetry::{
    Context as OtelContext,
    propagation::{Extractor, Injector, TextMapPropagator},
    trace::TraceContextExt,
};
use opentelemetry_sdk::propagation::{BaggagePropagator, TraceContextPropagator};
use serde_json::{Map, Value};
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use super::Peer;

#[derive(Debug, Clone)]
pub(crate) struct PeerCallResult {
    pub child_session_id: SessionId,
    pub response_text: String,
    pub stop_reason: String,
}

#[derive(Debug, Clone)]
pub(crate) struct TraceContextCarrier {
    pub traceparent: String,
    pub tracestate: Option<String>,
    pub baggage: Option<String>,
}

impl TraceContextCarrier {
    pub(crate) fn from_current_span() -> Option<Self> {
        Self::from_span(&Span::current())
    }

    pub(crate) fn from_span(span: &Span) -> Option<Self> {
        Self::from_context(&span.context())
    }

    fn from_context(context: &OtelContext) -> Option<Self> {
        let mut meta = Map::new();
        {
            let mut injector = JsonMetaInjector(&mut meta);
            TraceContextPropagator::new().inject_context(context, &mut injector);
            BaggagePropagator::new().inject_context(context, &mut injector);
        }
        Self::from_meta(&meta)
    }

    fn from_meta(meta: &Map<String, Value>) -> Option<Self> {
        let traceparent = meta
            .get("traceparent")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();

        Some(Self {
            traceparent,
            tracestate: meta
                .get("tracestate")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            baggage: meta
                .get("baggage")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
        })
    }

    fn into_meta(self) -> Map<String, Value> {
        let mut meta = Map::new();
        meta.insert("traceparent".to_string(), Value::String(self.traceparent));
        if let Some(tracestate) = self.tracestate {
            meta.insert("tracestate".to_string(), Value::String(tracestate));
        }
        if let Some(baggage) = self.baggage {
            meta.insert("baggage".to_string(), Value::String(baggage));
        }
        meta
    }
}

struct JsonMetaInjector<'a>(&'a mut Map<String, Value>);

impl Injector for JsonMetaInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        self.0.insert(key.to_string(), Value::String(value));
    }
}

struct JsonMetaExtractor<'a>(&'a Map<String, Value>);

impl Extractor for JsonMetaExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

pub fn extract_remote_trace_context(line: &str) -> Option<OtelContext> {
    let meta = jsonrpc_meta(line)?;
    let extractor = JsonMetaExtractor(&meta);
    let context = TraceContextPropagator::new().extract(&extractor);
    let context = BaggagePropagator::new().extract_with_context(&context, &extractor);
    context.span().span_context().is_valid().then_some(context)
}

fn jsonrpc_meta(line: &str) -> Option<Map<String, Value>> {
    let value: Value = serde_json::from_str(line).ok()?;
    value
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(Value::as_object)
        .cloned()
}

struct WebSocketTransport {
    url: String,
}

impl sacp::ConnectTo<sacp::Client> for WebSocketTransport {
    async fn connect_to(
        self,
        client: impl sacp::ConnectTo<sacp::Agent>,
    ) -> Result<(), sacp::Error> {
        let (ws, _) = tokio_tungstenite::connect_async(self.url.as_str())
            .await
            .map_err(|e| sacp::util::internal_error(format!("WebSocket connect: {e}")))?;

        let (write, read) = StreamExt::split(ws);

        let outgoing = SinkExt::with(
            SinkExt::sink_map_err(write, std::io::Error::other),
            |line: String| async move {
                Ok::<_, std::io::Error>(tokio_tungstenite::tungstenite::Message::Text(line.into()))
            },
        );

        let incoming = StreamExt::filter_map(read, |msg| async move {
            match msg {
                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                    let line = text.trim().to_string();
                    if line.is_empty() {
                        None
                    } else {
                        Some(Ok(line))
                    }
                }
                Ok(tokio_tungstenite::tungstenite::Message::Binary(bytes)) => {
                    String::from_utf8(bytes.to_vec()).ok().and_then(|text| {
                        let line = text.trim().to_string();
                        if line.is_empty() {
                            None
                        } else {
                            Some(Ok(line))
                        }
                    })
                }
                Ok(_) => None,
                Err(err) => Some(Err(std::io::Error::other(err))),
            }
        });

        sacp::ConnectTo::<sacp::Client>::connect_to(sacp::Lines::new(outgoing, incoming), client)
            .await
    }
}

pub(crate) async fn dispatch_peer_call(
    peer: &Peer,
    prompt_text: &str,
    trace_context: Option<TraceContextCarrier>,
) -> Result<PeerCallResult> {
    let transport = WebSocketTransport {
        url: peer.acp_url.clone(),
    };
    let prompt_text = prompt_text.to_string();
    let init_request = initialize_request(trace_context);

    sacp::Client
        .builder()
        .name(format!("fireline-peer-client-{}", peer.agent_name))
        .on_receive_request(
            async move |req: agent_client_protocol::RequestPermissionRequest, responder, _cx| {
                let outcome = if let Some(opt) = req.options.first() {
                    agent_client_protocol::RequestPermissionOutcome::Selected(
                        agent_client_protocol::SelectedPermissionOutcome::new(
                            opt.option_id.clone(),
                        ),
                    )
                } else {
                    agent_client_protocol::RequestPermissionOutcome::Cancelled
                };
                responder.respond(agent_client_protocol::RequestPermissionResponse::new(outcome))
            },
            sacp::on_receive_request!(),
        )
        .connect_with(transport, async move |cx| {
            cx.send_request(init_request).block_task().await?;

            let session_new_span =
                tracing::info_span!("fireline.session.new", peer_agent_name = %peer.agent_name);
            let _session_new_guard = session_new_span.enter();
            cx.build_session(std::path::Path::new("."))
                .block_task()
                .run_until(async |mut session| {
                    let child_session_id = session.session_id().clone();
                    let session_prompt_span = tracing::info_span!(
                        "fireline.session.prompt",
                        session_id = %child_session_id,
                    );
                    let _session_prompt_guard = session_prompt_span.enter();
                    session.send_prompt(&prompt_text)?;

                    let mut response_text = String::new();
                    loop {
                        let update = session.read_update().await?;
                        match update {
                            sacp::SessionMessage::SessionMessage(dispatch) => {
                                sacp::util::MatchDispatch::new(dispatch)
                                    .if_notification(async |notif: agent_client_protocol::SessionNotification| {
                                        if let agent_client_protocol::SessionUpdate::AgentMessageChunk(
                                            agent_client_protocol::ContentChunk {
                                                content: agent_client_protocol::ContentBlock::Text(text),
                                                ..
                                            },
                                        ) = notif.update
                                        {
                                            response_text.push_str(&text.text);
                                        }
                                        Ok(())
                                    })
                                    .await
                                    .otherwise_ignore()?;
                            }
                            sacp::SessionMessage::StopReason(reason) => {
                                break Ok(PeerCallResult {
                                    child_session_id: child_session_id.clone(),
                                    response_text,
                                    stop_reason: format!("{reason:?}"),
                                });
                            }
                            _ => {}
                        }
                    }
                })
                .await
        })
        .await
        .map_err(anyhow::Error::from)
        .with_context(|| format!("dispatch prompt to peer {}", peer.agent_name))
}

fn initialize_request(
    trace_context: Option<TraceContextCarrier>,
) -> agent_client_protocol::InitializeRequest {
    let mut init = agent_client_protocol::InitializeRequest::new(
        agent_client_protocol::ProtocolVersion::LATEST,
    );

    let Some(trace_context) = trace_context else {
        return init;
    };

    init = init.meta(trace_context.into_meta());
    init
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_client_protocol::{AgentCapabilities, InitializeResponse, ProtocolVersion};
    use anyhow::Result;
    use futures::{sink, stream};
    use opentelemetry::trace::TraceContextExt as _;
    use serde_json::json;
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn dispatch_peer_call_injects_w3c_trace_context_into_initialize_meta() -> Result<()> {
        let transport = CaptureInitializeTransport::default();
        let capture = transport.capture.clone();

        sacp::Client
            .builder()
            .name("peer-capture")
            .connect_with(transport, async move |cx| {
                cx.send_request(initialize_request(Some(TraceContextCarrier {
                    traceparent: "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"
                        .to_string(),
                    tracestate: Some("vendor=value".to_string()),
                    baggage: Some("demo=true".to_string()),
                })))
                .block_task()
                .await?;
                Ok(())
            })
            .await?;

        let line = capture
            .lock()
            .expect("capture state poisoned")
            .clone()
            .expect("capture initialize request");
        let payload: Value = serde_json::from_str(&line)?;
        let meta = payload
            .get("params")
            .and_then(|params| params.get("_meta"))
            .and_then(Value::as_object)
            .expect("initialize request _meta");

        assert_eq!(
            meta.get("traceparent").and_then(Value::as_str),
            Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
        );
        assert_eq!(
            meta.get("tracestate").and_then(Value::as_str),
            Some("vendor=value")
        );
        assert_eq!(
            meta.get("baggage").and_then(Value::as_str),
            Some("demo=true")
        );
        Ok(())
    }

    #[test]
    fn extract_remote_trace_context_reads_root_meta_keys() {
        let line = json!({
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "initialize",
            "params": {
                "_meta": {
                    "traceparent": "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01",
                    "tracestate": "vendor=value",
                    "baggage": "demo=true"
                }
            }
        })
        .to_string();

        let context = extract_remote_trace_context(&line).expect("extract trace context");
        let span = context.span();
        let span_context = span.span_context();

        assert!(span_context.is_valid(), "trace context should be valid");
        assert_eq!(
            span_context.trace_id().to_string(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert_eq!(span_context.span_id().to_string(), "bbbbbbbbbbbbbbbb");
        assert!(
            span_context.is_remote(),
            "extracted context should be remote"
        );
    }

    #[derive(Clone, Default)]
    struct CaptureInitializeTransport {
        capture: Arc<Mutex<Option<String>>>,
    }

    impl sacp::ConnectTo<sacp::Client> for CaptureInitializeTransport {
        async fn connect_to(
            self,
            client: impl sacp::ConnectTo<sacp::Agent>,
        ) -> Result<(), sacp::Error> {
            let capture = self.capture.clone();
            let (response_tx, response_rx) =
                mpsc::unbounded_channel::<Result<String, std::io::Error>>();

            let outgoing = sink::unfold(response_tx, move |response_tx, line: String| {
                let capture = capture.clone();
                async move {
                    if capture.lock().expect("capture state poisoned").is_none() {
                        *capture.lock().expect("capture state poisoned") = Some(line.clone());
                        let request_id = serde_json::from_str::<Value>(&line)
                            .ok()
                            .and_then(|value| value.get("id").cloned())
                            .ok_or_else(|| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    "initialize request missing id",
                                )
                            })?;
                        let response = json!({
                            "jsonrpc": "2.0",
                            "id": request_id,
                            "result": InitializeResponse::new(ProtocolVersion::LATEST)
                                .agent_capabilities(AgentCapabilities::new()),
                        })
                        .to_string();
                        response_tx
                            .send(Ok(response))
                            .map_err(std::io::Error::other)?;
                    }
                    Ok::<_, std::io::Error>(response_tx)
                }
            });

            let incoming = stream::unfold(response_rx, |mut response_rx| async move {
                response_rx.recv().await.map(|line| (line, response_rx))
            });

            sacp::ConnectTo::<sacp::Client>::connect_to(
                sacp::Lines::new(outgoing, incoming),
                client,
            )
            .await
        }
    }
}
