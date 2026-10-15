use maimai::mcp::McpClient;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

const TOKEN: &str = "private-test-token-never-display";

type HandlerFn = Arc<
    dyn Fn(&str, Option<&Value>, &HashMap<String, String>) -> (u16, Vec<(&'static str, String)>, Vec<u8>)
        + Send
        + Sync,
>;

struct TestServer {
    pub endpoint: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TestServer {
    fn new(handler: HandlerFn) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = format!("http://127.0.0.1:{port}/mcp");
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = shutdown.clone();

        listener.set_nonblocking(true).unwrap();

        let handle = thread::spawn(move || {
            while !shutdown_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut reader = BufReader::new(stream.try_clone().unwrap());
                        let mut req_line = String::new();
                        if reader.read_line(&mut req_line).is_err() || req_line.is_empty() {
                            continue;
                        }

                        let parts: Vec<&str> = req_line.split_whitespace().collect();
                        if parts.is_empty() {
                            continue;
                        }
                        let method = parts[0].to_string();

                        let mut headers = HashMap::new();
                        let mut content_length = 0;

                        loop {
                            let mut header_line = String::new();
                            if reader.read_line(&mut header_line).is_err() || header_line == "\r\n" || header_line == "\n" {
                                break;
                            }
                            if let Some((k, v)) = header_line.split_once(':') {
                                let key = k.trim().to_ascii_lowercase();
                                let val = v.trim().to_string();
                                if key == "content-length" {
                                    content_length = val.parse().unwrap_or(0);
                                }
                                headers.insert(key, val);
                            }
                        }

                        let body = if content_length > 0 {
                            let mut buf = vec![0u8; content_length];
                            let _ = reader.read_exact(&mut buf);
                            Some(buf)
                        } else {
                            None
                        };

                        let json_body: Option<Value> = body.as_ref().and_then(|b| serde_json::from_slice(b).ok());

                        let (status, resp_headers, data) = handler(&method, json_body.as_ref(), &headers);

                        let mut resp = format!("HTTP/1.1 {status} OK\r\n");
                        for (k, v) in resp_headers {
                            resp.push_str(&format!("{k}: {v}\r\n"));
                        }
                        resp.push_str(&format!("Content-Length: {}\r\n", data.len()));
                        resp.push_str("Connection: close\r\n\r\n");

                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.write_all(&data);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            endpoint,
            shutdown,
            handle: Some(handle),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn json_reply(status: u16, val: Value, extra_headers: &[(&'static str, String)]) -> (u16, Vec<(&'static str, String)>, Vec<u8>) {
    let mut headers = vec![("Content-Type", "application/json".to_string())];
    for (k, v) in extra_headers {
        headers.push((*k, v.clone()));
    }
    let data = serde_json::to_vec(&val).unwrap();
    (status, headers, data)
}

fn default_responder(
    method: &str,
    msg: Option<&Value>,
    _headers: &HashMap<String, String>,
) -> (u16, Vec<(&'static str, String)>, Vec<u8>) {
    if method == "POST" {
        if let Some(m) = msg {
            if m.get("method") == Some(&json!("initialize")) {
                let id = m.get("id").cloned().unwrap_or(json!(1));
                return json_reply(
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {},
                            "serverInfo": { "name": "test", "version": "1" }
                        }
                    }),
                    &[("Mcp-Session-Id", "test-session".to_string())],
                );
            } else if m.get("method") == Some(&json!("notifications/initialized")) {
                return (202, vec![], vec![]);
            }
        }
    }
    (204, vec![], vec![])
}

#[test]
fn test_sse_selects_matching_id_not_notification_or_other_response() {
    let server = TestServer::new(Arc::new(|method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("initialize")) {
            let id = msg.unwrap().get("id").unwrap().as_i64().unwrap();
            let stream = format!(
                ": keepalive\r\n\r\n\
                event: message\r\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}}\r\n\r\n\
                data: {{\"jsonrpc\":\"2.0\",\"id\":true,\"result\":{{\"protocolVersion\":\"bad\"}}}}\r\n\r\n\
                data: {{\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{{\"protocolVersion\":\"bad\"}}}}\r\n\r\n\
                event: message\r\ndata: {{\"jsonrpc\":\"2.0\",\r\n\
                data: \"id\":{id},\"result\":{{\"protocolVersion\":\"2025-06-18\",\"selected\":\"correct\"}}}}\r\n\r\n"
            );
            return (
                200,
                vec![
                    ("Content-Type", "text/event-stream".to_string()),
                    ("Mcp-Session-Id", "test-session".to_string()),
                ],
                stream.into_bytes(),
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 5.0).unwrap();
    let res = client.initialize().unwrap();
    assert_eq!(res["selected"], "correct");
    client.close();
}

#[test]
fn test_sse_without_matching_id_fails_even_with_valid_other_response() {
    let server = TestServer::new(Arc::new(|method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("initialize")) {
            let stream = "data: {\"jsonrpc\":\"2.0\",\"id\":999,\"result\":{\"protocolVersion\":\"2025-03-26\"}}\n\n";
            return (
                200,
                vec![("Content-Type", "text/event-stream".to_string())],
                stream.as_bytes().to_vec(),
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    assert!(client.initialize().is_err());
}

#[test]
fn test_json_response_id_requires_same_type_and_value() {
    let test_ids = vec![json!(true), json!("1"), json!(2)];

    for bad_id in test_ids {
        let bad_id_clone = bad_id.clone();
        let server = TestServer::new(Arc::new(move |method, msg, headers| {
            if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("initialize")) {
                return json_reply(
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": bad_id_clone,
                        "result": { "protocolVersion": "2025-03-26" }
                    }),
                    &[],
                );
            }
            default_responder(method, msg, headers)
        }));

        let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
        assert!(client.initialize().is_err());
    }
}

#[test]
fn test_auth_error_never_quotes_remote_body_or_credentials() {
    for status in [401, 403] {
        let server = TestServer::new(Arc::new(move |_method, _msg, _headers| {
            (
                status,
                vec![("WWW-Authenticate", format!("Bearer {TOKEN}"))],
                format!("Authorization: Bearer {TOKEN}").into_bytes(),
            )
        }));

        let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
        let err = client.initialize().unwrap_err();
        let text = err.to_string();

        assert!(text.contains(&status.to_string()));
        assert!(!text.contains(TOKEN));
        assert!(!text.contains("Authorization"));
    }
}

#[test]
fn test_redirect_does_not_contact_target_with_bearer() {
    let target_received = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let target_received_clone = target_received.clone();

    let target_server = TestServer::new(Arc::new(move |_method, _msg, headers| {
        target_received_clone.lock().push(headers.clone());
        (200, vec![], vec![])
    }));

    let target_url = target_server.endpoint.clone();
    let redirect_server = TestServer::new(Arc::new(move |_method, _msg, _headers| {
        (307, vec![("Location", target_url.clone())], vec![])
    }));

    let mut client = McpClient::new(TOKEN, &redirect_server.endpoint, 2.0).unwrap();
    assert!(client.initialize().is_err());
    assert!(target_received.lock().is_empty());
}

#[test]
fn test_unsupported_protocol_and_bad_session_fail_safely() {
    let cases = [
        ("2099-01-01", "valid"),
        ("2025-03-26", "invalid session with spaces"),
    ];

    for (proto, sess) in cases {
        let server = TestServer::new(Arc::new(move |method, msg, headers| {
            if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("initialize")) {
                let id = msg.unwrap().get("id").cloned().unwrap();
                return json_reply(
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "protocolVersion": proto }
                    }),
                    &[("Mcp-Session-Id", sess.to_string())],
                );
            }
            default_responder(method, msg, headers)
        }));

        let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
        assert!(client.initialize().is_err());
    }
}

#[test]
fn test_session_cannot_change_midstream() {
    let server = TestServer::new(Arc::new(|method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/list")) {
            let id = msg.unwrap().get("id").cloned().unwrap();
            return json_reply(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [] }
                }),
                &[("Mcp-Session-Id", "different-session".to_string())],
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    assert!(client.nutrition_tool().is_err());
}

#[test]
fn test_pagination_repeated_cursor_fails() {
    let pages = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let pages_clone = pages.clone();

    let server = TestServer::new(Arc::new(move |method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/list")) {
            pages_clone.lock().push(msg.and_then(|m| m.get("params")).cloned());
            let id = msg.unwrap().get("id").cloned().unwrap();
            return json_reply(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": [], "nextCursor": "same" }
                }),
                &[],
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    assert!(client.nutrition_tool().is_err());
    assert_eq!(pages.lock().len(), 2);
}

#[test]
fn test_discovered_required_fields_gate_calls_without_guessing() {
    let calls = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let calls_clone = calls.clone();

    let server = TestServer::new(Arc::new(move |method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/list")) {
            let id = msg.unwrap().get("id").cloned().unwrap();
            let cursor = msg.and_then(|m| m.get("params")).and_then(|p| p.get("cursor"));
            if cursor.is_none() {
                return json_reply(
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "tools": [], "nextCursor": "page-two" }
                    }),
                    &[],
                );
            }
            return json_reply(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [{
                            "name": "list-nutrition-foods",
                            "description": "untrusted text",
                            "inputSchema": {
                                "type": "object",
                                "properties": { "region": { "type": "string" } },
                                "required": ["region"]
                            }
                        }]
                    }
                }),
                &[],
            );
        }
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/call")) {
            calls_clone.lock().push(msg.and_then(|m| m.get("params")).cloned());
            let id = msg.unwrap().get("id").cloned().unwrap();
            return json_reply(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "content": [{ "type": "text", "text": "actual result" }] }
                }),
                &[],
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    assert!(client.call_nutrition(&json!({})).is_err());
    assert!(client.call_nutrition(&json!({ "region": null })).is_err());
    assert!(calls.lock().is_empty());
    client.close();
}

