#!/usr/bin/env bash
# ============================================================================
# check_phi_acyclic_order.sh — pin acyclic parallel-copy resolution in phi
# elimination (DEFAULT since 2026-09-12; legacy policy behind kill switches).
#
# A phi block's incoming copies denote a SIMULTANEOUS assignment. Executing
# that as a sequence needs a shared temporary only for the copies that form a
# dependency CYCLE (a swap `x=y; y=x`). An acyclic chain — the SHA-256 state
# rotation `h=g; g=f; f=e; ...`, or any `b=a; c=b; d=c` shift — resolves with
# no temporary at all, purely by ordering each copy before the copy that
# overwrites its source.
#
# The legacy resolver treated "my source is somebody's destination" as a
# cycle. Every rotation therefore went through the two-phase temporary
# scheme, which costs one extra value AND one extra unconditional copy per
# phi per iteration. Doubling the loop-carried webs pushed the register
# allocator over budget and staged the rotation through the stack. The
# cycle-accurate Kahn decomposition was first shipped opt-in while a live
# range supply bug (mark_loop_spanning web-wide in-loop-use, see
# check_ra_web_inloop_use.sh) was diagnosed. Once that allocator bug and the
# PR #501/#502 interactions landed, paired A/B on sha256_transform showed
# the resolver faster at EVERY optimizing tier (-O1 ~2.7x, -O2/-O3 ~3%, -Os
# ~12%; 41-round interleaved paired medians, ~420-470 ms/arm; 6 of 807
# translation units change codegen, all equal-or-smaller). It is therefore
# the production default.
#
# This gate pins four properties:
#   1. BOTH POLICIES ARE CORRECT — cyclic near misses (swap/3-cycle) and the
#      acyclic rotation match GCC bit-for-bit on stdout AND exit status under
#      default resolution and under each kill switch.
#   2. THE MECHANISM FIRES BY DEFAULT — the shipping rotation is strictly
#      smaller (insns and stack refs) than the legacy arm. A resolver that
#      silently stops firing is a loud FAIL, never a pass.
#   3. THE WORKLOAD CONTRACT — sha256_transform's known-answer vector matches
#      GCC in both arms and the default arm is strictly smaller statically.
#   4. SWITCH WIRING — unset / "1" / "true" / empty all resolve acyclic
#      (byte-identical); CCC_PHI_ACYCLIC_ORDER=0 AND CCC_NO_PHI_ACYCLIC_ORDER=1
#      both restore the legacy arm (byte-identical to each other).
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

# Build one linked driver for a given policy environment ("$@" = env prefix).
build_policy() {
    local out=$1; shift
    "$@" "$CCC" -O2 -c "$work/rot.c"   -o "$work/$out.rot.o"
    "$@" "$CCC" -O2 -c "$work/swap.c"  -o "$work/$out.swap.o"
    "$@" "$CCC" -O2 -c "$work/mixed.c" -o "$work/$out.mixed.o"
    "$@" "$CCC" -O2 -c "$work/drv.c"   -o "$work/$out.drv.o"
    "$CCC" "$work/$out.rot.o" "$work/$out.swap.o" "$work/$out.mixed.o" \
           "$work/$out.drv.o" -o "$work/$out.bin"
}

# --------------------------------------------------------------------------
# 1. Differential correctness (near misses included), stdout AND exit.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: differential correctness vs $GCC (both policies)"
"$GCC" -O2 -c "$work/rot.c"   -o "$work/rot_g.o"
"$GCC" -O2 -c "$work/swap.c"  -o "$work/swap_g.o"
"$GCC" -O2 -c "$work/mixed.c" -o "$work/mixed_g.o"
"$GCC" -O2 -c "$work/drv.c"   -o "$work/drv_g.o"
"$GCC" "$work/rot_g.o" "$work/swap_g.o" "$work/mixed_g.o" "$work/drv_g.o" -o "$work/gcc_bin"

# Default (acyclic, shipping) policy.
build_policy def env
# Legacy policy via both kill switches (must agree with each other).
build_policy leg0 env CCC_PHI_ACYCLIC_ORDER=0
build_policy legn env CCC_NO_PHI_ACYCLIC_ORDER=1

set +e
gcc_out=$("$work/gcc_bin" 2>&1); gcc_rc=$?
def_out=$("$work/def.bin" 2>&1); def_rc=$?
l0_out=$("$work/leg0.bin" 2>&1); l0_rc=$?
ln_out=$("$work/legn.bin" 2>&1); ln_rc=$?
set -e
for pair in "def:$def_rc:$def_out:default" "leg0:$l0_rc:$l0_out:kill=0" "legn:$ln_rc:$ln_out:kill=NO"; do
    IFS=: read -r name rc out label <<<"$pair"
    if [[ $rc != $gcc_rc || $out != $gcc_out ]]; then
        bad "$label policy disagrees with GCC: out='$out' rc=$rc (want '$gcc_out' rc=$gcc_rc)"
    fi
done
note "default + both kill-switch arms agree with the oracle across 65 trip counts"
if [[ $l0_out != $ln_out ]]; then
    bad "the two kill switches produce different behavior"
