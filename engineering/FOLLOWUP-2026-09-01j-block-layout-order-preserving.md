# Follow-up: a 19% layout defect, and the attribution method that found it

Session date: 2026-09-01 (continuation)
Base: `ms178/lccc` main @ `bcdd1d31`
Snapshots: `S36`, `S37`

---

## 1. Kritik und Selbstkritik: the previous report's regressions were not measured

The last report listed `arith_loop 1.227 → 1.469` and `spectral_norm
1.305 → 1.489` as regressions. That claim was **not supported by the data I
had**. The two numbers came from different runs, days apart, on a shared VM,
with different rep counts (15 vs 9). I even wrote in the same document that
absolute times were not comparable across reports — and then compared ratios
across reports anyway, which is the same error one level up: a ratio is only
stable if *both* arms were measured in the same window, and drift that hits
the two compilers unequally moves it.

The only valid attribution is a paired A/B **in one window on one machine**,
isolating the suspect passes by kill switch. Doing that produced a completely
different picture:

| kernel | my passes on/off, same window | verdict |
|---|---|---|
| `zlib_ng_adler32` | **1.172** | real, large |
| `expat_xml_scan` | 1.043 | real, just above the 3.4% noise floor |
| `arith_loop` | 1.021 | **within noise — not a regression** |
| `spectral_norm` | **0.934** | my passes make it **6.6% faster** |
| `sqlite_varint` | 0.989 | faster |
| `struct_copy` | 0.984 | faster |

Two of the three "regressions" I reported did not exist, and one of them was
an improvement.

## 2. Two wrong hypotheses, both implemented and measured

Bisecting `zlib_ng_adler32` per pass pointed at block layout
(`CCC_NO_BLOCK_RELAYOUT=1` → 47.7 vs 55.9 ms). I then guessed twice at *why*,
implemented both, and measured both:

1. **"Only sink blocks that never re-enter the loop."** Provable, cheap — and
   changed the emitted code **not at all**. The blocks being sunk already
   satisfied it.
2. **"…and only at loop depth 0."** Motivated by a real observation (adler32
   is NMAX-chunked, so an inner loop's exit block runs once per *outer*
   iteration and is hot). Also changed nothing measurable.

Both are kept — they are correct guards that prevent sinking a hot block — but
neither was the cost. Writing them down is the point: the next person does not
re-derive them.

## 3. The actual defect: RPO is a linearization, not a layout

The third measurement is the one that mattered, and it required comparing the
right two things. `CCC_NO_BLOCK_RELAYOUT=1` disables the **whole** pass,
including the reverse-post-order baseline that predates the loop-aware work.
I had been attributing a pass I only *modified* to myself. Comparing the three
states separately:

| layout | `zlib_ng_adler32` |
|---|---|
| none | **46.9 ms** |
| reverse post-order (pre-existing) | 55.8 ms |
| loop-aware (mine) | 55.5 ms |

**The pre-existing RPO relayout costs 19%**, and the loop-aware version is
marginally better than it. My change was not the regression; it inherited one.

RPO is a correct topological order and a **terrible layout**: it discards the
ordering earlier passes deliberately produced, and the backend then inverts
every conditional whose fall-through changed — turning not-taken hot edges
into taken ones.

And the pass never needed RPO. Its own docstring states the single reason it
exists: passes append new blocks at the **end** of `func.blocks`, so a loop's
exit block can end up far from its loop and stretch live intervals across
unrelated code. That is a **contiguity** problem. Contiguity is all that needs
fixing; everything else about the incoming order is information.

**The fix:** start from the existing order, apply only the loop-contiguity
compaction, park unreachable blocks at the end.

| kernel | before | after | vs GCC |
|---|---|---|---|
| `zlib_ng_adler32` | 55.7 ms | **47.0 ms** (−15.6%) | 1.468 → **1.243** |
| `sqlite_varint` | 32.1 ms | **30.7 ms** (−4.4%) | 1.285 → **1.244** |
| `memchr` | — | unchanged | **1.39× faster than GCC** |

Every other measured kernel is unchanged and every output byte-identical.

## 4. Tests

Two new cases pin the property whose absence cost 19%:

- `a_function_with_contiguous_loops_is_not_reordered_at_all` — the shape is
  chosen so RPO *would* permute it (block 3 precedes block 2 in program order
  while the CFG reaches 2 first). The pass must return "no change".
- `an_appended_out_of_order_block_is_still_compacted` — the case the pass
  exists for still works.

Eight `block_layout` tests total.

---

## 5. Validation

| Gate | Result |
|---|---|
| `cargo test --lib` | **1476 pass / 0 fail / 6 ignored** |
| `./scripts/run_regression_suite.sh` | **PASS=562 FAIL=0 SKIP=15**, AB-diff 0 |
| `ir_verify_sweep.py --levels O0..Oz` | **0 violations** / 3372 configs |
| MachInst 7-layer suite | green against **GAS 2.47.20260726** |
| Build | warning-free |

Oracle, `adler32` at `-O3 -march=x86-64-v3` (instructions): lccc 119, GCC 105,
ICX 93, Clang 87, ICC 76. **lccc is last on this kernel** — the remaining gap
is the DO8 accumulator chain, not layout, and it is now the honest next target
rather than being masked by a layout defect.

---

## 6. To do next

1. **`adler32` accumulator chain** — lccc 119 instructions vs ICC 76. Now that
   layout is not distorting it, this is a clean SLP/reassociation target.
2. **`expat_xml_scan` 1.043 attributable to my passes**, on top of the 2.1×
   classifier gap. The classifier itself is backlog `PF-CLS-1` (the
   eleven-branch chain, blocked on if-converting the pure SESE test region
   after splitting the critical edge).
3. **Clobber modelling in MachInst, then `Call`** — 7.8% of instructions, the
   last large coverage class.
4. Unchanged: `Store(float)`/XMM class, def-dominates-use in the IR verifier,
   linker oracle.

---

## 7. Notes for whoever picks this up

- **Never attribute across runs.** Kill-switch A/B in one window, or do not
  make the claim. Two of three reported regressions evaporated.
- **Check what the switch actually disables.** `CCC_NO_BLOCK_RELAYOUT` turns
  off a pass that existed long before the part I wrote; I spent two
  hypotheses inside my own code before comparing against the right baseline.
- **A pass can be correct, tested, and still be the wrong transform.** RPO
  linearization was none of it wrong — it simply optimized the wrong
  objective, and nothing in the suite could see it because every output stayed
  byte-identical.
