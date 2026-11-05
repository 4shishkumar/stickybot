#!/usr/bin/env python3
"""Macro dev portal — build, flash (USB or Wi-Fi) and watch logs for the boards
in this repo, from a browser. Like a tiny ESPHome dashboard for our firmware.

Run: tools/devportal/start.sh   then open http://localhost:8650
"""
import json
import os
import queue
import shlex
import socket
import subprocess
import threading
import time
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

import serial
from serial.tools import list_ports
from zeroconf import ServiceBrowser, ServiceListener, Zeroconf

ROOT = Path(__file__).resolve().parents[2]
PIO = os.path.expanduser("~/.platformio/penv/bin/pio")
PORT = int(os.environ.get("PORTAL_PORT", 8650))
ESPRESSIF_VID = 0x303A

# One entry per firmware project in the repo.
PROJECTS = {
    "m5-stick-s3": {"label": "M5StickS3", "board": "m5-sticks3", "env": "sticks3",
                    "ota_env": "sticks3_ota", "host": "stickybot.local"},
    "xiao-esp32-c3": {"label": "XIAO ESP32-C3", "board": "xiao-esp32-c3", "env": "xiao_c3",
                      "ota_env": "xiao_c3_ota", "host": "xiao-key.local"},
}

# --- Network discovery (mDNS records published by the firmware) ---------------

network = {}  # name -> {host, ip, board, fw}


class Listener(ServiceListener):
    def add_service(self, zc, type_, name):
        info = zc.get_service_info(type_, name, 2000)
        if not info:
            return
        props = {k.decode(): (v or b"").decode() for k, v in info.properties.items()}
        if props.get("macro") != "1":
            return
        ips = info.parsed_addresses()
        network[name] = {"host": info.server.rstrip("."), "ip": ips[0] if ips else "",
                         "board": props.get("board", ""), "fw": props.get("fw", ""), "seen": time.time()}

    update_service = add_service

    def remove_service(self, zc, type_, name):
        network.pop(name, None)


zeroconf = Zeroconf()
ServiceBrowser(zeroconf, "_http._tcp.local.", Listener())


def devices():
    usb = [{"port": p.device, "serial": p.serial_number or "", "desc": p.description}
           for p in list_ports.comports() if p.vid == ESPRESSIF_VID]
    net = [dict(v, name=k) for k, v in network.items()]
    return {"usb": usb, "network": net, "projects": PROJECTS}


# --- Jobs: build / flash, output streamed to the browser ------------------------

jobs = {}  # id -> {"lines": [...], "done": bool, "code": int|None}


def run_job(cmd, cwd):
    jid = uuid.uuid4().hex[:8]
    job = {"lines": [f"$ {' '.join(shlex.quote(c) for c in cmd)}"], "done": False, "code": None}
    jobs[jid] = job

    def worker():
        proc = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, bufsize=1)
        for line in proc.stdout:
            job["lines"].append(line.rstrip("\n").replace("\r", ""))
        job["code"] = proc.wait()
        job["lines"].append(f"[exit {job['code']}]")
        job["done"] = True

    threading.Thread(target=worker, daemon=True).start()
    return jid


def start_action(project, action, target):
    p = PROJECTS[project]
    cwd = ROOT / project
    if action == "build":
        cmd = [PIO, "run", "-e", p["env"]]
    elif action == "flash-usb":
        cmd = [PIO, "run", "-e", p["env"], "-t", "upload"] + (["--upload-port", target] if target else [])
    elif action == "flash-ota":
        cmd = [PIO, "run", "-e", p["ota_env"], "-t", "upload", "--upload-port", target or p["host"]]
    else:
        raise ValueError("unknown action")
    return run_job(cmd, cwd)


# --- Logs: USB serial or network (port 23) --------------------------------------

