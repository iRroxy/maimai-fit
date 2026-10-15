use super::*;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::cell::RefCell;
use std::sync::{mpsc, Arc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

const TOKEN: &str = "synthetic-bearer-for-transport-tests";
const EXACT: &str = "450.0000000000000000000000000001";

#[derive(Clone, Copy)]
enum Reply {
    Nutrition,
    ReflectedCatalog,
    ReflectedSchema,
    ReflectedRequired,
    ReflectedNestedKey,
    ReflectedNestedValue,
    NumericSchema,
    BadNutrition,
    AuthFailure,
    BadInitialize,
    BadNotification,
    DelayedNutrition,
}

struct Upstream {
    endpoint: String,
    stopped: Arc<AtomicBool>,
    calls: mpsc::Receiver<String>,
    seen: RefCell<Vec<String>>,
    arguments: mpsc::Receiver<Value>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Upstream {
    fn new(reply: Reply) -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let stopped = Arc::new(AtomicBool::new(false));
        let (observed, calls) = mpsc::channel();
        let (received_arguments, arguments) = mpsc::channel();
        let stop = stopped.clone();
        let worker = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("upstream accept: {error}"),
                };
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let method = line.split_whitespace().next().unwrap().to_owned();
                let mut length = 0;
                let mut session = None;
                loop {
                    line.clear();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" { break; }
                    assert!(!line.is_empty());
                    let (name, value) = line.trim_end().split_once(':').unwrap();
                    if name.eq_ignore_ascii_case("Content-Length") { length = value.trim().parse().unwrap(); }
                    if name.eq_ignore_ascii_case("Authorization") { assert_eq!(value.trim(), format!("Bearer {TOKEN}")); }
                    if name.eq_ignore_ascii_case("Mcp-Session-Id") { session = Some(value.trim().to_owned()); }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let message: Value = if body.is_empty() { Value::Null } else { serde_json::from_slice(&body).unwrap() };
                let rpc_method = if method == "DELETE" { "DELETE" } else { message["method"].as_str().unwrap() };
                observed.send(rpc_method.to_owned()).unwrap();
                if rpc_method == "DELETE" {
                    assert_eq!(session.as_deref(), Some("fixture-session"));
                    write!(stream, "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                }
                if matches!(reply, Reply::AuthFailure) {
                    let body = format!("untrusted authentication detail {TOKEN}");
                    write!(stream, "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                    continue;
                }
                if rpc_method == "notifications/initialized" {
                    let status = if matches!(reply, Reply::BadNotification) { 500 } else { 202 };
                    write!(stream, "HTTP/1.1 {status} Notification\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    continue;
                }
                let result = match rpc_method {
                    "initialize" => json!({"protocolVersion": if matches!(reply, Reply::BadInitialize) { "unsupported" } else { "2025-06-18" }, "capabilities": {}}),
                    "tools/list" => {
                        let required = if matches!(reply, Reply::ReflectedRequired) { TOKEN } else { "locale" };
                        let mut schema = json!({"type": "object", "required": [required], "properties": {"locale": {"type": "string"}}});
                        if matches!(reply, Reply::ReflectedNestedKey) {
                            schema["allOf"] = json!([{"properties": {}}]);
                            schema["allOf"][0]["properties"][TOKEN] = json!({"type": "string"});
                        }
                        if matches!(reply, Reply::ReflectedNestedValue) {
                            schema["properties"]["locale"]["examples"] = json!([{"nested": format!("prefix {TOKEN} suffix")}]);
                        }
                        if matches!(reply, Reply::NumericSchema) {
                            schema["minimum"] = parse_strict_json(EXACT).unwrap();
                        }
                        json!({"tools": [{"name": "list-nutrition-foods", "inputSchema": schema, "description": if matches!(reply, Reply::ReflectedSchema) { TOKEN } else { "fixture nutrition" }}]})
                    }
                    "tools/call" => {
                        assert_eq!(message["params"]["name"], "list-nutrition-foods");
                        received_arguments.send(message["params"]["arguments"].clone()).unwrap();
                        if matches!(reply, Reply::DelayedNutrition) { thread::sleep(Duration::from_millis(5200)); }
                        if matches!(reply, Reply::BadNutrition) {
                            json!({"structuredContent": {"foods": [{"name": TOKEN, "kcal": "not a number", "protein_g": "30"}]}})
                        } else {
                            let name = if matches!(reply, Reply::ReflectedCatalog) { TOKEN } else { "Exact" };
                            // A raw JSON number exercises the transport's arbitrary-precision decoding.
                            parse_strict_json(&format!("{{\"structuredContent\":{{\"foods\":[{{\"name\":\"{name}\",\"kcal\":{EXACT},\"protein_g\":30,\"sodium_mg\":null}}]}}}}" )).unwrap()
                        }
                    }
                    _ => panic!("unexpected MCP method"),
                };
                let body = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": message["id"], "result": result})).unwrap();
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: fixture-session\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        Self { endpoint, stopped, calls, seen: RefCell::new(Vec::new()), arguments, worker: Some(worker) }
    }

    fn methods(&self) -> Vec<String> {
        self.seen.borrow_mut().extend(self.calls.try_iter());
        self.seen.borrow().clone()
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn api(upstream: &Upstream, startup: Option<&CatalogDocument>, path: &str, body: &str, host: Option<&str>, origin: Option<&str>) -> (u16, Value) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let allowed_host = address.to_string();
    let allowed_origin = format!("http://{address}");
    let endpoint = upstream.endpoint.clone();
    let request = format!("POST {path} HTTP/1.1\r\nHost: {}\r\nOrigin: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", host.unwrap_or(&allowed_host), origin.unwrap_or(&allowed_origin), body.len());
    let bytes = thread::scope(|scope| {
        scope.spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
            let catalog_json = serde_json::to_vec(&startup).unwrap();
            let factory = move |token: &str| McpClient::new(token, &endpoint, 30.0);
            handle(&mut stream, startup, &catalog_json, &allowed_host, &allowed_origin, &factory).unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(15))).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).unwrap();
        bytes
    });
    let split = bytes.windows(4).position(|bytes| bytes == b"\r\n\r\n").unwrap();
    let headers = std::str::from_utf8(&bytes[..split]).unwrap();
    assert!(headers.contains("Cache-Control: no-store"));
    let status = headers.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_slice(&bytes[split + 4..]).unwrap())
}

fn fetch_body(arguments: &str) -> String {
    json!({"token": TOKEN, "arguments": arguments}).to_string()
}

#[test]
fn online_inspection_and_fetch_discover_schema_and_keep_exact_provenance() {
    let upstream = Upstream::new(Reply::Nutrition);
    let (status, tool) = api(&upstream, None, "/api/inspect", &json!({"token": format!("  {TOKEN}  ")}).to_string(), None, None);
    assert_eq!(status, 200);
    assert_eq!(tool["name"], "list-nutrition-foods");
    assert_eq!(tool["inputSchema"]["required"], json!(["locale"]));
    assert_eq!(upstream.methods(), ["initialize", "notifications/initialized", "tools/list", "DELETE"]);
    let before = chrono::Utc::now();
    let arguments = format!("{{\"locale\":\"zh-CN\",\"quantity\":{EXACT}}}");
    let (status, catalog) = api(&upstream, None, "/api/fetch", &fetch_body(&arguments), None, None);
    assert_eq!(status, 200);
    assert_eq!(catalog["source"]["kind"], "mcp");
    assert_eq!(catalog["source"]["tool"], "list-nutrition-foods");
    let fetched = chrono::DateTime::parse_from_rfc3339(catalog["source"]["fetched_at"].as_str().unwrap()).unwrap();
    assert!(fetched >= before && fetched <= chrono::Utc::now());
    assert!(catalog["source"]["note"].as_str().unwrap().contains("并非门店实时菜单"));
    assert_eq!(catalog["foods"][0]["kcal"], EXACT);
    assert_eq!(upstream.arguments.try_recv().unwrap()["quantity"].to_string(), EXACT);
    assert_eq!(upstream.methods(), ["initialize", "notifications/initialized", "tools/list", "DELETE", "initialize", "notifications/initialized", "tools/list", "tools/call", "DELETE"]);
}

#[test]
fn inspected_schema_preserves_exact_json_numbers_without_stringifying() {
    let upstream = Upstream::new(Reply::NumericSchema);
    let (status, tool) = api(&upstream, None, "/api/inspect", &json!({"token": TOKEN}).to_string(), None, None);
    assert_eq!(status, 200);
    let minimum = &tool["inputSchema"]["minimum"];
    assert!(minimum.is_number());
    assert_eq!(minimum.to_string(), EXACT);
    assert_eq!(upstream.methods().last().unwrap(), "DELETE");
}

#[test]
fn reflected_credentials_in_nested_schema_keys_and_values_are_rejected_not_rewritten() {
    for reply in [Reply::ReflectedNestedKey, Reply::ReflectedNestedValue] {
        let upstream = Upstream::new(reply);
        let (status, error) = api(&upstream, None, "/api/inspect", &json!({"token": TOKEN}).to_string(), None, None);
        assert_eq!(status, 502);
        assert!(!error.to_string().contains(TOKEN));
        assert!(error.get("inputSchema").is_none());
        assert_eq!(upstream.methods().last().unwrap(), "DELETE");
    }
}

#[test]
fn discovered_required_arguments_are_not_guessed_and_session_is_closed() {
    let upstream = Upstream::new(Reply::Nutrition);
    let (status, error) = api(&upstream, None, "/api/fetch", &json!({"token": TOKEN}).to_string(), None, None);
    assert_eq!(status, 400);
    assert!(error["error"].as_str().unwrap().contains("locale"));
    assert_eq!(upstream.methods(), ["initialize", "notifications/initialized", "tools/list", "DELETE"]);
}

#[test]
fn malformed_api_fields_and_arguments_are_rejected_before_creating_client() {
    let factory = |_: &str| -> Result<McpClient, McpError> { panic!("invalid input reached credential handling") };
    for document in [
        json!({}), json!({"token": null}), json!({"token": " "}), json!({"token": "bad token"}),
        json!({"token": "bad\u{7f}token"}), json!({"token": TOKEN, "endpoint": "https://evil.example"}),
        json!({"token": TOKEN, "tool": "other"}), json!({"token": TOKEN, "arguments": {}}),
        json!({"token": TOKEN, "arguments": "{\"x\":1,\"x\":2}"}),
        json!({"token": TOKEN, "arguments": "{\"x\":NaN}"}), json!({"token": TOKEN, "arguments": "{\"x\":Infinity}"}),
        json!({"token": TOKEN, "arguments": "[]"}), json!({"token": TOKEN, "arguments": "{} trailing"}),
    ] {
        assert_eq!(remote_request("/api/fetch", document, &factory).unwrap_err().0, 400);
    }
    for document in [json!({"token": TOKEN, "arguments": "{}"}), json!({"token": TOKEN, "endpoint": "https://evil.example"})] {
        assert_eq!(remote_request("/api/inspect", document, &factory).unwrap_err().0, 400);
    }
}

#[test]
fn host_origin_and_duplicate_json_are_rejected_before_upstream_activity() {
    let upstream = Upstream::new(Reply::Nutrition);
    let body = fetch_body("{}");
    assert_eq!(api(&upstream, None, "/api/fetch", &body, Some("localhost:1234"), None).0, 403);
    assert_eq!(api(&upstream, None, "/api/inspect", &body, None, Some("http://evil.example")).0, 403);
    assert_eq!(api(&upstream, None, "/api/fetch", "{\"token\":\"a\",\"token\":\"b\"}", None, None).0, 400);
    assert!(upstream.methods().is_empty());
}

#[test]
fn upstream_failures_and_reflections_never_escape_or_replace_startup_catalog() {
    let startup = validate_catalog(&json!({"schema_version": 1, "source": {"kind": "import"}, "foods": [{"name": "Offline", "kcal": "450", "protein_g": "30", "sodium_mg": null}]})).unwrap();
    for reply in [Reply::ReflectedCatalog, Reply::ReflectedSchema, Reply::BadNutrition, Reply::AuthFailure, Reply::BadInitialize, Reply::BadNotification, Reply::ReflectedRequired] {
        let upstream = Upstream::new(reply);
        let path = if matches!(reply, Reply::ReflectedSchema) { "/api/inspect" } else { "/api/fetch" };
        let body = if path == "/api/inspect" { json!({"token": TOKEN}).to_string() } else { fetch_body("{\"locale\":\"zh-CN\"}") };
        let (status, error) = api(&upstream, Some(&startup), path, &body, None, None);
        assert_eq!(status, if matches!(reply, Reply::ReflectedRequired) { 400 } else { 502 });
        assert!(!error.to_string().contains(TOKEN));
        assert!(error.get("foods").is_none());
        assert_eq!(startup.foods[0].name, "Offline");
        if !matches!(reply, Reply::AuthFailure) { assert_eq!(upstream.methods().last().unwrap(), "DELETE"); }
        let constraints = json!({"keep": ["Offline"], "min_kcal": "450", "max_kcal": "450", "min_protein": "30", "max_items": 1, "top": 1});
        assert_eq!(plan(json!({"constraints": constraints.clone()}), Some(&startup)).unwrap().status, "ok");
        assert!(plan(json!({"constraints": constraints}), None).unwrap_err().contains("获取"));
    }
}

#[test]
fn upstream_operations_are_not_limited_by_local_socket_io_timeout() {
    let upstream = Upstream::new(Reply::DelayedNutrition);
    let (status, catalog) = api(&upstream, None, "/api/fetch", &fetch_body("{\"locale\":\"zh-CN\"}"), None, None);
    assert_eq!(status, 200);
    assert_eq!(catalog["source"]["kind"], "mcp");
    assert_eq!(upstream.methods().last().unwrap(), "DELETE");
}
