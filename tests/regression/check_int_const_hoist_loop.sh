#!/usr/bin/env bash
# Gate for loop-invariant wide-constant hoisting (`int_const_hoist`) and for
# the pipeline ordering that lets its work SURVIVE to codegen.
#
# WHAT THIS PINS, AND WHY IT ONCE BROKE SILENTLY
# ----------------------------------------------
# A 64-bit immediate outside the sign-extended imm32 range cannot be encoded
# in an x86-64 ALU instruction: every in-loop use pays a 10-byte `movabsq`
# into a scratch register PER ITERATION unless the constant is hoisted into
# a register that lives across the loop. `int_const_hoist` does that hoist —
# and for months its output was silently destroyed in exactly the hottest
# shapes: `jump_thread` and `integer_bounds` run AFTER the hoist and their
# cleanups called the PLAIN copy propagator, which forwarded the hoisted
# `Copy K` straight back into the loop-body uses. The suite stayed green
# (semantics unchanged), benchmarks quietly paid one `movabsq` per iteration
# in FNV/xxhash-style digest loops (sqlite_vdbe_serial, fold-style
# accumulators). Both failure directions are silent:
#   * lose the hoist   -> pure per-iteration frontend cost, no failing test;
#   * over-hoist       -> rare-path constants pinned in callee-saved
#                         registers across hot loops (measured on
#                         sqlite_varint: two extra push/pop pairs plus a
#                         register lost to a path taken on a fraction of
#                         iterations).
# This gate pins both directions against the generated assembly AND runs
# every kernel against the GCC oracle so a shape-correct but semantically
# wrong rewrite cannot pass on asm alone.
#
# Asserted:
#   (1) HOIST, plain shape — an FNV fold over an array parameter keeps its
#       prime in a register: exactly one `movabsq $prime` in the function,
#       placed before the loop head (the first backward-jump target).
#   (2) HOIST SURVIVES THE THREADED GUARD — the static-global guarded
#       accumulator shape (loop_memory_promote's guarded writeback) gives
#       jump_thread a diamond to thread; the hoist must survive its cleanup.
#       This is the regression pin for propagate_copies_sparing_const_homes.
#   (3) THE PASS IS REAL — (2) recompiled with CCC_DISABLE_PASSES=intconst
#       must differ; otherwise some other mechanism produced the shape and
#       this gate would keep passing with the pass deleted.
#   (4) RARE-PATH DECLINES — a wide constant used only under a
#       data-dependent branch inside the loop must NOT be materialized ahead
#       of the loop (the every-iteration dominance gate; the sqlite_varint
#       lesson).
#   (5) EXECUTION — all kernels run; stdout must equal gcc-16's.
set -uo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repo_root"

