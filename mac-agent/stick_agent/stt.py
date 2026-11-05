"""Speech to text, on the Mac (Parakeet via MLX). Nothing leaves the machine."""
import logging
import time

import numpy as np

MODEL = "mlx-community/parakeet-tdt-0.6b-v3"
log = logging.getLogger("stt")


class STT:
    """MLX keeps its streams per thread, so the model is loaded and always run on
    one dedicated thread (self.thread) — never on asyncio's shared pool."""

    def __init__(self):
        from concurrent.futures import ThreadPoolExecutor
        self.model = None
        self.thread = ThreadPoolExecutor(max_workers=1, thread_name_prefix="stt")

    def load(self, name=MODEL):
        import os
        t = time.time()
        os.environ["HF_HUB_OFFLINE"] = "1"  # use the downloaded copy; no network check at every start
        try:
            from parakeet_mlx import from_pretrained
            self.model = from_pretrained(name)
        except Exception:  # noqa: BLE001 — first run: download it (~2.5 GB)
            log.info("downloading the speech model (first run only)…")
            os.environ.pop("HF_HUB_OFFLINE", None)
            import huggingface_hub.constants as hc
            hc.HF_HUB_OFFLINE = False
            self.model = from_pretrained(name)
        self.name = name
        self.transcribe(np.zeros(16000, dtype=np.float32))  # warm-up: the first run is slow
        log.info("speech model ready in %.1fs", time.time() - t)

    def transcribe(self, audio: np.ndarray) -> str:
        if self.model is None:
            raise RuntimeError("speech model not loaded yet")
        if len(audio) < 1600:  # under 0.1 s
            return ""
        import mlx.core as mx  # (after the checks: the release workflow's Linux tests have no MLX)
        from parakeet_mlx.audio import get_logmel
        mel = get_logmel(mx.array(audio), self.model.preprocessor_config)
        return self.model.generate(mel)[0].text.strip()
