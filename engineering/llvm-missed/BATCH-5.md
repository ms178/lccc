# Batch 5 — the 2026-10-31 re-inventory, a 151-report audit, and the two identities it shipped

Batch 5 does three things. It **re-inventorizes** the label against live GitHub and
records the delta instead of describing the old snapshot as current. It **reads
151 reports** — every post that joined the label since the last snapshot, every
post whose state changed, and 81 previously unreviewed backlog entries — with a
per-report verdict in [`batch5-sources.json`](batch5-sources.json). And it ships
the **two identities** the campaign's own probe corpus found LCCC missing, with
exhaustive tests, a native gate and a disabled-pass attribution leg.

Nothing in this batch claims whole-issue coverage. No report in the snapshot
isolates either fold, so the ledger's `covered` count for this cohort is **0** and
the folds are recorded as audit-discovered identities, not as closures.

## 1. Re-inventory: 3,111 → 3,169 posts

Collected 2026-10-31T17:43Z with `scripts/llvm_missed_inventory.py` (posts only):
two identical `state=all` enumerations of the `missed-optimization` label, 64
requests, membership identical across both, **3,169 posts = 3,159 issues + 10
PRs**. The delta against the 2026-10-20 snapshot:

| change | count | notes |
|---|---:|---|
| posts added | **58** | 24 supported-target, 21 middle-end, 11 unclassified, 2 unsupported-target |
| posts closed upstream | **11** | re-read below, because a closure is not a proof of relevance |
| posts removed from the label | 0 | membership only ever grew |

`inventory.csv` and `snapshot.json` are updated together, so the hashes the
corpus test checks (`csv_sha256`, `ledger_sha256`) stay consistent; the previous
snapshot's hashes are preserved inside `snapshot.json` under `previous`.
Conversations were **not** re-collected in this refresh, and the file says so:
the archived comments remain the 2026-10-20 run's (3,111 posts,
`comments_sha256 262905b0…`), and the 58 new posts have bodies only. Re-collecting
3,169 conversations would cost roughly 3,169 requests and change nothing the
audit below depends on.

## 2. The audit: 151 reports, 151 verdicts

Cohort composition — every row is in `batch5-sources.json` with a
`review_note` and an `archive_sha256` over the body that was read:

| cohort | n | why it is here |
|---|---:|---|
| new | 58 | arrived after the 2026-10-20 snapshot |
| changed | 11 | open → closed since the snapshot; closure is re-derived, never trusted |
| backlog | 81 | previously unreviewed, ranked toward portable scalar and middle-end reports |
| addendum | 1 | #128460, pulled in while checking whether any report isolates the shift-pair fold |

**Outcome: 151 reviewed-deferred, 0 covered.** That is the honest result: the two
folds this batch ships were found by the probe corpus, and no report in the
snapshot asks for them exactly. The recurring reasons, in the order they occur:

* **FP and libcall policy** — strict-FP, signed-zero and NaN rules refuse most of
  the `fabs`/`frem`/`sincos`/`fptoui` cohort before profitability matters.
* **Facts this IR does not carry** — `nuw`/`nsw`/`nneg`/`invariant.load`/`freeze`
  do not exist in LCCC's IR, so "the premise is unavailable" is a refusal, not a
  deferral of missing work.
* **No vectorizer / other ISA layers** — the AVX-512, SVE, NEON, Hexagon,
  RISC-V-Zibi and MLIR rows have no consumer in this compiler.
* **Layers that own the decision** — regalloc cost models, branch layout,
  prologue shape, SCEV over switch successors, `i128` selection (call-based
  here). A peephole cannot reach them, and pretending otherwise would be slop.
* **Real scalar peepholes that need a lattice LCCC does not have** — relational
  facts across edges, interval sets, and exact-division lattices are the meat of
  the remaining scalar rows. They are the queue for batch 6, not a fold.

### What the audit measured about LCCC

Every deferred row was checked against the tree as it stands, and eight
*reproducible* gaps came out of that (each one is a C function compiled at `-O2`
by both compilers, instruction counts over the emitted body):

| shape | LCCC | GCC 16.2 | what is missing |
|---|---:|---:|---|
| `x < 0 ? -x : x` | 6 | 4 | abs formation / `neg`+`cmovs` flag reuse |
| `a > b ? a - b : b - a` (unsigned) | 8 | 7 | absolute difference, plus a destination copy |
| `1u << __builtin_ctz(~x)` | 9 | 6 | `~x & (x + 1)` is 3 ops and beats both |
| `(x != 7) & ((x & 1) == 0)` (variable C) | 9 | 6 | parity/set reasoning over the constant |
| `x > 3 && x < 10` | 7 | 5 | single unsigned range test (`sub` + `cmp`) |
| `x == 0 \|\| x == 1` (short-circuit) | 7 | 4 | same rule as the fold below, but if-conversion runs first |
| `(unsigned char)(((x + 4) & ~7) - 4)` | 7 | 4 | mask immediate plus a redundant truncation |
| `(a - b) > 0` | 5 | 4 | compare-of-subtraction, needs a no-wrap fact |

