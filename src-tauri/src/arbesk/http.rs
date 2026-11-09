//! The HTTP requests Arbesk needs, behind [`Http`] so tests never open a
//! socket: GETs (config, token index, IPFS gateway), JSON POSTs (JSON-RPC,
//! the sign-in check, Arbesk's slow relay), multipart POSTs (the pre-signed
//! upload URLs) and one DELETE (ending the sign-in's session).

use std::time::Duration;

/// A failed request: the HTTP status when the server answered, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpError {
    pub status: Option<u16>,
    pub message: String,
}

impl HttpError {
    pub fn new(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
}

pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError>;
    /// POST `body` as JSON; `session` goes in `Authorization: Session <token>`.
    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError>;
    // Used by the writes that follow.
    #[allow(dead_code)]
    /// POST `body` as JSON like [`Http::post_json`], waiting up to four
    /// minutes: Arbesk's wallet relay answers when its transaction is mined.
    fn post_json_slow(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError>;
    #[allow(dead_code)]
    /// POST a `multipart/form-data` body: `fields`, then `bytes` as the part
    /// `file` named `file_name`.
    fn post_multipart(
        &self,
        url: &str,
        file_name: &str,
        bytes: &[u8],
        fields: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError>;
    fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError>;
}

/// A `multipart/form-data` body, built by hand: `fields` in order, then the
/// `file` part (`application/octet-stream`), closed with `--boundary--`.
#[allow(dead_code)]
pub fn multipart_body(
    boundary: &str,
    file_name: &str,
    bytes: &[u8],
    fields: &[(&str, &str)],
) -> Vec<u8> {
    let mut body = Vec::with_capacity(bytes.len() + 256);
    for (name, value) in fields {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

/// How long the relay and uploads may take: four minutes.
const SLOW_TIMEOUT_SECS: u64 = 240;

/// The most one answer may hold: an asset's largest part, packed.
pub const MAX_BODY: u64 = 512 * 1024 * 1024;

/// The real client: 30 seconds per request (four minutes for the relay and
/// for uploads), rustls, no cookies, and no answer read past `cap` bytes,
/// whatever the server says.
pub struct ReqwestHttp {
    client: reqwest::blocking::Client,
    #[allow(dead_code)]
    slow: reqwest::blocking::Client,
    cap: u64,
}

impl ReqwestHttp {
    pub fn new() -> Result<Self, String> {
        Self::with_cap(MAX_BODY)
    }

    pub fn with_cap(cap: u64) -> Result<Self, String> {
        let client_with = |timeout: u64| {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(timeout))
                .user_agent(concat!("FileBrowser/", env!("CARGO_PKG_VERSION")))
                .build()
                .map_err(|error| error.to_string())
        };
        Ok(Self {
            client: client_with(30)?,
            slow: client_with(SLOW_TIMEOUT_SECS)?,
            cap,
        })
    }

    fn send(
        &self,
        request: reqwest::blocking::RequestBuilder,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        let request = match session {
            Some(token) => request.header("Authorization", format!("Session {token}")),
            None => request,
        };
        let response = request
            .send()
            .map_err(|error| HttpError::new(None, error.to_string()))?;
        let status = response.status();
        let bytes = read_capped(response, self.cap)
            .map_err(|message| HttpError::new(Some(status.as_u16()), message))?;
        if status.is_success() {
            Ok(bytes)
        } else {
            Err(HttpError::new(
                Some(status.as_u16()),
                format!("The server answered {status}."),
            ))
        }
    }
}

/// A body of at most `cap` bytes: a longer one (by its length or its bytes) is refused.
fn read_capped(response: reqwest::blocking::Response, cap: u64) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let too_long = || "Arbesk sent more than File Browser reads at once.".to_string();
    if response.content_length().is_some_and(|length| length > cap) {
        return Err(too_long());
    }
    let mut bytes = Vec::new();
    response
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > cap {
        return Err(too_long());
    }
    Ok(bytes)
}

impl Http for ReqwestHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
        self.send(self.client.get(url), None)
    }

    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        self.send(self.client.post(url).json(body), session)
    }

    fn post_json_slow(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        self.send(self.slow.post(url).json(body), session)
    }

    fn post_multipart(
        &self,
        url: &str,
        file_name: &str,
        bytes: &[u8],
        fields: &[(&str, &str)],
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let boundary = format!("----FileBrowser{nanos:x}");
        let request = self
            .slow
            .post(url)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(multipart_body(&boundary, file_name, bytes, fields));
        self.send(request, session)
    }

    fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError> {
        self.send(self.client.delete(url), session).map(|_| ())
    }
}

