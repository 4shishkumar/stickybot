#!/usr/bin/env python3
"""A stand-in for `claude -p ... --output-format stream-json`.

One-shot (`-p "<text>"`): FAKE_CLAUDE_MODE picks the behaviour.
Session (`-p --input-format stream-json`): reads user messages / control requests on stdin
(the shapes brain.py sends) and answers each message by a keyword in its text:
  "chatter"  a preamble + a tool call, then the answer      "slow"   waits to be interrupted
  "die"      crashes mid-request                            "long"   a long reply, word by word
  anything else: a short streamed answer
FAKE_IGNORE_INTERRUPT=1 makes it ignore interrupts (the agent then kills it).

Every start appends its argv to FAKE_CLAUDE_LOG; every user message goes to FAKE_CLAUDE_LOG.msgs."""
import json
import os
import sys
import threading
import time

mode = os.environ.get("FAKE_CLAUDE_MODE", "answer")
LOG = os.environ["FAKE_CLAUDE_LOG"]
with open(LOG, "a") as f:
    f.write(json.dumps(sys.argv[1:]) + "\n")
out_lock = threading.Lock()


def say(obj):
    with out_lock:
        print(json.dumps(obj), flush=True)


def stream(kind, **kw):
    say({"type": "stream_event", "event": {"type": kind, **kw}, "parent_tool_use_id": None})


def text_message(text, pause=0.0):
    """A streamed assistant message with one text block (as --include-partial-messages emits it)."""
    stream("message_start", message={"role": "assistant"})
    stream("content_block_start", index=0, content_block={"type": "text", "text": ""})
    for i in range(0, len(text), 6):
        stream("content_block_delta", index=0, delta={"type": "text_delta", "text": text[i:i + 6]})
        if pause:
            time.sleep(pause)
    say({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}})
    stream("content_block_stop", index=0)
    stream("message_delta", delta={"stop_reason": "end_turn"})
    stream("message_stop")


def tool_message(preamble, name, inp):
    stream("message_start", message={"role": "assistant"})
    stream("content_block_start", index=0, content_block={"type": "text", "text": ""})
    stream("content_block_delta", index=0, delta={"type": "text_delta", "text": preamble})
    say({"type": "assistant", "message": {"content": [{"type": "text", "text": preamble}]}})
    stream("content_block_stop", index=0)
    stream("content_block_start", index=1, content_block={"type": "tool_use", "name": name, "input": {}})
    say({"type": "assistant", "message": {"content": [{"type": "tool_use", "name": name, "input": inp}]}})
    stream("content_block_stop", index=1)
    stream("message_delta", delta={"stop_reason": "tool_use"})
    stream("message_stop")


def result(sid, text, is_error=False, subtype="success"):
    say({"type": "result", "subtype": subtype, "session_id": sid, "is_error": is_error, "result": text})


if "--input-format" not in sys.argv:  # one-shot
    if mode == "answer":
        say({"type": "system", "subtype": "init"})
        say({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Bash", "input": {"command": "df -h /"}},
            {"type": "tool_use", "name": "WebSearch", "input": {"query": "weather bengaluru"}}]}})
        text_message("**It's 28°C** in Bengaluru.")
        say({"type": "result", "session_id": "sess-1", "is_error": False,
             "result": "**It's 28°C** in Bengaluru.\n```\nignored\n```"})
    elif mode == "denied":
        say({"type": "result", "session_id": "sess-2", "is_error": False, "result": "",
             "permission_denials": [{"tool_name": "Bash", "tool_input": {"command": "rm -rf build/"}}]})
    elif mode == "error":
        say({"type": "result", "session_id": "sess-3", "is_error": True, "result": "Rate limited"})
    elif mode == "crash":
        print("Error: not logged in", file=sys.stderr)
        sys.exit(1)
    elif mode == "slow":
        time.sleep(30)
    sys.exit(0)

# --- session -----------------------------------------------------------------------
sid = sys.argv[sys.argv.index("--resume") + 1] if "--resume" in sys.argv else f"sess-{os.getpid()}"
interrupted = threading.Event()
say({"type": "system", "subtype": "hook_started"})
if mode == "crash":
    print("Error: not logged in", file=sys.stderr)
    sys.exit(1)


def turn(text):
    interrupted.clear()
    say({"type": "system", "subtype": "init", "session_id": sid})
    if "die" in text:
        stream("message_start", message={"role": "assistant"})
        print("Error: something broke", file=sys.stderr, flush=True)
        os._exit(3)
    if "slow" in text:
        stream("message_start", message={"role": "assistant"})
        stream("content_block_delta", index=0, delta={"type": "text_delta", "text": "One moment"})
        for _ in range(600):
            if interrupted.is_set():
                say({"type": "user", "message": {"role": "user", "content": [
                    {"type": "text", "text": "[Request interrupted by user]"}]}})
                stream("message_stop")
                result(sid, None, True, "error_during_execution")
                return
            time.sleep(0.05)
        result(sid, "finally")
        return
    if "chatter" in text:
        tool_message("Let me check the weather.", "WebSearch", {"query": "weather bengaluru"})
        text_message("It's 28°C in Bengaluru. Sunny all day.")
        result(sid, "It's 28°C in Bengaluru. Sunny all day.")
        return
    if "long" in text:
        reply = " ".join(f"Word{i}." for i in range(60))
        text_message(reply, pause=0.01)
        result(sid, reply)
        return
    reply = f"You said: {text}."
    text_message(reply)
    result(sid, reply)


worker = None
for line in sys.stdin:
    try:
        msg = json.loads(line)
    except ValueError:
        continue
    if msg.get("type") == "control_request":
        if msg["request"].get("subtype") == "interrupt" and not os.environ.get("FAKE_IGNORE_INTERRUPT"):
            say({"type": "control_response", "response": {"subtype": "success", "request_id": msg["request_id"],
                                                          "response": {"still_queued": []}}})
            interrupted.set()
        continue
    if msg.get("type") == "user":
        with open(LOG + ".msgs", "a") as f:
            f.write(json.dumps(msg) + "\n")
        if worker:
            worker.join()
        worker = threading.Thread(target=turn, args=(msg["message"]["content"],))
        worker.start()
if worker:
    worker.join()
