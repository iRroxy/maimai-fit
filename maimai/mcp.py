"""Small, read-only MCP Streamable HTTP client for the nutrition tool."""

import ipaddress
from decimal import Decimal
from http.client import HTTPException
import json
import math
import time
from urllib import error, parse, request


class McpError(RuntimeError):
    """A safe transport/protocol error that never quotes remote bodies."""


class _NoRedirect(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


_SUPPORTED = {"2025-03-26", "2025-06-18", "2024-11-05"}
_TOOL = "list-nutrition-foods"
_MAX_BYTES = 2 * 1024 * 1024
_MAX_LINE = 65536
_MAX_EVENTS = 1024


def _loads(text):
    def invalid_constant(value):
        raise ValueError("nonfinite JSON")
    return json.loads(text, parse_float=Decimal, parse_constant=invalid_constant)


class McpClient:
    def __init__(self, token: str, endpoint: str = "https://mcp.mcd.cn", timeout: float = 30):
        if not isinstance(token, str) or not token or any(ord(c) <= 32 or ord(c) == 127 for c in token):
            raise McpError("A nonempty bearer token without whitespace is required.")
        if not isinstance(endpoint, str):
            raise McpError("Endpoint must be an HTTPS URL string.")
        try:
            url = parse.urlsplit(endpoint)
            host = url.hostname
            loopback = host == "localhost" or (host is not None and ipaddress.ip_address(host).is_loopback)
        except ValueError:
            loopback = False
        try:
            url = parse.urlsplit(endpoint)
            valid = (bool(url.hostname) and url.username is None and url.password is None and not url.fragment
                     and not any(ord(char) <= 32 or ord(char) == 127 for char in endpoint))
            valid = valid and (url.scheme == "https" or (url.scheme == "http" and loopback))
            url.port
        except (ValueError, TypeError):
            valid = False
        if not valid:
            raise McpError("Endpoint must be HTTPS (HTTP is allowed only on loopback for protocol tests).")
        if isinstance(timeout, bool) or not isinstance(timeout, (int, float)) or not math.isfinite(timeout) or timeout <= 0:
            raise McpError("Timeout must be a positive finite number.")
        self._token = token
        self._endpoint = endpoint
        self._timeout = timeout
        self._opener = request.build_opener(_NoRedirect())
        self._session = None
        self._protocol = None
        self._initialized = None
        self._tool = None
        self._next_id = 1

    def _safe_names(self, names):
        return json.dumps(names, ensure_ascii=True).replace(self._token, "[redacted]")

    def _headers(self):
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "Authorization": "Bearer " + self._token}
        if self._session is not None:
            headers["Mcp-Session-Id"] = self._session
        if self._protocol is not None:
            headers["MCP-Protocol-Version"] = self._protocol
        return headers

    @staticmethod
    def _matching(message, ident):
        return (isinstance(message, dict) and type(message.get("id")) is type(ident)
                and message.get("id") == ident)

    def _sse(self, response, ident):
        started = time.monotonic()
        size = 0
        events = 0
        data = []
        while True:
            if time.monotonic() - started > self._timeout:
                raise McpError("MCP event stream exceeded its time limit.")
            line = response.readline(_MAX_LINE + 1)
            size += len(line)
            if size > _MAX_BYTES or len(line) > _MAX_LINE:
                raise McpError("MCP event stream exceeded its size limit.")
            if not line or line in (b"\n", b"\r\n", b"\r"):
                if data:
                    events += 1
                    if events > _MAX_EVENTS:
                        raise McpError("MCP event stream exceeded its event limit.")
                    try:
                        message = _loads("\n".join(data))
                    except (ValueError, TypeError):
                        raise McpError("MCP event stream contained invalid JSON.") from None
                    data = []
                    if self._matching(message, ident):
                        return message
                if not line:
                    raise McpError("MCP event stream ended without the matching response ID.")
                continue
            try:
                text = line.decode("utf-8").rstrip("\r\n")
            except UnicodeError:
                raise McpError("MCP event stream was not UTF-8.") from None
            if text.startswith(":"):
                continue
            field, separator, value = text.partition(":")
            if separator and value.startswith(" "):
                value = value[1:]
            if field == "data":
                data.append(value)

    def _request(self, method, params=None, *, notification=False, http_method="POST"):
        ident = None
        payload = None
        if http_method == "POST":
            message = {"jsonrpc": "2.0", "method": method}
            if params is not None:
                message["params"] = params
            if not notification:
                ident = self._next_id
                self._next_id += 1
                message["id"] = ident
            try:
                payload = json.dumps(message, allow_nan=False).encode("utf-8")
            except (ValueError, TypeError, OverflowError):
                raise McpError("MCP arguments must be finite JSON values.") from None
        req = request.Request(self._endpoint, data=payload, headers=self._headers(), method=http_method)
        try:
            with self._opener.open(req, timeout=self._timeout) as response:
                status = response.status
                if http_method == "DELETE":
                    return None
                if notification:
                    if status not in (200, 202, 204):
                        raise McpError("MCP initialized notification was not accepted.")
                    return None
                if status != 200:
                    raise McpError("MCP request did not return a response (HTTP %d)." % status)
                session = response.headers.get("Mcp-Session-Id")
                if session is not None:
                    if not session or len(session) > 1024 or any(not 33 <= ord(c) <= 126 for c in session):
                        raise McpError("MCP server returned an invalid session ID.")
                    if self._session is not None and session != self._session:
                        raise McpError("MCP server unexpectedly changed the session ID.")
                    self._session = session
                content_type = response.headers.get_content_type()
                if content_type == "text/event-stream":
                    message = self._sse(response, ident)
                elif content_type == "application/json":
                    body = response.read(_MAX_BYTES + 1)
                    if len(body) > _MAX_BYTES:
                        raise McpError("MCP response exceeded its size limit.")
                    try:
                        message = _loads(body.decode("utf-8"))
                    except (ValueError, UnicodeError):
                        raise McpError("MCP response was not valid UTF-8 JSON.") from None
                else:
                    raise McpError("MCP response must be JSON or an SSE event stream.")
        except error.HTTPError as exc:
            code = exc.code
            exc.close()
            if code in (301, 302, 303, 307, 308):
                raise McpError("MCP redirect refused; credentials were not forwarded.") from None
            if code in (401, 403):
                raise McpError("MCP authentication was rejected (HTTP %d); check your local token." % code) from None
            raise McpError("MCP HTTP request failed (HTTP %d)." % code) from None
        except (error.URLError, TimeoutError, OSError, ValueError, HTTPException):
            raise McpError("MCP network request failed; check connectivity, endpoint and timeout.") from None
        if not self._matching(message, ident) or message.get("jsonrpc") != "2.0":
            raise McpError("MCP response JSON-RPC version or response ID did not match.")
        if "error" in message:
            raise McpError("MCP server returned a JSON-RPC error; no server details are displayed.")
        if not isinstance(message.get("result"), dict):
            raise McpError("MCP response is missing an object result.")
        return message["result"]

    def initialize(self) -> dict:
        if self._initialized is not None:
            return self._initialized
        result = self._request("initialize", {"protocolVersion": "2025-03-26", "capabilities": {},
                                               "clientInfo": {"name": "maimai-fit", "version": "1.0"}})
        protocol = result.get("protocolVersion")
        if not isinstance(protocol, str) or protocol not in _SUPPORTED:
            raise McpError("MCP server negotiated an unsupported protocol version.")
        self._protocol = protocol
        self._request("notifications/initialized", notification=True)
        self._initialized = result
        return result

    def nutrition_tool(self) -> dict:
        self.initialize()
        if self._tool is not None:
            return self._tool
        cursor = None
        seen = set()
        for _ in range(100):
            result = self._request("tools/list", {} if cursor is None else {"cursor": cursor})
            tools = result.get("tools")
            if not isinstance(tools, list) or any(not isinstance(tool, dict) for tool in tools):
                raise McpError("MCP tools/list did not provide a valid tools array.")
            for tool in tools:
                if tool.get("name") == _TOOL:
                    schema = tool.get("inputSchema")
                    if not isinstance(schema, dict) or schema.get("type") != "object":
                        raise McpError("Nutrition tool has no supported object inputSchema; inspect the actual schema.")
                    required = schema.get("required", [])
                    if not isinstance(required, list) or any(not isinstance(name, str) for name in required):
                        raise McpError("Nutrition tool inputSchema has malformed required fields.")
                    self._tool = tool
                    return tool
            cursor = result.get("nextCursor")
            if cursor is None:
                raise McpError("The server did not advertise list-nutrition-foods.")
            if not isinstance(cursor, str) or not cursor or cursor in seen:
                raise McpError("MCP tools/list returned an invalid or repeated pagination cursor.")
            seen.add(cursor)
        raise McpError("MCP tools/list exceeded the pagination limit.")

    def call_nutrition(self, arguments: dict) -> dict:
        if not isinstance(arguments, dict) or any(not isinstance(key, str) for key in arguments):
            raise McpError("Nutrition arguments must be a JSON object with string property names.")
        tool = self.nutrition_tool()
        missing = [name for name in tool["inputSchema"].get("required", [])
                   if name not in arguments or arguments[name] is None]
        if missing:
            raise McpError("Supply explicit nutrition tool arguments for required fields: " + self._safe_names(missing))
        result = self._request("tools/call", {"name": _TOOL, "arguments": arguments})
        if result.get("isError"):
            raise McpError("Nutrition tool reported an error; no server details are displayed.")
        return result

    def close(self) -> None:
        if self._initialized is not None and self._session is not None:
            try:
                self._request(None, http_method="DELETE")
            except McpError:
                pass
        self._session = None
        self._protocol = None
        self._initialized = None
        self._tool = None

    def __enter__(self):
        self.initialize()
        return self

    def __exit__(self, exc_type, exc, traceback):
        self.close()
