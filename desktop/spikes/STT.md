# Spike 1: speech to text from Rust

**Question:** can the Rust + Tauri rewrite do speech to text as well as today's Python agent
(`mac-agent/stick_agent/stt.py`: parakeet-mlx, `mlx-community/parakeet-tdt-0.6b-v3`)?

**Answer: yes.** Three Rust options run the same Parakeet TDT 0.6b v3 model and give the same
transcripts as today. One of them, FluidAudio on the Neural Engine, is also faster than today
and uses less memory. Whisper is not worth it.

Machine: Apple M5 Pro (5P+10E cores), 24 GB, macOS 27.2. Rust 1.99.1, Swift 6.4.2. Date: 2026-11-06.

## What the engine has to do

The stick sends 16 kHz mono 4-bit IMA-ADPCM. `adpcm.Recording.audio()` decodes it to `float32`
in [-1, 1], and `STT.transcribe(audio)` is called once per push-to-talk, on one dedicated thread.
Clips under 0.1 s return "", and anything under 0.6 s is rejected earlier. The Rust trait
(`desktop/crates/stt/src/lib.rs`) mirrors this:

```rust
pub trait Stt: Send {
    fn load(&mut self) -> Result<()>;
    fn transcribe(&mut self, audio: &[f32], sample_rate: u32) -> Result<String>;
    fn name(&self) -> String;
}
```

## Test set

There are 13 utterances, 1.5–4.4 s long (2.8 s on average), in `stt-testset/texts.tsv`. They are
voice commands of the kind a coding assistant gets, and include GitHub, Claude, Tauri, Rust,
numbers, a time and a version string. They were made with macOS `say` in four voices (Samantha
US, Daniel GB, Karen AU, Rishi IN) and converted with `afconvert` to 16 kHz mono 16-bit. #13 is
#3 again with seeded pink noise mixed in at 12 dB SNR. `stt-testset/make_wavs.sh` regenerates
them. The WAVs (1.2 MB) are not committed.

## Method

- Every engine reads the same WAVs as `f32/32768` (what the agent passes) and runs in its own
  process.
- **load:** model construction (`Stt::load()`, or `from_pretrained` for MLX). Files were already
  in the page cache: a "warm-cache cold start".
- **first call:** the first real transcription after load. Shader/graph warm-up shows up here.
  `stt.py` hides it behind a warm-up on 1 s of zeros.
- **warm:** 5 repeats per utterance after that. The table gives the median and p90 over the
  per-utterance medians, plus RTF = time / audio length.
- **memory:** from `/usr/bin/time -l`. *max RSS* and *peak footprint* (`phys_footprint`, what
  Activity Monitor calls "Memory") are both given, because GPU and ANE memory distort each in
  different ways (see the notes).
- **WER:** after light normalisation (case, punctuation, digits spelled out, `what's` →
  `what is`, hyphens). #10 (`4:30`) and #11 (`2.1.0`) are mostly differences in how numbers are
  written, so a second WER column leaves them out.
- The live StickyBot agent (MLX model loaded, idle) was running during all runs.

## Results

| engine | model | size on disk | download | load (s) | first call (s) | warm median (ms) | warm p90 (ms) | RTF | peak RSS (MB) | peak footprint (MB) | WER | WER excl. #10/#11 | exact |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **parakeet-mlx** (Python, today) | parakeet-tdt-0.6b-v3 MLX bf16 | 2.51 GB | 2.5 GB | 0.25 | 0.25–1.23 | 51 | 56 | 0.018 | 1157 | 2838–4322 | 2.5% | 1.0% | 11/13 |
| **FluidAudio** (Swift via Rust, CoreML, ANE) | parakeet-tdt-0.6b-v3-coreml (int8 enc.) | 470 MB | 470 MB | 0.16 | 0.06 | **36** | **37** | 0.013 | 86 ¹ | 47 ¹ | 2.5% | 1.0% | 11/13 |
| FluidAudio (CoreML, GPU encoder) | same | 470 MB | – | 0.99 | 2.94 | 28 | 30 | 0.010 | 7560 ² | 210 ² | 2.5% | 1.0% | 11/13 |
| **sherpa-onnx** (Rust, CPU, 8 threads) | nemo-parakeet-tdt-0.6b-v3-int8 ONNX | 670 MB | 487 MB (.tar.bz2) | 0.56 | 0.11 | 124 | 165 | 0.045 | 1148 | 1134 | 4.2% | 1.0% | 10/13 |
| sherpa-onnx (Rust, CPU, 4 threads) | same | 670 MB | – | 0.54 | 0.16 | 179 | 215 | 0.064 | 1134 | 1120 | 4.2% | 1.0% | 10/13 |
| sherpa-onnx (Rust, ONNX Runtime CoreML EP) | same | 670 MB | – | 3.08 | 0.37 | 112 | 129 | 0.039 | 2196 | 2179 | 4.2% | 1.0% | 10/13 |
| whisper.cpp (Rust, Metal) | large-v3-turbo q5_0 | 574 MB | 574 MB | 0.38 (5.4 on first run) | 0.58 | 563 | 569 | 0.203 | 738 | 771 | 3.4% | 3.1% | 10/13 |
| whisper.cpp (Rust, Metal) | small.en q5_1 | 190 MB | 190 MB | 0.20 | 0.15 | 136 | 148 | 0.049 | 368 | 401 | 7.6% | 6.2% | 9/13 |

