#!/usr/bin/env python3
"""Ollama client used by the shellmind Python companion tools.

A dependency-free (stdlib only) mirror of the Rust client in
`crates/core/src/ai/ollama.rs`. Useful for experimentation, batch
embedding jobs and CI scripts without a Rust toolchain.

Usage:
    from ollama import Ollama
    ollama = Ollama()                     # http://localhost:11434
    print(ollama.tags())
    answer = ollama.chat("qwen2.5-coder:3b", "system prompt", "user prompt")
    vectors = ollama.embed("nomic-embed-text", ["docker ps", "git log"])
"""

from __future__ import annotations

import json
import urllib.error
import urllib.request
from dataclasses import dataclass
from typing import Optional

DEFAULT_HOST = "http://localhost:11434"
DEFAULT_TIMEOUT = 60


@dataclass
class Ollama:
    host: str = DEFAULT_HOST
    timeout: int = DEFAULT_TIMEOUT

    def _post(self, path: str, payload: dict, timeout: Optional[int] = None) -> dict:
        req = urllib.request.Request(
            f"{self.host}{path}",
            data=json.dumps(payload).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
            return json.loads(resp.read().decode())

    def _get(self, path: str, timeout: Optional[int] = None) -> dict:
        with urllib.request.urlopen(f"{self.host}{path}", timeout=timeout or self.timeout) as resp:
            return json.loads(resp.read().decode())

    # -- chat ---------------------------------------------------------------

    def chat(
        self,
        model: str,
        system: str,
        user: str,
        temperature: float = 0.2,
    ) -> str:
        """Run a chat completion; returns the assistant message."""
        body = {
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "stream": False,
            "options": {"temperature": temperature},
        }
        data = self._post("/api/chat", body)
        return data.get("message", {}).get("content", "")

    # -- embeddings ------------------------------------------------------------

    def embed(self, model: str, texts: list[str]) -> list[list[float]]:
        """Embed texts (batched /api/embed, legacy /api/embeddings fallback)."""
        try:
            data = self._post("/api/embed", {"model": model, "input": texts})
            if data.get("embeddings"):
                return data["embeddings"]
        except urllib.error.URLError:
            raise
        # Legacy endpoint: one text per call.
        out: list[list[float]] = []
        for text in texts:
            data = self._post("/api/embeddings", {"model": model, "prompt": text})
            out.append(data["embedding"])
        return out

    # -- models ------------------------------------------------------------------

    def tags(self) -> list[str]:
        data = self._get("/api/tags")
        return [m["name"] for m in data.get("models", [])]

    def pull(self, model: str) -> None:
        self._post("/api/pull", {"name": model, "stream": False}, timeout=600)

    def ping(self, timeout: float = 1.5) -> bool:
        try:
            self._get("/api/tags", timeout=int(timeout))
            return True
        except (urllib.error.URLError, OSError):
            return False


if __name__ == "__main__":
    import sys

    client = Ollama()
    if not client.ping():
        print("ollama is not reachable at", client.host)
        sys.exit(1)
    print("installed models:")
    for name in client.tags():
        print(" -", name)
