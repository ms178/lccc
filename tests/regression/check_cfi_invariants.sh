#!/usr/bin/env bash
# Static invariants of the synthesized CFI over the whole regression corpus,
# x86-64 and i686, at -O2 (the post-peephole stream cfi_synth describes).
#
# The CFI of an lccc function is derived from its final instructions
# (src/backend/cfi_synth.rs). Two defects of that derivation survived every
# unwind test because they only showed in functions no test unwound through:
# i686 callees that pop stack bytes (`ret $4` of struct returns, fastcall's
# `ret $N`) left the CFA too high for the rest of the caller, and labels laid
# out before any branch to them took a guessed state that then propagated
# (18 i686 corpus files described negative CFA offsets). Unwinding from such
# a point reads a wrong return address. This gate checks what must hold for
# every function, whether or not something unwinds through it:
#
#   1. the CFA is never described below one slot above %sp (the return
#      address itself), in particular never negative;
#   2. at every function-returning `ret`/`ret $N`, CFA = %sp + slot (the
#      inline-retpoline `ret`, which follows `mov %reg, (%sp)`, is a jump);
#   3. the synthesizer finds no label reached with two different frame
#      rules and announces every prologue save (LCCC_CFI_DEBUG=1);
#   4. -fno-asynchronous-unwind-tables produces the same code with the CFI
#      directives removed, for every function of the corpus: CFI is
#      derived after the peephole, so its presence must not change what
#      the peephole does (check_nocfi_peephole_parity.sh pins one file);
#   5. every function the compiler emits has an FDE: a function without one
#      ends every unwind that reaches it. (Functions written in top-level
#      asm -- lccc does not bracket it with #APP -- are recognised by their
#      `name:` label in the C source and are the user's business.)
#
# Unmodelled %sp writes (longjmp-style `mov X, %sp`) are allowed: they make
# the synthesizer emit `.cfi_undefined` for the return address, which ends
# unwinding there instead of describing a wrong frame.
set -euo pipefail
CCC=${CCC:-./target/fastbuild/lccc}
CCC=$(realpath "$CCC")
root=$(cd "$(dirname "$0")" && pwd)
tmp=${TMPDIR:-/tmp}/lccc-cfi-inv.$$
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp"

m32=-m32
if ! echo 'int main(void){return 0;}' | "$CCC" -m32 -x c - -S -o "$tmp/probe.out" 2>/dev/null; then
    m32=""
fi

compile_one() {
    local f=$1 tmp=$2 m32=$3 CCC=$4
    local base flags=""
    base=$(basename "$f" .c)
    [[ -f "${f%.c}.flags" ]] && flags=$(cat "${f%.c}.flags")
    flags=${flags//@PROFDIR@/$tmp/prof}
    # Tests whose flags pick a target or a mode the invariants do not
    # describe are compiled as they are, for their own target only.
    case " $flags " in
        *" -m32 "* | *" -m64 "* | *" -m16 "* | *-march=i*86*) return 0 ;;
    esac
    for m in "" $m32; do
        local out=$tmp/$base${m:+.m32}
        if LCCC_CFI_DEBUG=1 timeout 120 "$CCC" $m -O2 $flags -S "$f" -o "$out.s" \
            2>"$out.err" >/dev/null; then
            # Instrumented code embeds a per-process default profile name.
            [[ "$flags" == *profile-generate* ]] && continue
            timeout 120 "$CCC" $m -O2 $flags -fno-asynchronous-unwind-tables -S "$f" \
                -o "$out.nocfi" 2>/dev/null >/dev/null || rm -f "$out.nocfi"
        else
            rm -f "$out.s"
        fi
    done
}
export -f compile_one
find "$root" -maxdepth 1 -name '*.c' -print0 | sort -z |
    xargs -0 -P "${JOBS:-2}" -I{} bash -c 'compile_one "$@"' _ {} "$tmp" "$m32" "$CCC"

