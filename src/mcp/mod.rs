use serde_json::Value;
use std::collections::HashSet;
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpError(pub String);

impl fmt::Display for McpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for McpError {}

impl From<&str> for McpError {
    fn from(s: &str) -> Self {
        McpError(s.to_string())
    }
}

impl From<String> for McpError {
    fn from(s: String) -> Self {
        McpError(s)
    }
}

const SUPPORTED_PROTOCOLS: &[&str] = &["2025-03-26", "2025-06-18", "2024-11-05"];
const TOOL_NAME: &str = "list-nutrition-foods";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_LINE: usize = 65536;
const MAX_EVENTS: usize = 1024;

pub struct McpClient {
    token: String,
    endpoint: String,
    timeout: Duration,
    agent: ureq::Agent,
    session: Option<String>,
    protocol: Option<String>,
    initialized: Option<Value>,
    tool: Option<Value>,
    next_id: i64,
}

impl McpClient {
    pub fn new(token: &str, endpoint: &str, timeout_secs: f64) -> Result<Self, McpError> {
        if token.is_empty() || token.chars().any(|c| (c as u32) <= 32 || (c as u32) == 127) {
            return Err(McpError(
                "A nonempty bearer token without whitespace is required.".into(),
            ));
        }

        Self::validate_endpoint(endpoint)?;

        if !timeout_secs.is_finite() || timeout_secs <= 0.0 {
            return Err(McpError("Timeout must be a positive finite number.".into()));
        }

        let timeout = Duration::from_secs_f64(timeout_secs);
        let agent = ureq::builder()
            .redirects(0)
            .timeout(timeout)
            .build();

        Ok(Self {
            token: token.to_string(),
            endpoint: endpoint.to_string(),
            timeout,
            agent,
            session: None,
            protocol: None,
            initialized: None,
            tool: None,
            next_id: 1,
        })
    }

    fn validate_endpoint(endpoint: &str) -> Result<(), McpError> {
        if endpoint.chars().any(|c| (c as u32) <= 32 || (c as u32) == 127) {
            return Err(McpError(
                "Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests)."
                    .into(),
            ));
        }
        let parsed = url::Url::parse(endpoint).map_err(|_| {
            McpError(
                "Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests)."
                    .into(),
            )
        })?;

        if !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(McpError(
                "Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests)."
                    .into(),
            ));
        }

        let host = parsed.host_str().ok_or_else(|| {
            McpError(
                "Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests)."
                    .into(),
            )
        })?;

