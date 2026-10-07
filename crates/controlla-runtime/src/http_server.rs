use crate::{http_auth::Credential, jobs::Journal, mcp::App};
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use std::{net::SocketAddr, sync::Arc};

const DEFAULT_PORT: u16 = 8765;
const MAX_REQUEST_BODY_BYTES: usize = 16 * 1024 * 1024;

struct Settings {
    port: u16,
    credential: Credential,
    origins: Vec<String>,
}

impl Settings {
    fn from_env() -> Result<Self, String> {
        let port = match std::env::var("CHROME_CONTROLLA_HTTP_PORT") {
            Ok(value) => value
                .parse::<u16>()
                .map_err(|_| "CHROME_CONTROLLA_HTTP_PORT must be a valid port".to_owned())?,
            Err(std::env::VarError::NotPresent) => DEFAULT_PORT,
            Err(error) => return Err(format!("unable to read HTTP port: {error}")),
        };
        if port == 0 {
            return Err("CHROME_CONTROLLA_HTTP_PORT must be nonzero".into());
        }
        let token = required_env("CHROME_CONTROLLA_HTTP_TOKEN")?;
        if token.len() < 32 {
            return Err("CHROME_CONTROLLA_HTTP_TOKEN must contain at least 32 bytes".into());
        }
        let principal = required_env("CHROME_CONTROLLA_HTTP_PRINCIPAL")?;
        let configured_audience = match std::env::var("CHROME_CONTROLLA_HTTP_AUDIENCE") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(error) => return Err(format!("unable to read HTTP audience: {error}")),
        };
        let audience = endpoint_audience(port, configured_audience.as_deref())?;
        let credential = Credential::configured(&token, &principal, &audience)
            .ok_or_else(|| "HTTP token, principal, and audience must be non-empty".to_owned())?;
        let raw_origins = match std::env::var("CHROME_CONTROLLA_HTTP_ORIGINS") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => String::new(),
            Err(error) => return Err(format!("unable to read HTTP origins: {error}")),
        };
        let origins = if raw_origins.trim().is_empty() {
            Vec::new()
        } else {
            raw_origins
                .split(',')
                .map(str::trim)
                .map(str::to_owned)
                .collect()
        };
        if origins.iter().any(|origin| origin.is_empty()) {
            return Err("CHROME_CONTROLLA_HTTP_ORIGINS contains an empty entry".into());
        }
        Ok(Self {
            port,
            credential,
            origins,
        })
    }
}

fn endpoint_audience(port: u16, configured: Option<&str>) -> Result<String, String> {
    let expected = format!("http://127.0.0.1:{port}/mcp");
    if configured.is_some_and(|value| value != expected) {
        return Err(format!(
            "CHROME_CONTROLLA_HTTP_AUDIENCE must equal the bound endpoint {expected}"
        ));
    }
    Ok(expected)
}

fn required_env(name: &str) -> Result<String, String> {
    let value = std::env::var(name).map_err(|_| format!("{name} must be configured"))?;
    if value.trim().is_empty() {
        return Err(format!("{name} must be non-empty"));
    }
    Ok(value)
}

#[derive(Clone)]
struct AuthState {
    credential: Arc<Credential>,
    audience: Arc<str>,
    principal: Arc<str>,
    host: Arc<str>,
    origins: Arc<Vec<String>>,
}