LCCC=${LCCC:-${CCC:-target/fastbuild/lccc}}
[[ "$LCCC" == /* ]] || LCCC="$repo_root/$LCCC"
[[ -x "$LCCC" ]] || { echo "FAIL: lccc not found at $LCCC" >&2; exit 1; }
GCC=${GCC16:-gcc-16}
command -v "$GCC" >/dev/null || { echo "FAIL: $GCC not found" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fails=0
bad() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
ok()  { echo "ok: $*"; }

# Loop intervals of an extracted function body: one "head backedge" line
# pair per backward jump (a jump to an already-defined label). A line
# inside any such interval executes per iteration of that loop.
loop_intervals() { # file
    awk '
        /^\.LBB[0-9]+:/ { lab[substr($1, 1, length($1)-1)] = NR }
        /j[a-z]+ \.LBB[0-9]+/ {
            t = $NF
            if (t in lab) print lab[t], NR
        }
    ' "$1"
}
# Whether any line matching $2 in file $1 falls inside a loop interval.
in_any_loop() { # file regex -> prints count
    local file=$1 re=$2 count=0 line
    while read -r line; do
        local h b
        while read -r h b; do
            if [[ "$line" -ge "$h" && "$line" -le "$b" ]]; then
                count=$((count + 1))
                break
            fi
        done < <(loop_intervals "$file")
    done < <(grep -n "$re" "$file" | cut -d: -f1)
    echo "$count"
}
# Extract one function body from an asm file.
extract_fn() { # asmfile fname out
    awk -v f="$2" '$0 == f ":" {on=1} on {print} on && $0 ~ ("\\.size " f) {exit}' "$1" > "$3"
}

cat > "$work/k1.c" <<'EOF'
#include <stdio.h>
unsigned long long fnv(const unsigned char *p, unsigned long n,
                       unsigned long long h) {
  for (unsigned long i = 0; i < n; i++) {
    h ^= p[i];
    h *= 1099511628211ULL;
  }
  return h;
}
int main(void) {
  unsigned char buf[4096];
  for (unsigned i = 0; i < sizeof buf; i++)
    buf[i] = (unsigned char)(i * 131u + 7u);
  printf("%llx\n", fnv(buf, sizeof buf, 1469598103934665603ULL));
  return 0;
}
EOF

cat > "$work/k2.c" <<'EOF'
#include <stdio.h>
static unsigned long long acc = 1469598103934665603ULL;
static void fold(const unsigned char *p, unsigned int n) {
  unsigned long long h = acc;
  for (unsigned int i = 0; i < n; i++) {
    h ^= p[i];
    h *= 1099511628211ULL;
  }
  acc = h;
}
int main(void) {
  unsigned char buf[4096];
  for (unsigned i = 0; i < sizeof buf; i++)
    buf[i] = (unsigned char)(i * 29u + 3u);
  fold(buf, 0u);          /* zero-trip: writeback must be guarded */
  fold(buf, sizeof buf);
  printf("%llx\n", acc);
  return 0;
}
EOF

cat > "$work/k3.c" <<'EOF'
#include <stdio.h>
/* The store in the rare arm cannot be speculated, so if-conversion must
 * refuse and the arm stays a genuinely conditional block. (A pure
 * `h ^= C` arm gets if-converted to xor+cmov, which runs every iteration
 * and then hoisting C is CORRECT — an earlier draft of this gate pinned
 * the wrong expectation on exactly that kernel.) */
unsigned long long scan(const unsigned char *p, unsigned long n,
                        unsigned long long h, unsigned long *hits) {
  for (unsigned long i = 0; i < n; i++) {
    if (p[i] == 0x7fu) {
      *hits += 1;
      h ^= 0x123456789abcdefULL; /* rare path: must NOT be pre-pinned */
    }
    h += p[i];
  }
  return h;
}
int main(void) {
  unsigned char buf[4096];
  unsigned long hits = 0;
  for (unsigned i = 0; i < sizeof buf; i++)
    buf[i] = (unsigned char)(i * 17u + 11u);
  /* Sequenced: scan() writes hits, and printf argument evaluation order
   * is unspecified — passing both unsequenced made GCC (right-to-left)
   * legitimately print the pre-call value. */
  unsigned long long r = scan(buf, sizeof buf, 99u, &hits);
  printf("%llx %lu\n", r, hits);
  return 0;
}
EOF

# ── (1) plain FNV loop: prime hoisted ahead of the loop ────────────────
"$LCCC" -O2 -S -o "$work/k1.s" "$work/k1.c" 2>/dev/null \
    || bad "k1: compile failed"
extract_fn "$work/k1.s" fnv "$work/k1.fnv.s"
n=$(grep -c 'movabsq \$1099511628211' "$work/k1.fnv.s" || true)
in_loop=$(in_any_loop "$work/k1.fnv.s" 'movabsq \$1099511628211')
if [[ "$n" -ne 1 || "$in_loop" -ne 0 ]]; then
    bad "k1: prime must be materialized exactly once, outside every loop (count=$n, in-loop=$in_loop)"
else
    ok "k1: prime hoisted out of the loop"
fi

# ── (2) guarded-global shape: hoist survives jump_thread's cleanup ─────
"$LCCC" -O2 -S -o "$work/k2.s" "$work/k2.c" 2>/dev/null \
    || bad "k2: compile failed"
extract_fn "$work/k2.s" fold "$work/k2.fold.s"
if [[ ! -s "$work/k2.fold.s" ]]; then
    # fold may be inlined into main; assert there instead.
    extract_fn "$work/k2.s" main "$work/k2.fold.s"
fi
total=$(grep -c 'movabsq \$1099511628211' "$work/k2.fold.s" || true)
in_loop=$(in_any_loop "$work/k2.fold.s" 'movabsq \$1099511628211')
if [[ "$total" -eq 0 ]]; then
    bad "k2: prime materialization not found at all"
elif [[ "$in_loop" -ne 0 ]]; then
    bad "k2: prime materialized inside a loop ($in_loop of $total sites) — the hoist was re-folded"
else
    ok "k2: prime stays hoisted through jump_thread cleanup ($total materializations, all outside loops)"
fi

# ── (3) the kill switch is real: disabling the pass must change k2 ─────
CCC_DISABLE_PASSES=intconst "$LCCC" -O2 -S -o "$work/k2.off.s" "$work/k2.c" 2>/dev/null \
    || bad "k2: compile with intconst disabled failed"
if cmp -s "$work/k2.s" "$work/k2.off.s"; then
    bad "k2: CCC_DISABLE_PASSES=intconst changed nothing — gate (2) is not testing the pass"
else
    ok "k2: intconst kill switch verified against the same shape"
fi

# ── (4) rare-path constant must not be pinned ahead of the loop ────────
"$LCCC" -O2 -S -o "$work/k3.s" "$work/k3.c" 2>/dev/null \
    || bad "k3: compile failed"
extract_fn "$work/k3.s" scan "$work/k3.scan.s"
total=$(grep -c 'movabsq \$81985529216486895' "$work/k3.scan.s" || true)
in_loop=$(in_any_loop "$work/k3.scan.s" 'movabsq \$81985529216486895')
if [[ "$total" -eq 0 ]]; then
    bad "k3: rare-path constant not found at all"
elif [[ "$in_loop" -ne "$total" ]]; then
    bad "k3: rare-path constant pre-pinned outside the loop (every-iteration gate lost: $in_loop of $total inside)"
else
    ok "k3: rare-path constant left at its conditional in-loop site"
fi

# ── (5) execution against the GCC oracle ───────────────────────────────
for k in k1 k2 k3; do
    "$LCCC" -O2 -o "$work/$k.lccc" "$work/$k.c" 2>/dev/null \
        || { bad "$k: lccc link failed"; continue; }
    "$GCC" -O2 -o "$work/$k.gcc" "$work/$k.c" 2>/dev/null \
        || { bad "$k: gcc link failed"; continue; }
    out_l=$("$work/$k.lccc") || { bad "$k: lccc binary failed"; continue; }
    out_g=$("$work/$k.gcc") || { bad "$k: gcc binary failed"; continue; }
    if [[ "$out_l" != "$out_g" ]]; then
        bad "$k: output mismatch (lccc=$out_l gcc=$out_g)"
    else
        ok "$k: output matches gcc"
    fi
done

if [[ "$fails" -ne 0 ]]; then
    echo "check_int_const_hoist_loop: $fails failure(s)" >&2
    exit 1
fi
echo "OK check_int_const_hoist_loop"
