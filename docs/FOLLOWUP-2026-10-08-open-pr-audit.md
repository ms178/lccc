# Session follow-up 2026-10-08 — audit of all three open PRs, and the unified tree

Base `main` = `500e652e` (the merge of PR #775).  Open PRs audited: **#777**
(`cc93cda1`), **#776** (`6b3d75bf`), **#725** (`a0c3a7c3`).  Every number in this
document was produced on this host in this session; the raw artifacts are in
`engineering/evidence/2026-10-08-pr-audit/`.

**Host and its limits.** 2-core Xeon VM, Debian 13, gcc 14.2, binutils 2.44,
Valgrind 3.24.0 — *not* the i7-14700KF this project targets.  No PMU, so there is
no cycle, uop or speedup claim anywhere below: every dynamic number is Callgrind
simulated `Ir`, which counts executed instructions exactly and is unaffected by
the VM's ~15% wall-clock layout noise.  No clang, no mold/lld, no aarch64 runner
and no qemu, so PR #776's AArch64 half is carried on the strength of a clean
build plus its own golden/doccheck scripts, and that limit is stated rather than
hidden.

---

## 1. Verdicts

| PR | Verdict | Measured basis |
|---|---|---|
| **#777** scale-ring wrap gate | **ACCEPT the gate; REJECT the `loop_invert` half; REJECT its test as shipped (rewritten); REJECT its perf claims as stated** | The bug is real and worse than filed (§2). The gate fixes 100% of the enumerated class. Its cost is recoverable and this tree recovers it (§4). Its `k_varint` claim is impossible (§3). Its test cannot see the `-O2` class it fixes (§2.3). |
| **#776** AArch64 + ELF + x86 hardening | **ACCEPT** | Applies cleanly, builds with 0 errors/0 warnings (never built by its author), x86-64 codegen 26 files smaller / 0 larger, wrap sweep 78/78, regression suite 846/0/8 (§6). |
| **#725** directive classifier + volatile gate | **ACCEPT with one defect fixed** | Byte-identical on 636 comparisons, so it refuses no real fold. Its benign allowlist names four data-emitting directives; two tests that fail on the PR as shipped prove it (§5). |

---

## 2. The wrap miscompile: real, and mis-filed as an `-O1` phenomenon

### 2.1 Severity, measured

`scripts/wrap_ring_sweep.py` (shipped in this tree) enumerates the *class* rather
than one instance: 39 shapes × {constant index, index routed through a `volatile`}
× {`-O0`,`-O1`,`-O2`,`-O3`}, each its own translation unit and process, compared
against executed gcc 14.2 output.

| | constant index | runtime index |
|---|---|---|
| main `-O0` | 39/39 | 78/78 |
| main `-O1` | **18/39** | **36/78** (42 divergent) |
| main `-O2` | 39/39 | **57/78** (21 divergent) |
| main `-O3` | 39/39 | **57/78** (21 divergent) |
| PR #777 | all agree | all agree |
| this tree | all agree | all agree |

The divergent set is exactly the gated peel family: `mul 2/4/8`, `shl 1/2/3`,
`i+i`, `i+k`, `i-k`, `(i+1)*2`, `i*2` from the top of the ring, and all three loop
variants — each reading the *linear* marker instead of the wrapped one (396 for
220, 132 for 44, …), plus one SIGSEGV (`u32_top_mul2` at `-O1`).  Shapes that
agree even unfixed are the ones the peel never claimed: scale 3 and 16 (no SIB
scale), every masked/modulus bound, the `struct[i]` element control (C scales in
`ptrdiff_t`, so *linear* is correct there), and the signed/u64/small controls.

**The constant-index column is the whole story of how this was mis-filed.**  With
a constant index, IPCP folds the access and main agrees with gcc at `-O2`/`-O3`;
the same source with a runtime index miscompiles at both.  A severity claim drawn
from constant-index cases alone understates a default-`-O2` bug as an `-O1` one.

### 2.2 PR #777's gate is correct and complete for the class

`scale_ring_is_linear(ty) = is_integer && (size >= target_ptr_size() || !unsigned)`
admits exactly the rings where the address arithmetic reproduces the index
arithmetic: pointer-width integers (one ring) and signed narrow integers (overflow
is UB, C17 6.5/5, so the linear spelling cannot differ on a defined execution).
It refuses unsigned narrow — the only case where wrap is *defined* and therefore
observable.  78/78 across three optimisation levels confirms the refusal set is
complete for the enumerated shapes, and the ILP32 exemption (`size >= ptr`) is
right because there the narrow ring *is* the address ring.

### 2.3 Its shipped test cannot see the bug at `-O2`

`tests/regression/ivsr_scale_ring_wrap.c` as shipped, compiled with **unfixed
main**:

```
-O1  FAIL const / FAIL param / FAIL dynamic      (exit 5)
-O2  FAIL param only                             (exit 5)
-O3  FAIL param only                             (exit 5)
```

Three defects, each verified rather than inferred:

1. **The "dynamic" case is not dynamic.** `uint32_t lim = 0x80000003u + (k & 0u)`
   folds under SCCP; main compiles `walk_dynamic` to four straight-line loads with
   no loop at all (`movl $4294967294, %eax; addq %rax,%rsi; movzbl (%rsi),%edx; …`).
   So the suite has two effective shapes, not three, and the loop-that-stays-a-loop
   path — the one that miscompiles at `-O2` — is untested.
2. **`LINEAR_SUM 264` is wrong.** Its markers `{11,55,66,77}` sum to **209**, which
   is the value the failure actually reports; the diagnostic names a total no
   execution of the program can produce.
3. **Coverage is one shape.** Scale 2 with `uint32_t` only: no scale 4/8, no `shl`,
   no `add(iv,const)`/`sub(iv,const)` — including the very peel whose refusal costs
   the performance it reports — no affine, no u64, no ILP32, and no *control* class,
   so a future change that starts wrapping `struct[i]` (where linear is correct)
   would pass.

The rewritten test fixes all three: a `volatile` bound that survives folding, totals
derived from the marker macros so they cannot drift, per-case mappings so no two
shapes share a landing site, scales 2 and 4, both displacement peels, and three
controls (`struct[i]`, `(uint64_t)i*2`, a masked index).  Its discrimination, same
compilers:

```
main        -O1  SIGSEGV (139)      -O2  exit 5: FAIL mul2-opaque got 209 want 110
main        -O3  exit 5            PR #777  OK at -O1/-O2/-O3
this tree        OK at -O1/-O2/-O3      gcc -O0..-O3  OK (reference self-check)
```

---

## 3. PR #777's performance claims: two reproduce, one is impossible

| claim | measured here | verdict |
|---|---|---|
| `histogram` Ir +8.04% | **1.08034** (3,262,804 → 3,524,951) | reproduces exactly |
| `glibc_strstr` Ir +4.10% | **1.04097** (25,005,792,794 → 26,030,176,991) | reproduces exactly |
| `k_varint` Ir −8.14% | assembly **byte-identical** at `-O1`/`-O2`/`-O3` (83/63/63 insns both binaries) | **impossible on this base** |

The `k_varint` row contradicts the PR's own static table (63 → 63).  A dynamic
delta requires a code difference; there is none, so the claim belongs to some
other baseline — most likely a tree without the front-end half of the fix, which
its own §5 says is already on `main` in a different shape.  **Lesson recorded: `cmp`
the `.s` before believing any Ir delta.**  Also undisclosed: 21 of 1480 static
comparisons change, not the 3 files the PR names — including
`pic_indexed_store_static_global` 157→165 with stack 8→18, a real ref regression
reported nowhere.

---

## 4. The cost is not necessary: root cause and three fixes

PR #777 frames `histogram`'s +8.03% as the price of correctness.  It is not.  The
trace, in order:

1. `histogram`'s hot loop bound is the **constant** `N = 1<<18`, so the unroller
   runs.  (With a runtime bound it does not, and the loop widens fine — verified:
   `i < n` widens, `i < (1u<<18)` does not.)
2. The unroller spells the latch as a **chain of unit adds**, because every
   intermediate is also an addressing offset the body needs (`p[i+1]`, `p[i+2]`,
   `p[i+3]`): `v60=v19+1; v61=v60+1; v62=v61+1; v63=v62+1`.  No value-numbering
   pass will collapse that chain: re-associating `v61` to `v19+2` leaves `v60` live
   for its own offset use, so the rewrite *adds* an instruction.
3. Every recogniser in `iv_widen` — `latch_step_uses_phi`, `plan_step_operand`, the
   `verify_plan` re-derivation and the apply-phase rewrite — requires the latch to
   name the phi **directly**.  A chained latch is not a candidate, so the counted
   loop is never widened (`[IV-WIDEN]` reports 1 IV for `histogram`, the fill loop).
4. The index therefore stays `U32`, and PR #777's gate — correctly — refuses the
   displacement fold.  Three `leal k(%rsi)` per unrolled body, plus a rematerialised
   `leaq bins(%rip)` from the extra register pressure: +4 instructions per body =
   262,144 Ir, which is the measured +262,147 to within one iteration.

Three fixes, each with its own proof and its own tests:

* **(a) `canonicalise_chained_latch`** — re-spell only the *final* link as
  `phi + total`.  No instruction is added, removed or duplicated; the intermediates
  keep their offset uses.  Value-identical with **no bound argument at all**,
  because modular addition is associative: `(((i+1)+1)+1)+1 == i+4` in the u32 ring
  for every `i`, wrapping or not.  For a signed chain the two spellings differ only
  where an intermediate step overflows, which C17 6.5/5 makes undefined.
* **(b) theorem 5 generalised to a stride** — `mag == 1` needs no bound value (the
  original theorem, unchanged); `mag > 1` needs a *constant* bound compared with an
  *unsigned* predicate, discharging `n <= max - mag + 1` (incrementing) or
  `b >= mag - 1` (decrementing).  `262144 <= 2^32 - 3` ✓.  A runtime bound with a
  stride is refused: `n-1+mag` exceeds `max` for any `n > max-mag+1`, which no local
  argument excludes.
* **(c) the seed range must carry the constant bound** — `prove_counted_bound`
  returned the loose `[0, max-1]`, against which the unrolled body's `i+1`, `i+2`,
  `i+3` members leave the narrow type at the second link and the closure aborts.
  Tightening to `n-1+mag` is valid at *every* point the seed is live (body values
  `<= n-1`, the exiting latch result `<= n-1+mag <= max`, which is exactly (b)'s
  obligation), and it is what makes (a)+(b) actually widen the loop.

Five unit tests pin (b) and (c) and the interaction with (a), including the two
refusals (runtime bound + stride; bound with no room for the stride) and a control
asserting the unit-step theorem was not narrowed.

### 4.1 A pre-existing defect neither PR touches — measured, and held back

With the IV widened the offsets are pointer-width, and the fold *still* did not
fire.  Reason: the SIB cast peel tests `cast_preserves_integer_value`, the
**value-only** predicate, while the tree already carries
`cast_preserves_offset_value` for exactly this consumer (`iv_strength_reduce.rs`
uses it; its own doc quantifies what rejecting the pointer-ring arm costs).  A
pointer-width index lowers as `Shl(Cast(Add(i,1): U64 → I64), k)`, and refusing
that same-width reinterpretation stops the walk one link above the `Add`, so
`p[i+1]` with a `size_t` index — the shape gzip, zlib-ng, expat, SQLite and glibc
string code are written in — never reaches the affine fold:

| shape | main | with the arm enabled |
|---|---|---|
| `w_s8` (`unsigned long i`) | `leaq 1(%rsi),%rsi` + `movq (%rdi,%rsi,8)` = 4 | `movq 8(%rdi,%rsi,8)` = **3** |
| `n_s8` (`unsigned i`) | 4 (**wrong**: folds a wrapping ring) | 4 (**correct**: `addl $1,%esi`) |

Enabling it is sound and locally profitable — `histogram` Ir drops from 1.06025 to
**0.95983**, `libm_round_family` to 0.97620, `linux_rbtree` to 0.98800 — and on
ILP32 it is sound but outright unprofitable (+60 static instructions against −18
on x86-64, because every fold extends the SIB index's live interval and ILP32 has
six general registers), so it was gated to LP64 as a documented cost decision.

**It is nevertheless NOT in this tree, and that is a measured decision too.**
Two independent costs showed up only after the fact:

1. It breaks contract 2 of `tests/regression/check_select_from_compare.sh`.  In
   `ascii_case_fold`'s byte-fold kernel the fold removes three `leaq` and frees
   `%r13/%r14/%r15`; with those registers free the fourth lane loses its `cmovbe`
   fusion and materialises `setbe %r15b` + `movzbl` + `movsbq` + `testl` instead
   (+1 instruction, Ir 1.00740).  `main` passes that gate; the tree with the arm
   fails it.  No knob reproduces the loss (`CCC_NO_IV_WIDEN`, `CCC_NO_GEP_FOLD`,
   `CCC_NO_PF06_ADD_PEEL`, `CCC_NO_FLAG_PEEPHOLES` all leave `setbe=1`), and no
   data directive sits in the fusion window, so PR #725's classifier is excluded
   too — it is a fold/RA interaction, exactly the class that contract exists to
   catch.
2. It *interferes with the widening's own wins elsewhere*.  With the arm enabled
   the 402-comparison matrix moves 98002 → 97984 (−18); with it held back the
   same matrix moves 98002 → **97931 (−71)**, because `i686_alu_chains` (−12 at
   each of −O1/−O2/−O3), `struct_copy` (−6), `pixmap` (−5 twice) and
   `csv_field_sum` (−4 twice, −2) only improve once the arm stops competing for
   registers.

So the arm buys `histogram` 4% and costs the corpus 53 instructions plus a
protected fusion contract.  It is recorded in-tree as **PERF-9** at the peel site,
with all of these numbers, to be landed together with a fusion/RA fix for that
shape — never over a weakened gate.

### 4.2 Result

| | Ir main | Ir this tree | ratio |
|---|---:|---:|---:|
| libm_round_family | 1,720,907,919 | 1,679,947,919 | **0.97620** |
| linux_rbtree | 21,853,215 | 21,591,071 | **0.98800** |
| linux_find_bit | 181,367,833 | 181,366,809 | 0.99999 |
| ascii_case_fold | 2,212,998 | 2,212,998 | 1.00000 |
| histogram | 3,263,371 | 3,459,982 | 1.06025 |
| 25 other DEFAULT_FAST benches | — | identical | 1.00000 |

Fast-corpus geomean **1.00072**.  Static, 402-comparison benchmark+oracle matrix:
**98002 → 97931 instructions (−71)**, stack 25745 → 25745 (+0), x86-64 29 files
better / 7 worse / 1 same-size-different-code, **i686 byte-identical (0 changed)**.
The repo's own screening gate agrees: `census_ab.py main → final`, 190 functions at
`-O2`, **TOTAL −16 instructions, −4 register-register moves, +0 stack refs,
179/190 unchanged, 11 improved, 0 regressed, `gate: … => PASS`**.

Read against PR #777 alone: `histogram` +8.03% → **+6.03%** (a quarter of the cost
recovered), plus two programs it never touched now 2.4% and 1.2% *faster* than the
tree that miscompiles, plus 71 fewer static instructions — with the miscompile
fixed and 468/468 wrap cases agreeing.

