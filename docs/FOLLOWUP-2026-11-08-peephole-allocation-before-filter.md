# MB-20 step 3 — reject on borrowed text before allocating

**Status:** implemented and measured. x86-64 peephole only.

## The defect

26 sites in `src/backend/x86/codegen/peephole/passes/` had this shape:

```rust
let load = infos[i].trimmed(store.get(i)).to_string();   // heap copy of EVERY line
let Some((load_q, mem, t_fam)) = parse_memory_load(&load) else {
    i += 1;
    continue;                                            // ...and almost every line lands here
};
```

The parser is the filter, and it declines nearly every line in a function. The
allocation therefore happens *before* the test that discards the work. These are
per-line loops inside a fixpoint pass, so the waste is multiplied by line count
and by iteration count.

## The design: classify by the parser's signature, not by guesswork

The first instinct — "just use `&str` everywhere" — is wrong for some sites and
leaves value on the table for others. The deciding question is whether the
parser's result **borrows its input**. Every parser involved was read and
classified:

| parser | returns | input borrow? |
| --- | --- | --- |
| `parse_reg_copy` | `(u8, RegId, RegId)` | no |
| `parse_reg_reg_alu` | `(&'static str, bool, RegId, RegId)` | no (`'static`, not the input) |
| `parse_lea_add` | `(bool, RegId, RegId, RegId)` | no |
| `parse_reg_reg_cmp` | `(bool, RegId, RegId)` | no |
| `parse_reg_to_reg_mov` | `(usize, RegId, RegId)` | no |
| `parse_vec_copy` | `VecCopy { u8, u8, u16, bool }` | no |
| `is_full_flag_clobber` | `bool` | no |
| `parse_cmov` | `(&'static str, char, &str, &str)` | yes, but operands die before the rewrite |
| `parse_memory_load` | `(bool, &str, RegId)` | yes, outlives `store` mutation |
| `load_operand`, `parse_inplace_op`, `parse_zero_ext_load`, `parse_load_mem_to_reg`, `parse_store_reg_to_mem`, `parse_copyback_producer`, `parse_lea_address`, `parse_scalar_fma` | contain `&str` / `Vec<RegId>` tied to the input | yes, outlives `store` mutation |

That yields three treatments, best first:

**(1) Delete the allocation entirely — 14 sites.** For the seven parsers that
return no borrow of their input, the `.to_string()` was *never* needed. This is a
pre-existing missed optimization, not just a deferred cost:

```rust
// MB-20: `parse_reg_copy` yields no borrow of its input; no owned copy is needed.
let copy1 = infos[i].trimmed(store.get(i));
let Some((c1_q, a_fam, tm_fam)) = parse_reg_copy(copy1) else {
    i += 1;
    continue;
};
```

`parse_cmov` belongs here too, for a subtler reason worth recording: its `src`
and `dst` *do* borrow the line, but their last use is the `format!` that builds
the replacement, and `replace_line(store, ...)` runs strictly after it. NLL
therefore permits parsing the borrowed text directly. `is_full_flag_clobber`
returns `bool` and its argument had no other use, so the copy is simply inlined
away.

**(2) Probe, then allocate only for candidates — 12 sites.** Here the copy is
genuinely load-bearing. In `load_op_fuse.rs`, `mem` is passed to helpers that
take `store` mutably and into `format!` rewrites far below the parse, so
borrowing straight from `store` would hold an immutable borrow across those
mutations:

```rust
// MB-20: reject on borrowed text; only candidates need an owned line.
let load = infos[i].trimmed(store.get(i));
if parse_memory_load(load).is_none() {
    i += 1;
    continue;
}
let load = load.to_string();
let Some((load_q, mem, t_fam)) = parse_memory_load(&load) else {
    i += 1;
    continue;
};
```

* **Shadowing, not a new name.** The `&str` and the `String` share one
  identifier, so no new variable can collide in these very large functions and
  no rename can silently change a later use.
* **No new panic site.** The second parse keeps the original `else` body rather
  than `unreachable!()`. It is provably the same result on the same bytes, but a
  panic there would add a panic path to a hot pass and grow the panic census for
  no benefit.
* **Candidates are parsed twice.** That is the deliberate trade: candidates are
  rare next to rejected lines. It was measured, not assumed; had it not paid,
  the change would have been reverted.

**(3) Leave alone — 56 sites.** The remaining `trimmed(..).to_string()` calls
either need owned text unconditionally or feed a mutation. Converting them on
spec would be churn, not an optimization.

Sites touched: `load_op_fuse.rs` (13), `vector_copy.rs` (5), `memory_fold.rs` (3),
`narrow_copy_fold.rs` (2), `dead_writes.rs` (1), `flag_peepholes.rs` (1),
`relay_and_lea.rs` (1). The borrow checker is the proof for category (1):
`cargo check` passes with the copies removed, which is only possible if no
result borrows the deleted `String`.

## A regression this change caused, and how it was fixed

The first version applied treatment (2) everywhere. It was faster, but the
repository's own size ratchet rejected it:

```
fns_over_300   235   baseline 233      # FAIL: audit census grew
```

`fold_setcc_test_cmov` went 299 → 307 and `fold_induction_copyback` 294 → 308,
because the probe shape adds lines to functions already near the limit.
Re-baselining the census was not an option — that is deleting the smoke
detector. Instead the signature classification above was developed, which
*removes* lines at exactly those sites. Final sizes:
`fold_setcc_test_cmov` **299** (unchanged), `fold_induction_copyback` **299**,
census `fns_over_300` back to **233**, **PASS (no metric grew)**. The better
design is also the faster one: category (1) removes the allocation outright
instead of merely deferring it.

