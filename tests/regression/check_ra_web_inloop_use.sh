#!/usr/bin/env bash
# ============================================================================
# check_ra_web_inloop_use.sh — pin the web-wide in-loop-use supply in
# `live_range::mark_loop_spanning` (kill switch `CCC_NO_WEB_INLOOP_USE=1`).
#
# THE BUG THIS PINS
# -----------------
# `mark_loop_spanning` decides, per loop-spanning range, whether the value is
# actually READ inside the loop (`span_has_in_loop_use`). The admission-cap rule
# then demotes ranges that span a loop but have no in-loop use: their reloads all
# sit outside the loop, so a register held across the body buys them nothing.
# That trade is sound — but only if the flag is right.
#
# The flag was computed web-wide by summing `uses_in_extents` over the leader and
# its coalesce members, and the code says so explicitly: "The member map carries
# the web-wide in-loop-use flag: a leader's own `uses` under-count a phi web
# exactly the way its priority does." The sum was a silent no-op. `uses_in_extents`
# was populated by a pass over `ranges`, and a coalesced member is merged into its
# leader's interval, so it owns no `LiveRange` and never entered that pass. Every
# member lookup missed, the sum added nothing, and the flag degraded to
# "does the LEADER have an in-extent use".
#
# That is exactly wrong for a phi web led by a cold preheader definition — the
# shape every loop-carried recurrence has. On `sha256_transform` the two
# recurrence words are `leader=v166 members=[166,389]` (`Load state[0]`) and
# `leader=v182 members=[182,392]` (`Load state[4]`); their own use points all sit
# in the preheader, so `span_has_in_loop_use` came out false and the demotion rule
# fired on the two hottest values in the round loop. Each was then reloaded seven
# times per iteration: 14 of the loop's 15 memory operations concentrated on two
# slots.
#
# The fix supplies merged members into `uses_in_extents` from a per-value use-point
# map, using the same dense numbering as `folded_at`. It touches SPAN FLAGS ONLY.
# It deliberately does not touch `LiveRange::uses` or `priority`: inflating a
# coalesce web's priority in the main scan waves reorders the whole allocation and
# is a measured negative (expat -30%, adler32 -23%, arith_loop -12%,
# sha256_transform -56%). See the NOTE in `regalloc.rs` above the priority waves.
#
# MEASURED EFFECT (base `25ed36de`, `sha256_transform`, PASSES=8 BLOCK_COUNT=131072,
# ~430 ms/arm, paired interleaved A/B, both arms digest-identical to gcc):
#   runtime  +3.63% (51 rounds, p=0.0000) and +4.33% (41 rounds, p=0.0000);
#            median and min ratios agree in both replicates.
#   static   function 210 -> 198 instructions; hottest loop 188 -> 176
#            instructions and its most-reloaded slot 17 -> 10 accesses.
# Raw record: engineering/evidence/ra-web-inloop-use-2026-09-11/.
#
# This gate pins five properties:
#   1. CORRECTNESS FIRST — both arms must match the gcc oracle bit for bit on
#      stdout AND exit status. An A/B across a miscompile is meaningless, so this
#      is checked before any codegen claim.
#   2. THE MECHANISM FIRES — the default arm's `sha256_transform` must be
#      strictly smaller than the kill-switch arm's, and its hottest loop's
#      most-reloaded slot must be strictly less concentrated. A supply that
#      silently stops firing (the failure mode this repo has been bitten by
#      repeatedly) is a loud FAIL, never a pass.
#   3. THE KILL SWITCH IS WIRED — `CCC_NO_WEB_INLOOP_USE=1` observably changes
#      the emitted code, so the A/B stays reproducible and a regression can be
#      attributed without a rebuild.
#   4. THE KILL SWITCH IS PRESENCE-BASED — like every other `CCC_NO_*` allocator
#      switch it asks only whether the variable EXISTS and ignores its value, so
#      unset means the fix is on while "=1", "=0", empty, "=true" and "=enabled"
#      must all reproduce the kill-switch arm byte for byte. Pinning the value
#      semantics (rather than assuming "== 1") is what stops a diagnostic switch
#      from silently becoming a production perturbation.
#   5. THE SUPPLY DOES NOT LEAK INTO THE ADMISSION CAP — `lz4_compress` must be
#      byte-identical between the two arms. A member's in-extent reads answer the
#      BOOLEAN "would demotion stage a hot reload", which is web-wide by physics
#      (coalesced members share one register). They must NOT also be counted into
#      `span_in_loop_uses` / `span_exposed_uses`, which the cap thresholds against
#      MAX_SPAN_EXPOSED_USES on a per-range calibration. Coupling them was
#      implemented and measured: it lifted lz4's `main` v212 (remcost 100,
#      exposed 1 -> 3) out of `worth_capping`, the span-pressure valve then took
#      the pressure and picked v133 at remcost 1110 — an 11x more expensive
#      victim chosen on a future-use count with no cost term — and lz4 came out
#      40.53% SLOWER at identical instruction count (251 insns both arms, median
#      and min in agreement, p=0.0000, +4 frame refs in the hot loop). sha256's
#      win is unchanged by the decoupling (its assembly is byte-identical either
#      way), so this property costs nothing and forbids a 40% regression.
#      Unit-level twin: live_range::tests::
#      mark_loop_spanning_member_uses_do_not_inflate_the_cap_counts.
# ============================================================================
set -euo pipefail

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CCC=${CCC:-$here/../../target/fastbuild/lccc}
GCC=${GCC:-gcc}
SELF=check_ra_web_inloop_use
[[ -x $CCC ]] || { echo "$SELF: lccc not found at $CCC" >&2; exit 1; }
command -v "$GCC" >/dev/null 2>&1 || { echo "$SELF: no gcc oracle" >&2; exit 1; }