fn http_router(
    app: App,
    credential: Credential,
    host: String,
    origins: Vec<String>,
) -> Result<Router, String> {
    let authority = app.principal().to_owned();
    if authority != credential.principal {
        return Err("HTTP credential principal does not match the MCP server principal".into());
    }
    let state = AuthState {
        audience: Arc::from(credential.audience.clone()),
        principal: Arc::from(authority),
        credential: Arc::new(credential.clone()),
        host: Arc::from(host.clone()),
        origins: Arc::new(origins.clone()),
    };
    let config = StreamableHttpServerConfig::default()
        .with_allowed_hosts([host])
        .with_allowed_origins(origins)
        .enforce_origin_validation()
        .with_max_request_body_bytes(MAX_REQUEST_BODY_BYTES)
        .with_json_response(true);
    let transport = StreamableHttpService::new(
        move || Ok(app.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Ok(Router::new()
        .route_service("/mcp", transport)
        .route_layer(middleware::from_fn_with_state(state, authenticate)))
}

async fn authenticate(
    State(state): State<AuthState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let headers = request.headers();
    let authorization_values = headers.get_all(header::AUTHORIZATION);
    let token = (authorization_values.iter().count() == 1)
        .then(|| authorization_values.iter().next())
        .flatten()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty());
    let host = match one_header_value(headers, header::HOST) {
        Ok(Some(host)) => host,
        _ => return StatusCode::FORBIDDEN.into_response(),
    };
    let origin = match one_header_value(headers, header::ORIGIN) {
        Ok(origin) => origin,
        Err(()) => return StatusCode::FORBIDDEN.into_response(),
    };
    let allowed_hosts = [state.host.as_ref()];
    let allowed_origins = state.origins.iter().map(String::as_str).collect::<Vec<_>>();
    if !allowed_hosts.contains(&host) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if origin.is_some_and(|value| !allowed_origins.contains(&value)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !state
        .credential
        .authorize(token, &state.audience, &state.principal)
    {
        let mut response = StatusCode::UNAUTHORIZED.into_response();
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            "Bearer".parse().expect("static header value"),
        );
        return response;
    }
    next.run(request).await
}

fn one_header_value(headers: &HeaderMap, name: header::HeaderName) -> Result<Option<&str>, ()> {
    let values = headers.get_all(name);
    let mut iter = values.iter();
    let Some(value) = iter.next() else {
        return Ok(None);
    };
    if iter.next().is_some() {
        return Err(());
    }
    value.to_str().map(Some).map_err(|_| ())
}

pub fn run(state_override: Option<std::path::PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let settings = Settings::from_env()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let state_dir = state_override.unwrap_or_else(crate::mcp::state_directory);
    std::fs::create_dir_all(&state_dir)?;
    let _state_lock = crate::mcp::acquire_state_directory_lock(&state_dir)?;
    let journal = Journal::open(state_dir.join("operations.sqlite"))?;
    journal.recover_after_restart()?;
    let app = App::with_principal_and_journal(settings.credential.principal.clone(), journal);
    let address = SocketAddr::from(([127, 0, 0, 1], settings.port));
    let router = http_router(
        app,
        settings.credential,
        format!("127.0.0.1:{}", settings.port),
        settings.origins,
    )
    .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async move {
        let listener = tokio::net::TcpListener::bind(address).await?;
        axum::serve(listener, router).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    const TOKEN: &str = "test-token-that-is-long-enough-for-http-auth";
    const PRINCIPAL: &str = "http-test-principal";
    const AUDIENCE: &str = "http://127.0.0.1:18876/mcp";
    const HOST: &str = "127.0.0.1:18876";

    fn router(origins: Vec<String>) -> Router {
        let jobs = Journal::open(":memory:").unwrap();
        let app = App::with_principal_and_journal(PRINCIPAL, jobs);
        let credential = Credential::configured(TOKEN, PRINCIPAL, AUDIENCE).unwrap();
        http_router(app, credential, HOST.into(), origins).unwrap()
    }

    #[test]
    fn rejects_credential_from_another_server_principal() {
        let jobs = Journal::open(":memory:").unwrap();
        let app = App::with_principal_and_journal(PRINCIPAL, jobs);
        let credential = Credential::configured(TOKEN, "different-principal", AUDIENCE).unwrap();
        assert!(http_router(app, credential, HOST.into(), Vec::new()).is_err());
    }

    #[test]
    fn audience_is_bound_to_the_loopback_endpoint() {
        assert_eq!(endpoint_audience(18_876, None).unwrap(), AUDIENCE);
        assert!(endpoint_audience(18_876, Some("http://other-host/mcp")).is_err());
    }

    fn request(
        body: Value,
        token: Option<&str>,
        host: &str,
        origin: Option<&str>,
        session_id: Option<&str>,
    ) -> Request<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, host)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("mcp-protocol-version", "2025-11-25");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(origin) = origin {
            request = request.header(header::ORIGIN, origin);
        }
        if let Some(session_id) = session_id {
            request = request.header("mcp-session-id", session_id);
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    async fn response_json(response: Response) -> Value {
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let bytes = to_bytes(response.into_body(), MAX_REQUEST_BODY_BYTES)
            .await
            .unwrap();
        if content_type.starts_with("text/event-stream") {
            let body = String::from_utf8(bytes.to_vec()).unwrap();
            let data = body
                .lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim))
                .find(|data| !data.is_empty())
                .unwrap_or_else(|| panic!("SSE response missing data event: {body:?}"));
            serde_json::from_str(data).unwrap_or_else(|error| {
                panic!("invalid SSE JSON ({content_type}): {body:?}: {error}")
            })
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                panic!("invalid JSON response ({content_type}): {bytes:?}: {error}")
            })
        }
    }

    #[tokio::test]
    async fn auth_host_and_origin_fail_closed_before_mcp_dispatch() {
        let router = router(vec!["http://127.0.0.1:18876".into()]);
        let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}});
        let missing_token = router
            .clone()
            .oneshot(request(body.clone(), None, HOST, None, None))
            .await
            .unwrap();
        assert_eq!(missing_token.status(), StatusCode::UNAUTHORIZED);
        let mut malformed = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header(header::HOST, HOST)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Basic bad")
            .body(Body::from(body.to_string()))
            .unwrap();
        malformed.headers_mut().append(
            header::AUTHORIZATION,
            format!("Bearer {TOKEN}").parse().unwrap(),
        );
        let duplicate_auth = router.clone().oneshot(malformed).await.unwrap();
        assert_eq!(duplicate_auth.status(), StatusCode::UNAUTHORIZED);
        let wrong_host = router
            .clone()
            .oneshot(request(
                body.clone(),
                Some(TOKEN),
                "attacker.test",
                None,
                None,
            ))
            .await
            .unwrap();
        assert_eq!(wrong_host.status(), StatusCode::FORBIDDEN);
        let wrong_origin = router
            .clone()
            .oneshot(request(
                body,
                Some(TOKEN),
                HOST,
                Some("https://attacker.test"),
                None,
            ))
            .await
            .unwrap();
        assert_eq!(wrong_origin.status(), StatusCode::FORBIDDEN);

        let mut duplicate_origin = request(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}),
            Some(TOKEN),
            HOST,
            Some("http://127.0.0.1:18876"),
            None,
        );
        duplicate_origin
            .headers_mut()
            .append(header::ORIGIN, "http://127.0.0.1:18876".parse().unwrap());
        let duplicate_origin = router.oneshot(duplicate_origin).await.unwrap();
        assert_eq!(duplicate_origin.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn authenticated_http_discovers_tools_and_reads_guide() {
        let router = router(Vec::new());
        let initialize = request(
            json!({
                "jsonrpc":"2.0","id":1,"method":"initialize",
                "params":{"protocolVersion":"2025-11-25","capabilities":{},
                    "clientInfo":{"name":"controlla-http-test","version":"0.1.0"}}
            }),
            Some(TOKEN),
            HOST,
            None,
            None,
        );
        let initialized = router.clone().oneshot(initialize).await.unwrap();
        assert_eq!(initialized.status(), StatusCode::OK);
        let session_id = initialized
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        response_json(initialized).await;

        let notification = router
            .clone()
            .oneshot(request(
                json!({"jsonrpc":"2.0","method":"notifications/initialized","params":{}}),
                Some(TOKEN),
                HOST,
                None,
                Some(&session_id),
            ))
            .await
            .unwrap();
        assert!(notification.status().is_success());

        let tools = router
            .clone()
            .oneshot(request(
                json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
                Some(TOKEN),
                HOST,
                None,
                Some(&session_id),
            ))
            .await
            .unwrap();
        assert_eq!(tools.status(), StatusCode::OK);
        let tools = response_json(tools).await;
        let list = tools["result"]["tools"].as_array().unwrap();
        assert!(list.iter().any(|tool| tool["name"] == "guide"));
        let slides = list
            .iter()
            .find(|tool| tool["name"] == "slides_plan_text_edit")
            .unwrap();
        assert!(slides["inputSchema"]["properties"]["principal"].is_null());

        let guide = router
            .oneshot(request(
                json!({
                    "jsonrpc":"2.0","id":3,"method":"tools/call",
                    "params":{"name":"guide","arguments":{"topic":"clients","server_version":"0.1.0"}}
                }),
                Some(TOKEN), HOST, None, Some(&session_id),
            ))
            .await
            .unwrap();
        assert_eq!(guide.status(), StatusCode::OK);
        assert!(matches!(
            guide
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            "application/json" | "text/event-stream"
        ));
        let guide = response_json(guide).await;
        assert!(
            guide["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Freebuff")
        );
    }
}
