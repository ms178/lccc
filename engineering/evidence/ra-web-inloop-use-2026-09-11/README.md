# RA web-wide in-loop-use supply — measured record (2026-09-11)

Raw paired A/B output behind landing the web-wide in-loop-use supply in
`live_range::mark_loop_spanning` (kill switch `CCC_NO_WEB_INLOOP_USE`,
`RaConfig::no_web_inloop_use`), and behind **withdrawing** the earlier decision
to make the cycle-accurate phi copy resolver the default.

Base for every number here: upstream `main` at `25ed36de` ("Merge pull request
#499"), i.e. **after** the rebase. Numbers recorded earlier against
`4f527199` are superseded and the reason is documented below — do not mix them.

## What was measured, and why it changed the disposition

Three binaries from one source, `tests/benchmark/programs/sha256_transform.c`:

| arm | compiler | env at compile time | meaning |
|---|---|---|---|
| A | pristine `25ed36de` | — | base |
| B | base + this patch | `CCC_NO_PHI_ACYCLIC_ORDER=1`¹ | **RA fix only** (shipping default) |
| C | base + this patch | — | RA fix + cycle-accurate phi resolver |

¹ measured while the phi resolver was still default-on; after the revert the
shipping default *is* arm B with no env at all, and its binary is byte-identical
(`sha256 503f6928c8516ee1f766bc27…`).

Amplified workload `-O2 -DPASSES=8 -DBLOCK_COUNT=131072` → ~430 ms/arm. The
un-amplified corpus default (~55 ms/arm) is also recorded, because the first
post-rebase reading was taken at that size and is visibly noisier.

All arms produce the same digest as `gcc -O2` (`054db5f638d89d8b` amplified,
`1a43d509d615897a` at PASSES=4/BLOCK_COUNT=32768), and the kernel's own built-in
FIPS 180-4 known-answer check gates every run (it returns 2 on failure).

### Results

| comparison | rounds | median ratio | min ratio | sign-test p | verdict |
|---|---|---|---|---|---|
| A → B (RA fix alone) | 51 | 0.9637 | 0.9587 | 0.0000 | **+3.63 %** |
| A → B (RA fix alone) | 41 | 0.9567 | 0.9612 | 0.0000 | **+4.33 %** |
| B → C (phi resolver on top) | 51 | 1.0199 | 1.0225 | 0.0008 | **−1.99 %** |
| B → C (phi resolver on top) | 41 | 1.0071 | 1.0254 | 0.0288 | **−0.71 %** |
| A → C (both) | 51 | 0.9812 | 0.9856 | 0.0000 | +1.88 % |
| A → C (both) | 41 | 0.9802 | 0.9829 | 0.0002 | +1.98 % |
| A → C (both, un-amplified ~55 ms) | 51 | 0.9834 | 0.9843 | 0.0000 | +1.66 % |

`identical_binaries=false` and `median_and_min_agree=true` in every record.
The decomposition composes: 0.9637 × 1.0199 = 0.9828 against 0.9812 measured
for A → C, a 0.16 % residual, so the three comparisons are mutually consistent
rather than three independent noise draws.

**Conclusion: ship the RA fix alone.** Adding the cycle-accurate phi resolver on
top of it halves the win (+4.0 % → +1.9 %), so the resolver stays opt-in
(`CCC_PHI_ACYCLIC_ORDER=1`). This *reverses* the pre-rebase reading, which had
the two changes as complementary and the pair as the winner (+8.21 %). That
reading is withdrawn; see "Why the earlier conclusion was wrong".

## Static / structural evidence

`sha256_transform`, `-O2`. The first two columns are function-scope and robust;
the loop columns come from the backward-jump span detector and are **only
comparable between LCCC arms** — read the detector caveat below before quoting
them, and do not compare them against gcc's.

| arm | insns | stack refs | span-detected loop | loop stack-memops | distinct slots | max per slot |
|---|---|---|---|---|---|---|
| A base | 210 | 62 | 188 insns | 59 | 16 | **17** |
| B RA fix only (**shipping**) | 198 | 61 | 176 insns | 58 | 17 | **10** |
| C both fixes | 178 | 28 | 156 insns | 25 | 9 | 10 |
| `gcc -O2` | **142** | **8** | 2 genuine loops: 36/0 and 47/0 | 0 | 0 | 0 |

The fix's mechanism is visible in the last column: the most-reloaded slot goes
from 17 accesses to 10, which is the two recurrence words `a` and `e` no longer
being demoted by the in-loop-USELESS-span admission rule. That comparison applies
the same detector to both arms, so it is internally consistent even though the
absolute span is not a single source loop.

Whole translation unit: A 396 insns / 149 stack refs → B 384 / 149.

### The remaining gap, stated honestly

Whole-function counts, no loop heuristics (`-O2`, `sha256_transform`):

| arm | insns | frame-relative stack refs | backward jumps | labels | cmp/test | mov |
|---|---|---|---|---|---|---|
| A base | 210 | 62 | 2 | 14 | 8 | 103 |
| B RA fix only (**shipping**) | 198 | 62 | 2 | 14 | 8 | 98 |
| C both fixes | 178 | 29 | 1 | 14 | 9 | 79 |
| `gcc -O2` | **142** | **8** | 2 | **5** | **2** | **61** |

And the runtime headline, measured the same way as everything else here
(31-round amplified paired A/B, arms digest-identical, `median_and_min_agree`):

| comparison | median ratio | min ratio | p | verdict |
|---|---|---|---|---|
| gcc −O2 → LCCC shipping | 1.4410 | 1.4361 | 0.0000 | **gcc is 44.10 % faster** |

So this patch's +4 % closes a small part of a 44 % gap. The gap's shape is
informative and it is **not** loop structure: LCCC and gcc emit the *same* number
of loops (2 backward jumps each), so nothing is fused or split differently. What
LCCC emits extra is **54 frame-relative stack references** (62 vs 8), **~40 more
`mov`s** (98 vs 61), **9 more labels** (14 vs 5) and **6 more compares** (8 vs 2).
Spill traffic and redundant moves, not control flow — which is what RA-06 has
always claimed, now with the loop-structure explanation ruled out.

gcc's two loops are tight and register-resident: 36 insns / 0 frame refs (message
schedule) and 47 insns / 0 frame refs (the 64 rounds), with only 3–4 memory
operands each, reached through register bases rather than the frame pointer.

