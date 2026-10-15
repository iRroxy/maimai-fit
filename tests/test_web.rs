use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::tempdir;

const BIN: &str = env!("CARGO_BIN_EXE_maimai");

struct WebServer {
    child: Child,
    host: String,
}

impl Drop for WebServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl WebServer {
    fn start(binary: &Path, cwd: &Path, extra: &[&str]) -> Self {
        let child = Command::new(binary).args(["serve", "--port", "0"]).args(extra)
            .current_dir(cwd).env_remove("MCD_MCP_TOKEN")
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().unwrap();
        let mut server = Self { child, host: String::new() };
        let stdout = server.child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let _ = BufReader::new(stdout).read_line(&mut line);
            let _ = sender.send(line);
        });
        let url = receiver.recv_timeout(Duration::from_secs(10)).expect("server startup URL");
        server.host = url.trim().strip_prefix("http://").unwrap().strip_suffix('/').unwrap().to_string();
        assert!(server.host.starts_with("127.0.0.1:"));
        server
    }

    fn raw(&self, request: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mut stream = TcpStream::connect(&self.host).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
        stream.write_all(request).unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let status = line.split_whitespace().nth(1).unwrap().parse().unwrap();
        let mut headers = Vec::new();
        let mut length = None;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" { break; }
            assert!(!line.is_empty(), "response headers must terminate");
            let (name, value) = line.trim_end().split_once(':').unwrap();
            if name.eq_ignore_ascii_case("Content-Length") { length = Some(value.trim().parse::<usize>().unwrap()); }
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
        let mut body = vec![0; length.expect("bounded response length")];
        reader.read_exact(&mut body).unwrap();
        (status, headers, body)
    }

    fn get(&self, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
        self.raw(format!("GET {path} HTTP/1.1\r\nHost: {}\r\n\r\n", self.host).as_bytes())
    }

    fn post(&self, path: &str, body: &str) -> (u16, Value) {
        let request = format!("POST {path} HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\n\r\n{body}", self.host, self.host, body.len());
        let (status, _, response) = self.raw(request.as_bytes());
        (status, serde_json::from_slice(&response).unwrap())
    }
}

fn catalog() -> Value {
    json!({
        "schema_version": 1, "source": {"kind": "import"},
        "foods": [{"name": "Exact", "kcal": "450.0000000000000000000000000001", "protein_g": "30", "sodium_mg": null}]
    })
}

fn constraints() -> Value {
    json!({"keep": ["Exact"], "min_kcal": "450.0000000000000000000000000001", "max_kcal": "450.0000000000000000000000000001", "min_protein": "30", "max_items": 1, "top": 1})
}

#[test]
fn standalone_server_keeps_imports_per_request_and_prevents_sensitive_caching() {
    let directory = tempdir().unwrap();
    let binary = directory.path().join("standalone-maimai");
    fs::copy(BIN, &binary).unwrap();
    let server = WebServer::start(&binary, directory.path(), &[]);
    let (status, headers, body) = server.get("/api/catalog");
    assert_eq!(status, 200);
    for (name, expected) in [("cache-control", "no-store"), ("x-content-type-options", "nosniff"), ("referrer-policy", "no-referrer")] {
        assert!(headers.iter().any(|(key, value)| key == name && value == expected));
    }
    assert!(headers.iter().any(|(key, value)| key == "content-security-policy" && value.contains("frame-ancestors 'none'")));
    assert!(!headers.iter().any(|(key, _)| key.starts_with("access-control-")));
    let original: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(original, Value::Null);
    let (status, imported) = server.post("/api/catalog", &catalog().to_string());
    assert_eq!(status, 200);
    assert_eq!(imported["foods"][0]["kcal"], catalog()["foods"][0]["kcal"]);
    assert_eq!(serde_json::from_slice::<Value>(&server.get("/api/catalog").2).unwrap(), original);
    let (status, error) = server.post("/api/plan", &json!({"constraints": constraints()}).to_string());
    assert_eq!(status, 400);
    assert!(error["error"].as_str().unwrap().contains("获取"));
    assert_eq!(server.get("/../Cargo.toml").0, 404);
    assert_eq!(server.get("/api/unknown").0, 404);
}

