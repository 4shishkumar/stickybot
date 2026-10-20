"""Flashes the bundled stick firmware over USB with esptool (no PlatformIO).

The bundle is stick_agent/firmware/ (manifest.json + the flash parts), made by
`make -C m5-stick-s3 release`. The parts are written at their own offsets, like
`pio run -t upload`, so the stick's settings storage (NVS at 0x9000-0xDFFF:
Mac pairings, combo, name, Wi-Fi, Mac password, sounds) is never erased.

Steps: (backup) -> reset into the ROM bootloader with the 1200-baud "touch"
on the stick's USB serial port -> wait for the bootloader's port -> write +
verify -> hard reset. esptool runs as a subprocess, so nothing blocks the
agent's event loop; its output is streamed line by line with progress.
"""
import asyncio
import json
import logging
import re
import sys
import time
from pathlib import Path

from . import config

log = logging.getLogger("flasher")

BUNDLE = Path(__file__).parent / "firmware"
ESPRESSIF_VID = 0x303A
NVS = (0x9000, 0xE000)  # the stick's settings: never write here (default_8MB.csv)
SECTOR = 0x1000
PROGRESS = re.compile(r"(\d{1,3}(?:\.\d+)?)\s*%")


class FlashError(Exception):
    pass


def bundled():
    """The bundled firmware's manifest, with "parts" as [(offset, path)], or None."""
    try:
        m = json.loads((BUNDLE / "manifest.json").read_text())
    except (OSError, ValueError):
        return None
    parts = []
    for p in m.get("parts") or []:
        path = BUNDLE / p["file"]
        if not path.exists():
            return None
        parts.append((p["offset"], str(path)))
    if not parts:
        return None
    return {**m, "files": parts}


def check_parts(parts):
    """Refuses any part whose erase range (whole 4 KB sectors) overlaps NVS."""
    for offset, path in parts:
        start = int(offset, 16)
        end = start + Path(path).stat().st_size
        first, last = start // SECTOR * SECTOR, (end + SECTOR - 1) // SECTOR * SECTOR
        if first < NVS[1] and last > NVS[0]:
            raise FlashError(f"{Path(path).name} at {offset} would overwrite the stick's settings (NVS) — refusing")


def version_tuple(v):
    """'1.6.0' -> (1, 6, 0); anything unparsable -> ()."""
    parts = re.findall(r"\d+", v or "")
    return tuple(int(p) for p in parts[:3])


def espressif_ports():
    """[{"port", "serial", "product", "pid"}] for Espressif USB devices only."""
    from serial.tools import list_ports
    out = []
    for p in list_ports.comports():
        if p.vid == ESPRESSIF_VID:
            out.append({"port": p.device, "serial": p.serial_number or "", "product": p.product or p.description or "",
                        "pid": p.pid})
    return out


def in_bootloader(info):
    """The ROM bootloader (manual download mode or after the touch) shows up as
    the chip's own USB-JTAG/serial unit; our firmware's port is named after the stick."""
    return "jtag" in (info.get("product") or "").lower()


def check_port(port):
    for info in espressif_ports():
        if info["port"] == port:
            return info
    raise FlashError(f"{port} isn't an Espressif USB device (not a stick) — refusing")


def touch_1200(port):
    """Opening + closing the stick's port at 1200 baud makes the firmware reboot into the bootloader."""
    import serial
    try:
        s = serial.Serial()
        s.port, s.baudrate = port, 1200
        s.dtr = False
        s.open()
        time.sleep(0.1)
        s.close()
    except OSError as e:
        raise FlashError(f"couldn't open {port} ({e}) — is something else using it?") from e


def wait_for_bootloader(old_port, timeout_s=12.0):
    """After the touch the port disappears and the bootloader's comes back (maybe with a new name)."""
    deadline = time.time() + timeout_s
    time.sleep(0.6)
    while time.time() < deadline:
        ports = espressif_ports()
        boot = [p for p in ports if in_bootloader(p)]
        if boot:
            same = [p for p in boot if p["port"] == old_port]
            return (same or boot)[0]["port"]
        time.sleep(0.3)
    raise FlashError("the stick didn't come back in download mode — hold its side button until the green "
                     "LED flashes (manual download mode) and try again")


