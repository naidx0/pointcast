#!/usr/bin/env python3
"""Smoke-test one whisper helper archive on this machine.

Unpacks the archive the way Pointcast will (plain files, mode bits kept), starts whisper-server as Pointcast does
(127.0.0.1, a free port, a private inference path, 4 threads), waits for /health, posts recorded speech and a tone with
the options Pointcast sends per request, and checks each reply is a 200 with JSON text (the speech must come back as
words). It then prints the lines where whisper and ggml say which backends they loaded and which device they use.
A second start with --request-path checks that every route, /load included, moves under the private prefix.

Usage: smoke.py ARCHIVE --model ggml-tiny.en.bin --speech jfk.wav [--expect-gpu none|Vulkan|Metal|any]
                [--env NAME=VALUE ...] [--logs DIR]
"""
import argparse
import io
import json
import math
import os
import re
import socket
import struct
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.request
import uuid
import wave

BACKEND_LINE = re.compile(r"load_backend|register_backend|whisper_backend_init|ggml_vulkan|ggml_metal|no GPU|"
                          r"use gpu|gpu_device|devices +=|backends +=|system_info|failed to (load|initialize)", re.I)
# What Pointcast sends with every request.
REQUEST = {"response_format": "json", "temperature": "0", "beam_size": "5", "language": "en"}


def fail(msg):
    print(f"FAIL: {msg}", flush=True)
    sys.exit(1)


def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def tone_wav(seconds=1, hz=440, rate=16000):
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * hz * i / rate)))
                               for i in range(seconds * rate)))
    return buf.getvalue()


def form(fields, wav=None):
    b = uuid.uuid4().hex
    parts = [f'--{b}\r\nContent-Disposition: form-data; name="{k}"\r\n\r\n{v}\r\n'.encode() for k, v in fields.items()]
    if wav is not None:
        parts.append(f'--{b}\r\nContent-Disposition: form-data; name="file"; filename="audio.wav"\r\n'
                     f"Content-Type: audio/wav\r\n\r\n".encode() + wav + b"\r\n")
    parts.append(f"--{b}--\r\n".encode())
    return b"".join(parts), f"multipart/form-data; boundary={b}"


def http(method, url, body=None, ctype=None, timeout=600):
    req = urllib.request.Request(url, data=body, method=method)
    if ctype:
        req.add_header("Content-Type", ctype)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()


class Server:
    def __init__(self, exe, args, env, log_path):
        self.port = free_port()
        self.base = f"http://127.0.0.1:{self.port}"
        self.log_path = log_path
        self.cmd = [exe] + args + ["--host", "127.0.0.1", "--port", str(self.port)]
        print("$ " + " ".join(self.cmd), flush=True)
        self.log = open(log_path, "wb")
        self.proc = subprocess.Popen(self.cmd, cwd=os.path.dirname(exe), env=env, stdin=subprocess.DEVNULL,
                                     stdout=self.log, stderr=subprocess.STDOUT)

    def wait_ready(self, health, timeout=300):
        t0 = time.time()
        while time.time() - t0 < timeout:
            if self.proc.poll() is not None:
                self.stop()
                fail(f"whisper-server exited with {self.proc.returncode} before it answered:\n{self.text()}")
            try:
                status, body = http("GET", self.base + health, timeout=5)
                if status == 200 and json.loads(body).get("status") == "ok":
                    print(f"ready after {time.time() - t0:.1f} s: GET {health} -> 200 {body.decode()}", flush=True)
                    return
            except (OSError, ValueError):
                pass
            time.sleep(0.5)
        self.stop()
        fail(f"no answer on {health} within {timeout} s:\n{self.text()}")

    def stop(self):
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self.log.close()

    def text(self):
        if not self.log.closed:
            self.log.flush()
        with open(self.log_path, "rb") as f:
            return f.read().decode("utf-8", "replace")


def transcribe(srv, path, wav, what):
    body, ctype = form(REQUEST, wav)
    t0 = time.time()
    status, raw = http("POST", srv.base + path, body, ctype)
    took = time.time() - t0
    if status != 200:
        fail(f"POST {path} ({what}) -> {status} {raw[:500]!r}")
    try:
        reply = json.loads(raw)
    except ValueError:
        fail(f"POST {path} ({what}) -> 200 but not JSON: {raw[:500]!r}")
    if not isinstance(reply.get("text"), str):
        fail(f"POST {path} ({what}) -> 200 JSON without a text field: {reply}")
    print(f"POST {path} ({what}) -> 200 in {took:.2f} s: {json.dumps(reply)}", flush=True)
    return reply["text"], took


