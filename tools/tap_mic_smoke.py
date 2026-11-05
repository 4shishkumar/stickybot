"""Real-Mac smoke test for the wireless mic's Apple audio tap (opt-in; not part of the test run).

  ~/.stick-agent/venv/bin/python tools/tap_mic_smoke.py [rounds]

Uses a TEST name and UID only ("stick-tap-smoke", com.stickkey.tap-smoke) — never the real
"<stick>-mic-bluetooth" device Wispr ranks — and plays nothing audible: the check's tone is
-80 dBFS. It runs the tap check (StickMicDevice.probe), then creates and removes the tapped
device a few times, timing each step, and makes sure nothing is left behind. The first run on a
Mac (from a given app) is what makes macOS ask for "System Audio Recording".
"""
import os
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "mac-agent"))

from stick_agent import stick_mic_device as smd  # noqa: E402

NAME, UID = "stick-tap-smoke", "com.stickkey.tap-smoke"


def leftovers(ca):
    devs = [(d, n) for d, n, u in ca.devices() if u.startswith(UID)]
    taps = [(t, n) for t, _u, n in ca.taps() if n.startswith(UID)]
    return devs, taps


def main(rounds=3):
    print(f"macOS {smd.macos_version()}, taps supported: {smd.macos_supports_tap()}")
    if not smd.macos_supports_tap():
        return 1
    ca = smd.CoreAudioDevices()
    d = smd.StickMicDevice(backend=ca, name=NAME, uid=UID, mode=smd.TAP)
    d.cleanup_stale()
    print(f"process object {ca.process_object(os.getpid())}, default output {ca.default_output_uid()!r}")

    res = d.probe()
    level = "-" if res["level"] is None else f"{res['level']:.2e}"
    print(f"check: {res['state']} — {res['detail']}")
    print(f"  tap + check device created in {res['create_ms']} ms, tone heard after {res['first_ms']} ms, "
          f"level {level} (sent {smd.PROBE_AMP / 2 ** 0.5:.2e} rms), whole check {res['ms']} ms")

    for i in range(rounds):
        t = time.monotonic()
        made = d.ensure_present()
        ms = (time.monotonic() - t) * 1000
        if not made:
            print(f"round {i + 1}: couldn't create it: {d.error}")
            break
        ins = ca._u32s(ca._get(d.dev, "stm#", smd.INPUT))
        print(f"round {i + 1}: tapped device {made} in {ms:.1f} ms (create {d.create_ms} ms, tap {d.tap}, "
              f"{len(ins)} input stream(s))", end="")
        t = time.monotonic()
        d.remove()
        print(f", removed in {(time.monotonic() - t) * 1000:.1f} ms")

    d.cleanup_stale()
    devs, taps = leftovers(ca)
    print(f"left behind: {len(devs)} devices, {len(taps)} taps" + (f" {devs} {taps}" if devs or taps else ""))
    return 0 if res["state"] == "ok" and not devs and not taps else 2


if __name__ == "__main__":
    sys.exit(main(int(sys.argv[1]) if len(sys.argv) > 1 else 3))
