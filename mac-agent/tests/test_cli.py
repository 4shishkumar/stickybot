import pytest

from stick_agent import __main__ as cli
from stick_agent import flasher, ota


@pytest.fixture
def no_flashing(monkeypatch):
    ran = []
    monkeypatch.setattr(flasher, "cli", lambda *a: ran.append(("usb", a)) or 0)
    monkeypatch.setattr(ota, "cli", lambda *a: ran.append(("ble", a)) or 0)
    return ran


@pytest.mark.parametrize("argv", [["--help"], ["help"], ["flash", "--help"], ["flash", "--ble", "-h"],
                                  ["run", "--help"], ["uninstall", "--help"], ["restart", "-h"]])
def test_help_only_prints(argv, monkeypatch, capsys, no_flashing):
    monkeypatch.setattr("sys.argv", ["stick-agent", *argv])
    monkeypatch.setattr(cli, "run", lambda **_: pytest.fail("ran the agent"))
    monkeypatch.setattr(cli, "restart", lambda: pytest.fail("restarted"))
    cli.main()
    out = capsys.readouterr().out
    assert "stickybot flash" in out and "`stick-agent` works too" in out and no_flashing == []


def test_flash_refuses_unknown_options(monkeypatch, no_flashing):
    monkeypatch.setattr("sys.argv", ["stick-agent", "flash", "--bel"])
    with pytest.raises(SystemExit, match="unknown option --bel"):
        cli.main()
    assert no_flashing == []


def test_flash_options_still_work(monkeypatch, no_flashing):
    for argv, want in [(["flash", "--port", "/dev/x", "--backup"], ("usb", ("/dev/x", True))),
                       (["flash", "--ble", "--force"], ("ble", (True,)))]:
        monkeypatch.setattr("sys.argv", ["stick-agent", *argv])
        with pytest.raises(SystemExit):
            cli.main()
        assert no_flashing.pop() == want


def test_both_command_names_are_entry_points():
    import tomllib
    from pathlib import Path
    scripts = tomllib.loads((Path(__file__).parent.parent / "pyproject.toml").read_text())["project"]["scripts"]
    assert scripts == {"stickybot": "stick_agent.__main__:main", "stick-agent": "stick_agent.__main__:main"}
