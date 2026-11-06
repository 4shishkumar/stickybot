"""link.send: the stick's Bluetooth buffers can be busy for a moment ("Insufficient
Resource" while a long reply goes out); the chunk is sent again instead of being lost."""
from stick_agent import link


class Client:
    def __init__(self, busy=0, error="(17, 'GATT Protocol Error: Insufficient Resource')"):
        self.busy, self.error, self.parts = busy, error, []

    async def write_gatt_char(self, char, data, response):
        if self.busy:
            self.busy -= 1
            raise OSError(self.error)
        self.parts.append(bytes(data))


def make(client, monkeypatch):
    async def no_wait(_s):
        pass
    monkeypatch.setattr(link.asyncio, "sleep", no_wait)
    lk = link.Link(lambda m: None, lambda a: None, lambda s: None)
    lk.client = client
    monkeypatch.setattr(link.Link, "connected", property(lambda self: True))
    return lk


async def test_a_busy_stick_gets_the_chunk_again(monkeypatch):
    c = Client(busy=2)
    lk = make(c, monkeypatch)
    assert await lk.send({"src": "agent", "x": "y" * 400})
    assert b"".join(c.parts).endswith(b"}\n") and len(c.parts) == 3


async def test_it_gives_up_after_a_few_tries(monkeypatch):
    lk = make(Client(busy=10), monkeypatch)
    assert not await lk.send({"src": "agent"})


async def test_other_errors_are_not_retried(monkeypatch):
    c = Client(busy=1, error="disconnected")
    lk = make(c, monkeypatch)
    assert not await lk.send({"src": "agent"}) and c.busy == 0 and not c.parts
