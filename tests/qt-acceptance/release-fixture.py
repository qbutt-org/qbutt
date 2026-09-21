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
signing_key, sign_input = root / "fixture-signing.der", root / "sign-input.txt"


def sums_for(version, data=payload):
    return f"{sha(data)}  qbutt-{version}-windows-x64.zip\n".encode()


def fixture_signature(seed, message):
    # RFC 8410 PKCS#8 encoding; this published RFC 8032 seed is never a release key.
    signing_key.write_bytes(bytes.fromhex("302e020100300506032b657004220420" + seed))
    sign_input.write_bytes(message)
    result = subprocess.run([sys.argv[2], "pkeyutl", "-sign", "-rawin", "-keyform", "DER",
                             "-inkey", str(signing_key), "-in", str(sign_input)],
                            check=True, capture_output=True, timeout=10)
    assert len(result.stdout) == 64
    return result.stdout


fixture_seed = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
signature = fixture_signature(fixture_seed, sums_for(future_version))
wrong_signature = fixture_signature("00" * 32, sums_for(future_version))
replayed_signature = fixture_signature(fixture_seed, sums_for(current_version))
signing_key.unlink()
sign_input.unlink()


def release(version):
    name = f"qbutt-{version}-windows-x64.zip"
    data = b"!" + payload[1:] if scenario == "tampered-metadata" else payload
    sums = sums_for(version, data)
    assets = [{"name": name, "size": len(data), "digest": "sha256:" + sha(data),
               "browser_download_url": base + f"download/v{version}/{name}", "state": "uploaded"},
              {"name": "SHA256SUMS.txt", "size": len(sums), "digest": "sha256:" + sha(sums),
               "browser_download_url": base + f"download/v{version}/SHA256SUMS.txt", "state": "uploaded"}]
    if scenario != "unsigned":
        assets.append({"name": "SHA256SUMS.txt.sig", "size": 64,
                       "browser_download_url": base + f"download/v{version}/SHA256SUMS.txt.sig", "state": "uploaded"})
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
                    if mode == "identity":
                        releases[0]["assets"][0]["browser_download_url"] = "https://github.com/other/repo/bundle.zip"
                    body = json.dumps(releases).encode()
                    if mode == "network":
                        status, body = "403 Forbidden", b"rate limited"
                    elif mode == "redirect":
                        status, body, extra = "302 Found", b"", "Location: http://example.invalid/release\r\n"
                elif path.endswith("SHA256SUMS.txt"):
                    body = sums_for(future_version, b"!" + payload[1:] if mode == "tampered-metadata" else payload)
                    if mode == "checksum":
                        body = b"f" + body[1:]
                elif path.endswith("SHA256SUMS.txt.sig") or path.endswith("fixture-signature"):
                    body = signature
                    if mode == "signature-redirect" and path.endswith(".sig"):
                        status, body = "302 Found", b"<html>Redirecting to release storage.</html>" * 8
                        extra = "Location: https://github.com/fixture-signature\r\n"
                    elif mode == "wrong-key":
                        body = wrong_signature
                    elif mode == "replayed-signature":
                        body = replayed_signature
                    elif mode == "signature":
                        body = bytes([signature[0] ^ 1]) + signature[1:]
                    elif mode == "short-signature":
                        body = signature[:-1]
                    elif mode == "oversize-signature":
                        body = signature + b"!"
                elif path.endswith(".zip"):
                    body = payload if mode != "corrupt" else b"!" + payload[1:]
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
                                    ("corrupt", 7), ("checksum", 7), ("interrupted", 7), ("identity", 7),
                                    ("redirect", 7), ("network", 7), ("untrusted", 7), ("unsigned", 7),
                                    ("signature", 7), ("wrong-key", 7), ("replayed-signature", 7),
                                    ("tampered-metadata", 7), ("short-signature", 7), ("oversize-signature", 7),
                                    ("signature-redirect", 5)]:
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
                if scenario in ("signature", "wrong-key", "replayed-signature", "tampered-metadata", "short-signature"):
                    assert "signature is invalid" in item["message"], item
                if scenario in ("unsigned", "signature", "wrong-key", "replayed-signature",
                                "tampered-metadata", "short-signature", "oversize-signature"):
                    assert not any(r["scenario"] == scenario and ".zip " in r["request"] for r in requests), requests
                expected = payload if scenario in ("success", "signature-redirect") else b"existing destination must survive failed downloads"
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
    signing_key.unlink(missing_ok=True)
    sign_input.unlink(missing_ok=True)
    cert.unlink(missing_ok=True)
    key.unlink(missing_ok=True)
    for partial in root.glob("*.zip*"):
        if partial.suffix != ".png":
            partial.unlink()
