"""Scores STT bench results (JSON files written by bench_parakeet_mlx.py / the Rust `stt-bench`).

usage: python3 score.py results/*.json   -> markdown: summary table + transcripts side by side

WER uses a light normalisation so formatting choices aren't counted as errors: lower case, no
punctuation, digits spelled out ("2.1.0" -> "two point one point zero", "4:30" -> "four thirty"),
"what's" -> "what is", hyphens -> spaces, and a few compounds joined ("re run" -> "rerun").
"""
import json
import os
import re
import statistics
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ONES = "zero one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen " \
       "sixteen seventeen eighteen nineteen".split()
TENS = "_ _ twenty thirty forty fifty sixty seventy eighty ninety".split()
# "4:30" and "2.1.0": every engine hears these right but writes them differently ("430", "4.30",
# "2, 1, 0", "2.10"); the second WER column leaves them out so formatting doesn't swamp the comparison.
NUMBER_FORMAT_ITEMS = {"10", "11"}
JOIN = {"re run": "rerun", "change log": "changelog", "git hub": "github", "lofi": "lo fi"}


def say_int(n):
    if n < 20:
        return ONES[n]
    if n < 100:
        return TENS[n // 10] + ("" if n % 10 == 0 else " " + ONES[n % 10])
    if n < 1000:
        return ONES[n // 100] + " hundred" + ("" if n % 100 == 0 else " " + say_int(n % 100))
    return say_int(n // 1000) + " thousand" + ("" if n % 1000 == 0 else " " + say_int(n % 1000))


def norm(text):
    t = text.lower().replace("’", "'")
    t = re.sub(r"(\d)\.(?=\d)", r"\1 point ", t)
    t = re.sub(r"(\d):(\d\d)", r"\1 \2", t)
    t = re.sub(r"\bwhat's\b", "what is", t)
    t = re.sub(r"[-_/]", " ", t)
    t = re.sub(r"[^\w' ]+", " ", t)
    t = re.sub(r"\d+", lambda m: " " + say_int(int(m.group())) + " ", t)
    t = " ".join(t.replace("'", "").split())
    # "Git Hub" is joined so WER reflects meaning; the side-by-side table still shows the raw text
    for a, b in JOIN.items():
        t = re.sub(rf"\b{a}\b", b, t)
    return t.split()


def edits(ref, hyp):
    d = list(range(len(hyp) + 1))
    for i, r in enumerate(ref, 1):
        prev, d[0] = d[0], i
        for j, h in enumerate(hyp, 1):
            cur = min(d[j] + 1, d[j - 1] + 1, prev + (r != h))
            prev, d[j] = d[j], cur
    return d[len(hyp)]


def refs():
    out = {}
    for line in open(os.path.join(HERE, "texts.tsv")):
        if line.startswith("#") or not line.strip():
            continue
        id_, voice, snr, text = line.rstrip("\n").split("\t")
        out[id_] = (voice, snr, text)
    return out


def pct(xs, p):
    xs = sorted(xs)
    k = (len(xs) - 1) * p
    lo, hi = int(k), min(int(k) + 1, len(xs) - 1)
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def main(paths):
    R = refs()
    runs = [json.load(open(p)) for p in paths]
    print("| engine | model | size on disk | load (s) | first call (s) | warm median (ms) | warm p90 (ms) "
          "| RTF (warm median) | peak RSS (MB) | peak footprint (MB) | WER | WER excl. 10+11 | utterances exact |")
    print("|---|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in runs:
        warm = [statistics.median(u["warm_s"]) for u in r["results"]]
        rtf = [statistics.median(u["warm_s"]) / u["audio_s"] for u in r["results"]]
        errs = words = exact = errs2 = words2 = 0
        for u in r["results"]:
            ref, hyp = norm(R[u["id"]][2]), norm(u["text"])
            e = edits(ref, hyp)
            errs, words, exact = errs + e, words + len(ref), exact + (e == 0)
            if u["id"] not in NUMBER_FORMAT_ITEMS:
                errs2, words2 = errs2 + e, words2 + len(ref)
        size = r.get("model_bytes", 0) / 1e6
        size = f"{size / 1000:.2f} GB" if size >= 1000 else f"{size:.0f} MB"
        mem = r.get("mem", {})
        print(f"| {r['engine']} | {r['model']} | {size} | {r['load_s']:.2f} | {r['first_s']:.2f} "
              f"| {statistics.median(warm) * 1000:.0f} | {pct(warm, 0.9) * 1000:.0f} | {statistics.median(rtf):.3f} "
              f"| {mem.get('max_rss_mb', '?')} | {mem.get('peak_footprint_mb', '?')} "
              f"| {100 * errs / words:.1f}% | {100 * errs2 / words2:.1f}% | {exact}/{len(r['results'])} |")
    print()
    print("| id | voice | reference | " + " | ".join(r["engine"] for r in runs) + " |")
    print("|---|---|---|" + "---|" * len(runs))
    for id_, (voice, snr, text) in R.items():
        cells = []
        for r in runs:
            u = next((u for u in r["results"] if u["id"] == id_), None)
            if u is None:
                cells.append("—")
                continue
            ok = edits(norm(text), norm(u["text"])) == 0
            cells.append(("" if ok else "**✗** ") + u["text"].replace("|", "\\|"))
        v = voice + ("" if snr == "-" else f" +noise {snr} dB")
        print(f"| {id_} | {v} | {text} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main(sys.argv[1:])
