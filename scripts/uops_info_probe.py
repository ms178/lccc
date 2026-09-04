#!/usr/bin/env python3
"""Re-read instruction latency / throughput / µop facts from uops.info.

The CPU tuning model (`src/backend/x86/cpu_model.rs`) stores measured numbers
with a provenance tag.  This script re-fetches the per-instruction pages from
uops.info (Abel & Reineke, ASPLOS 2019) so a reviewer can re-verify any `[uops.info]`
field without trusting the comment, and prints one compact row per
microarchitecture:

    $ scripts/uops_info_probe.py LEA_B_I_D8_R64 SHL_R64_CL IMUL_R64_R64
    == LEA_B_I_D8_R64
      SNB    lat=3      tp=1.00  uops=1   ports=1*p1
      ADL-P  lat=1      tp=0.20  uops=1   ports=1*p0156B
      ADL-E  lat=2      tp=1.00  uops=1   ports=...

Instruction page names follow uops.info's `html-instr/<NAME>.html` scheme
(mnemonic, operand kinds joined by `_`).  Pages are cached in
`$XDG_CACHE_HOME/lccc-uops` (default `~/.cache/lccc-uops`) so repeated runs
and offline sessions do not hammer the site.

Options:
  --arch A[,B,...]   restrict to these uops.info architecture keys
  --json             emit machine-readable JSON instead of the table
  --no-cache         always re-fetch
"""
import argparse
import html
import json
import os
import re
import sys
import urllib.request

DEFAULT_ARCHES = [
    "SNB", "IVB", "HSW", "BDW", "SKL", "SKX", "CLX", "ICL", "TGL", "RKL",
    "ADL-P", "ADL-E", "MTL-P", "MTL-E", "EMR", "ARL-P", "ARL-E",
    "ZEN+", "ZEN2", "ZEN3", "ZEN4", "ZEN5",
]


def cache_dir():
    base = os.environ.get("XDG_CACHE_HOME") or os.path.expanduser("~/.cache")
    d = os.path.join(base, "lccc-uops")
    os.makedirs(d, exist_ok=True)
    return d


def fetch(name, use_cache=True):
    path = os.path.join(cache_dir(), name + ".html")
    if use_cache and os.path.exists(path):
        return open(path, encoding="utf-8", errors="replace").read()
    url = f"https://uops.info/html-instr/{name}.html"
    req = urllib.request.Request(url, headers={"User-Agent": "lccc-uops-probe/1.0"})
    with urllib.request.urlopen(req, timeout=30) as r:
        data = r.read().decode("utf-8", errors="replace")
    with open(path, "w", encoding="utf-8") as f:
        f.write(data)
    return data


def strip(s):
    return re.sub(r"\s+", " ", html.unescape(re.sub(r"<[^>]+>", "", s))).strip()


def parse(page, arches):
    """Return {arch: {lat: [..], tp: str, uops: str, ports: str}}."""
    out = {}
    # Each architecture section starts with <h2 id="ARCH">.
    sections = re.split(r'<hr><h2 id="', page)[1:]
    for sec in sections:
        arch = sec.split('"', 1)[0]
        if arch not in arches:
            continue
        info = {"lat": [], "tp": None, "uops": None, "ports": None}
        for m in re.finditer(r"Latency operand ([^<]*?):</a>\s*([^<]+)</li>", sec):
            info["lat"].append((strip(m.group(1)), strip(m.group(2))))
        m = re.search(r"Computed from the port usage:\s*([0-9.]+)", sec)
        if m:
            info["tp"] = m.group(1)
        m = re.search(r"Measurements\.html#tp[^>]*>\s*(?:Measured|Throughput)[^<]*</a>\s*([0-9.]+)", sec)
        if m and not info["tp"]:
            info["tp"] = m.group(1)
        m = re.search(r"(?:µops|uops)\s*(?:\(retire slots\))?:?\s*</?[^>]*>?\s*([0-9]+)", sec)
        if m:
            info["uops"] = m.group(1)
        m = re.search(r"Port usage(?:</a>)?:?\s*(?:<[^>]+>\s*)*([0-9]+\*p[0-9A-Z]+(?:\+[0-9]+\*p[0-9A-Z]+)*)", sec)
        if m:
            info["ports"] = m.group(1)
        out[arch] = info
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("instructions", nargs="+")
    ap.add_argument("--arch", default=",".join(DEFAULT_ARCHES))
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--no-cache", action="store_true")
    a = ap.parse_args()
    arches = [x for x in a.arch.split(",") if x]
    result = {}
    for name in a.instructions:
        try:
            page = fetch(name, use_cache=not a.no_cache)
        except Exception as e:  # noqa: BLE001
            print(f"== {name}: fetch failed: {e}", file=sys.stderr)
            continue
        result[name] = parse(page, arches)
        if not a.json:
            print(f"== {name}")
            for arch in arches:
                if arch not in result[name]:
                    continue
                i = result[name][arch]
                lat = ",".join(v for _, v in i["lat"]) or "-"
                print(f"  {arch:6} lat={lat:10} tp={i['tp'] or '-':6} uops={i['uops'] or '-':4} ports={i['ports'] or '-'}")
    if a.json:
        json.dump(result, sys.stdout, indent=2)
        print()


if __name__ == "__main__":
    main()
