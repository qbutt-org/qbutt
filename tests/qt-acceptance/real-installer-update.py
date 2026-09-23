"""Exercise native Qt update shutdown and genuine isolated Inno replacement.

The drivers are qbutt-qt-acceptance builds from the stated source revisions.
They use production Application, MainWindow and ReleaseUpdater, while selecting
the update action programmatically. Each Inno setup and driver copy receives a
fresh test AppId; the production uninstall entry and installation are untouched.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
import winreg

import psutil


PRODUCTION_APP_ID = "64A54F85-79F8-43D3-9B5B-2336052C370E"


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def patch_app_id(path, app_id):
    original = path.read_bytes()
    before = PRODUCTION_APP_ID.encode("utf-16le")
    after = app_id.encode("utf-16le")
    count = original.count(before)
    if count not in (1, 2):
        raise RuntimeError(f"Unexpected AppId occurrence count in {path}: {count}")
    path.write_bytes(original.replace(before, after))
    return count


def bencode(value):
    if isinstance(value, int):
        return b"i" + str(value).encode() + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode() + b":" + value
    return b"d" + b"".join(bencode(key) + bencode(item)
                          for key, item in sorted(value.items())) + b"e"


def make_torrent(path):
    payload = b"qbutt isolated update fixture\n"
    info = {b"length": len(payload), b"name": b"fixture.bin",
            b"piece length": 16384, b"pieces": hashlib.sha1(payload).digest()}
    path.write_bytes(bencode({b"info": info}))


def make_script(source, target, install, app_id):
    script = (source / "dist/windows/qbutt.iss").read_text(encoding="utf-8")
    if script.count(PRODUCTION_APP_ID) != 2:
        raise RuntimeError("The Inno AppId contract changed")
    script = script.replace(PRODUCTION_APP_ID, app_id)
    script = re.sub(r"(?ms)^\[Icons\]\s*.*?(?=^\[Run\])", "[Icons]\n\n", script)
    script = re.sub(r"(?m)^DefaultDirName=.*$", lambda _: "DefaultDirName=" + str(install), script)
    script = script.replace("AppName=qbutt\n", f"AppName=qbutt acceptance {app_id}\n")
    script = script.replace("[Setup]\n", "[Setup]\nSetupLogging=yes\n")
    if PRODUCTION_APP_ID in script or "Name: \"{autoprograms}" in script:
        raise RuntimeError("Test installer still references production identity or shortcuts")
    target.write_text(script, encoding="utf-8")


def compile_setup(iscc, source, script, bundle, output, version):
    command = [str(iscc), "/Qp", f"/DAppVersion={version}", f"/DAppFileVersion={version}",
               "/DRequiredCompilerVersion=6.7.3", f"/DBundleDir={bundle}",
               f"/DProjectDir={source}", f"/O{output}", str(script)]
    result = subprocess.run(command, capture_output=True, text=True, timeout=180)
    if result.returncode:
        raise RuntimeError(f"ISCC failed ({result.returncode}): {result.stdout[-1200:]} {result.stderr[-1200:]}")
    setup = output / f"qbutt-{version}-windows-x64-setup.exe"
    if not setup.is_file():
        raise RuntimeError("ISCC did not produce the expected setup")
    return setup


def processes_at(path):
    expected = os.path.normcase(os.path.normpath(path))
    result = []
    for process in psutil.process_iter(["pid", "exe", "create_time"]):
        try:
            executable = process.info["exe"]
            if executable and os.path.normcase(os.path.normpath(executable)) == expected:
                result.append(process)
        except (psutil.NoSuchProcess, psutil.AccessDenied):
            pass
    return result


def wait_for(predicate, seconds, label):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {label}")


def run_case(args, root, producer_version, incoming_version, driver, producer_bundle,
             incoming_bundle, expected_driver_exit):
    root.mkdir()
    app_id = str(uuid.uuid4()).upper()
    install = root / "install"
    profile = root / "profile"
    payload = root / "payload"
    incoming_payload = root / "incoming-payload"
    setups = root / "setups"
    logs = root / "logs"
    for directory in (payload, incoming_payload, setups, logs, profile):
        directory.mkdir(parents=True)
    for item in producer_bundle.iterdir():
        if item.name != "profile":
            destination = payload / item.name
            if item.is_dir():
                shutil.copytree(item, destination)
            else:
                shutil.copy2(item, destination)
    for item in incoming_bundle.iterdir():
        if item.name != "profile":
            destination = incoming_payload / item.name
            if item.is_dir():
                shutil.copytree(item, destination)
            else:
                shutil.copy2(item, destination)
    shutil.copy2(driver, payload / "qbutt.exe")
    producer_patches = patch_app_id(payload / "qbutt.exe", app_id)
    incoming_patches = patch_app_id(incoming_payload / "qbutt.exe", app_id)
    make_script(args.source, root / "test.iss", install, app_id)
    producer_setup = compile_setup(args.iscc, args.source, root / "test.iss", payload, setups, producer_version)
    incoming_setup = compile_setup(args.iscc, args.source, root / "test.iss", incoming_payload, setups, incoming_version)
    installation = subprocess.run([str(producer_setup), "/VERYSILENT", "/SUPPRESSMSGBOXES",
        "/NORESTART", f"/DIR={install}", f"/LOG={logs / 'initial.log'}"],
        capture_output=True, text=True, timeout=150)
    if installation.returncode:
        raise RuntimeError(f"Initial Inno install exited {installation.returncode}: {installation.stderr[-600:]}")
    installed_exe = install / "qbutt.exe"
    if digest(installed_exe) != digest(payload / "qbutt.exe"):
        raise RuntimeError("Initial Inno installation has unexpected executable bytes")
    key_path = rf"Software\Microsoft\Windows\CurrentVersion\Uninstall\{{{app_id}}}_is1"
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_path, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
        registered_path = winreg.QueryValueEx(key, "Inno Setup: App Path")[0]
    if os.path.normcase(os.path.normpath(registered_path)) != os.path.normcase(os.path.normpath(install)):
        raise RuntimeError("Unique test AppId points to the wrong installation")

    make_torrent(root / "fixture.torrent")
    (root / "destination").mkdir()
    spec = {"mode": "installed-update", "nativeWindow": True, "registryFixture": False,
            "cacheOrganization": f"qbutt-acceptance-{app_id}", "torrentPath": str(root / "fixture.torrent"),
            "destination": str(root / "destination"), "screenshotDirectory": str(logs),
            "evidencePath": str(logs / "driver.json")}
    (root / "spec.json").write_text(json.dumps(spec), encoding="utf-8")
    local_temp = root / "temp"
    local_temp.mkdir()
    env = {**os.environ, "TEMP": str(local_temp), "TMP": str(local_temp),
           "QBUTT_QT_ACCEPTANCE_NATIVE": "1", "QT_QPA_PLATFORM": "windows",
           "QBUTT_QT_ACCEPTANCE_SPEC": str(root / "spec.json"),
           "QBUTT_UPDATE_FIXTURE_SETUP": str(incoming_setup),
           "QBUTT_UPDATE_FIXTURE_RELEASE_VERSION": incoming_version,
           "QBUTT_UPDATE_APPLICATION_ARGS": json.dumps([f"--profile={profile}"])}
    command = [sys.executable, str(args.source / "tests/qt-acceptance/release-fixture.py"),
               str(installed_exe), str(args.openssl)]
    old_pid = None
    with (logs / "fixture.stdout.log").open("w") as stdout, (logs / "fixture.stderr.log").open("w") as stderr:
        fixture = subprocess.Popen(command, env=env, stdout=stdout, stderr=stderr)
        deadline = time.monotonic() + 210
        while fixture.poll() is None and time.monotonic() < deadline:
            if old_pid is None:
                running = processes_at(installed_exe)
                if running:
                    old_pid = min(running, key=lambda process: process.create_time()).pid
            time.sleep(0.2)
        if fixture.poll() is None:
            fixture.kill()
            raise RuntimeError("HTTPS fixture timed out")
    if old_pid is None:
        raise RuntimeError("Updater producer process never started")
    driver_result = json.loads((logs / "driver.json").read_text(encoding="utf-8"))
    if fixture.returncode != expected_driver_exit:
        raise RuntimeError(f"Unexpected producer exit {fixture.returncode}; {driver_result.get('error')}; inspect {logs}")
    if expected_driver_exit:
        if driver_result.get("error") != "Update did not quit the shown application":
            raise RuntimeError(f"Baseline did not reproduce tray-active quit veto: {driver_result.get('error')}")
    elif driver_result.get("status") != "passed":
        raise RuntimeError(f"Fixed producer did not pass: {driver_result}")
    checks = driver_result.get("checks", [])
    if not any(check.get("name") == "installed-update" and check.get("installerAcknowledged")
               and check.get("nativeSystemTray") for check in checks):
        raise RuntimeError("Driver did not prove native tray and Inno readiness")
    wait_for(lambda: installed_exe.is_file() and digest(installed_exe) == digest(incoming_payload / "qbutt.exe"), 150,
             "genuine Inno payload replacement")
    relaunched = wait_for(lambda: [process for process in processes_at(installed_exe)
                                   if process.pid != old_pid], 30, "automatic relaunch")
    new_pid = relaunched[0].pid
    restarted_profile = relaunched[0].environ().get("QBUTT_PROFILE")
    if restarted_profile != str(profile):
        raise RuntimeError(f"Automatic relaunch did not retain the isolated profile: {restarted_profile}")
    if psutil.pid_exists(old_pid):
        raise RuntimeError("Old producer PID survived Inno replacement")
    setup_log = wait_for(lambda: next((path for path in local_temp.glob("Setup Log*.txt")
        if "Installation process succeeded." in path.read_text(errors="replace")
        and "Log closed." in path.read_text(errors="replace")), None), 30,
        "successful Inno update completion")
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_path, 0, winreg.KEY_READ | winreg.KEY_WOW64_64KEY) as key:
        display_version = winreg.QueryValueEx(key, "DisplayVersion")[0]
    if display_version != incoming_version:
        raise RuntimeError(f"Inno registered {display_version}, expected {incoming_version}")
    for process in processes_at(installed_exe):
        process.terminate()  # Only the test AppId's isolated install is owned here.
        process.wait(timeout=15)
    uninstaller = install / "unins000.exe"
    removed = subprocess.run([str(uninstaller), "/VERYSILENT", "/SUPPRESSMSGBOXES",
        "/NORESTART", f"/LOG={logs / 'uninstall.log'}"], capture_output=True, text=True, timeout=90)
    if removed.returncode or processes_at(installed_exe):
        raise RuntimeError("Isolated Inno uninstallation did not release owned processes and files")
    def uninstalled():
        try:
            with winreg.OpenKey(winreg.HKEY_CURRENT_USER, key_path, 0,
                                winreg.KEY_READ | winreg.KEY_WOW64_64KEY):
                return False
        except FileNotFoundError:
            return not installed_exe.exists()
    wait_for(uninstalled, 30, "isolated Inno uninstallation")
    cache_root = Path(os.environ["LOCALAPPDATA"]) / f"qbutt-acceptance-{app_id}"
    if cache_root.exists():
        if cache_root.is_symlink() or cache_root.resolve().parent != Path(os.environ["LOCALAPPDATA"]).resolve():
            raise RuntimeError("Refusing to remove an unexpected fixture cache path")
        shutil.rmtree(cache_root)
    return {"case": "baseline" if expected_driver_exit else "fixed",
            "producerVersion": producer_version, "incomingVersion": incoming_version,
            "appId": app_id, "driverExit": fixture.returncode, "oldPid": old_pid,
            "relaunchPid": new_pid, "restartedProfile": restarted_profile,
            "registeredVersion": display_version,
            "producerPatches": producer_patches,
            "incomingPatches": incoming_patches, "installedExeSha256": digest(incoming_payload / "qbutt.exe"),
            "driverEvidence": str(logs / "driver.json"), "installerLog": str(setup_log),
            "registryRemoved": True, "cacheRemoved": not cache_root.exists()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--old-driver", type=Path, required=True)
    parser.add_argument("--fixed-driver", type=Path, required=True)
    parser.add_argument("--old-bundle", type=Path, required=True)
    parser.add_argument("--fixed-bundle", type=Path, required=True)
    parser.add_argument("--iscc", type=Path, required=True)
    parser.add_argument("--openssl", type=Path, required=True)
    args = parser.parse_args()
    root = Path(tempfile.mkdtemp(prefix="qbutt-real-inno-"))
    results = []
    try:
        results.append(run_case(args, root / "baseline", "1.0.1", "1.1.1", args.old_driver,
                                args.old_bundle, args.fixed_bundle, 1))
        results.append(run_case(args, root / "fixed", "1.1.1", "1.1.2", args.fixed_driver,
                                args.fixed_bundle, args.fixed_bundle, 0))
        evidence_root = root.with_name(root.name + "-evidence")
        evidence_root.mkdir()
        for result in results:
            case = result["case"]
            case_evidence = evidence_root / case
            case_evidence.mkdir()
            driver = Path(result["driverEvidence"])
            setup_log = Path(result["installerLog"])
            shutil.copy2(driver, case_evidence / "driver.json")
            shutil.copy2(setup_log, case_evidence / "inno-update.log")
            screenshot = driver.parent / "update-ready.png"
            if screenshot.is_file():
                shutil.copy2(screenshot, case_evidence / screenshot.name)
            result["driverEvidence"] = str(case_evidence / "driver.json")
            result["installerLog"] = str(case_evidence / "inno-update.log")
        evidence = evidence_root / "evidence.json"
        evidence.write_text(json.dumps({"cases": results}, indent=2), encoding="utf-8")
        if root.is_symlink() or root.resolve().parent != Path(tempfile.gettempdir()).resolve():
            raise RuntimeError("Refusing to remove an unexpected fixture runtime path")
        shutil.rmtree(root)
        print(json.dumps({"passed": True, "evidence": str(evidence)}))
    except Exception:
        print(f"Isolated fixture retained for diagnosis: {root}", file=sys.stderr)
        raise


if __name__ == "__main__":
    main()
