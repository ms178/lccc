#!/usr/bin/env python3
"""Census of the accepted feature names of `__builtin_cpu_supports` (AUD-CPU-1).

LCCC advertises a *superset* of GCC's accepted name set: every name GCC 16.2
accepts must keep working (dispatch code written for GCC must build here), and
Clang/Intel spellings (`pclmulqdq`, `rdrand`) that GCC rejects are accepted
too because they cost nothing.  Anything else must be a hard compile error,
exactly like GCC — a silently folded `0` is the AUD-SILENT-ZERO-1 disease in a
new place.

The accepted set is derived from the compilers themselves instead of
transcribed documentation:

  * candidate names are batched into one translation unit per compile
    (`int p<i>(void){ return __builtin_cpu_supports("name"); }`),
  * a batch that compiles proves every name in it accepted; a failing batch is
    bisected until the offending names are isolated — chunks are classified in
    parallel (`--jobs`), which is what makes a whole-ISA census take minutes,
  * every remote compile is cached under `.godbolt-cache/census-v1/` keyed by
    (compiler, source, flags), so a re-run after an interruption is free.

`--lccc-src` diffs the census against LCCC's own tables
(`src/common/cpu_probe.rs`, `src/common/isa_features.rs`) and fails the run
when GCC accepts a name LCCC does not.  The CI gate
(`tests/regression/check_cpu_supports_runtime.sh`) uses the recorded census at
`tests/data/cpu_supports_name_census.json` for the same property without any
network access; this tool is how that file is regenerated.

Usage:
    scripts/cpu_supports_name_census.py --compilers gcc16.2,clang,icx -j6 \\
        --json tests/data/cpu_supports_name_census.json
    scripts/cpu_supports_name_census.py --compilers gcc16.2 --lccc-src .
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import threading
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import godbolt  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
CACHE = Path(os.environ.get("GODBOLT_CACHE", str(REPO / ".godbolt-cache"))) / "census-v1"
_lock = threading.Lock()

# Candidate spellings.  Three kinds of entries:
#   * names GCC is known to accept (must stay accepted),
#   * Clang/Intel spellings and underscore variants (accepted by LCCC by
#     design; rejections by GCC are recorded, not failures),
#   * plausible-but-nonexistent names as positive controls for the bisection.
CANDIDATES: list[str] = [
    # integer / pre-SSE
    "cmov", "mmx", "sse", "sse2", "sse3", "ssse3", "sse4.1", "sse4.2",
    "sse4a", "sse4_1", "sse4_2", "pni", "popcnt", "movbe", "cx16",
    "cmpxchg16b", "lahf-lm", "lahf_lm", "lzcnt", "abm", "prefetchw",
    "prfchw", "fxsr", "clflush", "clflushopt", "clwb", "clzero",
    "wbnoinvd", "cldemote", "movdiri", "movdir64b", "enqcmd", "rdpid",
    "rdrnd", "rdrand", "rdseed", "rtm", "hle", "tsxldtrk", "serialize",
    "hreset", "pconfig", "uintr", "waitpkg", "ptwrite", "kl", "shstk",
    "sgx", "pku", "ospke", "pks", "movrs", "bus-lock-detect",
    # crypto
    "aes", "pclmul", "pclmulqdq", "vpclmulqdq", "sha", "sha_ni", "sha512",
    "sm3", "sm4", "gfni", "vaes", "aeskl", "keylocker",
    # FP / vector
    "f16c", "fma", "fma4", "xop", "tbm", "avx", "avx2", "avxvnni",
    "avxvnniint8", "avxvnniint16", "avxifma", "avxneconvert", "avx512f",
    "avx512cd", "avx512dq", "avx512bw", "avx512vl", "avx512ifma",
    "avx512vbmi", "avx512vbmi2", "avx512vnni", "avx512bitalg",
    "avx512vpopcntdq", "avx512vpopcntdqvl", "avx512bf16", "avx512fp16",
    "avx512er", "avx512pf", "avx512vp2intersect", "avx5124vnniw",
    "avx5124fmaps", "avx10.1-256", "avx10.1", "avx10.2",
    # xstate
    "xsave", "xsaveopt", "xsavec", "xsaves", "osxsave", "xgetbv1",
    # AMX
    "amx-tile", "amx-int8", "amx-bf16", "amx-fp16", "amx-complex",
    "amx_tile", "amx_int8", "amx_bf16", "amx_fp16",
    # AMD legacy and platform bits
    "3dnow", "3dnowprefetch", "3dnowext", "lwp", "mwaitx", "monitor",
    "mwait", "svm", "vmx", "avic", "npt", "ept", "smap", "smep", "invpcid",
    "la57", "5levelpaging", "gds", "pta", "crc32", "pcommit", "erms",
    "rdpru", "cmpccxadd", "raoint", "rng", "amx-vnni",
    # constructive controls: must be rejected by every compiler
    "not-a-real-feature", "avx9", "sse5",
]

DEFAULT_COMPILERS = ["gcc16.2", "clang", "icx", "icc"]

# Compilers whose accepted set CI enforces as a *lower bound* on LCCC's set.
# GCC defines the builtin's name space, so a name GCC accepts that LCCC
# rejects breaks code written against GCC.  Clang accepts every string
# (unmodelled names fold to 0), so its set is informational only; ICX/ICC are
# recorded for reference.
STRICT_COMPILERS = ["gcc16.2"]

_ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")


def clean_diagnostic(text: str) -> str:
    """Drop ANSI colour and keep the compiler's own message lines."""
    plain = _ANSI_RE.sub("", text)
    keep = [ln.strip() for ln in plain.splitlines()
            if ("error:" in ln or "warning:" in ln) and "<source>" in ln]
    if not keep:
        keep = [ln.strip() for ln in plain.splitlines() if "error:" in ln]
    return "\n".join(keep)[:400]


