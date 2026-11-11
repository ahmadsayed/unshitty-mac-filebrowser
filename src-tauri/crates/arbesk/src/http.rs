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
    /// GET a download that may be large (an asset's other file), waiting
    /// longer the larger `expected` bytes are ([`transfer_timeout`]) and
    /// handing the body to `chunk` piece by piece as it arrives. While
    /// nothing arrives, `chunk` is handed an empty piece every [`TICK`], so
    /// a cancel is heard on a stalled connection too. `chunk` stops the
    /// read by answering `false` (a cancel, a body past its cap): the call
    /// answers `Ok` at once, and the rest is never handed over.
    fn get_streamed(
        &self,
        url: &str,
        expected: u64,
        chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), HttpError>;
    /// POST `body` as JSON; `session` goes in `Authorization: Session <token>`.
    fn post_json(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError>;
    /// POST `body` as JSON like [`Http::post_json`], waiting up to four
    /// minutes: Arbesk's wallet relay answers when its transaction is mined.
    fn post_json_slow(
        &self,
        url: &str,
        body: &serde_json::Value,
        session: Option<&str>,
    ) -> Result<Vec<u8>, HttpError>;
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
    /// The limits this client's reads go through (performance plan,
    /// phase 2): the gateway takes a read permit and shares a read already
    /// under way, the chain an RPC permit. None for the bare client, which
    /// a write and the tests use: unlimited, as before.
    fn limits(&self) -> Option<crate::limiter::Limits<'_>> {
        None
    }
}

/// A `multipart/form-data` body, built by hand: `fields` in order, then the
/// `file` part (`application/octet-stream`), closed with `--boundary--`.
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
    let file_name = quoted_file_name(file_name);
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

/// The slowest rate an upload or a large download is given time for: 64 KiB/s.
const TRANSFER_BYTES_PER_SEC: u64 = 64 * 1024;

/// How long an upload or a large download of `len` bytes may take: four
/// minutes, plus the time it takes at `TRANSFER_BYTES_PER_SEC` (100 MB:
/// about half an hour). A fixed limit would fail a large file on a slow
/// link, and no limit would wait on a stalled one for ever.
pub(crate) fn transfer_timeout(len: u64) -> Duration {
    Duration::from_secs(SLOW_TIMEOUT_SECS + len / TRANSFER_BYTES_PER_SEC)
}

/// How much of a streamed download is read at a time, between checks.
const STREAM_CHUNK: usize = 64 * 1024;

/// How often a streamed GET that is waiting for data asks its caller
/// whether to go on (an empty piece): a cancel is heard within this long.
pub(crate) const TICK: Duration = Duration::from_millis(200);

/// What a streamed GET's reading thread hands over.
enum Piece {
    Bytes(Vec<u8>),
    End,
    Failed(HttpError),
}

/// Runs `next` on a thread of its own until it answers `None` (the end) or
/// fails, handing each piece it reads to `chunk` on this thread; while none
/// comes, `chunk` is handed an empty piece every [`TICK`]. When `chunk`
/// answers `false` this returns `Ok` at once: the reading thread, which may
/// be blocked on a stalled connection, is left to end by itself when its
/// next piece finds nobody to take it, or at its request's timeout. It
/// reads a piece only once `chunk` has taken the one before, so nothing is
/// read past a stop.
fn pump(
    mut next: impl FnMut() -> Result<Option<Vec<u8>>, HttpError> + Send + 'static,
    chunk: &mut dyn FnMut(&[u8]) -> bool,
) -> Result<(), HttpError> {
    use std::sync::mpsc;
    let (sender, receiver) = mpsc::sync_channel(1);
    let (taken, wait_taken) = mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("arbesk-download".into())
        .spawn(move || {
            loop {
                let (piece, last) = match next() {
                    Ok(Some(bytes)) => (Piece::Bytes(bytes), false),
                    Ok(None) => (Piece::End, true),
                    Err(error) => (Piece::Failed(error), true),
                };
                if sender.send(piece).is_err() || last || wait_taken.recv().is_err() {
                    return;
                }
            }
        })
        .map_err(|error| HttpError::new(None, error.to_string()))?;
    loop {
        let go_on = match receiver.recv_timeout(TICK) {
            Ok(Piece::Bytes(bytes)) => {
                let go_on = chunk(&bytes);
                if go_on {
                    // A dead reading thread shows as `Disconnected` next turn.
                    let _ = taken.send(());
                }
                go_on
            }
            Ok(Piece::End) => return Ok(()),
            Ok(Piece::Failed(error)) => return Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => chunk(&[]),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(HttpError::new(None, "The download stopped unexpectedly."));
            }
        };
        if !go_on {
            return Ok(());
        }
    }
}

