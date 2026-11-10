//! A one-shot HTTP server for tests: serves one canned response per
//! connection, in order, on a random local port, and keeps every request
//! (head and body) so a test can check what was sent.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub content_type: &'static str,
    /// `{base}` in a value is replaced with the server's base URL.
    pub headers: Vec<(&'static str, String)>,
}

impl Reply {
    pub fn json(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.as_bytes().to_vec(),
            content_type: "application/json",
            headers: Vec::new(),
        }
    }

    pub fn header(mut self, name: &'static str, value: &str) -> Self {
        self.headers.push((name, value.to_string()));
        self
    }
}

/// One request as received: the head percent-decoded (`+` as space) for
/// easy matching, and the raw body.
#[derive(Clone, Debug)]
pub struct Request {
    pub head: String,
    pub body: Vec<u8>,
}

impl Request {
    pub fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

pub type Log = Arc<Mutex<Vec<Request>>>;

pub fn serve_replies(replies: Vec<Reply>) -> (String, Log, thread::JoinHandle<()>) {
    let log: Log = Arc::new(Mutex::new(Vec::new()));
    let seen = log.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_base = base.clone();
    // A test whose code makes fewer requests than expected must fail,
    // not hang: give up after five seconds without a connection.
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        for reply in replies {
            let Some(mut stream) = accept_within(&listener, std::time::Duration::from_secs(5))
            else {
                return;
            };
            seen.lock().unwrap().push(read_request(&mut stream));
            let reason = if reply.status == 200 { "OK" } else { "Status" };
            let mut head = format!(
                "HTTP/1.1 {} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reply.content_type,
                reply.body.len()
            );
            for (name, value) in &reply.headers {
                head.push_str(&format!(
                    "{name}: {}\r\n",
                    value.replace("{base}", &server_base)
                ));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&reply.body);
        }
    });
    (base, log, handle)
}

fn accept_within(listener: &TcpListener, wait: std::time::Duration) -> Option<TcpStream> {
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok()?;
                return Some(stream);
            }
            Err(_) => thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
    None
}

/// Read the head up to the blank line, then `Content-Length` bytes.
fn read_request(stream: &mut TcpStream) -> Request {
    let mut data = Vec::new();
    let mut buffer = [0u8; 65536];
    let head_end = loop {
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break data.len(),
            Ok(read) => data.extend_from_slice(&buffer[..read]),
        }
    };
    let raw_head = String::from_utf8_lossy(&data[..head_end]).into_owned();
    let length = raw_head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while data.len() < head_end + length {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => data.extend_from_slice(&buffer[..read]),
        }
    }
    let head = percent_encoding::percent_decode_str(&raw_head.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned();
    Request {
        head,
        body: data[head_end..].to_vec(),
    }
}

fn to_replies(responses: Vec<(u16, Vec<u8>, &'static str)>) -> Vec<Reply> {
    responses
        .into_iter()
        .map(|(status, body, content_type)| Reply {
            status,
            body,
            content_type,
            headers: Vec::new(),
        })
        .collect()
}

pub fn serve(responses: Vec<(u16, Vec<u8>, &'static str)>) -> (String, thread::JoinHandle<()>) {
    let (base, _log, handle) = serve_replies(to_replies(responses));
    (base, handle)
}

/// Like `serve`, keeping each request's decoded head for assertions.
pub fn serve_logged(
    responses: Vec<(u16, Vec<u8>, &'static str)>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let (base, log, handle) = serve_replies(to_replies(responses));
    // A view of the heads only, filled in as requests arrive.
    let heads = Arc::new(Mutex::new(Vec::new()));
    let (log_view, heads_view) = (log.clone(), heads.clone());
    let joined = thread::spawn(move || {
        handle.join().unwrap();
        *heads_view.lock().unwrap() = log_view
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.head.clone())
            .collect();
    });
    (base, heads, joined)
}

pub fn json(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}
