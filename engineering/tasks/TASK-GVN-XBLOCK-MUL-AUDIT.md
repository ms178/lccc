# PR #611 Review AI Audit — Adjudication and Follow-up (2026-09-24)

Scope: `GVN: safely reuse dominated integer multiplications` (cross-block
pure CSE for integer `Mul` via use-renaming). This document records, with
code citations and measurements, how each Review AI finding was
adjudicated and what was changed. Every claim below was verified against
the tree at the rebase point, not inferred.

## Rebase

Upstream moved (HLE assembler + style, tree-identical to the PR's new
base) after the original PR was cut on the PR #608 merge. The re-based
series carries no semantic interaction: the merged work touches
assembler/encoder/parser files only, zero overlap with `src/passes/gvn.rs`.

## Finding 1 — `source_spans` desync after multiple eliminations: AGREED, fixed

The original code removed spans by the ORIGINAL instruction index
(`inst_idx`) from a vector that earlier removals had already shrunk. With
`[mul, mul, keep]` and spans `[s1,s2,s3]`, the second deletion's original
index 3 fails the `inst_idx <= new_spans.len()` guard and silently
removes nothing (length desync), and in other orderings removes the WRONG
span. The modified regression test used `source_spans: Vec::new()` and
could not see any of this.

Fix (same convention as `restore_phi_prefix` and the late-vectorizer
sweep — the repo's established span contract):

* `spans_in_lockstep` is computed once from the block's arriving
  instruction/span lengths; a pre-broken vector is left byte-identical
  (clear-don't-guess).
* A rename deletion removes the CURRENT instruction's span at index
  `new_instructions.len()` — the survivor count IS the current position,
  which is correct for ANY number of deletions in one block.
* A post-loop `debug_assert!` pins the lockstep invariant for future
  modifiers.
* `test_cross_block_cse_multiple_deletions_preserve_spans` (three
  eliminations in one block, unique spans, exact correspondence asserted)
  and `test_cross_block_cse_pre_broken_spans_left_alone` (mismatched
  vector untouched) pin both paths.

## Finding 2 — "Mul has no structural consumers": claim was FALSE, hazard analysis below

The comment was a post-hoc summary of corpus measurements, not a code
audit. The verified inventory of every body-local Mul lookup:

| Site | Requirement | Why the rename is safe |
|---|---|---|
| `vectorize.rs` F64 matmul (~1187) | body-local `mul_dest` | FLOAT — excluded by the `!is_float()` gate |
| `vectorize.rs` dot-product matcher (~2463/2780) | `added_value` def in body; mul operands are body LOADS | The dot-mul's operands are body loads. GVN invalidates load entries at merge points (`gvn_dfs`: `preds.len(block) > 1` bumps `load_generation`), and a loop header always has ≥ 2 preds, so body loads carry value numbers that cannot appear in ANY dominator's pure-expression entry. The body dot-mul can therefore never hit a cross-block table entry. |
| `vectorize.rs` lane matcher (~23675) | body-local `term` Mul with `lhs == rhs` | F32/F64 lanes only — excluded by the type gate |
| `vectorize.rs` `vector_load_key` (~3278) | GEP-offset Mul body-local | Degrades gracefully: `_ => go.0` key fallback, no rejection |
| `iv_strength_reduce.rs` (~928) | scans loop blocks structurally for `iv * C` | A renamed duplicate keeps its canonical inside the loop; a loop-variant `iv * C` cannot exist in a preheader (the preheader sees the init value, not the phi) |

The comment now states this inventory verbatim at the decision site.
Two pins:

* `test_cross_block_cse_preserves_dot_product` (IR level): a loop body
  with BOTH a renamed invariant duplicate Mul and a surviving body-local
  dot-product Mul over body loads.
* `check_gvn_cross_block_mul_dot.sh` (end-to-end gate, wired into
  `ci_local --fast`): the header-exit-compare shape — exactly ONE
  register-register `imul` for `i*i` (the canonical) plus ONE
  memory-form dot `imul` in the body. Negative-verified against a
  cross-block-CSE-disabled build (detects the surviving duplicates).

The Review AI's proposed phase gating / shape-aware eligibility would
add permanent complexity for a hazard that is unreachable by
construction; the load-VN freshness argument above is the actual
invariant, and it is now documented and pinned instead.

## Finding 3 — Phi protection is direct-only: DOCUMENTED as intended scope

Every structural consumer keys on the DIRECT feed's instruction inside
its block (`find_iv_in_loop` scans the latch for the Add producing the
back-edge value; the reduction matcher requires `added_value`'s def in
the body). A value that reaches a phi only THROUGH a forwarding op
(`v4 = Copy v3; phi(v4)`) is invisible to those matchers regardless of
what computes it — and the forwarding op itself is protected as the
direct feed. Deleting `v3` behind the Copy is sound SSA (the dominator's
value dominates `v3`'s block, which dominates every use) and
shape-preserving for every existing consumer. A transitive closure
through Copy/Cast would protect nothing that anything reads.

Pinned by `test_cross_block_cse_phi_feed_direct_kept` (direct feed
survives) and `test_cross_block_cse_phi_feed_copy_is_renamable` (the Mul
behind a phi-feeding Copy renames; the Copy and the phi shape survive).

## Finding 4 — silent 16-hop limit: AGREED, fixed

`resolve_rename` now follows the mapping to its fixpoint with
hop-count-based cycle proof (an acyclic path through a finite map is at
most `rename.len()` edges; more proves a cycle and panics). The mapping
is validated: conflicting double mappings panic, self-renames are
`debug_assert`ed, and a post-rewrite pass panics if ANY use of a deleted
value survives (a visitor gap or mapping bug is a miscompile, not a
warning). `test_resolve_rename_chain_and_cycle` drives a 32-link chain
(2× the old cutoff) and a synthetic cycle.

The collector's structural no-chain property (every target is the table's
canonical KEPT value, so targets are never themselves deleted) is
documented at the collector and pinned by
`test_cross_block_cse_nested_dominators_map_to_canonical`.

## The linux_find_bit performance claim: REFUTED with paired data

The audit asserted the PR "regresses performance of linux_find_bit
significantly". Measured:

* `linux_find_bit` codegen is BYTE-IDENTICAL between the pre-PR and
  post-PR compilers at `-O0/-O1/-O2/-O3`, and the compiled benchmark
  binaries share a sha256 — `paired_ab.py` declares the comparison
  UNINFORMATIVE (identical arms).
* Full 39-benchmark corpus, 11 reps, both arms against the same gcc
  anchor: linux_find_bit ratio base 1.301 vs PR 1.290; corpus geomean
  base 0.6982 vs PR 0.6964.
* Only two programs in the corpus changed codegen at all: `sieve`
  (intended; `count_primes` 45→40 insns, 3→1 `imull`) and
  `csv_field_sum` (12→7 `imull`, bit-identical output).
* Paired interleaved A/B on sieve (30 rounds, sign test): p = 0.58 —
  the mul elimination is a code-quality win, not a measurable runtime
  win on this benchmark (the marking loop dominates; the eliminated
  muls were ~0.03% of executed instructions).

## Godbolt oracle data (scripts/godbolt.py + codegen_oracle.py)

Oracle pins audited current (gcc 16.2 / clang 23.1.0 / icc 2021.10.0 /
icx-latest). `sieve.c :: count_primes`:

| compiler | -O2 insns | -O3 insns |
|---|---|---|
| **lccc (this PR)** | **40** | **40** |
| gcc 16.2 | 34 | 99 |
| clang 23.1.0 | 97 | 97 |
| icc 2021.10.0 | 99 | 145 |
| icx (latest) | 194 | 198 |

At -O3 lccc emits the tightest `count_primes` of all five compilers
(2.5× fewer instructions than gcc's unrolled form). The remaining -O2
gap to gcc (6 insns) is decomposed and root-caused (next-session
material, see the worklog): `movw` store-merge for adjacent constant
byte stores, the `cmp $1, x; sbb $-1, acc` counting idiom, and
loop-guard rotation for provably-nonzero trip counts — all backend
concerns outside this PR's scope.

`linux_find_bit :: linux_find_next_andnot_bit` at -O2: lccc 45 insns vs
gcc 72 (clang/icc/icx inline it away in their translation units). The
runtime gap (1.29×) is NOT instruction-count-driven: lccc re-materializes
`(index+1)*64` per scanned word (`leaq`+`shlq`) where gcc maintains a
running bit position (`addq $64`). That is a pre-existing exit-bound
strength-reduction opportunity (the disabled-by-default
`CCC_IVSR_SCALAR_DERIVED` territory), byte-identical before and after
this PR.

## Test coverage added

Ten new unit tests (spans ×2, nested canonical targets, integer widths,
every mutable use form, direct phi feed kept, copy-mediated phi feed
renamable, multi-def kept, dot-product preservation, rename resolver
chain/cycle) plus the end-to-end gate and its negative control.
