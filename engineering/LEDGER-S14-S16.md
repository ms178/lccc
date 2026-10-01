# Ledger — S14 … S16

Branch `perf-main`, rebased onto `ms178/lccc` `origin/main` = **8db75621**.

All four commits that were lost to the workspace reset
(`587659b19`, `76d80de62`, `681d3c8b9`, `b07c40983`) were **re-derived from
first principles**, not recovered. Nothing below is asserted to be identical to
what was lost. The only recovered material was a *design note*
(`/home/user/artifacts/ms178-1.S10-zero-ext-cmp-peephole-and-lz4-iv-spill-rootcause.patch`,
whose FOLLOWUP-2026-09-30 lz4-match-extend-iv-spill section is the
root-cause write-up that identified the stack slot) and the recorded
checksum `29f49acaab81f800`. That note is not a file in this tree, so it is
named by artifact path rather than linked.

## Commits

| SHA | What |
|---|---|
| `2c51e29e` | rebased S13: precise flag-walk for the `cmp`→`test` zero-compare fold |
| `10355078` | rebased S13: harden `flags_written_mask`; pin the SDM masks |
| `3098989a` | **S14** per-path flag liveness in the flag-consumer walk |
| `de99d61e` | **S15** coalesce a loop induction variable left in a stack slot |
| `4ea730db` | **S16** close two loop-shape gaps + correct the label rationale |

## S14 — per-path flag liveness

The flag-consumer walk stopped at **any** writer, so a *partial* writer (`stc`
= CF only, `sahf` = all but OF) hid every consumer behind it.
`eliminate_redundant_self_test` deletes `test %R,%R` based on
`flag_consumers_are_zf_only`, so a `jo` behind an `stc` read the producer's OF
without the walk ever seeing it and the fold fired. Executed as an A/B this
returns 1 originally and 0 folded at `x = 0x7fffffff`.

Fix: thread a per-path `LIVE` mask through the worklist and intersect it with
`cc_reads(cc)` and `!preserved` at each consumer. A writer only **kills** the
flags it defines; `cmc` is special-cased because it *complements* CF rather than
defining it. Strictly more information than the old static `preserved`
comparison, and strictly more precise: a full writer still ends the walk, and a
consumer of an already-retired flag is charged nothing.

Side effect: `flags_reach_a_whole_flags_reader` (`preserved = 0`) became
strictly dominated and lost its last production caller, so it was removed rather
than left as a trap.

## S15/S16 — loop-IV spill coalescing

`Value(ip)`'s live range spans most of `main`, so the windowed allocator leaves
it in `112(%rsp)` and the loop pays a **store-to-load forwarding round trip on
its own recurrence**. `%rcx` was already live from the reload through the
increment to the store — the register was never lost, it was just
re-synchronised with memory every iteration. The pass hoists the reload onto
the entry edge and sinks the store to the exits.

The rewrite is **transparent to the loop body**: the register holds the same
value at every program point. The obligations are about the slot, not the
register, except that nothing in the loop may clobber the family behind the
pass's back (the next iteration's reload used to repair stray writes).

### Measured

| | value |
|---|---|
| checksum (pass on = pass off) | `29f49acaab81f800` |
| lz4_match_extend, 11 paired reps | **−22.45 %**, ratio `0.7755`, **11/11 wins**, IQR `[0.7647, 0.7955]` |
| earlier run this session | −22.28 %, ratio `0.7772`, 11/11, IQR `[0.7596, 0.7953]` |
| historical pre-reset run | −23.46 %, ratio `0.7654`, 11/11, IQR `[0.7123, 0.8357]` |
| hot loop, instructions | 11 → 9 |
| corpus differential | **1 of 937** programs changes assembly, and it is the target |
| unit tests | 3951 pass, 0 fail |

### Red-team findings, and what each cost

1. **Upstream miscompile** (`3098989a`) — fixed; see above.
2. **Per-latch natural loops** (`4ea730db`) — a two-latch loop's other arm is
   executed every iteration but is in neither natural loop, so its exits were
   invisible. Now the union over all back edges to the header.
3. **Non-terminator branch out of the loop** (`4ea730db`) — the exit model is
   stated over terminators, so a conditional early exit emitted *ahead* of an
   unconditional jump was not an exit site. Now refused.
4. **Redundant reachability guard removed** (`4ea730db`) — a "loop-private
   block" analysis was added, then deleted: it could reject nothing the
   terminator guard had not, and no test distinguished it. Redundant machinery
   with no discriminating test is the same mistake as the liveness memo reverted
   earlier.
5. **Label-duplication rationale corrected** (`4ea730db`) — the claim that
   lccc's inliner duplicates callee-local labels is **not supported**: 0 of 935
   corpus assemblies contain a duplicated local label, and a forced
   `always_inline` function inlined at two sites emits 8 distinct labels. The
   emitter renumbers per unit. The uniqueness search is kept (a collision is a
   silent miscompile, the cost is nil) but no longer cited as a fix for an
   observed defect.

### Test discipline

Every refusal test was checked to **fail against the un-guarded pass**. Two
earlier drafts did not discriminate and were discarded rather than kept:

* an operand-padding assertion (`movq  %rcx` vs `movq %rcx`) that silently
  matched nothing, turning every "must survive" check into `0 == 1`;
* a two-latch fixture appended **after the epilogue's `ret`**, so the four
  "an extra instruction inside the loop" tests never placed an extra
  instruction inside a loop.

## Gates

| gate | result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --all-targets -D warnings` | 0 warnings |
| `scripts/ci_local.sh --fast` | 123 passed, 7 failed |
| `cargo test --lib` | 3951 passed, 0 failed, 7 ignored |

The 7 `ci_local.sh --fast` failures are **pre-existing environmental**, all
`-m32`: the image has no 32-bit libc (`/usr/include/i386-linux-gnu/bits/…`,
`crti.o`, `libgcc_s.so.1` all absent), so they die in the preprocessor or the
linker, strictly upstream of the x86-64 peephole. They fail identically with
the pass disabled, and `engineering/DECISIONS.md` plus four prior
`engineering/FOLLOWUP-*.md` records the same failures. The apt/multilib
workaround was **not** reapplied.

Affected: `reassoc-latency`, `regression-corpus-link`, `copy-alias-sizes`,
`notype-code-routing`, `i686-integer-isa-parity`, `map-i64-two-lane`,
`linker-suite`.
