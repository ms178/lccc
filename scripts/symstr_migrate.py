#!/usr/bin/env python3
"""Span-driven migration helper for the `String` -> `SymStr` change.

Swapping `Elf64Symbol::name` from `String` to `SymStr` produces ~150 type
errors across four backends. They fall into a handful of purely mechanical
classes, so rather than hand-editing (error-prone, and impossible to review)
this walks rustc's JSON diagnostics and rewrites the exact byte spans rustc
points at, then re-runs the compiler until it reaches a fixed point.

Every rewrite is *semantics preserving by construction*: it only inserts
`.as_str()`, `.to_string()`, or `SymStr::new(...)` conversions at sites where
the compiler has already proven the types disagree. Anything it does not
recognise is reported and left alone for a human.

Usage:  scripts/symstr_migrate.py [--apply] [--max-rounds N]
        (default is a dry run that just classifies the errors)
"""
import json
import re
import subprocess
import sys
from collections import Counter

CARGO = ["cargo", "build", "--profile", "fastbuild", "--locked", "-j2",
         "--message-format=json-diagnostic-rendered-ansi"]


def diagnostics():
    """Yield (file, byte_start, byte_end, code, message, expected, found)."""
    p = subprocess.run(CARGO, capture_output=True, text=True)
    out = []
    for line in p.stdout.splitlines():
        try:
            msg = json.loads(line)
        except json.JSONDecodeError:
            continue
        if msg.get("reason") != "compiler-message":
            continue
        d = msg["message"]
        if d.get("level") != "error":
            continue
        code = (d.get("code") or {}).get("code", "")
        prim = [s for s in d.get("spans", []) if s.get("is_primary")]
        if not prim:
            continue
        s = prim[0]
        if not s["file_name"].startswith("src/"):
            continue
        out.append({
            "file": s["file_name"],
            "start": s["byte_start"],
            "end": s["byte_end"],
            "text": s.get("text", [{}])[0].get("text", ""),
            "code": code,
            "msg": d.get("message", ""),
            "label": s.get("label") or "",
            "children": [c.get("message", "") for c in d.get("children", [])],
        })
    return out


# `expected `X`, found `Y`` -> ("X", "Y")
EXPFOUND = re.compile(r"expected `([^`]+)`, found `([^`]+)`")


def classify(d, src):
    """Return the replacement string for this diagnostic's span, or None."""
    expr = src[d["start"]:d["end"]]

    # Hard safety guard. rustc sometimes reports a span that stretches across
    # a line break and swallows an intervening comment or struct field (e.g.
    # `name: sym.name` in a literal whose next field is preceded by a `//`
    # comment). Rewriting such a span splices code into the middle of a
    # comment and produces unbalanced delimiters. Those sites are rare and
    # must be fixed by hand, so refuse anything that is not a single-line,
    # comment-free expression.
    if "\n" in expr or "//" in expr or "/*" in expr:
        return None
    blob = d["msg"] + " " + d["label"] + " " + " ".join(d["children"])

    # Class 1: map/set probed with `&sym.name` where the key is still String.
    # `Borrow<SymStr>` is not implemented for String, but Borrow<str> is, so
    # hand the container a plain &str.
    if d["code"] == "E0277" and "Borrow<SymStr>" in blob:
        inner = expr[1:] if expr.startswith("&") else expr
        return f"{inner}.as_str()"

    # Class 2: a SymStr flowing into a String slot.
    m = EXPFOUND.search(blob)
    if m:
        exp, found = m.group(1), m.group(2)
        if exp == "String" and found == "SymStr":
            # `x.name.clone()` -> `x.name.to_string()` reads better than
            # `x.name.clone().to_string()` and avoids a double copy.
            if expr.endswith(".clone()"):
                return expr[: -len(".clone()")] + ".to_string()"
            return f"{expr}.to_string()"
        if exp == "SymStr" and found == "String":
            if expr.endswith(".clone()"):
                return f"SymStr::new(&{expr[: -len('.clone()')]})"
            return f"SymStr::new(&{expr})"
        if exp == "&str" and found == "&SymStr":
            inner = expr[1:] if expr.startswith("&") else expr
            return f"{inner}.as_str()"
        if exp == "&SymStr" and found == "&str":
            return f"&SymStr::new({expr[1:] if expr.startswith('&') else expr})"
        if exp == "SymStr" and found == "&str":
            return f"SymStr::new({expr})"
        if exp == "&String" and found == "&SymStr":
            # Callee wants &String; the only zero-risk rewrite is to hand it a
            # &str-shaped API instead, so this is left to the per-callee fixups
            # below unless the expression is a plain borrow we can deref.
            inner = expr[1:] if expr.startswith("&") else expr
            return f"&{inner}.to_string()"
        if exp == "Option<String>" and found == "Option<SymStr>":
            return f"{expr}.map(|s| s.to_string())"
    return None