Ranges for MLX are spread across two runs. Its first call took 1.23 s in a fresh run and 0.25 s
when the Metal kernel cache was warm.

¹ With FluidAudio on the ANE, the model and its working memory live in the system's ANE service
(`aned`), not in our process. The real cost to the system is higher than 86 MB, and I didn't
measure it.
² With the GPU encoder, RSS counts the shared/IOSurface buffers CoreML maps. The footprint is the
truer figure for our own process, though GPU memory is still unified RAM.

**First load ever**, which needs a progress UI just like today's "downloading the speech model":
- FluidAudio: 230 s, which covers the 470 MB download plus CoreML compiling the model for the
  ANE. Later loads take 0.16 s.
- whisper turbo: 5.4 s (Metal shaders and a cold file).
- sherpa-onnx: about 0.7 s after extracting the archive.

I didn't measure true cold-from-reboot loads (no `sudo purge`).

### Transcripts

Every Parakeet build (MLX, FluidAudio, sherpa) gives the **same words** on all 13. They differ
only in how they write numbers:

| id | voice | reference | parakeet-mlx (today) | FluidAudio | sherpa-onnx | whisper turbo | whisper small.en |
|---|---|---|---|---|---|---|---|
| 01 | Samantha | Open the pull request for the login fix. | = | = | = | = | = |
| 02 | Daniel | What's the weather tomorrow? | = | = | = | = | = |
| 03 | Karen | Run the tests and tell me what failed. | = | = | = | = | run the tests and tell me what failed |
| 04 | Samantha | Push the branch to GitHub and open a draft PR. | = | = | = | push the branch … | = |
| 05 | Daniel | Ask Claude to refactor the Tauri window code. | **Tory** | **Tory** | **Tory** | **Tory** | **Tori** |
| 06 | Karen | Set a timer for 25 minutes. | twenty-five | = | = | = | = |
| 07 | Rishi | Rename the function parse config to load settings. | = | = | = | **PaaSconfig** | **passConfigToLoadSettings** |
| 08 | Samantha | How many open issues are assigned to me on GitHub? | = | = | = | = | = |
| 09 | Daniel | Revert the last 3 commits on main and rerun the build. | three | three | three | = | three |
| 10 | Karen | Remind me to call Sarah at 4:30 tomorrow afternoon. | 4 30 | four thirty | **430** | **4.30** | **4.30** |
| 11 | Rishi | Bump the version to 2.1.0 and update the changelog. | **2, 1, 0** … change log | **2, 1, 0** … change log | **2.10** … change log | 2.1.0 … change log | **2,1,0** … change log |
| 12 | Samantha | Summarize the build log and fix the first error in the Rust code. | = | = | = | = | = |
| 13 | Daniel + noise | Run the tests and tell me what failed. | = | = | = | = | = |

"=" means the same as the reference. The full raw tables are in `stt-results/*.json`, and
`python3 stt-testset/score.py stt-results/*.json` prints them.

- **"Tauri" → "Tory"** happens in every engine. It is a vocabulary problem, not an engine
  problem. Two possible fixes: sherpa-onnx supports hotwords/contextual biasing for transducers
  (`create_stream_with_hotwords`), and FluidAudio has `CustomVocabulary`. Neither was tried.
- **sherpa's "430" and "2.10"** come from how it joins digit tokens. MLX gives "4 30" and
  "2, 1, 0" from the same model, so the words it heard are the same. This only matters if the brain reads versions or times literally.

## Recommendation

1. **Default engine for the rewrite: sherpa-onnx (official `sherpa-onnx` crate 2.0.1), CPU,
   static, 8 threads, Parakeet TDT 0.6b v3 int8.**
   - It meets the bar: same model, same words as today, 124 ms for a 2.8 s command (RTF 0.045).
     That is invisible next to the BLE audio transfer and the LLM call.
   - It builds with plain `cargo build`. There's no Swift toolchain, no cmake and no macOS-version
     floor, and it works on Linux/Windows if that ever matters.
   - It downloads 487 MB instead of 2.5 GB.
   - It holds about 1.1 GB resident, while MLX's footprint is about 2.8–4.3 GB.
   - Skip the CoreML EP. It is only about 10% faster and costs twice the memory, a 3 s load and a
     shipped `libonnxruntime.dylib`.
2. **Upgrade path: FluidAudio (CoreML on the ANE) behind the same `Stt` trait.** It is the only
   option that beats today on every axis: 36 ms (1.4× faster than MLX), almost no memory in our
   process, and identical words, with "four thirty" even spelled out properly. Adopt it once the
   Tauri bundle exists and these are solved:
   - The Swift build step in the app build.
   - The macOS 14+ floor.
   - The duplicate `_rust_eh_personality` at link. FluidAudio bundles a Rust staticlib for text
     normalisation, so there are two Rust standard libraries in one binary. Likely fix: build
     FluidBridge as a dynamic library inside the app's Frameworks. Not tried.
   - The first load of several minutes (download plus ANE compile), which needs progress UI.
   - Effort for this spike: about 1.5 h. The FFI shim is about 90 lines of Swift and 90 of Rust.