/// A scripted [`Http`] for tests: `answer` decides each request's response,
/// and every request is logged.
#[cfg(test)]
pub mod fake {
    use super::*;
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Request {
        pub method: &'static str,
        pub url: String,
        pub body: Option<serde_json::Value>,
        pub session: Option<String>,
        /// The uploaded file of a multipart POST.
        pub data: Option<Vec<u8>>,
        /// The other form fields of a multipart POST.
        pub fields: Vec<(String, String)>,
    }

    type Answer = Box<dyn Fn(&Request) -> Result<Vec<u8>, HttpError> + Send + Sync>;

    pub struct FakeHttp {
        answer: Answer,
        pub log: Mutex<Vec<Request>>,
    }

    impl FakeHttp {
        pub fn new(
            answer: impl Fn(&Request) -> Result<Vec<u8>, HttpError> + Send + Sync + 'static,
        ) -> Self {
            Self {
                answer: Box::new(answer),
                log: Mutex::new(Vec::new()),
            }
        }

        pub fn requests(&self) -> Vec<Request> {
            self.log.lock().unwrap().clone()
        }

        fn call(&self, request: Request) -> Result<Vec<u8>, HttpError> {
            let answer = (self.answer)(&request);
            self.log.lock().unwrap().push(request);
            answer
        }
    }

    impl Http for FakeHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, HttpError> {
            self.call(Request {
                method: "GET",
                url: url.into(),
                body: None,
                session: None,
                data: None,
                fields: Vec::new(),
            })
        }

        fn post_json(
            &self,
            url: &str,
            body: &serde_json::Value,
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            let session = session.map(String::from);
            self.call(Request {
                method: "POST",
                url: url.into(),
                body: Some(body.clone()),
                session,
                data: None,
                fields: Vec::new(),
            })
        }

        fn post_json_slow(
            &self,
            url: &str,
            body: &serde_json::Value,
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            self.post_json(url, body, session)
        }

        fn post_multipart(
            &self,
            url: &str,
            _file_name: &str,
            bytes: &[u8],
            fields: &[(&str, &str)],
            session: Option<&str>,
        ) -> Result<Vec<u8>, HttpError> {
            self.call(Request {
                method: "POST",
                url: url.into(),
                body: None,
                session: session.map(String::from),
                data: Some(bytes.to_vec()),
                fields: fields
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            })
        }