def unpack(archive, dest):
    with tarfile.open(archive, "r:bz2") as tf:
        members = tf.getmembers()
        for m in members:
            parts = m.name.split("/")
            if not (m.isfile() or m.isdir()) or m.name.startswith("/") or ".." in parts:
                fail(f"{archive}: entry {m.name!r} is not a plain file or folder inside the archive")
        tops = {m.name.split("/")[0] for m in members}
        if len(tops) != 1:
            fail(f"{archive}: expected one top folder, found {sorted(tops)}")
        if hasattr(tarfile, "data_filter"):
            tf.extractall(dest, filter="data")
        else:
            tf.extractall(dest)
    return os.path.join(dest, tops.pop())


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("archive")
    ap.add_argument("--model", required=True)
    ap.add_argument("--speech", required=True, help="16 kHz WAV of speech that says 'country'")
    ap.add_argument("--expect-gpu", default="any", help="none, any, or a backend name such as Vulkan or Metal")
    ap.add_argument("--env", action="append", default=[], help="NAME=VALUE for the server's environment")
    ap.add_argument("--logs", default=None, help="folder to keep the server logs in")
    a = ap.parse_args()

    logs = a.logs or tempfile.mkdtemp(prefix="whisper-helper-logs-")
    os.makedirs(logs, exist_ok=True)
    folder = unpack(a.archive, tempfile.mkdtemp(prefix="whisper-helper-smoke-"))
    exe = os.path.join(folder, "whisper-server.exe" if os.name == "nt" else "whisper-server")
    if not os.path.isfile(exe):
        fail(f"{exe} is not in the archive")
    if os.name != "nt" and not os.access(exe, os.X_OK):
        fail(f"{exe} is not executable after unpacking: the archive lost its mode bits")
    print(f"unpacked {a.archive} to {folder}: {', '.join(sorted(os.listdir(folder)))}", flush=True)

    env = dict(os.environ)
    env.pop("GGML_BACKEND_PATH", None)
    for kv in a.env:
        k, _, v = kv.partition("=")
        env[k] = v
    with open(a.speech, "rb") as f:
        speech = f.read()
    with wave.open(a.speech, "rb") as w:
        speech_s = w.getnframes() / w.getframerate()
    model = os.path.abspath(a.model)

    # 1. The flags Pointcast starts it with.
    srv = Server(exe, ["-m", model, "--inference-path", "/i-test", "-t", "4"], env, os.path.join(logs, "server.log"))
    srv.wait_ready("/health")
    text, took = transcribe(srv, "/i-test", speech, f"{speech_s:.1f} s of speech")
    if "country" not in text.lower():
        srv.stop()
        fail(f"the speech came back as {text!r}, which lacks 'country'")
    print(f"speech: {speech_s / took:.1f}x real time including the HTTP round trip", flush=True)
    transcribe(srv, "/i-test", tone_wav(), "1 s tone")
    status, _ = http("POST", srv.base + "/inference", *form(REQUEST, tone_wav()))
    if status != 404:
        srv.stop()
        fail(f"POST /inference (the default path) -> {status}, expected 404 with --inference-path set")
    print("POST /inference (the default path) -> 404, as it should with --inference-path set", flush=True)
    status, _ = http("POST", srv.base + "/load", *form({"model": model}))
    print(f"note: POST /load at the root -> {status}: without --request-path any local client can make it load a file",
          flush=True)
    srv.stop()
    log = srv.text()

    # 2. With --request-path every route, /load included, sits under the private prefix.
    srv2 = Server(exe, ["-m", model, "--request-path", "/r-test", "--inference-path", "/i-test", "-t", "4"], env,
                  os.path.join(logs, "server-request-path.log"))
    srv2.wait_ready("/r-test/health")
    status, _ = http("POST", srv2.base + "/load", *form({"model": model}))
    if status != 404:
        srv2.stop()
        fail(f"POST /load with --request-path set -> {status}, expected 404")
    print("POST /load with --request-path set -> 404", flush=True)
    transcribe(srv2, "/r-test/i-test", tone_wav(), "1 s tone under the prefix")
    srv2.stop()

    print("\n==== backend lines (first start) ====")
    for line in log.splitlines():
        if BACKEND_LINE.search(line):
            print("  " + line)
    m = re.search(r"whisper_backend_init_gpu: using (\S+) backend", log)
    gpu = m.group(1) if m else ("none" if "whisper_backend_init_gpu: no GPU found" in log else "unreported")
    print(f"GPU picked: {gpu}")
    print("==== full log of the first start ====")
    print(log)
    want = a.expect_gpu
    if want == "none" and gpu != "none":
        fail(f"expected no GPU on this machine, the server used {gpu}")
    if want not in ("none", "any") and not gpu.lower().startswith(want.lower()):
        fail(f"expected the {want} backend, the server used {gpu}")
    print(f"PASS: {os.path.basename(a.archive)} serves on this machine (GPU: {gpu})")


if __name__ == "__main__":
    main()