**Detector caveat, recorded so nobody repeats it.** Attributing counts to "the hot
loop" by taking a backward jump's target-to-jump span is unreliable in LCCC's
output here: one span covers most of the function, so an early reading reported
"LCCC's round loop is 176 insns / 58 stack-memops vs gcc's 47 / 0" and suggested
the three source loops had been fused. The backward-jump counts above disprove the
fusion story (2 vs 2). Use whole-function counts for LCCC; gcc's two innermost
loops are genuinely separate and its per-loop numbers are trustworthy.

A second measurement error worth recording: an earlier reading reported gcc at
"240 insns / 31 stack refs" and concluded LCCC's 178/28 beat it. That was wrong.
The extractor matched `^\s*\.size <name>` with a literal space; gcc emits
`.size\tsha256_transform`, so the match failed and the code silently fell back to
the *whole translation unit*. Both gate scripts now use `\.size\s+<name>\b` and
**fail loudly** instead of falling back.

## `rot()` isolation matrix — why the phi gate's absolute target moved

The `phi-acyclic-copy-order` gate used to require the opt-in rotation kernel to
be fully register-allocated (55 insns / 0 stack refs). It no longer reaches that
at production settings, and the cause is not the resolver:

| `CCC_EVICT_SHORT_K` | resolver | insns / stkref |
|---|---|---|
| 0 (escape off) | off | 71 / 33 |
| 0 (escape off) | on | **56 / 2** |
| 16 (default) | off | 71 / 27 |
| 16 (default) | on | 61 / 4 |