        fn delete(&self, url: &str, session: Option<&str>) -> Result<(), HttpError> {
            let session = session.map(String::from);
            self.call(Request {
                method: "DELETE",
                url: url.into(),
                body: None,
                session,
                data: None,
                fields: Vec::new(),
            })
            .map(|_| ())
        }
    }

    pub fn not_found() -> HttpError {
        HttpError::new(Some(404), "The server answered 404 Not Found.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    /// Serve one request with `status` and `body`; returns the base URL and
    /// a handle that yields the request's head.
    fn serve_once(
        status: &'static str,
        body: &'static str,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut buffer = [0u8; 4096];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                seen.extend_from_slice(&buffer[..read]);
            }
            let head = String::from_utf8_lossy(&seen).into_owned();
            let length: usize = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|n| n.trim().parse().unwrap())
                })
                .unwrap_or(0);
            let body_start = seen.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
            let mut have = seen.len() - body_start;
            while have < length {
                have += stream.read(&mut buffer).unwrap();
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            head
        });
        (url, handle)
    }

    #[test]
    fn a_get_returns_the_body() {
        let (url, server) = serve_once("200 OK", "hello");
        assert_eq!(ReqwestHttp::new().unwrap().get(&url).unwrap(), b"hello");
        let head = server.join().unwrap();
        assert!(head.starts_with("GET / "), "{head}");
        assert!(
            !head.to_ascii_lowercase().contains("authorization"),
            "{head}"
        );
    }

    #[test]
    fn a_post_sends_json_and_the_session() {
        let (url, server) = serve_once("200 OK", "{}");
        let body = serde_json::json!({ "email": "a@b.c" });
        ReqwestHttp::new()
            .unwrap()
            .post_json(&url, &body, Some("tok"))
            .unwrap();
        let head = server.join().unwrap();
        assert!(head.starts_with("POST / "), "{head}");
        assert!(
            head.contains("authorization: Session tok")
                || head.contains("Authorization: Session tok"),
            "{head}"
        );
        assert!(
            head.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{head}"
        );
    }

    #[test]
    fn a_delete_reports_success() {
        let (url, server) = serve_once("200 OK", "{\"invalidated\":true}");
        ReqwestHttp::new()
            .unwrap()
            .delete(&url, Some("tok"))
            .unwrap();
        assert!(server.join().unwrap().starts_with("DELETE / "));
    }

    #[test]
    fn an_error_status_carries_its_code() {
        let (url, server) = serve_once("503 Service Unavailable", "busy");
        let error = ReqwestHttp::new().unwrap().get(&url).unwrap_err();
        server.join().unwrap();
        assert_eq!(error.status, Some(503));
    }

    #[test]
    fn a_body_over_the_cap_is_refused() {
        let (url, server) = serve_once("200 OK", "0123456789");
        let error = ReqwestHttp::with_cap(9).unwrap().get(&url).unwrap_err();
        server.join().unwrap();
        assert_eq!(
            error.message,
            "Arbesk sent more than File Browser reads at once."
        );
        let (url, server) = serve_once("200 OK", "0123456789");
        assert_eq!(
            ReqwestHttp::with_cap(10).unwrap().get(&url).unwrap(),
            b"0123456789"
        );
        server.join().unwrap();
    }

    #[test]
    fn multipart_body_has_fields_then_the_file() {
        let body = multipart_body("XB", "a.bin", b"\x00\x01data", &[("network", "public")]);
        let expected: Vec<u8> = [
            &b"--XB\r\nContent-Disposition: form-data; name=\"network\"\r\n\r\npublic\r\n"[..],
            b"--XB\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.bin\"\r\n",
            b"Content-Type: application/octet-stream\r\n\r\n",
            b"\x00\x01data\r\n--XB--\r\n",
        ]
        .concat();
        assert_eq!(body, expected);
    }

    #[test]
    fn a_multipart_post_sends_the_form_and_the_session() {
        let (url, server) = serve_once("200 OK", "{}");
        ReqwestHttp::new()
            .unwrap()
            .post_multipart(&url, "a.bin", b"abc", &[("network", "public")], Some("tok"))
            .unwrap();
        let head = server.join().unwrap().to_ascii_lowercase();
        assert!(head.starts_with("post / "), "{head}");
        assert!(
            head.contains("content-type: multipart/form-data; boundary="),
            "{head}"
        );
        assert!(head.contains("authorization: session tok"), "{head}");
    }

    #[test]
    fn a_slow_json_post_sends_json() {
        let (url, server) = serve_once("200 OK", "{}");
        ReqwestHttp::new()
            .unwrap()
            .post_json_slow(&url, &serde_json::json!({ "op": "x" }), Some("tok"))
            .unwrap();
        let head = server.join().unwrap().to_ascii_lowercase();
        assert!(head.contains("content-type: application/json"), "{head}");
        assert!(head.contains("authorization: session tok"), "{head}");
    }

    #[test]
    fn an_unreachable_server_has_no_status() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = ReqwestHttp::new()
            .unwrap()
            .get(&format!("http://127.0.0.1:{port}"))
            .unwrap_err();
        assert_eq!(error.status, None);
    }
}
