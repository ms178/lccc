#!/usr/bin/env python3
"""Cross-vendor oracle: lccc vs GCC, Clang and ICX on Compiler Explorer.

WHY THIS EXISTS.  `oracle_cmp.py` compares lccc-i686 against the *local*
`gcc -m32` by counting instructions.  That answers one question -- is the
generated code big? -- using one reference, on one target, and it says
nothing about whether the code is CORRECT.  A miscompilation that is
perfectly small still shows up as a win.

This harness asks the stronger question first and the smaller one second:

  1. SEMANTICS.  The program is compiled by lccc locally and by each
     remote oracle on godbolt.org, then ALL OF THEM ARE EXECUTED and their
     stdout and exit status compared.  Divergence is a miscompilation, and
     no amount of instruction-count agreement excuses it.  This is the
     oracle that matters, and it is the one a static diff cannot supply.

  2. CODE SIZE.  Instruction counts and `.text` bytes, computed the same
     way for every compiler (from `-S` output, so the comparison is
     apples-to-apples rather than mixing a local objdump against remote
     text).  Reported as data, never as a pass/fail gate: "smaller" is not
     a specification, and a compiler that wins this table by miscompiling
     has lost the table that matters.

THE ORACLES.  Defaults are the newest of each independent vendor:

    cg162        GCC 16.2        (GNU)
    cclang2310   Clang 23.1.0    (LLVM)
    cicxlatest   ICX latest      (Intel -- icx, not the legacy icc)

Compiler Explorer is treated as EVIDENCE, never as authority.  A remote
compiler that rejects a program, times out or is unavailable is reported
as SKIP with its reason; it is never allowed to pass silently, and a
lccc-side divergence is always a FAIL regardless of what the oracle says.
That distinction is deliberate: the GOT64 defect in
`docs/PR663_CI_REPAIR_AND_FOLLOWUP.md` §2 was a case where the reference
linker was itself wrong, and an oracle-agreement gate would have passed it.

USAGE
    godbolt_oracle.py --list
    godbolt_oracle.py                       # every program, every oracle
    godbolt_oracle.py --filter swizzle
    godbolt_oracle.py --oracles cg162,cclang2310 --json out.json
    godbolt_oracle.py --semantics-only

Exit status is 0 only when every program that ran agreed semantically on
every oracle that ran AND at least one oracle ran.  Zero oracles is an
error, not a pass: an empty table must never read as a green run.
"""

from __future__ import annotations

from pathlib import Path

import argparse
import concurrent.futures
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request

CE = "https://godbolt.org/api"

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
LCCC = os.environ.get("LCCC_BIN", os.path.join(REPO, "target/fastbuild/lccc"))
PROGRAMS = os.path.join(REPO, "tests/oracle/programs")

# id -> label.  Overridden by --oracles; ORACLE_SET selects a preset.
ORACLE_SET = {
    "default": ["cg162", "cclang2310", "cicxlatest"],
    "gnu": ["cg162"],
    "llvm": ["cclang2310"],
    "intel": ["cicxlatest"],
    "all-vendors": ["cg162", "cclang2310", "cicxlatest"],
}
VENDOR = {"cg162": "GNU", "cclang2310": "LLVM", "cicxlatest": "Intel"}

# An x86 mnemonic at the start of a line in `as -S` output.  Deliberately
# narrow: labels, directives, comments and .cfi_* lines must not count.
INSN = re.compile(r"^\s+[a-z][a-z0-9.]*\b")

# A full sweep issues two requests per compiler per program (asm, then
# execute) and Compiler Explorer rate-limits hard, so results are persisted
# through the cache shared with scripts/codegen_oracle.py, scripts/encdiff.py
# and scripts/oracle_asm.py -- one .godbolt-cache/ for the whole tree, hit
# once per (compiler, flags, source, execute) tuple across runs, not once per
# process. Without this a sweep could not be resumed after a 429 and could
# not be re-run offline to reproduce a table it had already published.
sys.path.insert(0, os.path.join(REPO, "scripts"))
import godbolt_cache  # noqa: E402

# Failures are not all alike, and caching them all is how an oracle disappears
# from every future sweep with no way back. A transport failure -- rate limit,
# dropped connection, malformed reply -- is retried next run. A deterministic
# one -- this oracle rejects this program, or CE declines to execute it -- is
# cached, because the answer will be identical and re-asking only spends the
# rate limit we are trying to conserve.
_TRANSPORT = (urllib.error.URLError, urllib.error.HTTPError, OSError,
              TimeoutError, json.JSONDecodeError)