def main():
    apply = "--apply" in sys.argv
    max_rounds = 60
    if "--max-rounds" in sys.argv:
        max_rounds = int(sys.argv[sys.argv.index("--max-rounds") + 1])

    for rnd in range(1, max_rounds + 1):
        diags = diagnostics()
        if not diags:
            print(f"round {rnd}: clean build, no errors")
            return 0

        # Group by file and apply from the end so earlier byte offsets stay valid.
        by_file = {}
        unknown = Counter()
        for d in diags:
            by_file.setdefault(d["file"], []).append(d)

        # Apply at most ONE file per round.
        #
        # rustc emits byte offsets computed against the file as it was when the
        # crate was parsed. Within a single round those offsets are consistent,
        # but only for files we have not touched yet -- and cargo may reuse a
        # cached diagnostic for a file whose on-disk contents changed in the
        # previous round, so its spans point into the *old* text. Splicing at
        # those offsets silently corrupts string literals and comments. Editing
        # one file per round and re-invoking the compiler guarantees every span
        # we act on was computed from the exact bytes now on disk.
        total = 0
        for path, ds in by_file.items():
            if total:
                break  # one file per round; see comment above
            raw = open(path, "rb").read()
            src = raw.decode("utf-8")
            # byte offsets from rustc index the raw bytes
            edits = []
            for d in ds:
                # Cross-check: rustc echoes the source line it complained
                # about. If the bytes at the reported offsets are not a
                # substring of that line, the span is stale -- skip it.
                actual = src[d["start"]:d["end"]]
                if d["text"] and actual and actual not in d["text"]:
                    unknown[f'STALE SPAN in {path}: {actual[:40]!r}'] += 1
                    continue
                rep = classify(d, raw.decode("utf-8", "surrogateescape"))
                if rep is None:
                    unknown[f'{d["code"]} {d["msg"][:70]}'] += 1
                    continue
                edits.append((d["start"], d["end"], rep))
            # Deduplicate identical spans (rustc can report a span twice).
            edits = sorted(set(edits), key=lambda e: -e[0])
            # Drop overlapping spans; the next round will catch them.
            kept, last = [], None
            for st, en, rep in edits:
                if last is not None and en > last:
                    continue
                kept.append((st, en, rep))
                last = st
            if not kept:
                continue
            b = bytearray(raw)
            for st, en, rep in kept:
                b[st:en] = rep.encode("utf-8")
            total += len(kept)
            if apply:
                open(path, "wb").write(bytes(b))

        print(f"round {rnd}: {len(diags)} errors, {total} auto-fixable"
              + ("" if apply else " (dry run)"))
        for k, v in unknown.most_common(10):
            print(f"    UNHANDLED x{v}: {k}")
        if not apply:
            return 0
        if total == 0:
            print("no further progress; remaining errors need a human")
            return 1
    print("hit round limit")
    return 1


if __name__ == "__main__":
    sys.exit(main())
