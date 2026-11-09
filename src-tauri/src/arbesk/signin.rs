//! Signing in to Arbesk as `besk login` does: Arbesk's own page
//! (`/api/v1/cli-auth`) has the email's owner type the code Arbesk emails,
//! then sends the browser to `http://127.0.0.1:<port>/?token=&address=&email=`.
//! That page sends no `state`, so the callback proves nothing by itself.
//! Arbesk is then asked, with the session the callback carries, which
//! address the email typed here has (`resolve-email`): the session must be
//! one Arbesk accepts, and the address must be the callback's. So only the
//! typed email's own address is ever kept. What this doesn't prove is that
//! the session was made by this sign-in (Arbesk resolves any email for any
//! session, and has no "who am I"). The session of a confirmed sign-in is
//! kept (`session.rs`) to copy files into Arbesk; one that isn't confirmed
//! is ended.

use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use percent_encoding::utf8_percent_encode;
use serde_json::{Value, json};
use url::Url;

use crate::arbesk::chain::Site;
use crate::arbesk::http::Http;
use crate::arbesk::session::Session;
use crate::cloud::location::NAME_SET;
use crate::cloud::loopback::{self, Waited};

pub const NOT_CONFIRMED: &str = "Sign-in failed: Arbesk did not confirm this account.";
const CANNOT_CHECK: &str = "Arbesk could not confirm this account. Try again later.";
/// How long a session lasts when the callback doesn't say.
const SESSION_FALLBACK_MS: i64 = 24 * 60 * 60 * 1000;
/// As long as `besk login` waits for the browser.
pub const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

/// One sign-in waiting for the browser, kept between `arbesk_connect_start`
/// and `arbesk_connect_finish`.
pub struct Flow {
    pub id: String,
    /// The email typed, trimmed and lower case.
    pub email: String,
    pub listener: TcpListener,
    pub sign_in_url: String,
    pub created: Instant,
    cancelled: AtomicBool,
    claimed: AtomicBool,
}

impl Flow {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Only one `finish` may wait on the browser.
    pub fn claim(&self) -> Result<(), String> {
        if self.claimed.swap(true, Ordering::SeqCst) {
            Err("This sign-in attempt is already in progress.".into())
        } else {
            Ok(())
        }
    }
}

/// Listen on a free local port and name Arbesk's sign-in page for `email`.
pub fn start(site: &Site, email: &str) -> Result<Flow, String> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') || email.chars().any(char::is_whitespace) {
        return Err("Enter the email address you use with Arbesk.".into());
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .map_err(|error| format!("Could not wait for the browser: {error}"))?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let sign_in_url = format!(
        "{}/api/v1/cli-auth?email={}&port={port}",
        site.origin,
        utf8_percent_encode(&email, NAME_SET)
    );
    Ok(Flow {
        id: crate::cloud::accounts::new_account_id(),
        email,
        listener,
        sign_in_url,
        created: Instant::now(),
        cancelled: AtomicBool::new(false),
        claimed: AtomicBool::new(false),
    })
}

/// What Arbesk's page sends back.
struct Callback {
    token: String,
    expires_at_ms: i64,
    address: String,
    email: String,
}

fn parse_callback(url: &Url) -> Option<Callback> {
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let token = param("token").filter(|token| !token.is_empty())?;
    let expires_at_ms = param("expiresAt")
        .and_then(|at| at.parse().ok())
        .unwrap_or_else(|| now_ms() + SESSION_FALLBACK_MS);
    Some(Callback {
        token,
        expires_at_ms,
        address: param("address").unwrap_or_default(),
        email: param("email").unwrap_or_default(),
    })
}

/// A confirmed sign-in: the email and lower-case address of the account, and
/// the session it made, kept to act as the account.
#[derive(Debug)]
pub struct SignedIn {
    pub email: String,
    pub address: String,
    pub session: Session,
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as i64)
}

/// Wait for the browser, then have Arbesk confirm the callback. A session
/// that isn't confirmed is ended; a confirmed one is handed back to keep.
pub fn finish(
    flow: &Flow,
    http: &dyn Http,
    site: &Site,
    timeout: Duration,
) -> Result<SignedIn, String> {
    let callback = loopback::wait_for_callback(
        &flow.listener,
        timeout,
        || flow.is_cancelled(),
        parse_callback,
        "You can close this window and return to File Browser.",
    )
    .map_err(|waited| match waited {
        Waited::Cancelled => "Sign-in was cancelled.".to_string(),
        Waited::TimedOut => "Sign-in timed out. Try again.".to_string(),
        Waited::Io(error) => format!("Arbesk sign-in failed: {error}"),
    })?;
    match confirm(http, site, &flow.email, &callback) {
        Ok(address) => Ok(SignedIn {
            email: flow.email.clone(),
            address,
            session: Session {
                token: callback.token,
                expires_at_ms: callback.expires_at_ms,
            },
        }),
        Err(error) => {
            // Nothing is kept for a session Arbesk didn't confirm: end it.
            let _ = http.delete(
                &format!("{}/api/v1/sessions", site.origin),
                Some(&callback.token),
            );
            Err(error)
        }
    }
}

