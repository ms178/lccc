#!/usr/bin/env python3
"""Real-workload differential linker test: expat, zlib-ng, gzip.

Why this exists
===============
Synthetic fixtures exercise one linker feature at a time.  Real projects
exercise the *combinations* that actually break linkers: `-ffunction-sections`
plus `--gc-sections`, hidden visibility, weak aliases, constructors, archive
member selection, PLT/GOT through libc, `.eh_frame` from `-fexceptions`,
COMDAT groups, and a symbol table with thousands of entries.

Contract per workload
---------------------
For each linker (lccc, bfd, mold, wild) we link the *same* set of object
files, then **run the resulting program against a real input and compare the
output byte-for-byte**.  A linker passes only if its binary behaves
identically to the reference.  Sizes and link times are reported alongside,
but correctness is the gate: a smaller or faster wrong binary fails.

The object files are produced once by the system compiler, so the generated
code is held constant and only the linker varies.

Oracles
-------
The competing linkers are the PINNED oracles provisioned by
tools/linker/setup_oracles.sh (GNU ld 2.47, mold 2.42.1 built for
X86_64;I386, LLVM lld 23.1.x, optionally wild HEAD), found as the wrappers in
$LCCC_ORACLE_BIN (default /home/user/artifacts/bin).  A same-named linker on
PATH is used only as a labelled fallback ("bfd(system)"), and
`--require-pinned` turns a missing pin into an error, so a comparison is never
silently run against a distro linker of unknown vintage.  GNU ld is the
reference every other binary's behaviour is compared with.

Measurement
-----------
Every linker is handed the IDENTICAL argument vector: gcc is run once through
a recording shim that captures what collect2 passes to `ld`, and each linker
is then executed directly with that vector (only the `-o` operand differs).
The timings therefore contain the linker alone -- no gcc/collect2 start-up --
and no linker sees different crt files, search paths or flags.  `--runs N`
takes N timed runs after one warm-up (page cache hot for all) and reports the
median wall time; peak RSS comes from wait4(2) of an extra run spawned by
a tiny C helper (see peak_rss_kib; mold's default --fork would report the
parent's RSS only, so that run passes --no-fork).  Output size is reported both as the file size and as the
PT_LOAD bytes the kernel maps, since linkers differ in how much non-loaded
metadata (.symtab, section headers) they keep.

Usage:
    tests/linker/real_workloads.py [--workloads DIR] [--filter NAME] [-v]
                                   [--runs N] [--require-pinned]
"""

import argparse
import hashlib
import os
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
DEFAULT_LD = os.environ.get(
    "LCCC_LD", os.path.join(REPO, "target", "release", "lccc-ld"))
WORKLOADS = os.environ.get("LCCC_WORKLOADS", "/home/user/workloads")
CC = os.environ.get("LINKTEST_CC", "gcc")
ORACLE_BIN = os.environ.get("LCCC_ORACLE_BIN", "/home/user/artifacts/bin")
# (label, pinned wrapper written by tools/linker/setup_oracles.sh, PATH fallback)
ORACLES = (
    ("bfd", "ld-2.47", "ld.bfd"),
    ("mold", "mold-2.42.1", "mold"),
    ("lld", "ld.lld-23.1", "ld.lld"),
    ("wild", "wild", "wild"),
)


def sh(cmd, cwd=None, timeout=300, stdin=None):
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, input=stdin,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)


class Result:
    def __init__(self, name, status, detail=""):
        self.name, self.status, self.detail = name, status, detail


def resolve_linkers(lccc_ld, require_pinned):
    """[(label, path, banner)] -- lccc first, then the pinned oracles."""
    out = [("lccc", os.path.abspath(lccc_ld))]
    missing = []
    for label, pinned, fallback in ORACLES:
        p = os.path.join(ORACLE_BIN, pinned)
        if os.access(p, os.X_OK):
            out.append((label, p))
            continue
        missing.append(pinned)
        q = shutil.which(fallback)
        if q and not require_pinned:
            out.append((f"{label}(system)", q))
    if require_pinned and [m for m in missing if m != "wild"]:
        sys.exit(f"--require-pinned: missing {missing} in {ORACLE_BIN} "
                 "(run tools/linker/setup_oracles.sh)")
    res = []
    for label, p in out:
        r = sh([p, "--version"], timeout=30)
        banner = (r.stdout or r.stderr).decode(errors="replace").splitlines()
        res.append((label, p, banner[0].strip() if banner else "?"))
    return res