# Cleared by --no-cache. Module-level rather than threaded through every
# signature because `remote()` is invoked from a thread-pool lambda and a
# fifth positional argument would obscure it for no gain: this is written
# once, before any sweep starts, and only read afterwards.
USE_CACHE = True
# `<number>:` local labels and `.size`/`.type`/`.globl` directives.
DIRECTIVE = re.compile(r"^\s*\.|^\s*[0-9]+:|^\s*\.[A-Za-z_]+")


def _post(url: str, payload: dict, timeout: int = 120, attempts: int = 5) -> dict:
    """POST to Compiler Explorer, retrying the failures that are worth retrying.

    CE rate-limits aggressively (HTTP 429) and a full sweep issues two
    requests per compiler per program, so a single sweep can trip it.  429
    and 5xx are transient by definition -- honour `Retry-After` when given,
    otherwise back off exponentially with a deterministic jitter derived
    from the URL so parallel workers do not resynchronise into a second
    burst.  A 4xx that is not 429 is a real client error and is raised
    immediately: retrying it would only waste the rate-limit budget.
    """
    body = json.dumps(payload).encode()
    last = None
    for attempt in range(attempts):
        req = urllib.request.Request(
            url, data=body,
            headers={"Content-Type": "application/json", "Accept": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as fh:
                return json.loads(fh.read().decode("utf-8", "replace"))
        except urllib.error.HTTPError as e:
            retry_after = e.headers.get("Retry-After") if e.headers else None
            last = e
            if e.code != 429 and e.code < 500:
                raise
            if attempt == attempts - 1:
                break
            try:
                wait = float(retry_after) if retry_after else 0.0
            except ValueError:
                wait = 0.0
            wait = max(wait, min(30.0, 1.5 * (2 ** attempt)) *
                       (0.75 + 0.5 * (hash(url) % 100) / 100.0))
            time.sleep(wait)
        except (urllib.error.URLError, OSError, TimeoutError) as e:
            last = e
            if attempt == attempts - 1:
                break
            time.sleep(min(30.0, 1.5 * (2 ** attempt)))
    raise last


def _ce_call(cid: str, source: str, flags: str, execute: bool, timeout: int) -> dict:
    """Compile `source` on Compiler Explorer with `cid`; optionally execute.

    `asm` is only returned by a NON-executing request, so the two are
    separate calls: `remote()` issues both and reports whichever failed.
    """
    options = {
        "userArguments": flags,
        # `intel: False` asks for AT&T.  Compiler Explorer's default is
        # Intel syntax, which GNU `as` will not accept, and the timing
        # path re-assembles the oracle's output with the LOCAL `as` -- so
        # the syntax has to be the local assembler's, or every oracle is
        # skipped with an "operand size mismatch" that is really just
        # "you handed me the wrong dialect".
        "filters": {"intel": {"intel": False, "demangle": True, "directives": True}},
        "tools": [{"id": "execute", "args": flags.split()}] if execute else [],
        "executeParameters": {"args": [], "stdin": ""},
    }
    # `executorRequest` makes CE run the binary INSTEAD of returning
    # `asm` -- asking for both yields execution with an empty assembly.
    # The two passes are therefore genuinely separate requests.
    if execute:
        options["compilerOptions"] = {"executorRequest": True}
    return _post(f"{CE}/compiler/{cid}/compile",
                 {"languages": "c", "source": source, "options": options},
                 timeout=timeout)


def _join(stream, sep: str = "") -> str:
    """Flatten Compiler Explorer's chunked stream.

    Compiler Explorer returns a LIST of chunks, and the newline BETWEEN two
    chunks belongs to neither of them.  Both streams therefore rejoin with
    "\\n".  Joining with "" fuses two stdout lines into `ab` -- which reads
    exactly like a miscompilation and is not one -- and collapses the whole
    `asm` body into a single line, so the instruction count silently
    becomes 0 and the size table reports a perfect 0.000 ratio for every
    compiler.  Both failure modes were observed while building this.
    """
    if not stream:
        return ""
    if isinstance(stream, str):
        return stream
    return sep.join(part.get("text", "") if isinstance(part, dict) else str(part)
                   for part in stream)


def _get_json(url: str, timeout: int = 60, attempts: int = 3):
    """GET a JSON document from CE, or ``None`` on any failure.

    Used only by ``--revalidate``, where a failed probe must not abort the
    sweep: not knowing the current version is a reason to KEEP a cached record
    (and say so), never a reason to delete it or to fail the run.
    """
    last = None
    for attempt in range(attempts):
        req = urllib.request.Request(
            url, headers={"Accept": "application/json",
                          "User-Agent": "lccc-codegen-research/2"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as fh:
                return json.loads(fh.read().decode("utf-8", "replace"))
        except Exception as e:            # noqa: BLE001 - probe must not raise
            last = e
            time.sleep(2 ** attempt)
    print(f"  note: version probe failed ({type(last).__name__}: {last}); "
          f"keeping cached records", file=sys.stderr)
    return None


# Leash for the compiler-identity probe.  It is a nicety, never a blocker: a
# probe that fails for any reason is "unknown", and `--revalidate` then refuses
# to judge rather than guessing.  Kept short (one request per run, and only when
# `--revalidate` was asked for) so it can never dominate a sweep's wall clock.
PROBE_TIMEOUT = 20


def live_semvers(cids, timeout: int = PROBE_TIMEOUT) -> dict:
    """``{cid: semver}`` from ONE ``/api/compilers/c`` request.

    One request for the whole sweep, not one per compiler: this is the cheap
    probe that makes cache staleness detectable without turning every compile
    into a probe-then-compile pair.  Moving channels come back as
    ``(trunk)``/``(latest)``, which is not a version -- the caller must treat
    such a pair as "no drift information" rather than as a match, and fall
    back to the age limit.
    """
    data = _get_json(f"{CE}/compilers/c", timeout=timeout, attempts=2)
    if not isinstance(data, list):
        return {}
    want = set(cids)
    return {c["id"]: (c.get("semver") or "")
            for c in data if isinstance(c, dict) and c.get("id") in want}


UNPINNED = ("(trunk)", "(latest)", "trunk", "latest", "")


def revalidate(cids, max_age_days: float, timeout: int = 60) -> int:
    """Drop cached oracle records that no longer describe the compiler CE runs.

    Two independent reasons, because the channels are not alike:

    * **Version drift** (pinned releases).  ``cg162`` means "x86-64 gcc 16.2";
      if CE ever re-points the id, every record under it silently describes a
      different compiler.  A record whose stored ``ce_semver`` differs from the
      live one is dropped and re-fetched.
    * **Age** (moving channels).  ``cicxlatest`` has no version to compare --
      CE reports ``(latest)`` -- so the only available evidence is how long ago
      the record was taken.  Records older than ``max_age_days`` are dropped.

    Records written before provenance existed carry neither field; they are
    treated as expired, because "unknown origin" is exactly what this command
    is for.  Returns the number of records dropped.
    """
    live = live_semvers(cids, timeout=timeout)
    now = time.time()
    dropped = {"drift": 0, "aged": 0, "unpinned-probe-failed": 0}
    kept = 0
    for path, _key in godbolt_cache.iter_records(godbolt_cache.NS_ORACLE):
        try:
            rec = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue                      # unreadable: load_*() treats it as a miss
        if not isinstance(rec, dict):
            continue
        cid = rec.get("id")
        if cid not in cids:
            kept += 1
            continue
        recorded = rec.get("ce_semver")
        current = live.get(cid)
        if recorded and current and current not in UNPINNED:
            if recorded != current:
                if godbolt_cache.drop(path):
                    dropped["drift"] += 1
                continue
            kept += 1
            continue                      # pinned and identical: keep, no age limit
        # No usable version pair: fall back to age.
        if not live:
            dropped["unpinned-probe-failed"] += 1   # kept on disk; counted for the report
            kept += 1
            continue
        stamped = rec.get("cached_at")
        if not isinstance(stamped, (int, float)):
            if godbolt_cache.drop(path):
                dropped["aged"] += 1      # no provenance at all: unknown origin
            continue
        if (now - stamped) > max_age_days * 86400.0:
            if godbolt_cache.drop(path):
                dropped["aged"] += 1
            continue
        kept += 1
    print(f"revalidate: dropped {dropped['drift']} drifted, "
          f"{dropped['aged']} aged/unknown-provenance; kept {kept}")
    if dropped["unpinned-probe-failed"]:
        print(f"  note: {dropped['unpinned-probe-failed']} record(s) could not be "
              f"version-checked (probe unavailable) and were kept on age alone")
    for cid in sorted(cids):
        print(f"  {cid:<14} live semver: {live.get(cid, '<probe failed>')}")
    return dropped["drift"] + dropped["aged"]


def remote(cid: str, source: str, flags: str, execute: bool, timeout: int) -> dict:
    """Compile on one oracle.  Never raises: every failure is a SKIP record.

    Results are memoised in the tree-wide persistent cache (see the comment on
    the `godbolt_cache` import) keyed by (compiler, flags, source, execute),
    so a program that asks for both an asm pass and an execute pass -- or a
    second sweep with a different --filter, or a re-run next week to reproduce
    a published table -- does not re-spend the rate limit on identical work.

    The CE compiler version is deliberately NOT part of the key.  Putting it
    there would double every request (probe the version, then compile), and CE
    reports `(trunk)` / `(latest)` for the moving channels anyway, so for
    `cicxlatest` the version string carries no information.  Instead each
    record carries the semver it was compiled under (`ce_semver`) and the time
    it was stored (`cached_at`), and `--revalidate` uses ONE `/api/compilers`
    request for the whole sweep to drop every record that a pinned channel has
    drifted away from, or that is older than `--max-age-days`.  That answers
    both questions without conflating them: the cache as stored reproduces a
    published table exactly, and `--revalidate` measures the compiler CE is
    serving *today*.

    Not every result is cached. See `_TRANSPORT`: a rate-limit or network
    failure is retried next run, because caching it would make a transient
    blip indistinguishable from an oracle that genuinely cannot run the
    program, and would strand it out of every future sweep.
    """
    rec = {"id": cid, "vendor": VENDOR.get(cid, "?"), "ok": False,
           "asm_insns": None, "stdout": None, "exit": None, "reason": None,
           # Provenance for --revalidate; see the docstring.  `ce_semver` is
           # None when the version has not been probed this run, which is the
           # normal case: probing costs a request and only --revalidate needs
           # it.  A record with no semver is still TTL-checkable.
           "ce_semver": None, "cached_at": time.time()}
    ck = (cid, flags, hashlib.sha256(source.encode()).hexdigest(), execute)
    if USE_CACHE:
        hit = godbolt_cache.load_json(godbolt_cache.NS_ORACLE, *ck)
        if hit is not None:
            return hit

    def safe(execute_it):
        """Return ``(response, error, was_transport_failure)``."""
        try:
            return _ce_call(cid, source, flags, execute_it, timeout), None, False
        except _TRANSPORT as e:
            return None, f"{type(e).__name__}: {e}", True

    def finish(cacheable: bool) -> dict:
        if cacheable and USE_CACHE:
            godbolt_cache.store_json(godbolt_cache.NS_ORACLE, rec, *ck)
        return dict(rec)

    r, err, transport = safe(False)            # asm pass (no execution)
    if r is None:
        rec["reason"] = err
        return finish(not transport)
    asm = _join(r.get("asm"), "\n")
    if r.get("code") not in (0, None) and not asm:
        rec["reason"] = (r.get("stderr") or "compile failed").strip().splitlines()[0][:160]
        return finish(True)     # the oracle rejected the program: same next time
    rec["ok"] = True
    rec["asm_insns"] = sum(1 for l in asm.splitlines() if INSN.match(l)) if asm else 0

    if execute:
        x, err, transport = safe(True)         # execute pass
        if x is None:
            rec["reason"] = err
            rec["ok"] = False
            return finish(not transport)
        if not x.get("didExecute"):
            rec["reason"] = "CE compiled but did not execute"
            rec["ok"] = False
            return finish(True)     # CE's refusal is a property of the program
        br = x.get("buildResult") or {}
        rec["stdout"] = _join(x.get("stdout") or br.get("stdout"), "\n")
        er = x.get("execResult") or {}
        rec["exit"] = er.get("code") if "code" in er else br.get("code")
    return finish(True)


def local(binary: str, source: str, flags: str, execute: bool, tmp: str) -> dict:
    """Compile+run with the tree's lccc, on exactly the same source and flags."""
    rec = {"id": "lccc", "vendor": "local", "ok": False,
           "asm_insns": None, "stdout": None, "exit": None, "reason": None}
    s = os.path.join(tmp, "p.s")
    r = subprocess.run([LCCC, *flags.split(), "-S", "-o", s, source],
                       capture_output=True, text=True)
    if r.returncode != 0:
        rec["reason"] = (r.stderr or r.stdout).strip().splitlines()[0][:160] if (
            r.stderr or r.stdout) else "compile failed"
        return rec
    with open(s, errors="replace") as fh:
        rec["asm_insns"] = sum(1 for l in fh if INSN.match(l))

    if execute:
        exe = os.path.join(tmp, "p.bin")
        r = subprocess.run([LCCC, *flags.split(), "-o", exe, source],
                           capture_output=True, text=True)
        if r.returncode != 0:
            rec["reason"] = (r.stderr or r.stdout).strip().splitlines()[0][:160] if (
                r.stderr or r.stdout) else "link failed"
            rec["ok"] = False
            return rec
        try:
            p = subprocess.run([exe], capture_output=True, text=True, timeout=30)
        except subprocess.TimeoutExpired:
            rec["reason"] = "lccc binary timed out"
            return rec
        rec["stdout"], rec["exit"] = p.stdout, p.returncode
    rec["ok"] = True
    return rec


def bench_remote(cid: str, source: str, flags: str, reps: int, timeout: int) -> dict:
    """Assemble an oracle's output LOCALLY, link it, and time it here.

    Compiler Explorer will not hand over a linked binary (`binary` tool
    returns no `downloads`), but it WILL hand over the assembly it emitted.
    Assembling and linking that with the local `as`/`ld` puts every
    compiler's machine code behind the identical assembler, linker, libc
    and CPU -- so the only thing that varies is the compiler.  That is the
    controlled experiment: the alternative, comparing wall-clock inside
    Compiler Explorer's fleet, measures someone else's machine.

    Returns best-of-`reps` seconds, plus the output for a semantics
    cross-check, so a binary that wins by being wrong cannot look good.
    """
    rec = {"id": cid, "vendor": VENDOR.get(cid, "?"), "ok": False,
           "sec": None, "stdout": None, "exit": None, "reason": None}
    # `-masm=att` is honoured by GCC and ICX.  Compiler Explorer's Clang
    # instances answer in Intel syntax and ignore the `intel` filter, so
    # the re-assembly below fails for them -- reported as a SKIP with that
    # reason rather than quietly dropping the column, because a timing
    # table that silently omits a vendor is worse than no table.
    att = f"{flags} -masm=att".strip()
    try:
        r = _ce_call(cid, source, att, False, timeout)
    except Exception as e:                                  # noqa: BLE001
        rec["reason"] = f"{type(e).__name__}: {e}"
        return rec
    asm = _join(r.get("asm"), "\n")
    if not asm:
        rec["reason"] = "no asm returned"
        return rec
    with tempfile.TemporaryDirectory() as tmp:
        s = os.path.join(tmp, "o.s")
        exe = os.path.join(tmp, "o.bin")
        # Compiler Explorer prints the entry point as a bare `main:` label
        # with no `.globl`, so the result is not a linkable program and
        # `ld` fails with "undefined reference to `main`" from crt1.o --
        # which reads like a compiler bug and is a missing directive.
        if re.search(r"^\s*main:", asm, re.M):
            asm = ".globl main\n" + asm
        with open(s, "w") as fh:
            fh.write(asm)
        a = subprocess.run(["as", "--64", "-o", os.path.join(tmp, "o.o"), s],
                           capture_output=True, text=True)
        if a.returncode != 0:
            first = next((l for l in a.stderr.splitlines() if "Error" in l), "as failed")
            rec["reason"] = ("CE returned a dialect the local assembler rejects "
                             f"(-masm=att ignored): {first.strip()[:100]}")
            return rec
        lk = subprocess.run(["gcc", "-no-pie", "-o", exe, os.path.join(tmp, "o.o")],
                            capture_output=True, text=True)
        if lk.returncode != 0:
            rec["reason"] = "ld: " + (lk.stderr.strip().splitlines() or ["?"])[0][:120]
            return rec
        os.chmod(exe, 0o755)
        best = None
        for _ in range(reps):
            t0 = time.perf_counter()
            p = subprocess.run([exe], capture_output=True, text=True, timeout=60)
            dt = time.perf_counter() - t0
            best = dt if best is None else min(best, dt)
        rec["ok"] = True
        rec["sec"] = best
        rec["stdout"] = p.stdout
        rec["exit"] = p.returncode
    return rec


def bench_local(source: str, flags: str, reps: int, cc: str | None = None) -> dict:
    """Build and time `source` with `cc` (default: this tree's lccc)."""
    tool = cc or LCCC
    rec = {"id": "lccc" if cc is None else Path(cc).name,
           "vendor": "local", "ok": False,
           "sec": None, "stdout": None, "exit": None, "reason": None}
    with tempfile.TemporaryDirectory() as tmp:
        exe = os.path.join(tmp, "l.bin")
        b = subprocess.run([tool, *flags.split(), "-o", exe, source],
                           capture_output=True, text=True)
        if b.returncode != 0:
            rec["reason"] = (b.stderr or b.stdout).strip()[:160]
            return rec
        os.chmod(exe, 0o755)
        best = None
        for _ in range(reps):
            t0 = time.perf_counter()
            p = subprocess.run([exe], capture_output=True, text=True, timeout=60)
            dt = time.perf_counter() - t0
            best = dt if best is None else min(best, dt)
        rec.update(ok=True, sec=best, stdout=p.stdout, exit=p.returncode)
    return rec


def _norm(text) -> str:
    """Normalise stdout for comparison: CRLF from Windows oracles, and
    trailing-whitespace differences that carry no information."""
    if text is None:
        return None
    return "\n".join(line.rstrip() for line in text.replace("\r\n", "\n").splitlines())


def check(prog: str, cids: list, flags: str, execute: bool, timeout: int) -> dict:
    path = os.path.join(PROGRAMS, prog)
    with open(path) as fh:
        source = fh.read()
    with tempfile.TemporaryDirectory() as tmp:
        l = local(binary=None, source=path, flags=flags, execute=execute, tmp=tmp)
        with concurrent.futures.ThreadPoolExecutor(max_workers=len(cids) or 1) as ex:
            remotes = list(ex.map(lambda c: remote(c, source, flags, execute, timeout), cids))
    row = {"program": prog, "lccc": l, "oracles": remotes, "status": "PASS", "why": []}

    if not l["ok"]:
        row["status"] = "FAIL"
        row["why"].append(f"lccc did not build: {l['reason']}")
        return row

    ran = 0
    for r in remotes:
        if not r["ok"]:
            row["why"].append(f"{r['id']} SKIP: {r['reason']}")
            continue
        ran += 1
        if not execute:
            continue
        # Semantic equality.  A remote difference is a real lccc divergence
        # worth investigating -- but it is reported as DIVERGES, not as a
        # silent pass, so a reviewer can adjudicate it against the spec.
        if _norm(l["stdout"]) != _norm(r["stdout"]) or l["exit"] != r["exit"]:
            row["status"] = "DIVERGES"
            row["why"].append(
                f"{r['id']}: stdout {r['stdout']!r} exit {r['exit']} "
                f"vs lccc {_norm(l['stdout'])!r} exit {l['exit']}")
    if ran == 0:
        row["status"] = "ERROR"
        row["why"].append("no oracle ran")
    if execute and ran == 0 and row["status"] == "PASS":
        row["status"] = "ERROR"
    return row


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--list", action="store_true", help="list oracle compiler ids and exit")
    ap.add_argument("--oracles", help="comma-separated Compiler Explorer ids")
    ap.add_argument("--oracle-set", choices=sorted(ORACLE_SET), default="default")
    ap.add_argument("--filter", default="", help="substring filter on program name")
    ap.add_argument("-O", default="-O2", help="optimisation flags (default -O2)")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--semantics-only", action="store_true",
                    help="skip the size table (still executed everywhere)")
    ap.add_argument("--json", help="write the full record here")
    ap.add_argument("--bench", type=int, metavar="REPS", default=0,
                    help="also TIME every compiler locally (best of REPS runs); "
                         "0 disables.  Each oracle's assembly is assembled and "
                         "linked HERE, so all four binaries share one machine, "
                         "one libc and one linker.")
    ap.add_argument("--no-size", action="store_true", help="suppress the size table")
    ap.add_argument("--local-ref", default="gcc",
                    help="comma-separated LOCAL reference compilers for the "
                         "timing table (default gcc; empty string disables). "
                         "Compiler Explorer's asm view omits section "
                         "directives, so a remote binary can only be rebuilt "
                         "locally for some programs -- a local reference "
                         "compiler makes the speed column always available.")
    ap.add_argument("--jobs", type=int, default=2)
    ap.add_argument("--no-cache", action="store_true",
                    help="ignore and do not write the persistent CE cache "
                         "(every request hits the network)")
    ap.add_argument("--cache-stats", action="store_true",
                    help="print the persistent cache's record counts and exit")
    ap.add_argument("--revalidate", action="store_true",
                    help="drop cached oracle records whose compiler version "
                         "drifted or that are older than --max-age-days, then "
                         "continue with the sweep (one /api/compilers request "
                         "for the whole run)")
    ap.add_argument("--max-age-days", type=float, default=28.0,
                    help="age limit for records that cannot be version-checked "
                         "(moving channels such as icx-latest, and records "
                         "written before provenance was stored). 0 drops all "
                         "of them. Default 28.")
    a = ap.parse_args(argv)

    global USE_CACHE
    USE_CACHE = not a.no_cache

    # Resolved before the cache commands so `--revalidate` knows which
    # compilers' records it is reasoning about.
    cids = ([x.strip() for x in a.oracles.split(",") if x.strip()]
            if a.oracles else ORACLE_SET[a.oracle_set])

    if a.revalidate and USE_CACHE:
        revalidate(set(cids), a.max_age_days)
    elif a.revalidate:
        print("--revalidate ignored: --no-cache is in effect", file=sys.stderr)

    if a.cache_stats:
        st = godbolt_cache.stats()
        if not st:
            print(f"cache empty ({godbolt_cache.CACHE})")
        else:
            print(f"cache at {godbolt_cache.CACHE}")
            for ns, n in sorted(st.items()):
                print(f"  {ns:12s} {n:6d} records")
        return 0

    if a.list:
        for name, ids in sorted(ORACLE_SET.items()):
            print(f"{name:12s} {' '.join(ids)}")
        return 0

    progs = sorted(f for f in os.listdir(PROGRAMS) if f.endswith(".c") and a.filter in f)
    if not progs:
        print("no programs matched", file=sys.stderr)
        return 2
    if not os.path.exists(LCCC):
        print(f"lccc not built: {LCCC}", file=sys.stderr)
        return 2

    t0 = time.time()
    rows = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as ex:
        futs = {ex.submit(check, p, cids, a.O, True, a.timeout): p for p in progs}
        for fut in concurrent.futures.as_completed(futs):
            rows.append(fut.result())
    rows.sort(key=lambda r: r["program"])

    print(f"\n== godbolt oracle: lccc vs {' '.join(cids)} at {a.O} "
          f"({time.time() - t0:.1f}s) ==")
    if a.bench:
        refs = [x.strip() for x in a.local_ref.split(",") if x.strip()]
        print(f"\n== timing: best of {a.bench} runs, every binary built and run "
              f"on THIS host ==")
        print(f"{'program':26s} {'lccc ms':>9s}"
              + "".join(f" {Path(r).name[:9]:>10s}" for r in refs)
              + "".join(f" {c[:9]:>10s}" for c in cids)
              + f" {'lccc/ref':>9s}  status")
        worse = []
        for prog in progs:
            path = os.path.join(PROGRAMS, prog)
            source = open(path).read()
            lb = bench_local(path, a.O, a.bench)
            row = {"program": prog, "lccc": lb, "oracles": [], "status": "PASS", "why": []}
            refs = [x.strip() for x in a.local_ref.split(",") if x.strip()]
            for c in cids:
                o = bench_remote(c, source, a.O, a.bench, a.timeout)
                # A REBUILT binary earns the right to be timed only by first
                # reproducing what Compiler Explorer's OWN executor printed.
                # The asm view is a disassembly: `.comm`, `.local`, `.section`
                # and `.extern` are stripped, so it is not always a linkable
                # program.  Without this check a stripped-away GOT entry shows
                # up as an empty output and gets reported as a miscompilation
                # of lccc, when the truth is that the re-assembly is invalid.
                if o["ok"]:
                    ce = remote(c, source, a.O, True, a.timeout)
                    if not ce["ok"] or _norm(ce["stdout"]) != _norm(o["stdout"]):
                        o["ok"] = False
                        o["sec"] = None
                        o["reason"] = ("re-assembled binary does not reproduce "
                                       "Compiler Explorer's own execution; the "
                                       "asm view is not self-contained")
                row["oracles"].append(o)
            if not lb["ok"]:
                row["status"], _ = "ERROR", row["why"].append(f"lccc: {lb['reason']}")
            for o in row["oracles"]:
                if not o["ok"]:
                    row["why"].append(f"{o['id']} SKIP: {o['reason']}")
                elif lb["ok"] and _norm(lb["stdout"]) != _norm(o["stdout"]):
                    # Only meaningful when lccc produced output to compare
                    # against.  If lccc failed to build, `lb["stdout"]` is
                    # None and EVERY oracle "differs" from it, which would
                    # report N phantom divergences instead of the one real
                    # failure that is already in `why`.
                    row["status"] = "DIVERGES"
                    row["why"].append(f"{o['id']}: output differs when run locally")
            for r in refs:
                rb = bench_local(path, a.O, a.bench, cc=r)
                if rb["ok"] and _norm(rb["stdout"]) != _norm(lb["stdout"]):
                    rb["ok"] = False
                    rb["sec"] = None
                    rb["reason"] = "local reference compiler output differs"
                row["oracles"].append(rb)
            ref = next((o for o in row["oracles"] if o["id"] == "gcc"), None)
            rel = None
            if lb["ok"] and ref and ref["ok"] and ref["sec"]:
                rel = lb["sec"] / ref["sec"]
            cells = "".join(
                f" {(o['sec'] * 1e3) if o['ok'] and o['sec'] else -1:10.3f}"
                for o in row["oracles"])
            # lccc may have failed to build at all; never multiply a None.
            mine = f"{lb['sec'] * 1e3:9.3f}" if lb["ok"] and lb["sec"] else "     n/a"
            ratio = f"{rel:9.3f}" if rel else "     n/a"
            print(f"{prog[:-2]:26s} {mine}{cells} {ratio}  {row['status']}")
            if rel and rel > 1.02:
                worse.append((rel, prog))
            for w in row["why"]:
                print("    " + w)
        if worse:
            worse.sort(reverse=True)
            print("\n  slower than the local reference: "
                  + ", ".join(f"{p[:-2]} {r:.2f}x" for r, p in worse))
        print("  (ratios are best-of; 1.00 means indistinguishable.  A size win is "
              "NOT a speed win -- vectorising cuts instructions and raises throughput.)")

    if not a.semantics_only and not a.no_size:
        hdr = f"{'program':26s} {'lccc':>7s}"
        for c in cids:
            hdr += f" {c[:10]:>10s}"
        hdr += f" {'ratio':>7s}  status"
        print(hdr)
        for r in rows:
            li = r["lccc"]["asm_insns"]
            line = f"{r['program'][:-2]:26s} {li if li is not None else -1:7d}"
            per = []
            for c in cids:
                m = next((x for x in r["oracles"] if x["id"] == c), None)
                n = m["asm_insns"] if m and m["ok"] else None
                per.append(n)
                line += f" {n if n is not None else -1:10d}"
            good = [n for n in per if n]
            line += f" {(sum(good) / len(good) / li) if good and li else 0:7.3f}"
            line += f"  {r['status']}"
            print(line)
        tot_l = sum(r["lccc"]["asm_insns"] or 0 for r in rows)
        for c in cids:
            tot = sum(next((x["asm_insns"] for x in r["oracles"] if x["id"] == c), None) or 0
                      for r in rows)
            n = sum(1 for r in rows if next((x["asm_insns"] for x in r["oracles"] if x["id"] == c), None))
            if n:
                print(f"  total vs {c:12s} lccc={tot_l:7d} oracle={tot:7d} "
                      f"ratio={tot_l / tot:.4f} over {n} program(s)")

    bad = [r for r in rows if r["status"] in ("FAIL", "ERROR", "DIVERGES")]
    for r in bad:
        print(f"\n[{r['status']}] {r['program']}")
        for w in r["why"]:
            print("    " + w)
    npass = sum(1 for r in rows if r["status"] == "PASS")
    ndiv = sum(1 for r in rows if r["status"] == "DIVERGES")
    print(f"\n== {npass} agree, {ndiv} diverge, {len(rows) - npass - ndiv} error "
          f"({len(rows)} programs) ==")

    if a.json:
        with open(a.json, "w") as fh:
            json.dump(rows, fh, indent=1)

    # An empty or all-skipped oracle set is an error, never a pass.
    if not any(any(x["ok"] for x in r["oracles"]) for r in rows):
        print("\nno oracle produced a result -- network or CE outage, not a pass",
              file=sys.stderr)
        return 3
    return 0 if not bad else 1


if __name__ == "__main__":
    sys.exit(main())