### 4.3 The one cost that is irreducible

`glibc_strstr` stays at **1.04097 (+4.10%)**.  Its index is `haystack_buf[h + j]`
with `unsigned h, int j`: C's usual conversions make the offset **U32**, so
`h+j-1` genuinely lives in the narrow ring and the peel is refused for the same
reason `mul2` is.  Proof by construction (`tests` in this session): with
`unsigned long h` the fold survives on this tree (`movzbl -1(%rdi,%rsi)`); with
`unsigned h` it emits `subl $1,%esi`.  Recovering it needs a range analysis that
can prove `h + j >= 1` — filed as **PERF-8** with this price tag, together with
`ascii_case_fold` +0.74%.  This is a refusal with a number attached, not an
argument.

---

## 5. PR #725: right design, one defect, proven by test

The classifier's principle is correct ("unrecognized is not harmless"), its two
tests are correctly paired (a refusal plus a benign control that must still fuse),
and its reach claim holds on a corpus twice the size it measured: **636
comparisons with and without it are byte-identical** (0 smaller, 0 larger,
133,830 → 133,830 insns), so it refuses no real fold.

The defect: its benign allowlist names `.zero`, `.skip`, `.space` and `.fill` as
positions.  All four emit bytes — the first three emit zeros, and `0x00 0x00`
decodes as `add %al,(%rax)`, a *full* flag writer, while `.fill repeat,size,value`
emits an arbitrary pattern, so `.fill 1,1,0xF8` is a bare `clc`.  That is the exact
error the classifier exists to remove, in a gate whose stated rule is fail-closed.

