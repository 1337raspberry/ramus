#!/usr/bin/env python3
"""Local genre-tree editor server.

Serves the static frontend and exposes a tiny JSON API for listing,
loading, and saving genre-tree files under `ramus-tauri/data/`.

Usage:
    python3 tools/genre-editor/server.py            # default port 8765
    python3 tools/genre-editor/server.py --port N
"""

from __future__ import annotations

import argparse
import json
import sys
from collections import OrderedDict
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
DATA_DIR = REPO_ROOT / "ramus-tauri" / "data"
WEB_DIR = Path(__file__).resolve().parent


# --- validation -------------------------------------------------------------


class ValidationError(Exception):
    pass


def validate_node(node: object, path: str) -> None:
    if not isinstance(node, dict):
        raise ValidationError(f"{path}: expected object, got {type(node).__name__}")
    name = node.get("name")
    if not isinstance(name, str) or not name.strip():
        raise ValidationError(f"{path}: 'name' must be a non-empty string")
    if "short_summary" in node:
        ss = node["short_summary"]
        if ss is not None and not isinstance(ss, str):
            raise ValidationError(f"{path} ({name}): 'short_summary' must be string or null")
    if "aka" in node:
        aka = node["aka"]
        if not isinstance(aka, list) or not all(isinstance(a, str) and a.strip() for a in aka):
            raise ValidationError(f"{path} ({name}): 'aka' must be a list of non-empty strings")
    if "children" in node:
        children = node["children"]
        if not isinstance(children, list):
            raise ValidationError(f"{path} ({name}): 'children' must be a list")
        for i, c in enumerate(children):
            validate_node(c, f"{path}/{name}[{i}]")


def validate_tree(data: object) -> None:
    if not isinstance(data, dict):
        raise ValidationError("root: expected object")
    genres = data.get("genres")
    if not isinstance(genres, list):
        raise ValidationError("root: 'genres' must be a list")
    for i, g in enumerate(genres):
        validate_node(g, f"genres[{i}]")


# --- canonicalisation -------------------------------------------------------


def canonicalise_node(node: dict) -> OrderedDict:
    """Rebuild a node with stable field order: name, short_summary, aka, children."""
    rebuilt: OrderedDict = OrderedDict()
    rebuilt["name"] = node["name"]
    if "short_summary" in node:
        rebuilt["short_summary"] = node["short_summary"]
    else:
        rebuilt["short_summary"] = None
    aka = node.get("aka")
    if aka:
        rebuilt["aka"] = list(aka)
    rebuilt["children"] = [canonicalise_node(c) for c in (node.get("children") or [])]
    return rebuilt


def canonicalise_tree(data: dict) -> OrderedDict:
    out: OrderedDict = OrderedDict()
    if "updated_at" in data:
        out["updated_at"] = data["updated_at"]
    out["genres"] = [canonicalise_node(g) for g in data.get("genres", [])]
    return out


# --- path safety ------------------------------------------------------------


def resolve_safe(rel: str) -> Path:
    if not rel:
        raise ValueError("path is required")
    p = (DATA_DIR / rel).resolve()
    if DATA_DIR.resolve() not in p.parents and p != DATA_DIR.resolve():
        raise ValueError("path escapes data dir")
    if p.suffix != ".json":
        raise ValueError("only .json files allowed")
    return p


# --- HTTP handler -----------------------------------------------------------


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt: str, *args: object) -> None:  # noqa: D401
        sys.stderr.write("[server] " + fmt % args + "\n")

    def _send_json(self, status: int, body: object) -> None:
        data = json.dumps(body).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(data)

    def _send_static(self, file_path: Path, content_type: str) -> None:
        if not file_path.is_file():
            self.send_error(404)
            return
        body = file_path.read_bytes()
        self.send_response(200)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802
        url = urlparse(self.path)
        path = url.path

        if path == "/" or path == "/index.html":
            self._send_static(WEB_DIR / "index.html", "text/html; charset=utf-8")
            return
        if path == "/app.js":
            self._send_static(WEB_DIR / "app.js", "application/javascript; charset=utf-8")
            return
        if path == "/styles.css":
            self._send_static(WEB_DIR / "styles.css", "text/css; charset=utf-8")
            return

        if path == "/api/files":
            files = []
            for p in sorted(DATA_DIR.glob("*.json")):
                try:
                    files.append({"name": p.name, "size": p.stat().st_size})
                except OSError:
                    pass
            self._send_json(200, {"files": files, "data_dir": str(DATA_DIR)})
            return

        if path == "/api/file":
            qs = parse_qs(url.query)
            rel = (qs.get("path") or [""])[0]
            try:
                p = resolve_safe(rel)
            except ValueError as e:
                self._send_json(400, {"error": str(e)})
                return
            if not p.is_file():
                self._send_json(404, {"error": "not found"})
                return
            try:
                data = json.loads(p.read_text())
            except json.JSONDecodeError as e:
                self._send_json(400, {"error": f"invalid JSON: {e}"})
                return
            self._send_json(200, {"name": p.name, "data": data})
            return

        self.send_error(404)

    def do_POST(self) -> None:  # noqa: N802
        url = urlparse(self.path)
        path = url.path

        length = int(self.headers.get("Content-Length") or "0")
        raw = self.rfile.read(length) if length > 0 else b""

        if path == "/api/save":
            try:
                payload = json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as e:
                self._send_json(400, {"error": f"bad JSON body: {e}"})
                return
            rel = payload.get("path")
            data = payload.get("data")
            try:
                p = resolve_safe(rel or "")
            except ValueError as e:
                self._send_json(400, {"error": str(e)})
                return
            try:
                validate_tree(data)
            except ValidationError as e:
                self._send_json(400, {"error": f"validation: {e}"})
                return
            canon = canonicalise_tree(data)
            text = json.dumps(canon, indent=2, ensure_ascii=False) + "\n"
            try:
                p.write_text(text)
            except OSError as e:
                self._send_json(500, {"error": f"write failed: {e}"})
                return
            self._send_json(200, {"ok": True, "path": str(p), "bytes": len(text)})
            return

        if path == "/api/validate":
            try:
                payload = json.loads(raw.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as e:
                self._send_json(400, {"error": f"bad JSON body: {e}"})
                return
            try:
                validate_tree(payload.get("data"))
            except ValidationError as e:
                self._send_json(200, {"ok": False, "error": str(e)})
                return
            self._send_json(200, {"ok": True})
            return

        self.send_error(404)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    if not DATA_DIR.is_dir():
        print(f"data dir not found: {DATA_DIR}", file=sys.stderr)
        return 1

    server = HTTPServer((args.host, args.port), Handler)
    print(f"genre-editor serving on http://{args.host}:{args.port}")
    print(f"data dir: {DATA_DIR}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        print("\nshutting down")
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