class LogSession:
    """One open log connection; fans lines out to any number of browser streams."""

    def __init__(self, kind, target):
        self.kind, self.target = kind, target
        self.subscribers = set()
        self.alive = True
        self.conn = None
        threading.Thread(target=self.reader, daemon=True).start()

    def publish(self, line):
        for q in list(self.subscribers):
            q.put(line)

    def open(self):
        if self.kind == "usb":
            s = serial.Serial()
            s.port, s.baudrate, s.timeout = self.target, 115200, 0.3
            s.dtr = s.rts = False  # don't reset the board on open (it may still restart)
            s.open()
            return s
        sock = socket.create_connection((self.target, 23), timeout=5)
        sock.settimeout(0.5)
        return sock

    def reader(self):
        buf = b""
        while self.alive:
            try:
                if self.conn is None:
                    self.conn = self.open()
                    self.publish(f"-- connected ({self.kind} {self.target}) --")
                data = self.conn.read(4096) if self.kind == "usb" else self.recv()
                if data is None:
                    raise ConnectionError("closed")
                buf += data
                while b"\n" in buf:
                    line, buf = buf.split(b"\n", 1)
                    self.publish(line.decode(errors="replace").rstrip("\r"))
            except Exception as e:  # board restarted, unplugged, Wi-Fi off...
                if self.conn is not None:
                    self.publish(f"-- disconnected ({e.__class__.__name__}), retrying --")
                try:
                    self.conn and self.conn.close()
                except Exception:
                    pass
                self.conn = None
                time.sleep(2)

    def recv(self):
        try:
            data = self.conn.recv(4096)
            return data if data else None
        except socket.timeout:
            return b""

    def send(self, line):
        data = (line + "\n").encode()
        if self.conn is None:
            raise ConnectionError("not connected")
        self.conn.write(data) if self.kind == "usb" else self.conn.sendall(data)

    def close(self):
        self.alive = False
        try:
            self.conn and self.conn.close()
        except Exception:
            pass


logs = {}  # "kind:target" -> LogSession


def log_session(kind, target):
    key = f"{kind}:{target}"
    if key not in logs or not logs[key].alive:
        logs[key] = LogSession(kind, target)
    return logs[key]


# --- HTTP -------------------------------------------------------------------------

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send_json(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def sse_start(self):
        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.end_headers()

    def sse(self, data):
        self.wfile.write(f"data: {json.dumps(data)}\n\n".encode())
        self.wfile.flush()

    def body(self):
        n = int(self.headers.get("Content-Length", 0))
        return json.loads(self.rfile.read(n) or b"{}")

    def do_GET(self):
        url = urlparse(self.path)
        q = {k: v[0] for k, v in parse_qs(url.query).items()}
        if url.path == "/":
            body = (Path(__file__).parent / "index.html").read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(body)
        elif url.path == "/api/devices":
            self.send_json(devices())
        elif url.path.startswith("/api/jobs/"):
            job = jobs.get(url.path.split("/")[3])
            if not job:
                return self.send_json({"error": "no such job"}, 404)
            self.sse_start()
            i = 0
            try:
                while True:
                    while i < len(job["lines"]):
                        self.sse({"line": job["lines"][i]})
                        i += 1
                    if job["done"]:
                        self.sse({"done": True, "code": job["code"]})
                        return
                    time.sleep(0.2)
            except (BrokenPipeError, ConnectionResetError):
                return
        elif url.path == "/api/logs":
            sess = log_session(q["kind"], q["target"])
            sub = queue.Queue()
            sess.subscribers.add(sub)
            self.sse_start()
            try:
                while True:
                    try:
                        self.sse({"line": sub.get(timeout=15)})
                    except queue.Empty:
                        self.wfile.write(b": keep-alive\n\n")
                        self.wfile.flush()
            except (BrokenPipeError, ConnectionResetError):
                sess.subscribers.discard(sub)
                if not sess.subscribers:  # last viewer left: free the port
                    sess.close()
        else:
            self.send_json({"error": "not found"}, 404)

    def do_POST(self):
        url = urlparse(self.path)
        try:
            b = self.body()
            if url.path == "/api/run":
                # Flashing over USB needs the port: close a USB log on it first.
                if b["action"] == "flash-usb" and b.get("target"):
                    s = logs.pop(f"usb:{b['target']}", None)
                    s and s.close()
                return self.send_json({"job": start_action(b["project"], b["action"], b.get("target", ""))})
            if url.path == "/api/logs/send":
                log_session(b["kind"], b["target"]).send(b["line"])
                return self.send_json({"ok": True})
            if url.path == "/api/logs/close":
                s = logs.pop(f"{b['kind']}:{b['target']}", None)
                s and s.close()
                return self.send_json({"ok": True})
            self.send_json({"error": "not found"}, 404)
        except Exception as e:
            self.send_json({"error": str(e)}, 400)


if __name__ == "__main__":
    print(f"Macro dev portal: http://localhost:{PORT}")
    ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