        let is_loopback = if host == "localhost" {
            true
        } else if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            ip.is_loopback()
        } else {
            false
        };

        if parsed.scheme() == "https" || (parsed.scheme() == "http" && is_loopback) {
            Ok(())
        } else {
            Err(McpError(
                "Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests)."
                    .into(),
            ))
        }
    }

    fn safe_names(&self, names: &[String]) -> String {
        let serialized = serde_json::to_string(names).unwrap_or_default();
        serialized.replace(&self.token, "[redacted]")
    }

    fn matching(message: &Value, ident: i64) -> bool {
        if let Some(id_val) = message.get("id") {
            if let Some(n) = id_val.as_i64() {
                // Must be number, matching value
                return id_val.is_number() && n == ident;
            }
        }
        false
    }

    fn sse(&self, reader: Box<dyn Read + Send>, ident: i64) -> Result<Value, McpError> {
        let started = Instant::now();
        let mut total_size = 0;
        let mut events = 0;
        let mut data_lines = Vec::new();

        let mut buf_reader = BufReader::new(reader);
        let mut line_buf = Vec::new();

        loop {
            if started.elapsed() > self.timeout {
                return Err(McpError("MCP event stream exceeded its time limit.".into()));
            }

            line_buf.clear();
            let n = buf_reader.read_until(b'\n', &mut line_buf).map_err(|_| {
                McpError("MCP event stream read error.".into())
            })?;

            if n > MAX_LINE || total_size + n > MAX_BYTES {
                return Err(McpError("MCP event stream exceeded its size limit.".into()));
            }
            total_size += n;

            if n == 0 || line_buf == b"\n" || line_buf == b"\r\n" || line_buf == b"\r" {
                if !data_lines.is_empty() {
                    events += 1;
                    if events > MAX_EVENTS {
                        return Err(McpError(
                            "MCP event stream exceeded its event limit.".into(),
                        ));
                    }

                    let joined = data_lines.join("\n");
                    data_lines.clear();

                    let message: Value = serde_json::from_str(&joined).map_err(|_| {
                        McpError("MCP event stream contained invalid JSON.".into())
                    })?;

                    if Self::matching(&message, ident) {
                        return Ok(message);
                    }
                }
                if n == 0 {
                    return Err(McpError(
                        "MCP event stream ended without the matching response ID.".into(),
                    ));
                }
                continue;
            }

            let text = std::str::from_utf8(&line_buf)
                .map_err(|_| McpError("MCP event stream was not UTF-8.".into()))?
                .trim_end_matches(|c| c == '\r' || c == '\n');

            if text.starts_with(':') {
                continue;
            }

            if let Some((field, rest)) = text.split_once(':') {
                let val = if rest.starts_with(' ') {
                    &rest[1..]
                } else {
                    rest
                };
                if field == "data" {
                    data_lines.push(val.to_string());
                }
            }
        }
    }

    fn request(
        &mut self,
        method: Option<&str>,
        params: Option<Value>,
        notification: bool,
        http_method: &str,
    ) -> Result<Option<Value>, McpError> {
        let mut ident = None;
        let payload = if http_method == "POST" {
            let mut msg = serde_json::Map::new();
            msg.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
            if let Some(m) = method {
                msg.insert("method".to_string(), Value::String(m.to_string()));
            }
            if let Some(p) = params {
                msg.insert("params".to_string(), p);
            }
            if !notification {
                let id = self.next_id;
                self.next_id += 1;
                ident = Some(id);
                msg.insert("id".to_string(), Value::Number(id.into()));
            }
            Some(serde_json::to_vec(&Value::Object(msg)).map_err(|_| {
                McpError("MCP arguments must be finite JSON values.".into())
            })?)
        } else {
            None
        };

        let mut req = self.agent.request(http_method, &self.endpoint);
        req = req.set("Content-Type", "application/json");
        req = req.set("Accept", "application/json, text/event-stream");
        req = req.set("Authorization", &format!("Bearer {}", self.token));

        if let Some(ref s) = self.session {
            req = req.set("Mcp-Session-Id", s);
        }
        if let Some(ref p) = self.protocol {
            req = req.set("MCP-Protocol-Version", p);
        }

        let resp_result = if let Some(body) = payload {
            req.send_bytes(&body)
        } else {
            req.call()
        };

        let resp = match resp_result {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _resp)) => {
                if [301, 302, 303, 307, 308].contains(&code) {
                    return Err(McpError(
                        "MCP redirect refused; credentials were not forwarded.".into(),
                    ));
                }
                if code == 401 || code == 403 {
                    return Err(McpError(format!(
                        "MCP authentication was rejected (HTTP {code}); check your local token."
                    )));
                }
                return Err(McpError(format!("MCP HTTP request failed (HTTP {code}).")));
            }
            Err(_) => {
                return Err(McpError(
                    "MCP network request failed; check connectivity, endpoint and timeout.".into(),
                ));
            }
        };

        let status = resp.status();
        if http_method == "DELETE" {
            return Ok(None);
        }

        if notification {
            if status != 200 && status != 202 && status != 204 {
                return Err(McpError(
                    "MCP initialized notification was not accepted.".into(),
                ));
            }
            return Ok(None);
        }

        if status != 200 {
            return Err(McpError(format!(
                "MCP request did not return a response (HTTP {status})."
            )));
        }

        if let Some(new_session) = resp.header("Mcp-Session-Id") {
            if new_session.is_empty()
                || new_session.len() > 1024
                || new_session
                    .chars()
                    .any(|c| (c as u32) < 33 || (c as u32) > 126)
            {
                return Err(McpError(
                    "MCP server returned an invalid session ID.".into(),
                ));
            }
            if let Some(ref existing) = self.session {
                if existing != new_session {
                    return Err(McpError(
                        "MCP server unexpectedly changed the session ID.".into(),
                    ));
                }
            }
            self.session = Some(new_session.to_string());
        }

        let content_type = resp
            .header("Content-Type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();

        let message = if content_type == "text/event-stream" {
            let req_id = ident.ok_or_else(|| {
                McpError("Notification cannot expect SSE response matching ID.".into())
            })?;
            self.sse(resp.into_reader(), req_id)?
        } else if content_type == "application/json" {
            let mut reader = resp.into_reader();
            let mut body = Vec::new();
            let mut take = (&mut reader).take((MAX_BYTES + 1) as u64);
            take.read_to_end(&mut body).map_err(|_| {
                McpError("MCP response read error.".into())
            })?;
            if body.len() > MAX_BYTES {
                return Err(McpError("MCP response exceeded its size limit.".into()));
            }
            serde_json::from_slice::<Value>(&body).map_err(|_| {
                McpError("MCP response was not valid UTF-8 JSON.".into())
            })?
        } else {
            return Err(McpError(
                "MCP response must be JSON or an SSE event stream.".into(),
            ));
        };

        if let Some(id) = ident {
            if !Self::matching(&message, id)
                || message.get("jsonrpc") != Some(&Value::String("2.0".to_string()))
            {
                return Err(McpError(
                    "MCP response JSON-RPC version or response ID did not match.".into(),
                ));
            }
        }

        if message.get("error").is_some() {
            return Err(McpError(
                "MCP server returned a JSON-RPC error; no server details are displayed.".into(),
            ));
        }

        let res = message
            .get("result")
            .and_then(|v| v.as_object())
            .ok_or_else(|| McpError("MCP response is missing an object result.".into()))?;

        Ok(Some(Value::Object(res.clone())))
    }

    pub fn initialize(&mut self) -> Result<Value, McpError> {
        if let Some(ref init) = self.initialized {
            return Ok(init.clone());
        }

        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "maimai-fit",
                "version": "1.0"
            }
        });

        let result = self
            .request(Some("initialize"), Some(params), false, "POST")?
            .ok_or_else(|| McpError("initialize returned no result".into()))?;

        let proto = result
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                McpError("MCP server negotiated an unsupported protocol version.".into())
            })?;

        if !SUPPORTED_PROTOCOLS.contains(&proto) {
            return Err(McpError(
                "MCP server negotiated an unsupported protocol version.".into(),
            ));
        }

        self.protocol = Some(proto.to_string());
        self.request(
            Some("notifications/initialized"),
            None,
            true,
            "POST",
        )?;
        self.initialized = Some(result.clone());

        Ok(result)
    }

    pub fn nutrition_tool(&mut self) -> Result<Value, McpError> {
        self.initialize()?;
        if let Some(ref t) = self.tool {
            return Ok(t.clone());
        }

        let mut cursor: Option<String> = None;
        let mut seen = HashSet::new();

        for _ in 0..100 {
            let params = match cursor {
                Some(ref c) => serde_json::json!({ "cursor": c }),
                None => serde_json::json!({}),
            };

            let res = self
                .request(Some("tools/list"), Some(params), false, "POST")?
                .ok_or_else(|| McpError("tools/list returned no result".into()))?;

            let tools = res
                .get("tools")
                .and_then(|v| v.as_array())
                .ok_or_else(|| McpError("MCP tools/list did not provide a valid tools array.".into()))?;

            for tool_val in tools {
                if !tool_val.is_object() {
                    return Err(McpError(
                        "MCP tools/list did not provide a valid tools array.".into(),
                    ));
                }
                if tool_val.get("name") == Some(&Value::String(TOOL_NAME.to_string())) {
                    let schema = tool_val.get("inputSchema");
                    let schema_obj = schema.and_then(|v| v.as_object()).ok_or_else(|| {
                        McpError("Nutrition tool has no supported object inputSchema; inspect the actual schema.".into())
                    })?;

                    if schema_obj.get("type") != Some(&Value::String("object".to_string())) {
                        return Err(McpError("Nutrition tool has no supported object inputSchema; inspect the actual schema.".into()));
                    }

                    if let Some(req_val) = schema_obj.get("required") {
                        let arr = req_val.as_array().ok_or_else(|| {
                            McpError("Nutrition tool inputSchema has malformed required fields.".into())
                        })?;
                        if arr.iter().any(|v| !v.is_string()) {
                            return Err(McpError("Nutrition tool inputSchema has malformed required fields.".into()));
                        }
                    }

                    self.tool = Some(tool_val.clone());
                    return Ok(tool_val.clone());
                }
            }

            match res.get("nextCursor") {
                Some(Value::String(next)) if !next.is_empty() => {
                    if !seen.insert(next.clone()) {
                        return Err(McpError(
                            "MCP tools/list returned an invalid or repeated pagination cursor."
                                .into(),
                        ));
                    }
                    cursor = Some(next.clone());
                }
                Some(_) => {
                    return Err(McpError(
                        "MCP tools/list returned an invalid or repeated pagination cursor.".into(),
                    ));
                }
                None => {
                    return Err(McpError(
                        "The server did not advertise list-nutrition-foods.".into(),
                    ));
                }
            }
        }

        Err(McpError("MCP tools/list exceeded the pagination limit.".into()))
    }

    pub fn call_nutrition(&mut self, arguments: &Value) -> Result<Value, McpError> {
        let arg_obj = arguments.as_object().ok_or_else(|| {
            McpError("Nutrition arguments must be a JSON object with string property names.".into())
        })?;

        let tool = self.nutrition_tool()?;
        let required = tool
            .get("inputSchema")
            .and_then(|s| s.get("required"))
            .and_then(|r| r.as_array());

        let mut missing = Vec::new();
        if let Some(req_fields) = required {
            for f in req_fields {
                if let Some(field_name) = f.as_str() {
                    match arg_obj.get(field_name) {
                        None | Some(Value::Null) => {
                            missing.push(field_name.to_string());
                        }
                        _ => {}
                    }
                }
            }
        }

        if !missing.is_empty() {
            let names = self.safe_names(&missing);
            return Err(McpError(format!(
                "Supply explicit nutrition tool arguments for required fields: {names}"
            )));
        }

        let params = serde_json::json!({
            "name": TOOL_NAME,
            "arguments": arguments,
        });

        let result = self
            .request(Some("tools/call"), Some(params), false, "POST")?
            .ok_or_else(|| McpError("tools/call returned no result".into()))?;

        if result.get("isError") == Some(&Value::Bool(true)) {
            return Err(McpError(
                "Nutrition tool reported an error; no server details are displayed.".into(),
            ));
        }

        Ok(result)
    }

    pub fn close(&mut self) {
        if self.session.is_some() {
            let _ = self.request(None, None, false, "DELETE");
        }
        self.session = None;
        self.protocol = None;
        self.initialized = None;
        self.tool = None;
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.close();
    }
}
