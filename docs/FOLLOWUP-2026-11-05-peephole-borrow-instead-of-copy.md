# Peephole: borrow the line instead of copying it — and fix the ratchet that hid the worst site

**Status:** measured, output byte-identical. MB-20 step 3 continuation.

## How the target was chosen (no guesswork)

The previous two rounds of this work picked sites by reading source. That found
25 + 5 sites but left the expensive one untouched. This round started from
**ablation profiling** instead: compile a fixed 200-function TU with each of the
135 sub-passes individually disabled via `CCC_PEEPHOLE_SKIP`, and attribute the
difference in `CCC_TIME_PEEPHOLE`'s own total to that pass.

The first sweep ran two workers in parallel and produced a **−57.4 ms noise
floor** — larger than most reported deltas, so it was discarded rather than
reported. The re-run was serial, 5 reps, with a baseline measurement interleaved
before *and* after every candidate so machine drift cannot masquerade as a pass
cost:

| pass | ms saved | % of peephole | drift |
| --- | --- | --- | --- |
| **`lea_base_fold`** | **99.9** | **30.4 %** | +5.8 |
| `copy_prop` | 42.2 | 12.7 % | +0.1 |
| `dead_pure_writes` | 19.4 | 5.8 % | +5.6 |
| `symbol_lea_cse` | 10.8 | 3.2 % | +0.9 |
| `vec_self_move` | 10.9 | 3.3 % | −10.5 |
| `shl_sib` | 0.9 | 0.3 % | +8.2 |

`lea_base_fold` is `fold_copy_into_lea_base`, and its first act per line was:

```rust
let lea = infos[li].trimmed(store.get(li)).to_string();   // heap copy of EVERY line
let (lea_rest, prod_w) = if let Some(r) = lea.strip_prefix("leaq ") { (r, 64u8) }
    else if let Some(r) = lea.strip_prefix("leal ") { (r, 32u8) }
    else { continue };                                   // ~99% of lines land here
```

## Defect 1: the ratchet could not see this, or 22 other sites

`check_peephole_alloc_before_filter.py` recognised exactly two filter shapes:
`let Some(..) = freefn(&var) else {` and `if !freefn(&var) {`. Classifying **all
66** `trimmed(..).to_string()` bindings in the backends by the statement that
follows them showed what that missed:

| shape | count | example | old gate |
| --- | --- | --- | --- |
| `let X = if let Some(r) = var.strip_prefix(..) { .. } else { continue }` | 20 | the 30.4 % pass | **invisible** |
| `if freefn(&var) { continue }` (positive guard) | 1 | `has_implicit_reg_usage` | **invisible** |
| `let Some(..) = var.method(..) else { .. }` | 1 | `t.strip_prefix` | **invisible** |
| `if !var.method(..) { continue }` | 1 | `t.starts_with` | **invisible** |
| window-matcher closure `let text = \|k\| ..to_string()` | 3 | `match_highbit_sub` | **invisible** |

The gate reported PASS on a tree containing all of them. It was not wrong about
what it checked; its *scope* was two regexes wide.

**Fix.** Express the contract the way it reads: *the first statement after the
copy must not be able to reject the line based on that copy's contents.* The
scanner walks one statement with a bracket-depth counter, so an if-let/else
chain is one statement rather than five lines, then flags it when it both
mentions the variable and contains `continue`/`break`/`return`.

**Non-vacuity, measured:** the new gate reports **39 violations** on the
unmodified `782b9f4` sources and **0** after this change. The old gate reported
**0** on both.

## Defect 2: 62 copies that were never needed at all

The previous rounds assumed an owned copy was usually necessary and settled for
"probe, then allocate". That assumption is wrong for most sites, and there is a
**sound oracle** for it rather than a judgement call:

* `LineStore` is `{ original: String, entries: Vec<LineEntry>, replacements: Vec<String> }`
  — no `Cell`, `RefCell` or `UnsafeCell`, so no interior mutability;
* its whole API is `new`, `get(&self)`, `len`, `replace(&mut self)`;
* `LineInfo::trimmed<'a>(&self, line: &'a str) -> &'a str` borrows only the line.

