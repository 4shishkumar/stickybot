"""Quick commands: spoken phrase -> action, matched without any AI."""
import difflib
import json
import logging
import re
import subprocess
from pathlib import Path

from . import config

log = logging.getLogger("commands")

FILLER = r"(?:(?:hey|ok|okay|please|can you|could you|would you|go ahead and|just)\s+)*"
SMALL = {"zero": 0, "oh": 0, "one": 1, "two": 2, "three": 3, "four": 4, "five": 5, "six": 6, "seven": 7,
         "eight": 8, "nine": 9, "ten": 10, "eleven": 11, "twelve": 12, "thirteen": 13, "fourteen": 14,
         "fifteen": 15, "sixteen": 16, "seventeen": 17, "eighteen": 18, "nineteen": 19}
TENS = {"twenty": 20, "thirty": 30, "forty": 40, "fifty": 50, "sixty": 60, "seventy": 70, "eighty": 80,
        "ninety": 90}
NUMBER_WORDS = set(SMALL) | set(TENS) | {"hundred", "thousand", "double", "triple"}


def words_to_number(words):
    """Spoken numbers -> digits. Speech-to-text usually writes digits already;
    this covers the rest: 'one two three four' -> 1234, 'twelve thirty four' ->
    1234, 'double five' -> 55, 'four thousand five hundred twelve' -> 4512."""
    if "hundred" in words or "thousand" in words:  # a cardinal number
        total = cur = 0
        for w in words:
            if w in SMALL:
                cur += SMALL[w]
            elif w in TENS:
                cur += TENS[w]
            elif w == "hundred":
                cur = (cur or 1) * 100
            elif w == "thousand":
                total, cur = total + (cur or 1) * 1000, 0
            else:
                return None
        return str(total + cur)
    out, i, rep = "", 0, 1
    while i < len(words):  # chunks spoken one after another: "twelve" "thirty four" "oh" "five"
        w = words[i]
        if w in ("double", "triple"):
            rep = 2 if w == "double" else 3
        elif w in TENS:
            v = TENS[w]
            if i + 1 < len(words) and words[i + 1] in SMALL and 0 < SMALL[words[i + 1]] < 10:
                v += SMALL[words[i + 1]]
                i += 1
            out += str(v)
        elif w in SMALL:
            out += str(SMALL[w]) * rep
            rep = 1
        else:
            return None
        i += 1
    return out or None


# Look-alike letters a multilingual speech model may use for English ("П Р 1234")
LOOKALIKE = str.maketrans("АВЕКМНОРСТХаеорсухПпРрИиЛлФф", "ABEKMHOPCTXaeopcyxPpRrIiLlFf")


def normalize(text: str) -> str:
    t = text.translate(LOOKALIKE).lower()
    t = re.sub(r"(?<=\d)[,\s](?=\d{3}\b)", "", t)        # 1,234 / 1 234 -> 1234
    t = re.sub(r"#\s*(\d)", r"\1", t)                     # #1234 -> 1234
    t = re.sub(r"[^\w\s-]", " ", t).replace("-", " ")
    t = re.sub(r"\bnumber\b", " ", t)                      # "PR number 1234"
    t = re.sub(r"\b(the|a|an|my)\b", " ", t)              # "open the PR ..."
    t = re.sub(r"\b(p\s*r|pee are|p r)\b", "pr", t)        # "P R 1234"
    words, out, run = t.split(), [], []
    for w in words + [""]:
        if w in NUMBER_WORDS:
            run.append(w)
            continue
        if run:
            n = words_to_number(run)
            out.append(n if n else " ".join(run))
            run = []
        if w:
            out.append(w)
    return " ".join(out)


def phrase_regex(phrase: str):
    p = re.escape(normalize(phrase.replace("{n}", "NUMSLOT").replace("{app}", "APPSLOT").replace("{q}", "QSLOT")))
    p = p.replace("numslot", r"(?P<n>\d+)").replace("appslot", r"(?P<app>.+)").replace("qslot", r"(?P<q>.+)")
    return re.compile(rf"^{FILLER}{p}$")


def list_apps():
    apps = {}
    for d in (Path("/Applications"), Path("/System/Applications"), Path("/System/Applications/Utilities"),
              Path.home() / "Applications"):
        for a in d.glob("*.app"):
            apps[a.stem.lower()] = a.stem
    return apps


# Spoken names that don't look like the app's file name
ALIASES = {"vs code": "visual studio code", "vscode": "visual studio code", "code": "visual studio code",
           "chrome": "google chrome", "zoom": "zoom.us", "brave": "brave browser", "browser": "brave browser",
           "terminal": "terminal", "settings": "system settings", "mail": "mail", "calendar": "calendar",
           "github": "github desktop", "docs": "google docs", "sheets": "google sheets", "slides": "google slides",
           "one password": "1password"}


def find_app(spoken: str):
    apps = list_apps()
    s = spoken.lower().strip().removesuffix(" app")
    s = ALIASES.get(s, s)
    if s in apps:
        return apps[s]
    starts = [n for n in apps if n.startswith(s) or s.startswith(n)]
    if starts:
        return apps[min(starts, key=len)]
    close = difflib.get_close_matches(s, apps.keys(), n=1, cutoff=0.75)
    return apps[close[0]] if close else None


def load():
    try:
        return json.loads(config.COMMANDS.read_text()).get("commands", [])
    except (OSError, ValueError) as e:
        log.error("commands.json unreadable: %s", e)
        return []


def match(text: str):
    """Returns (command, slots) for the first matching phrase, or None."""
    norm = normalize(text)
    for cmd in load():
        for phrase in cmd.get("phrases", []):
            m = phrase_regex(phrase).match(norm)
            if m:
                return cmd, {k: v for k, v in m.groupdict().items() if v}
    return None


def run(cmd, slots):
    """Runs a matched command. Returns a short result line, or None if it doesn't apply
    (e.g. no such app) so the request goes to Claude instead."""
    fill = lambda s: s.format(**slots) if s else s
    if "open_url" in cmd:
        url = fill(cmd["open_url"])
        if "<org>" in url or "<repo>" in url:
            return "Set your repo in the dashboard (Commands)."
        args = ["open", "-a", cmd["browser"], url] if cmd.get("browser") else ["open", url]
        subprocess.run(args, check=False)
        return f"Opened {cmd.get('name', 'link')}" + (f" #{slots['n']}" if "n" in slots else "")
    if "open_app" in cmd:
        app = find_app(fill(cmd["open_app"]))
        if not app:
            return None
        subprocess.run(["open", "-a", app], check=False)
        return f"Opened {app}."
    if "shell" in cmd:
        r = subprocess.run(fill(cmd["shell"]), shell=True, capture_output=True, text=True, timeout=30)
        return (r.stdout or r.stderr).strip()[:200] or "Done."
    return None
