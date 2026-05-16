use crate::webhook::{parse_event, verify_signature, JobRequest, WebhookError};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const WEBHOOK_READ_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WEBHOOK_HEADER_BYTES: usize = 16 * 1024;
const MAX_WEBHOOK_BODY_BYTES: usize = 1024 * 1024;
const MAX_WEBHOOK_CONNECTIONS: usize = 128;

pub fn serve_webhooks(
    listen_addr: &str,
    webhook_secret: Vec<u8>,
    sender: Sender<JobRequest>,
) -> Result<(), std::io::Error> {
    let listener = TcpListener::bind(listen_addr)?;
    tracing::info!(%listen_addr, "runlet webhook listener started");
    let active_connections = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                if active_connections.fetch_add(1, Ordering::AcqRel) >= MAX_WEBHOOK_CONNECTIONS {
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                    tracing::warn!("webhook connection limit reached");
                    continue;
                }
                let secret = webhook_secret.clone();
                let sender = sender.clone();
                let active_connections = active_connections.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &secret, &sender) {
                        tracing::warn!(%error, "webhook request failed");
                    }
                    active_connections.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Err(error) => tracing::warn!(%error, "failed to accept webhook connection"),
        }
    }
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    webhook_secret: &[u8],
    sender: &Sender<JobRequest>,
) -> Result<(), std::io::Error> {
    let request = HttpRequest::read(&mut stream)?;
    let response = handle_webhook_request(webhook_secret, sender, request);
    stream.write_all(&response.as_bytes())?;
    Ok(())
}

fn handle_webhook_request(
    webhook_secret: &[u8],
    sender: &Sender<JobRequest>,
    request: HttpRequest,
) -> HttpResponse {
    if request.method != "POST" || request.path != "/webhook" {
        return HttpResponse::new(404, "not found");
    }
    let signature = match request.header("x-hub-signature-256") {
        Some(signature) => signature,
        None => return HttpResponse::new(401, "missing signature"),
    };
    if verify_signature(webhook_secret, &request.body, signature).is_err() {
        return HttpResponse::new(401, "invalid signature");
    }
    let Some(event) = request.header("x-github-event") else {
        return HttpResponse::new(400, "missing event");
    };
    match parse_event(event, &request.body) {
        Ok(Some(job)) => match sender.send(job) {
            Ok(()) => HttpResponse::new(202, "queued"),
            Err(_) => HttpResponse::new(503, "job queue unavailable"),
        },
        Ok(None) => HttpResponse::new(202, "ignored"),
        Err(WebhookError::IgnoredAction(_)) => HttpResponse::new(202, "ignored"),
        Err(error) => HttpResponse::new(400, &error.to_string()),
    }
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn read(stream: &mut TcpStream) -> Result<Self, std::io::Error> {
        stream.set_read_timeout(Some(WEBHOOK_READ_TIMEOUT))?;
        Self::read_from(stream)
    }

    fn read_from(reader: &mut impl Read) -> Result<Self, std::io::Error> {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let header_end = loop {
            let count = reader.read(&mut chunk)?;
            if count == 0 {
                break None;
            }
            buffer.extend_from_slice(&chunk[..count]);
            if buffer.len() > MAX_WEBHOOK_HEADER_BYTES && find_header_end(&buffer).is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HTTP headers exceed limit",
                ));
            }
            if let Some(index) = find_header_end(&buffer) {
                break Some(index);
            }
        };
        let header_end = header_end.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing HTTP headers")
        })?;
        if header_end > MAX_WEBHOOK_HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP headers exceed limit",
            ));
        }
        let headers_text = String::from_utf8_lossy(&buffer[..header_end]);
        let mut lines = headers_text.lines();
        let request_line = lines.next().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "missing request line")
        })?;
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap_or_default().to_string();
        let path = request_parts.next().unwrap_or_default().to_string();
        let mut headers = BTreeMap::new();
        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
        let content_length = headers
            .get("content-length")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        if content_length > MAX_WEBHOOK_BODY_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP body exceeds limit",
            ));
        }
        let mut body = buffer[header_end + 4..].to_vec();
        if body.len() > content_length {
            body.truncate(content_length);
        }
        while body.len() < content_length {
            let count = reader.read(&mut chunk)?;
            if count == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "HTTP body ended before content-length",
                ));
            }
            body.extend_from_slice(&chunk[..count]);
            if body.len() > MAX_WEBHOOK_BODY_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "HTTP body exceeds limit",
                ));
            }
        }

        Ok(Self {
            method,
            path,
            headers,
            body,
        })
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

struct HttpResponse {
    status: u16,
    body: String,
}

impl HttpResponse {
    fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }

    fn as_bytes(&self) -> Vec<u8> {
        let reason = match self.status {
            202 => "Accepted",
            400 => "Bad Request",
            401 => "Unauthorized",
            404 => "Not Found",
            503 => "Service Unavailable",
            _ => "OK",
        };
        format!(
            "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n{}",
            self.status,
            reason,
            self.body.len(),
            self.body
        )
        .into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use std::io::Cursor;
    use std::sync::mpsc;

    fn signed_request(body: &[u8]) -> HttpRequest {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        HttpRequest {
            method: "POST".to_string(),
            path: "/webhook".to_string(),
            headers: BTreeMap::from([
                ("x-hub-signature-256".to_string(), signature),
                ("x-github-event".to_string(), "workflow_job".to_string()),
            ]),
            body: body.to_vec(),
        }
    }

    #[test]
    fn queues_valid_webhook_request() {
        let body = br#"{
            "action": "queued",
            "workflow_job": {
                "id": 42,
                "head_branch": "main",
                "labels": ["self-hosted"],
                "pull_requests": []
            },
            "repository": {
                "full_name": "org/project",
                "html_url": "https://github.com/org/project"
            }
        }"#;
        let (sender, receiver) = mpsc::channel();
        let response = handle_webhook_request(b"secret", &sender, signed_request(body));

        assert_eq!(response.status, 202);
        assert_eq!(receiver.recv().unwrap().github_job_id, 42);
    }

    #[test]
    fn rejects_bad_signature() {
        let (sender, _receiver) = mpsc::channel();
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/webhook".to_string(),
            headers: BTreeMap::from([
                ("x-hub-signature-256".to_string(), "sha256=bad".to_string()),
                ("x-github-event".to_string(), "workflow_job".to_string()),
            ]),
            body: b"{}".to_vec(),
        };
        let response = handle_webhook_request(b"secret", &sender, request);

        assert_eq!(response.status, 401);
    }

    #[test]
    fn rejects_oversized_webhook_headers() {
        let request = format!(
            "GET /webhook HTTP/1.1\r\nX-Fill: {}\r\n\r\n",
            "a".repeat(MAX_WEBHOOK_HEADER_BYTES)
        );
        let error = HttpRequest::read_from(&mut Cursor::new(request.into_bytes())).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_oversized_webhook_body() {
        let request = format!(
            "POST /webhook HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_WEBHOOK_BODY_BYTES + 1
        );
        let error = HttpRequest::read_from(&mut Cursor::new(request.into_bytes())).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_short_webhook_body() {
        let request = "POST /webhook HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort";
        let error = HttpRequest::read_from(&mut Cursor::new(request.as_bytes())).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }
}
