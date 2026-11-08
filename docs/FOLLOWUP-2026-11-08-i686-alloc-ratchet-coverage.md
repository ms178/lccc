# The allocate-before-filter ratchet was vacuous outside x86-64

**Status:** fixed and measured. Follow-up to the merged MB-20 step 3 work (#1137).

## The defect: a gate that could not see three of four backends

`tests/regression/check_peephole_alloc_before_filter.py` landed with #1137 to stop
the allocate-before-filter pattern from returning. It had two independent
blind spots, and together they made it report PASS for every backend except
x86-64:

1. **The regex only matched the method idiom.** It required a literal
   `.trimmed(`, i.e. x86-64's `infos[i].trimmed(store.get(i)).to_string()`.
   i686 uses a free function instead:

   ```rust
   let jcc = trimmed(store, &infos[a], a).to_string();   // i686
   ```

   No `.`, so no match.

2. **The default root was x86-only** (`src/backend/x86/codegen/peephole`), so even
   a corrected regex would never have been pointed at i686, arm or riscv.

Running the widened gate against the tree it was supposed to be protecting
found **5 real violations in i686** immediately. This is the same failure mode
the gate's own test corpus was written to prevent — a check that passes because
it looked nowhere — so it is fixed the same way: pin the real shape in a fixture
and prove the gate fails without the fix.

## The five i686 sites

Classified by the parser's signature, exactly as in the x86-64 work:

| site | parser | returns | treatment |
| --- | --- | --- | --- |
| `peephole.rs:8989` `div_text` | `div_reg` | `Option<RegId>` (Copy) | parse borrowed, keep the Copy result, allocate only for a candidate |
| `15244` `jcc` | `parse_condjmp` | `(&str, &str)` | **delete the copy** |
| `15286` `false_text` | `parse_select_arm_mov` | `(RegId, RegId, MoveSize)` | **delete the copy** |
| `15304` `jmp_text` | `parse_jmp_target` | `&str` | **delete the copy** |
| `15327` `true_text` | `parse_select_arm_mov` | `(RegId, RegId, MoveSize)` | **delete the copy** |

Four copies are removed outright, not merely deferred:

* `parse_select_arm_mov` returns only `Copy` fields, so the line never needed an
  owner (two sites).
* `parse_jmp_target` borrows, but the very next statement already converts the
  label to an owned `String` (`end_label.trim().to_string()`), so the line's
  borrow ends there.
* `parse_condjmp` borrows `cc` and `true_label`, and both stay live much longer —
  through `label_has_other_ref` and two `format!` calls. That is still safe:
  `label_has_other_ref` takes `&LineStore`, and the last use of either operand is
  the `format!` that builds the cmov, which runs **before** `store.replace(c, cmov)`.
  NLL therefore permits borrowing the store directly.

`div_text` is the one site that genuinely needs an owner: the candidate record
stores it and a later comparison reads `site_a.div_text == site_b.div_text`. But
`div_reg` returns a `Copy` `RegId`, so the filter can run on borrowed text and
**keep that result** — the owned copy is then made only for a candidate, with no
second parse. That is strictly better than the x86-64 probe shape, which has to
re-parse because its result borrows the text.

The borrow checker is the proof for the four deletions: `cargo check` passes with
the copies gone, which is only possible if nothing borrows them past a mutation.

## Measurement

A corpus was needed that actually reaches these passes; a zero would have been
reported as a win otherwise. `tests/regression/alloc_cost/i686_peephole_corpus.c`
is a deterministic, self-contained fixture (120 select functions + 20 division
functions + a `main` that calls them all).

Its density was **verified on the baseline compiler**, not assumed: the emitted
assembly contains **120 `cmov`** and **80 `div`/`idiv`**. Getting there required
one measured correction: a single register-arm ternary folds to `cmovcc`, but two
or more chained selects in one function do **not** (n=1 → 1 cmov, n=2 → 0,
n=3 → 0), because the extra live values spill and the arms stop being plain
register moves. Many small functions therefore maximise fold attempts, which is
what the removed allocation was paid on.

Result, `-O2 -S`, 2 repeats, `--require-improvement`:

| metric | before | after | delta |
| --- | --- | --- | --- |
| allocations | 974,342 | 973,850 | **−492 (−0.050 %)** |
| assembly | — | — | byte-identical |

**This is a small win and is reported as one.** These two passes are a tiny
fraction of total compiler allocation, meaning the denominator is ~974k
allocations for the whole compilation. The durable value here is the coverage
fix: the ratchet now actually protects four backends instead of one.

The `tests/bench/k_*.c` corpus could not be measured for i686 locally: it
`#include`s system headers and this host lacks the i686 multilib headers
(`bits/wchar.h`, `bits/wordsize.h`, …), so both arms fail identically in the
preprocessor. CI has real multilib. The fixture above is self-contained
precisely so the measurement does not depend on that.

## Output identity

i686, 1,327 sources × `-O1`/`-O2` × with and without `-g` = **5,308 compiles**:
**1,580 identical, 0 differing**, 3,728 rejected identically by both arms
(the header limitation above). Both arms are the same driver invoked as
`lccc-i686` and placed at identical directory depth, because the compiler embeds
include paths derived from its own location into DWARF — comparing binaries in
different directories produces spurious `-g` diffs.

## Gates

* The ratchet now scans **`src/backend`** (all four codegen backends: x86, i686,
  arm, riscv) and matches **both** idioms. It reports 5 violations on the
  pre-fix tree and 0 after — non-vacuous by construction.
* Three new contracts in `test_check_peephole_alloc_before_filter.py` (17 total):
  the real i686 free-function shape must be detected, its compliant form must be
  accepted, and the default root must contain every backend's `codegen`
  directory. The last one is what would have caught the x86-only default.

## Verification

* `scripts/build_lccc_fast.sh`: exit 0, **0 warnings**.
* `cargo check`: rc=0, 0 warnings (the borrow proof above).
* `cargo clippy --all-targets -- -D warnings`: **rc=0**.
* `cargo test --lib`: **8446 passed, 0 failed, 10 ignored**.
* Same with `debug-assertions` **and** `overflow-checks`: **8460 passed, 0 failed,
  10 ignored**.

Re-verified after rebasing onto `3be2663` (#1139, which added tests): fastbuild
exit 0 / 0 warnings, clippy rc=0, and the strict lane **8471 passed, 0 failed,
10 ignored**. The allocation A/B was repeated against the post-rebase binary and
is unchanged: 974,342 -> 973,850, byte-identical.
* Ratchet: **PASS, 372 files**.
* Ratchet contracts: **17 passed**.
