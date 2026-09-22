"""Bounded HTTPS release-download acceptance; no system trust or installed files change.

Run firewall preflight for Python and the stable Qt driver before this script.
Arguments: Qt acceptance executable, openssl executable. Qt DLL/plugin paths must
already be available in PATH/QT_PLUGIN_PATH. Only generated fixture bytes are served.
"""

import hashlib
import io
import json
import os
from pathlib import Path
import socketserver
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import zipfile

root = Path(tempfile.mkdtemp(prefix="qbutt-update-"))
cert, key = root / "ca.pem", root / "key.pem"
subprocess.run([sys.argv[2], "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                "-keyout", str(key), "-out", str(cert), "-days", "1", "-subj", "/CN=qbutt fixture",
                "-addext", "subjectAltName=DNS:api.github.com,DNS:github.com"],
               check=True, capture_output=True, timeout=30)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(cert, key)
bundle = io.BytesIO()
with zipfile.ZipFile(bundle, "w") as archive:
    archive.writestr("qbutt.exe", bytes(range(256)) * 4096)
    archive.writestr("qbutt-net.exe", b"generated fixture; never executed")
    archive.writestr("Qt6Core.dll", b"generated fixture; never loaded")
payload = bundle.getvalue()
sha = lambda data: hashlib.sha256(data).hexdigest()
scenario = ""
requests = []
base = "https://github.com/qbutt-org/qbutt/releases/"
current_version = (Path(__file__).resolve().parents[2] / "qbutt-version.txt").read_text().strip()
next_major = int(current_version.split(".")[0]) + 1
future_version = f"{next_major}.0.0-alpha.3" if "-" in current_version else f"{next_major}.0.3"
version_9 = future_version.rsplit(".", 1)[0] + ".9"
version_10 = future_version.rsplit(".", 1)[0] + ".10"
invalid_version = future_version.rsplit(".", 1)[0] + ".01"


def release(version):
    name = f"qbutt-{version}-windows-x64.zip"
    asset = {"name": name, "size": len(payload), "digest": "sha256:" + sha(payload),
             "browser_download_url": base + f"download/v{version}/{name}", "state": "uploaded"}
    if scenario == "identity":
        asset["browser_download_url"] = "https://github.com/other/repo/bundle.zip"
    elif scenario == "missing-digest":
        del asset["digest"]
    elif scenario == "invalid-digest":
        asset["digest"] = "sha256:invalid"
    elif scenario == "oversize-asset":
        asset["size"] = 512 * 1024 * 1024 + 1
    elif scenario == "short-asset":
        asset["size"] = len(payload) - 1
    assets = [asset, dict(asset)] if scenario == "duplicate" else [asset]
    return {"tag_name": "v" + version, "html_url": base + "tag/v" + version,
            "draft": False, "prerelease": "-" in version, "assets": assets}


def headers(connection):
    data = b""
    while not data.endswith(b"\r\n\r\n"):
        if len(data) >= 16384:
            raise ValueError("oversize HTTP header")
        byte = connection.recv(1)
        if not byte:
            raise EOFError()
        data += byte
    return data.split(b"\r\n", 1)[0].decode("ascii")