#[test]
fn server_rejects_rebinding_cross_origin_and_unbounded_requests_without_draining() {
    let directory = tempdir().unwrap();
    let server = WebServer::start(Path::new(BIN), directory.path(), &[]);
    for headers in [String::new(), "Host: localhost\r\n".into(), format!("Host: {}\r\nHost: {}\r\n", server.host, server.host)] {
        assert_eq!(server.raw(format!("GET / HTTP/1.1\r\n{headers}\r\n").as_bytes()).0, 403);
    }
    for (headers, expected) in [
        ("Origin: http://evil.example\r\nContent-Type: application/json\r\nContent-Length: 0\r\n".to_string(), 403),
        (format!("Origin: http://{}\r\n", server.host), 400),
        (format!("Origin: http://{}\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n", server.host), 400),
    ] {
        let request = format!("POST /api/catalog HTTP/1.1\r\nHost: {}\r\n{headers}\r\n", server.host);
        assert_eq!(server.raw(request.as_bytes()).0, expected);
    }
    for (length_headers, expected) in [
        ("", 400), ("Content-Length: -1\r\n", 400),
        ("Content-Length: 0\r\nContent-Length: 0\r\n", 400),
        ("Transfer-Encoding: chunked\r\n", 400),
        ("Content-Length: 18446744073709551615\r\n", 413),
        ("Content-Length: 2097153\r\n", 413),
    ] {
        let request = format!("POST /api/catalog HTTP/1.1\r\nHost: {}\r\nOrigin: http://{}\r\nContent-Type: application/json\r\n{length_headers}\r\n", server.host, server.host);
        assert_eq!(server.raw(request.as_bytes()).0, expected);
        assert_eq!(server.get("/api/catalog").0, 200);
    }
    let head = format!("GET / HTTP/1.1\r\nHost: {}\r\nX-Padding: {}", server.host, "x".repeat(16 * 1024));
    assert_eq!(server.raw(head.as_bytes()).0, 431);
    assert_eq!(server.get("/").0, 200);
}

#[test]
fn planning_preserves_decimal_boundaries_and_rejects_wrong_json_types() {
    let directory = tempdir().unwrap();
    let server = WebServer::start(Path::new(BIN), directory.path(), &[]);
    let request = json!({"catalog": catalog(), "constraints": constraints()});
    let (status, result) = server.post("/api/plan", &request.to_string());
    assert_eq!(status, 200);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["plans"][0]["totals"]["kcal"], catalog()["foods"][0]["kcal"]);
    let mut request = request;
    request["constraints"]["min_kcal"] = json!("450");
    request["constraints"]["max_kcal"] = json!("450");
    let (status, result) = server.post("/api/plan", &request.to_string());
    assert_eq!(status, 200);
    assert_eq!(result["status"], "infeasible");
    for (field, bad_value) in [("min_kcal", json!(450)), ("top", json!(true)), ("max_items", json!(1.0)), ("keep", json!([1]))] {
        let mut request = json!({"catalog": catalog(), "constraints": constraints()});
        request["constraints"][field] = bad_value;
        assert_eq!(server.post("/api/plan", &request.to_string()).0, 400);
    }
    assert_eq!(server.post("/api/plan", &json!({"catalog": null, "constraints": constraints()}).to_string()).0, 400);
    for body in ["{\"schema_version\":1,\"schema_version\":1}", "{\"x\":NaN}", "{\"x\":Infinity}", "{} trailing"] {
        assert_eq!(server.post("/api/catalog", body).0, 400);
    }
    let mut secret = constraints();
    secret["keep"] = json!(["private-request-marker"]);
    let (status, error) = server.post("/api/plan", &json!({"catalog": catalog(), "constraints": secret}).to_string());
    assert_eq!(status, 400);
    assert!(!error.to_string().contains("private-request-marker"));
}

#[test]
fn installed_server_loads_relative_catalog_from_caller_directory_without_demo_fallback() {
    let directory = tempdir().unwrap();
    let installed = directory.path().join("installed");
    let output = Command::new(BIN).args(["install", "--to", installed.to_str().unwrap()])
        .current_dir(directory.path()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let binary = installed.join("scripts").join(format!("maimai{}", std::env::consts::EXE_SUFFIX));
    fs::write(directory.path().join("catalog.json"), catalog().to_string()).unwrap();
    let server = WebServer::start(&binary, directory.path(), &["--catalog", "catalog.json"]);
    let startup: Value = serde_json::from_slice(&server.get("/api/catalog").2).unwrap();
    assert_eq!(startup["source"]["kind"], "import");
    fs::write(directory.path().join("catalog.json"), "invalid").unwrap();
    let (status, result) = server.post("/api/plan", &json!({"constraints": constraints()}).to_string());
    assert_eq!(status, 200);
    assert_eq!(result["status"], "ok");
    let (status, result) = server.post("/api/plan", &json!({"catalog": null, "constraints": constraints()}).to_string());
    assert_eq!(status, 200);
    assert_eq!(result["status"], "ok");
    let output = Command::new(&binary).args(["serve", "--port", "0", "--catalog", "catalog.json"])
        .current_dir(directory.path()).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
