"""Speech to text with the real model (slow: run with `pytest -m slow`)."""
import shutil
import subprocess

import numpy as np
import pytest

from stick_agent import stt


@pytest.mark.slow
def test_real_model_transcribes_a_spoken_pr_number(tmp_path):
    if not shutil.which("say"):
        pytest.skip("needs macOS `say`")
    import soundfile as sf
    import soxr
    wav = tmp_path / "pr.aiff"
    subprocess.run(["say", "-o", str(wav), "Open pull request twelve thirty four"], check=True)
    data, sr = sf.read(str(wav), dtype="float32", always_2d=True)
    audio = soxr.resample(data.mean(axis=1), sr, 16000).astype(np.float32)
    s = stt.STT()
    s.thread.submit(s.load, "mlx-community/parakeet-tdt-0.6b-v2").result()
    text = s.thread.submit(s.transcribe, audio).result()
    assert "twelve thirty four" in text.lower() or "1234" in text


def test_too_little_audio_is_empty_without_a_model():
    s = stt.STT()
    s.model = object()  # loaded-looking; short audio returns before using it
    assert s.transcribe(np.zeros(100, dtype=np.float32)) == ""


def test_not_loaded():
    with pytest.raises(RuntimeError):
        stt.STT().transcribe(np.zeros(16000, dtype=np.float32))
