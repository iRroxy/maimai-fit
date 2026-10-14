from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import threading
import unittest

from maimai.mcp import McpClient, McpError


TOKEN = "private-test-token-never-display"


def reply(message, status=200, headers=None):
    return status, {"Content-Type": "application/json", **(headers or {})}, json.dumps(message).encode()


def result(message, value, headers=None):
    return reply({"jsonrpc": "2.0", "id": message["id"], "result": value}, headers=headers)


@contextmanager
def server(responder):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.handle_request()

        def do_DELETE(self):
            self.handle_request()

        def handle_request(self):
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            message = json.loads(body) if body else None
            response = responder(self.command, message, self.headers)
            if response is None:
                if message and message.get("method") == "initialize":
                    response = result(message, {"protocolVersion": "2025-06-18", "capabilities": {},
                                                "serverInfo": {"name": "test", "version": "1"}},
                                      {"Mcp-Session-Id": "test-session"})
                elif message and message.get("method") == "notifications/initialized":
                    response = (202, {}, b"")
                else:
                    response = (204, {}, b"")
            status, headers, data = response
            self.send_response(status)
            for key, value in headers.items():
                self.send_header(key, value)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *args):
            pass

    httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield "http://127.0.0.1:%d/mcp" % httpd.server_port
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join()


