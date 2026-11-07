# `-g` changed the code: hidden debug rows became basic-block leaders

**Status: RESOLVED** (this PR). Confirmed on CI, reproduced locally, root-caused
by instrumentation, fixed in three backends, pinned by five regression tests,
and the duplicated rule folded into one shared predicate.

## Symptom

CI job *Assembler and linker contracts*, step *Verify -g leaves generated code
unchanged*, failed on the PR while `main` was green. Locally the same gate
passed — because it **skipped** the failing rows.

`check_debug_info_invariance.sh` records `SKIP` when the *plain* compile fails.
This host has no i386 multilib headers, so every i686 row failed to preprocess
and was skipped: 195 skips locally against CI's working multilib. A green local
run was therefore not evidence about the row CI runs.

Reproducing CI's coverage needed the staged sysroot's headers on the search
path. A wrapper supplying

```sh
-idirafter /usr/include/x86_64-linux-gnu \
-idirafter /var/tmp/lccc-i686-sysroot/usr/include/x86_64-linux-gnu
```

dropped skips from 195 to 53 and exposed exactly one failure:

```
FAIL lccc-i686 -O1 crc32_index_narrowing_variants.c: -g changes the code
    671c671
    <     movl %eax, %esi
    ---
    >     movl 12(%esp), %esi
```

Note the direction: `-g` produced the **worse** code. This is not "debug info
perturbs an allocator tie" — an optimization was lost.

## Bisection

| Experiment | Result | Conclusion |
| --- | --- | --- |
| `CCC_DUMP_EACH_PASS` IR, `-g` vs `-g0` | byte-identical (10078 lines each) | not the optimizer |
| `CCC_NO_I686_PEEPHOLE=1` | invariant | the i686 peephole |
| `CCC_NO_GLOBAL_SLOT_FWD=1` | invariant | `forward_slot_loads_cfg` |
| `CCC_RA_GLOBAL_LOCATION=0`, `CCC_NO_SPLIT_RANGES=1`, `CCC_DISABLE_PASSES=ipa_ra`, `CCC_NO_LIVE_RANGE_SHRINKAGE=1` | all still differ | not register allocation |

## Root cause

`hide_debug_rows` correctly marks `.loc` rows and D6 endpoints deleted
(`LineKind::Nop`), and the block builder correctly ignores `Nop` when deciding
whether a row is *itself* a label or an edge. The defect was in how the
**fall-through leader** was chosen:

```rust
if j + 1 < fe {
    starts.push(j + 1);   // j+1 may be a hidden row
}
```

An endpoint is routinely emitted immediately after a transfer:

```asm
    jmp .LBB52
.Llccc_inline_e_1_14_main:      <-- hidden, but pushed as a leader
.LBB65:
```

so the hidden row became a block start, and the real label after it opened a
*second* block. Instrumenting the pass printed the block count directly:

```
plain: nb=120      debug: nb=122
plain: apply j=642 "movl 12(%esp), %esi" forward=Some((0, 6, L))
debug: apply j=742 "movl 12(%esp), %esi" forward=None
```

Two phantom blocks changed the join structure the dataflow meets over, and the
forward was lost. Every marker was verifiably hidden (`hidden=true`,
`kind=Nop`) — hiding was never the problem; **leader selection was**.

### Fix

A leader must name a row that survives into the code. Skip rows that emit
nothing; a real label stops the walk and dedups with the leader its own arm
already pushed:

```rust
let mut lead = j + 1;
while lead < fe && row_does_no_work(store, infos, lead) {
    lead += 1;
}
if lead < fe { starts.push(lead); }
```

This makes the CFG a function of the live text alone, so `-g` invariance holds
by construction rather than by each pass remembering to step over debug rows.

### One rule, one spelling

The first version of this fix left the "does this row do work?" rule written out
in four places: the analysis budget, the leader walk, the apply-phase budget and
the dead-store walk. They were not even identical -- the leader walk skipped
only deleted/blank rows while the budget also skipped `.loc`. In production
`hide_debug_rows` rewrites `.loc` to `Nop`, so the two agreed by accident; a
caller analysing unhidden text would still have made a `.loc` a block leader.

That is the same defect class this PR already fixed once for assembly dialects:
one rule with two implementations, where the second silently wins. All four
sites now call one `#[inline]` predicate:

```rust
fn row_does_no_work(store: &LineStore, infos: &[LineInfo], j: usize) -> bool {
    infos[j].is_nop() || infos[j].kind == LineKind::Empty || is_debug_location(store, infos, j)
}
```

The two integer tests answer first, so the string probe inside
`is_debug_location` is never reached on a hidden row -- the hot path is
unchanged. A fifth test drives the pass on deliberately **unhidden** text and
requires the forward to survive, which is what pins the property the four
spellings could not.

### The same construction in the other backends

A tree-wide scan for `starts.push(<var> + 1)` found three sites:

| Backend | Site | Status |
| --- | --- | --- |
| i686 | `forward_slot_loads_cfg_range` | confirmed by CI, fixed |
| AArch64 | `Liveness::build_with` | same construction, fixed |
| RISC-V | `RvCfg::build` | same construction, fixed |
| x86-64 | — | no block-leader builder; `successor_lines` is a line-level worklist whose transfer is the identity on inert rows, so extra rows cannot change its fixpoint |

## A second, independent defect found on the way

`slot_fwd_block` charged its work budget **before** the skip guards:

```rust
*budget -= 1;                                  // charged first
if infos[j].is_nop() || ... { continue; }      // hidden rows do no work
if is_debug_location(..) { continue; }
```

so every hidden row spent budget. The doc comment says the budget "bounds the
work"; charging for skipped rows contradicts that and makes the analysis depth
a function of the debug row count. The same pattern existed in the
never-read-store proof (`budget = 20_000`). Both now charge after the guards.

This was **not** the cause of the CI row — the fix was applied first and the row
still failed, which is what forced the instrumentation above. It is a real
defect on its own terms (a `-g` build silently bought less optimization, and a
sufficiently large debug-heavy function would abandon the pass entirely), so it
is fixed and pinned separately rather than reverted.

`apply_budget` was also sized from `fe - fs`, putting the raw line count back
into the pass. It now counts the rows the transfer actually processes.

## Verification

* `check_debug_info_invariance.sh` with CI-equivalent i686 coverage:
  **303 compilations identical**, 274 sentinels, 0 failures (was 1 of 303).
* The previously degraded instruction is now `movl %eax, %esi` in **both**
  builds — the fix restores the optimization under `-g`, it does not disable it
  for both.
* Four new tests:
  * i686 `cfg_fwd_hidden_debug_rows_do_not_split_the_join` — the exact phantom
    leader shape; asserts the forward fires in *both* variants, so it cannot
    pass vacuously by optimizing neither.
  * i686 `cfg_fwd_debug_rows_do_not_spend_the_analysis_budget` — 45 000 hidden
    rows, past the 40 000 budget, must still forward.
  * AArch64 / RISC-V `hidden_debug_rows_do_not_change_the_code` — invariance plus
    a redundancy assertion that keeps them non-vacuous.
  * i686 `cfg_fwd_is_debug_invariant_even_without_hiding` — runs the pass without
    `hide_debug_rows` and still requires the join forward, pinning the shared
    predicate rather than the caller's discipline.

## Lesson worth keeping

A gate that skips a row it cannot build is not evidence about that row. The
local run was green for 195 reasons that had nothing to do with correctness.
When CI fails a gate that passes locally, the first question is **what does CI
run that I skip**, not "is CI flaky".
