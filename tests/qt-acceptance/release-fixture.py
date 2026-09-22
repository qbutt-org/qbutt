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
import shutil
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
    archive.writestr("qbutt.exe", bytes(range(256)) * 65536)
    archive.writestr("qbutt-net.exe", b"generated fixture; never executed")
    archive.writestr("Qt6Core.dll", b"generated fixture; never loaded")
payload = bundle.getvalue()
installer = Path(os.environ["QBUTT_UPDATE_FIXTURE_SETUP"]).read_bytes() if os.environ.get("QBUTT_UPDATE_FIXTURE_SETUP") else b"MZ" + payload
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
    mode = scenario.removeprefix("installed-")
    def asset_for(suffix, data):
        name = f"qbutt-{version}-windows-x64{suffix}"
        return {"name": name, "size": len(data), "digest": "sha256:" + sha(data),
                "browser_download_url": base + f"download/v{version}/{name}", "state": "uploaded"}
    asset = asset_for("-setup.exe", installer) if scenario.startswith("installed-") else asset_for(".zip", payload)
    if mode == "identity":
        asset["browser_download_url"] = "https://github.com/other/repo/bundle.zip"
    elif mode == "missing-digest":
        del asset["digest"]
    elif mode == "invalid-digest":
        asset["digest"] = "sha256:invalid"
    elif mode == "oversize-asset":
        asset["size"] = 512 * 1024 * 1024 + 1
    elif mode == "short-asset":
        asset["size"] = len(payload) - 1
    assets = [asset, dict(asset)] if mode == "duplicate" else [asset]
    if scenario.startswith("installed-"):
        assets.insert(0, asset_for(".zip", payload))
        if mode == "missing-installer":
            assets = assets[:1]
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
        case = scenario
        mode = scenario.removeprefix("installed-")
        self.request.settimeout(10)
        try:
            connect = headers(self.request)
            assert connect in ("CONNECT api.github.com:443 HTTP/1.1", "CONNECT github.com:443 HTTP/1.1"), connect
            self.request.sendall(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            with context.wrap_socket(self.request, server_side=True) as connection:
                request = headers(connection)
                requests.append({"scenario": case, "request": request})
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
                elif path.endswith((".zip", ".exe", "fixture-archive")):
                    data = installer if path.endswith(".exe") else payload
                    body = data if mode != "corrupt" else b"!" + data[1:]
                    if mode == "archive-redirect" and path.endswith(".zip"):
                        status, body = "302 Found", b"<html>Redirecting to release storage.</html>"
                        extra = "Location: https://github.com/fixture-archive\r\n"
                else:
                    raise ValueError("unexpected fixture path")
                connection.sendall(f"HTTP/1.1 {status}\r\nContent-Length: {len(body)}\r\n{extra}Connection: close\r\n\r\n".encode())
                if mode == "interrupted" and path.endswith((".zip", ".exe")):
                    connection.sendall(body[:16384])
                    return
                for start in range(0, len(body), 16384):
                    connection.sendall(body[start:start + 16384])
                    if mode == "cancel" and path.endswith((".zip", ".exe")):
                        time.sleep(0.02)
        except (OSError, EOFError):
            pass  # Cancellation, an untrusted certificate, or early close is expected.


class Server(socketserver.ThreadingTCPServer):
    daemon_threads = True


evidence = []
cache = None
try:
    with Server(("127.0.0.1", 0), Handler) as server:
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        env = {**os.environ, "QT_QPA_PLATFORM": "offscreen", "QBUTT_UPDATE_FIXTURE_CA": str(cert),
               "QBUTT_UPDATE_FIXTURE_PORT": str(server.server_address[1]),
               "QBUTT_UPDATE_FIXTURE_CACHE_NAME": root.name}
        try:
            if os.environ.get("QBUTT_UPDATE_APPLICATION_ARGS"):
                scenario = "installed-application"
                with (root / "application.stdout.log").open("w") as stdout, (root / "application.stderr.log").open("w") as stderr:
                    result = subprocess.run([sys.argv[1], *json.loads(os.environ["QBUTT_UPDATE_APPLICATION_ARGS"])],
                                            env=env, stdout=stdout, stderr=stderr, timeout=160)
                if result.returncode:
                    raise RuntimeError(f"application: exit {result.returncode}; inspect {root}")
                evidence.append({"scenario": scenario, "exitCode": result.returncode})
                cases = []
            else:
                cases = [("current", 3), ("versions", 2), ("success", 5), ("cancel", 6),
                                    ("corrupt", 7), ("interrupted", 7), ("identity", 7),
                                    ("redirect", 7), ("network", 7), ("untrusted", 7),
                                    ("missing-digest", 7), ("invalid-digest", 7), ("duplicate", 7),
                                    ("oversize-asset", 7), ("short-asset", 7), ("archive-redirect", 5)]
                if len(sys.argv) > 3:
                    cases += [("installed-success", 5), ("installed-cache-hit", 5), ("installed-cache-corrupt", 5),
                              ("installed-cancel", 6), ("installed-corrupt", 7), ("installed-missing-installer", 7),
                              ("installed-duplicate-check", 5), ("installed-tampered", 7)]
            for scenario, state in cases:
                installed = scenario.startswith("installed-")
                if cache and scenario == "installed-cache-corrupt":
                    cached = cache / "updates" / f"qbutt-{future_version}-windows-x64-setup.exe"
                    with cached.open("r+b") as stream:
                        stream.write(b"!")
                target = root / f"{scenario}.zip"
                target.write_bytes(b"existing destination must survive failed downloads")
                result = subprocess.run([sys.argv[3] if installed else sys.argv[1], scenario, str(target)], env=env,
                                        capture_output=True, text=True, timeout=75)
                if result.returncode:
                    raise RuntimeError(f"{scenario}: driver exit {result.returncode}: {result.stdout} {result.stderr}")
                item = json.loads(result.stdout.strip())
                assert item["state"] == state and item["controls"], (scenario, item)
                assert item["installed"] == installed, item
                if installed:
                    cache = Path(item["cacheRoot"]).resolve()
                    assert cache.name == root.name, (cache, root.name)
                if scenario == "corrupt":
                    assert "incomplete or damaged" in item["message"], item
                if scenario == "short-asset":
                    assert "exceeded its expected size" in item["message"], item
                if scenario == "versions":
                    assert item["fileName"] == f"qbutt-{version_10}-windows-x64.zip", item
                archive_requests = [r for r in requests if r["scenario"] == scenario and ".zip " in r["request"]]
                if scenario in ("identity", "missing-digest", "invalid-digest", "duplicate", "oversize-asset"):
                    assert not archive_requests, requests
                if scenario == "success":
                    assert len(archive_requests) == 1, requests
                    assert len([r for r in requests if r["scenario"] == scenario]) == 2, requests
                if installed:
                    installer_requests = [r for r in requests if r["scenario"] == scenario and ".exe " in r["request"]]
                    assert not archive_requests, (scenario, requests)
                    assert len(installer_requests) == (0 if scenario in ("installed-cache-hit", "installed-missing-installer") else 1), (scenario, requests)
                    if state == 5:
                        assert Path(item["savedPath"]).read_bytes() == installer, item
                    if scenario == "installed-cache-hit":
                        assert item["receivedBytes"] == 0, item
                    if scenario == "installed-duplicate-check":
                        assert item["duplicateCheck"], item
                        assert len([r for r in requests if r["scenario"] == scenario]) == 2, requests
                expected = payload if scenario in ("success", "archive-redirect") else b"existing destination must survive failed downloads"
                assert target.read_bytes() == expected, scenario
                assert not [path for path in root.glob(f"{scenario}.zip.*") if path.suffix != ".png"], "partial file remained"
                item.update(scenario=scenario, destinationSha256=sha(target.read_bytes()))
                evidence.append(item)
                target.unlink()
                if cache and scenario not in ("installed-success", "installed-cache-hit"):
                    if cache.exists():
                        shutil.rmtree(cache)
                    cache = None
        finally:
            server.shutdown()
            server_thread.join(timeout=10)
    (root / "evidence.json").write_text(json.dumps({"cases": evidence, "requests": requests}, indent=2))
    print(json.dumps({"passed": len(evidence), "evidence": str(root / "evidence.json")}))
finally:
    if cache and cache.name == root.name and cache.exists():
        shutil.rmtree(cache)
    cert.unlink(missing_ok=True)
    key.unlink(missing_ok=True)
    for partial in root.glob("*.zip*"):
        if partial.suffix != ".png":
            partial.unlink()