python3 - "$tmp" "$root" <<'PY'
import glob, os, re, sys
tmp, root = sys.argv[1], sys.argv[2]
bad = []
nfn = nret = 0
for fn in sorted(glob.glob(os.path.join(tmp, "*.s"))):
    slot = 4 if fn.endswith(".m32.s") else 8
    name = os.path.basename(fn)
    cfa = off = None
    func = None
    app = False
    prev = ""
    for line in open(fn, errors="replace"):
        t = line.strip()
        if t == "#APP":
            app = True
        elif t == "#NO_APP":
            app = False
        if t.endswith(":") and not t.startswith("."):
            func = t[:-1]
        if t == ".cfi_startproc":
            cfa, off = "sp", slot
            nfn += 1
            continue
        if t == ".cfi_endproc":
            cfa = None
            continue
        if cfa is None:
            continue
        m = re.match(r"\.cfi_def_cfa_offset (-?\d+)$", t)
        if m:
            off = int(m.group(1))
        m = re.match(r"\.cfi_def_cfa %(\w+), (-?\d+)$", t)
        if m:
            cfa = "sp" if m.group(1) in ("rsp", "esp") else m.group(1)
            off = int(m.group(2))
        m = re.match(r"\.cfi_def_cfa_register %(\w+)$", t)
        if m:
            cfa = "sp" if m.group(1) in ("rsp", "esp") else m.group(1)
        if re.match(r"\.cfi_undefined %[re]ip$", t):
            cfa = "undef"
        if t.startswith(".cfi_") and cfa == "sp" and off is not None and off < slot:
            bad.append(f"{name}: {func}: CFA %sp{off:+d} ({t})")
        if not app and re.match(r"ret[lq]?(\s|$)", t) and cfa != "undef":
            nret += 1
            retpoline = re.match(r"mov[lq]?\s+%\w+,\s*(0)?\(%[re]sp\)$", prev)
            if not retpoline and not (cfa == "sp" and off == slot):
                bad.append(f"{name}: {func}: `{t}` with CFA {cfa}{off:+d}, want sp+{slot}")
        if t and not t.startswith((".", "#")):
            prev = t
# 5. FDE coverage of compiler-emitted functions.
for fn in sorted(glob.glob(os.path.join(tmp, "*.s"))):
    base = os.path.basename(fn).split(".")[0]
    src = open(os.path.join(root, base + ".c"), errors="replace").read()
    app = False
    typed = set()
    pending = None
    for line in open(fn, errors="replace"):
        t = line.strip()
        if t == "#APP":
            app = True
        elif t == "#NO_APP":
            app = False
        if app:
            continue
        m = re.match(r"\.type\s+([^,\s]+),\s*@function$", t)
        if m:
            typed.add(m.group(1))
        elif t.endswith(":") and t[:-1] in typed:
            pending = t[:-1]
        elif pending and t == ".cfi_startproc":
            pending = None
        elif pending and (t.startswith(".size ") or (t.endswith(":") and t[:-1] in typed)):
            if pending + ":" not in src:
                bad.append(f"{os.path.basename(fn)}: {pending}: no FDE")
            pending = None
for err in sorted(glob.glob(os.path.join(tmp, "*.err"))):
    for line in open(err, errors="replace"):
        if "lccc: cfi:" in line and ("two frame" in line or "prologue saves announced" in line):
            bad.append(f"{os.path.basename(err)[:-4]}: {line.strip()}")
for fn in sorted(glob.glob(os.path.join(tmp, "*.s"))):
    nc = fn[:-2] + ".nocfi"
    if not os.path.exists(nc):
        continue
    strip = lambda p: [l for l in open(p, errors="replace") if ".cfi_" not in l]
    a, b = strip(fn), strip(nc)
    if a != b:
        i = next((k for k, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        ctx = (a[i] if i < len(a) else "<eof>").strip()
        alt = (b[i] if i < len(b) else "<eof>").strip()
        bad.append(f"{os.path.basename(fn)}: code differs without unwind tables at line {i + 1}: `{ctx}` vs `{alt}`")
if not nfn:
    print("FAIL: no function compiled", file=sys.stderr)
    sys.exit(1)
if bad:
    print(f"FAIL: {len(bad)} CFI invariant violations", file=sys.stderr)
    for b in bad[:40]:
        print("  " + b, file=sys.stderr)
    sys.exit(1)
print(f"PASS: CFI invariants hold ({nfn} functions, {nret} returns)")
PY