def batch_source(names: list[str]) -> str:
    lines = ["/* generated by scripts/cpu_supports_name_census.py */", "#include <stdio.h>"]
    for i, name in enumerate(names):
        lines.append(f'int probe_{i}(void) {{ return __builtin_cpu_supports("{name}"); }}')
    return "\n".join(lines) + "\n"


def _cache_path(compiler: str, names: list[str], flags: str) -> Path:
    digest = hashlib.sha256()
    digest.update(compiler.encode())
    digest.update(b"\0")
    digest.update(flags.encode())
    digest.update(b"\0")
    digest.update(batch_source(names).encode())
    return CACHE / (digest.hexdigest()[:32] + ".json")


def compile_ok(compiler: str, names: list[str], flags: str) -> tuple[bool, str]:
    """Compile a batch (or a single name); report acceptance + diagnostic text."""
    path = _cache_path(compiler, names, flags)
    if path.exists():
        record = json.loads(path.read_text(encoding="utf-8"))
        return record["ok"], record["message"]
    # Serialise remote work: the executor is shared and rate-limited; the
    # cache turns this into a one-time cost per tuple.
    with _lock:
        if path.exists():
            record = json.loads(path.read_text(encoding="utf-8"))
            return record["ok"], record["message"]
        import contextlib
        import io

        buf = io.StringIO()
        with contextlib.redirect_stderr(buf):
            result = godbolt.compile_on_godbolt(compiler, batch_source(names), flags)
        ok = result is not None
        message = buf.getvalue().strip()
        path.parent.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps({"ok": ok, "message": message, "compiler": compiler,
                                   "names": names, "flags": flags}, indent=1) + "\n",
                       encoding="utf-8")
        os.replace(tmp, path)
    return ok, message


def classify(compiler: str, names: list[str], flags: str, depth: int = 0) -> dict[str, bool]:
    """{name: accepted} for a batch, bisecting failing batches."""
    if not names:
        return {}
    ok, _ = compile_ok(compiler, names, flags)
    if ok:
        return dict.fromkeys(names, True)
    if len(names) == 1:
        return {names[0]: False}
    if depth > 64:
        raise RuntimeError(f"bisection did not terminate at {names!r}")
    mid = len(names) // 2
    result = classify(compiler, names[:mid], flags, depth + 1)
    result.update(classify(compiler, names[mid:], flags, depth + 1))
    return result


