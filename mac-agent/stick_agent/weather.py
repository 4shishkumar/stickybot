"""Weather for the stick's home screen, from Open-Meteo (free, no API key).

Location: "auto" guesses it from this Mac's internet connection (ipinfo.io,
falling back to geojs.io), or a city name set in the dashboard (Open-Meteo's
geocoding). Refreshed every 30 minutes; only a few numbers go to the stick.
"""
import logging
import time

import aiohttp

log = logging.getLogger("weather")
REFRESH_S = 30 * 60
RETRY_S = 5 * 60

# WMO weather codes -> (icon key the stick draws, one word)
def describe(code):
    if code == 0:
        return "clear", "clear"
    if code in (1, 2):
        return "partly", "partly cloudy"
    if code == 3:
        return "cloudy", "cloudy"
    if code in (45, 48):
        return "fog", "fog"
    if 51 <= code <= 57:
        return "rain", "drizzle"
    if 61 <= code <= 67 or 80 <= code <= 82:
        return "rain", "rain"
    if 71 <= code <= 77 or code in (85, 86):
        return "snow", "snow"
    if code >= 95:
        return "storm", "storm"
    return "cloudy", "cloudy"


class Weather:
    def __init__(self):
        self.now = None          # dict sent to the stick, or None
        self.place = None        # {"name", "lat", "lon"}
        self._next = 0.0
        self._place_for = None   # the config value self.place was resolved from

    async def _json(self, session, url, **params):
        async with session.get(url, params=params, timeout=aiohttp.ClientTimeout(total=10)) as r:
            r.raise_for_status()
            return await r.json(content_type=None)

    async def _locate(self, session, where):
        if where and where != "auto":
            d = await self._json(session, "https://geocoding-api.open-meteo.com/v1/search", name=where, count=1)
            if not d.get("results"):
                raise ValueError(f"no place called {where!r}")
            r = d["results"][0]
            return {"name": r["name"], "lat": r["latitude"], "lon": r["longitude"]}
        try:
            d = await self._json(session, "https://ipinfo.io/json")
            lat, lon = (float(x) for x in d["loc"].split(","))
            return {"name": d.get("city", ""), "lat": lat, "lon": lon}
        except Exception:  # noqa: BLE001
            d = await self._json(session, "https://get.geojs.io/v1/ip/geo.json")
            return {"name": d.get("city", ""), "lat": float(d["latitude"]), "lon": float(d["longitude"])}

    async def refresh(self, cfg, force=False):
        if not cfg.get("weather", True):
            self.now = None
            return
        if not force and time.time() < self._next:
            return
        where = cfg.get("weather_location") or "auto"
        try:
            async with aiohttp.ClientSession() as s:
                if self.place is None or self._place_for != where:
                    self.place = await self._locate(s, where)
                    self._place_for = where
                d = await self._json(
                    s, "https://api.open-meteo.com/v1/forecast", latitude=self.place["lat"],
                    longitude=self.place["lon"], current="temperature_2m,weather_code,is_day",
                    daily="temperature_2m_max,temperature_2m_min", forecast_days=1, timezone="auto")
            cur, day = d["current"], d["daily"]
            icon, word = describe(int(cur["weather_code"]))
            self.now = {"t": round(cur["temperature_2m"]), "i": icon, "c": word, "day": int(cur.get("is_day", 1)),
                        "hi": round(day["temperature_2m_max"][0]), "lo": round(day["temperature_2m_min"][0]),
                        "at": self.place["name"]}
            self._next = time.time() + REFRESH_S
            log.info("weather %s: %s°, %s", self.place["name"], self.now["t"], word)
        except Exception as e:  # noqa: BLE001 — offline, blocked, rate-limited...
            log.warning("weather: %s", e)
            self._next = time.time() + RETRY_S
