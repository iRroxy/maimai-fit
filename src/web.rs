use maimai::catalog::{catalog_from_tool_result, parse_strict_json, validate_catalog, CatalogDocument};
use maimai::mcp::{McpClient, McpError};
use maimai::planner::{parse_number, solve, Constraints, PlanningResult};
use serde::Deserialize;
use serde_json::Value;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;

const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 2 * 1024 * 1024;
const CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
const JSON_TYPE: &str = "application/json; charset=utf-8";

#[derive(Deserialize)]
struct PlanInput {
    keep: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
    min_kcal: String,
    max_kcal: String,
    min_protein: String,
    #[serde(default)]
    max_sodium: Option<String>,
    max_items: usize,
    top: usize,
}

fn plan(mut document: Value, startup: Option<&CatalogDocument>) -> Result<PlanningResult, String> {
    let object = document.as_object_mut().ok_or("Request must be a JSON object.")?;
    let imported = object.remove("catalog")
        .filter(|value| !value.is_null())
        .map(|value| validate_catalog(&value).map_err(|_| "目录无效，请检查格式、精确餐品名称及营养数值。".to_string()))
        .transpose()?;
    let input: PlanInput = serde_json::from_value(
        object.remove("constraints").ok_or("constraints is required.")?,
    ).map_err(|_| "Invalid constraints: targets must be strings, keep/exclude string arrays, and max_items/top integers.".to_string())?;
    let catalog = imported.as_ref().or(startup)
        .ok_or("请先获取官方营养目录或导入目录，再计算配餐。")?;
    let constraints = Constraints {
        keep: input.keep,
        exclude: input.exclude,
        min_kcal: parse_number(&input.min_kcal, "最低热量").map_err(|error| error.to_string())?,
        max_kcal: parse_number(&input.max_kcal, "最高热量").map_err(|error| error.to_string())?,
        min_protein: parse_number(&input.min_protein, "最低蛋白质").map_err(|error| error.to_string())?,
        max_sodium: input.max_sodium.as_deref()
            .map(|value| parse_number(value, "最高钠")).transpose()
            .map_err(|error| error.to_string())?,
        max_items: input.max_items,
        top: input.top,
    };
    constraints.validate().map_err(|error| error.to_string())?;
    // Planner errors may contain user-supplied food names; do not reflect them in API errors.
    solve(catalog, &constraints)
        .map_err(|_| "保留餐品不存在或与排除条件冲突，请检查选择。".to_string())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectInput {
    token: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FetchInput {
    token: String,
    #[serde(default = "empty_arguments")]
    arguments: String,
}

fn empty_arguments() -> String {
    "{}".into()
}

type ClientFactory = dyn Fn(&str) -> Result<McpClient, McpError>;

fn official_client(token: &str) -> Result<McpClient, McpError> {
    McpClient::new(token, "https://mcp.mcd.cn", 30.0)
}

fn remote_request(
    path: &str,
    document: Value,
    make_client: &ClientFactory,
) -> Result<Vec<u8>, (u16, String)> {
    let (token, arguments) = if path == "/api/inspect" {
        let input: InspectInput = serde_json::from_value(document)
            .map_err(|_| (400, "Inspect accepts only a token string.".into()))?;
        (input.token, None)
    } else {
        let input: FetchInput = serde_json::from_value(document)
            .map_err(|_| (400, "Fetch accepts a token string and an optional arguments JSON string.".into()))?;
        let arguments = parse_strict_json(&input.arguments)
            .ok().filter(Value::is_object)
            .ok_or_else(|| (400, "arguments must be a JSON object string without duplicate properties or nonfinite numbers.".into()))?;
        (input.token, Some(arguments))
    };
    let token = token.trim();
    if token.is_empty() || token.chars().any(|c| (c as u32) <= 32 || (c as u32) == 127) {
        return Err((400, "请提供非空且不含空白或控制字符的 Bearer Token。".into()));
    }
    let safe_mcp_error = |error: McpError| {
        let status = if error.0.starts_with("Supply explicit nutrition tool arguments for required fields:") {
            400
        } else {
            502
        };
        (status, error.0.replace(token, "[redacted]"))
    };
    let mut client = make_client(token).map_err(safe_mcp_error)?;
    let result = if let Some(arguments) = arguments {
        client.call_nutrition(&arguments).map_err(safe_mcp_error)
            .and_then(|result| {
                catalog_from_tool_result(&result)
                    .map_err(|_| (502, "官方营养返回无法规范化；未加载任何替代数据，请检查实际格式。".into()))
            })
            .and_then(|catalog| serde_json::to_value(catalog)
                .map_err(|_| (502, "Cannot encode official nutrition catalog.".into())))
    } else {
        client.nutrition_tool().map_err(safe_mcp_error).map(|tool| {
            serde_json::json!({
                "name": tool.get("name"),
                "inputSchema": tool.get("inputSchema").unwrap_or(&Value::Null),
                "description": tool.get("description").unwrap_or(&Value::Null),
            })
        })
    };
    // Drop closes even a partially initialized session, then releases the client's credential.
    drop(client);
    let result = result?;
    let body = serde_json::to_vec(&result)
        .map_err(|_| (502, "Cannot encode official response.".into()))?;
    let serialized = std::str::from_utf8(&body).unwrap();
    let encoded_token = serde_json::to_string(token).unwrap();
    if serialized.contains(token) || serialized.contains(&encoded_token[1..encoded_token.len() - 1]) {
        return Err((502, "Official response reflected the credential; response discarded.".into()));
    }
    Ok(body)
}

fn single_header<'a>(headers: &[httparse::Header<'a>], name: &str) -> Option<&'a [u8]> {
    let mut values = headers.iter().filter(|header| header.name.eq_ignore_ascii_case(name));
    let value = values.next()?.value;
    if values.next().is_some() { None } else { Some(value) }
}

