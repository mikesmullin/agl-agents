#!/usr/bin/env python3
"""A MOCK System One endpoint — not a model.

It speaks the real wire shape (`POST /v1/systemone`, `{state, questions}` →
`{answers}`) so mail1's rule engine can be exercised end to end before a model
is hosted. Its "judgment" is word overlap between each criterion's wording and
the state: deterministic, explainable, and completely unlike a trained decision
model. Never read its numbers as accuracy.

    python3 scripts/mock-systemone.py --port 8741
    SYSTEMONE_URL=http://127.0.0.1:8741   (mail1's default)

Request  {"state": "From: …\\nSubject: …\\nBody: …",
          "questions": {"r0p0": {"type":"choice","instructions":…,"criteria":{k:phrase}},
                        "r0p1": {"type":"noul","instructions":…},
                        "r0p2": {"type":"score","instructions":…,"criteria":[lo,mid,hi]}}}
           (an extra "images" array is accepted and ignored — the stock shim
           ignores it too, so vision plumbing can be exercised end to end)
Reply    {"answers": {"r0p0": {"type":"choice","choice":k,"confidence":p,"probabilities":{…}},
                      "r0p1": {"type":"noul","noul":p},
                      "r0p2": {"type":"score","score":s,"confidence":p,"probabilities":{…}}}}
"""
import argparse
import json
import math
import re
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

WORD = re.compile(r"[a-z0-9]+")
STOP = set("a an the this that these those is are was were of to for and or in on at as it its with from".split())


def words(text: str):
    return [w for w in WORD.findall((text or "").lower()) if w not in STOP and len(w) > 2]


def overlap(state_words, phrase: str) -> float:
    """A crude stand-in for entailment: shared word mass, sqrt-damped."""
    if not state_words:
        return 0.0
    pw = set(words(phrase))
    if not pw:
        return 0.0
    hits = sum(1 for w in state_words if w in pw)
    return math.sqrt(hits) / (1.0 + 0.05 * len(state_words))


def softmax(scores):
    m = max(scores) if scores else 0.0
    exps = [math.exp(s - m + 0.35) for s in scores]
    total = sum(exps) or 1.0
    return [e / total for e in exps]


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
            return self._send({"ok": True, "mock": True, "model": "mock-systemone"})
        self._send({"error": "not found"}, 404)

    def do_POST(self):
        if not self.path.startswith("/v1/systemone"):
            return self._send({"error": "not found"}, 404)
        try:
            req = json.loads(self.rfile.read(int(self.headers.get("Content-Length", 0))) or b"{}")
            state = req.get("state") or ""
            sw = words(state)
            answers = {}
            for name, q in (req.get("questions") or {}).items():
                qtype = q.get("type", "choice")
                if qtype == "choice":
                    criteria = q.get("criteria") or {}
                    keys = list(criteria.keys())
                    probs = softmax([overlap(sw, criteria[k]) for k in keys])
                    best = max(range(len(keys)), key=lambda i: probs[i]) if keys else -1
                    answers[name] = {
                        "type": "choice",
                        "choice": keys[best] if best >= 0 else "",
                        "confidence": round(probs[best], 4) if best >= 0 else 0.0,
                        "probabilities": {k: round(p, 4) for k, p in zip(keys, probs)},
                    }
                elif qtype == "noul":
                    raw = overlap(sw, q.get("instructions", ""))
                    answers[name] = {"type": "noul", "noul": round(min(1.0, raw / 2.0), 4)}
                elif qtype == "score":
                    levels = q.get("criteria") or []
                    n = max(2, len(levels))
                    # spread the mass toward whichever end shares words with the levels
                    scores = [overlap(sw, lv) for lv in levels] or [0.0] * n
                    tot = sum(scores) or 1.0
                    probs = [s / tot for s in scores]
                    mean = sum(i * p for i, p in enumerate(probs))
                    answers[name] = {
                        "type": "score",
                        "score": round(mean, 4),
                        "legend": {str(i): lv for i, lv in enumerate(levels)},
                        "probabilities": {str(i): round(p, 4) for i, p in enumerate(probs)},
                        "confidence": round(max(probs) if probs else 0.0, 4),
                    }
                else:
                    answers[name] = {"type": qtype, "error": "mock does not implement this type"}
            self._send({"answers": answers, "model": "mock-systemone", "stateChars": len(state)})
        except Exception as e:  # a mock must fail loudly, not quietly
            self._send({"error": f"{type(e).__name__}: {e}"}, 500)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=1237)
    args = ap.parse_args()
    print(f"MOCK System One on http://127.0.0.1:{args.port}/v1/systemone — word overlap, not a model",
          flush=True)
    ThreadingHTTPServer(("127.0.0.1", args.port), Handler).serve_forever()


if __name__ == "__main__":
    main()
