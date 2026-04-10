//! `session/load` coordination proxy.
//!
//! This component keeps Fireline on the SDK's normal proxy path while making
//! durable session lookup explicit:
//!
//! - intercept `session/load`
//! - consult the materialized [`crate::session_index::SessionIndex`]
//! - if the durable record is missing, return `resource_not_found`
//! - if the durable record exists but the downstream agent does not advertise
//!   `loadSession`, return an explicit `session_not_resumable` error with the
//!   durable session record attached under `error.data._meta.fireline`
//! - otherwise, forward `session/load` to the successor unchanged
//!
//! This slice deliberately does not claim cross-transport resume. It proves
//! durable catalog lookup and restart replay only.

use serde_json::json;

use fireline_conductor::session::SessionRecord;
use sacp::{Client, Conductor, ConnectTo, Proxy};

#[derive(Debug, Clone)]
pub struct LoadCoordinatorComponent {
    session_index: crate::session_index::SessionIndex,
}

impl LoadCoordinatorComponent {
    pub fn new(session_index: crate::session_index::SessionIndex) -> Self {
        Self { session_index }
    }
}

impl ConnectTo<Conductor> for LoadCoordinatorComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let session_index = self.session_index;

        sacp::Proxy
            .builder()
            .name("fireline-load-coordinator")
            .on_receive_request_from(
                Client,
                {
                    let session_index = session_index.clone();
                    async move |request: sacp::schema::LoadSessionRequest, responder, cx| {
                        let session_id = request.session_id.to_string();
                        let Some(record) = session_index.get(&session_id).await else {
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

fn session_not_resumable_error(record: &SessionRecord) -> sacp::Error {
    sacp::Error::new(
        crate::error_codes::SESSION_NOT_RESUMABLE_CODE,
        crate::error_codes::SESSION_NOT_RESUMABLE,
    )
    .data(json!({
        "_meta": {
            "fireline": {
                "error": crate::error_codes::SESSION_NOT_RESUMABLE,
                "reason": crate::error_codes::REASON_DOWNSTREAM_LOAD_SESSION_UNSUPPORTED,
                "sessionRecord": record,
            }
        }
    }))
}

fn session_not_found_error(session_id: &str) -> sacp::Error {
    sacp::Error::resource_not_found(Some(format!("acp://session/{session_id}"))).data(json!({
        "_meta": {
            "fireline": {
                "error": crate::error_codes::SESSION_NOT_FOUND,
                "sessionId": session_id,
            }
        }
    }))
}