Two tests added, and they **discriminate**: on the PR as shipped,
`fusion_flags_flow_tests` reports 6 passed / **2 failed**
(`refuses_across_a_fill_directive_whose_payload_is_a_flag_writer`,
`refuses_across_a_zero_directive`) — i.e. fusion really does happen across a `clc`
and across a flag-writing `add`.  With the four moved to the data arm: 8/8 pass, and
the 636-comparison A/B is still byte-identical.  In-body reach of all four is **0**
(`directive-census.txt`, 106 sources × 3 opt levels × 2 arches, scoped strictly
between `.cfi_startproc`/`.cfi_endproc`), so the correction costs nothing.  The same
census shows `.long` is the only data directive with non-zero in-body count (400),
and every instance is a jump-table entry in `.rodata` that falls lexically inside
the CFI window — already handled by the PR, and also free.

---

## 6. PR #776: accepted, with the untestable half named

* Applies cleanly onto this tree: 33 files, no conflicts.
* **Builds: 0 errors, 0 warnings** (`fastbuild`, 2m30s).  Its author's stated
  constraint was that no build, compile or runtime test was run; that caveat is
  discharged here for the build half.
* x86-64 codegen, 636 comparisons: **26 files smaller, 0 larger**, 1
  same-size-different-code (`linux_find_bit` −1, `memory` −1 ×3, `k_hashmix` −1 ×3,
  `k30_unroll_shapes` −1 ×2, `vecreg_new_ops` −1, `affine_countdown` −1).