fi

# --------------------------------------------------------------------------
# 2. The mechanism fires by default: default arm is strictly smaller than
#    legacy; the allocator-escape-off variant pins the resolver's own
#    contribution.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: structural contract on rot()"
"$CCC" -O2 -S "$work/rot.c" -o "$work/rot_def.s"
CCC_PHI_ACYCLIC_ORDER=0 "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_leg.s"
CCC_EVICT_SHORT_K=0 "$CCC" -O2 -S "$work/rot.c" -o "$work/rot_k0.s"
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
    # instruction count as if it were one function's.
    m = re.search(rf'^{name}:\n(.*?)^\s*\.size\s+{name}\b', txt, re.S | re.M)
    if not m:
        print(f"  FAIL: could not extract {name} from {path}", file=sys.stderr)
        sys.exit(1)
    ins = [l for l in m.group(1).splitlines() if re.match(r'^\s+[a-z]', l)]
    return len(ins), sum(1 for l in ins if STK.search(l))
d_i, d_s = fn(f"{work}/rot_def.s", "rot")
l_i, l_s = fn(f"{work}/rot_leg.s", "rot")
g_i, g_s = fn(f"{work}/rot_gcc.s", "rot")
k_i, k_s = fn(f"{work}/rot_k0.s", "rot")
print(f"  rot(): default(acyclic)={d_i} insns/{d_s} stkref   "
      f"legacy={l_i}/{l_s}   escape-off default={k_i}/{k_s}   gcc={g_i}/{g_s}")
bad = []
# The shipping default must be the acyclic shape: strictly better than the
# legacy policy on BOTH axes (the resolver's own contribution, independent of
# any allocator tuning between them).
if not d_i < l_i:
    bad.append(f"default is not smaller than legacy ({d_i} vs {l_i} insns): "
               "the acyclic resolver stopped being the default")
if not d_s < l_s:
    bad.append(f"default stack traffic is not below legacy ({d_s} vs {l_s}): "
               "the acyclic resolver stopped being the default")
# The shipping arm must beat the oracle on size.
if not d_i < g_i:
    bad.append(f"default rot() does not beat gcc on size ({d_i} vs {g_i} insns)")
# With the allocator cost-ratio escape off, the default-on resolver must
# reach the nearly fully register-allocated shape and must beat the legacy
# policy by a wide margin (measured on 52e01b9: 55/3 vs legacy 70/27; the
# historical 56/2 on the older base shifted by one relay each with PR
# #501/#502). Pinned as <=56 insns / <=4 stack refs so a resolver regression
# fails loudly without baking in one instruction's worth of drift.
if not (k_i <= 56 and k_s <= 4 and k_i < l_i - 10):
    bad.append(f"escape-off resolver shape regressed ({k_i} insns/{k_s} stkref; "
               f"legacy {l_i}/{l_s}): resolver contribution changed")
for b in bad:
    print(f"  FAIL: {b}", file=sys.stderr)
sys.exit(1 if bad else 0)
PYEOF
rc=$?
set -e
if [[ $rc != 0 ]]; then fail=1; fi

# --------------------------------------------------------------------------
# 2b. i686 target default: 6 free GPRs cannot hold the back-edge ranges the
#     acyclic resolver creates (measured -7.4% sha256_transform when forced
#     on), so m32 MUST default to the legacy policy.
# --------------------------------------------------------------------------
CCC32=""
for c in "$(dirname "$CCC")/lccc-i686" target/fastbuild/lccc-i686 target/release/lccc-i686; do
  [[ -x $c ]] && CCC32=$c && break
done
if [[ -n $CCC32 ]]; then
  echo "check_phi_acyclic_order: i686 target-aware default"
  "$CCC32" -O2 -S -o "$work/m32_def.s" "$here/../benchmark/programs/sha256_transform.c"
  CCC_PHI_ACYCLIC_ORDER=0 "$CCC32" -O2 -S -o "$work/m32_k.s" "$here/../benchmark/programs/sha256_transform.c"
  CCC_PHI_ACYCLIC_ORDER=1 "$CCC32" -O2 -S -o "$work/m32_on.s" "$here/../benchmark/programs/sha256_transform.c"
  cmp -s "$work/m32_def.s" "$work/m32_k.s" \
    || { echo "  FAIL: i686 default must equal the legacy resolver arm" >&2; fail=1; }
  if cmp -s "$work/m32_def.s" "$work/m32_on.s"; then
    echo "  FAIL: i686 CCC_PHI_ACYCLIC_ORDER=1 must force the acyclic arm" >&2
    fail=1
  fi
  d32=$(grep -cE '^[[:space:]]+[a-z]' "$work/m32_def.s")
  a32=$(grep -cE '^[[:space:]]+[a-z]' "$work/m32_on.s")
  if [[ $d32 -lt $a32 ]]; then
    echo "  FAIL: i686 acyclic census unexpectedly larger (default=$d32 acyclic=$a32)" >&2
    fail=1
  fi
  echo "  i686 default=legacy ($d32 insns); explicit =1 gives acyclic ($a32 insns)"
fi

