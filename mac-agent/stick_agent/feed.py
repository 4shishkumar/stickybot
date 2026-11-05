"""Experimental: something to read on the home screen when nothing needs you.

Sources (all free, no keys), chosen in the dashboard:
  facts     — a random fun fact (uselessfacts.jsph.pl)
  hn        — top Hacker News stories
  onthisday — Wikipedia's "on this day"
  wikipedia — Wikipedia's featured article of the day
Refreshed every 30 minutes; the home screen rotates through them.
"""
import logging
import random
import time

import aiohttp

log = logging.getLogger("feed")
REFRESH_S = 30 * 60
UA = {"User-Agent": "StickyBot/0.1 (desk gadget)"}


class Feed:
    def __init__(self):
        self.items = []   # [{"k": "fact"|"news"|"history", "a": title, "b": text}]
        self._next = 0.0
        self._i = 0

    async def _get(self, s, url):
        async with s.get(url, headers=UA, timeout=aiohttp.ClientTimeout(total=10)) as r:
            r.raise_for_status()
            return await r.json(content_type=None)

    async def refresh(self, cfg):
        sources = cfg.get("idle_content") or []
        if not sources:
            self.items = []
            return
        if time.time() < self._next:
            return
        items = []
        async with aiohttp.ClientSession() as s:
            if "facts" in sources:
                for _ in range(3):
                    try:
                        d = await self._get(s, "https://uselessfacts.jsph.pl/api/v2/facts/random?language=en")
                        items.append({"k": "fact", "a": "Fun fact", "b": d["text"].strip()})
                    except Exception as e:  # noqa: BLE001
                        log.info("facts: %s", e)
                        break
            if "hn" in sources:
                try:
                    ids = (await self._get(s, "https://hacker-news.firebaseio.com/v0/topstories.json"))[:3]
                    for i in ids:
                        it = await self._get(s, f"https://hacker-news.firebaseio.com/v0/item/{i}.json")
                        items.append({"k": "news", "a": "Hacker News", "b": it["title"]})
                except Exception as e:  # noqa: BLE001
                    log.info("hn: %s", e)
            if "wikipedia" in sources:
                try:
                    t = time.localtime()
                    d = await self._get(s, f"https://api.wikimedia.org/feed/v1/wikipedia/en/featured/{t.tm_year}/{t.tm_mon:02d}/{t.tm_mday:02d}")
                    tfa = d.get("tfa") or {}
                    title = (tfa.get("titles") or {}).get("normalized") or tfa.get("title", "")
                    extract = (tfa.get("extract") or "").strip()
                    if title and extract:
                        first = extract.split(". ")[0].rstrip(".") + "."
                        # the first sentence names the article
                        items.append({"k": "wiki", "a": "Wikipedia", "b": first})
                except Exception as e:  # noqa: BLE001
                    log.info("wikipedia: %s", e)
            if "onthisday" in sources:
                try:
                    t = time.localtime()
                    d = await self._get(s, f"https://api.wikimedia.org/feed/v1/wikipedia/en/onthisday/selected/{t.tm_mon:02d}/{t.tm_mday:02d}")
                    picks = random.sample(d.get("selected", []), min(2, len(d.get("selected", []))))
                    for p in picks:
                        items.append({"k": "history", "a": "On this day", "b": f"{p.get('year')}: {p['text'].strip()}"})
                except Exception as e:  # noqa: BLE001
                    log.info("onthisday: %s", e)
        if items:
            random.shuffle(items)
            self.items = items
        self._next = time.time() + (REFRESH_S if items else 5 * 60)
        log.info("feed: %d items", len(self.items))

    def next(self):
        if not self.items:
            return None
        self._i = (self._i + 1) % len(self.items)
        return self.items[self._i]