RECORDING_SHIM = """#!/bin/sh
for a in "$@"; do printf '%s\\0' "$a"; done > "$LCCC_ARGV_OUT"
exec "$LCCC_ARGV_LD" "$@"
"""


def capture_link_argv(cc_args, ref_ld, td):
    """Run `gcc cc_args` with a shim `ld` that records the argument vector
    collect2 hands the linker (and then links with `ref_ld`).  Returns the
    vector, or raises with gcc's diagnostics."""
    shim = tempfile.mkdtemp(prefix="ldshim.")
    try:
        ld = os.path.join(shim, "ld")
        with open(ld, "w") as f:
            f.write(RECORDING_SHIM)
        os.chmod(ld, 0o755)
        argv_file = os.path.join(td, "ld.argv")
        env = dict(os.environ, LCCC_ARGV_OUT=argv_file, LCCC_ARGV_LD=ref_ld)
        r = subprocess.run([CC, "-B" + shim] + cc_args, cwd=td, env=env,
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=600)
        if r.returncode != 0 or not os.path.exists(argv_file):
            raise RuntimeError("reference link failed: "
                               + r.stderr.decode(errors="replace")[:400])
        with open(argv_file, "rb") as f:
            raw = f.read()
        argv = [a.decode() for a in raw.split(b"\0")[:-1]]
        if argv.count("-o") != 1:
            raise RuntimeError(f"expected exactly one -o in {argv!r}")
        return argv
    finally:
        shutil.rmtree(shim, ignore_errors=True)


def with_output(argv, out):
    v = list(argv)
    v[v.index("-o") + 1] = out
    return v


# Peak RSS must be measured from a SMALL parent.  Linux carries the RSS
# high-water mark of the image a process replaces into its ru_maxrss at
# execve (exec_mmap -> setmax_mm_hiwater_rss), so a linker forked straight
# from this Python process reports max(python's ~19 MiB, its own): measured
# here, lccc-ld and GNU ld both came out as exactly 19216 KiB.  This helper is
# tiny, so its child starts from a ~1 MiB floor, and it reports the child's
# own ru_maxrss from wait4.
RSS_HELPER_SRC = r"""
#include <stdio.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <unistd.h>
int main(int argc, char **argv) {
    (void)argc;
    pid_t pid = fork();
    if (pid < 0) return 125;
    if (pid == 0) {
        execvp(argv[1], argv + 1);
        _exit(127);
    }
    int status;
    struct rusage ru;
    if (wait4(pid, &status, 0, &ru) < 0) return 125;
    fprintf(stderr, "\nLCCC_MAXRSS_KIB=%ld\n", ru.ru_maxrss);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 126;
}
"""
_rss_helper = None


def peak_rss_kib(cmd, cwd):
    global _rss_helper
    if _rss_helper is None:
        d = tempfile.mkdtemp(prefix="rsshelper.")
        with open(os.path.join(d, "h.c"), "w") as f:
            f.write(RSS_HELPER_SRC)
        exe = os.path.join(d, "h")
        r = sh([CC, "-O2", "-static", "h.c", "-o", exe], cwd=d)
        if r.returncode != 0:
            r = sh([CC, "-O2", "h.c", "-o", exe], cwd=d)
        _rss_helper = exe if r.returncode == 0 else ""
    if not _rss_helper:
        return None
    r = sh([_rss_helper] + cmd, cwd=cwd, timeout=600)
    marker = b"\nLCCC_MAXRSS_KIB="
    if r.returncode != 0 or marker not in r.stderr:
        return None
    return int(r.stderr.rsplit(marker, 1)[1].split()[0])