sha=$here/../benchmark/programs/sha256_transform.c
[[ -f $sha ]] || { echo "$SELF: corpus kernel missing at $sha" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0
note() { printf '  %s\n' "$*"; }
bad()  { printf '  FAIL: %s\n' "$*" >&2; fail=1; }

# --------------------------------------------------------------------------
# 1. Correctness in BOTH arms against the gcc oracle. The kernel carries its
#    own FIPS 180-4 known-answer check (it returns 2 if the "abc" vector fails),
#    so stdout and exit status together cover the digest and the built-in KAT.
#    Small trip counts keep this inside a CI budget; the runtime win itself is
#    measured out of band (see the evidence directory) because a 2-core shared
#    VM cannot resolve a few percent in a CI slot.
# --------------------------------------------------------------------------
echo "$SELF: correctness of both arms against $GCC"
"$CCC" -O2 -DBLOCK_COUNT=64 -DPASSES=1 "$sha" -o "$work/on_bin"
CCC_NO_WEB_INLOOP_USE=1 "$CCC" -O2 -DBLOCK_COUNT=64 -DPASSES=1 "$sha" -o "$work/off_bin"
"$GCC" -O2 -DBLOCK_COUNT=64 -DPASSES=1 "$sha" -o "$work/gcc_bin"

set +e
g_out=$("$work/gcc_bin" 2>&1); g_rc=$?
n_out=$("$work/on_bin"  2>&1); n_rc=$?
f_out=$("$work/off_bin" 2>&1); f_rc=$?
set -e

if [[ $n_rc != $g_rc || "$n_out" != "$g_out" ]]; then
    bad "default arm differs from gcc: out='$n_out'(rc=$n_rc) want '$g_out'(rc=$g_rc)"
else
    note "default arm matches gcc: '$n_out' (rc=$n_rc)"
fi
if [[ $f_rc != $g_rc || "$f_out" != "$g_out" ]]; then
    bad "kill-switch arm differs from gcc: out='$f_out'(rc=$f_rc) want '$g_out'(rc=$g_rc)"
else
    note "kill-switch arm matches gcc: '$f_out' (rc=$f_rc)"
fi
# The two arms must also agree with each other; a digest that depends on an
# allocator switch would mean the switch changes semantics, not just codegen.
if [[ "$n_out" != "$f_out" || $n_rc != $f_rc ]]; then
    bad "arms disagree with each other: '$n_out'(rc=$n_rc) vs '$f_out'(rc=$f_rc)"
fi

# --------------------------------------------------------------------------
# 2 + 3. The mechanism fires, and the kill switch is wired.
# --------------------------------------------------------------------------
echo "$SELF: structural contract on sha256_transform"
# These two arms hold an INDEPENDENT optimization constant. The contract below
# discriminates the supply by instruction count and stack traffic, and an
# unrelated pass can mask that proxy: `reuse_redundant_loads` now deletes a
# reload of a frame slot that survives a store to a *different*, non-overlapping
# slot, which strips stack traffic from BOTH arms. With it enabled the
# kill-switch arm came out SMALLER (192 insns / 47 slot refs) than the default
# arm (194 / 52) even though the supply was still worth +6.06% at runtime
# (15 amplified interleaved reps, low3 1.062 in agreement) -- the exact
# "fewer instructions and stack refs, yet slower" trap RA-06B warns about.
# So the structural arms disable that pass on both sides and measure the supply
# in isolation. The shipping configuration is still what the correctness-vs-gcc
# and lz4 blast-radius sections exercise.
ISO="CCC_NO_SAME_DST_RELOAD=1 CCC_NO_FRAME_SLOT_ALIASING=1"
env $ISO "$CCC" -O2 -S "$sha" -o "$work/on.s"
env $ISO CCC_NO_WEB_INLOOP_USE=1 "$CCC" -O2 -S "$sha" -o "$work/off.s"
"$GCC" -O2 -S "$sha" -o "$work/gcc.s"

if cmp -s "$work/on.s" "$work/off.s"; then
    bad "CCC_NO_WEB_INLOOP_USE=1 does not change the emitted code: the kill switch is not wired"
else
    note "kill switch observably changes codegen (A/B reproducible without a rebuild)"
fi

set +e
python3 - "$work" <<'PYEOF'
import collections, re, sys

work = sys.argv[1]
STK = re.compile(r"(-?\d+)\(%(?:rsp|rbp|esp|ebp)\b")
FN = "sha256_transform"


def body(path):
    txt = open(path).read()
    # gcc separates `.size` from the symbol with a TAB, lccc with a space;
    # match either, and fail loudly rather than silently falling back to the
    # whole translation unit (that fallback once produced a bogus oracle count).
    m = re.search(rf'^{FN}:\n(.*?)^\s*\.size\s+{FN}\b', txt, re.S | re.M)
    if not m:
        print(f"  FAIL: could not extract {FN} from {path}", file=sys.stderr)
        sys.exit(1)
    return m.group(1).splitlines()


def insns(lines):
    return [l for l in lines if re.match(r'^\s+[a-z]', l)]


def hottest_loop(lines):
    """Largest backward-jump loop: (insns, stack-memops, slots, max-per-slot)."""
    labels = {}
    for i, l in enumerate(lines):
        m = re.match(r'^(\.L[A-Za-z0-9_]+):', l)
        if m:
            labels[m.group(1)] = i
    best = None
    for i, l in enumerate(lines):
        m = re.match(r'^\s+(j\w+)\s+\.?(\.L[A-Za-z0-9_]+)', l)
        if not m or m.group(2) not in labels or labels[m.group(2)] > i:
            continue
        seg = insns(lines[labels[m.group(2)]:i + 1])
        if best is None or len(seg) > best[0]:
            slots = collections.Counter()
            for x in seg:
                for off in STK.findall(x):
                    slots[off] += 1
            stk = sum(len(STK.findall(x)) for x in seg)
            best = (len(seg), stk, len(slots), max(slots.values()) if slots else 0)
    return best


arms = {}
for lbl, name in (("default", "on.s"), ("killswitch", "off.s"), ("gcc", "gcc.s")):
    b = body(f"{work}/{name}")
    ins = insns(b)
    loop = hottest_loop(b)
    arms[lbl] = dict(
        insns=len(ins),
        stkref=sum(1 for l in ins if STK.search(l)),
        loop=loop,
    )

for lbl in ("default", "killswitch", "gcc"):
    a = arms[lbl]
    lp = a["loop"]
    lps = "no backward-jump loop" if lp is None else (
        f"loop {lp[0]} insns/{lp[1]} stack-memops/{lp[2]} slots/max-per-slot {lp[3]}")
    print(f"  {lbl:11s} {FN}: {a['insns']} insns, {a['stkref']} stack refs; {lps}")

bad = []
d, k, g = arms["default"], arms["killswitch"], arms["gcc"]

# The supply must make the function strictly smaller.
if not d["insns"] < k["insns"]:
    bad.append(f"default arm is not smaller than the kill-switch arm "
               f"({d['insns']} vs {k['insns']} instructions): the web-wide "
               f"supply is not firing")
# The mechanism's actual target is reload CONCENTRATION on the hottest slot,
# not merely the instruction total: a web that is no longer demoted stops being
# re-read from one slot seven times per iteration.
if d["loop"] is None or k["loop"] is None:
    bad.append("could not locate the round loop in one of the arms")
else:
    if not d["loop"][3] < k["loop"][3]:
        bad.append(f"hottest-loop reload concentration did not improve "
                   f"(max-per-slot {d['loop'][3]} vs kill switch {k['loop'][3]})")
    if not d["loop"][1] <= k["loop"][1]:
        bad.append(f"hottest-loop stack traffic grew "
                   f"({d['loop'][1]} vs kill switch {k['loop'][1]} memops)")
# The switch must not cost stack references at function scope either.
if d["stkref"] > k["stkref"]:
    bad.append(f"default arm has MORE stack references than the kill switch "
               f"({d['stkref']} vs {k['stkref']})")

for b in bad:
    print(f"  FAIL: {b}", file=sys.stderr)
sys.exit(1 if bad else 0)
PYEOF
rc=$?
set -e
if [[ $rc != 0 ]]; then fail=1; else note "web-wide supply fires: smaller function, less concentrated reloads"; fi

# --------------------------------------------------------------------------
# 4. Presence semantics. `RaConfig::from_sources` is given a predicate that
#    reports whether a variable exists at all, so the VALUE is irrelevant: any
#    set value must land on the kill-switch arm, and only an unset variable
#    leaves the fix on. Assert both directions.
# --------------------------------------------------------------------------
echo "$SELF: kill switch is presence-based and consistent"
# Compiled under the same isolation as on.s/off.s above, so that this section
# compares like with like: it is testing the SWITCH's presence semantics, not
# the independent reload-reuse precision.
env $ISO CCC_NO_WEB_INLOOP_USE=0        "$CCC" -O2 -S "$sha" -o "$work/v0.s"
env $ISO CCC_NO_WEB_INLOOP_USE=         "$CCC" -O2 -S "$sha" -o "$work/vempty.s"
env $ISO CCC_NO_WEB_INLOOP_USE=true     "$CCC" -O2 -S "$sha" -o "$work/vtrue.s"
env $ISO CCC_NO_WEB_INLOOP_USE=enabled  "$CCC" -O2 -S "$sha" -o "$work/venabled.s"
for v in v0 vempty vtrue venabled; do
    if ! cmp -s "$work/off.s" "$work/$v.s"; then
        bad "presence semantics broken: CCC_NO_WEB_INLOOP_USE='$v' does not reproduce the kill-switch arm"
    fi
done
if cmp -s "$work/on.s" "$work/off.s"; then
    bad "unset and set produce identical code: the switch is inert"
fi
if [[ $fail == 0 ]]; then
    note "unset => fix on; '1', '0', empty, 'true' and 'enabled' => fix off, all byte-identical"
fi

# --------------------------------------------------------------------------
# 5. Blast-radius guard: the supply must not reach the admission cap's counts.
#    lz4_compress is the measured victim of that coupling (-40.53%), and under
#    the decoupled design its assembly is bit-for-bit the same with the supply
#    on and off. This is the cheapest possible detector for a re-coupling: no
#    timing needed, and it fails loudly instead of shipping a 40% loss.
# --------------------------------------------------------------------------
lz4=$here/../benchmark/programs/lz4_compress.c
if [[ -f $lz4 ]]; then
    echo "$SELF: admission-cap counts are not inflated by the supply"
    "$CCC" -O2 -S "$lz4" -o "$work/lz4_on.s"
    CCC_NO_WEB_INLOOP_USE=1 "$CCC" -O2 -S "$lz4" -o "$work/lz4_off.s"
    if cmp -s "$work/lz4_on.s" "$work/lz4_off.s"; then
        note "lz4_compress byte-identical across arms: the supply feeds the boolean only"
    else
        delta=$(diff "$work/lz4_off.s" "$work/lz4_on.s" | grep -c '^[<>]' || true)
        bad "lz4_compress changed across arms ($delta differing lines): the web-wide supply is leaking into span_in_loop_uses/span_exposed_uses, a measured -40.53% on this kernel (property 5 in the header)"
    fi
else
    note "lz4_compress corpus kernel absent; skipping the blast-radius guard"
fi

if [[ $fail != 0 ]]; then echo "$SELF: FAILED"; exit 1; fi
echo "$SELF: contract holds"
