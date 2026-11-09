#!/bin/bash
# Runs every STT bench on the same WAVs and prints the scored tables.
# usage: desktop/spikes/stt-testset/run_all.sh [names...]   (default: all)
#   names: mlx sherpa-cpu sherpa-cpu8 sherpa-coreml whisper-turbo whisper-small fluid fluid-gpu
# Needs: make_wavs.sh run once; models in ~/Library/Caches/stickybot-spike (see ../STT.md).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
desk="$(cd "$here/../.." && pwd)"
res="$here/../stt-results"
M="$HOME/Library/Caches/stickybot-spike"
W="$here/wav"
PARAKEET="$M/sherpa/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8"
BENCH="$desk/target/release/stt-bench"
BENCH_SHARED="$desk/target-shared/release/stt-bench"   # built with --features sherpa-shared (CoreML EP)
BENCH_FLUID="$desk/target-fluid/release/stt-bench"     # built with --features fluid
mkdir -p "$res"
names=("$@")
[ ${#names[@]} -eq 0 ] && names=(mlx sherpa-cpu sherpa-cpu8 sherpa-coreml whisper-turbo whisper-small fluid fluid-gpu)

run() { local name="$1"; shift; /usr/bin/python3 "$here/with_mem.py" "$res/$name.json" -- "$@" --out "$res/$name.json"; }

for n in "${names[@]}"; do
  case "$n" in
    mlx)           /usr/bin/python3 "$here/with_mem.py" "$res/parakeet-mlx.json" -- \
                     "$HOME/.stick-agent/venv/bin/python" "$here/bench_parakeet_mlx.py" "$res/parakeet-mlx.json" ;;
    sherpa-cpu)    run sherpa-cpu "$BENCH" --engine sherpa --model "$PARAKEET" --provider cpu --threads 4 --wavs "$W" \
                     --label "sherpa-onnx Parakeet (Rust, CPU 4 thr)" ;;
    sherpa-cpu8)   run sherpa-cpu8 "$BENCH" --engine sherpa --model "$PARAKEET" --provider cpu --threads 8 --wavs "$W" \
                     --label "sherpa-onnx Parakeet (Rust, CPU 8 thr)" ;;
    sherpa-coreml) find "$desk/target-shared/release/build" -path "*sherpa-onnx-prebuilt*" -name "*.dylib" \
                     -exec cp {} "$desk/target-shared/release/" \;
                   run sherpa-coreml "$BENCH_SHARED" --engine sherpa --model "$PARAKEET" --provider coreml --threads 4 --wavs "$W" \
                     --label "sherpa-onnx Parakeet (Rust, CoreML EP)" ;;
    whisper-turbo) run whisper-turbo "$BENCH" --engine whisper --model "$M/whisper/ggml-large-v3-turbo-q5_0.bin" --threads 4 --wavs "$W" \
                     --label "whisper.cpp large-v3-turbo q5_0 (Rust, Metal)" ;;
    whisper-small) run whisper-small "$BENCH" --engine whisper --model "$M/whisper/ggml-small.en-q5_1.bin" --threads 4 --wavs "$W" \
                     --label "whisper.cpp small.en q5_1 (Rust, Metal)" ;;
    fluid)         run fluid "$BENCH_FLUID" --engine fluid --model "$M/fluidaudio" --wavs "$W" \
                     --label "FluidAudio Parakeet v3 (Swift via Rust, CoreML, ANE)" ;;
    fluid-gpu)     run fluid-gpu "$BENCH_FLUID" --engine fluid --model "$M/fluidaudio" --units gpu --wavs "$W" \
                     --label "FluidAudio Parakeet v3 (Swift via Rust, CoreML, GPU encoder)" ;;
    *) echo "unknown bench $n" >&2; exit 2 ;;
  esac
done
/usr/bin/python3 "$here/score.py" "$res"/*.json
