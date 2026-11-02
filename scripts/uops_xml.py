#!/usr/bin/env python3
"""Offline index over the uops.info `instructions.xml` measurement database.

`scripts/uops_info_probe.py` scrapes individual HTML pages; this module reads
the *complete* published dataset (Abel & Reineke, ASPLOS 2019, refreshed by the
site; ~140 MB XML) once, builds a compact pickle index, and answers queries in
milliseconds afterwards.  It is the data backend of
`scripts/cpu_model_audit.py` (table-vs-measurement cross-check) and is usable
stand-alone:

    $ scripts/uops_xml.py query LEA_B_I_D8_R64 --arch ICL,ADL-P,ADL-E
    $ scripts/uops_xml.py grep '^VFMADD231PS_YMM' --arch SKL,ADL-P

Index key is the uops.info page name (`html-instr/<NAME>.html`), e.g.
`IMUL_R64_R64`, `SHL_R64_CL`, `VADDPS_YMM_YMM_YMM`.  For every architecture the
index stores: `uops`, `ports`, `tp_loop`, `tp_unrolled`, `tp_ports`, and the
latency edges `(start_op, target_op, cycles, same_reg, min, max, addr, mem)`.

The XML is fetched from https://uops.info/instructions.xml on first use and
cached in `$XDG_CACHE_HOME/lccc-uops/` (default `~/.cache/lccc-uops/`); pass
`--xml PATH` to use a local copy (offline sessions, CI).
"""
from __future__ import annotations

import argparse
import os
import pickle
import re
import sys
import urllib.request
import xml.etree.ElementTree as ET

URL = "https://uops.info/instructions.xml"
CACHE = os.path.join(
    os.environ.get("XDG_CACHE_HOME", os.path.expanduser("~/.cache")), "lccc-uops"
)
XML_PATH = os.path.join(CACHE, "instructions.xml")
IDX_PATH = os.path.join(CACHE, "index.pkl")

# Model row (`X86Tune::name`) -> uops.info measurement column.  Raptor Lake has
# no column of its own: Raptor Cove is the Golden Cove core, so ADL-P *is* the
# measurement (the model's module docs list the deltas that are not
# instruction timings: caches, E-core count, clocks).
ROW_ARCH = {
    "meteorlake": "MTL-P", "sandybridge": "SNB", "ivybridge": "IVB", "haswell": "HSW",
    "broadwell": "BDW", "skylake": "SKL", "skylake-avx512": "SKX",
    "icelake-client": "ICL", "alderlake": "ADL-P", "raptorlake": "ADL-P",
    "sapphirerapids": "EMR", "arrowlake": "ARL-P", "gracemont": "ADL-E",
    "znver1": "ZEN+", "znver2": "ZEN2", "znver3": "ZEN3", "znver4": "ZEN4",
    "znver5": "ZEN5",
}


def _fetch_xml(path: str) -> None:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    sys.stderr.write(f"fetching {URL} -> {path}\n")
    with urllib.request.urlopen(URL, timeout=300) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    os.replace(tmp, path)


def _page_name(url: str) -> str | None:
    m = re.search(r"html-instr/([^/]+)\.html", url or "")
    return m.group(1) if m else None


def build_index(xml_path: str) -> dict:
    idx: dict[str, dict[str, dict]] = {}
    for _, el in ET.iterparse(xml_path, events=("end",)):
        if el.tag != "instruction":
            continue
        name = _page_name(el.get("url", ""))
        if name:
            per: dict[str, dict] = {}
            for arch in el.iter("architecture"):
                m = arch.find("measurement")
                if m is None:
                    continue
                lats = []
                for l in m.findall("latency"):
                    g = l.get
                    lats.append((
                        int(g("start_op", 0)), int(g("target_op", 0)),
                        g("cycles"), g("cycles_same_reg"), g("min_cycles"),
                        g("max_cycles"), g("cycles_addr"), g("cycles_mem"),
                    ))
                per[arch.get("name")] = {
                    "uops": m.get("uops"), "ports": m.get("ports"),
                    "tp_loop": m.get("TP_loop"),
                    "tp_unrolled": m.get("TP_unrolled"),
                    "tp_ports": m.get("TP_ports"),
                    "lat": lats,
                }
            if per:
                idx[name] = per
        el.clear()
    return idx


def load(xml: str | None = None) -> dict:
    if xml is None and os.path.exists(IDX_PATH) and (
        not os.path.exists(XML_PATH)
        or os.path.getmtime(IDX_PATH) >= os.path.getmtime(XML_PATH)
    ):
        with open(IDX_PATH, "rb") as f:
            return pickle.load(f)
    path = xml or XML_PATH
    if not os.path.exists(path):
        _fetch_xml(path)
    idx = build_index(path)
    if xml is None:
        os.makedirs(CACHE, exist_ok=True)
        tmp = IDX_PATH + ".tmp"
        with open(tmp, "wb") as f:
            pickle.dump(idx, f, protocol=4)
        os.replace(tmp, IDX_PATH)
    return idx


def _f(v):
    return int(float(v))


def lat_range(entry: dict, reg_only: bool = True):
    """(lo, hi) over the register data edges (memory-address edges skipped)."""
    lo, hi = [], []
    for (_s, _t, c, _sr, mn, mx, _a, _m) in entry["lat"]:
        if c is not None:
            lo.append(_f(c)); hi.append(_f(c))
        if mn is not None:
            lo.append(_f(mn))
        if mx is not None:
            hi.append(_f(mx))
    return (min(lo), max(hi)) if lo and hi else None


def lat(entry: dict):
    r = lat_range(entry)
    return None if r is None else r[1]


def lat_same_reg(entry: dict):
    """Latency of the same-register form (e.g. dependency-breaking idioms)."""
    vals = [_f(sr) for (_s, _t, _c, sr, *_r) in entry["lat"] if sr is not None]
    return max(vals) if vals else None


def tp(entry: dict):
    for k in ("tp_ports", "tp_loop", "tp_unrolled"):
        if entry.get(k):
            return float(entry[k])
    return None


def nuops(entry: dict):
    try:
        return int(entry["uops"])
    except (TypeError, ValueError):
        return None


def fmt(e: dict) -> str:
    r = lat_range(e)
    ls = "?" if r is None else (str(r[0]) if r[0] == r[1] else f"{r[0]}-{r[1]}")
    t = tp(e)
    return (f"lat={ls:<7} tp={'?' if t is None else t:<6} "
            f"uops={e['uops']!s:<3} ports={e['ports']}")


def main() -> int:
    ap = argparse.ArgumentParser(description="uops.info XML index")
    ap.add_argument("cmd", choices=["query", "grep", "build"])
    ap.add_argument("pattern", nargs="?")
    ap.add_argument("--arch", default="SNB,SKL,ICL,ADL-P,ADL-E,ZEN4")
    ap.add_argument("--xml")
    a = ap.parse_args()
    idx = load(a.xml)
    if a.cmd == "build":
        print(f"{len(idx)} instruction pages indexed")
        return 0
    arches = a.arch.split(",")
    names = ([a.pattern] if a.cmd == "query"
             else sorted(n for n in idx if re.search(a.pattern, n)))
    for n in names:
        print(f"== {n}")
        for ar in arches:
            e = idx.get(n, {}).get(ar)
            print(f"  {ar:<6} {fmt(e) if e else '-'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