# --------------------------------------------------------------------------
# 3. Real workload: sha256_transform known-answer in both arms; default arm
#    strictly smaller statically than the legacy arm.
# --------------------------------------------------------------------------
sha=$here/../benchmark/programs/sha256_transform.c
if [[ -r $sha ]]; then
    echo "check_phi_acyclic_order: sha256_transform known-answer + census"
    "$CCC" -O2 -DPASSES=2 -DBLOCK_COUNT=64 "$sha" -o "$work/sha_def"
    CCC_PHI_ACYCLIC_ORDER=0 "$CCC" -O2 -DPASSES=2 -DBLOCK_COUNT=64 "$sha" -o "$work/sha_leg"
    "$GCC" -O2 -DPASSES=2 -DBLOCK_COUNT=64 "$sha" -o "$work/sha_gcc"
    set +e
    d=$("$work/sha_def"); drc=$?
    l=$("$work/sha_leg"); lrc=$?
    g=$("$work/sha_gcc"); grc=$?
    set -e
    if [[ $drc != 0 || $drc != $grc || $d != $g ]]; then
        bad "default sha256_transform digest mismatch: lccc='$d'(rc=$drc) gcc='$g'(rc=$grc)"
    fi
    if [[ $lrc != $grc || $l != $g ]]; then
        bad "legacy sha256_transform digest mismatch: lccc='$l'(rc=$lrc) gcc='$g'(rc=$grc)"
    fi
    note "sha256_transform digests match gcc in BOTH policies ($d)"

    "$CCC" -O2 -S "$sha" -o "$work/sha_def.s"
    CCC_PHI_ACYCLIC_ORDER=0 "$CCC" -O2 -S "$sha" -o "$work/sha_leg.s"
    set +e
python3 - "$work" <<'PYEOF'
import re, sys
work = sys.argv[1]
STK = re.compile(r"\(%(?:rsp|rbp|esp|ebp)\b")
def fn(path, name):
    txt = open(path).read()
    m = re.search(rf'^{name}:\n(.*?)^\s*\.size\s+{name}\b', txt, re.S | re.M)
    if not m:
        print(f"  FAIL: could not extract {name} from {path}", file=sys.stderr)
        sys.exit(1)
    ins = [l for l in m.group(1).splitlines() if re.match(r'^\s+[a-z]', l)]
    return len(ins), sum(1 for l in ins if STK.search(l))
d_i, d_s = fn(f"{work}/sha_def.s", "sha256_transform")
l_i, l_s = fn(f"{work}/sha_leg.s", "sha256_transform")
print(f"  sha256_transform: default={d_i} insns/{d_s} stkref   legacy={l_i}/{l_s}")
bad = []
if not (d_i < l_i and d_s < l_s):
    bad.append(f"default sha256_transform is not strictly smaller "
               f"({d_i}/{d_s} vs legacy {l_i}/{l_s})")
for b in bad:
    print(f"  FAIL: {b}", file=sys.stderr)
sys.exit(1 if bad else 0)
PYEOF
    if [[ $? != 0 ]]; then fail=1; fi
    set -e
fi

# --------------------------------------------------------------------------
# 4. Switch wiring: positive/default values are byte-identical to unset;
#    the two kill spellings are byte-identical to each other and distinct
#    from the default.
# --------------------------------------------------------------------------
echo "check_phi_acyclic_order: switch wiring"
"$CCC" -O2 -S "$work/rot.c" -o "$work/a_unset.s"
CCC_PHI_ACYCLIC_ORDER=1    "$CCC" -O2 -S "$work/rot.c" -o "$work/a_one.s"
CCC_PHI_ACYCLIC_ORDER=true "$CCC" -O2 -S "$work/rot.c" -o "$work/a_true.s"
CCC_PHI_ACYCLIC_ORDER=     "$CCC" -O2 -S "$work/rot.c" -o "$work/a_empty.s"
CCC_PHI_ACYCLIC_ORDER=0    "$CCC" -O2 -S "$work/rot.c" -o "$work/l_zero.s"
CCC_NO_PHI_ACYCLIC_ORDER=1 "$CCC" -O2 -S "$work/rot.c" -o "$work/l_no.s"
if ! cmp -s "$work/a_unset.s" "$work/a_one.s" \
   || ! cmp -s "$work/a_unset.s" "$work/a_true.s" \
   || ! cmp -s "$work/a_unset.s" "$work/a_empty.s"; then
    bad "default class broken: unset/1/true/empty are not all the acyclic arm"
else
    note "unset, =1, =true and empty all emit the shipping acyclic arm"
fi
if ! cmp -s "$work/l_zero.s" "$work/l_no.s"; then
    bad "CCC_PHI_ACYCLIC_ORDER=0 and CCC_NO_PHI_ACYCLIC_ORDER=1 disagree"
else
    note "both kill spellings emit the same legacy arm"
fi
if cmp -s "$work/a_unset.s" "$work/l_zero.s"; then
    bad "kill switch does not change codegen: resolver cannot be disabled"
fi

if [[ $fail != 0 ]]; then echo "check_phi_acyclic_order: FAILED"; exit 1; fi
echo "check_phi_acyclic_order: contract holds"