#[test]
fn test_structured_nutrition_decimals_keep_remote_precision() {
    let server = TestServer::new(Arc::new(|method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/list")) {
            let id = msg.unwrap().get("id").cloned().unwrap();
            return json_reply(
                200,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [{
                            "name": "list-nutrition-foods",
                            "inputSchema": { "type": "object" }
                        }]
                    }
                }),
                &[],
            );
        }
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/call")) {
            let id = msg.unwrap().get("id").unwrap().as_i64().unwrap();
            let body = format!(
                r#"{{"jsonrpc":"2.0","id":{id},"result":{{"structuredContent":{{"foods":[{{"name":"A","kcal":500.00000000000000000000000000001,"protein_g":30}}]}}}}}}"#
            );
            return (
                200,
                vec![("Content-Type", "application/json".to_string())],
                body.into_bytes(),
            );
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    let res = client.call_nutrition(&json!({})).unwrap();
    let kcal = res["structuredContent"]["foods"][0]["kcal"].to_string();
    assert_eq!(kcal, "500.00000000000000000000000000001");
    client.close();
}

#[test]
fn test_session_and_negotiated_protocol_survive_until_delete() {
    let deletes = Arc::new(parking_lot::Mutex::new(0));
    let deletes_clone = deletes.clone();

    let server = TestServer::new(Arc::new(move |method, msg, headers| {
        if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("initialize")) {
            return default_responder(method, msg, headers);
        }
        if headers.get("mcp-session-id").map(|s| s.as_str()) != Some("test-session")
            || headers.get("mcp-protocol-version").map(|s| s.as_str()) != Some("2025-06-18")
        {
            return (400, vec![], b"bad session/protocol".to_vec());
        }
        if method == "DELETE" {
            *deletes_clone.lock() += 1;
        }
        (204, vec![], vec![])
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    client.initialize().unwrap();
    client.close();
    client.close();
    assert_eq!(*deletes.lock(), 1);
}

#[test]
fn test_tool_and_jsonrpc_errors_are_sanitized() {
    for rpc_error in [true, false] {
        let server = TestServer::new(Arc::new(move |method, msg, headers| {
            if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/list")) {
                let id = msg.unwrap().get("id").cloned().unwrap();
                return json_reply(
                    200,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "tools": [{
                                "name": "list-nutrition-foods",
                                "inputSchema": { "type": "object" }
                            }]
                        }
                    }),
                    &[],
                );
            }
            if method == "POST" && msg.and_then(|m| m.get("method")) == Some(&json!("tools/call")) {
                let id = msg.unwrap().get("id").cloned().unwrap();
                if rpc_error {
                    return json_reply(
                        200,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -1, "message": TOKEN }
                        }),
                        &[],
                    );
                } else {
                    return json_reply(
                        200,
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": {
                                "isError": true,
                                "content": [{ "type": "text", "text": TOKEN }]
                            }
                        }),
                        &[],
                    );
                }
            }
            default_responder(method, msg, headers)
        }));

        let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
        let err = client.call_nutrition(&json!({})).unwrap_err();
        assert!(!err.to_string().contains(TOKEN));
    }
}

#[test]
fn test_close_is_best_effort_and_no_delete_without_session() {
    let deletes = Arc::new(parking_lot::Mutex::new(0));
    let deletes_clone = deletes.clone();

    let server = TestServer::new(Arc::new(move |method, msg, headers| {
        if method == "DELETE" {
            *deletes_clone.lock() += 1;
            return (500, vec![], TOKEN.as_bytes().to_vec());
        }
        default_responder(method, msg, headers)
    }));

    let mut client = McpClient::new(TOKEN, &server.endpoint, 2.0).unwrap();
    client.close();
    assert_eq!(*deletes.lock(), 0);

    client.initialize().unwrap();
    client.close();
    assert_eq!(*deletes.lock(), 1);
}

#[test]
fn test_remote_plain_http_and_unsafe_token_are_rejected() {
    let bad_endpoints = [
        "http://example.com/mcp",
        "https://user:password@example.com/mcp",
        "file:///tmp/mcp",
    ];

    for ep in bad_endpoints {
        assert!(McpClient::new(TOKEN, ep, 2.0).is_err());
    }

    assert!(McpClient::new("secret\r\nInjected: value", "https://mcp.mcd.cn", 2.0).is_err());
}
