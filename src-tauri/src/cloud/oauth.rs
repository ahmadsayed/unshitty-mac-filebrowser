//! Google's OAuth 2.0 Authorization Code flow with PKCE.
//!
//! The consent page must open in the system browser (Google rejects embedded
//! webviews), so the app listens on `127.0.0.1:<random>` for the redirect and
//! shows its own wait/cancel dialog. The token exchange is behind
//! [`TokenExchange`] so tests never open a socket to Google.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use oauth2::basic::BasicClient;
use oauth2::reqwest::blocking::{Client as HttpClient, ClientBuilder};
use oauth2::{
    AuthType, AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet,
    EndpointSet, PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, RefreshToken, Scope,
    TokenResponse, TokenUrl,
};
use url::Url;

use crate::cloud::token::Token;

/// Desktop-client credentials. Both are injected at build time from the
/// git-ignored `.env`; an installed app cannot keep the secret confidential,
/// and PKCE still protects the code exchange.
pub const CLIENT_ID: &str = match option_env!("FILEBROWSER_GOOGLE_CLIENT_ID") {
    Some(value) => value,
    None => "",
};
pub const CLIENT_SECRET: &str = match option_env!("FILEBROWSER_GOOGLE_CLIENT_SECRET") {
    Some(value) => value,
    None => "",
};

fn client_id() -> &'static str {
    if CLIENT_ID.is_empty() && cfg!(test) {
        "test-client-id.apps.googleusercontent.com"
    } else {
        CLIENT_ID
    }
}

fn client_secret() -> &'static str {
    if CLIENT_SECRET.is_empty() && cfg!(test) {
        "test-client-secret"
    } else {
        CLIENT_SECRET
    }
}

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";

pub const SCOPE_READONLY: &str = "https://www.googleapis.com/auth/drive.readonly";
/// Requested only by the write milestone, after a new consent.
pub const SCOPE_FULL: &str = "https://www.googleapis.com/auth/drive";

pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(180);

/// One sign-in attempt waiting for the browser. Stored in `CloudState` between
/// `drive_connect_start` and `drive_connect_finish`.
pub struct PendingFlow {
    pub id: String,
    pub auth_url: String,
    pub listener: TcpListener,
    pub redirect_uri: String,
    pub csrf: String,
    pub verifier: PkceCodeVerifier,
    pub cancel: Arc<AtomicBool>,
    pub created: Instant,
    claimed: AtomicBool,
}

impl PendingFlow {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Only one `finish` may wait on the callback. A second click gets a
    /// friendly error instead of two threads racing for the same connection.
    pub fn claim(&self) -> Result<(), String> {
        if self.claimed.swap(true, Ordering::SeqCst) {
            Err("This sign-in attempt is already in progress.".into())
        } else {
            Ok(())
        }
    }
}

/// Start a read-only sign-in. The caller stores the flow and returns the URL to
/// the frontend, which opens it with the system browser.
pub fn start(scopes: &[&str]) -> Result<PendingFlow, String> {
    if client_id().is_empty() || client_secret().is_empty() {
        return Err(
            "Google Drive sign-in is not configured. Add the OAuth client to .env and rebuild."
                .into(),
        );
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Could not start the Google sign-in listener: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| format!("Could not read the sign-in port: {error}"))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let client = google_client(client_id(), client_secret())?
        .set_auth_uri(AuthUrl::new(AUTH_URL.to_string()).map_err(|e| e.to_string())?)
        .set_redirect_uri(RedirectUrl::new(redirect_uri.clone()).map_err(|e| e.to_string())?);

    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge)
        // Google returns a refresh token only for an offline request, and only
        // reliably with an explicit consent prompt.
        .add_extra_param("access_type", "offline")
        .add_extra_param("prompt", "consent")
        .add_extra_param("include_granted_scopes", "true");
    for scope in scopes {
        request = request.add_scope(Scope::new((*scope).to_string()));
    }
    let (auth_url, csrf) = request.url();

    Ok(PendingFlow {
        id: format!("flow-{}", crate::cloud::accounts::new_account_id()),
        auth_url: auth_url.to_string(),
        listener,
        redirect_uri,
        csrf: csrf.secret().to_string(),
        verifier,
        cancel: Arc::new(AtomicBool::new(false)),
        created: Instant::now(),
        claimed: AtomicBool::new(false),
    })
}