/// The response to a streamed GET, refused unless it succeeded and says
/// it holds at most `cap` bytes.
fn open_streamed(
    request: reqwest::blocking::RequestBuilder,
    cap: u64,
) -> Result<reqwest::blocking::Response, HttpError> {
    let response = request
        .send()
        .map_err(|error| HttpError::new(None, error.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        let message = format!("The server answered {status}.");
        return Err(HttpError::new(Some(status.as_u16()), message));
    }
    if response.content_length().is_some_and(|length| length > cap) {
        return Err(too_long(status.as_u16()));
    }
    Ok(response)
}

fn too_long(status: u16) -> HttpError {
    HttpError::new(
        Some(status),
        "Arbesk sent more than File Browser reads at once.",
    )
}

/// `name` safe inside `filename="…"`: `"`, CR and LF percent-encoded, as
/// browsers send them.
fn quoted_file_name(name: &str) -> String {
    name.replace('"', "%22")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// The most one answer may hold: an asset's largest part, packed.
pub const MAX_BODY: u64 = 512 * 1024 * 1024;

/// The real client: 30 seconds per request (four minutes for the relay,
/// longer for a large upload or download: `transfer_timeout`), rustls, no cookies, and no answer read past `cap` bytes,
/// whatever the server says.
pub struct ReqwestHttp {
    client: reqwest::blocking::Client,
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
            // Without the URL: an upload's is a one-use pre-signed address.
            .map_err(|error| HttpError::new(None, error.without_url().to_string()))?;
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

    /// On the slow client, given [`transfer_timeout`] of `expected`, read
    /// [`STREAM_CHUNK`] at a time on a thread of its own ([`pump`]: the
    /// request is sent there too, so a cancel is heard while the server is
    /// silent); never more than the client's cap.
    fn get_streamed(
        &self,
        url: &str,
        expected: u64,
        chunk: &mut dyn FnMut(&[u8]) -> bool,
    ) -> Result<(), HttpError> {
        use std::io::Read;
        let cap = self.cap;
        let mut request = Some(self.slow.get(url).timeout(transfer_timeout(expected)));
        let mut response = None;
        let mut read = 0u64;
        pump(
            move || {
                if let Some(request) = request.take() {
                    response = Some(open_streamed(request, cap)?);
                }
                let Some(response) = response.as_mut() else {
                    return Ok(None);
                };
                let status = response.status().as_u16();
                let mut buffer = vec![0; STREAM_CHUNK];
                let n = response
                    .read(&mut buffer)
                    .map_err(|error| HttpError::new(Some(status), error.to_string()))?;
                read += n as u64;
                if read > cap {
                    return Err(too_long(status));
                }
                buffer.truncate(n);
                Ok((n > 0).then_some(buffer))
            },
            chunk,
        )
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
            .timeout(transfer_timeout(bytes.len() as u64))
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
    type BeforeChunk = std::sync::Arc<dyn Fn(usize) + Send + Sync>;

    pub struct FakeHttp {
        answer: Answer,
        pub log: Mutex<Vec<Request>>,
        /// How many bytes a streamed GET hands over at a time.
        chunk: usize,
        /// Runs before a streamed GET reads chunk `n` (from 0), on the
        /// reading thread: it may block, as a stalled connection does.
        before_chunk: Option<BeforeChunk>,
        /// How many chunks streamed GETs have handed over.
        pub chunks: std::sync::atomic::AtomicUsize,
    }

    impl FakeHttp {
        pub fn new(
            answer: impl Fn(&Request) -> Result<Vec<u8>, HttpError> + Send + Sync + 'static,
        ) -> Self {
            Self {
                answer: Box::new(answer),
                log: Mutex::new(Vec::new()),
                chunk: usize::MAX,
                before_chunk: None,
                chunks: Default::default(),
            }
        }

        /// The same fake, streaming bodies `size` bytes at a time and
        /// running `before` ahead of each chunk.
        pub fn in_chunks(
            self,
            size: usize,
            before: impl Fn(usize) + Send + Sync + 'static,
        ) -> Self {
            Self {
                chunk: size,
                before_chunk: Some(std::sync::Arc::new(before)),
                ..self
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

        /// The answer to a GET, handed over in chunks through [`pump`],
        /// as the real client hands them; `chunks` counts the ones handed.
        fn get_streamed(
            &self,
            url: &str,
            _expected: u64,
            chunk: &mut dyn FnMut(&[u8]) -> bool,
        ) -> Result<(), HttpError> {
            let body = self.get(url)?;
            let size = self.chunk.max(1);
            let mut pieces = body
                .chunks(size)
                .map(<[u8]>::to_vec)
                .collect::<Vec<_>>()
                .into_iter()
                .enumerate();
            let before = self.before_chunk.clone();
            let next = move || {
                Ok(pieces.next().map(|(n, piece)| {
                    if let Some(before) = &before {
                        before(n);
                    }
                    piece
                }))
            };
            pump(next, &mut |piece| {
                if !piece.is_empty() {
                    self.chunks
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                chunk(piece)
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
    fn a_streamed_get_hands_over_the_body_until_told_to_stop() {
        let (url, server) = serve_once("200 OK", "hello");
        let mut got = Vec::new();
        ReqwestHttp::new()
            .unwrap()
            .get_streamed(&url, 5, &mut |piece| {
                got.extend_from_slice(piece);
                true
            })
            .unwrap();
        assert_eq!(got, b"hello");
        assert!(server.join().unwrap().starts_with("GET / "));
        let (url, server) = serve_once("200 OK", "hello");
        let mut calls = 0;
        let stopped = ReqwestHttp::new().unwrap().get_streamed(&url, 5, &mut |_| {
            calls += 1;
            false
        });
        assert_eq!((stopped, calls), (Ok(()), 1));
        server.join().unwrap();
        let (url, server) = serve_once("404 Not Found", "");
        let error = ReqwestHttp::new()
            .unwrap()
            .get_streamed(&url, 0, &mut |_| true);
        assert_eq!(error.unwrap_err().status, Some(404));
        server.join().unwrap();
        let (url, server) = serve_once("200 OK", "too long");
        let capped = ReqwestHttp::with_cap(4).unwrap();
        assert!(capped.get_streamed(&url, 0, &mut |_| true).is_err());
        server.join().unwrap();
    }

    /// A server that says it sends a body and then goes silent: the call
    /// asks `chunk` every tick meanwhile, and returns as soon as it stops.
    #[test]
    fn a_streamed_get_hears_a_stop_while_the_server_is_silent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (done, finished) = std::sync::mpsc::channel::<()>();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer).unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhe")
                .unwrap();
            // Silent until the test is over.
            let _ = finished.recv_timeout(Duration::from_secs(10));
        });
        let started = std::time::Instant::now();
        let (mut got, mut ticks) = (Vec::new(), 0);
        let stopped = ReqwestHttp::new()
            .unwrap()
            .get_streamed(&url, 10, &mut |piece| {
                got.extend_from_slice(piece);
                ticks += usize::from(piece.is_empty());
                ticks < 2
            });
        assert_eq!((stopped, got.as_slice(), ticks), (Ok(()), &b"he"[..], 2));
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        done.send(()).unwrap();
        server.join().unwrap();
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
    fn a_file_name_cannot_break_out_of_its_quotes() {
        let body = multipart_body("XB", "a\"b\r\nc.bin", b"x", &[]);
        let text = String::from_utf8(body).unwrap();
        assert!(text.contains("filename=\"a%22b%0D%0Ac.bin\"\r\n"), "{text}");
    }

    #[test]
    fn a_transfer_may_take_longer_the_larger_it_is() {
        assert_eq!(transfer_timeout(0), Duration::from_secs(240));
        assert_eq!(transfer_timeout(64 * 1024), Duration::from_secs(241));
        // 100 MB at 64 KiB/s, on top of the four minutes.
        assert_eq!(
            transfer_timeout(100 * 1024 * 1024),
            Duration::from_secs(240 + 1600)
        );
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

    /// A transport failure's message never carries the URL: an upload's is a
    /// one-use pre-signed address, and the message reaches a toast.
    #[test]
    fn a_transport_error_does_not_name_the_url() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}/secret-presigned-path?sig=abc");
        let error = ReqwestHttp::new().unwrap().get(&url).unwrap_err();
        assert!(
            !error.message.contains("secret-presigned-path"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("sig=abc"), "{}", error.message);
    }
}
