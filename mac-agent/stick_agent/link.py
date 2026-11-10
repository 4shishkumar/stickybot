"""Bluetooth link to the stick: its Nordic UART service on the keyboard's
connection (see PROTOCOL.md, "Mac link").

Only talks to the stick picked in the dashboard (config "stick"), because
several sticks can be in range in an office.
"""
import asyncio
import json
import logging
import time

from bleak import BleakClient, BleakScanner

NUS = "6e400001-b5a3-f393-e0a9-e50e24dcca9e"
RX = "6e400002-b5a3-f393-e0a9-e50e24dcca9e"     # Mac -> stick
TX = "6e400003-b5a3-f393-e0a9-e50e24dcca9e"     # stick -> Mac
AUDIO = "6e400004-b5a3-f393-e0a9-e50e24dcca9e"  # stick -> Mac, agent audio frames
AUDIO_IN = "6e400005-b5a3-f393-e0a9-e50e24dcca9e"  # Mac -> stick, sound clips
OTA_DATA = "6e400006-b5a3-f393-e0a9-e50e24dcca9e"  # Mac -> stick, firmware image (stick 1.8.0+, ota.py)
HELLO_EVERY_S = 8
CHUNK = 180

log = logging.getLogger("link")


async def scan(timeout=5.0):
    """Nearby sticks: [{address, name, rssi}] (strongest first)."""
    found = await BleakScanner.discover(timeout=timeout, return_adv=True, service_uuids=[NUS])
    out = [{"address": d.address, "name": ad.local_name or d.name or "?", "rssi": ad.rssi}
           for d, ad in found.values() if NUS in [u.lower() for u in ad.service_uuids]]
    return sorted(out, key=lambda x: -x["rssi"])


