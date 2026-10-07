# Follow-up: performance round — IVSR-PTRADD-1 and five measured findings

**Date:** 2026-10-07
**Upstream base:** `66c2452838eedef2322fbad44d242e3b990e6c10` (`ms178/lccc` `main`,
merge of PR #771).
**Host:** Debian 13 VM, Xeon, GCC 14.2, binutils 2.44, Rust 1.99.0,
valgrind 3.24.0, 8 GiB swap active throughout.
**Build policy:** `scripts/build_lccc_fast.sh` — fastbuild, Rust `-O1`, `-j2`.
**Companion documents:**
[`FOLLOWUP-2026-10-06-ivsr-domain-audit.md`](FOLLOWUP-2026-10-06-ivsr-domain-audit.md)
(the correctness round this one builds on) and
[`FOLLOWUP-2026-10-06-codegen-audit.md`](FOLLOWUP-2026-10-06-codegen-audit.md)
(the first round).

**Target hardware note:** the i7-14700KF (Raptor Lake) is **not** this host and was
**not** measured. PMU counters are unavailable in this VM (`perf` exits 0, the
counters are unsupported). Every number below is a Callgrind simulated instruction
count, a static assembly comparison, or a VM wall-clock sample reported with its
noise floor. **No cycle, uop or speedup claim is made anywhere.**

---

## 1. Summary

One performance change landed. Two were implemented, measured, and **reverted**.
Five findings were recorded with reproduction commands, three of which exist
specifically to stop a future engineer from repeating a mistake made here.

| | |
|---|---|
| **Landed** | IVSR-PTRADD-1 — pointer recurrence for the address-forming `Add` |
| Best single result | `k_varint` **−9.59% Callgrind Ir** and **−14.6% wall** (both metrics agree) |
| Suite result | 11 kernels, **−0.49% Ir** (−350,287), 11/11 checksums agree across 3 compilers |
| Correctness validation | **845 PASS / 0 FAIL / 0 AB-diff failures**, GCC-differential, executed |
| Static blast radius | 402 + 841 + 237 + 158 comparisons measured (§4) |
| **Rejected on measurement** | SELECT-CHAIN-FOLD (+2.63% Ir), RANGEFOLD-OR-2 (inert, 0/158) |
| **Recorded findings** | METRIC-TRAP-1, SETMEM-SHAPE-1, METRIC-SPLIT-1, RANGEFOLD-OR-1/2, CONSTLOOP-FOLD-1 |

---

## 2. The measurement problem came first, and it changed the method

`scripts/bench_kernels.py --reps 7 --cpu 0` on the rebased tree gave the starting
picture against system GCC 14.2 (`-O3`, best-of-7, arms interleaved, checksum-gated):
geomean **0.900×**, worst kernels `varint` 0.60×, `strcmp_signed` 0.78×,
`classify` 0.79×, `namechars` 0.88×.

The same run then reported that **7 of 11 kernels moved beyond the 3% tolerance
with byte-identical codegen** — a ~15% layout-noise floor on this host. That is the
most important methodological fact of the session: wall clock here cannot resolve
an effect smaller than ~15%, so it was demoted from decision metric to colour and
Callgrind `Ir` was used to *size* every change instead.

§5 then shows the limit of that choice: on `nbody` the two metrics disagreed by 74
points and `Ir` was the wrong one. The acceptance rule this session converged on is
in `METRIC-SPLIT-1` — **a codegen change must pass both metrics to be promoted.**
`Ir` detects and sizes a shape change and A/Bs inside the wall-clock noise floor;
only wall clock can *accept* a promotion. IVSR-PTRADD-1 is the change that passes
both, which is why it is the one that landed.

---

## 3. IVSR-PTRADD-1 — the pointer scan could not see LCCC's own addressing form

`try_lower_pointer_arithmetic` (`src/ir/lowering/expr_ops.rs`) lowers **every** C
subscript to `Add(ptr, scale_index(i, elem_size))` in pointer-width integer
arithmetic — `scale_index` is the identity for `elem_size == 1` and a
`Mul(i, elem_size)` otherwise. It does not emit `GetElementPtr`. IVSR's
`find_derived_exprs` collected only `GetElementPtr` offsets, so the pointer
recurrence never fired on the most common addressing idiom in C: byte-buffer walks
got no recurrence at all, and wider-element arrays found the `Mul` but no GEP
consuming it, so the whole group was dropped (`has_uses` alone only arms the
opt-in scalar flavor, off by default).

Confirmed against the real IR of `k_varint.c`, not inferred:

```
Cast  v12 = (I64)v27                    # sext(i)
Add   v13 = v28 + v12   ty I64          # &v[i]  <- address formed by Add, not GEP
Load  v36 = *v13
```

`bench_run` at `-O3` rebuilt the address from scratch every iteration, including
the loop-invariant global:

```
.LBB6:  movslq %esi, %rdx      ->      leaq v(%rip), %rdi   # hoisted to preheader
        leaq   v(%rip), %r8            movzbl (%rdi,%rdx), %r9d
        addq   %rdx, %r8               movzbl 1(%rdi,%rdx), %r8d
        movzbl (%r8), %r9d             movzbl 2(%rdi,%rdx), %r8d
```

### Result — Callgrind `Ir` for `bench_run`

Pinned geometry I1/D1 `32768,8,64`, LL `33554432,16,64`, identical for every arm.

| kernel | before | after | delta | GCC | after/GCC |
|---|---|---|---|---|---|
| **varint** | 3,578,595 | **3,235,371** | **−9.59%** | 2,533,635 | 1.412 → **1.277** |
| matchlen | 10,340,483 | 10,333,427 | −0.07% | 12,019,475 | 0.860 |
| strcmp_signed | 11,782,455 | 11,782,448 | −0.00% | 11,762,435 | 1.002 |
| other 8 kernels | — | — | **+0.00%** (identical codegen) | — | — |
| **suite total** | 71,225,717 | **70,875,430** | **−0.49%** | 59,498,729 | 1.197 → **1.191** |

Wall clock, same kernels, independently: `varint` 66.536 → 56.819 ms (**−14.6%**).
Both metrics agree in sign, which is the condition §5 shows is not optional.

**Correctness: all 11 kernels, pre/post/GCC checksums identical.** The driver's
checksum is derived from the whole computation, so a different answer is a
failure, not a win.

### Guards

Each is a precondition of the argument, not a heuristic:

- the `Add` must be in the **pointer ring** — a narrower `Add` is ordinary integer
  arithmetic;
- its result must be **used as an address** (`Load.ptr`, `Store.ptr`,
  `GetElementPtr.base`, `Memcpy`, or an intrinsic pointer argument, delegated to
  `IntrinsicOp::reads_pointer_arg`/`writes_memory_via_args` so a new vector memory
  opcode is picked up automatically rather than silently missing);
- exactly one side may be IV-derived and the other must be loop-invariant.

An integer accumulation `invariant + iv` that nobody dereferences is left exactly
alone — that is the scalar-derived net loss `CCC_IVSR_SCALAR_DERIVED` documents,
and this must not become it.

**ILP32 is excluded on measurement, not taste.** Without the gate,
`simd_crc_adler.c` `-O2 -m32` grew **142 → 191 stack refs (+49)** for +2
instructions, while the same source on x86-64 was +2 insns / −1 stack. The
recurrence adds a loop-carried pointer web and a 6-GPR file parks it in a slot at
the latch. With the gate **every `-m32` difference disappeared** (18 changed files
→ 6). ILP32 correctness is still covered: `ivsr_address_add.c` runs and passes on
`-m32` at `-O0..-O3`.

### The knob respects the env-read ratchet

`check_env_test_hygiene` ratchets environment reads in `src/passes` (`mod.rs`
excepted) at **155**. The first version spent two and turned the fast suite red at
157. Raising the budget would have been the lazy fix and is exactly what the
ratchet exists to prevent; the gate's own exemption of `mod.rs` states the
sanctioned pattern, so `CCC_NO_IVSR_PTR_ADD` is read **once** in the pipeline and
threaded as a parameter — `run_ivsr_ptr_add` → `ivsr_with_analysis` →
`reduce_loop` → `find_derived_exprs` — exactly how `CCC_IVSR_SCALAR_DERIVED`
already reaches IVSR. Reads are back to 155. The threading is provably
codegen-neutral: **237/237 assembly byte-comparisons identical** to the build the
Callgrind A/B was measured on.

---

## 4. Static blast radius, measured rather than assumed

| corpus | comparisons | changed | net |
|---|---|---|---|
| benchmark + oracle (67 sources — 55 `tests/benchmark/programs` + 12 `tests/oracle/programs` — × 6 configs) | 402 | **0** | 87386 → 87386 insns |
| regression, x86-64 `-O2` | 841 | 6 | **+4** insns, **−3** stack refs |
| knob-threading refactor (benchmark+oracle+bench) | 237 | **0** | identical |
| reverted case-fold experiment | 158 | **0** | identical |

The six changed regression files: `simd_vecreg` −2 insns/−2 stack,
`accumulator_pointer_load` −1, `memcpy_unaligned_load_fwd` +1,
`temp_promotion_window` +1, `simd_crc_adler` +2 insns/−1 stack,
`simd_avx2_defer_chain` +3.

The trade is stated plainly rather than buried: **+4 static instructions in four
synthetic x86-64 regression files, −350,287 executed instructions on the kernel
suite.**

### Independent execution validation

Because this changes generated code and the slow half may not run here,
`scripts/run_regression_suite.sh` was run as well — a strictly stronger test than
assembly comparison, since it **compiles and runs** every case:

```
regression suite: PASS=845 FAIL=0 SKIP=8 (AB-diff failures: 0)
```

For each case that runner compiles and runs it with the candidate build; compiles
and runs it with **GCC** and requires identical stdout *and* exit status; then
re-runs with `CCC_NO_SMALL_SLOTS=1` and requires identical output again. 27 cases
carry `-m32` in their `.flags` sidecar and `gcc-multilib` is installed, so the
ILP32 path — where IVSR-PTRADD-1 is deliberately gated **off** — is executed and
correct rather than merely uncompiled.

---

## 5. METRIC-SPLIT-1 — `Ir` said −11.1%, wall clock said +63.2%, and `Ir` was wrong

`CCC_FP_EXTRACT_HOMES=1` on `tests/benchmark/programs/nbody.c`,
`-O2 -march=x86-64-v3`, same compiler binary, output byte-identical between arms:

| metric | homes OFF | homes ON | ON/OFF |
|---|---|---|---|
| Callgrind `Ir` (`scripts/callgrind_ab.py`, correctness-gated) | 2,700,122,542 | **2,400,122,541** | **0.8889 (−11.1%)** |
| I1miss / D1miss / LLmiss / Bcm / Bim | 1378 / 1618 / 2739 / 2968 / 180 | 1380 / 1616 / 2740 / 2967 / 179 | **flat** |
| wall clock, median of 15 interleaved CPU-pinned reps | 224.51 ms | **366.42 ms** | **1.6321 (+63.2%)** |
| wall clock, min of 15 | 211.98 ms | 331.94 ms | 1.5659 (+56.6%) |
| vs GCC 14.2 (median) | 1.076× | **1.756×** | — |

**11.1% fewer executed instructions and 63% slower**, with every simulated cache
and branch metric flat. Far outside the ~15% noise floor, reproducing on both
median and min.

`Ir` counts instructions retired, not stalls. It is structurally blind to a
store-forwarding or dependency stall introduced by keeping FP lanes in XMM
registers, and pinned cache geometry does not help — the geometry models misses,
and the misses did not move.

**Consequence:** FP-LANE-1 stays opt-in, and the earlier case for that was
*understated*. The first round declined promotion on wall-clock evidence that
looked layout-dependent (+52.9% without `-lm`, −20.5% with it, −10.6% on matched
data addresses) and was therefore arguable. This is not arguable. Do not
re-propose enabling it on the strength of an instruction count, a `.text` size, or
a static matrix. The mechanism remains **unproven** — the page-split-store
hypothesis is plausible and this VM has no PMU; what is proven is the size and
reproducibility of the effect.

The same paired test is why the other two decisions this session stand:
IVSR-PTRADD-1 improves `Ir` **and** wall clock; SELECT-CHAIN-FOLD worsened `Ir`
with no wall-clock win.

---

## 6. Two changes implemented, measured, and reverted

### SELECT-CHAIN-FOLD

`k_classify` is 2.057× GCC and `k_namechars` 1.653× — the two worst ratios in the
suite. C's `a || b || c || d` if-converts to
`Select(c,1,Select(c2,1,Select(c3,1,Y)))`, and `c ? K : (c2 ? K : Y)` collapses to
`(c|c2) ? K : Y` by case analysis alone. Implemented with the OR formed in the
*narrower* carrier after peeling integer casts off known-boolean conditions
(widening measured worse — it sign-extended each byte term to 64 bits before an
`orq`).

**Result: `bench_run` 49 → 48 instructions, but Callgrind `Ir` 5,962,067 →
6,118,811 (+2.63%).** Fewer static instructions, *more* executed ones: the folded
form makes every term unconditional where the chain previously short-circuited
through the branch. **Reverted byte-identically.**

### RANGEFOLD-OR-2

`range_fold` folds each conjunct of `isalpha`-style classifiers into the
unsigned-bias form but cannot merge the two results, because `extract_range`
requires both bounds on ONE value and after folding they are `x-97` and `x-65`.
`try_case_fold_or` was implemented to apply `set_membership`'s already-proven
case-fold pair merge to that spelling: `Or(Ule(Sub(x,a2),s), Ule(Sub(x,a1),s))` →
`Ule(Sub(And(x,~32),a1),s)`, 5 instructions → 3, under `set_membership`'s exact
preconditions (`a2==a1+32`, `b2==b1+32`, `(a1&32)==0`, `a1>=0`, `a1>>5==b1>>5`).
Four unit tests, including one violating each precondition separately; 4152 lib
tests passed. Correct in isolation.

**End to end it did nothing at all: 158 comparisons, 0 changed.** Provably inert —
it never fires anywhere in this corpus, including on the two kernels it was written
for. The blocking shape exists in the *final* IR but is produced **after**
`range_fold`'s last invocation, so activating it needs a pipeline change, not a
pattern change — and a pipeline change cannot be validated on a VM forbidden from
running the benchmark-output gate. **Reverted**, verified by codegen differential
(0/158 against the gated pre-fold build), not by `cmp`: separate builds embed
distinct build metadata, so binary identity is the wrong test and initially
reported a spurious difference.

**A measurement error, and how it was caught.** The first differential ran against
a binary saved *before* the ILP32 gate was added and reported "2 of 158 changed,
one better (`k_matchlen -O2 -m32`) one worse (`k_varint -O2 -m32`)", read as the
fold firing incidentally with mixed sign. It was not: both rows are `-m32`, and
re-running between the *ungated* and *gated* IVSR builds reproduces exactly those
two and no others. The 2 diffs were the ILP32 gate doing its job; the fold
contributed zero. An A/B is only as good as the provenance of its two arms, and
"which build is the baseline" has to be checked, not assumed — the lesson
`scripts/differential_corpus.sh`'s own header states.

---

## 7. Cross-metric triage: which gaps are real, and one metric that lies

`lccc/gcc`, so **>1 means LCCC is worse** on that metric:

| kernel | Ir ratio | wall ratio | verdict |
|---|---|---|---|
| **classify** | **2.057** | **1.208** | **GENUINE GAP** (both metrics) |
| **namechars** | **1.653** | **1.248** | **GENUINE GAP** (both metrics) |
| **varint** | **1.277** | **1.577** | **GENUINE GAP** — was 1.412 / 1.668 before IVSR-PTRADD-1 |
| strcmp_signed | 1.002 | 1.261 | wall-only: identical instruction count, 26% slower → block layout / branch prediction |
| strlen_scan | 1.333 | 0.984 | **Ir lies here** — see below |
| adler32_do8 | 1.112 | 0.963 | Ir-only: 11% more instructions, 4% faster |
| hashmix | 1.019 | 1.020 | parity |
| adler32 | 1.000 | 0.978 | parity or better |
| memchr | 1.000 | 0.985 | parity or better |
| matchlen | 0.860 | 0.985 | LCCC emits 14% fewer instructions |
| map64_sub | 0.630 | 0.938 | LCCC emits 37% fewer instructions |

**`strlen_scan` would have sent a future engineer after a non-defect.** Its 1.333×
`Ir` ratio (and the suite's largest absolute gap, +7.35M) is fully explained by
exactly one instruction: LCCC's inner loop is `cmpb $0,(%r8); je; addq $1,%r8; jmp`
(4) against GCC's rotated `addq $1,%rax; cmpb $0,(%rax); jne` (3), and 4/3 = 1.333.
LCCC keeps the **top-tested** form and hoists `leaq buf(%rip), %rdi` out of the
32-iteration outer loop; GCC re-materialises it inside. On wall clock LCCC is
*faster* (0.984). The kernel header already records 49.3 ms top-tested vs 59.2 ms
rotated on this host — LCCC ~15.9% faster — and states that any transform which
rotates it "has to beat that number, not merely be defensible." Chasing the `Ir`
ratio there would have destroyed a real win.

`strcmp_signed` is the mirror-image trap: **1.002× Ir but 1.261× wall.** An
identical instruction count running 26% slower is block layout and branch
prediction, and no instruction-count work will move it.

---

## 8. Four-vendor execution oracle, re-run on the delivered build

`tools/oracle/godbolt_oracle.py --oracle-set all-vendors`, `-O2`, all binaries
assembled and linked **locally** so every vendor shares one machine, one libc and
one linker. Semantics first, size second.

```
program                       lccc      cg162 cclang2310 cicc202110 cicxlatest   ratio  status
bitops                         122        128        130        189        213   1.352  PASS
control                        168        116        211        153        148   0.935  PASS
float                          155        123        154        329        162   1.239  PASS
int_alu                         92         68         23         81         28   0.543  PASS
ivsr_index_domains             163        118        343        338        370   1.793  PASS
loops                          128        229        282        268        229   1.969  PASS
memory                         242        579        296        622        449   2.010  PASS
pixmap                         256        283        467       1402        524   2.613  PASS
strings                        184        208        363        294        230   1.488  PASS

  total vs cg162        lccc=1510 oracle=1852 ratio=0.8153
  total vs cclang2310   lccc=1510 oracle=2269 ratio=0.6655
  total vs cicc2021100  lccc=1510 oracle=3676 ratio=0.4108
  total vs cicxlatest   lccc=1510 oracle=2353 ratio=0.6417

== 9 agree, 0 diverge, 0 error (9 programs) ==
```

Oracle identities as reported by Compiler Explorer: `cg162` = x86-64 gcc 16.2,
`cclang2310` = x86-64 clang 23.1.0, `cicc2021100` = x86-64 icc 2021.10.0,
`cicxlatest` = x86-64 icx (latest).

This is a **re-run on the exact delivered build**, not a carried-over result: the
base had moved to PR #771 (which touches `src/passes/loop_invert.rs`) and
IVSR-PTRADD-1 is a codegen change. Every row is identical to the earlier run, which
is evidence rather than luck — the 402-comparison differential in §4 already showed
IVSR-PTRADD-1 changes **zero** assembly across `tests/oracle/programs`.

**LCCC is the smallest of the five in aggregate against every vendor.** Instruction
count is data, not a specification: a compiler that wins this table by
miscompiling has lost the table that matters, and the semantics column — 9/9 agree
across four independent vendors, including `ivsr_index_domains` whose base-tree
build segfaults — is why this one is trustworthy.

---

## 9. What remains open, with the reason it was not attempted

All four remaining genuine gaps are in **register allocation, copy coalescing or
block layout**, or need a **pipeline reordering** — not in the IR-level shape
recognition this session could reach. Each is recorded in
[`backlog.md`](../backlog.md) with a reproducer:

| item | gap | why not attempted here |
|---|---|---|
| SETMEM-SHAPE-1 | `classify` 2.057×, `namechars` 1.653× | `set_membership` cannot see either shape the kernels produce: `classify`'s `Select` chain is built by the **frontend** (verified: 3 Selects under all six `CCC_DISABLE_PASSES` configurations including `ifconv`), so the required block chain never exists; `namechars` has the chain but a counting-form join Phi (`c` vs `c+1`) instead of a 0/1 predicate Phi. Needs a new member kind and a new join form. |
| RANGEFOLD-OR-1/2 | same two kernels | Transform implemented and unit-proven, but **inert end to end** (0/158): the `Or` appears after `range_fold`'s last run. Needs a pipeline change, whose only validator is the benchmark-output gate. |
| backend boolean materialisation | `classify`, `namechars` | Each `setcc` is followed by a redundant `movzbl` **and** a `movsbq`/`movsbl` of the same byte, and the surviving `Select` costs `movl $1,%ecx; movq %rN,%rM; cmovneq`. On x86-64 a 32-bit write already zero-extends, so the second extension is worth zero instructions if the allocator coalesces it. RA/peephole work. |
| CONSTLOOP-FOLD-1 | `int_alu`: LCCC 92, Clang **23**, ICX 28, GCC 68 | Clang folds the entire 64-iteration FNV-1a chain to one `movabsq`; LCCC's complete unroller caps trip at 16 with a 512-instruction budget. The self-limiting design (speculatively unroll, fold, **revert unless smaller**) is specified in the backlog; raising the cap blindly is code bloat that only the benchmark gate would catch. |
| `strcmp_signed` | 1.002× Ir but 1.261× wall | Block layout and branch prediction, not codegen size. |

Also carried forward unchanged: IVOPTS-1's **vector** half (SLP
`VecLoadF64x2`/`VecStoreF64x2` carry base and byte offset as separate operands, so
there is still no single address value for a recurrence to attach to — though
`is_used_as_address` now makes those shapes *visible* to the scan), RA-CSAVE-1,
PF-SCHEDULER-1, PF-CHACHA-1.

---

## 10. Validation

| gate | result |
|---|---|
| `scripts/ci_local.sh --fast` | **167 passed, 0 failed, 5 skipped**, `ALL GATES GREEN` |
| `rustfmt` gate | PASS |
| `clippy` gate (`RUSTFLAGS="-D warnings"`) | PASS, zero warnings |
| Rust lib tests | **4149 passed, 0 failed, 7 ignored** (upstream base: 4123) |
| `scripts/run_regression_suite.sh` | **845 PASS, 0 FAIL, 8 SKIP, 0 AB-diff failures** |
| `check_env_test_hygiene` | PASS — 155/155 env reads, at budget |
| `check_ci_gate_parity` | PASS — 145 commands, 19 invocation contracts |
| `check_doc_links` | PASS |
| Split regression gates | 4/4 PASS on x86-64 **and** i686 |
| Four-vendor execution oracle | **9 agree / 0 diverge / 0 error** |
| New tests | `ivsr_address_add.c` + 3 unit tests; two prior tests fixed (§11) |

**The slow half was NOT run**, by explicit instruction; it is delegated to GitHub
CI, so the delivery stamp is `mode=fast` and the snapshot records
`ci_local-fast-PARTIAL-NOT-DELIVERABLE`. What that costs: the **220-check
benchmark-output oracle did not run** on this tree and the **regression corpus
under debug assertions did not run**. No earlier slow result is reused as evidence
for this tree. §4's substitute evidence is strong and is weaker than executed
benchmark outputs.

---

## 11. A latent defect in the previous round's own tests

Adding i686 coverage to `check_ivsr_domains.sh` immediately exposed that
`ivsr_signed_wrap_impldef.c` (a pin, not a reproducer — see
[FOLLOWUP-2026-10-06-ivsr-domain-audit.md](FOLLOWUP-2026-10-06-ivsr-domain-audit.md)
§1) and `ivsr_unsigned_sparse_wrap.c` (a reproducer) — both shipped in the
previous delivery — are LP64-only by construction. On ILP32
`(size_t)UINT32_MAX + 1` wraps to **0**, the span collapses, and the writes go
through a wild pointer: both **segfaulted** rather than reporting anything. GCC's
own `-Wstringop-overflow` flags it. They now self-skip loudly, so the i686 leg of
that gate is real coverage instead of a crash.

The first attempt at the guard was itself wrong and is recorded because the failure
mode is instructive: `#if UINTPTR_MAX < 0xFFFFFFFFu` is **false** on ILP32, where
`UINTPTR_MAX` *equals* `0xFFFFFFFF`, so it selected the broken path. The condition
is now `< 0xFFFFFFFFFFFFFFFFull`.

---

## 12. Self-assessment

What went well: the measurement problem was found before it corrupted a decision,
and the method was changed rather than the tolerance widened. One real win landed
and passes both metrics. Two plausible, correct-in-isolation optimizations were
implemented, measured, and **reverted** instead of being kept because they were
elegant or because a reviewer had asked for them. One of this session's own
measurements was found to be based on a misidentified baseline and was corrected in
the record rather than left standing. An env-read ratchet was satisfied the hard
way instead of by raising the budget.

What did not go well: the session's single landed win is **−0.49%** on the kernel
suite. The two worst kernels (`classify` 2.057×, `namechars` 1.653×) were
diagnosed precisely and **not fixed**, because both fixes need pipeline or
register-allocator work whose only validator is a gate this VM is instructed not to
run. The first attempt at the case-fold merge also cost a build-and-measure cycle
on a hypothesis about pass ordering that the measurement then falsified — cheap,
but it was a guess made before the cheapest available check.

Nothing here is a Raptor Lake result, and no speedup claim is made.
