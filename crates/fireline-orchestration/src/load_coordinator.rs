//! `session/load` coordination proxy.
//!
//! This component keeps Fireline on the SDK's normal proxy path while making
//! durable session lookup explicit:
//!
//! - intercept `session/load`
//! - consult the materialized [`fireline_session::SessionIndex`]
//! - if the durable record is missing, return `resource_not_found`
//! - if the durable record exists but the downstream agent does not advertise
//!   `loadSession`, return an explicit `session_not_resumable` error with the
//!   durable session record attached under `error.data._meta.fireline`
//! - otherwise, forward `session/load` to the successor unchanged
//!
//! This slice deliberately does not claim cross-transport resume. It proves
//! durable catalog lookup and restart replay only.

use anyhow::Context as _;
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset};
use serde_json::{Value, json};

use fireline_session::{ChangeOperation, SessionIndex, SessionRecord, StateEnvelope};
use sacp::{Client, Conductor, ConnectTo, Proxy};

const SESSION_NOT_RESUMABLE_CODE: i32 = -32050;
const SESSION_NOT_RESUMABLE: &str = "session_not_resumable";
const SESSION_NOT_FOUND: &str = "session_not_found";
const REASON_DOWNSTREAM_LOAD_SESSION_UNSUPPORTED: &str = "downstream_load_session_unsupported";

#[derive(Debug, Clone)]
pub struct LoadCoordinatorComponent {
    session_index: SessionIndex,
    state_stream_url: String,
}

impl LoadCoordinatorComponent {
    pub fn new(session_index: SessionIndex, state_stream_url: impl Into<String>) -> Self {
        Self {
            session_index,
            state_stream_url: state_stream_url.into(),
        }
    }
}

impl ConnectTo<Conductor> for LoadCoordinatorComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let session_index = self.session_index;
        let state_stream_url = self.state_stream_url;

        sacp::Proxy
            .builder()
            .name("fireline-load-coordinator")
            .on_receive_request_from(
                Client,
                {
                    let session_index = session_index.clone();
                    let state_stream_url = state_stream_url.clone();
                    async move |request: sacp::schema::LoadSessionRequest, responder, cx| {
                        let session_id = request.session_id.to_string();
                        let record = match session_index.get(&session_id).await {
                            Some(record) => Some(record),
                            None => match find_session_record(&state_stream_url, &session_id).await {
                                Ok(Some(record)) => {
                                    session_index.upsert(record.clone()).await;
                                    Some(record)
                                }
                                Ok(None) => None,
                                Err(error) => {
                                    return responder.respond_with_error(sacp::util::internal_error(
                                        format!(
                                            "reload session '{session_id}' from durable state: {error:#}"
                                        ),
                                    ));
                                }
                            },
                        };
                        let Some(record) = record else {
                            return responder
                                .respond_with_error(session_not_found_error(&session_id));
                        };

                        if !record.supports_load_session {
                            return responder
                                .respond_with_error(session_not_resumable_error(&record));
                        }

                        cx.send_request_to(sacp::Agent, request)
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

async fn find_session_record(
    state_stream_url: &str,
    session_id: &str,
) -> anyhow::Result<Option<SessionRecord>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(state_stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
        .with_context(|| format!("build session/load replay reader for '{state_stream_url}'"))?;

    let mut latest = None;
    while let Some(chunk) = reader
        .next_chunk()
        .await
        .with_context(|| format!("read session/load replay stream '{state_stream_url}'"))?
    {
        if chunk.data.is_empty() {
            continue;
        }
        let events: Vec<Value> = match serde_json::from_slice(&chunk.data) {
            Ok(events) => events,
            Err(_) => continue,
        };
        for event in events {
            let envelope = match serde_json::from_value::<StateEnvelope>(event) {
                Ok(envelope) => envelope,
                Err(_) => continue,
            };
            if envelope.entity_type() != Some("session_v2") {
                continue;
            }
            match envelope.change_operation() {
                Some(ChangeOperation::Insert | ChangeOperation::Update) => {
                    let Some(value) = envelope.value.as_ref() else {
                        continue;
                    };
                    let record: SessionRecord = serde_json::from_value(value.clone())
                        .with_context(|| "decode session_v2 row while reloading session/load")?;
                    if record.session_id.to_string() == session_id {
                        latest = Some(record);
                    }
                }
                Some(ChangeOperation::Delete) => {
                    if envelope.key() == Some(session_id) {
                        latest = None;
                    }
                }
                _ => {}
            }
        }
    }

    Ok(latest)
}

fn session_not_resumable_error(record: &SessionRecord) -> sacp::Error {
    sacp::Error::new(SESSION_NOT_RESUMABLE_CODE, SESSION_NOT_RESUMABLE).data(json!({
        "_meta": {
            "fireline": {
                "error": SESSION_NOT_RESUMABLE,
                "reason": REASON_DOWNSTREAM_LOAD_SESSION_UNSUPPORTED,
                "sessionRecord": record,
            }
        }
    }))
}

fn session_not_found_error(session_id: &str) -> sacp::Error {
    sacp::Error::resource_not_found(Some(format!("acp://session/{session_id}"))).data(json!({
        "_meta": {
            "fireline": {
                "error": SESSION_NOT_FOUND,
                "sessionId": session_id,
            }
        }
    }))
}