/// The callback's address, once Arbesk accepts its session and resolves
/// `email` to that same address.
fn confirm(
    http: &dyn Http,
    site: &Site,
    email: &str,
    callback: &Callback,
) -> Result<String, String> {
    if !callback.email.eq_ignore_ascii_case(email) {
        return Err(NOT_CONFIRMED.into());
    }
    let url = format!("{}/api/v1/users/resolve-email", site.origin);
    let answer = http
        .post_json(&url, &json!({ "email": email }), Some(&callback.token))
        .map_err(|error| match error.status {
            Some(status) if status < 500 && status != 429 => NOT_CONFIRMED.to_string(),
            _ => CANNOT_CHECK.to_string(),
        })?;
    let body: Value = serde_json::from_slice(&answer).map_err(|_| CANNOT_CHECK.to_string())?;
    let resolved = body["address"].as_str().filter(|_| body["exists"] == true);
    match resolved {
        Some(address)
            if !callback.address.is_empty() && address.eq_ignore_ascii_case(&callback.address) =>
        {
            Ok(address.to_ascii_lowercase())
        }
        _ => Err(NOT_CONFIRMED.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arbesk::http::HttpError;
    use crate::arbesk::http::fake::FakeHttp;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::thread;

    const ORIGIN: &str = "https://arbesk.test";
    const ADDRESS: &str = "0x00000000000000000000000000000000000000ab";

    fn site() -> Site {
        Site {
            origin: ORIGIN.into(),
            chain_id: 84532,
        }
    }

    fn port_of(flow: &Flow) -> u16 {
        flow.listener.local_addr().unwrap().port()
    }

    fn call(port: u16, target: &str) {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
    }

    fn callback(token: &str, address: &str, email: &str) -> String {
        format!(
            "/?token={token}&expiresAt=1&address={address}&email={}",
            email.replace('@', "%40")
        )
    }

    /// Arbesk's backend: resolve-email answers `resolved` (or `status`).
    fn backend(resolved: Result<serde_json::Value, u16>) -> FakeHttp {
        FakeHttp::new(move |request| match (request.method, &resolved) {
            ("POST", Ok(body)) => Ok(body.to_string().into_bytes()),
            ("POST", Err(status)) => Err(HttpError::new(Some(*status), "refused")),
            ("DELETE", _) => Ok(b"{\"invalidated\":true}".to_vec()),
            _ => panic!("unexpected {request:?}"),
        })
    }

    fn sign_in(http: &FakeHttp, targets: Vec<String>) -> Result<SignedIn, String> {
        let flow = start(&site(), "Me@Example.com ").unwrap();
        let port = port_of(&flow);
        let sender = thread::spawn(move || targets.iter().for_each(|target| call(port, target)));
        let result = finish(&flow, http, &site(), Duration::from_secs(5));
        sender.join().unwrap();
        result
    }

    #[test]
    fn the_sign_in_page_is_arbesks_with_the_email_and_port() {
        let flow = start(&site(), " Me@Example.com").unwrap();
        assert_eq!(flow.email, "me@example.com");
        assert_eq!(
            flow.sign_in_url,
            format!(
                "{ORIGIN}/api/v1/cli-auth?email=me%40example.com&port={}",
                port_of(&flow)
            )
        );
    }

    #[test]
    fn an_address_that_is_not_an_email_is_refused() {
        for email in ["", "not-an-email", "a b@c.d"] {
            assert!(start(&site(), email).is_err(), "{email}");
        }
    }

    #[test]
    fn a_confirmed_sign_in_keeps_its_session() {
        let http = backend(Ok(
            json!({ "exists": true, "address": ADDRESS.to_uppercase().replace("0X", "0x") }),
        ));
        let target = "/?token=tok&expiresAt=1762000000000&address=".to_string()
            + ADDRESS
            + "&email=me%40example.com";
        let got = sign_in(&http, vec![target]).unwrap();
        assert_eq!(
            (got.email.as_str(), got.address.as_str()),
            ("me@example.com", ADDRESS)
        );
        assert_eq!(
            got.session,
            Session {
                token: "tok".into(),
                expires_at_ms: 1_762_000_000_000
            }
        );
        let requests = http.requests();
        assert_eq!(requests.len(), 1, "{requests:?}");
        assert_eq!(
            requests[0].url,
            format!("{ORIGIN}/api/v1/users/resolve-email")
        );
        assert_eq!(requests[0].body, Some(json!({ "email": "me@example.com" })));
        assert_eq!(requests[0].session.as_deref(), Some("tok"));
    }

    #[test]
    fn a_missing_or_unreadable_expiry_is_a_day_from_now() {
        for target in [
            format!("/?token=tok&address={ADDRESS}&email=me%40example.com"),
            format!("/?token=tok&expiresAt=soon&address={ADDRESS}&email=me%40example.com"),
        ] {
            let http = backend(Ok(json!({ "exists": true, "address": ADDRESS })));
            let before = now_ms();
            let got = sign_in(&http, vec![target]).unwrap();
            let day = 24 * 60 * 60 * 1000;
            assert!(got.session.expires_at_ms >= before + day);
            assert!(got.session.expires_at_ms <= now_ms() + day);
        }
    }

    #[test]
    fn an_unconfirmed_sign_in_ends_the_session_it_was_handed() {
        let http = backend(Ok(json!({ "exists": false })));
        let got = sign_in(&http, vec![callback("tok", ADDRESS, "me@example.com")]);
        assert_eq!(got.unwrap_err(), NOT_CONFIRMED);
        let requests = http.requests();
        let last = requests.last().unwrap();
        assert_eq!(
            (last.method, last.url.as_str()),
            ("DELETE", format!("{ORIGIN}/api/v1/sessions").as_str())
        );
        assert_eq!(last.session.as_deref(), Some("tok"));
    }

    #[test]
    fn a_request_without_a_token_is_ignored() {
        let http = backend(Ok(json!({ "exists": true, "address": ADDRESS })));
        let got = sign_in(
            &http,
            vec![
                "/favicon.ico".into(),
                callback("tok", ADDRESS, "me@example.com"),
            ],
        );
        assert!(got.is_ok(), "{got:?}");
    }

    #[test]
    fn a_callback_for_another_email_is_refused() {
        let http = backend(Ok(json!({ "exists": true, "address": ADDRESS })));
        let got = sign_in(&http, vec![callback("tok", ADDRESS, "someone@else.com")]);
        assert_eq!(got.unwrap_err(), NOT_CONFIRMED);
    }

    #[test]
    fn a_callback_whose_address_arbesk_does_not_confirm_is_refused() {
        let other = "0x00000000000000000000000000000000000000cd";
        for answer in [
            json!({ "exists": true, "address": other }),
            json!({ "exists": false }),
            json!({ "exists": true, "address": null }),
        ] {
            let http = backend(Ok(answer.clone()));
            let got = sign_in(&http, vec![callback("tok", ADDRESS, "me@example.com")]);
            assert_eq!(got.unwrap_err(), NOT_CONFIRMED, "{answer}");
        }
    }

    #[test]
    fn a_session_arbesk_refuses_is_refused() {
        let http = backend(Err(401));
        let got = sign_in(&http, vec![callback("forged", ADDRESS, "me@example.com")]);
        assert_eq!(got.unwrap_err(), NOT_CONFIRMED);
    }

    #[test]
    fn arbesk_unable_to_check_says_try_again() {
        for status in [429, 502, 503] {
            let http = backend(Err(status));
            let got = sign_in(&http, vec![callback("tok", ADDRESS, "me@example.com")]);
            assert_eq!(
                got.unwrap_err(),
                "Arbesk could not confirm this account. Try again later."
            );
        }
    }

    #[test]
    fn a_cancelled_sign_in_stops_waiting() {
        let flow = start(&site(), "me@example.com").unwrap();
        flow.cancel();
        let http = backend(Ok(json!({})));
        let got = finish(&flow, &http, &site(), Duration::from_secs(30));
        assert_eq!(got.unwrap_err(), "Sign-in was cancelled.");
    }

    #[test]
    fn a_sign_in_nobody_finishes_times_out() {
        let flow = start(&site(), "me@example.com").unwrap();
        let http = backend(Ok(json!({})));
        let got = finish(&flow, &http, &site(), Duration::from_millis(50));
        assert_eq!(got.unwrap_err(), "Sign-in timed out. Try again.");
    }

    #[test]
    fn only_one_finish_can_claim_a_flow() {
        let flow = start(&site(), "me@example.com").unwrap();
        assert!(flow.claim().is_ok());
        assert!(flow.claim().is_err());
    }
}