Therefore **if deleting `.to_string()` still compiles, no mutation can occur
while that borrow is live, so the bytes observed are identical by construction.**
The borrow checker is not a heuristic here; it is a proof.

An automated sweep deleted the copy at all 66 sites, compiled, restored the
sites the compiler rejected, and iterated to a fixpoint. The first pass's error
attribution was nearest-line and over-rejected (it condemned
`memory_fold.rs`'s `lj`, which does compile as a borrow), so every survivor was
then re-tested **individually**, one site per `cargo check`, for exact
attribution.

Result: **62 copy expressions deleted** across 14 files (51 x86-64, 11 i686).
**5 sites remain owned**, each individually proven to need it. Zero copies were
added back.

## Defect 3: three matchers demanded `String` for `&str` work

```rust
let text = |k: usize| infos[k].trimmed(store.get(k)).to_string();
let Some(m) = match_highbit_sub(&[text(0), text(1), text(2), text(3), text(4)]) else { continue };
```

Five heap copies per candidate window, before a highly selective matcher.
`match_highbit_sub`, `match_guarded_scan_select` and `match_preloaded_scan_copy`
were audited for `String`-specific API use: they call **only** `starts_with` and
`strip_prefix`, and their return structs (`HighbitSubRewrite`,
`GuardedScanRewrite`) are fully owned with no lifetime parameter. So the
parameters became `&[&str; N]` and the closures stopped copying — 12 allocations
per three candidate windows, gone.

## Defect 4: an unconditional whole-file scan for a rare answer

`eliminate_vector_self_moves` built `WiderMentions::scan(store, infos)` — a full
file scan — before looking at a single line. Its result is read at exactly one
place: the barrier fallback inside `vex_self_move_zeroes_live_upper`, which is
only reached for a VEX vector self-move candidate. Integer-only and legacy-SSE
files never reach it and paid the scan anyway.

The cache is now `Option<WiderMentions>`, built by `get_or_insert_with` **at that
one read site** — maximally lazy, not merely deferred to the first candidate.

**Equivalence argument.** Eagerly, the scan counted every non-NOP line and
`note_deleted` decremented the count when this pass NOPed a wider self-move.
Lazily, lines NOPed before the build are skipped *by the scan itself*
(`info.is_nop()`), which is the same arithmetic; lines NOPed after the build
still go through `note_deleted`. A deletion that would have decremented a
`%ymm`/`%zmm` count is necessarily a VEX candidate, so it passes through the
guard that builds the cache first. The two answers are identical.

## Defect 5 (i686): a copy paid on lines no pattern can use — and the trap in fixing it

`peephole.rs:12501` copied line `j` before dispatching on line `i` (`s`). Eleven
readers consume that copy, and **all eleven** sit inside arms guarded by one of
five `s` parsers. Those five were read exhaustively: `parse_mov_imm_reg`,
`parse_mov_imm_value`, `parse_mov_reg_reg`, `parse_store_from_reg` all begin
`strip_prefix("movl ")`; `parse_xor_self` begins `strip_prefix("xorl ")`.

The obvious fix — `if !s.starts_with("movl ") && !s.starts_with("xorl ") { continue }`
— **is a miscompile**, and the audit caught it: *Pattern 8 (adjacent block
tail-merge) further down the same loop body dispatches on block shape, not on
`s`*, and must still run for those lines. An early `continue` would have silently
disabled it.

The copy is instead made conditional, and the invariant is **machine-checked**
rather than comment-asserted:

```rust
let t: Option<String> = (s.starts_with("movl ") || s.starts_with("xorl "))
    .then(|| trimmed(store, &infos[j], j).to_string());
```

with every reader routed through

```rust
#[inline]
fn staging_line_text(t: &Option<String>) -> &str {
    t.as_deref()
        .expect("line-j copy owed: every reader is guarded by an `s` parser")
}
```

A future pattern that reads the copy without an `s` guard now fails loudly under
`debug-assertions` (which CI enables) instead of quietly matching against `""`.
`String::new()`/`None` allocates nothing, so the reject path is free.

The copy genuinely cannot be a plain borrow here: `t` must preserve line `j`'s
*original* text across `store.replace(j, ..)` at 12630/12669 for the reader at
12793. The compiler says so explicitly (`E0502`, "immutable borrow later used
here"), which is why this site is conditional rather than deleted.

## Measurement

Allocation A/B, `tests/bench/k_*.c`, `-O2 -S`, 2 repeats, `--require-improvement`
(the harness refuses to report a win unless every input's assembly is
byte-identical and the count reproduces exactly):

| metric | before | after | delta |
| --- | --- | --- | --- |
| allocations | 3,154,424 | 2,615,612 | **−538,812 (−17.081 %)** |
| allocator bytes | 555,926,729 | 547,474,410 | −8,452,319 |
| assembly | — | — | **byte-identical, 21/21** |

For comparison, the merged predecessor rounds measured −9.456 % (x86-64) and
−0.050 % (i686). Choosing the target by ablation instead of by reading source
roughly doubled the result.

Paired wall time on the profiled 200-function TU, measured **after** the build
and both test lanes finished, A/B interleaved, n=11 each:

| metric | before | after | delta |
| --- | --- | --- | --- |
| peephole total (internal timer) | 327.7 ms | 308.7 ms | **−5.80 %** |
| whole `-O2` compile | 844.8 ms | 832.4 ms | −1.46 % |
| assembly | — | — | byte-identical |

The whole-compile figure is inside the noise (interquartile ranges 837.6–892.6
before, 812.0–865.3 after, overlapping); the peephole figure comes from the
compiler's own timer and is the meaningful one. The allocation count is
deterministic and is the gate.

**An earlier wall-time measurement was discarded rather than reported.** It
showed +4.7 %, contradicting the allocation result; `ps` then showed
`clippy-driver` at 89 % CPU on this 2-core host during the run. Wall time here is
only meaningful when the box is idle — which is why the numbers above were taken
after every lane exited.

## Output identity across both changed backends

The patch touches only x86-64 (51 sites) and i686 (11 sites); arm and riscv have
zero changes, so those two backends cover 100 % of it. A/B over **1,349 sources**
(`tests/bench/k_*.c`, `tests/regression/*.c`, `tests/regression/alloc_cost/*.c`)
× 7 option sets — `-O1`, `-O2`, `-O2 -g`, `-O3`, `-O2 -m32`, `-O1 -m32`,
`-O2 -m32 -g` — = **9,443 compiles per arm**:

| | |
| --- | --- |
| byte-identical | **6,375** |
| **differing** | **0** |
| rejected identically by both arms | 3,068 |

Both arms are invoked at identical directory depth, because LCCC embeds include
paths derived from its own location into DWARF; comparing binaries in different
directories produces spurious `-g` diffs.

## Verification

| check | result |
| --- | --- |
| `scripts/build_lccc_fast.sh` (fastbuild, 4 GiB swap) | exit 0, **0 warnings** |
| `cargo check` | rc=0, **0 errors / 0 warnings** |
| `cargo clippy --all-targets -- -D warnings` | **rc=0** |
| `cargo test --lib` | **8471 passed, 0 failed, 10 ignored** |
| same + `debug-assertions` + `overflow-checks` | **8485 passed, 0 failed, 10 ignored** |
| ratchet on this tree | **PASS, 372 files** |
| ratchet on unmodified `782b9f4` | **39 violations** (non-vacuity proof) |
| ratchet contracts | **34 passed** |

The 34 contracts include the four previously-invisible shapes, the closure form,
`let mut`, `break`/`return` as rejection verbs, all five compliant shapes
(borrowed, probe-then-allocate, conditional copy, unconditional consumption,
stored-in-record), comment immunity in both directions, and two tests pinning the
gate's deliberate scope boundary so it is a decision rather than an accident.

## Known scope limit

The ratchet checks the **first** statement after the copy. A copy consumed by an
intervening binding and rejected afterwards (`let n = t.len(); if n == 0 { continue; }`)
is not flagged. No such site exists in the tree today; widening the scan would
flag legitimate multi-step consumers. `ScopeBoundary` pins this so a future
widening is a deliberate change with a failing test to update.