class Handler(socketserver.BaseRequestHandler):
    def handle(self):
        mode = scenario
        self.request.settimeout(10)
        try:
            connect = headers(self.request)
            assert connect in ("CONNECT api.github.com:443 HTTP/1.1", "CONNECT github.com:443 HTTP/1.1"), connect
            self.request.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            with context.wrap_socket(self.request, server_side=True) as connection:
                request = headers(connection)
                requests.append({"scenario": mode, "request": request})
                path = request.split(" ")[1]
                status, extra = "200 OK", ""
                if path.startswith("/repos/"):
                    versions = [future_version]
                    if mode == "current":
                        versions = [current_version]
                    elif mode == "versions":
                        versions = [future_version, version_10, version_9, invalid_version]
                    releases = [release(version) for version in versions]
                    body = json.dumps(releases).encode()
                    if mode == "network":
                        status, body = "403 Forbidden", b"rate limited"
                    elif mode == "redirect":
                        status, body, extra = "302 Found", b"", "Location: http://example.invalid/release\r\n"
                elif path.endswith(".zip") or path.endswith("fixture-archive"):
                    body = payload if mode != "corrupt" else b"!" + payload[1:]
                    if mode == "archive-redirect" and path.endswith(".zip"):
                        status, body = "302 Found", b"<html>Redirecting to release storage.</html>"
                        extra = "Location: https://github.com/fixture-archive\r\n"
                else:
                    raise ValueError("unexpected fixture path")
                connection.sendall(f"HTTP/1.1 {status}\r\nContent-Length: {len(body)}\r\n{extra}Connection: close\r\n\r\n".encode())
                if mode == "interrupted" and path.endswith(".zip"):
                    connection.sendall(body[:16384])
                    return
                for start in range(0, len(body), 16384):
                    connection.sendall(body[start:start + 16384])
                    if mode == "cancel" and path.endswith(".zip"):
                        time.sleep(0.02)
        except (OSError, EOFError):
            pass  # Cancellation, an untrusted certificate, or early close is expected.


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True


evidence = []
try:
    with Server(("127.0.0.1", 0), Handler) as server:
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        env = {**os.environ, "QT_QPA_PLATFORM": "offscreen", "QBUTT_UPDATE_FIXTURE_CA": str(cert),
               "QBUTT_UPDATE_FIXTURE_PORT": str(server.server_address[1])}
        try:
            for scenario, state in [("current", 3), ("versions", 2), ("success", 5), ("cancel", 6),
                                    ("corrupt", 7), ("interrupted", 7), ("identity", 7),
                                    ("redirect", 7), ("network", 7), ("untrusted", 7),
                                    ("missing-digest", 7), ("invalid-digest", 7), ("duplicate", 7),
                                    ("oversize-asset", 7), ("short-asset", 7), ("archive-redirect", 5)]:
                target = root / f"{scenario}.zip"
                target.write_bytes(b"existing destination must survive failed downloads")
                result = subprocess.run([sys.argv[1], scenario, str(target)], env=env,
                                        capture_output=True, text=True, timeout=75)
                if result.returncode:
                    raise RuntimeError(f"{scenario}: driver exit {result.returncode}: {result.stdout} {result.stderr}")
                item = json.loads(result.stdout.strip())
                assert item["state"] == state and item["controls"], (scenario, item)
                if scenario == "versions":
                    assert item["fileName"] == f"qbutt-{version_10}-windows-x64.zip", item
                archive_requests = [r for r in requests if r["scenario"] == scenario and ".zip " in r["request"]]
                if scenario in ("identity", "missing-digest", "invalid-digest", "duplicate", "oversize-asset"):
                    assert not archive_requests, requests
                if scenario == "success":
                    assert len(archive_requests) == 1, requests
                    assert len([r for r in requests if r["scenario"] == scenario]) == 2, requests
                expected = payload if scenario in ("success", "archive-redirect") else b"existing destination must survive failed downloads"
                assert target.read_bytes() == expected, scenario
                assert not [path for path in root.glob(f"{scenario}.zip.*") if path.suffix != ".png"], "partial file remained"
                item.update(scenario=scenario, destinationSha256=sha(target.read_bytes()))
                evidence.append(item)
                target.unlink()
        finally:
            server.shutdown()
            server_thread.join(timeout=10)
    (root / "evidence.json").write_text(json.dumps({"cases": evidence, "requests": requests}, indent=2))
    print(json.dumps({"passed": len(evidence), "evidence": str(root / "evidence.json")}))
finally:
    cert.unlink(missing_ok=True)
    key.unlink(missing_ok=True)
    for partial in root.glob("*.zip*"):
        if partial.suffix != ".png":
            partial.unlink()