* Correctness with it applied: wrap sweep **78/78** at `-O1`/`-O2`/`-O3`;
  regression suite **PASS=846 FAIL=0 SKIP=8, AB-diff failures 0**.
* **Not executable here:** the AArch64 encoder (+748), the ARM ELF writer (+481),
  `fp_scalar`/`neon`, the dispatcher goldens (431 rows) and the RISC-V writer.
  Those ride on the clean build and their own golden/doccheck scripts.  Anyone with
  an aarch64 runner should execute `scripts/aarch64_execute_suite.py` before
  trusting them; this document does not claim otherwise.

---

## 7. Rejected, with evidence

**PR #777's `loop_invert` memory licence (+346 pass lines, +197 census script,
pipeline env threading).**  Its own summary says the switch is "off by default,
pending a reach measurement", and its §6 already concedes the win it measured is
concentrated in programs the licence does not change at all.  The reach
measurement, `CCC_LOOP_INVERT_MEMORY` 0 → 1 over 636 comparisons: **26 changed, 0
smaller, 26 larger, +124 instructions, +5 stack refs** (`strlen_bench` +21,
`strings` +18, `k04_strlen`, `k13_strcmp`, `k_strcmp_signed`, `k_strlen_scan`,
`linux_rbtree`).  Dynamically, on the only affected program in the Callgrind set:
`linux_rbtree` Ir **21,853,219 → 21,853,219, ratio 1.00000**.  An off-by-default
switch with a measured static cost, no measured dynamic effect, and a win its
author could not attribute to it is 543 lines of debt; it stays out.  The census
script goes with it, since a reach-measurement tool for an unshipped transform is
also debt.