Upstream's cost-ratio escape in `select_evict_victim` (`RaConfig::evict_short_k`,
landed in `d6e2a7f5`) is worth −6 stack refs on the legacy arm (33 → 27) and
costs +2 on the resolver arm (2 → 4). The resolver's own contribution is large at
both settings (−15 insns / −31 stkref at K=0; −10 / −23 at K=16), so the gate now
pins that contribution plus a `CCC_EVICT_SHORT_K=0` arm, rather than an absolute
zero that belongs to a different component. `check_phi_acyclic_order.sh` section 2
carries the full note.

The RA fix's effect on `rot()` at production settings: legacy arm 73 → 71 insns
(improvement), resolver arm 61 / 4 → 61 / 4 (byte-identical). No regression.

## Why the earlier conclusion was wrong

Pre-rebase, against `4f527199`, the same factorial read: phi alone −5.1 %, RA fix
alone −3.2 %, both **+8.21 %** — hence "complementary, ship together". Three
things were wrong with that reading:

1. **It was taken un-amplified.** At ~55 ms/arm on a 2-core shared VM the
   per-arm sd was 6–10 %. The +8.21 % median came with a min ratio of 0.9622
   (+3.8 %), i.e. the harness's own `median_and_min_agree` criterion was
   *violated* and the headline number was the contaminated estimator. Today's
   un-amplified re-run of the same pair reads +1.66 %, and the amplified one
   +1.88 % / +1.98 %.
2. **The base moved.** Upstream landed four commits, one of which
   (`d6e2a7f5`) changes eviction in `select_evict_victim`. Complementarity
   between a copy-ordering change and an allocator demotion rule is exactly the
   kind of result that does not survive an allocator change.
3. **The "RA fix alone is a regression" leg was never re-measured amplified.**
   Amplified, it is the *only* leg that wins: +3.63 % / +4.33 %, p=0.0000, with
   median and min agreeing in both replicates.

The lesson recorded in `engineering/agent/RULES.md`: a factorial conclusion must
be re-derived after any upstream allocator change, and never carried across a
rebase on the strength of an un-amplified median.

## The red-team audit: a 40.53 % regression found in our own fix, and removed

The supply was audited after it shipped, and the audit found two classes of
problem: defects in how it was *implemented*, and a defect in what it *fed*.

### Implementation defects (all fixed, machine code unchanged)

The first version hand-rolled a use-point walk over `func.blocks`. That
re-implemented `collect_range_metadata`, dropped terminator uses (which
`record_terminator_uses` records at the block-end point — and the block-end point
*is* `loop_extents`' latch end, so a value read only by a loop's terminating
branch was invisible), counted operand occurrences where the leader side counts
deduplicated distinct points (`set_uses_weighted` sorts and merges), placed its
`loop_extents.is_empty()` bail-out *after* both full IR walks, and left a
write-only `any_use_in_extent` map behind.

All five are fixed: the canonical `RangeMetadata::uses` is now threaded out of
`build_live_ranges_with_config_and_meta` and passed in, so the leader and member
sides of the web-wide sum share one numbering by construction; the dead wrapper
`collect_uses_for_values` and the dead map are gone; the bail-out is first.

Two of those were latent bugs rather than style, and both are now pinned by tests
that were **mutation-verified** — each fails against a mutant that reintroduces
exactly that defect, so neither is a tautology:

| test | mutant | result |
|---|---|---|
| `mark_loop_spanning_member_terminator_use_supplies_the_flag` | supply made blind to terminator points | FAILED (as required) |
| `mark_loop_spanning_member_duplicate_point_counts_once` | distinct-point dedup removed | FAILED (as required) |

The refactor itself is provably output-neutral: `scripts/differential_corpus.sh`
byte-compares `-O2 -S` output for every `.c` under `tests/` between the
hand-rolled and canonical-map builds — **805 of 807 translation units compile,
0 exit-status differences, identical 2-file failure set, 0 assembly
differences**. Identical machine code means the runtime win carries over exactly
and that re-timing it would be an uninformative identical-arm comparison
(`scripts/paired_ab.py` exits 3 on byte-identical arms by design).

### The regression the audit's own blast-radius screen found

Screening base `25ed36de` against the fix across the whole corpus — rather than
only re-timing `sha256_transform`, the benchmark the fix was aimed at — named 13
changed translation units. One of them was `lz4_compress`, and it was timed:

