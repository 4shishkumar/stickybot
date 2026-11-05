"""Selectable brains: who answers a voice request that isn't a quick command (config "brain").

- "claude" (default): Claude Code with the user's own subscription (brain.py), no key.
- "openrouter": any model on OpenRouter with the user's own API key, pay per use (openai_brain.py).

core.py talks to one Brains object with the Brain interface (busy, cancelled, alive, pid, status(),
ask(), cancel(), forget(), prewarm()); it hands each request to the brain picked in Settings, so a
change applies from the next request on, without a restart. Switching away from Claude stops its
session process; Claude isn't pre-warmed while another brain is picked.
"""
import logging

from . import brain, config, openai_brain

log = logging.getLogger("brain")

NAMES = ("claude", "openrouter")


def selected(cfg=None):
    name = (cfg or config.load()).get("brain") or "claude"
    return name if name in NAMES else "claude"


class Brains:
    def __init__(self, agent=None):
        self.claude = brain.Brain()
        self.openrouter = openai_brain.OpenRouterBrain(agent)
        self._last = self.claude   # the brain of the last request (its `cancelled` flag)
        self._name = selected()

    def all(self):
        return (self.claude, self.openrouter)

    @property
    def name(self):
        return selected()

    @property
    def current(self):
        return getattr(self, self.name)

    @property
    def busy(self):
        return any(b.busy for b in self.all())

    @property
    def cancelled(self):
        return self._last.cancelled

    @property
    def alive(self):
        return self.current.alive

    @property
    def pid(self):
        """Our own Claude Code process, if running (home.py doesn't count it as one of the user's)."""
        return self.claude.pid

    def switched(self):
        """The picked brain changed (Settings): the old one forgets its conversation; Claude's session
        process stops (it would only sit there until its idle time)."""
        name = self.name
        if name == self._name:
            return
        log.info("brain: %s -> %s", self._name, name)
        getattr(self, self._name).forget()
        self._name = name

    def status(self):
        return {"brain": self.name, **self.current.status()}

    async def ask(self, text, on_progress, on_text=None):
        self.switched()  # (a config.json edited by hand)
        if self.busy:
            return False, "Still working on the last request."
        self._last = b = self.current
        return await b.ask(text, on_progress, on_text)

    def cancel(self):
        for b in self.all():
            b.cancel()

    def forget(self):
        for b in self.all():
            b.forget()

    async def prewarm(self):
        if self.name == "claude":
            await self.claude.prewarm()

    async def close(self):
        await self.openrouter.close()
