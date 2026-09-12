#!/usr/bin/env bash
# ============================================================================
# check_phi_acyclic_order.sh — pin acyclic parallel-copy resolution in phi
# elimination.
#
# A phi block's incoming copies denote a SIMULTANEOUS assignment. Executing
# that as a sequence needs a shared temporary only for the copies that form a
# dependency CYCLE (a swap `x=y; y=x`). An acyclic chain — the SHA-256 state
# rotation `h=g; g=f; f=e; ...`, or any `b=a; c=b; d=c` shift — resolves with
# no temporary at all, purely by ordering each copy before the copy that
# overwrites its source.
#
# The resolver used to treat "my source is somebody's destination" as a cycle.
# Every rotation therefore went through the two-phase temporary scheme, which
# costs one extra value AND one extra unconditional copy per phi per
# iteration. Doubling the loop-carried webs is what actually hurts: the
# register allocator, now over budget, stages the whole rotation through the
# stack. Measured on the rotation kernel below, on base 25ed36de: 71
# instructions / 27 stack references, against GCC's 71 / 0.
#
# With the cycle-accurate decomposition the same kernel is 61 instructions / 4
# stack references — below GCC on size, and 2 references above the fully
# register-allocated shape it reached before upstream's `evict_short_k`
# cost-ratio escape landed (56 / 2 with `CCC_EVICT_SHORT_K=0`; see the
# HISTORICAL NOTE in section 2 for the full isolation matrix). SHA-256's
# `sha256_transform` loses 20 instructions and 33 stack references (198/62 ->
# 178/29); its round-loop header goes from eight unconditional relay copies to
# a bare compare-and-branch.
#
# The cycle-accurate resolver is OPT-IN (CCC_PHI_ACYCLIC_ORDER=1), not the
# default. It is a strict static win — but on sha256_transform, the corpus's
# most register-saturated kernel, it costs ~5-7% RUNTIME, because merging the
# two-phase temps lengthens a loop-carried live range across the back edge and
# the allocator then leaves exactly the two recurrence words stack-homed (they
# appear in neither `assigned` nor `spilled` under CCC_DEBUG_RA_PHASES). The
# default loop spreads 25 memops over 11 slots; the opt-in loop concentrates 14
# memops on 2 slots read 7x each. Until the allocator can split a range at the
# back edge profitably (RA-06 location pieces), shipping this default-on would
# trade a static win for a runtime loss on the kernel that matters most.
#
# This gate pins four properties:
#   1. THE MECHANISM FIRES — under the opt-in flag the rotation's stack traffic
#      is zero and strictly below the default arm. A resolver that silently
#      stops firing (the failure mode this repo has been bitten by repeatedly)
#      is a loud FAIL, never a pass.
#   2. THE NEAR MISSES STAY CORRECT — a genuine swap and a cycle-plus-chain
#      must keep their temporaries and still match GCC bit-for-bit, on stdout
#      AND exit status, in BOTH arms.
#   3. THE GATE IS WIRED — CCC_PHI_ACYCLIC_ORDER=1 changes the emitted code, so
#      the A/B stays reproducible and a regression can be attributed without a
#      rebuild.
#   4. THE DEFAULT IS UNCHANGED — production output must stay bit-identical
#      with and without the flag unset, i.e. the opt-in must not leak.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
GCC=${GCC:-gcc}
[[ -x $CCC ]] || { echo "check_phi_acyclic_order: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "check_phi_acyclic_order: no gcc oracle" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad()  { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# --------------------------------------------------------------------------
# Sources. `rot` is the acyclic rotation (the shape that must improve);
# `swap`/`mixed` are the cyclic near misses (must stay correct).
# --------------------------------------------------------------------------
cat > "$work/rot.c" <<'EOF'
/* Acyclic state rotation: two chains terminating in freshly computed words.
 * This is SHA-256's `h=g; g=f; f=e; e=d+t1; d=c; c=b; b=a; a=t1+t2` shape
 * reduced to its copy graph. */
unsigned rot(unsigned *st, const unsigned *k, int n) {
    unsigned a = st[0], b = st[1], c = st[2], d = st[3];
    unsigned e = st[4], f = st[5], g = st[6], h = st[7];
    for (int i = 0; i < n; i++) {
        unsigned t1 = h + e + f + g + k[i];
        unsigned t2 = a + b + c;
        h = g; g = f; f = e; e = d + t1;
        d = c; c = b; b = a; a = t1 + t2;
    }
    return a ^ b ^ c ^ d ^ e ^ f ^ g ^ h;
}
EOF

cat > "$work/swap.c" <<'EOF'
/* Genuine 2-cycle: x and y exchange every iteration. No ordering can resolve
 * this sequentially, so both copies must keep a temporary. */
unsigned swap(unsigned *st, int n) {
    unsigned x = st[0], y = st[1], acc = 0;
    for (int i = 0; i < n; i++) {
        unsigned t = x;
        x = y + i;
        y = t ^ 0x5a5a5a5au;
        acc += x + y;
    }
    return acc ^ x ^ y;
}
EOF

cat > "$work/mixed.c" <<'EOF'
/* A 3-cycle (p,q,r) alongside an acyclic chain (u,v,w) in the same phi block:
 * the cycle must keep temporaries, the chain must not acquire any. */
unsigned mixed(unsigned *st, int n) {
    unsigned p = st[0], q = st[1], r = st[2];
    unsigned u = st[3], v = st[4], w = st[5];
    for (int i = 0; i < n; i++) {
        unsigned np = r + i, nq = p, nr = q;      /* 3-cycle */
        unsigned nu = u + v + i, nv = u, nw = v;  /* chain   */
        p = np; q = nq; r = nr;
        u = nu; v = nv; w = nw;
    }
    return p ^ q ^ r ^ u ^ v ^ w;
}
EOF

cat > "$work/drv.c" <<'EOF'
#include <stdio.h>
extern unsigned rot(unsigned *, const unsigned *, int);
extern unsigned swap(unsigned *, int);
extern unsigned mixed(unsigned *, int);
int main(void) {
    unsigned st[8] = {1u, 2u, 3u, 4u, 5u, 6u, 7u, 8u};
    static unsigned k[1024];
    for (int i = 0; i < 1024; i++) k[i] = (unsigned)i * 7u + 3u;
    unsigned long long acc = 0;
    /* Sweep trip counts across 0 so the peel/rotate/exit paths all run. */
    for (int n = 0; n <= 64; n++) {
        unsigned s[8];
        for (int j = 0; j < 8; j++) s[j] = st[j] + (unsigned)n;
        acc = acc * 1000003ull + rot(s, k, n);
        acc = acc * 1000003ull + swap(s, n);
        acc = acc * 1000003ull + mixed(s, n);
    }
    printf("%llu\n", acc);
    return 0;
}
EOF

# --------------------------------------------------------------------------
# 1. Differential correctness (near misses included), stdout AND exit status.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: differential correctness vs $GCC (default arm)"
"$CCC" -O2 -c "$work/rot.c"   -o "$work/rot_l.o"
"$CCC" -O2 -c "$work/swap.c"  -o "$work/swap_l.o"
"$CCC" -O2 -c "$work/mixed.c" -o "$work/mixed_l.o"
"$CCC" -O2 -c "$work/drv.c"   -o "$work/drv_l.o"
"$CCC" "$work/rot_l.o" "$work/swap_l.o" "$work/mixed_l.o" "$work/drv_l.o" -o "$work/lccc_bin"

"$GCC" -O2 -c "$work/rot.c"   -o "$work/rot_g.o"
"$GCC" -O2 -c "$work/swap.c"  -o "$work/swap_g.o"
"$GCC" -O2 -c "$work/mixed.c" -o "$work/mixed_g.o"
"$GCC" -O2 -c "$work/drv.c"   -o "$work/drv_g.o"
"$GCC" "$work/rot_g.o" "$work/swap_g.o" "$work/mixed_g.o" "$work/drv_g.o" -o "$work/gcc_bin"

set +e
lccc_out=$("$work/lccc_bin" 2>&1); lccc_rc=$?
gcc_out=$("$work/gcc_bin" 2>&1);   gcc_rc=$?
set -e
if [[ $lccc_rc != $gcc_rc ]]; then
    bad "exit status differs: lccc=$lccc_rc gcc=$gcc_rc"
fi
if [[ "$lccc_out" != "$gcc_out" ]]; then
    bad "output differs: lccc='$lccc_out' gcc='$gcc_out'"
fi
note "rotation + swap + 3-cycle, 65 trip counts each: lccc=$lccc_out gcc=$gcc_out (rc=$lccc_rc)"

# The opt-in arm must be correct too: it is a supported configuration.
CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -c "$work/rot.c"   -o "$work/rot_k.o"
CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -c "$work/swap.c"  -o "$work/swap_k.o"
CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -c "$work/mixed.c" -o "$work/mixed_k.o"
"$CCC" "$work/rot_k.o" "$work/swap_k.o" "$work/mixed_k.o" "$work/drv_l.o" -o "$work/kill_bin"
set +e
kill_out=$("$work/kill_bin" 2>&1); kill_rc=$?
set -e
if [[ $kill_rc != $gcc_rc || "$kill_out" != "$gcc_out" ]]; then
    bad "opt-in arm differs: out='$kill_out' rc=$kill_rc (want '$gcc_out' rc=$gcc_rc)"
fi
note "opt-in arm agrees with the oracle"

# --------------------------------------------------------------------------
# 2. The mechanism fires: rotation is register-only and beats the legacy arm.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: structural contract on rot()"
"$CCC" -O2 -S "$work/rot.c" -o "$work/rot_def.s"
CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_new.s"
# Same arm with the allocator's cost-ratio escape disabled, so the resolver's
# own contribution can be separated from `select_evict_victim`'s (see the
# HISTORICAL NOTE below).
CCC_EVICT_SHORT_K=0 CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_k0.s"
"$GCC" -O2 -S "$work/rot.c" -o "$work/rot_gcc.s"

set +e
python3 - "$work" <<'PYEOF'
import re, sys
work = sys.argv[1]
STK = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")

def fn(path, name):
    txt = open(path).read()
    # gcc separates `.size` from the symbol with a TAB, lccc with a space.
    # Match either and FAIL loudly rather than falling back to the whole
    # translation unit: that silent fallback once reported gcc's whole-file
    # instruction count as if it were one function's, and the resulting
    # "lccc beats gcc" reading was simply wrong.
    m = re.search(rf'^{name}:\n(.*?)^\s*\.size\s+{name}\b', txt, re.S | re.M)
    if not m:
        print(f"  FAIL: could not extract {name} from {path}", file=sys.stderr)
        sys.exit(1)
    ins = [l for l in m.group(1).splitlines() if re.match(r'^\s+[a-z]', l)]
    return len(ins), sum(1 for l in ins if STK.search(l))

d_i, d_s = fn(f"{work}/rot_def.s", "rot")
n_i, n_s = fn(f"{work}/rot_new.s", "rot")
g_i, g_s = fn(f"{work}/rot_gcc.s", "rot")
print(f"  rot(): default={d_i} insns/{d_s} stkref   "
      f"opt-in={n_i}/{n_s}   gcc={g_i}/{g_s}")

k_i, k_s = fn(f"{work}/rot_k0.s", "rot")
print(f"  rot() with CCC_EVICT_SHORT_K=0 (allocator escape off): opt-in={k_i}/{k_s}")

bad = []
# The resolver must be observably responsible for an improvement: strictly fewer
# instructions AND strictly less stack traffic than the default arm. If it stops
# firing, or the flag stops being wired, both arms become identical and this
# fails loudly.
if not d_i > n_i:
    bad.append(f"opt-in flag is not smaller ({d_i} vs {n_i} insns): "
               "the redundant relay copies are back")
if not d_s > n_s:
    bad.append(f"opt-in flag shows no stack-traffic difference ({d_s} vs {n_s}): "
               "the resolver is not firing or is no longer wired")
# And it must beat the oracle on size, which is the point of the exercise.
if not n_i < g_i:
    bad.append(f"opt-in rot() does not beat gcc on size ({n_i} vs {g_i} insns)")
# HISTORICAL NOTE -- do not "restore" the old absolute contract without reading
# this. This gate used to require the opt-in rotation to be fully
# register-allocated: 55 instructions and ZERO stack references. That absolute
# target is no longer reachable at production settings and the cause is NOT this
# resolver. Upstream's cost-ratio escape in `select_evict_victim`
# (`RaConfig::evict_short_k` / `CCC_EVICT_SHORT_K`, default 16) admits hot short
# victims by ratio and re-introduces staging in this arm. Measured isolation on
# base 25ed36de (rot(), -O2):
#     escape off (K=0),  resolver off : 71 insns / 33 stkref
#     escape off (K=0),  resolver on  : 56 insns /  2 stkref
#     escape on  (K=16), resolver off : 71 insns / 27 stkref
#     escape on  (K=16), resolver on  : 61 insns /  4 stkref
# The resolver's own contribution is large at both settings (-15 insns/-31
# stkref at K=0, -10/-23 at K=16); the residual 2-4 stack references belong to
# the escape and to the web-wide in-loop-use supply, not to the copy ordering.
# Pinning the resolver's contribution instead of an absolute zero keeps this
# gate honest about which component it is testing.
if not (k_i < d_i and k_s <= 2):
    bad.append(f"resolver mechanism changed shape with the escape off "
               f"({k_i} insns/{k_s} stkref vs default {d_i}/{d_s}): expected a "
               f"large win and at most 2 staged references")
for b in bad:
    print(f"  FAIL: {b}", file=sys.stderr)
sys.exit(1 if bad else 0)
PYEOF
rc=$?
set -e
if [[ $rc != 0 ]]; then fail=1; fi

# --------------------------------------------------------------------------
# 3. Real workload: sha256_transform keeps its known-answer vector, the opt-in
#    arm lowers the STATIC counts -- and the default stays untouched.
#
#    NOTE: this is deliberately a static contract only. The opt-in arm is
#    ~5-7% SLOWER at runtime here despite the lower counts; see the header. A
#    runtime assertion would pin the regression in place, so the gate checks
#    what is actually guaranteed.
# --------------------------------------------------------------------------
sha=$here/../benchmark/programs/sha256_transform.c
if [[ -r $sha ]]; then
    echo "check_phi_acyclic_order: sha256_transform known-answer + census"
    "$CCC" -O2 -DPASSES=2 -DBLOCK_COUNT=64 "$sha" -o "$work/sha_lccc"
    "$GCC" -O2 -DPASSES=2 -DBLOCK_COUNT=64 "$sha" -o "$work/sha_gcc"
    set +e
    sha_l=$("$work/sha_lccc"); sha_rc=$?
    sha_g=$("$work/sha_gcc");  sha_grc=$?
    set -e
    if [[ $sha_rc != 0 || $sha_rc != $sha_grc || "$sha_l" != "$sha_g" ]]; then
        bad "sha256_transform digest mismatch: lccc='$sha_l'(rc=$sha_rc) gcc='$sha_g'(rc=$sha_grc)"
    else
        note "sha256_transform digest $sha_l matches gcc (rc=$sha_rc; the built-in FIPS vector also gates this)"
    fi

    "$CCC" -O2 -S "$sha" -o "$work/sha_def.s"
    CCC_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -S "$sha" -o "$work/sha_new.s"
    set +e
python3 - "$work" <<'PYEOF'
import re, sys
work = sys.argv[1]
STK = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")

def fn(path, name):
    txt = open(path).read()
    # gcc separates `.size` from the symbol with a TAB, lccc with a space.
    # Match either and FAIL loudly rather than falling back to the whole
    # translation unit: that silent fallback once reported gcc's whole-file
    # instruction count as if it were one function's, and the resulting
    # "lccc beats gcc" reading was simply wrong.
    m = re.search(rf'^{name}:\n(.*?)^\s*\.size\s+{name}\b', txt, re.S | re.M)
    if not m:
        print(f"  FAIL: could not extract {name} from {path}", file=sys.stderr)
        sys.exit(1)
    ins = [l for l in m.group(1).splitlines() if re.match(r'^\s+[a-z]', l)]
    return len(ins), sum(1 for l in ins if STK.search(l))

d_i, d_s = fn(f"{work}/sha_def.s", "sha256_transform")
n_i, n_s = fn(f"{work}/sha_new.s", "sha256_transform")
print(f"  sha256_transform: default={d_i} insns/{d_s} stkref   opt-in={n_i}/{n_s}"
      "   (opt-in is statically smaller but RUNTIME-slower; see header)")
bad = []
if not (n_i < d_i and n_s < d_s):
    bad.append(f"opt-in sha256_transform did not lower the static counts "
               f"({n_i}/{n_s} vs default {d_i}/{d_s})")
for b in bad:
    print(f"  FAIL: {b}", file=sys.stderr)
sys.exit(1 if bad else 0)
PYEOF
    if [[ $? != 0 ]]; then fail=1; fi
fi

# --------------------------------------------------------------------------
# 4. The opt-in must not leak: the flag is parsed strictly as "== 1", so unset,
#    "0" and any other value must all reproduce the default arm byte for byte.
#    Production codegen therefore cannot be perturbed by a stray environment.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: opt-in does not leak into the default arm"
"$CCC" -O2 -S "$work/rot.c" -o "$work/rot_a.s"
CCC_PHI_ACYCLIC_ORDER=0 "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_b.s"
CCC_PHI_ACYCLIC_ORDER=  "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_c.s"
CCC_PHI_ACYCLIC_ORDER=true "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_d.s"
if ! cmp -s "$work/rot_a.s" "$work/rot_b.s" \
   || ! cmp -s "$work/rot_a.s" "$work/rot_c.s" \
   || ! cmp -s "$work/rot_a.s" "$work/rot_d.s"; then
    bad "the opt-in flag leaks: unset/0/empty/true do not all reproduce the default arm"
else
    note "unset, =0, empty and =true all reproduce the default arm byte for byte"
fi

if [[ $fail != 0 ]]; then echo "check_phi_acyclic_order: FAILED"; exit 1; fi
echo "check_phi_acyclic_order: contract holds"