```
lz4_compress, -O2 -DSRC_SIZE=(1UL<<22) -DPASSES=96U, ~186 ms/arm, 41 rounds
  base         : min 182.71  med 183.94  mean 184.93  sd  2.36
  counts fed   : min 257.22  med 258.49  mean 271.33  sd 39.89
  median ratio 1.4053   min ratio 1.4078   sign-test p = 0.0000
  => base FASTER by 40.53 %   (251 instructions in main, BOTH arms)
```

Identical instruction count with a 40 % runtime gap is the case mission §22/§23
says to chase into dependency chains, latency and port pressure, and the trace
gave the mechanism:

```
counts NOT fed: demote v212 remcost=100  exposed=1 site=2   (admission cap)
counts fed    : v212 not demoted; demote v133 remcost=1110 exposed=3 site=5 (valve)
```

Web-wide counting pushed v212's `span_exposed_uses` from 1 to 3, past
`MAX_SPAN_EXPOSED_USES = 2`, so the admission cap's veto fired; the pressure then
fell to the span-pressure valve, which picks on `MAX_VALVE_SPAN_FUTURE_USES`
future-use points and has **no cost term**, so it spilled an 11× more expensive
span. `main`'s hot loop went 25 → 29 frame refs, the function 12 → 14 stores.

The web-wide count is not wrong — members share one register, so those three
reads really would become hot reloads. The wrongness is that `worth_capping` is a
veto with no cost-aware replacement, so a more accurate input produced a worse
global decision. That is recorded as
`engineering/FOLLOWUP-2026-09-11-valve-cost-blindness.md`, including the required
sequencing (make the valve cost-ordered *first*, then re-feed the counts).

### What ships: the decoupled design

The member supply feeds the web-wide BOOLEAN `span_has_in_loop_use` — which is
where the sha256 win actually comes from — and leaves the cap's per-range counts
exactly as calibrated. Measured consequences:

| | counts coupled | decoupled (ships) |
|---|---|---|
| corpus TUs changed vs base | 13 of 805 | **10 of 805** |
| `lz4_compress` | −40.53 % | **byte-identical to base** |
| `sha256_transform` asm | — | **byte-identical to the coupled build** |
| `linux_rbtree` asm | — | **byte-identical to the coupled build** |
| also stops touching | — | `divrem_pair_opposite_flavour.c`, `o0_phi_multidef.c` |

Because `sha256_transform`'s and `linux_rbtree`'s assembly is bit-for-bit
unchanged by the decoupling, every sha256 number published above in this document
still describes the shipping compiler.

Runtime, base → decoupled, paired interleaved, both estimators reported:

| benchmark | amplification | rounds | median | min | p | verdict |
|---|---|---|---|---|---|---|
| `sha256_transform` | `PASSES=8 BLOCK_COUNT=131072`, ~430 ms | 31 | 0.9637 | 0.9619 | 0.0000 | **+3.63 %** |
| `linux_rbtree` | `NODE_COUNT=49152U`, ~65 ms | 61 | 0.9895 | 0.9894 | 0.0000 | **+1.05 %** |
| `strlen_bench` | `NSTRINGS=400000`, ~1.18 s | 31 | 0.9882 | 0.9711 | 0.0012 | +1.18 % (sd ≈ 9 %, magnitude soft) |
| `lz4_compress` | `SRC_SIZE=(1UL<<22) PASSES=96U` | — | — | — | — | arms byte-identical → UNINFORMATIVE by construction |
| `adler32_do8` | `bench_kernels.py --reps 15 --inner 3000` | 15 | 1.001× vs kill switch | — | — | neutral (69 insns both arms) |

`linux_rbtree` amplification is below the 200 ms guideline: `NODE_COUNT=65536U`
and any `LOOKUP_ROUNDS` change make the codegen difference vanish, so 65 ms at
`NODE_COUNT=49152U` is the largest run that still exercises the change. 61 rounds
and median/min agreement in the same direction are the compensation, and the
effect is small enough that it should be read as "not a regression, probably a
small win", not as a +1 % claim.