3. **Don't use whisper.** large-v3-turbo is 4.5× slower than sherpa and no more accurate here
   (it still says "Tory", and adds "PaaSconfig"). small.en is fast but less accurate. Keep it in
   mind only if languages outside Parakeet v3's 25 European ones are ever needed.

## Build and run (exact steps)

**Prerequisites:**
- rustup stable (already present here).
- Xcode command line tools (swift, for `fluid` only).
- `brew install cmake` (installed for this spike, needed only by `whisper-rs`).

```bash
cd desktop

# 1. models → ~/Library/Caches/stickybot-spike (FluidAudio downloads its own on first load)
M=~/Library/Caches/stickybot-spike; mkdir -p $M/sherpa $M/whisper $M/fluidaudio
curl -L -o $M/sherpa/p.tar.bz2 https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2
tar xjf $M/sherpa/p.tar.bz2 -C $M/sherpa && rm $M/sherpa/p.tar.bz2
curl -L -o $M/whisper/ggml-large-v3-turbo-q5_0.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
curl -L -o $M/whisper/ggml-small.en-q5_1.bin     https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin

# 2. builds (three binaries: the engines' link modes differ)
cargo build --release                                              # sherpa (static) + whisper (Metal)
cargo build --release --no-default-features --features sherpa-shared --target-dir target-shared  # CoreML EP
cargo build --release --no-default-features --features fluid --target-dir target-fluid           # FluidAudio

# 3. test WAVs, then every bench + the tables (the baseline uses ~/.stick-agent/venv/bin/python, read-only)
spikes/stt-testset/make_wavs.sh
spikes/stt-testset/run_all.sh            # or e.g.: run_all.sh sherpa-cpu8 fluid
```

**Build notes:**
- `sherpa-onnx-sys` downloads a prebuilt static archive (`sherpa-onnx-v2.0.1-osx-arm64-static-lib`)
  from GitHub at build time. For offline or CI builds, point `SHERPA_ONNX_ARCHIVE_DIR` at a
  cached copy.
- That static archive has **no CoreML EP**. Asking for `provider = "coreml"` logs
  `Fallback to cpu!` and runs on the CPU. The `shared` feature links `libonnxruntime.dylib`,
  which does have the CoreML EP. It needs an rpath; `build.rs` adds `@executable_path`, and
  `run_all.sh` copies the dylibs next to the binary.
- `fluid`: `build.rs` runs `swift build -c release` on `crates/stt/swift/FluidBridge` (FluidAudio
  0.18.2, pinned in `Package.resolved`). It then links `libFluidBridge.a` and
  `libtext_processing_rs.a` plus the Swift runtime search paths. The link prints a harmless-looking
  but real `duplicate symbol '_rust_eh_personality'` warning (see the recommendation).
- `[profile.release] strip = "none"`: `rust-objcopy` aborts (SIGABRT) when it strips on macOS
  27.2. It does no harm, but the output is noisy.

## Files

- `desktop/crates/stt/` contains:
  - the `Stt` trait
  - `sherpa.rs`, `whisper.rs` and `fluid.rs`
  - `swift/FluidBridge`, the C-ABI wrapper around FluidAudio
  - `src/bin/stt-bench.rs`
- `desktop/spikes/stt-testset/` contains:
  - `texts.tsv`, the references
  - `make_wavs.sh` and `add_noise.py`
  - `bench_parakeet_mlx.py`, the baseline
  - `with_mem.py`, the `/usr/bin/time -l` wrapper
  - `score.py`, for WER and tables
  - `run_all.sh`
- `desktop/spikes/stt-results/*.json` holds the raw timings and transcripts behind the tables
  above.

## Not verified

- **Real voices through the real path.** The test audio is all TTS, which is clean and
  well-paced. It was never run through the stick's mic and IMA-ADPCM. The next step is to record
  about 20 real push-to-talk clips from the stick, for example by dumping `rec.audio()` in the
  agent, and rerun `run_all.sh` on them.
- **The test set is small.** 13 utterances can't separate the Parakeet builds. Their only
  differences are number formatting.
- **Long input.** Nothing longer than 4.4 s was tried. FluidAudio chunks at about 15 s and
  whisper pads to 30 s windows.
- **Truly cold starts** (after a reboot or once the OS evicts the CoreML/ANE compile cache) and
  the system-wide ANE memory used by `aned`.
- **Tauri bundling, codesigning and notarization** of any of these: the sherpa dylibs, the
  FluidBridge Swift runtime, the whisper Metal shaders.
- **FluidBridge thread safety.** Its `lastError` is a plain global, which is fine for one
  worker thread but not more.
- **Sherpa and whisper features that could help but weren't tried:** sherpa hotwords for
  "Tauri", and whisper `audio_ctx` trimming, which would make turbo much faster on short clips.