def classify_chunk(compiler: str, chunk: list[str], flags: str) -> dict[str, bool]:
    return classify(compiler, chunk, flags)


def census_compiler(compiler: str, names: list[str], flags: str, jobs: int,
                    diagnostics: bool) -> dict[str, object]:
    chunk = max(2, (len(names) + jobs - 1) // jobs)
    chunks = [names[i:i + chunk] for i in range(0, len(names), chunk)]
    accepted: dict[str, bool] = {}
    with ThreadPoolExecutor(max_workers=jobs) as pool:
        for result in pool.map(lambda c: classify_chunk(compiler, c, flags), chunks):
            accepted.update(result)
    rejects: dict[str, str] = {}
    if diagnostics:
        for name in sorted(n for n, ok in accepted.items() if not ok):
            _, message = compile_ok(compiler, [name], flags)
            rejects[name] = clean_diagnostic(message)
    return {
        "accepted": sorted(n for n, ok in accepted.items() if ok),
        "rejected": sorted(n for n, ok in accepted.items() if not ok),
        "diagnostics": rejects,
    }


# An entry is `"name", Probe::new(...)`.  The pattern is deliberately blind to
# the layout around it: `cargo fmt` is a required gate, and rustfmt splits any
# entry that does not fit on one line *before* the name, turning
# `("avx512pf", Probe::new(...))` into
#     (
#         "avx512pf",
#         Probe::new(...),
#     ),
# The previous `"("name", \s*Probe::new` pattern required the opening paren
# directly before the quote, so 19 AVX-512 names silently vanished from the
# census and the CI gate reported "source drift" on a tree whose source had not
# changed semantically.  A layout-sensitive extractor feeding a drift ratchet is
# a false-alarm generator; this one ignores whitespace and the paren.
LCCC_PROBE_ENTRY_RE = re.compile(r'"([^"]+)"\s*,\s*Probe::new\s*\(')


def lccc_names(src_root: Path) -> tuple[set[str], set[str]]:
    """LCCC's runtime-probe names (canonical + aliases) and static names.

    Each table is extracted from its own body, so a name mentioned in a doc
    comment or in another table cannot enter the set.
    """
    probe = (src_root / "src/common/cpu_probe.rs").read_text(encoding="utf-8")
    table = probe.split("pub const PROBE_TABLE", 1)[-1].split("\n];", 1)[0]
    canonical = set(LCCC_PROBE_ENTRY_RE.findall(table))
    alias_block = probe.split("pub const NAME_ALIASES", 1)[1] if "pub const NAME_ALIASES" in probe else ""
    aliases = {alias for alias, _canonical in
               re.findall(r'\("([^"]+)",\s*"([^"]+)"\s*\)', alias_block)}
    static = (src_root / "src/common/isa_features.rs").read_text(encoding="utf-8")
    body = static.split("pub const FEATURE_TABLE", 1)[1].split("];", 1)[0]
    static_names = set(re.findall(r'"([^"]+)"', body))
    return canonical | aliases, static_names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--compilers", default=",".join(DEFAULT_COMPILERS))
    parser.add_argument("--flags", default="-O0 -c")
    parser.add_argument("--names", default=None,
                        help="file with one candidate name per line (overrides the list)")
    parser.add_argument("--json", default=None, help="write the census JSON here")
    parser.add_argument("--lccc-src", default=None, help="LCCC root; diff the tables")
    parser.add_argument("--verify-json", default=None,
                        help="offline: check a recorded census against the "
                             "current source tables (used by the CI gate)")
    parser.add_argument("-j", "--jobs", type=int, default=4,
                        help="parallel remote compiles (default: %(default)s)")
    parser.add_argument("--diagnostics", action="store_true",
                        help="recompile each rejected name alone to record its message")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)

    names = (Path(args.names).read_text(encoding="utf-8").split()
             if args.names else list(CANDIDATES))
    seen: set[str] = set()
    names = [n for n in names if not (n in seen or seen.add(n))]

    if args.verify_json:
        status = 0
        recorded = json.loads(Path(args.verify_json).read_text(encoding="utf-8"))
        root = Path(args.lccc_src or ".")
        probe_names, static_names = lccc_names(root)
        recorded_probe = set(recorded.get("lccc_names", {}).get("probe", []))
        if recorded_probe and recorded_probe != probe_names:
            print("CENSUS DRIFT: recorded lccc probe names differ from the source",
                  file=sys.stderr)
            for name in sorted(probe_names - recorded_probe):
                print(f"    only in source: {name}", file=sys.stderr)
            for name in sorted(recorded_probe - probe_names):
                print(f"    only in census: {name}", file=sys.stderr)
            status = 1
        for compiler in recorded.get("strict", STRICT_COMPILERS):
            data = recorded.get("compilers", {}).get(compiler)
            if data is None:
                print(f"CENSUS MISSING compiler section {compiler!r}", file=sys.stderr)
                status = 1
                continue
            missing = sorted(set(data["accepted"]) - probe_names)
            if missing:
                print(f"{compiler}: recorded accepted names LCCC no longer takes: "
                      + ", ".join(missing), file=sys.stderr)
                status = 1
        if status == 0:
            print(f"recorded census verified against src/common/cpu_probe.rs "
                  f"({len(probe_names)} names)")
        return status
    census: dict[str, dict[str, object]] = {}
    for compiler in [c.strip() for c in args.compilers.split(",") if c.strip()]:
        data = census_compiler(compiler, names, args.flags, args.jobs, args.diagnostics)
        census[compiler] = data
        accepted, rejected = data["accepted"], data["rejected"]
        assert isinstance(accepted, list) and isinstance(rejected, list)
        print(f"{compiler}: {len(accepted)} accepted, {len(rejected)} rejected", file=sys.stderr)
        if args.verbose:
            print("  accepted: " + " ".join(accepted), file=sys.stderr)
            print("  rejected: " + " ".join(rejected), file=sys.stderr)

    status = 0
    if args.lccc_src:
        root = Path(args.lccc_src)
        probe_names, static_names = lccc_names(root)
        for compiler, data in census.items():
            accepted = set(data["accepted"])  # type: ignore[arg-type]
            missing = sorted(accepted - probe_names)
            extra = sorted(probe_names - accepted)
            print(f"{compiler}: lccc probe names {len(probe_names)}; "
                  f"GCC-accepted-not-in-LCCC {len(missing)}; LCCC-only {len(extra)}")
            if missing:
                print("  MISSING (GCC accepts, LCCC must too):", file=sys.stderr)
                for name in missing:
                    print(f"    {name}", file=sys.stderr)
                status = 1
            if extra:
                print("  lccc-only extras (informational): " + ", ".join(extra))
        uncovered = sorted(n for n in static_names if n not in probe_names)
        if uncovered:
            print("STATIC TABLE NAMES WITHOUT RUNTIME PROBE:", file=sys.stderr)
            for name in uncovered:
                print(f"    {name}", file=sys.stderr)
            status = 1
        else:
            print(f"static table: {len(static_names)} names, all covered by the runtime probe")

    if args.json:
        out = Path(args.json)
        out.parent.mkdir(parents=True, exist_ok=True)
        header = {
            "tool": "scripts/cpu_supports_name_census.py",
            "flags": args.flags,
            "strict": [c for c in STRICT_COMPILERS if c in census],
            "candidates": names,
            "lccc_names": {
                "probe": sorted(lccc_names(Path(args.lccc_src or "."))[0]) if (
                    Path(args.lccc_src or ".") / "src/common/cpu_probe.rs"
                ).exists() else [],
            },
            "compilers": census,
        }
        out.write_text(json.dumps(header, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"census written to {out}", file=sys.stderr)
    return status


if __name__ == "__main__":
    sys.exit(main())
