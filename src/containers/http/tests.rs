use super::*;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

pub(crate) enum Reply {
    Json(serde_json::Value),
    Raw(Vec<u8>),
    Stall,
}

pub(crate) struct Server {
    directory: Option<tempfile::TempDir>,
    pub socket: std::path::PathBuf,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    pub paths: Arc<Mutex<Vec<String>>>,
    pub peak: Arc<AtomicUsize>,
}

impl Server {
    pub fn new(handler: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let socket = directory.path().join("engine.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let paths = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let worker = {
            let stop = stop.clone();
            let paths = paths.clone();
            let peak = peak.clone();
            let active = Arc::new(AtomicUsize::new(0));
            let handler = Arc::new(handler);
            thread::spawn(move || {
                let mut children = Vec::new();
                let deadline = Instant::now() + Duration::from_secs(10);
                while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                    let (mut socket, _) = match listener.accept() {
                        Ok(pair) => pair,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(1));
                            continue;
                        }
                        Err(error) => panic!("fixture accept: {error}"),
                    };
                    let stop = stop.clone();
                    let paths = paths.clone();
                    let peak = peak.clone();
                    let active = active.clone();
                    let handler = handler.clone();
                    children.push(thread::spawn(move || {
                        socket.set_nonblocking(false).unwrap();
                        socket.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
                        socket.set_write_timeout(Some(Duration::from_secs(1))).unwrap();
                        let mut reader = BufReader::new(&socket);
                        let mut line = String::new();
                        if reader.read_line(&mut line).is_err() { return; }
                        let path = line.split_whitespace().nth(1).unwrap().to_owned();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" { break; }
                        }
                        paths.lock().unwrap().push(path.clone());
                        peak.fetch_max(active.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                        match handler(&path) {
                            Reply::Json(value) => {
                                let body = value.to_string();
                                let _ = write!(socket, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                            }
                            Reply::Raw(bytes) => { let _ = bytes.chunks(8192).try_for_each(|chunk| socket.write_all(chunk)); }
                            Reply::Stall => {
                                while !stop.load(Ordering::Relaxed) && Instant::now() < deadline {
                                    match socket.read(&mut [0]) {
                                        Ok(0) => break,
                                        Ok(_) => {}
                                        Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                                        Err(_) => break,
                                    }
                                }
                            }
                        }
                        active.fetch_sub(1, Ordering::SeqCst);
                    }));
                }
                for child in children {
                    child.join().unwrap();
                }
            })
        };
        Self {
            directory: Some(directory),
            socket,
            stop,
            worker: Some(worker),
            paths,
            peak,
        }
    }

    pub fn client<'a>(&'a self, cancel: &'a Cancellation) -> Client<'a> {
        Client {
            socket: &self.socket,
            cancel,
            deadline: Instant::now() + Duration::from_secs(5),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
        self.directory.take().unwrap().close().unwrap();
    }
}

#[test]
fn chunked_json_and_multiple_requests_use_bounded_concurrency() {
    let server = Server::new(|_| {
        thread::sleep(Duration::from_millis(20));
        Reply::Raw(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n".to_vec())
    });
    let cancel = Cancellation::default();
    let responses: Vec<_> = server
        .client(&cancel)
        .batch(&(0..9).map(|i| format!("/{i}")).collect::<Vec<_>>())
        .collect();
    assert_eq!(responses.len(), 9);
    for response in responses {
        assert_eq!(
            response.unwrap().json::<serde_json::Value>().unwrap(),
            serde_json::json!({"a":1})
        );
    }
    assert_eq!(server.paths.lock().unwrap().len(), 9);
    assert!(server.peak.load(Ordering::Relaxed) <= MAX_CONCURRENT);
    assert!(server.peak.load(Ordering::Relaxed) > 1);
}

#[test]
fn redirects_errors_and_invalid_bodies_are_not_followed_or_exposed() {
    for response in [
        b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private\r\nContent-Length: 0\r\n\r\n"
            .to_vec(),
        b"HTTP/1.1 503 Error\r\nContent-Length: 14\r\n\r\nprivate-secret".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 14\r\n\r\nprivate-secret".to_vec(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nprivate-secret".to_vec(),
    ] {
        let server = Server::new(move |_| Reply::Raw(response.clone()));
        let cancel = Cancellation::default();
        let error = server
            .client(&cancel)
            .get("/version")
            .and_then(|r| r.json::<serde_json::Value>())
            .unwrap_err()
            .to_string();
        assert!(!error.contains("private"));
        assert_eq!(server.paths.lock().unwrap().len(), 1);
    }
}

#[test]
fn oversized_chunked_response_is_rejected() {
    let server = Server::new(|_| {
        let mut bytes = format!(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{:x}\r\n",
            MAX_BODY_BYTES + 1
        )
        .into_bytes();
        bytes.resize(bytes.len() + MAX_BODY_BYTES + 1, b'x');
        bytes.extend_from_slice(b"\r\n0\r\n\r\n");
        Reply::Raw(bytes)
    });
    let cancel = Cancellation::default();
    let error = server
        .client(&cancel)
        .get("/version")
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("size limit"), "{error}");
}

#[test]
fn cancellation_and_deadline_close_stalled_requests() {
    for cancel_request in [true, false] {
        let server = Server::new(|_| Reply::Stall);
        let cancel = Cancellation::default();
        thread::scope(|scope| {
            let client = Client {
                socket: &server.socket,
                cancel: &cancel,
                deadline: Instant::now()
                    + Duration::from_millis(if cancel_request { 2000 } else { 80 }),
            };
            let start = Instant::now();
            let task = scope.spawn(move || client.get("/version"));
            if cancel_request {
                while server.paths.lock().unwrap().is_empty()
                    && start.elapsed() < Duration::from_secs(1)
                {
                    thread::sleep(Duration::from_millis(1));
                }
                cancel.cancel();
            }
            let error = task.join().unwrap().err().unwrap().to_string();
            assert!(
                error.contains(if cancel_request {
                    "canceled"
                } else {
                    "deadline"
                }),
                "cancel={cancel_request}: {error}"
            );
            assert!(start.elapsed() < Duration::from_millis(1500));
        });
    }
}
