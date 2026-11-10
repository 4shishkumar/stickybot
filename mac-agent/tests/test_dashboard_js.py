"""The dashboard's JavaScript tests (tests/js/*.test.mjs) under Node's own runner; skipped without Node."""

import shutil
import subprocess
from pathlib import Path

import pytest

JS = Path(__file__).parent / "js"
NODE = shutil.which("node")


@pytest.mark.skipif(not NODE, reason="node isn't installed")
def test_dashboard_js():
    files = sorted(str(p) for p in JS.glob("*.test.mjs"))
    assert files
    r = subprocess.run([NODE, "--test", *files], capture_output=True, text=True, timeout=120)
    assert r.returncode == 0, r.stdout + r.stderr
