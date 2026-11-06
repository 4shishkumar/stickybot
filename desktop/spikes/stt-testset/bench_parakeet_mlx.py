"""Baseline: today's Python STT (parakeet-mlx, exactly as mac-agent/stick_agent/stt.py calls it).

usage: ~/.stick-agent/venv/bin/python bench_parakeet_mlx.py OUT.json [--runs 5]
Run under `/usr/bin/time -l` to get peak RSS / footprint (run_all.sh does that and merges it in).
"""
import json
import os
import sys
import time
import wave

import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
MODEL = "mlx-community/parakeet-tdt-0.6b-v3"


def read_wav(path):
    with wave.open(path) as w:
        assert w.getframerate() == 16000 and w.getnchannels() == 1 and w.getsampwidth() == 2
        return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0


def main():
    out = sys.argv[1]
    runs = int(sys.argv[sys.argv.index("--runs") + 1]) if "--runs" in sys.argv else 5
    os.environ["HF_HUB_OFFLINE"] = "1"
    t = time.perf_counter()
    import mlx.core as mx
    from parakeet_mlx import from_pretrained
    from parakeet_mlx.audio import get_logmel
    from huggingface_hub import snapshot_download
    import_s = time.perf_counter() - t

    t = time.perf_counter()
    model = from_pretrained(MODEL)
    load_s = time.perf_counter() - t

    def transcribe(audio):
        mel = get_logmel(mx.array(audio), model.preprocessor_config)
        return model.generate(mel)[0].text.strip()

    ids = sorted(f[:-4] for f in os.listdir(os.path.join(HERE, "wav")) if f.endswith(".wav"))
    audios = {i: read_wav(os.path.join(HERE, "wav", i + ".wav")) for i in ids}
    # "first call": the first real transcription after load (stt.py hides this behind a 1 s warm-up on zeros)
    t = time.perf_counter()
    transcribe(audios[ids[0]])
    first_s = time.perf_counter() - t
    results = []
    for i in ids:
        times, text = [], ""
        for _ in range(runs):
            t = time.perf_counter()
            text = transcribe(audios[i])
            times.append(time.perf_counter() - t)
        results.append({"id": i, "audio_s": len(audios[i]) / 16000, "warm_s": times, "text": text})
    snap = snapshot_download(MODEL)
    size = sum(os.path.getsize(os.path.join(snap, f)) for f in os.listdir(snap))
    json.dump({"engine": "parakeet-mlx (Python, today)", "model": "parakeet-tdt-0.6b-v3 (MLX, bf16)",
               "model_bytes": size, "load_s": load_s, "first_s": first_s,
               "import_s": import_s, "results": results},
              open(out, "w"), indent=1)


if __name__ == "__main__":
    main()