`adler32_do8` matters because `zlib_ng_adler32`'s checksum webs are the recorded
counter-example that calibrated `MAX_SPAN_REMCOST`; it is in the blast radius
(69 → 69 instructions, changed code) and measured neutral, and `k01_adler`
improved 62 → 59 instructions.

### Guard added

`tests/regression/check_ra_web_inloop_use.sh` property 5 now requires
`lz4_compress` to be byte-identical between the supply-on and supply-off arms. It
is mutation-verified: against a count-coupled build it fails with exit 1 and
reports "224 differing lines", while still confirming the sha256 mechanism fires.

## Reproduction

```sh
# compilers
git worktree add /var/tmp/basebuild 25ed36de && (cd /var/tmp/basebuild && bash scripts/build_lccc_fast.sh)
bash scripts/build_lccc_fast.sh                      # base + this patch

# arms
SRC=tests/benchmark/programs/sha256_transform.c
F='-O2 -DPASSES=8 -DBLOCK_COUNT=131072'
/var/tmp/basebuild/target/fastbuild/lccc $F $SRC -o A_base.bin
target/fastbuild/lccc                    $F $SRC -o C_both.bin
CCC_NO_PHI_ACYCLIC_ORDER=1 target/fastbuild/lccc $F $SRC -o B_rafix_only.bin   # pre-revert polarity

# paired, interleaved, identity- and correctness-screened
scripts/paired_ab.py --bin-a A_base.bin --bin-b B_rafix_only.bin \
    --name-a A-base --name-b B-rafix --rounds 51 --warmup 3 \
    --label 'sha256 RA web-inloop-use fix alone' --json <this-dir>/paired-sha256-rafix-only-51r.json
```

The `--rounds 41 --warmup 4` replicates were run with the same binaries and
different round/warm-up counts so the two readings do not share a drift pattern.

## Files

| file | content |
|---|---|
| `paired-sha256-rafix-only-51r.json` | A → B, 51 rounds, amplified (headline win) |
| `paired-sha256-rafix-only-41r.json` | A → B, 41 rounds, amplified (replicate) |
| `paired-sha256-phi-on-top-51r.json` | B → C, 51 rounds (resolver is negative) |
| `paired-sha256-phi-on-top-41r.json` | B → C, 41 rounds (replicate) |
| `paired-sha256-combined-51r.json` | A → C, 51 rounds |
| `paired-sha256-combined-41r.json` | A → C, 41 rounds (replicate) |
| `paired-sha256-unamplified-51r.json` | A → C at corpus-default size, showing the noise floor |
| `paired-sha256-base-vs-decoupled.json` | base → shipping (decoupled) build, 31 rounds: **+3.63 % / +3.81 %** |
| `paired-rbtree-base-vs-decoupled.json` | base → decoupled, `linux_rbtree`, 61 rounds: +1.05 % / +1.06 % |
| `paired-strlen-base-vs-decoupled.json` | base → decoupled, `strlen_bench`, 31 rounds: +1.18 % / +2.89 % (noisy kernel) |
| `paired-lz4-base-vs-decoupled.json` | base → decoupled: arms byte-identical, recorded UNINFORMATIVE (exit 3) |
| `paired-lz4-base-vs-coupled.json` | base → count-coupled build: **base 40.53 % FASTER** — the regression the audit found |
| `paired-sha256-base-vs-coupled.json` | base → count-coupled, 31 rounds: +3.68 % (same win, different collateral) |
| `paired-rbtree-base-vs-coupled.json` | base → count-coupled, 61 rounds: +1.02 % |
| `paired-strlen-base-vs-coupled.json` | base → count-coupled, 21 rounds: not significant |
| `corpus-differential.txt` | 807-TU byte differential, base vs BOTH designs: 13 changed (coupled) → 10 (decoupled), and the 3 files the decoupling stops touching |
| `paired-sha256-lccc-vs-gcc-31r.json` | gcc −O2 → LCCC shipping: **gcc 44.10 % faster** (median 1.4410, min 1.4361, p=0.0000) — the size of the gap this patch is chipping at |

Every JSON carries the per-round sample vectors, both arm hashes, the
identity/correctness screen results, and the sign-test inputs.
