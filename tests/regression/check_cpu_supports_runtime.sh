#!/usr/bin/env bash
# AUD-CPU-1 gate: the semantics of `__builtin_cpu_supports`.
#
# The defect this pins (measured against GCC 16.2, Clang 23.1 and ICX): LCCC
# used to fold the builtin against the *effective target* feature set, so
#
#   * `-march=x86-64-v3` (or the default) answered 1 for AVX2/BMI2/SHA on a
#     CPU without them, and `-march=x86-64` answered 0 for a CPU that has
#     them — every runtime dispatcher in zlib-ng/zstd/xz/OpenSSL/FFmpeg took
#     the wrong path (SIGILL, or a needlessly slow path);
#   * the four constant contexts GCC/Clang/ICX all reject (array bound, case
#     label, static initializer, enumerator) were silently accepted.
#
# GCC, Clang and ICX all lower the builtin to a *runtime* CPU model lookup for
# every name and every flag combination, including target-guaranteed ones, so
# this gate checks:
#
#   1. name space: every name the recorded GCC census accepts compiles, and
#      nonsense names are hard errors with GCC's exact wording (no silent 0 —
#      that was AUD-SILENT-ZERO-1 in a new place);
#   2. constant contexts: all four are rejected with GCC's exact wording, and
#      valid constant expressions still compile (no false positives);
#   3. runtime truth: for every name GCC accepts, the LCCC-built and the
#      GCC-built probe agree on the *executing* host, under the default
#      target, `-march=x86-64-v3` (which guarantees AVX2) and
#      `-march=raptorlake` (which guarantees AVX-VNNI) — the three flag sets
#      where the old fold produced host-independent answers;
#   4. assembly contract: the probe function contains a real CPUID sequence
#      (never a folded constant answer) in all three flag sets;
#   5. the i686 helper flavour answers the same as GCC -m32, and the
#      AArch64/RISC-V backends still compile the builtin (conservative 0).
#
# No network: the census is the recorded oracle evidence
# (`tests/data/cpu_supports_name_census.json`, regenerate with
# `scripts/cpu_supports_name_census.py`).
set -uo pipefail

CCC=$(realpath "${CCC:-./target/fastbuild/lccc}")
CCC_I686=${LCCC_I686:-$(dirname "$CCC")/lccc-i686}
CCC_ARM=${LCCC_ARM:-$(dirname "$CCC")/lccc-arm}
CCC_RISCV=${LCCC_RISCV:-$(dirname "$CCC")/lccc-riscv}
REPO=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$REPO"
CENSUS=tests/data/cpu_supports_name_census.json
GCC=${GCC_BIN:-gcc}
td=$(mktemp -d)
trap 'rm -rf "$td"' EXIT
fail=0
ok()  { printf 'ok   %s\n' "$*"; }
bad() { printf 'FAIL %s\n' "$*"; fail=1; }

[ -x "$CCC" ] || { echo "FAIL: no compiler at $CCC"; exit 1; }
[ -f "$CENSUS" ] || { echo "FAIL: missing $CENSUS"; exit 1; }

# ---------------------------------------------------------------- 1. name space
if python3 scripts/cpu_supports_name_census.py --verify-json "$CENSUS" --lccc-src . \
        >"$td/census.out" 2>&1; then
    ok "census matches src/common/cpu_probe.rs"
else
    bad "census/source drift"; sed 's/^/      /' "$td/census.out"
fi

python3 - "$CENSUS" "$td" <<'PY'
import json, sys
census = json.load(open(sys.argv[1]))
td = sys.argv[2]
strict = census.get("strict", ["gcc16.2"])
names = sorted({n for c in strict for n in census["compilers"][c]["accepted"]})
with open(f"{td}/accept.c", "w") as f:
    for i, n in enumerate(names):
        f.write(f'int a{i}(void) {{ return __builtin_cpu_supports("{n}"); }}\n')
    f.write("int main(void) { return 0; }\n")
with open(f"{td}/values.c", "w") as f:
    f.write("#include <stdio.h>\nint main(void) {\n")
    for n in names:
        f.write(f'    printf("{n}=%d\\n", !!__builtin_cpu_supports("{n}"));\n')
    f.write("    return 0;\n}\n")
with open(f"{td}/names.txt", "w") as f:
    f.write("\n".join(names) + "\n")