/// Wait for the browser callback, verify `state`, and exchange the code.
/// Must run on a blocking thread; it polls the listener until `timeout`.
pub fn finish(
    flow: &PendingFlow,
    exchange: &dyn TokenExchange,
    timeout: Duration,
) -> Result<Token, String> {
    flow.listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not wait for Google sign-in: {error}"))?;
    let deadline = Instant::now() + timeout;

    while Instant::now() < deadline {
        if flow.is_cancelled() {
            return Err("Sign-in was cancelled.".into());
        }
        match flow.listener.accept() {
            Ok((mut stream, _)) => {
                let request = read_request(&mut stream)?;
                let Some(callback) = parse_callback(&request)? else {
                    // A browser may ask for /favicon.ico or another path first.
                    respond(&mut stream, 404, "Not found");
                    continue;
                };
                respond(
                    &mut stream,
                    200,
                    "You can close this window and return to File Browser.",
                );

                if let Some(error) = callback.error {
                    return Err(if error == "access_denied" {
                        "Sign-in was cancelled.".into()
                    } else {
                        format!("Google sign-in failed: {error}")
                    });
                }
                if callback.state.as_deref() != Some(flow.csrf.as_str()) {
                    return Err("Sign-in failed: the callback did not match this attempt.".into());
                }
                let Some(code) = callback.code else {
                    return Err("Sign-in failed: Google did not return a code.".into());
                };
                return exchange.exchange(&code, flow.verifier.secret(), &flow.redirect_uri);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(format!("Google sign-in failed: {error}")),
        }
    }

    Err("Sign-in timed out. Try again.".into())
}

/// The token endpoint, behind a trait so the callback tests can use a fake.
pub trait TokenExchange {
    fn exchange(&self, code: &str, verifier: &str, redirect_uri: &str) -> Result<Token, String>;
    fn refresh(&self, refresh_token: &str) -> Result<Token, String>;
}

pub struct GoogleExchange {
    client_id: String,
    client_secret: String,
}

impl GoogleExchange {
    pub fn new() -> Self {
        Self {
            client_id: client_id().to_string(),
            client_secret: client_secret().to_string(),
        }
    }
}

impl Default for GoogleExchange {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenExchange for GoogleExchange {
    fn exchange(&self, code: &str, verifier: &str, redirect_uri: &str) -> Result<Token, String> {
        let client = google_client(&self.client_id, &self.client_secret)?.set_redirect_uri(
            RedirectUrl::new(redirect_uri.to_string()).map_err(|e| e.to_string())?,
        );
        let http = http_client()?;
        let response = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .set_pkce_verifier(PkceCodeVerifier::new(verifier.to_string()))
            .request(&http)
            .map_err(|error| format!("Google sign-in failed: {error}"))?;
        Ok(to_token(response))
    }

    fn refresh(&self, refresh_token: &str) -> Result<Token, String> {
        let client = google_client(&self.client_id, &self.client_secret)?;
        let http = http_client()?;
        let response = client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
            .request(&http)
            .map_err(|error| format!("Google Drive session expired: {error}"))?;
        Ok(to_token(response))
    }
}

/// Best-effort revocation on disconnect. Local state is removed even if this
/// fails; a revoked/expired token is not an error the user can act on.
pub fn revoke(token: &str) {
    if let Ok(http) = http_client() {
        let _ = http.post(REVOKE_URL).form(&[("token", token)]).send();
    }
}

/// The Desktop client with Google's token endpoint. Google wants the secret in
/// the request body, not in a Basic auth header.
fn google_client(
    id: &str,
    secret: &str,
) -> Result<
    BasicClient<EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>,
    String,
> {
    Ok(BasicClient::new(ClientId::new(id.to_string()))
        .set_client_secret(ClientSecret::new(secret.to_string()))
        .set_auth_type(AuthType::RequestBody)
        .set_token_uri(TokenUrl::new(TOKEN_URL.to_string()).map_err(|e| e.to_string())?))
}

fn http_client() -> Result<HttpClient, String> {
    ClientBuilder::new()
        // Following redirects on the token endpoint would be an SSRF footgun.
        .redirect(oauth2::reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("Could not create the Google HTTP client: {error}"))
}

fn to_token(response: impl TokenResponse) -> Token {
    let now = crate::recent::now();
    let lifetime = response
        .expires_in()
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(3600);
    let scopes = response
        .scopes()
        .map(|scopes| scopes.iter().map(|scope| scope.to_string()).collect())
        .unwrap_or_default();
    Token::new(
        response.access_token().secret().to_string(),
        response
            .refresh_token()
            .map(|token| token.secret().to_string()),
        now + lifetime.saturating_sub(60).max(1),
        scopes,
    )
}

struct Callback {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

fn parse_callback(request: &str) -> Result<Option<Callback>, String> {
    let Some(target) = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
    else {
        return Ok(None);
    };
    let url = Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|error| format!("Invalid Google sign-in callback: {error}"))?;

    let mut callback = Callback {
        code: None,
        state: None,
        error: None,
    };
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => callback.code = Some(value.into_owned()),
            "state" => callback.state = Some(value.into_owned()),
            "error" => callback.error = Some(value.into_owned()),
            _ => {}
        }
    }

    if callback.code.is_none() && callback.state.is_none() && callback.error.is_none() {
        return Ok(None);
    }
    Ok(Some(callback))
}

