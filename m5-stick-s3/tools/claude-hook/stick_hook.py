#!/usr/bin/env python3
"""Claude Code PermissionRequest hook: approve or deny on the M5StickS3.

Claude Code runs this when a tool needs permission (terminal / IDE sessions).
It sends the request to the stick over Bluetooth (the Claude buddy service),
waits for button A (approve) or B (deny), and prints the decision.

If the stick isn't found, or nobody answers in time, it prints nothing and
Claude Code shows its normal permission prompt instead — it never blocks you.

Install: see tools/claude-hook/README.md (install.sh sets up the Python env).
"""
import asyncio
import json
import os
import sys
import time
import uuid
from pathlib import Path

NUS_SVC = "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
NUS_RX = "6e400002-b5a3-f393-e0a9-e50e24dcca9e"  # Mac -> stick
NUS_TX = "6e400003-b5a3-f393-e0a9-e50e24dcca9e"  # stick -> Mac

FIND_S = float(os.environ.get("STICK_FIND_SECONDS", 4))   # give up fast if the stick isn't around
WAIT_S = float(os.environ.get("STICK_WAIT_SECONDS", 25))  # then fall back to the terminal prompt
RESEND_S = 3
LOG = Path.home() / ".claude-stick" / "hook.log"


def log(msg):
    try:
        LOG.parent.mkdir(parents=True, exist_ok=True)
        with LOG.open("a") as f:
            f.write(time.strftime("%Y-%m-%d %H:%M:%S ") + msg + "\n")
    except OSError:
        pass


def describe(tool, inp):
    if tool == "Bash":
        return inp.get("command", "")
    for key in ("file_path", "notebook_path", "path", "url", "pattern", "query"):
        if key in inp:
            return str(inp[key])
    return json.dumps(inp, separators=(",", ":"))


async def ask_stick(req):
    from bleak import BleakClient, BleakScanner

    dev = await BleakScanner.find_device_by_filter(
        lambda d, ad: NUS_SVC in [u.lower() for u in ad.service_uuids], timeout=FIND_S)
    if dev is None:
        log("stick not found")
        return None

    tool = req.get("tool_name", "?")
    prompt_id = "cli-" + uuid.uuid4().hex[:8]
    prompt = {"src": "cli", "prompt": {
        "id": prompt_id,
        "tool": tool,
        "hint": describe(tool, req.get("tool_input", {}))[:160],
        "from": Path(req.get("cwd", "")).name or "terminal",
    }}
    decision = asyncio.get_running_loop().create_future()
    buf = bytearray()

    def on_tx(_, data):
        buf.extend(data)
        while b"\n" in buf:
            i = buf.index(b"\n")
            line = bytes(buf[:i])
            del buf[: i + 1]
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            if msg.get("cmd") == "permission" and msg.get("id") == prompt_id and not decision.done():
                decision.set_result(msg.get("decision"))

    async with BleakClient(dev, timeout=8) as client:
        async def send(obj):
            data = (json.dumps(obj) + "\n").encode()
            for i in range(0, len(data), 180):
                await client.write_gatt_char(NUS_RX, data[i:i + 180], response=True)

        await client.start_notify(NUS_TX, on_tx)
        log(f"asking: {tool} {prompt['prompt']['hint'][:80]}")
        try:
            deadline = time.monotonic() + WAIT_S
            while time.monotonic() < deadline and not decision.done():
                await send(prompt)
                try:
                    await asyncio.wait_for(asyncio.shield(decision), RESEND_S)
                except asyncio.TimeoutError:
                    pass
            return decision.result() if decision.done() else None
        finally:
            await send({"src": "cli", "prompt": None})  # take it off the screen


def main():
    try:
        req = json.load(sys.stdin)
    except ValueError:
        return
    try:
        answer = asyncio.run(asyncio.wait_for(ask_stick(req), FIND_S + WAIT_S + 10))
    except Exception as e:  # Bluetooth off, permission missing, stick busy...
        log(f"error: {e!r}")
        answer = None

    if answer is None:
        log("no answer, falling back to the terminal prompt")
        return  # no output = Claude Code shows its normal prompt
    allow = answer == "once"
    log("approved on stick" if allow else "denied on stick")
    decision = {"behavior": "allow" if allow else "deny"}
    if not allow:
        decision["message"] = "Denied on the stick"
    print(json.dumps({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}}))


if __name__ == "__main__":
    main()