class McpTests(unittest.TestCase):
    def test_sse_selects_matching_id_not_notification_or_other_response(self):
        def responder(method, message, headers):
            if not message or message["method"] != "initialize":
                return None
            ident = message["id"]
            stream = (": keepalive\r\n\r\n"
                      "event: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\n"
                      'data: {"jsonrpc":"2.0","id":true,"result":{"protocolVersion":"bad"}}\r\n\r\n'
                      'data: {"jsonrpc":"2.0","id":999,"result":{"protocolVersion":"bad"}}\r\n\r\n'
                      'event: message\r\ndata: {"jsonrpc":"2.0",\r\n'
                      'data: "id":%d,"result":{"protocolVersion":"2025-06-18","selected":"correct"}}\r\n\r\n') % ident
            return 200, {"Content-Type": "text/event-stream", "Mcp-Session-Id": "test-session"}, stream.encode()
        with server(responder) as endpoint:
            client = McpClient(TOKEN, endpoint)
            self.assertEqual(client.initialize()["selected"], "correct")
            client.close()

    def test_sse_without_matching_id_fails_even_with_valid_other_response(self):
        def responder(method, message, headers):
            stream = b'data: {"jsonrpc":"2.0","id":999,"result":{"protocolVersion":"2025-03-26"}}\n\n'
            return 200, {"Content-Type": "text/event-stream"}, stream
        with server(responder) as endpoint, self.assertRaises(McpError):
            McpClient(TOKEN, endpoint).initialize()

    def test_json_response_id_requires_same_type_and_value(self):
        for ident in (True, "1", 2):
            def responder(method, message, headers):
                return reply({"jsonrpc": "2.0", "id": ident, "result": {"protocolVersion": "2025-03-26"}})
            with self.subTest(ident=ident), server(responder) as endpoint:
                with self.assertRaises(McpError):
                    McpClient(TOKEN, endpoint).initialize()

    def test_auth_error_never_quotes_remote_body_or_credentials(self):
        for status in (401, 403):
            def responder(method, message, headers):
                return status, {"WWW-Authenticate": "Bearer " + TOKEN}, ("Authorization: Bearer " + TOKEN).encode()
            with self.subTest(status=status), server(responder) as endpoint:
                with self.assertRaises(McpError) as caught:
                    McpClient(TOKEN, endpoint).initialize()
                text = str(caught.exception)
                self.assertIn(str(status), text)
                self.assertNotIn(TOKEN, text)
                self.assertNotIn("Authorization", text)
                self.assertIsNone(caught.exception.__cause__)

    def test_redirect_does_not_contact_target_with_bearer(self):
        received = []
        def target(method, message, headers):
            received.append(dict(headers))
            return None
        with server(target) as destination:
            def redirect(method, message, headers):
                return 307, {"Location": destination}, b""
            with server(redirect) as endpoint:
                with self.assertRaises(McpError):
                    McpClient(TOKEN, endpoint).initialize()
        self.assertEqual(received, [])

    def test_unsupported_protocol_and_bad_session_fail_safely(self):
        for protocol, session in (("2099-01-01", "valid"), ("2025-03-26", "invalid session")):
            def responder(method, message, headers):
                return result(message, {"protocolVersion": protocol}, {"Mcp-Session-Id": session})
            with self.subTest(protocol=protocol, session=session), server(responder) as endpoint:
                with self.assertRaises(McpError):
                    McpClient(TOKEN, endpoint).initialize()

    def test_session_cannot_change_midstream(self):
        def responder(method, message, headers):
            if message and message.get("method") == "tools/list":
                return result(message, {"tools": []}, {"Mcp-Session-Id": "different-session"})
            return None
        with server(responder) as endpoint:
            with self.assertRaises(McpError):
                McpClient(TOKEN, endpoint).nutrition_tool()

    def test_pagination_repeated_cursor_fails(self):
        pages = []
        def responder(method, message, headers):
            if message and message.get("method") == "tools/list":
                pages.append(message.get("params"))
                return result(message, {"tools": [], "nextCursor": "same"})
            return None
        with server(responder) as endpoint:
            with self.assertRaises(McpError):
                McpClient(TOKEN, endpoint).nutrition_tool()
        self.assertEqual(len(pages), 2)

    def test_discovered_required_fields_gate_calls_without_guessing(self):
        calls = []
        def responder(method, message, headers):
            if message and message.get("method") == "tools/list":
                if not message.get("params", {}).get("cursor"):
                    return result(message, {"tools": [], "nextCursor": "page-two"})
                return result(message, {"tools": [{"name": "list-nutrition-foods", "description": "untrusted text",
                    "inputSchema": {"type": "object", "properties": {"region": {"type": "string"}}, "required": ["region"]}}]})
            if message and message.get("method") == "tools/call":
                calls.append(message["params"])
                return result(message, {"content": [{"type": "text", "text": "actual result"}]})
            return None
        with server(responder) as endpoint:
            client = McpClient(TOKEN, endpoint)
            for arguments in ({}, {"region": None}):
                with self.assertRaises(McpError):
                    client.call_nutrition(arguments)
            self.assertEqual(calls, [])
            client.close()

    def test_structured_nutrition_decimals_keep_remote_precision(self):
        def responder(method, message, headers):
            if message and message.get("method") == "tools/list":
                return result(message, {"tools": [{"name": "list-nutrition-foods", "inputSchema": {"type": "object"}}]})
            if message and message.get("method") == "tools/call":
                body = ('{"jsonrpc":"2.0","id":%d,"result":{"structuredContent":{"foods":'
                        '[{"name":"A","kcal":500.00000000000000000000000000001,"protein_g":30}]}}}') % message["id"]
                return 200, {"Content-Type": "application/json"}, body.encode()
            return None
        with server(responder) as endpoint:
            client = McpClient(TOKEN, endpoint)
            value = client.call_nutrition({})["structuredContent"]["foods"][0]["kcal"]
            self.assertEqual(str(value), "500.00000000000000000000000000001")
            client.close()

    def test_session_and_negotiated_protocol_survive_until_delete(self):
        deletes = []
        def responder(method, message, headers):
            if message and message.get("method") == "initialize":
                return None
            if headers.get("Mcp-Session-Id") != "test-session" or headers.get("MCP-Protocol-Version") != "2025-06-18":
                return 400, {}, b"bad session/protocol"
            if method == "DELETE":
                deletes.append(True)
            return None
        with server(responder) as endpoint:
            client = McpClient(TOKEN, endpoint)
            client.initialize()
            client.close()
            client.close()
        self.assertEqual(deletes, [True])

    def test_tool_and_jsonrpc_errors_are_sanitized(self):
        for rpc_error in (True, False):
            def responder(method, message, headers):
                if message and message.get("method") == "tools/list":
                    return result(message, {"tools": [{"name": "list-nutrition-foods", "inputSchema": {"type": "object"}}]})
                if message and message.get("method") == "tools/call":
                    if rpc_error:
                        return reply({"jsonrpc": "2.0", "id": message["id"], "error": {"code": -1, "message": TOKEN}})
                    return result(message, {"isError": True, "content": [{"type": "text", "text": TOKEN}]})
                return None
            with self.subTest(rpc_error=rpc_error), server(responder) as endpoint:
                with self.assertRaises(McpError) as caught:
                    McpClient(TOKEN, endpoint).call_nutrition({})
                self.assertNotIn(TOKEN, str(caught.exception))

    def test_close_is_best_effort_and_no_delete_without_session(self):
        deletes = []
        def responder(method, message, headers):
            if method == "DELETE":
                deletes.append(True)
                return 500, {}, TOKEN.encode()
            return None
        with server(responder) as endpoint:
            client = McpClient(TOKEN, endpoint)
            client.close()
            self.assertEqual(deletes, [])
            client.initialize()
            client.close()
        self.assertEqual(deletes, [True])

    def test_remote_plain_http_and_unsafe_token_are_rejected(self):
        for endpoint in ("http://example.com/mcp", "https://user:password@example.com/mcp", "file:///tmp/mcp"):
            with self.subTest(endpoint=endpoint), self.assertRaises(McpError):
                McpClient(TOKEN, endpoint)
        with self.assertRaises(McpError):
            McpClient("secret\r\nInjected: value")


if __name__ == "__main__":
    unittest.main()