fn json_content_type(value: &[u8]) -> bool {
    let Ok(value) = std::str::from_utf8(value) else { return false; };
    let mut parts = value.split(';');
    if !parts.next().unwrap_or("").trim().eq_ignore_ascii_case("application/json") {
        return false;
    }
    if let Some(parameter) = parts.next() {
        let Some((name, charset)) = parameter.trim().split_once('=') else { return false; };
        let charset = charset.trim();
        let charset = charset.strip_prefix('"').and_then(|value| value.strip_suffix('"')).unwrap_or(charset);
        if !name.trim().eq_ignore_ascii_case("charset") || !charset.eq_ignore_ascii_case("utf-8") {
            return false;
        }
    }
    parts.next().is_none()
}

fn content_length(headers: &[httparse::Header<'_>], required: bool) -> Result<usize, (u16, &'static str)> {
    if headers.iter().any(|header| header.name.eq_ignore_ascii_case("Transfer-Encoding")) {
        return Err((400, "Transfer-Encoding is not supported; provide Content-Length."));
    }
    let length = match single_header(headers, "Content-Length") {
        Some(length) => length,
        None if !required && !headers.iter().any(|header| header.name.eq_ignore_ascii_case("Content-Length")) => return Ok(0),
        None => return Err((400, "A single valid Content-Length is required.")),
    };
    if length.is_empty() || !length.iter().all(u8::is_ascii_digit) {
        return Err((400, "A single valid Content-Length is required."));
    }
    let length = std::str::from_utf8(length).unwrap().parse::<usize>()
        .map_err(|_| (413, "Request body exceeds 2 MiB."))?;
    if length > MAX_BODY {
        return Err((413, "Request body exceeds 2 MiB."));
    }
    Ok(length)
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        502 => "Bad Gateway",
        _ => "Internal Server Error",
    };
    write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: {CSP}\r\n\r\n", body.len())?;
    stream.write_all(body)
}

fn json_response<T: serde::Serialize>(stream: &mut TcpStream, status: u16, value: &T) -> io::Result<()> {
    match serde_json::to_vec(value) {
        Ok(body) => respond(stream, status, JSON_TYPE, &body),
        Err(_) => error(stream, 500, "Cannot encode response."),
    }
}

fn error(stream: &mut TcpStream, status: u16, message: &str) -> io::Result<()> {
    json_response(stream, status, &serde_json::json!({"error": message}))
}

