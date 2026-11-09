//! Waiting for the browser to come back after a sign-in: the crate listens on
//! `127.0.0.1:<random>` and takes the first request whose URL Arbesk's sign-in
//! accepts. (The app keeps the same helper for Google Drive's OAuth flow.)

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use url::Url;

/// Why the wait ended without a request the caller accepted.
#[derive(Debug)]
pub enum Waited {
    Cancelled,
    TimedOut,
    Io(String),
}

/// Wait on `listener` until `accept` takes a request's URL
/// (`http://127.0.0.1<target>`), `cancelled` says so, or `timeout` passes.
/// A request `accept` declines (a browser's `/favicon.ico`, say) is answered
/// 404 and the wait goes on; the accepted one is answered 200 with
/// `done_text`. A connection that goes quiet or fails (a browser preconnect)
/// is dropped; only the deadline or a cancel ends the wait.
pub fn wait_for_callback<T>(
    listener: &TcpListener,
    timeout: Duration,
    cancelled: impl Fn() -> bool,
    mut accept: impl FnMut(&Url) -> Option<T>,
    done_text: &str,
) -> Result<T, Waited> {
    listener
        .set_nonblocking(true)
        .map_err(|error| Waited::Io(error.to_string()))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cancelled() {
            return Err(Waited::Cancelled);
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let until = deadline.min(Instant::now() + REQUEST_TIMEOUT);
                let Some(request) = read_request(&mut stream, until, &cancelled) else {
                    continue;
                };
                match request_url(&request).as_ref().and_then(&mut accept) {
                    Some(value) => {
                        respond(&mut stream, 200, done_text);
                        return Ok(value);
                    }
                    None => respond(&mut stream, 404, "Not found"),
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(Waited::Io(error.to_string())),
        }
    }
    Err(Waited::TimedOut)
}

/// The URL a request head asks for, as `http://127.0.0.1<target>`.
fn request_url(request: &str) -> Option<Url> {
    let target = request.lines().next()?.split_whitespace().nth(1)?;
    Url::parse(&format!("http://127.0.0.1{target}")).ok()
}

/// How long one connection may take to send its request head.
const REQUEST_TIMEOUT: Duration = if cfg!(test) {
    Duration::from_millis(300)
} else {
    Duration::from_secs(5)
};

/// Read the request head. TCP may deliver it in several pieces, so keep
/// reading until the blank line that ends the headers, the peer closes, or
/// 16 KB arrived. `None` when the read fails, `until` passes or `give_up`
/// says so; reads are short so a cancel is noticed while the peer is quiet.
pub(crate) fn read_request(
    stream: &mut TcpStream,
    until: Instant,
    give_up: impl Fn() -> bool,
) -> Option<String> {
    const LIMIT: usize = 16 * 1024;
    stream.set_nonblocking(false).ok();
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    while request.len() < LIMIT && !request.windows(4).any(|w| w == b"\r\n\r\n") {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() || give_up() {
            return None;
        }
        stream
            .set_read_timeout(Some(left.min(Duration::from_millis(100))))
            .ok();
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
            // A signal, or a quiet moment: check the deadline and read again.
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }
    Some(String::from_utf8_lossy(&request).into_owned())
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
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::thread;
    use std::time::{Duration, Instant};

    /// Send one GET and return the response (status line and body).
    fn call(addr: SocketAddr, target: &str) -> String {
        let mut stream = TcpStream::connect(addr).unwrap();
        write!(
            stream,
            "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        let _ = stream.read_to_string(&mut response);
        response
    }

    fn token_of(url: &url::Url) -> Option<String> {
        url.query_pairs()
            .find(|(key, _)| key == "token")
            .map(|(_, value)| value.into_owned())
    }

    #[test]
    fn ignores_favicon_then_returns_the_callback() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || (call(addr, "/favicon.ico"), call(addr, "/?token=t")));
        let got = wait_for_callback(
            &listener,
            Duration::from_secs(5),
            || false,
            |url| token_of(url),
            "Done.",
        );
        let (first, _) = worker.join().unwrap();
        assert_eq!(got.unwrap(), "t");
        assert!(first.starts_with("HTTP/1.1 404"), "{first}");
    }

    #[test]
    fn answers_with_done_text() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let worker = thread::spawn(move || call(addr, "/?token=t"));
        wait_for_callback(
            &listener,
            Duration::from_secs(5),
            || false,
            |url| token_of(url),
            "All done here.",
        )
        .unwrap();
        let response = worker.join().unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("All done here."), "{response}");
    }

    #[test]
    fn stops_when_cancelled() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let begun = Instant::now();
        let got = wait_for_callback(
            &listener,
            Duration::from_secs(30),
            || true,
            |url| token_of(url),
            "Done.",
        );
        assert!(matches!(got, Err(Waited::Cancelled)));
        assert!(begun.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn times_out() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let got = wait_for_callback(
            &listener,
            Duration::from_millis(50),
            || false,
            |url| token_of(url),
            "Done.",
        );
        assert!(matches!(got, Err(Waited::TimedOut)));
    }

    /// A peer that sends one byte at a time until it is disconnected.
    fn trickle(addr: SocketAddr) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.set_nodelay(true).unwrap();
            while stream.write_all(b"G").is_ok() {
                thread::sleep(Duration::from_millis(20));
            }
        })
    }

    #[test]
    fn a_trickling_request_is_given_up_at_its_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = trickle(listener.local_addr().unwrap());
        let (mut stream, _) = listener.accept().unwrap();
        let begun = Instant::now();
        let read = read_request(&mut stream, begun + Duration::from_millis(200), || false);
        assert!(read.is_none());
        assert!(begun.elapsed() < Duration::from_secs(1));
        drop(stream);
        peer.join().unwrap();
    }

    #[test]
    fn a_trickling_request_cannot_hold_off_a_cancel() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = trickle(listener.local_addr().unwrap());
        let (mut stream, _) = listener.accept().unwrap();
        let begun = Instant::now();
        let cancel_at = begun + Duration::from_millis(200);
        let read = read_request(&mut stream, begun + Duration::from_secs(30), || {
            Instant::now() >= cancel_at
        });
        assert!(read.is_none());
        assert!(begun.elapsed() < Duration::from_secs(1));
        drop(stream);
        peer.join().unwrap();
    }
}
