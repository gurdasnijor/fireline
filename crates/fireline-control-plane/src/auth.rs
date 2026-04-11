use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use uuid::Uuid;

use crate::router::ControlPlaneError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTokenClaims {
    pub runtime_key: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct IssuedRuntimeToken {
    pub token: String,
    pub runtime_key: String,
    pub expires_at_ms: i64,
}

#[derive(Clone, Default)]
pub struct RuntimeTokenStore {
    inner: Arc<Mutex<HashMap<String, IssuedRuntimeToken>>>,
}

impl RuntimeTokenStore {
    pub fn issue(&self, runtime_key: &str, ttl: Duration) -> IssuedRuntimeToken {
        let issued = IssuedRuntimeToken {
            token: Uuid::new_v4().to_string(),
            runtime_key: runtime_key.to_string(),
            expires_at_ms: now_ms() + ttl.as_millis() as i64,
        };
        self.inner
            .lock()
            .expect("runtime token store poisoned")
            .insert(issued.token.clone(), issued.clone());
        issued
    }

    pub fn validate(&self, token: &str) -> Option<RuntimeTokenClaims> {
        let mut guard = self.inner.lock().expect("runtime token store poisoned");
        let issued = guard.get(token)?.clone();
        if issued.expires_at_ms < now_ms() {
            guard.remove(token);
            return None;
        }
        Some(RuntimeTokenClaims {
            runtime_key: issued.runtime_key,
            expires_at_ms: issued.expires_at_ms,
        })
    }
}

pub async fn require_runtime_bearer(
    State(token_store): State<RuntimeTokenStore>,
    mut request: Request,
    next: Next,
) -> Response {
    match extract_bearer_token(request.headers()).and_then(|token| {
        token_store
            .validate(token)
            .ok_or_else(|| ControlPlaneError::unauthorized("invalid or expired bearer token"))
    }) {
        Ok(claims) => {
            request.extensions_mut().insert(claims);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

fn extract_bearer_token(headers: &HeaderMap) -> Result<&str, ControlPlaneError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|raw| raw.to_str().ok())
        .ok_or_else(|| ControlPlaneError::unauthorized("missing Authorization header"))?;
    let token = value
        .strip_prefix("Bearer ")
        .ok_or_else(|| ControlPlaneError::unauthorized("expected Bearer token"))?;
    if token.is_empty() {
        return Err(ControlPlaneError::unauthorized("expected Bearer token"));
    }
    Ok(token)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Extension, Router};

    use super::{require_runtime_bearer, RuntimeTokenClaims, RuntimeTokenStore};

    #[test]
    fn issued_tokens_validate_until_expiry() {
        let store = RuntimeTokenStore::default();
        let issued = store.issue("runtime:test", Duration::from_secs(60));
        let claims = store
            .validate(&issued.token)
            .expect("fresh token should validate");
        assert_eq!(
            claims,
            RuntimeTokenClaims {
                runtime_key: "runtime:test".to_string(),
                expires_at_ms: issued.expires_at_ms,
            }
        );
    }

    #[tokio::test]
    async fn middleware_rejects_missing_bearer() {
        let app = Router::new()
            .route("/protected", get(|| async { StatusCode::NO_CONTENT }))
            .route_layer(axum::middleware::from_fn_with_state(
                RuntimeTokenStore::default(),
                require_runtime_bearer,
            ));

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test listener");
        let addr = listener
            .local_addr()
            .expect("resolve test listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve auth test app");
        });

        let response = reqwest::Client::new()
            .get(format!("http://{addr}/protected"))
            .send()
            .await
            .expect("request protected route");
        server.abort();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn middleware_inserts_runtime_claims_for_valid_token() {
        let token_store = RuntimeTokenStore::default();
        let issued = token_store.issue("runtime:test", Duration::from_secs(60));
        let app =
            Router::new()
                .route(
                    "/protected",
                    get(
                        |Extension(claims): Extension<RuntimeTokenClaims>| async move {
                            claims.runtime_key
                        },
                    ),
                )
                .route_layer(axum::middleware::from_fn_with_state(
                    token_store,
                    require_runtime_bearer,
                ));

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test listener");
        let addr = listener
            .local_addr()
            .expect("resolve test listener address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve auth test app");
        });

        let response = reqwest::Client::new()
            .get(format!("http://{addr}/protected"))
            .bearer_auth(&issued.token)
            .send()
            .await
            .expect("request protected route")
            .error_for_status()
            .expect("valid token should pass");
        let body = response.text().await.expect("read protected response body");
        server.abort();

        assert_eq!(body, "runtime:test");
    }
}
