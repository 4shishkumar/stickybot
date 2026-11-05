import pytest

rumps = pytest.importorskip("rumps")  # macOS only

from stick_agent import menubar  # noqa: E402


def test_check_result_says_what_the_check_found():
    assert menubar.check_result({"available": True, "title": "Update available: v1.9.0"}) == \
        "Update available: v1.9.0 — see the item above"
    assert menubar.check_result({"error": "HTTP 403", "current": {"agent": "1.8.1"}}) == \
        "Couldn't check for updates: HTTP 403"
    assert menubar.check_result({"current": {"agent": "1.8.1"}}) == "Up to date (v1.8.1)"
