"""Runs a bench command under `/usr/bin/time -l` and merges peak memory into its JSON output.

usage: python3 with_mem.py OUT.json -- command args...   (the command must write OUT.json)
"""
import json
import re
import subprocess
import sys

out = sys.argv[1]
cmd = sys.argv[sys.argv.index("--") + 1:]
p = subprocess.run(["/usr/bin/time", "-l", *cmd], stderr=subprocess.PIPE, text=True)
sys.stderr.write("\n".join(l for l in p.stderr.splitlines() if not re.match(r"\s+\d+\s", l)) + "\n")
if p.returncode:
    sys.exit(p.returncode)
rss = int(re.search(r"(\d+)\s+maximum resident set size", p.stderr).group(1))
foot = int(re.search(r"(\d+)\s+peak memory footprint", p.stderr).group(1))
wall = float(re.search(r"([\d.]+) real", p.stderr).group(1))
r = json.load(open(out))
r["mem"] = {"max_rss_mb": round(rss / 2**20), "peak_footprint_mb": round(foot / 2**20)}
r["process_wall_s"] = wall
json.dump(r, open(out, "w"), indent=1)
print(f"{r['engine']}: rss {r['mem']['max_rss_mb']} MB, footprint {r['mem']['peak_footprint_mb']} MB, wall {wall}s")