PY
gcc_names=$(wc -l < "$td/names.txt")

if "$CCC" -O2 -c "$td/accept.c" -o "$td/accept.o" 2>"$td/accept.err"; then
    ok "all $gcc_names GCC-accepted names compile"
else
    bad "GCC-accepted name rejected"; head -3 "$td/accept.err" | sed 's/^/      /'
fi

for n in not-a-real-feature avx9 sse5; do
    printf 'int f(void) { return __builtin_cpu_supports("%s"); }\n' "$n" > "$td/bogus.c"
    if "$CCC" -c "$td/bogus.c" -o /dev/null 2>"$td/bogus.err"; then
        bad "unknown name '$n' was silently accepted"
    elif grep -q "parameter to builtin not valid: $n" "$td/bogus.err"; then
        ok "unknown name '$n' rejected with GCC's wording"
    else
        bad "unknown name '$n' rejected with the wrong message"
        head -2 "$td/bogus.err" | sed 's/^/      /'
    fi
done

# ------------------------------------------------------- 2. constant contexts
cat >"$td/ctxt_array.c" <<'EOF'
int a[__builtin_cpu_supports("avx2") ? 2 : 1];
int main(void) { return (int)sizeof(a); }
EOF
cat >"$td/ctxt_case.c" <<'EOF'
int f(int x) { switch (x) { case __builtin_cpu_supports("avx2"): return 1; default: return 0; } }
int main(void) { return f(0); }
EOF
cat >"$td/ctxt_init.c" <<'EOF'
static int v = __builtin_cpu_supports("avx2");
int main(void) { return v; }
EOF
cat >"$td/ctxt_enum.c" <<'EOF'
enum { E = __builtin_cpu_supports("avx2") };
int main(void) { return E; }
EOF
ctxt_row() {  # file, expected wording
    local f=$1 want=$2
    if "$CCC" -O2 -c "$td/$f" -o /dev/null 2>"$td/ctxt.err"; then
        bad "$f: constant context accepted (GCC/Clang/ICX reject it)"
    elif grep -qF "$want" "$td/ctxt.err"; then
        ok "$f rejected: $want"
    else
        bad "$f: rejected with the wrong message"
        head -2 "$td/ctxt.err" | sed 's/^/      /'
    fi
}
ctxt_row ctxt_array.c "variably modified 'a' at file scope"
ctxt_row ctxt_case.c  "case label does not reduce to an integer constant"
ctxt_row ctxt_init.c  "initializer element is not constant"
ctxt_row ctxt_enum.c  "enumerator value for 'E' is not an integer constant"

cat >"$td/valid_ctx.c" <<'EOF'
enum { A = 1, B = A + 2, C = (int)sizeof(long) };
int b[C * 4];
int c[3 + 4 * 2];
struct S { int x; char pad[8]; } s = { 1 };
int main(void) {
    switch (b[0]) { case 11: return 0; case 3 ... 5: return 1; default: return 2; }
}
EOF
if "$CCC" -O2 "$td/valid_ctx.c" -o "$td/valid_ctx" 2>"$td/valid.err"; then
    ok "valid constant contexts still compile"
else
    bad "valid constant contexts rejected (false positive)"; head -3 "$td/valid.err" | sed 's/^/      /'
fi