fn handle(stream: &mut TcpStream, startup: Option<&CatalogDocument>, catalog_json: &[u8], host: &str, origin: &str, make_client: &ClientFactory) -> io::Result<()> {
    let mut head = [0; MAX_HEAD];
    let mut used: usize = 0;
    let head_end = loop {
        if used == head.len() {
            return error(stream, 431, "Request headers exceed 16 KiB.");
        }
        let count = match stream.read(&mut head[used..]) {
            Ok(0) => return error(stream, 400, "Incomplete request headers."),
            Ok(count) => count,
            Err(_) => return error(stream, 400, "Cannot read complete request headers."),
        };
        let start = used.saturating_sub(3);
        used += count;
        if let Some(position) = head[start..used].windows(4).position(|bytes| bytes == b"\r\n\r\n") {
            break start + position + 4;
        }
    };
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut request = httparse::Request::new(&mut headers);
    if !matches!(request.parse(&head[..head_end]), Ok(httparse::Status::Complete(_))) {
        return error(stream, 400, "Invalid HTTP request.");
    }
    if single_header(request.headers, "Host") != Some(host.as_bytes()) {
        return error(stream, 403, "Host is not allowed.");
    }
    let method = request.method.unwrap_or("");
    if method == "POST" {
        if single_header(request.headers, "Origin") != Some(origin.as_bytes()) {
            return error(stream, 403, "Origin is not allowed.");
        }
        if !single_header(request.headers, "Content-Type").is_some_and(json_content_type) {
            return error(stream, 400, "Content-Type must be application/json with optional UTF-8 charset.");
        }
    }
    let length = match content_length(request.headers, method == "POST") {
        Ok(length) => length,
        Err((status, message)) => return error(stream, status, message),
    };
    let path = request.path.unwrap_or("");
    if method == "GET" {
        let (content_type, body): (&str, &[u8]) = match path {
            "/" => ("text/html; charset=utf-8", include_bytes!("../web/index.html")),
            "/style.css" => ("text/css; charset=utf-8", include_bytes!("../web/style.css")),
            "/loader.js" => ("text/javascript; charset=utf-8", include_bytes!("../web/loader.js")),
            "/pkg/maimai_ui.js" => ("text/javascript; charset=utf-8", include_bytes!("../web/pkg/maimai_ui.js")),
            "/pkg/maimai_ui_bg.wasm" => ("application/wasm", include_bytes!("../web/pkg/maimai_ui_bg.wasm")),
            "/api/catalog" => (JSON_TYPE, catalog_json),
            _ => return error(stream, 404, "Not found."),
        };
        return respond(stream, 200, content_type, body);
    }
    if method != "POST" || !matches!(path, "/api/catalog" | "/api/plan" | "/api/inspect" | "/api/fetch") {
        return error(stream, 404, "Not found.");
    }
    let mut body = vec![0; length];
    let buffered = length.min(used - head_end);
    body[..buffered].copy_from_slice(&head[head_end..head_end + buffered]);
    if stream.read_exact(&mut body[buffered..]).is_err() {
        return error(stream, 400, "Cannot read complete request body.");
    }
    let document = match std::str::from_utf8(&body).ok().and_then(|text| parse_strict_json(text).ok()) {
        Some(document) => document,
        None => return error(stream, 400, "请提供 UTF-8 JSON；不接受重复属性或非有限数值。"),
    };
    drop(body);
    if path == "/api/catalog" {
        match validate_catalog(&document) {
            Ok(catalog) => json_response(stream, 200, &catalog),
            Err(_) => error(stream, 400, "目录无效，请检查格式、精确餐品名称及营养数值。"),
        }
    } else if matches!(path, "/api/inspect" | "/api/fetch") {
        match remote_request(path, document, make_client) {
            Ok(body) => respond(stream, 200, JSON_TYPE, &body),
            Err((status, reason)) => error(stream, status, &reason),
        }
    } else {
        match plan(document, startup) {
            Ok(result) => json_response(stream, 200, &result),
            Err(reason) => error(stream, 400, &reason),
        }
    }
}

pub(super) fn serve(catalog: Option<CatalogDocument>, port: u16) -> Result<(), Box<dyn std::error::Error>> {
    let catalog_json = serde_json::to_vec(&catalog)?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let host = format!("127.0.0.1:{}", listener.local_addr()?.port());
    let origin = format!("http://{host}");
    println!("{origin}/");
    io::stdout().flush()?;
    for stream in listener.incoming() {
        let mut stream = stream?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        // Exactly one request per connection: rejected bodies are never read or drained.
        let _ = handle(&mut stream, catalog.as_ref(), &catalog_json, &host, &origin, &official_client);
    }
    Ok(())
}

#[cfg(test)]
#[path = "web_tests.rs"]
mod tests;