def loaded_bytes(path):
    """Sum of PT_LOAD p_filesz -- the bytes the kernel maps."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"\x7fELF":
        return None
    is64, end = data[4] == 2, "<" if data[5] == 1 else ">"
    if is64:
        phoff, = struct.unpack_from(end + "Q", data, 0x20)
        phentsize, phnum = struct.unpack_from(end + "HH", data, 0x36)
    else:
        phoff, = struct.unpack_from(end + "I", data, 0x1C)
        phentsize, phnum = struct.unpack_from(end + "HH", data, 0x2A)
    total = 0
    for i in range(phnum):
        o = phoff + i * phentsize
        p_type, = struct.unpack_from(end + "I", data, o)
        if p_type == 1:
            filesz, = struct.unpack_from(end + ("Q" if is64 else "I"), data,
                                         o + (0x20 if is64 else 0x10))
            total += filesz
    return total


# ---------------------------------------------------------------------------
# workload definitions
# ---------------------------------------------------------------------------

class Workload:
    """One real program to link and then exercise."""

    def __init__(self, name, root, objects, extra_ldflags=None,
                 make_input=None, run=None, note=""):
        self.name = name
        self.root = root
        self.objects = objects          # paths relative to root
        self.extra_ldflags = extra_ldflags or []
        self.make_input = make_input    # callable(td) -> None
        self.run = run                  # callable(binary, td) -> bytes
        self.note = note


def _expat_input(td):
    with open(os.path.join(td, "in.xml"), "w") as f:
        f.write('<?xml version="1.0"?>\n<root attr="v">\n')
        for i in range(500):
            f.write(f'  <item id="{i}"><name>n{i}</name><val>{i*7}</val></item>\n')
        f.write("</root>\n")
    # A deliberately malformed document: the parser must report an error at a
    # specific line/column, which only works if the real parser tables linked.
    with open(os.path.join(td, "bad.xml"), "w") as f:
        f.write('<?xml version="1.0"?>\n<root>\n  <unclosed>\n</root>\n')


def _expat_run(binary, td):
    """Exercise xmlwf on a valid and an invalid document.

    Two calls, because a linker bug that breaks the parser tables would still
    let a "just validate" run exit 0 by doing nothing.  Requiring a *correct
    diagnostic* on malformed input proves the parser really ran.

    `-d DIR` is deliberately not used: it makes xmlwf rewrite the input in
    place, which destroyed the fixture and made every linker look equally
    broken ("no element found") in an earlier version of this harness.
    """
    good = os.path.join(td, "in.xml")
    bad = os.path.join(td, "bad.xml")
    r1 = sh([binary, good], cwd=td, timeout=60)
    r2 = sh([binary, bad], cwd=td, timeout=60)
    # -c echoes the parsed document, which makes the parser's output itself
    # part of the comparison rather than only its exit status.
    r3 = sh([binary, "-c", good], cwd=td, timeout=60)
    digest = hashlib.sha256(r3.stdout).hexdigest()[:16]
    return b"valid_rc=%d valid_err=%s | invalid_rc=%d invalid_err=%s | echo=%s" % (
        r1.returncode, r1.stdout.strip()[:80],
        r2.returncode, (r2.stdout.strip() or r2.stderr.strip())[:80],
        digest.encode())


def _gzip_input(td):
    data = (b"the quick brown fox jumps over the lazy dog 0123456789\n" * 2000)
    with open(os.path.join(td, "in.txt"), "wb") as f:
        f.write(data)


def _gzip_run(binary, td):
    src = os.path.join(td, "in.txt")
    r1 = sh([binary, "-c", "-9", src], cwd=td, timeout=60)
    if r1.returncode != 0:
        return b"compress-failed rc=%d %s" % (r1.returncode, r1.stderr[:200])
    r2 = sh([binary, "-d", "-c"], cwd=td, timeout=60, stdin=r1.stdout)
    original = open(src, "rb").read()
    ok = (r2.returncode == 0 and r2.stdout == original)
    # Report the compressed size too: a linker bug that silently drops an
    # optimised code path would still round-trip but change the output size.
    return b"roundtrip=%s csize=%d" % (b"ok" if ok else b"BROKEN", len(r1.stdout))


def discover(workload_dir):
    wls = []

    # ---- expat / xmlwf -----------------------------------------------------
    expat = None
    for d in sorted(os.listdir(workload_dir)) if os.path.isdir(workload_dir) else []:
        if d.startswith("expat-"):
            expat = os.path.join(workload_dir, d)
    if expat:
        objs = [
            "xmlwf/xmlwf-xmlwf.o", "xmlwf/xmlwf-xmlfile.o",
            "xmlwf/xmlwf-codepage.o", "xmlwf/xmlwf-unixfilemap.o",
            "lib/.libs/xmlparse.o", "lib/.libs/xmlrole.o", "lib/.libs/xmltok.o",
        ]
        if all(os.path.exists(os.path.join(expat, o)) for o in objs):
            wls.append(Workload(
                "expat_xmlwf", expat, objs,
                make_input=_expat_input, run=_expat_run,
                note="XML parser + CLI: hidden visibility, tables, "
                     "-ffunction-sections"))

    # ---- gzip --------------------------------------------------------------
    gz = None
    for d in sorted(os.listdir(workload_dir)) if os.path.isdir(workload_dir) else []:
        if d.startswith("gzip-"):
            gz = os.path.join(workload_dir, d)
    if gz:
        objs = [f for f in sorted(os.listdir(gz)) if f.endswith(".o")]
        libgz = os.path.join(gz, "lib", "libgzip.a")
        if objs and os.path.exists(libgz):
            wls.append(Workload(
                "gzip", gz, objs + ["lib/libgzip.a"],
                make_input=_gzip_input, run=_gzip_run,
                note="compressor: gnulib archive, ctors, many TUs"))

    # ---- zlib-ng (static lib -> test program) -----------------------------
    zng = None
    for d in sorted(os.listdir(workload_dir)) if os.path.isdir(workload_dir) else []:
        if d.startswith("zlib-ng-"):
            zng = os.path.join(workload_dir, d)
    if zng:
        lib = os.path.join(zng, "build", "libz.a")
        if not os.path.exists(lib):
            lib = os.path.join(zng, "build", "libz-ng.a")
        if os.path.exists(lib):
            wls.append(Workload(
                "zlib_ng", zng, [os.path.relpath(lib, zng)],
                make_input=_zng_input, run=_zng_run,
                note="deflate/inflate static archive: CPU-dispatch, "
                     "many archive members"))
    return wls


ZNG_DRIVER = r"""
#include <stdio.h>
#include <string.h>
#include <stdlib.h>
#include <zlib.h>
int main(void) {
    static unsigned char src[262144];
    for (size_t i = 0; i < sizeof src; i++)
        src[i] = (unsigned char)((i * 31 + (i >> 5)) & 0xff);
    uLongf clen = compressBound(sizeof src);
    unsigned char *comp = malloc(clen);
    if (compress2(comp, &clen, src, sizeof src, 9) != Z_OK) {
        puts("compress failed"); return 1;
    }
    static unsigned char back[262144];
    uLongf blen = sizeof back;
    if (uncompress(back, &blen, comp, clen) != Z_OK) {
        puts("uncompress failed"); return 1;
    }
    if (blen != sizeof src || memcmp(src, back, blen) != 0) {
        puts("ROUNDTRIP MISMATCH"); return 1;
    }
    printf("roundtrip=ok csize=%lu crc=%08lx\n",
           (unsigned long)clen,
           (unsigned long)crc32(0L, src, sizeof src));
    return 0;
}
"""


def _zng_input(td):
    with open(os.path.join(td, "zdrv.c"), "w") as f:
        f.write(ZNG_DRIVER)


def _zng_run(binary, td):
    r = sh([binary], cwd=td, timeout=120)
    return b"rc=%d|%s" % (r.returncode, r.stdout.strip())


# ---------------------------------------------------------------------------
# driver
# ---------------------------------------------------------------------------

def run_workload(wl, linkers, args, board):
    td = tempfile.mkdtemp(prefix=f"rw.{wl.name}.")
    try:
        if wl.make_input:
            wl.make_input(td)

        inputs = [os.path.join(wl.root, o) for o in wl.objects]
        missing = [i for i in inputs if not os.path.exists(i)]
        if missing:
            return [Result(wl.name, "SKIP", f"missing objects: {missing[:2]}")]

        # zlib-ng needs its driver compiled against the built headers.
        if wl.name == "zlib_ng":
            inc = os.path.join(wl.root, "build")
            r = sh([CC, "-c", "-O2", "-I", inc, "-I", wl.root,
                    "zdrv.c", "-o", "zdrv.o"], cwd=td)
            if r.returncode != 0:
                return [Result(wl.name, "SKIP",
                               f"driver compile failed: {r.stderr.decode()[:200]}")]
            inputs = [os.path.join(td, "zdrv.o")] + inputs

        ref = next(p for n, p, _ in linkers if n.startswith("bfd"))
        argv = capture_link_argv(inputs + ["-o", "capture"] + wl.extra_ldflags,
                                 ref, td)

        results, outputs, metrics = [], {}, {}
        for lname, ldpath, _ in linkers:
            out = os.path.join(td, f"bin.{lname}")
            cmd = [ldpath] + with_output(argv, out)
            times = []
            r = None
            for i in range(args.runs + 1):          # run 0 is the warm-up
                t0 = time.perf_counter()
                r = sh(cmd, cwd=td, timeout=600)
                dt = time.perf_counter() - t0
                if r.returncode != 0:
                    break
                if i:
                    times.append(dt)
            if r.returncode != 0 or not os.path.exists(out):
                results.append(Result(f"{wl.name}[{lname}]", "LINK-FAIL",
                                      r.stderr.decode(errors="replace")[:300]))
                continue
            rss = peak_rss_kib(cmd + (["--no-fork"] if lname.startswith("mold") else []), td)
            behaviour = wl.run(out, td) if wl.run else b""
            outputs[lname] = behaviour
            ms = statistics.median(times) * 1e3
            size, load = os.path.getsize(out), loaded_bytes(out)
            metrics[lname] = {"ms": ms, "size": size, "load": load, "rss": rss}
            results.append(Result(
                f"{wl.name}[{lname}]", "OK",
                f"{ms:7.1f} ms  {size/1024:8.1f} KiB  load {load/1024:8.1f} KiB  "
                f"rss {(rss or 0)/1024:6.1f} MiB  "
                f"{behaviour.decode(errors='replace')[:60]}"))
        board[wl.name] = metrics

        # differential comparison against the reference (GNU ld)
        refname = next((n for n in outputs if n.startswith("bfd")), None)
        if refname is None:
            results.append(Result(f"{wl.name}:DIFFERENTIAL", "FAIL",
                                  "the GNU ld reference did not link"))
            return results
        for lname, behaviour in outputs.items():
            if lname == refname or behaviour == outputs[refname]:
                continue
            status = "FAIL" if lname == "lccc" else "ORACLE-DIVERGES"
            results.append(Result(
                f"{wl.name}:DIFFERENTIAL[{lname}]", status,
                f"{behaviour[:120]!r} != {refname} {outputs[refname][:120]!r}"))
        if "lccc" in outputs and outputs["lccc"] == outputs[refname]:
            results.append(Result(f"{wl.name}:DIFFERENTIAL", "PASS",
                                  f"lccc binary behaves identically to {refname}'s"))
        return results
    except Exception as e:
        return [Result(wl.name, "FAIL", f"harness exception: {e!r}")]
    finally:
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def print_scoreboard(board, linkers):
    """Per metric (lower is better), where lccc ranks among the linkers."""
    names = [n for n, _, _ in linkers]
    print("\n== scoreboard (lower is better; rank of lccc among linkers that linked) ==")
    keys = (("ms", "link ms (median)"), ("size", "file bytes"),
            ("load", "PT_LOAD bytes"), ("rss", "peak RSS KiB"))
    wins = total = 0
    for wl, m in board.items():
        print(f"  {wl}")
        for k, title in keys:
            vals = {n: m[n][k] for n in names if n in m and m[n][k] is not None}
            if "lccc" not in vals or len(vals) < 2:
                continue
            best = min(vals.values())
            row = "  ".join(f"{n}={vals[n]:.1f}" if k == "ms" else f"{n}={vals[n]}"
                            for n in names if n in vals)
            rank = 1 + sum(1 for v in vals.values() if v < vals["lccc"])
            total += 1
            wins += vals["lccc"] == best
            print(f"    {title:18s} lccc rank {rank}/{len(vals)}   {row}")
    if total:
        print(f"  lccc best (or tied) on {wins}/{total} workload metrics")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc-ld", default=DEFAULT_LD)
    ap.add_argument("--workloads", default=WORKLOADS)
    ap.add_argument("--filter", default="")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    ap.add_argument("--runs", type=int, default=5,
                    help="timed runs per linker after one warm-up (median reported)")
    ap.add_argument("--require-pinned", action="store_true",
                    help="error out unless the pinned oracles are provisioned")
    args = ap.parse_args()
    if args.runs < 1:
        ap.error("--runs must be >= 1")

    linkers = resolve_linkers(args.lccc_ld, args.require_pinned)
    if not any(n.startswith("bfd") for n, _, _ in linkers):
        print("no GNU ld reference found (run tools/linker/setup_oracles.sh)")
        return 1

    wls = discover(args.workloads)
    if not wls:
        print(f"no built workloads found under {args.workloads}")
        return 0

    for n, p, banner in linkers:
        print(f"linker {n:12s} {banner}  [{p}]")
    all_results, board = [], {}
    for wl in wls:
        if args.filter and args.filter not in wl.name:
            continue
        print(f"\n### {wl.name} — {wl.note}")
        for r in run_workload(wl, linkers, args, board):
            all_results.append(r)
            if r.status != "OK" or args.verbose:
                print(f"  [{r.status}] {r.name}" + (f"  {r.detail}" if r.detail else ""))
            else:
                print(f"  [ OK ] {r.name}  {r.detail}")
    print_scoreboard(board, linkers)

    nfail = sum(1 for r in all_results if r.status in ("FAIL", "LINK-FAIL"))
    npass = sum(1 for r in all_results if r.status in ("OK", "PASS"))
    print(f"\n== real workloads: {npass} ok, {nfail} fail ==")
    return 1 if nfail else 0


if __name__ == "__main__":
    sys.exit(main())
