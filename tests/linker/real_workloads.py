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

Usage:
    tests/linker/real_workloads.py [--workloads DIR] [--filter NAME] [-v]
"""

import argparse
import os
import shutil
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


def sh(cmd, cwd=None, timeout=300, stdin=None):
    return subprocess.run(cmd, cwd=cwd, timeout=timeout, input=stdin,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE)


class Result:
    def __init__(self, name, status, detail=""):
        self.name, self.status, self.detail = name, status, detail


def gcc_link_driver(ld_path):
    """Return a gcc argv prefix that forces gcc to use `ld_path`.

    Using gcc as the front end (rather than invoking the linker bare) is what
    real build systems do: it supplies crt1.o/crti.o/crtn.o, the libc search
    paths and --dynamic-linker.  `-B<dir>` makes gcc pick up a directory that
    contains an executable named `ld`.
    """
    d = tempfile.mkdtemp(prefix="ldshim.")
    link = os.path.join(d, "ld")
    os.symlink(os.path.abspath(ld_path), link)
    return ["-B" + d], d


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
    import hashlib
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

def run_workload(wl, linkers, args):
    td = tempfile.mkdtemp(prefix=f"rw.{wl.name}.")
    shims = []
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

        results, outputs = [], {}
        for lname, ldpath in linkers:
            out = os.path.join(td, f"bin.{lname}")
            if ldpath is None:                      # system default (bfd)
                pre = []
            else:
                pre, shimdir = gcc_link_driver(ldpath)
                shims.append(shimdir)
            cmd = [CC] + pre + inputs + ["-o", out] + wl.extra_ldflags
            t0 = time.perf_counter()
            r = sh(cmd, cwd=td, timeout=600)
            dt = time.perf_counter() - t0
            if r.returncode != 0 or not os.path.exists(out):
                results.append(Result(f"{wl.name}[{lname}]", "LINK-FAIL",
                                      r.stderr.decode(errors="replace")[:300]))
                continue

            behaviour = wl.run(out, td) if wl.run else b""
            outputs[lname] = behaviour
            size = os.path.getsize(out)
            results.append(Result(
                f"{wl.name}[{lname}]", "OK",
                f"{dt*1e3:7.1f} ms  {size/1024:8.1f} KiB  {behaviour.decode(errors='replace')[:70]}"))

        # differential comparison against the reference linker
        if "bfd" in outputs and "lccc" in outputs:
            if outputs["lccc"] != outputs["bfd"]:
                results.append(Result(
                    f"{wl.name}:DIFFERENTIAL", "FAIL",
                    f"lccc behaviour {outputs['lccc'][:120]!r} != "
                    f"bfd {outputs['bfd'][:120]!r}"))
            else:
                results.append(Result(f"{wl.name}:DIFFERENTIAL", "PASS",
                                      "lccc binary behaves identically to bfd's"))
        return results
    except Exception as e:
        return [Result(wl.name, "FAIL", f"harness exception: {e!r}")]
    finally:
        for d in shims:
            shutil.rmtree(d, ignore_errors=True)
        if not args.keep:
            shutil.rmtree(td, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--lccc-ld", default=DEFAULT_LD)
    ap.add_argument("--workloads", default=WORKLOADS)
    ap.add_argument("--filter", default="")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    linkers = [("lccc", args.lccc_ld), ("bfd", shutil.which("ld.bfd"))]
    for n, e in (("mold", "mold"), ("wild", "wild")):
        p = shutil.which(e)
        if p:
            linkers.append((n, p))
    linkers = [(n, p) for n, p in linkers if p]

    wls = discover(args.workloads)
    if not wls:
        print(f"no built workloads found under {args.workloads}")
        return 0

    print("linkers: " + ", ".join(n for n, _ in linkers))
    all_results = []
    for wl in wls:
        if args.filter and args.filter not in wl.name:
            continue
        print(f"\n### {wl.name} — {wl.note}")
        for r in run_workload(wl, linkers, args):
            all_results.append(r)
            if r.status != "OK" or args.verbose:
                print(f"  [{r.status}] {r.name}" + (f"  {r.detail}" if r.detail else ""))
            else:
                print(f"  [ OK ] {r.name}  {r.detail}")

    nfail = sum(1 for r in all_results if r.status in ("FAIL", "LINK-FAIL"))
    npass = sum(1 for r in all_results if r.status in ("OK", "PASS"))
    print(f"\n== real workloads: {npass} ok, {nfail} fail ==")
    return 1 if nfail else 0


if __name__ == "__main__":
    sys.exit(main())