async def run_esptool(args, emit, stage):
    """Runs `python -m esptool <args>`, emitting each line and parsed progress."""
    cmd = [sys.executable, "-m", "esptool", *args]
    emit({"line": "$ esptool " + " ".join(args)})
    proc = await asyncio.create_subprocess_exec(*cmd, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT)
    buf = b""
    last = -1
    while True:
        chunk = await proc.stdout.read(256)
        if not chunk:
            break
        buf += chunk
        # esptool redraws progress with \r; treat \r and \n as line ends
        while True:
            m = re.search(rb"[\r\n]", buf)
            if not m:
                break
            line, buf = buf[: m.start()].decode(errors="replace").strip(), buf[m.end():]
            if not line:
                continue
            pm = PROGRESS.search(line)
            if pm:
                pct = min(100, int(float(pm.group(1))))
                if pct != last:
                    last = pct
                    emit({"progress": pct, "stage": stage})
                continue  # don't flood the log with progress redraws
            emit({"line": line})
    if buf.strip():
        emit({"line": buf.decode(errors="replace").strip()})
    code = await proc.wait()
    if code != 0:
        raise FlashError(f"esptool failed (exit {code})")


async def flash(port, backup=False, emit=None, parts=None):
    """Flashes the bundled firmware (or the given [(offset, path)] parts) to the stick on
    `port`, keeping its settings. emit(event dict) gets {"line"} / {"progress","stage"}
    events. Raises FlashError on failure."""
    emit = emit or (lambda e: None)
    fw = bundled()
    parts = parts or (fw and fw["files"])
    if not parts:
        raise FlashError("no bundled firmware — run `make -C m5-stick-s3 release`")
    check_parts(parts)
    info = check_port(port)
    loop = asyncio.get_running_loop()
    flash_cfg = (fw or {}).get("flash", {})

    # 1. into the bootloader
    emit({"progress": 0, "stage": "reset"})
    if in_bootloader(info):
        emit({"line": f"{port} is already in download mode"})
        boot_port = port
    else:
        emit({"line": f"resetting {port} into download mode (1200-baud touch)"})
        await loop.run_in_executor(None, touch_1200, port)
        boot_port = await loop.run_in_executor(None, wait_for_bootloader, port)
        emit({"line": f"bootloader on {boot_port}"})
    emit({"progress": 100, "stage": "reset"})

    base = ["--chip", flash_cfg.get("chip", "esp32s3"), "--port", boot_port, "--baud", "921600"]

    # 2. optional backup of the whole flash
    if backup:
        name = (info.get("serial") or "stick").replace("/", "_")
        out = config.HOME / "backups" / f"{name}-{time.strftime('%Y%m%d-%H%M%S')}.bin"
        out.parent.mkdir(parents=True, exist_ok=True)
        emit({"line": f"backing up the stick's flash to {out}"})
        await run_esptool([*base, "--before", "usb-reset", "--after", "no-reset", "read-flash", "0x0",
                           "0x800000", str(out)], emit, "backup")

    # 3. write + verify (esptool checks the written data's hash), then reboot.
    # "usb-reset" (like PlatformIO) matters: after the 1200-baud touch the chip is held
    # in download mode, and only esptool's USB reset sequence lets the final hard reset
    # start the new firmware (with "no-reset" it comes back in download mode).
    await run_esptool([*base, "--before", "usb-reset", "--after", "hard-reset", "write-flash",
                       "--flash-mode", flash_cfg.get("mode", "dio"), "--flash-freq", flash_cfg.get("freq", "80m"),
                       "--flash-size", flash_cfg.get("size", "8MB"), *[x for part in parts for x in part]],
                      emit, "flash")
    emit({"progress": 100, "stage": "verify"})
    emit({"progress": 100, "stage": "reboot"})
    emit({"line": "done — the stick restarts with the new firmware (its settings and pairings are kept)"})


def cli(port=None, backup=False):
    """`stick-agent flash [--port P] [--backup]`."""
    fw = bundled()
    if not fw:
        print("no bundled firmware — run: make -C m5-stick-s3 release")
        return 1
    if not port:
        ports = espressif_ports()
        if not ports:
            print("no stick on USB — plug it in (or hold its side button for download mode)")
            return 1
        if len(ports) > 1:
            print("several sticks on USB, pick one with --port:", ", ".join(p["port"] for p in ports))
            return 1
        port = ports[0]["port"]
    print(f"flashing firmware {fw['version']} to {port}{' (with backup)' if backup else ''}")

    def show(e):
        if "line" in e:
            print(e["line"])
        elif "progress" in e:
            print(f"\r  {e['stage']}: {e['progress']:3d}%", end="\n" if e["progress"] == 100 else "", flush=True)

    try:
        asyncio.run(flash(port, backup, show))
    except FlashError as e:
        print(f"failed: {e}")
        return 1
    return 0
