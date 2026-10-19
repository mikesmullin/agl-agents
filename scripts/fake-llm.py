#!/usr/bin/env python3
"""A FAKE text LLM — OpenAI-compatible chat completions, canned replies.

Speaks just enough of the :1234 contract for mail1's phraser path to be
exercised end to end without a GPU: GET /health and POST
/v1/chat/completions {model, messages, max_tokens} -> a fixed SUBJECT:/SUMMARY:
reply. The words are canned, the plumbing is real.

    python3 scripts/fake-llm.py --port 1234
"""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPLY = (
    "SUBJECT: Fake rewording of the email\n"
    "SUMMARY: This summary was written by the fake LLM so the phrase plumbing can be tested."
)


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def _send(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        if self.path.startswith("/health"):
            return self._send({"ok": True, "fake": True})
        if self.path.startswith("/v1/models"):
            return self._send({"object": "list", "data": [{"id": "default"}]})
        self._send({"error": "not found"}, 404)

    def do_POST(self):
        if not self.path.startswith("/v1/chat/completions"):
            return self._send({"error": "not found"}, 404)
        try:
            n = int(self.headers.get("Content-Length") or 0)
            req = json.loads(self.rfile.read(n) or b"{}")
            msgs = req.get("messages") or []
            prompt = msgs[-1].get("content", "") if msgs else ""
        except Exception as e:
            return self._send({"error": f"{type(e).__name__}: {e}"}, 400)
        self._send({
            "id": "fake-1",
            "object": "chat.completion",
            "model": "default",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": REPLY},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": len(prompt.split()), "completion_tokens": 20, "total_tokens": 0},
        })


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=1234)
    args = ap.parse_args()
    print(f"FAKE text LLM on http://127.0.0.1:{args.port}/v1/chat/completions — canned replies, not a model",
          flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
