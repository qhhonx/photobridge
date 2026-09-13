#!/usr/bin/env python3
"""Opt-in Mac-to-Android LAN smoke test against a running debug receiver.

Only pass synthetic fixtures. Leaves three test assets on the receiver for visual
inspection. Pairing credentials stay in memory and are never printed or saved.
ADB obtains the debug pairing; all media and status requests use Wi-Fi HTTPS.
Reuse --state to verify deduplication after restarting either application.
"""
import argparse
import base64
import ctypes
import json
from pathlib import Path
import sqlite3
import ssl
import subprocess
import time
import urllib.request
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("adb", "serial", "library", "photo", "video", "state"):
        parser.add_argument("--" + name, required=True)
    args = parser.parse_args()
    state = Path(args.state).resolve()
    state.mkdir(parents=True, exist_ok=True, mode=0o700)
    run_file = state / "fixture-id"
    if not run_file.exists():
        run_file.write_text(str(uuid.uuid4()))
    run_id = run_file.read_text().strip()
    raw = subprocess.run(
        [args.adb, "-s", args.serial, "exec-out", "run-as", "app.photobridge",
         "cat", "files/receiver/identity.json"],
        check=True, capture_output=True,
    ).stdout
    pairing = json.loads(raw)["pairing"]
    lib = ctypes.CDLL(str(Path(args.library).resolve()))
    lib.photobridge_call.argtypes = [ctypes.c_char_p]
    lib.photobridge_call.restype = ctypes.c_void_p
    lib.photobridge_free.argtypes = [ctypes.c_void_p]
    lib.photobridge_free.restype = None

    def call(op, **values):
        pointer = lib.photobridge_call(json.dumps(dict(op=op, **values)).encode())
        if not pointer:
            raise RuntimeError("native_response_missing")
        try:
            result = json.loads(ctypes.string_at(pointer))
        finally:
            lib.photobridge_free(pointer)
        if not result["ok"]:
            raise RuntimeError(result["error"])
        return result["value"]

    call("check_pairing", pairing=pairing)
    print("PASS: receiver identity and Wi-Fi TLS pairing", flush=True)
    call("open_sender", root=str(state / "sender"))
    for kind in ("photo", "video", "motion"):
        def resource(path, role, mime):
            file = Path(path).resolve()
            return dict(path=str(file), role=role, filename=file.name, media_type=mime)
        resources = [resource(args.video, "video", "video/mp4")] if kind == "video" else [resource(args.photo, "photo", "image/jpeg")]
        if kind == "motion":
            resources.append(resource(args.video, "paired_video", "video/mp4"))
        queued = call("enqueue", receiver_id=pairing["receiver_id"],
                      source_id=f"device-smoke-{run_id}-{kind}", revision="1",
                      kind=kind, resources=resources)
        job = queued if queued["state"] == "received" else call("run_sender", pairing=pairing)
        assert job and job["state"] == "received", "receipt_missing"
        assert job["confirmed_bytes"] == sum(r["size"] for r in job["asset"]["resources"]), "byte_count"
        replay = call("enqueue", receiver_id=pairing["receiver_id"],
                      source_id=f"device-smoke-{run_id}-{kind}", revision="1",
                      kind=kind, resources=resources)
        assert replay["id"] == job["id"] and replay["state"] == "received", "dedupe"
        print(f"PASS: {kind} receipt, byte count and enqueue replay", flush=True)
    assert call("run_sender", pairing=pairing) is None, "unexpected_pending_job"
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    context.load_verify_locations(cadata=ssl.DER_cert_to_PEM_cert(base64.b64decode(pairing["certificate"])))
    # Do not route private receiver traffic through environment proxy settings.
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=context))
    with sqlite3.connect(state / "sender" / "sender.sqlite3") as db:
        ids = [row[0] for row in db.execute("SELECT asset_id FROM jobs ORDER BY id")]
    deadline = time.monotonic() + 60
    while True:
        statuses = []
        for asset_id in ids:
            request = urllib.request.Request(pairing["endpoint"] + "/v1/assets/" + asset_id,
                                             headers={"Authorization": "Bearer " + pairing["token"]})
            with http.open(request, timeout=10) as response:
                statuses.append(json.load(response))
        if all(s["processing"] == "complete" for s in statuses):
            print("PASS: all three assets published by the Android foreground service", flush=True)
            break
        assert not any(s["processing"] == "failed" for s in statuses), "target_processing_failed"
        assert time.monotonic() < deadline, "publication_timeout"
        time.sleep(2)


if __name__ == "__main__":
    main()