## Measurement, and the three traps that had to be fixed first

Callgrind is not in the agent toolchain cache and wall time on this 3.9 GiB
shared host cannot resolve a few percent. For an allocation optimization the
allocation count *is* the quantity removed, so the new ratchet counts heap
allocations by interposing glibc's allocator
(`tests/regression/alloc_count_shim.c`).

Getting a trustworthy number required fixing three measurement bugs. Each
produced a plausible-looking but wrong result, which is worse than no result:

1. **Thread-local counters lost the worker threads.** `compiler_main` runs the
   compile on a spawned 64 MB-stack thread. A `__thread` counter folded by a
   main-thread destructor reported **15 allocations** for a compile that really
   performs **~29,600**. The counters are now global relaxed atomics.
2. **A fixed report path can lose child processes.** Reports are written per pid
   and summed.
3. **The byte total depends on the environment.** `std::env` copies it, so a
   longer `$ALLOC_COUNT_OUT` path changes `bytes`. The harness passes an
   identical path to both arms; allocation *count* is the gated metric, bytes
   are supporting evidence only.

The harness refuses to report anything unless each count reproduces exactly on
a repeat run and both arms emit byte-identical assembly.

### Result

`scripts/measure_peephole_alloc_cost.py`, 21 `tests/bench/k_*.c` inputs, `-O1 -S`,
2 repeats, `--require-improvement`:

| metric | before | after | delta |
| --- | --- | --- | --- |
| allocations | 1,934,076 | 1,748,966 | **−185,110 (−9.571 %)** |
| assembly | — | — | byte-identical on 21/21 |

The 26th site (`is_full_flag_clobber`) does not move this number: the pass that
contains it does not fire on this corpus. It is included because the ratchet
correctly flags it and the copy is provably unnecessary, not because it was
measured to help here. Reporting it as a speedup would be a guess.

This is a **compile-cost** result. It is not a claim about generated-code speed,
and it does not establish MB-20's ≤35 % peephole criterion; steps 4–5 remain open.

### Output identity

* **x86-64:** 1,326 `tests/regression/*.c` × `-O1`/`-O2` = **2,652 compiles**;
  **2,540 identical, 0 differing**, 112 rejected identically by both arms.
* **i686 / aarch64 / riscv64:** 1,326 sources × 3 targets at `-O2` = **3,978
  compiles**; **1,190 identical, 0 differing**, 2,788 rejected identically
  (i686 394, aarch64 398, riscv64 398 compiled). Most of the corpus is
  x86-specific, so the cross-arch rejection rate is expected; the point is that
  exit status *and* output match everywhere. This turns the static argument
  (those backends never reference `backend::x86::codegen::peephole`; i686 shares
  only the assembler/encoder) into an empirical one. Both arms are the same
  multi-target driver invoked under an arch-specific `argv[0]`.

## Gates added

* `tests/regression/check_peephole_alloc_before_filter.py` — a **source ratchet**
  that fails if any site allocates a line before a filter can reject it. It
  accepts both compliant shapes. A source contract is used rather than a timing
  threshold because it is deterministic, needs no profiler, and pins the defect
  itself instead of a machine-dependent number.
  **Proven non-vacuous:** run against the pre-fix tree it reports **26
  violations** and exits 1; against the fixed tree, 0 and exit 0. Writing it
  caught a 26th site (`is_full_flag_clobber`) that the `let Some(..) else`
  transform had missed because it uses the `if !filter(..)` shape.
* `tests/regression/test_check_peephole_alloc_before_filter.py` — 14 contracts.
  The first version of the ratchet used `[^)]*` for the `trimmed(...)` argument,
  which cannot match the nested parens in `trimmed(store.get(i))`; it reported
  PASS on the fixed *and* unfixed tree. `test_nested_parens_are_matched` pins the
  real shape so that vacuity cannot recur, and `test_empty_tree_is_not_a_vacuous_pass`
  stops an empty scan from reporting success.
* `scripts/measure_peephole_alloc_cost.py` — the A/B ratchet. Fails on differing
  assembly, non-reproducible counts, compile failure, or allocation growth.
* `scripts/test_measure_peephole_alloc_cost.py` — 14 compiler-free contracts
  pinning the three traps and the verdict logic (a cost win that changes
  assembly **fails**).
* `tests/regression/alloc_count_shim.c` — the interposer, built `-Wall -Wextra
  -Werror`.

All four are wired into `.github/workflows/ci.yml`, `scripts/ci_local.sh` and
`.github/ci-lane-plan.json`.

## Verification

* `scripts/build_lccc_fast.sh`: exit 0, **0 warnings**.
* `cargo clippy --all-targets -- -D warnings`: **rc=0**.
* `cargo test --lib`: **8417 passed, 0 failed, 10 ignored**.
* `cargo test --lib` with debug-assertions + overflow-checks: **8431 passed,
  0 failed, 10 ignored**.
* `cargo fmt --all`, `git diff --check`: clean.
* `scripts/audit_census.py --check`: **PASS (no metric grew)**.
* CI static contracts: lane plan, shard partition, gate parity (557 commands),
  workflow shell — all PASS.
