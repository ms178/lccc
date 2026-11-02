# Red-team audit of my own rebased branch — 2026-09-30

**Base:** `71e3b1ae` (main, PR #686 merged) · **head:** `7e5a81be`
**Deliverable:** `/home/user/ms178-1.patch` — 94 files, +4457/−546, applies clean.

This is an audit of *my* work, written to be argued with. Every number came from
a tool call this session. Where I was wrong, the wrong claim is stated next to
the correction, because the corrections are the useful part.

---

## 1. Did the rebase do the right thing?

#686 landing upstream made most of my branch redundant. Measured, not assumed:
of the 43 files both branches touched, **21 were already byte-identical** — the
scoped x86-64 late twin, the terminator-complete `find_loop_preheader`, the
relaxed marching-pointer assert. So the correct rebase is a *small* one, and the
result is 94 files rather than the 113 my pre-merge patch had.

The discipline that mattered: for each of the 19 true conflicts I asked "which
side is better", not "which side is mine". **Six times the answer was upstream.**

| conflict | taken | why |
|---|---|---|
| entry-block guard in `loop_preheader.rs` | **main** | main refuses (`if header_idx == 0 { continue }`); I had a `debug_assert!`. An assert panics in debug and is **compiled out of release** — inverted for a guard whose purpose is containing a catastrophic silent outcome. My own comment called the outcome "catastrophic" and then chose the instrument that does nothing in the shipped binary. |
| `late_vectorize_entry` + AArch64 measurement | **main** | I did not have it at all. Taking my `mod.rs` would have deleted a real measurement (95→73 insns on the affected code). |
| the "dead code on x86" **correction** | **main** | I claimed the `VecMaxI32x8`/`vpmaxsd` machinery was dead code on x86. **It was not** — the twin reran the vectorizer with the same flags. Upstream retracted my sentence in three places and was right to. |
| `_compile_fail_detail` | **main** | a helper refactor; mine inlined the same string. |
| volatile debug message | **main** | carries `block_idx`; mine dropped it. |
| `LateMinMaxOnlyScope` doc block | **main** | mine duplicated a paragraph of its own doc comment. |

Kept from mine, because upstream lacks it: `hoist_dynamic_limit` (the F3 dedup —
the marker string still occurs **2×** upstream, **1×** here), the header-only
`preheader_would_unlock_a_hoist` scan with a `debug_assert` that re-proves the
equivalence, the removed crate-wide `unused_variables` allow, four gates, the
cache-provenance work, and the empty-oracle-set fail-closed arm.

**Verification that the rebase changed no behaviour:** 51 benchmark programs × 2
flag settings = **102/102 byte-identical** to new main.

### Where I disagree with upstream, and why

**The `unused_variables` allow.** Main still carries
`#![allow(dead_code, unused_variables, ...)]` in `src/lib.rs`. That is the
mechanism that let the original volatile miscompile ship: deleting LICM's
`if *volatile` arm left a destructured binding unread, rustc *would* have named
the file and line, and the crate-level allow silenced it. Removing it is one
line and buys a build-level guard over every destructured safety flag in the
crate. Upstream has not taken it; I think that is the single cheapest piece of
hardening still on the table, and my patch keeps it removed.

**The assert-vs-refusal point cuts both ways.** Upstream was right about the
entry block, and I have generalised the lesson rather than just accepting the
hunk: an assertion documents an invariant you believe cannot be violated; a
refusal enforces one you are not willing to bet the function on. My remaining
`debug_assert`s were re-read against that test. The one in
`preheader_would_unlock_a_hoist` stays an assert *correctly* — it re-proves an
equivalence whose violation would mean a slower compile, not a wrong one.

---

## 2. The audit of my own measurements — three retractions

This is the section I would want a reviewer to hold me to.

**(a) "Spills are the dominant defect."** Wrong. The rank table's spill column
looked decisive (`i686_alu_chains` 75, `sha256_transform` 61, `struct_copy` 60).
Measured over the 77 functions behind:

```
corr(gap, spills)       = +0.453
corr(gap, spills/insns) = +0.305
corr(gap, loads+stores) = +0.739
median spill fraction   = 0.0 %
>=20% spill refs: 5 of 77 functions, holding 242/1899 = 13% of the gap
```

`nbody` — the worst real gap — has **zero** stack references. Spills are a
minority effect; total memory traffic is the correlated quantity.

**(b) "A `leaq K(%r),%t; movq %t,%r` → `addq $K,%r` peephole is a systemic
win."** Wrong. I found it in adler32's `len_16` loop and generalised from one
example. Corpus-wide it occurs **12 times in 6869 instructions**. Real, worth
fixing, not where the 15.8 % lives.

**(c) "We beat GCC by 12.5 % on the corpus."** Misleading, and I nearly shipped
it as a headline. It is a *total*, and four recursion benchmarks where GCC
explodes carry the whole aggregate:

| set | n | LCCC | GCC | delta |
|---|---:|---:|---:|---:|
| all 51 | 51 | 6869 | 7851 | −12.5 % |
| the 4 recursion files | 4 | 172 | 2068 | −91.7 % |
| **the other 47** | 47 | **6697** | **5783** | **+15.8 %** |

The honest statistic is the **median per-file ratio, 1.17**, larger on 37/47.
The same error in the opposite direction is what the oracle's "204" was.

---

## 3. The finding that reframes the work queue

`codegen_oracle.py --rank` — the tool the standing work queue was ordered by —
**ranks inlining decisions as codegen gaps.**

ICC's `zlib_ng_adler32::main` is 73 instructions because ICC left
`zlib_ng_adler32_c` out of line as two specialised copies (`..0`, `..1` — 4
calls) that the single-function view never measured. We inline, so 277 is the
whole computation. Whole translation unit over the six files that table put on
top:

| compiler | insns | calls |
|---|---:|---:|
| gcc16.2 | 1169 | — |
| **lccc** | **1613** | — |
| icc | 2303 | 4 in `main` |
| clang | 2716 | — |
| icx | 4104 | — |

**ICC is 43 % larger than us, not 204 instructions ahead.** We are a clear
second behind GCC. 16 of the 77 "behind" rows were in this state and carried
**781 of 1899 = 41 %** of the headline, including both top rows.

Landed rather than just reported: a `calls` column, a per-row comparability
marker, a stdout warning naming the unrankable rows, and the same in the
markdown report. `jmp`/`b`/`j` are deliberately **not** calls — a tail jump to an
out-of-line copy is precisely the shape that fakes a smaller function. Guarded
by 11 known-answer tests, mutation-verified both directions, registered in both
CI drivers (parity PASS, 105 commands).

---

## 4. The real defect, scoped and quantified

`nbody`'s inner loop — 5 bodies × 5 000 000 iterations, the hottest loop in the
corpus — is **110 instructions against GCC's 14 (7.9×), with zero spills**:

| per iteration | ours | GCC |
|---|---:|---|
| `imulq $56` — index×stride by multiply | 2 | `addq $56, %rax` once |
| `leaq bodies(%rip)` — static base re-materialised | 2 | hoisted to `%r12` |
| `cmpl $5000000, -72(%rbp)` — outer bound from stack | 3 | register |
| data movement (`movsd`/`movupd`/`movq`) | 56 of 110 | displacement addressing |

Corpus-wide the same three signatures: `movsd` **171 vs 54 (3.2×)**,
`leaq sym(%rip)` **145 vs 91**, `imul $const` **87 vs 55**. Stack refs 655 vs
773 — we are *better* there, which is why the allocator is the wrong place to
start. Recorded as `IVOPTS-1` with a work order whose steps are each
independently measurable.

---

## 5. What I did *not* do, stated plainly

- **No IVopts implementation.** Diagnosing it and shipping a half-verified
  transform in the same session would be exactly the shortcut this work forbids.
  It is scoped, quantified, has a reproduction, and is ordered cheapest-first.
- **No runtime claims from static counts.** Every number here is a screening
  metric. Nothing in this document is PMU evidence; this box has no PMU, and the
  runtime harnesses (`bench_kernels.py`, `perf_ab.py`) were not re-run for
  adler32/nbody this session.
- **The full CI was re-run once** after I mutated the environment mid-run by
  installing multilib — which caused three spurious `-m32` failures that I
  then reproduced as passing on the stable tree (`reassoc-latency` 12/12 PASS).
  That was my error and the re-run is the correction.

## 6. Verification actually run

| check | result |
|---|---|
| build, `fastbuild` preset | 0 errors, **0 warnings** with `unused_variables` enabled |
| `cargo fmt --check` | clean |
| codegen vs new main | **102/102 byte-identical** (51 programs × 2 flag sets) |
| `test_codegen_oracle.py` | 11/11; both mutations caught |
| `check_nocfi_peephole_parity.sh` | PASS, x86-64 **and** i686 (probe fix) |
| `check_reassoc_latency.sh` | 12/12 PASS including both `-m32` checks |
| `check_ci_gate_parity.py` | PASS, 105 commands |
| `check_doc_links.py` | OK, all references resolve |
| patch vs `71e3b1ae` | applies clean, 94 files, +4457/−546 |