fn read_request(stream: &mut TcpStream) -> Result<String, String> {
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut buffer = [0u8; 16 * 1024];
    let read = stream
        .read(&mut buffer)
        .map_err(|error| format!("Google sign-in callback failed: {error}"))?;
    Ok(String::from_utf8_lossy(&buffer[..read]).into_owned())
}

fn respond(stream: &mut TcpStream, status: u16, message: &str) {
    let reason = if status == 200 { "OK" } else { "Not Found" };
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>File Browser</title><p>{message}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::thread;

    struct FakeExchange {
        seen: Mutex<Vec<(String, String, String)>>,
    }

    impl FakeExchange {
        fn new() -> Self {
            Self {
                seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl TokenExchange for FakeExchange {
        fn exchange(
            &self,
            code: &str,
            verifier: &str,
            redirect_uri: &str,
        ) -> Result<Token, String> {
            self.seen
                .lock()
                .unwrap()
                .push((code.into(), verifier.into(), redirect_uri.into()));
            Ok(Token::new(
                "access".into(),
                Some("refresh".into()),
                crate::recent::now() + 3600,
                vec![SCOPE_READONLY.into()],
            ))
        }

        fn refresh(&self, _refresh_token: &str) -> Result<Token, String> {
            unreachable!("the callback tests do not refresh")
        }
    }

    fn call(addr: std::net::SocketAddr, target: &str) {
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut buffer = [0u8; 256];
        let _ = stream.read(&mut buffer);
    }

    #[test]
    fn the_auth_url_has_pkce_offline_and_the_shipped_client() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let url = Url::parse(&flow.auth_url).unwrap();
        let param = |name: &str| {
            url.query_pairs()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.into_owned())
        };
        assert_eq!(url.host_str(), Some("accounts.google.com"));
        assert_eq!(param("client_id").as_deref(), Some(client_id()));
        assert!(
            param("client_secret").is_none(),
            "the secret never goes in the auth URL"
        );
        assert!(!client_secret().is_empty());
        assert_eq!(param("response_type").as_deref(), Some("code"));
        assert_eq!(param("code_challenge_method").as_deref(), Some("S256"));
        assert_eq!(param("access_type").as_deref(), Some("offline"));
        assert_eq!(param("prompt").as_deref(), Some("consent"));
        assert_eq!(
            param("redirect_uri").as_deref(),
            Some(flow.redirect_uri.as_str())
        );
        assert!(param("scope").unwrap().contains(SCOPE_READONLY));
        assert!(param("code_challenge").is_some());
        assert!(param("state").is_some());
        assert!(flow.id.starts_with("flow-"));
    }

    #[test]
    fn a_matching_callback_is_exchanged_once() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let addr = flow.listener.local_addr().unwrap();
        let state = flow.csrf.clone();
        let worker = thread::spawn(move || {
            call(addr, &format!("/?code=the-code&state={state}"));
        });

        let exchange = FakeExchange::new();
        let token = finish(&flow, &exchange, Duration::from_secs(2)).unwrap();
        worker.join().unwrap();

        assert_eq!(token.access_token, "access");
        assert_eq!(token.refresh_token.as_deref(), Some("refresh"));
        let seen = exchange.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "the-code");
        assert!(!seen[0].1.is_empty());
        assert!(seen[0].2.starts_with("http://127.0.0.1:"));
    }

    #[test]
    fn a_favicon_request_is_ignored_and_the_callback_still_arrives() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let addr = flow.listener.local_addr().unwrap();
        let state = flow.csrf.clone();
        let worker = thread::spawn(move || {
            call(addr, "/favicon.ico");
            call(addr, &format!("/?code=the-code&state={state}"));
        });

        let token = finish(&flow, &FakeExchange::new(), Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
        assert_eq!(token.access_token, "access");
    }

    #[test]
    fn a_wrong_state_is_refused_before_the_code_is_exchanged() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let addr = flow.listener.local_addr().unwrap();
        let worker = thread::spawn(move || call(addr, "/?code=the-code&state=wrong"));

        let exchange = FakeExchange::new();
        let error = finish(&flow, &exchange, Duration::from_secs(2)).unwrap_err();
        worker.join().unwrap();
        assert!(error.contains("did not match"));
        assert!(exchange.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn access_denied_is_a_cancellation() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let addr = flow.listener.local_addr().unwrap();
        let state = flow.csrf.clone();
        let worker = thread::spawn(move || {
            call(addr, &format!("/?error=access_denied&state={state}"));
        });

        let error = finish(&flow, &FakeExchange::new(), Duration::from_secs(2)).unwrap_err();
        worker.join().unwrap();
        assert_eq!(error, "Sign-in was cancelled.");
    }

    #[test]
    fn no_callback_times_out_cleanly() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let error = finish(&flow, &FakeExchange::new(), Duration::from_millis(80)).unwrap_err();
        assert!(error.contains("timed out"));
    }

    #[test]
    fn cancelling_stops_the_wait_immediately() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        flow.cancel();
        let error = finish(&flow, &FakeExchange::new(), Duration::from_secs(5)).unwrap_err();
        assert_eq!(error, "Sign-in was cancelled.");
    }

    #[test]
    fn only_one_finish_can_claim_a_flow() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        flow.claim().unwrap();
        assert_eq!(
            flow.claim().unwrap_err(),
            "This sign-in attempt is already in progress."
        );
    }

    #[test]
    fn a_callback_without_a_code_is_refused() {
        let flow = start(&[SCOPE_READONLY]).unwrap();
        let addr = flow.listener.local_addr().unwrap();
        let state = flow.csrf.clone();
        let worker = thread::spawn(move || {
            call(addr, &format!("/?state={state}"));
        });
        let error = finish(&flow, &FakeExchange::new(), Duration::from_secs(2)).unwrap_err();
        worker.join().unwrap();
        assert!(error.contains("did not return a code"));
    }
}