class Link:
    """Keeps a connection to the chosen stick. Callbacks:
    on_message(dict) for JSON lines, on_audio(bytes) for audio frames,
    on_state(str) with 'no-stick' | 'searching' | 'connected' | 'paused'."""

    def __init__(self, on_message, on_audio, on_state, hello_extra=None):
        self.on_message, self.on_audio, self.on_state = on_message, on_audio, on_state
        self.hello_extra = hello_extra  # () -> dict merged into every hello (e.g. the home screen line)
        self.client = None
        self.target = None      # {"address", "name"}
        self.paused = False
        self.mtu = 0
        self._buf = bytearray()
        self._wake = asyncio.Event()
        self._write_lock = asyncio.Lock()

    @property
    def connected(self):
        return self.client is not None and self.client.is_connected

    def set_target(self, target):
        self.target = target
        self._wake.set()
        if self.client:
            asyncio.ensure_future(self.client.disconnect())

    def pause(self, paused):
        self.paused = paused
        self._wake.set()
        if paused and self.client:
            asyncio.ensure_future(self.client.disconnect())

    async def send(self, msg: dict):
        if not self.connected:
            return False
        data = (json.dumps(msg, separators=(",", ":")) + "\n").encode()
        try:
            async with self._write_lock:
                for i in range(0, len(data), CHUNK):
                    await self._write(data[i:i + CHUNK])
            return True
        except Exception as e:  # noqa: BLE001 — link dropped mid-write
            log.warning("send failed (%s, %d bytes): %s", next(iter(msg), "?"), len(data), e)
            return False

    async def _write(self, part, tries=4):
        """One chunk; "Insufficient Resource" means the stick's Bluetooth buffers are busy
        for a moment (a long reply going out): wait and send it again."""
        for n in range(tries):
            try:
                return await self.client.write_gatt_char(RX, part, response=True)
            except Exception as e:  # noqa: BLE001
                if "Insufficient Resource" not in str(e) or n == tries - 1:
                    raise
                await asyncio.sleep(0.1 * (n + 1))

    async def send_audio(self, pkts):
        """Sends clip packets in order (write without response; Core Bluetooth paces them)."""
        if not self.connected:
            return False
        try:
            async with self._write_lock:
                # Core Bluetooth drops write-without-response packets when its queue is
                # full and bleak doesn't wait for room, so we check canSendWriteWithoutResponse
                # ourselves. First and last packets are acknowledged (start / end of a clip).
                periph = getattr(getattr(self.client, "_backend", None), "_peripheral", None)
                for i, p in enumerate(pkts):
                    if i == 0 or i == len(pkts) - 1 or periph is None:
                        await self.client.write_gatt_char(AUDIO_IN, p, response=True)
                        continue
                    for _ in range(400):  # up to ~2 s
                        if periph.canSendWriteWithoutResponse():
                            break
                        await asyncio.sleep(0.005)
                    await self.client.write_gatt_char(AUDIO_IN, p, response=False)
            return True
        except Exception as e:  # noqa: BLE001
            log.warning("audio send failed: %s", e)
            return False

    def ota_max_write(self):
        """Largest write without response to the OTA characteristic (MTU - 3 if unknown)."""
        try:
            ch = self.client.services.get_characteristic(OTA_DATA)
            if ch is not None and ch.max_write_without_response_size:
                return int(ch.max_write_without_response_size)
        except Exception:  # noqa: BLE001 — not connected / old bleak
            pass
        return max(20, (self.mtu or 23) - 3)

    async def ota_write(self, pkt):
        """One firmware packet (see ota.py), write without response. Waits for room in
        Core Bluetooth's queue first (it drops packets silently when full, like clips).
        False if the link is down."""
        if not self.connected:
            return False
        periph = getattr(getattr(self.client, "_backend", None), "_peripheral", None)
        try:
            async with self._write_lock:
                if periph is None:  # not Core Bluetooth: slow but safe
                    await self.client.write_gatt_char(OTA_DATA, pkt, response=True)
                    return True
                for _ in range(1000):  # up to ~2 s
                    if periph.canSendWriteWithoutResponse():
                        break
                    await asyncio.sleep(0.002)
                await self.client.write_gatt_char(OTA_DATA, pkt, response=False)
            return True
        except Exception as e:  # noqa: BLE001 — link dropped mid-write
            log.warning("ota write failed: %s", e)
            return False

    def _on_tx(self, _, data):
        self._buf.extend(data)
        while b"\n" in self._buf:
            i = self._buf.index(b"\n")
            line, self._buf = bytes(self._buf[:i]), self._buf[i + 1:]
            try:
                msg = json.loads(line)
            except ValueError:
                log.warning("unreadable line from the stick (%d bytes): %r", len(line), line[:80])
                continue
            self.on_message(msg)

    async def run(self):
        while True:
            if self.paused:
                self.on_state("paused")
            elif not self.target:
                self.on_state("no-stick")
            if self.paused or not self.target:
                self._wake.clear()
                await self._wake.wait()
                continue
            self.on_state("searching")
            try:
                dev = await BleakScanner.find_device_by_address(self.target["address"], timeout=10)
                if dev is None:
                    await asyncio.sleep(3)
                    continue
                disconnected = asyncio.Event()
                # bind this connection's event now: an old connection's late callback mustn't end the next one
                # only our service: firmware 2.5.0's HID makes macOS hand back the Battery service in a
                # way bleak can't read ("'CBService' object has no attribute 'handle'"), and we need nothing else
                async with BleakClient(dev, services=[NUS],
                                       disconnected_callback=lambda _, ev=disconnected: ev.set()) as c:
                    self.client = c
                    self.mtu = c.mtu_size
                    await c.start_notify(TX, self._on_tx)
                    await c.start_notify(AUDIO, lambda _, d: self.on_audio(bytes(d)))
                    log.info("connected to %s (MTU %d)", self.target.get("name"), self.mtu)
                    self.on_state("connected")
                    while not disconnected.is_set() and not self.paused:
                        offset = -time.altzone if time.daylight and time.localtime().tm_isdst else -time.timezone
                        hello = {"src": "agent", "hello": 1, "v": 1, "t": int(time.time()), "tz": offset}
                        if self.hello_extra:
                            try:
                                hello.update(self.hello_extra())
                            except Exception as e:  # noqa: BLE001
                                log.warning("hello extra: %s", e)
                        await self.send(hello)
                        try:
                            await asyncio.wait_for(disconnected.wait(), HELLO_EVERY_S)
                        except TimeoutError:
                            pass
            except asyncio.CancelledError:
                raise
            except Exception as e:  # noqa: BLE001 — Bluetooth off, stick asleep, out of range...
                log.info("link: %s", e)
                await asyncio.sleep(3)
            finally:
                self.client = None
            if not self.paused:
                log.info("stick disconnected, retrying")
