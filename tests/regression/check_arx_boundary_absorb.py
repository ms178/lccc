#!/usr/bin/env python3
"""Assembly witnesses for vec_arx's boundary absorption (vec_arx.rs).

Three structural gates on the x86-64 BASELINE (-O2, SSE2) build of
chacha20_core, each tied to a Zen 5 store-forwarding property the
absorbers exist for:

  A. element-wise entry: the straight-line prologue (everything before
     the first local label) packs the input with scalar 4-byte loads —
     at least PREFIX_MIN_MOVL `movl mem, reg` — and contains NO vector
     memory operation.  The old shape (16-byte copy loop + narrow
     reloads out of vector stores) had a label-fronted loop and zero
     prologue loads, so this gate fails loudly on regression.
  B. the one-sided overlap guard: `cmpq $57` — (nslots-2)*elem + 1 for
     the 16-word state — must appear exactly once.  No other chacha
     constant collides with it.
  C. the absorbed feed-forward: some straight-line region between two
     labels performs >= 4 packed adds and >= 4 vector stores with no
     branch inside and NO %rsp traffic (the state spill lives on the
     cold scalar path when the state is dead after the loop).

Modes:
  absorbed  - all three gates must hold (the transformed build);
  control   - at least one gate must FAIL (CCC_DISABLE_PASSES=vec_arx
              build): proves the predicates are non-vacuous.
"""

import re
import sys

PREFIX_MIN_MOVL = 12
LABEL = re.compile(r"^\s*(\.?L[\w.$]+):")
VEC_MEM = re.compile(r"\b(v?mov(dqu|dqa|ups|aps))\b[^#]*\(")
MOVL_LOAD = re.compile(r"\bmovl\s+(?:-?\d+)?\(%r[a-z0-9]+\),\s*%")
PACKED_ADD = re.compile(r"\bv?paddd\b")
VEC_STORE = re.compile(r"\bv?mov(dqu|dqa|ups|aps)\s+%[xy]mm\d+,\s*[^,]*\(")
BRANCH = re.compile(r"\b(j[a-z]+|call|ret)\b")


def function_body(lines, name):
    out, inside = [], False
    for line in lines:
        s = line.split("#", 1)[0].rstrip()
        if not inside:
            if s.strip() == f"{name}:":
                inside = True
            continue
        if s.strip().startswith((".size", ".cfi_endproc")):
            break
        out.append(s)
    if not out:
        sys.exit(f"FAIL: function {name!r} not found")
    return out


def gate_a_entry_pack(body):
    movl = vec = 0
    for line in body:
        if LABEL.match(line):
            break
        if MOVL_LOAD.search(line):
            movl += 1
        if VEC_MEM.search(line):
            vec += 1
    ok = movl >= PREFIX_MIN_MOVL and vec == 0
    return ok, f"prologue: {movl} scalar loads (need >= {PREFIX_MIN_MOVL}), {vec} vector memory ops (need 0)"


def gate_b_guard(body):
    n = sum(1 for line in body if re.search(r"\bcmpq\s+\$57,", line))
    return n == 1, f"overlap-guard compare `cmpq $57`: {n} occurrence(s) (need exactly 1)"


def gate_c_fast_feed_forward(body):
    regions, cur = [], []
    for line in body:
        if LABEL.match(line):
            regions.append(cur)
            cur = []
            continue
        cur.append(line)
    regions.append(cur)
    best = None
    for region in regions:
        adds = stores = 0
        straight = True
        rsp = False
        for line in region:
            text = line.strip()
            if not text or text.startswith("."):
                continue
            if BRANCH.search(text) and not text.startswith(("jmp", "ret")):
                straight = False
            if PACKED_ADD.search(text):
                adds += 1
            if VEC_STORE.search(text):
                stores += 1
            if "%rsp" in text:
                rsp = True
        if straight and adds >= 4 and stores >= 4 and not rsp:
            best = (adds, stores)
            break
    if best is None:
        return False, "no straight-line region with >=4 packed adds, >=4 vector stores, and no %rsp traffic"
    return True, f"fast feed-forward region: {best[0]} packed adds, {best[1]} vector stores, no spills"


def main():
    if len(sys.argv) != 4 or sys.argv[2] not in ("absorbed", "control"):
        sys.exit(f"usage: {sys.argv[0]} <asm-file> absorbed|control <function>")
    path, mode, func = sys.argv[1], sys.argv[2], sys.argv[3]
    with open(path, encoding="utf-8", errors="replace") as fh:
        body = function_body(fh.read().splitlines(), func)
    results = [
        ("A entry pack", *gate_a_entry_pack(body)),
        ("B overlap guard", *gate_b_guard(body)),
        ("C fast feed-forward", *gate_c_fast_feed_forward(body)),
    ]
    all_ok = all(ok for _, ok, _ in results)
    for name, ok, detail in results:
        print(f"  [{'ok' if ok else 'MISS'}] {name}: {detail}")
    if mode == "absorbed" and not all_ok:
        sys.exit(f"FAIL: {path}: boundary absorption witnesses missing")
    if mode == "control" and all_ok:
        sys.exit(
            f"FAIL: {path}: control build satisfies every witness - the predicates are vacuous"
        )
    print(f"ok: {path} ({mode})")


if __name__ == "__main__":
    main()