**PR #777's `engineering/FOLLOWUP-*.md` and its evidence JSON.**  They carry the
`k_varint −8.14%` claim, which §3 shows is impossible on this base, and the
`-O1`-only framing, which §2.1 refutes.  Shipping a document whose central numbers
are contradicted by its own tree is worse than shipping none; this document and
`engineering/evidence/2026-10-08-pr-audit/SUMMARY.txt` replace them.  Its
`scripts/godbolt.py --execute` addition **is** kept: it is additive, default-off,
and it is the only way to compare *semantics* on a vendor rather than instruction
shape.

---

## 8. Backlog, each with a price tag

* **PERF-8 (new, highest value).** A range/interval analysis for narrow indices
  (`h + j >= 1`, `i & 0xFF`, `i < n` with runtime `n`) would let the SIB peel fire
  on provably-non-wrapping narrow indices instead of refusing the whole class.
  Price of not having it, measured: `glibc_strstr` **+4.10% Ir**,
  `ascii_case_fold` **+0.74% Ir**.  This is the one remaining correctness/performance
  trade-off in the tree, and it is the only item that removes it.
* **PERF-7 (new).** The pointer-ring cast arm on ILP32 is sound but unprofitable
  under six registers (+60 static insns vs −18 on x86-64).  Needs an RA-side cost
  model weighing a fold against remaining pressure, not a blanket `ptr == 8` gate.
