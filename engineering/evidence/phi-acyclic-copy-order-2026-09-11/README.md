# Phi acyclic copy order — raw A/B evidence, 2026-09-11

Frozen measurement record for the decision to land the cycle-accurate phi copy
resolver **opt-in** (`CCC_PHI_ACYCLIC_ORDER=1`) rather than as the default.
Full root-cause write-up:
[`../../FOLLOWUP-2026-09-11-phi-acyclic-copy-order.md`](../../FOLLOWUP-2026-09-11-phi-acyclic-copy-order.md).

All timings are shared-VM wall-clock screens from `scripts/paired_ab.py`:
25 counted rounds after 2 discarded warm-ups, both arms interleaved *within*
each round, arm order alternated round to round. Both arms' stdout and exit
status were compared before any timing. **Not PMU measurements** — this VM
exposes no usable counters (MS-14).

| File | What |
|------|------|
| `paired-sha256-transform.{json,txt}` | the decision-bearing run: `sha256_transform`, `-O2 -DPASSES=4 -DBLOCK_COUNT=32768` |
| `noise-floor-base64-identical-arms.{json,txt}` | `base64_enc` with the knob set on one arm — the two binaries are **byte-identical**, measured deliberately with `--allow-identical` |
| `paired-fib.{json,txt}` | `fib`, the only other corpus kernel whose code changes under the knob |

Reproduce any of them with the exact command in the `.txt` header, e.g.

```
scripts/paired_ab.py --src tests/benchmark/programs/sha256_transform.c \
  --ccc target/fastbuild/lccc \
  --cflags '-O2 -DPASSES=4 -DBLOCK_COUNT=32768' \
  --env-a '' --env-b 'CCC_PHI_ACYCLIC_ORDER=1' \
  --name-a default --name-b opt-in --rounds 25
```

## Results

| kernel | arms differ? | default med | opt-in med | verdict | sign-test p |
|---|---|---|---|---|---|
| `sha256_transform` | **yes** | 40.30 ms | 42.34 ms | opt-in **5.08% slower** | **0.0003** |
| `base64_enc` | **no — byte-identical** | 1.09 ms | 1.15 ms | "opt-in 5.67% slower" | 0.0164 |
| `fib` | yes | 1.04 ms | 0.99 ms | opt-in 4.84% faster | 0.2301 (n.s.) |

`sha256_transform` is the only result that supports a conclusion. Its `min`
ratio (1.0511) agrees with its `median` ratio (1.0508), the sign test rejects
the null at `p = 0.0003`, and the direction reproduces across independent runs
(7.05% at 25 rounds, 5.46% at 21 rounds, 5.08% here — the magnitude moves with
VM load, the direction does not). `fib` runs ~1 ms per invocation, below this
harness's resolution, and its delta is not significant.

## The methodological result — why the identity guard is load-bearing

The `base64_enc` row is the important one, and it is *not* a performance result.
The knob does not affect that kernel at all: both arms hash to the same sha256,
so the true effect is **exactly zero**. The harness nonetheless measured a
**5.67%** median delta with a paired sign test at **`p = 0.0164`** — nominally
significant at the conventional 0.05 threshold.

Interleaving and order alternation did not remove it. On a 2-core shared VM the
bias is systematic (neighbour load, page-cache state, frequency drift) and
correlates across a round, so a paired test over paired samples inherits it
rather than averaging it out.

**Consequence: statistical significance does not imply a real effect, and no
amount of paired-round discipline substitutes for checking that the two arms
actually differ.** `scripts/paired_ab.py` therefore hashes both arms and exits
`3` *UNINFORMATIVE* with no verdict when they match; `--allow-identical` exists
only to measure this noise floor deliberately, as here.

This is the failure mode that made `perf_ab.py`'s corpus geomean unusable for
this decision: screening the knob across the corpus, **6 of 8 arms compiled
byte-identically**, and their noise was averaged together with the two real
deltas into a single "2.08% slower" verdict.

## Non-regression proof for the shipped default

Independent of timing: the default arm emits **byte-identical assembly to base
`4f527199` across 450 translation units** (51 `tests/benchmark/programs/*.c`
plus 399 further `tests/**/*.c`; one file is not compilable by either arm),
verified by building the base commit in a separate worktree and `cmp`-ing every
`-O2 -S` output. The measured regression is therefore entirely contained behind
the opt-in flag.