These are recorded, not fixed: six of the eight need either a new IR operation
(`abs`), an instruction-inserting rewrite (the range test), or a target-level
flags idiom (`cmovs`) — none of which fits the 1-to-1 `bit_idioms` contract this
batch stayed inside. They are the measured starting point for batch 6.

## 3. The two folds

### 3.1 `(x << k) >>u k` → `x & (2^(W-k) - 1)`

The left shift drops the top `k` bits, the logical right shift returns zeros
above them, so the pair keeps exactly the low `W-k` bits. `AShr` is not admitted
(`(x << k) >>s k` sign-extends a low field), nor is `k == 0`, `k >= W`, a
mismatched amount or a mismatched width. Only the consumer is rewritten: the
shifts become dead and the ordinary dead-code pass removes them, so a live user
of the left shift keeps its value.

    unsigned f(unsigned x) { return (x << 3) >> 3; }
    before: shll $3,%edi; shrl $3,%edi; movl %edi,%eax; ret        (4)
    after:  movl %edi,%eax; andl $536870911,%eax; ret              (3, GCC parity)

**The first draft was wrong, and the differential caught it.** It masked with
`2^k - 1` instead of `2^(W-k) - 1`; that is correct only for `k` above every set
bit of `x`. The native witness compared the fold against the *same wrong
expectation*, so it passed; compiling the same source with GCC 16.2 and Clang
23.1.4 and diffing the printed values exposed it immediately (192,510
mismatches). The Rust test now compares the emitted mask against the pair's own
two-step evaluation over every value of the 8- and 16-bit rings, and the native
gate diffs three independently compiled binaries.

### 3.2 `(x == 0) | (x == 1)` → `x <=u 1`, and the `!=`/`&` dual

Both sides test one value against the two constants that form `{0, 1}`; a single
unsigned comparison expresses it exactly, because every other value of the ring —
including every negative one — is unsigned-above 1. The comparisons must agree
on op, on value (after `Copy` peeling) and on type, and the constants must be
`{0, 1}` in either order; `x != 0 | x != 1` (the tautology) and `x == 0 & x == 1`
(the empty set) are different functions and are refused, as is any other
constant pair.

    int f(unsigned x) { return (x == 0u) | (x == 1u); }
    before: 9 instructions (two compares, two setcc, and, cmov, ...)
    after:  4 (xorl; cmpl $1; setbe; ret), GCC parity

`memb_control`, `x == 0 | x == 2`, is the **negative control** in both the Rust
tests and the gate: one constant away, and it must keep both comparisons. It does.

The short-circuit spelling (`x == 0 || x == 1`) is *not* improved: by the time
`bit_idioms` runs, if-conversion has already turned the `||` into a select tree,
and folding that needs a new comparison instruction, which this pass's
index-preserving contract cannot insert. That is the first batch-6 item.

## 4. Verification

| check | result |
|---|---|
| `cargo test --profile fastbuild --locked` | **7,073 passed, 0 failed** (6 new) |
| new tests: exhaustive 8/16-bit rings, 32/64 boundaries, refusal table | 6 pass |
| native witness at `-O0..-O3` (200,000-value sweep, self-checking) | ok |
| value differential: LCCC vs GCC 16.2 vs Clang 23.1.4 | identical on all probes and the sweep |
| gate `check_llvm_batch5_scalar_sets.py` | PASS, 0.36 s, wired into `ci_local.sh`, `ci.yml` (shard b) and the lane plan |
| gate against the pre-fold binary | **5 FAILs** (mechanism + oracle), so it is sensitive |
| 40-file corpus instruction delta | 9,598 → 9,598 (**0**) — no regression, and the patterns are rare in the corpus |
| lane plan / shard partition / parity / workflow-shell | PASS (468 commands, backend-contracts 423.85 s / 423.86 s) |

## 5. What is not done

* The eight measured scalar gaps in §2, in the order above.
* The short-circuit membership spelling (needs an instruction-inserting rewrite).
* Conversation comments for the 58 new posts (bodies only).
* Coverage rows for the two folds: deliberately absent, because no upstream
  report in the snapshot isolates them and the ledger's rule is that a local
  reduction is not an issue closure.