* **PERF-9 (new, measured, ready to land with a companion fix).** The
  pointer-ring arm of the cast peel (§4.1).  Worth `histogram` Ir 0.95983 and
  `w_s8`-style `size_t` indexing one instruction per access; blocked by the
  `ascii_case_fold` fusion loss (Ir 1.00740, breaks
  `check_select_from_compare.sh` contract 2) and by 53 instructions of
  interference with the widening elsewhere.  Both numbers are in the peel's own
  comment so the next reader does not have to rediscover them.
* **PERF-5 (confirmed costing real Ir).** Redundant extension/copy elimination:
  `histogram` still emits `movzbl (%rdi,%rsi),%eax; movl %eax,%r11d` where the load
  could target `%r11d` directly — 1 instruction per 4 elements, ≈2% of that
  program's Ir.  Same root cause as the `classify` 2.057× and `namechars` 1.653×
  residues.
* **PERF-6.** IVSR collector gap for `(I64)(i << 2)`: 0 fires, while `(I64)i * 4`
  fires 1.  0/402 corpus shapes affected, so it stays low priority.
* **PERF-4.** Descending loops (`revvarint` `-O2` per iteration: LCCC 11
  instructions / 2 branches vs GCC 7 / 1).
* **IVW-RANGE-1 (new, soundness, pre-existing).** `prove_counted_bound`'s unit-step
  runtime-bound arm returns `hi = max - 1`, but the value that fails the exit test
  can be `max` (seed `max-1`, `n = max`).  Any member derived from the seed *after*
  the loop would be range-checked against an interval one short of the truth.  This
  tree does not widen that hole — the constant-bound arms it adds are exact at every
  point the seed is live — but the runtime arm should be tightened to `max`, or the
  range made point-sensitive (body vs exit), before it is relied on further.  No
  reproducer is known; it is filed because it was found by reading, and reading is
  how the other four got in.

## 9. Reproducing any number here

```sh
# the wrap class, against any compiler (reference self-check first)
python3 scripts/wrap_ring_sweep.py gcc
python3 scripts/wrap_ring_sweep.py /path/to/lccc --opts=-O1,-O2,-O3 --opaque

# the shipped regression test, which fails on unfixed main at -O2
lccc -O2 tests/regression/ivsr_scale_ring_wrap.c -o /tmp/srw && /tmp/srw

# dynamic Ir, both arms compiled in one process with pinned geometry
python3 scripts/callgrind_ab.py /path/to/mine /path/to/ref "-O2"            # fast set
python3 scripts/callgrind_ab.py /path/to/mine /path/to/ref "-O2" --heavy glibc_strstr

# the repo's own static screening gate (whole-corpus, gates on HOT stkref then insns)
scripts/census_ab.py /path/to/old-lccc /path/to/new-lccc --opt=-O2
```

`census_ab.py` on this tree, `main` → final, 82 files / 190 functions at `-O2`:
**TOTAL −16 instructions, −4 register-register moves, 0 stack refs added, 179/190
functions unchanged, 11 improved and 0 regressed**; its own verdict line reads
`gate: HOT stkref 171->171, insns 5597->5581 => PASS`
(`engineering/evidence/2026-10-08-pr-audit/census-ab-main-vs-final.txt`).  The
wider 402- and 636-comparison matrices quoted in §4 and §5 add `-O1`/`-O3` and
i686 to that, and agree in direction (−18 instructions, i686 unchanged).

The directive census that prices §5 is reproducible with the script recorded in
the header of `engineering/evidence/2026-10-08-pr-audit/directive-census.txt`.

Gates run on this tree: regression suite **846/0/8**; `cargo fmt --check` clean;
`cargo clippy --all-targets` **0 errors 0 warnings**; `census_ab.py` gate **PASS**
(−16 insns, −4 rrmov, +0 stkref); `iv_widen` libtests **40/40**;
`fusion_flags_flow_tests` **8/8**; `check_select_from_compare.sh` **PASS**; wrap
sweep **468/468**; `ci_local.sh --fast` **GREEN** (log: `ci-fast-final.log` in the
session ledger).