# --------------------------------------------------------- 3. runtime truth
value_row() {  # description, extra flags...
    local desc=$1; shift
    local flags=("$@")
    if ! "$CCC" -O2 "${flags[@]}" "$td/values.c" -o "$td/probe_l" 2>"$td/l.err"; then
        bad "$desc: LCCC compile failed"; head -3 "$td/l.err" | sed 's/^/      /'; return
    fi
    "$td/probe_l" >"$td/out_l"
    # GCC folds nothing either, but a *name* it does not know is a hard error,
    # so drop exactly those until it compiles (GCC reports all of them at once).
    cp "$td/values.c" "$td/values_gcc.c"
    local -a gcc_flags=("${flags[@]}")
    if [ ${#gcc_flags[@]} -gt 0 ] && ! "$GCC" "${gcc_flags[@]}" -xc -c /dev/null -o /dev/null 2>/dev/null; then
        # GCC 12 predates -march=raptorlake; alderlake is the same Golden Cove
        # / Gracemont ISA (including AVX-VNNI) for the host oracle build.
        gcc_flags=(-march=alderlake)
    fi
    local attempt
    for attempt in 1 2 3 4 5; do
        if "$GCC" -O2 "${gcc_flags[@]}" "$td/values_gcc.c" -o "$td/probe_g" 2>"$td/g.err"; then
            break
        fi
        grep -o 'parameter to builtin not valid: [A-Za-z0-9._-]*' "$td/g.err" \
            | awk '{print $NF}' | sort -u > "$td/unknown.txt"
        [ -s "$td/unknown.txt" ] || { bad "$desc: gcc rejected values.c"; head -3 "$td/g.err" | sed 's/^/      /'; return; }
        python3 - "$td/unknown.txt" "$td/values_gcc.c" <<'PY'
import sys
unknown = set(open(sys.argv[1]).read().split())
path = sys.argv[2]
keep = [ln for ln in open(path) if not any(f'"{u}"' in ln for u in unknown)]
open(path, "w").writelines(keep)
PY
    done
    if [ ! -x "$td/probe_g" ]; then bad "$desc: gcc probe unavailable"; return; fi
    "$td/probe_g" >"$td/out_g"
    # Names the local GCC does not know (avx10.1/avx10.2/movrs on GCC 14)
    # cannot be compared, so restrict both outputs to the names in the GCC TU.
    if python3 - "$td/values_gcc.c" "$td/out_l" "$td/out_g" <<'PY'
import re, sys
src, out_l, out_g = sys.argv[1:]
known = set(re.findall(r'printf\("([^"=]+)=', open(src).read()))
def filt(path):
    return [ln for ln in open(path) if ln.split("=")[0] in known]
lo, go = filt(out_l), filt(out_g)
if lo != go:
    for a, b in zip(lo, go):
        if a != b:
            print(f"      lccc {a.strip()} vs gcc {b.strip()}")
    sys.exit(1)
print(f"{len(lo)}")
PY
    then
        ok "$desc: LCCC and GCC agree ($(wc -l < "$td/out_g") common names)"
    else
        bad "$desc: runtime answers differ from GCC"
    fi
    # 4. assembly contract, same flags
    "$CCC" -O2 "${flags[@]}" -S "$td/values.c" -o "$td/values.s" 2>/dev/null
    if grep -q 'cpuid' "$td/values.s" && grep -q '__lccc_cpu_init' "$td/values.s"; then
        ok "$desc: probe is a runtime CPUID sequence ($(grep -c cpuid "$td/values.s") cpuid)"
    else
        bad "$desc: no runtime CPUID probe in the emitted assembly"
    fi
}
value_row "default target"
value_row "-march=x86-64-v3 (AVX2 guaranteed)" -march=x86-64-v3
value_row "-march=raptorlake (AVX-VNNI guaranteed)" -march=raptorlake

# --------------------------------------------------------- 5. other targets
cat >"$td/i686.c" <<'EOF'
#include <stdio.h>
int main(void) {
    printf("%d %d %d\n", !!__builtin_cpu_supports("sse2"),
                         !!__builtin_cpu_supports("avx2"),
                         !!__builtin_cpu_supports("avx512f"));
    return 0;
}
EOF
if [ -x "$CCC_I686" ]; then
    if "$GCC" -m32 -xc /dev/null -o "$td/m32_probe" 2>/dev/null; then
        if "$CCC_I686" -O2 "$td/i686.c" -o "$td/i686_l" 2>"$td/i686.err"; then
            "$td/i686_l" >"$td/i686_l.txt"
            "$GCC" -m32 -O2 "$td/i686.c" -o "$td/i686_g" 2>/dev/null
            "$td/i686_g" >"$td/i686_g.txt"
            if diff -q "$td/i686_l.txt" "$td/i686_g.txt" >/dev/null; then
                ok "i686: LCCC and GCC -m32 agree ($(cat "$td/i686_l.txt"))"
            else
                bad "i686: runtime answers differ from GCC -m32"
                diff "$td/i686_l.txt" "$td/i686_g.txt" | sed -n '1,4p' | sed 's/^/      /'
            fi
        else
            bad "i686: compile failed"; head -3 "$td/i686.err" | sed 's/^/      /'
        fi
    else
        # Host has no 32-bit multilib libc/libgcc: verify 32-bit PIC CPUID
        # probe execution via a freestanding _start binary against the 64-bit
        # GCC oracle answers from out_g.  A host whose kernel or seccomp
        # policy refuses ia32 syscalls kills every int $0x80 binary with
        # SIGSYS before the program's first instruction; probe that with the
        # REFERENCE GCC first so the skip measures the environment, not the
        # compiler under test, and the leg skips loudly instead of failing
        # as a mystery exit 159.
        printf 'void _start(void){ __asm__ volatile("movl $0, %%ebx; movl $1, %%eax; int $0x80"); }\n' >"$td/ia32probe.c"
        if ! { "$GCC" -m32 -nostdlib "$td/ia32probe.c" -o "$td/ia32probe" >/dev/null 2>&1 &&
               "$td/ia32probe" >/dev/null 2>&1; }; then
            echo "SKIP i686 freestanding row (host cannot execute ia32 int \$0x80 binaries; CI's runner executes them)"
        else
        sse2_exp=$(awk -F= '$1=="sse2"{print $2}' "$td/out_g")
        avx2_exp=$(awk -F= '$1=="avx2"{print $2}' "$td/out_g")
        avx512f_exp=$(awk -F= '$1=="avx512f"{print $2}' "$td/out_g")
        cat >"$td/i686_freestanding.c" <<EOF
void _start(void) {
    int ok = (!!__builtin_cpu_supports("sse2") == ${sse2_exp:-1})
          && (!!__builtin_cpu_supports("avx2") == ${avx2_exp:-1})
          && (!!__builtin_cpu_supports("avx512f") == ${avx512f_exp:-0});
    int rc = ok ? 0 : 1;
    __asm__ volatile ("int \$0x80" :: "a"(1), "b"(rc));
}
EOF
        if "$CCC_I686" -O2 -fPIC -nostdlib "$td/i686_freestanding.c" -o "$td/i686_l" 2>"$td/i686.err" && "$td/i686_l"; then
            ok "i686: freestanding PIC runtime probe agrees with host CPUID (${sse2_exp:-1} ${avx2_exp:-1} ${avx512f_exp:-0})"
        else
            bad "i686: freestanding runtime probe failed"; head -3 "$td/i686.err" | sed 's/^/      /'
        fi
        fi
    fi
else
    echo "SKIP i686 rows (no $CCC_I686)"
fi

cat >"$td/cross.c" <<'EOF'
int f(void) { return __builtin_cpu_supports("avx2"); }
int main(void) { return f(); }
EOF
# AUD-CPU-1 (3): an x86 feature NAME on a target that cannot answer it is a
# diagnostic, not a silent 0 -- GCC's non-x86 builtin accepts only its own
# architecture's names, and folding `avx2` to 0 turns a dispatch test into a
# silent slow path.  Names from other architectures' sets keep the conservative
# 0 (below), because LCCC does not model those names yet and rejecting them
# would turn accepted GCC code into a compile error.
for pair in "aarch64:$CCC_ARM" "riscv64:$CCC_RISCV"; do
    arch=${pair%%:*}; cc=${pair#*:}
    if [ -x "$cc" ]; then
        if "$cc" -O2 -c "$td/cross.c" -o /dev/null 2>"$td/cross.err"; then
            bad "$arch: an x86 feature name compiled (expected a diagnostic)"
        elif grep -qF "parameter to builtin not valid: avx2" "$td/cross.err"; then
            ok "$arch: x86 feature name diagnosed, not folded"
        else
            bad "$arch: not diagnosed with the builtin's own wording"
            head -3 "$td/cross.err" | sed 's/^/      /'
        fi
        # A name this backend does not model (GCC's per-architecture set) still
        # folds to the conservative 0 rather than being rejected.
        cat >"$td/cross_other.c" <<'EOF'
int f(void) { return __builtin_cpu_supports("crypto"); }
EOF
        if "$cc" -O2 -c "$td/cross_other.c" -o /dev/null 2>"$td/cross_other.err"; then
            ok "$arch: an unmodelled architecture-specific name still folds"
        else
            bad "$arch: an unmodelled name is rejected"
            head -3 "$td/cross_other.err" | sed 's/^/      /'
        fi
    else
        echo "SKIP $arch rows (no $cc)"
    fi
done

# ------------------------------------------------ 6. initialiser contract
# The runtime initialiser is materialised once per probing function, behind an
# inline test of the ready word.  The old form — an unconditional
# `call __lccc_cpu_init` at the end of the entry block — cost a call, the
# initialiser's prologue and a caller-side register save on *every* probe
# (callgrind: 27 instructions per probe call in a leaf function, against 12
# for GCC's constructor-based model; see tests/workloads/dispatch/run.sh).
# These checks pin the guarded form so the regression cannot come back
# silently.
cat >"$td/guard.c" <<'EOF'
int one(void) { return __builtin_cpu_supports("avx2"); }
int two(int a, int b) {
    return (__builtin_cpu_supports("avx2") ? a : 0)
         + (__builtin_cpu_supports("bmi2") ? b : 0);
}
int untouched(int x) { return x + 1; }
int main(void) { return one() + two(1, 2) + untouched(0); }
EOF
if "$CCC" -O2 -S "$td/guard.c" -o "$td/guard.s" 2>"$td/guard.err"; then
    calls=$(grep -c 'call[[:space:]]*__lccc_cpu_init' "$td/guard.s" || true)
    if [ "$calls" = 2 ]; then
        ok "one initialiser call per probing function (2 functions, 2 calls)"
    else
        bad "expected 2 initialiser calls (one, two), found ${calls:-0}"
    fi
    if python3 - "$td/guard.s" <<'PY'
import re, sys
text = open(sys.argv[1]).read().splitlines()
calls = [n for n, l in enumerate(text)
         if re.search(r'^[ \t]*call[ \t]+__lccc_cpu_init', l)]
if not calls:
    sys.exit(1)
for n in calls:
    label = None
    for m in range(n - 1, -1, -1):
        lm = re.match(r'^([.\w$]+):', text[m])
        if lm:
            label = lm.group(1)
            break
    if label is None:
        sys.exit(1)
    jumps = [k for k, l in enumerate(text)
             if re.search(r'\bj\w+[ \t]+' + re.escape(label) + r'\b', l)]
    if not jumps:
        sys.exit(1)
    window = " ".join(text[max(0, jumps[0] - 3):jumps[0] + 1])
    if "__lccc_cpu_ready" not in window:
        sys.exit(1)
PY
    then
        ok "every initialiser call sits behind a conditional jump on __lccc_cpu_ready"
    else
        bad "initialiser call is not guarded by the ready word (hot-path cost regressed)"
    fi
else
    bad "guard contract probe did not compile"; head -3 "$td/guard.err" | sed 's/^/      /'
fi

# The same contract, at runtime, in eight function shapes (loop body, nested
# conditional, goto loop, two probe sites, probe as first statement, eight
# arguments all live across the initialiser, an otherwise-leaf helper,
# recursion).  The program derives its expectations from top-level probes, so
# the check is host-independent and the GCC comparison is exact.
if "$CCC" -O2 tests/regression/builtin_cpu_supports_guard.c -o "$td/guard_l" 2>"$td/gl.err"; then
    "$td/guard_l" >"$td/guard_l.txt"; rc_l=$?
    if "$GCC" -O2 tests/regression/builtin_cpu_supports_guard.c -o "$td/guard_g" 2>/dev/null; then
        "$td/guard_g" >"$td/guard_g.txt"; rc_g=$?
        if diff -q "$td/guard_l.txt" "$td/guard_g.txt" >/dev/null && [ "$rc_l" = "$rc_g" ]; then
            ok "guard shapes: output and exit status match GCC ($(cat "$td/guard_l.txt"))"
        else
            bad "guard shapes: output or exit status differs from GCC (lccc rc=$rc_l gcc rc=$rc_g)"
            diff "$td/guard_l.txt" "$td/guard_g.txt" | sed -n '1,4p' | sed 's/^/      /'
        fi
    else
        ok "guard shapes: compiled and ran ($(cat "$td/guard_l.txt")), no gcc to compare"
    fi
else
    bad "guard shapes: compile failed"; head -3 "$td/gl.err" | sed 's/^/      /'
fi


if [ "$fail" != 0 ]; then
    echo "cpu-supports runtime gate: FAIL"
    exit 1
fi
echo "cpu-supports runtime gate: PASS"
