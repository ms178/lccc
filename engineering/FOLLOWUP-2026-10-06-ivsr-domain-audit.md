# Follow-up: IVSR index-domain audit and adjudication of the external review

**Date:** 2026-10-06
**Upstream base:** `6051e87304c6b07a2ed1a9a516aa5744d8110955` (`ms178/lccc` `main`,
merge of PR #769). This base already contains the first-round patch as
`dd0127998aae32a0a56626091bd9c93b597d2aff`.
**Host:** Debian 13 VM, Xeon, GCC 14.2, binutils 2.44, Rust 1.99.0, 8 GiB swap.
**Build policy:** `scripts/build_lccc_fast.sh` — fastbuild, Rust `-O1`, `-j2`.
**Target hardware note:** the i7-14700KF (Raptor Lake) is **not** this host and
was **not** measured. PMU counters are unavailable in this VM. Every number below
is a static count or a VM execution result, and is labelled as such.

Companion document for the first round:
[`FOLLOWUP-2026-10-06-codegen-audit.md`](FOLLOWUP-2026-10-06-codegen-audit.md).

---

## 1. Headline

Two **live miscompiles** were found on the merged upstream base, in the exact
subsystem the external review called "correct… no edge case found that breaks
them". Both are fixed, both have an executable reproducer that segfaults on the
base and passes on the fix, and both are covered by a four-vendor execution
oracle. The fix costs **zero** instructions on the 55-program default corpus.

| # | Defect | Base `6051e87`, `-O2` | Fixed | GCC 14 / 16.2, Clang 23.1, ICC, ICX |
|---|---|---|---|---|
| IVSR-DOMAIN-1 | `p[(int32_t)i]`, `uint32_t i` crossing `UINT32_MAX` | **SIGSEGV** | correct | correct |
| IVSR-DOMAIN-2 | `p[i]`, `uint32_t i` crossing `UINT32_MAX`, no bound | **SIGSEGV** | correct | correct |

These four files do two DIFFERENT jobs, and an earlier revision of this document
called all four "reproducers", which is wrong for one of them. Measured on base
`6051e873` at `-O1`/`-O2`/`-O3`:

**Reproducers** — fault on the base, pass on the fix:

```
tests/regression/ivsr_signedness_domain.c        # 32- and 64-bit signed views + affine form
tests/regression/ivsr_unsigned_sparse_wrap.c     # 16 GiB sparse index space, guard pages
tests/oracle/programs/ivsr_index_domains.c       # four-vendor execution oracle program
```

**Pin** — passes on the base TOO (`ivsr_signed_wrap_impldef: OK` at all three
levels), so it demonstrates nothing about the defect. Its job is different and
still necessary: it freezes the C17 6.3.1.3p3 implementation-defined-wrap
semantics so that a FUTURE change to the predicate is caught. Calling it a
reproducer overstates the evidence, so it is listed separately.

```
tests/regression/ivsr_signed_wrap_impldef.c      # C17 6.3.1.3p3 implementation-defined wrap
```

`ivsr_unsigned_sparse_wrap.c` and `ivsr_signed_wrap_impldef.c` reserve 16 GiB of
**virtual** address space with `MAP_NORESERVE` and commit four pages. A wrong
pointer recurrence therefore faults instead of silently returning a plausible
number — the failure mode is observable, not statistical.

---

## 2. Adjudication of the external review

The review is competent and its two headline endorsements are correct. It is
also **wrong on the one thing it was most confident about**, and two of its
file citations do not exist. Verdicts below are backed by measurements taken in
this tree, not by reading.

### 2.1 Agree

- **"Fix #1 and #2 are real, well-isolated, correctly-reasoned bug fixes."**
  Confirmed, and independently re-derived. The `va_arg_pack_len` double-offset
  and the truncating-backedge cast peel are both genuine.
- **"Keeping `CCC_FP_EXTRACT_HOMES` opt-in is correct."** Confirmed. The
  first-round evidence (52.9% regression without `-lm`, 20.5% improvement with
  it, 10.6% on matched data addresses) is a link-layout sensitivity, not a win.
  It stays opt-in.
- **"The oracle/tooling fixes are legitimate methodology corrections."**
  Confirmed: 26/26 workload-extraction helper tests pass; the `memcmp` sign
  normalisation retains the exact hand-rolled scan output.
- **"180KB of evidence."** Accurate to the byte: `du -sb` on
  `engineering/evidence/2026-10-06-audit` reports **143267** bytes (180K on-disk
  blocks). Consistent with the repo's existing `engineering/evidence/<date>/`
  convention.
- **Item 3's substance** — that the `result_type()` change is unconditional and
  its blast radius was undersold — is **correct and was the most valuable thing
  in the review**. See §2.2 for where its enumeration is wrong.

### 2.2 Disagree, with evidence

**(a) "no edge case found that breaks them" (fix #1) is false.**
The review scored correctness of the two headline fixes 9.5/10 and wrote that it
"re-derived both bugs from the surrounding code and they check out". Re-deriving
the *narrow* (`u8`/`u16`) case is not the same as auditing the *domain* of the
recurrence. Two executable shapes segfault at `-O2` on the merged base. They are
NOT pre-existing benchmark programs — they were written for this audit from the
base's own predicate and are shipped here as new regression tests, so the honest
claim is "two new reproducers fault on the base", not "two shipped benchmarks
were broken":

```
$ 6051e87-lccc -O2 tests/regression/ivsr_signedness_domain.c -o t && ./t      # SIGSEGV
$ 6051e87-lccc -O2 tests/regression/ivsr_unsigned_sparse_wrap.c -o t && ./t   # SIGSEGV
$ gcc -O2 ... && ./t                                                          # 160 / OK
```

The first-round fix closed the *truncating* cast and left the *same-width
signedness* cast open, because the derived-expression matcher admitted any cast
with `to_ty.size() >= from_ty.size()` — an equality that silently classifies
`U32 -> I32` as a widening.

**(b) "eight independent consumers", and two of the eight paths do not exist.**
The real count is **17 call sites in 9 files**:

```
$ grep -RIn '\.result_type()' src | wc -l
17
src/backend/common.rs                      1     src/backend/generation.rs               1
src/backend/i686/codegen/prologue.rs       1     src/backend/regalloc.rs                 2
src/backend/stack_layout/slot_assignment.rs 8    src/ir/provenance.rs                    1
src/passes/iv_widen.rs                     1     src/passes/loop_carried_forward.rs      1
src/passes/reassoc_latency.rs              1
```

Two of the files the review cited do not exist in this tree:
its `src/passes/provenance.rs` was `src/ir/provenance.rs`, and its
`src/backend/i686/prologue.rs` was `src/backend/i686/codegen/prologue.rs`.
It also missed `regalloc.rs`'s own two
sites and counted 4 of `slot_assignment.rs`'s 8. Its *conclusion* about
`classify_value` shrinking the F32 lane slot from 8 to 4 bytes is nonetheless
**correct** — verified at `slot_assignment.rs:711` — and is now pinned by a test
that asserts non-overlapping spans for four simultaneously-live lanes and a
byte-exact reload across a real adjacent 4-byte boundary.

Every one of the 17 sites now carries its verdict in a comment at the changed
arm in `src/ir/instruction.rs`, so the next reviewer does not have to chase them.
The i686 effects are real and were **not** in the review's list: F64 lane results
become `wide_values` and now veto the prologue's 32-bit compaction they previously
slipped through as `None`. `check_fp_extract_homes.sh` therefore covers i686 as
well as x86-64 and x86-64-v3.

**(c) Item 2's proposed relaxation is both unsound and worthless here.**
The review asked to widen `look_through_casts` from `from_ty == to_ty` to
`from_ty.size() == to_ty.size()`, reasoning that "a phi's backedge value must
match the phi's declared type, so any Cast there is unavoidably narrowing for
that idiom" and that a same-width signedness reinterpretation carries "no
wraparound risk".

Measured, on this tree, with the relaxation built:

```
55-program default corpus, -O2 -march=x86-64-v3, static instruction count
  conservative (from_ty == to_ty) : 8139
  relaxed  (equal size)           : 8139     <- zero difference, no program changed
```

It recovers **no** optimization. And the soundness argument is weaker than
claimed: the "signed overflow is UB" theorem that licenses a linear pointer
recurrence for a signed IV does **not** cover a signed counter *incremented in
unsigned arithmetic*. C17 6.3.1.3p3 makes `INT_MAX -> INT_MIN`
implementation-defined, not undefined, so

```
2147483646, 2147483647, -2147483648, -2147483647
```

is a legal, non-linear index sequence whose *bit patterns* are consecutive —
precisely what a same-width peel accepts. `tests/regression/ivsr_signed_wrap_impldef.c`
pins the end-to-end behaviour over a 16 GiB sparse mapping. (In practice LCCC's
frontend lowers `i = (int)((unsigned)i + 1u)` straight to `Add(I32)`, so the shape
does not currently arise from C source — but that is a frontend implementation
detail, not a theorem, and it is not a reason to weaken the pass.)

**Decision: reject.** Zero measured benefit, a real soundness cost, and the
review itself conditioned the change on a differential test that this tree now
has — and that test argues against it. Recorded as `IVSR-SAMEWIDTH-1` in
[`backlog.md`](../backlog.md) so the proposal is not re-litigated without a
workload where it fires.

**(d) Item 4's "never asserts they remain GPR-eligible" — accepted and fixed.**
`scalar_lane_class_tests` now asserts `!non_gpr.contains(&id)` for the integer
lane-extract IDs.

**(e) Item 3's rename request — accepted, and extended.**
`check_audit_loop_contracts.sh` is split into `check_ivsr_domains.sh`,
`check_fp_extract_homes.sh` and `check_va_arg_pack_len.sh`, with the original
script reduced to the bottom-tested branch contracts it is named for. All four
are wired into both `scripts/ci_local.sh` and `.github/workflows/ci.yml`;
`scripts/check_ci_gate_parity.py` reports PASS.

**(f) The review's scorecard dimension "Change scoping / communication honesty: 6"
is fair**, and the remedy is in the tree rather than in a PR description: the
consumer audit lives as a comment at the code that changed.

---

## 3. What was actually changed

### 3.1 `cast_preserves_offset_value` — two questions, not one

The old test `to_ty.size() >= from_ty.size()` conflated *value preservation* with
*storage size*. The replacement separates them:

- `cast_preserves_integer_value(from, to)` — preserves the mathematical value over
  the **whole** source domain. Widening an unsigned source, or a signed source
  into a signed target. `U32 -> I32` fails: `(int32_t)UINT32_MAX` is `-1`, and
  `sext(-1) != zext(UINT32_MAX)`.
- `cast_preserves_offset_value(from, to)` — the above **or** a same-width
  signedness reinterpretation *at or above the pointer width*. Address arithmetic
  lives in the pointer ring, so on LP64 `(ptrdiff_t)(size_t)x` and `(size_t)x` are
  the same 64-bit pattern and the recurrence `base + k*step*stride` is formed in
  that same ring.

The distinction is not cosmetic; **both** errors are measurable. Using the
value-only predicate (i.e. rejecting the pointer-ring reinterpretation too) costs
real code on the default corpus, because five programs spell their 64-bit index
`U64 -> I64`:

| program | value-only predicate | ring-aware predicate | base `6051e87` |
|---|---|---|---|
| `histogram` | 64 | **59** | 59 |
| `linux_rbtree` | 249 insns / 17 stack | **237 / 10** | 237 / 10 |
| `linux_find_bit` | 148 | **147** | 147 |
| `i686_alu_chains` | 494 | **492** | 492 |
| `zlib_ng_adler32_combine` | 294 | **293** | 293 |
| **corpus total (55 programs)** | 8154 | **8139** | 8139 |

So: the ring-aware predicate is byte-for-byte neutral against the base on the
whole corpus, and fixes both segfaults. That is the criterion used — not "smaller
than before", which would have hidden the regressions behind one improved program
(`sqlite_varint` is 6 instructions smaller with the *value-only* predicate,
because that predicate accidentally suppresses an unprofitable reduction there;
see §5).

### 3.2 `unsigned_iv_bound` — the proof IVSR-WRAP-1 said was still missing

The first-round document closed with: *"This does **not** prove all unsigned
32-bit pointer recurrences sound; a reusable no-wrap proof for every derived
expression remains a hard prerequisite."* That prerequisite is now discharged
locally, per IV, per loop:

- the loop is single-latch and the header is **not** its own latch (a self-loop
  proves nothing about which edge is the backedge);
- the header's conditional terminator branches on a `Cmp` whose **destination is
  that condition**, in **the phi's own unsigned type** — a comparison of a cast
  copy says nothing about the phi's domain;
- the comparison references **this exact phi** on one side;
- the polarity, after normalising for which target stays inside the loop, is a
  **strict** `Ult` for `+1` or `Ugt` for `-1`. An inclusive limit (`Ule`) or a
  multi-unit step can still wrap;
- the other operand is a constant or a loop-invariant value.

It returns the proven body range plus whether the limit was a compile-time
constant. No bound ⇒ no pointer recurrence. The modular index recurrence is still
available and is always correct, so this is a loss of an optimization in the
unprovable case, never a loss of correctness.

### 3.3 `offset_product_cannot_overflow` — the offset's own ring

A proven non-wrapping IV is necessary but not sufficient: `offset = (iv + k) *
stride` is evaluated in `mul_ty`, while the recurrence is formed in the pointer
ring. They agree when

- `mul_ty` is signed — a signed product cannot wrap in a defined program
  (C17 6.5/5, the same theorem `iv_widen` relies on); or
- `mul_ty` is at least the pointer width — the product wraps in exactly the ring
  the recurrence lives in; or
- `mul_ty` is unsigned and **narrower** than the pointer ring, *and* an exact
  constant bound proves `(hi + k) * stride < 2^width` and `(lo + k) * stride >= 0`.

A first attempt used a blanket "veto every sub-pointer-width multiply". That is
lazy and wrong: it would forbid the extremely common constant-trip `i * 4`
computed in 32 bits. The proof-based version admits it whenever the bound is
exact. No shipped workload currently reaches the narrow-unsigned-product arm, so
this is a **proof, not a measurement** — stated as such rather than dressed up as
a win.

### 3.4 Overflow-hardened offset arithmetic

`iv.step * stride` and `(init + add_offset) * stride` are now `checked_mul` /
`checked_add`, and the initial constant is reinterpreted in the **IV's own
domain** before scaling: `IrConst`'s signed storage is not the IV's mathematical
value, so a `U32` seed of `0xFFFFFFFE` was being scaled as `-2`.

### 3.5 The predicate is now shared, because it was already duplicated three times

Auditing the fix rather than stopping at it turned up the reason it was possible:
**the codebase already knew this rule, in two other files, spelled two other
ways.**

- `src/backend/generation.rs`'s SIB-index cast peel carried the whole-domain
  value-preservation rule inline, with a comment describing *exactly* the
  `I32 -> U32 -> I64` miscompile it prevents ("`t[(unsigned char) c]` with
  `c = -1` addressed `t[-1]` instead of `t[255]`… that shape is every
  character/CRC/tolower table lookup").
- `src/passes/loop_carried_forward.rs` carried a same-width peel that is sound
  only because that analysis is structurally restricted to pointer-width
  integers — i.e. the pointer-ring notion, expressed as an invariant of the
  caller rather than of the predicate.
- `src/passes/iv_strength_reduce.rs` carried `to_ty.size() >= from_ty.size()` —
  a third spelling, weaker than either, and the one that miscompiled.

Three spellings of two notions in three files, with no shared statement of what
either notion *is*, is how the third drifted. Both predicates now live on
`IrType`, next to the size and signedness facts they are stated in terms of:

```rust
IrType::cast_preserves_integer_value(to)  // value preserved over the WHOLE domain
IrType::cast_preserves_offset_value(to)   // …or a reinterpretation inside the pointer ring
```

`generation.rs`'s inline condition is replaced by a call to the first (its extra
`val_ty` obligation — a cast whose `from_ty` disagrees with the source value's
real type is a width lie — stays local, because it is about this peel's inputs,
not about types). `loop_carried_forward.rs` keeps its structural form and gains a
comment stating which predicate arm it is an instance of and what invariant makes
it sound. The exhaustive tests move next to the predicates
(`src/common/types.rs::cast_predicate_tests`) so they cannot be orphaned by a
later refactor of either consumer.

**The two predicates are provably equivalent to the inline rule they replace.**
`generation.rs` asked for `to.size() >= from.size()` **and** (same signedness
**or** (`from` unsigned **and** `to` strictly wider)); `cast_preserves_integer_value`
asks for identity **or** (`to` strictly wider **and** (`from` unsigned **or** `to`
signed)). Case analysis over the four signedness combinations at equal and
unequal width gives the same truth table, and `cast_predicate_tests` checks it
against each type's actual domain extrema rather than against the implementation.

Because "provably equivalent" is a claim and not evidence, it was measured:

| differential | result |
|---|---|
| sources | **911** (every `.c` in `tests/regression`, `tests/benchmark/programs`, `tests/oracle/programs`) |
| matrix | `-O2`, `-O3`, `-O2 -march=x86-64-v3` × x86-64 and i686 (`-m32 -msse2`) |
| assembly comparisons | **5388** |
| byte-identical | **5388** |
| differing | **0** |
| not compilable / skipped | 78 |

The rebuilt post-refactor binary is `cmp`-identical to the one saved before the
differential run, so the comparison is not an artifact of a nondeterministic
build. Record: `engineering/evidence/2026-10-06-ivsr-domain/refactor-neutrality.json`.

### 3.6 `find_affine` is type-aware

`(iv + k) * stride` now requires the `Add`'s own type to equal the multiply's
type, so an affine offset computed in a different ring cannot seed a recurrence
in another one.

---

## 4. Four-vendor execution oracle

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
`cicxlatest` = x86-64 icx (latest). Raw record:
`target/review-audit/oracle-full-corpus.json`.

Reading of the table, without inflation:

- **Semantics: 9/9 agree, 0 diverge, 0 error** across four independent vendors,
  including the new `ivsr_index_domains` program whose base-tree build segfaults.
- **Size: LCCC is the smallest total against all four vendors** — 0.82× GCC,
  0.67× Clang, 0.41× ICC, 0.64× ICX. Instruction count is *data*, not a
  specification; a compiler that wins this table by miscompiling has lost the
  table that matters, and the semantics column is why this one is trustworthy.
- **Two programs are behind**: `int_alu` (0.543 — Clang folds the whole loop, see
  §5) and `control` (0.935 — GCC 116 vs LCCC 168). Both are recorded as open
  items with reproducers rather than claimed as fixed.

---

## 5. Remaining gaps, with reproducers

### CONSTLOOP-FOLD-1 — constant-trip-count loops are not evaluated at compile time

`int_alu`: LCCC **92**, GCC **68**, Clang **23**, ICC **81**, ICX **28**.
Clang's actual output for the 64-iteration FNV-1a chain is one instruction:

```
movabsq $6495794128378805577, %rsi      # the entire 64-iteration hash, folded
```

and it strength-reduces the 256-iteration narrow-cast loop to an 8-instruction
closed form. ICX does the same (28). LCCC and GCC execute both loops.

LCCC's complete unroller caps trip at 16 with a 512-expanded-instruction budget
(halved for FP bodies) — `src/passes/loop_unroll.rs`. Raising the cap blindly is
code bloat, and this VM is explicitly **not** permitted to run the slow/benchmark
gates that would catch it. The design worth building is self-limiting rather than
threshold-tuned:

> Speculatively complete-unroll a call-free, side-effect-free, constant-trip loop;
> run the constant folder; **revert unless the folded result is smaller than the
> loop it replaced.**

That makes the transform monotone by construction — it cannot lose — instead of
depending on a budget number that has to be re-tuned per corpus.

Reproduce: `python3 scripts/godbolt.py compile cclang2310 tests/oracle/programs/int_alu.c --flags -O2`.

### `sqlite_varint` — an unprofitable reduction, not a correctness issue

The value-only cast predicate made `sqlite_varint` 6 instructions *smaller*
(398 → 392) purely by accident: it suppressed a `U64 -> I64` reduction that costs
more than it saves in that loop. The ring-aware predicate correctly allows the
cast, so the 6 instructions come back (398, equal to base). This is a
**profitability** gap in IVSR — the pass has no cost model for a reduction whose
pointer recurrence loses to an `imul` on this backend — and it is the same root
cause the first round documented for the opt-in scalar-derived flavor (latch
phi-web parking: every added loop-carried web materialises to a slot home). It is
recorded here because hiding it behind a cast predicate that is wrong for other
programs would be exactly the kind of accidental win that does not survive the
next corpus.

### Carried forward, unchanged

`IVOPTS-1` (shared scalar/SIMD address recurrences — SLP `VecLoadF64x2` /
`VecStoreF64x2` carry base/offset directly and IVSR enumerates only
`GetElementPtr` uses), `RA-CSAVE-1` (callee-save traffic: 45.4% of the
final-default causal stack census), `PF-SCHEDULER-1` (SHA message-schedule
vectorisation), `PF-CHACHA-1`, `FP-LANE-1` (allocation still opt-in). None of
these is claimed as addressed by this round.

---

## 6. Validation actually performed

| Gate | Command | Result |
|---|---|---|
| Format | `cargo fmt --all -- --check` | **clean** |
| Lint | `RUSTFLAGS="-D warnings" cargo clippy --profile fastbuild --all-targets --locked -j2` | **clean, zero warnings** |
| Rust units (lib) | `cargo test --profile fastbuild --lib --locked -j2` | **4145 passed, 0 failed, 7 ignored** (base: 4123) |
| Refactor neutrality | pre/post-refactor `-S` byte comparison, 911 sources × 3 flag sets × 2 targets | **5388 / 5388 identical, 0 differ** |
| Rust units (all targets) | `cargo test --profile fastbuild --all-targets --locked -j2` | **all green** |
| Local CI mirror, fast half | `scripts/ci_local.sh --fast` | see §7 |
| IVSR domains | `tests/regression/check_ivsr_domains.sh` | 4 programs × `-O0..-O3` pass |
| FP lane slots | `tests/regression/check_fp_extract_homes.sh` | 2 programs × 3 ISAs × 3 knob modes × `-O0..-O3` pass |
| VA pack length | `tests/regression/check_va_arg_pack_len.sh` | pass |
| Loop contracts | `tests/regression/check_audit_loop_contracts.sh` | pass |
| Gate parity | `scripts/check_ci_gate_parity.py` | PASS (144 commands, 17 invocation contracts) |
| Workflow shell | `scripts/check_ci_workflow_shell.py` | ok (2 workflow files) |
| Doc links | `scripts/check_doc_links.py` | pass |
| Corpus neutrality | 55 programs, `-O2 -march=x86-64-v3`, static insns + stack refs | **8139 vs 8139 base, no program differs** |
| Four-vendor semantics | `godbolt_oracle.py --oracle-set all-vendors` | **9 agree / 0 diverge / 0 error** |

New unit tests (22 over the base):

```
promoted_narrow_backedges_are_not_linear_ivs
identity_casts_and_copies_keep_basic_ivs_but_signedness_needs_proof
derived_cast_proof_is_about_values_not_storage_size
offset_cast_proof_adds_only_the_pointer_ring_reinterpretation
offset_product_proof_needs_an_exact_bound_only_below_the_pointer_ring
value_preservation_is_exactly_domain_containment            (src/common/types.rs)
offset_preservation_adds_only_the_pointer_ring_reinterpretation (src/common/types.rs)
the_two_reported_miscompile_shapes_stay_rejected            (src/common/types.rs)
non_integer_types_are_never_admitted                        (src/common/types.rs)
unsigned_bound_records_whether_the_limit_was_a_constant
unsigned_no_wrap_requires_header_polarity_unit_step_and_unsigned_cmp
derived_signedness_then_widening_is_not_a_pointer_iv
scalar_lane_slots_are_width_partitioned_even_without_fp_homes
+ the extended scalar_lane_class_tests GPR-eligibility assertions
```

`derived_cast_proof_is_about_values_not_storage_size` is exhaustive: it walks all
64 ordered pairs of the eight integer IR types and checks the predicate against
the actual domain extrema of each pair, so a future "simplification" of the
condition fails on a concrete counterexample rather than on a code review.

`unsigned_no_wrap_requires_header_polarity_unit_step_and_unsigned_cmp` is a
differential matrix over comparison opcode × branch polarity × step sign, and
additionally asserts that a self-loop header is never accepted.

---

## 7. What was **not** run, and why

Per explicit instruction, `scripts/ci_local.sh` was run with `--fast` **only**;
the slow half (`cargo test` with debug assertions, the regression corpus, the
220-check benchmark-output oracle, peephole whitespace invariance) was **not**
executed on this VM and is delegated to GitHub CI. Consequences, stated plainly:

- The local pass stamp is `mode=fast`, so `scripts/lccc-snapshot.sh` records this
  delivery as **`ci_local-fast-PARTIAL-NOT-DELIVERABLE`**, not `mode=full`. That
  label is the script's own honesty mechanism and is reproduced here rather than
  worked around with `LCCC_SNAPSHOT_UNGATED`.
- The **benchmark-output oracle (220 checks) did not run**. Static corpus
  neutrality (§6) is the substitute evidence and is weaker: it compares instruction
  and stack-reference counts, not executed outputs.
- The **regression corpus with debug assertions did not run**.
- The first round's full-tree slow result (6 passed / 0 failed, Rust debug suite
  4137 passed) applies to tree `acbba833`, **not** to this tree, and is not
  reused as evidence here.

Also not available, unchanged from the first round:

- **No PMU.** `perf` exits 0 but the counters are unsupported in this VM. No
  cycle, branch-miss, cache-miss or page-split claim is made anywhere in this
  document.
- **No Raptor Lake.** The requested i7-14700KF is not this host. Every runtime
  number in the first-round document was a VM measurement and is labelled there.
- **No runtime A/B for these fixes.** They are wrong-code fixes: the correct
  comparison is base-segfaults / fix-does-not, plus static corpus neutrality to
  show nothing was paid for it. A speedup claim would be unsupported and is not
  made.
- **The exact requested linker releases** (mold 2.42.1, lld 23.1, bfd 2.47) are
  still not installed locally; `tools/linker/oracle_versions.sh` guards the
  versions that are present and refuses to report a version it did not verify.
  The CI-downloaded GNU assembler reports banner **2.47.20260726**, which is
  recorded verbatim rather than conflated with a bfd 2.47 linker.

---

## 8. Reproducing this round

Prerequisites that the earlier revision of this section left out, both of which
produce failures that look like compiler bugs:

* a **full** clone. `git clone --depth 1` cannot check out `6051e873`, because it
  is not the tip; run `git fetch --unshallow` first if the clone was shallow.
* **32-bit glibc headers** (`gcc-multilib`, `libc6-dev-i386` on Debian). Without
  them every `-m32` leg of `check_ivsr_domains.sh` dies in the preprocessor with
  `bits/libc-header-start.h: No such file or directory`, and the gate exits 1 for
  a reason that has nothing to do with the change under test. This was hit for
  real on 2026-10-07: a wiped workspace lost the packages and the gate failed
  identically on the merged base and on the new tree, which is how it was
  identified as environmental rather than a regression.

```bash
git clone https://github.com/ms178/lccc.git && cd lccc   # NOT --depth 1
git checkout 6051e87304c6b07a2ed1a9a516aa5744d8110955
# This round is merged upstream as dd012799, so the patch below is historical:
# the reproducible route is `git checkout dd012799` on a full clone. The
# /home/user path is a sandbox-local artifact and will not exist elsewhere.
git apply /home/user/ms178-1.patch          # or: git am the series in artifacts/

./scripts/build_lccc_fast.sh                 # fastbuild, Rust -O1, -j2
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo clippy --profile fastbuild --all-targets --locked -j2
cargo test --profile fastbuild --all-targets --locked -j2

env CCC=target/fastbuild/lccc bash tests/regression/check_ivsr_domains.sh
env CCC=target/fastbuild/lccc bash tests/regression/check_fp_extract_homes.sh
env CCC=target/fastbuild/lccc bash tests/regression/check_va_arg_pack_len.sh
env CCC=target/fastbuild/lccc bash tests/regression/check_audit_loop_contracts.sh
python3 scripts/check_ci_gate_parity.py
python3 tools/oracle/godbolt_oracle.py --oracle-set all-vendors   # needs network
bash scripts/ci_local.sh --fast
```

To see the two miscompiles on the unpatched base, build `6051e87` separately and
run `tests/regression/ivsr_signedness_domain.c` and
`tests/regression/ivsr_unsigned_sparse_wrap.c` at `-O2`; both SIGSEGV.

---

## 9. Self-assessment

What this round did well: it did not accept the review's confidence about
correctness, and it found two live segfaults in the subsystem the review had
scored 9.5/10. It replaced a blanket restriction with two explicit proofs and
measured the cost of getting each direction wrong. It rejected a proposed change
with data instead of adopting it because a reviewer asked.

What it did not do: no new performance was won. The corpus is neutral, two
programs remain behind GCC/Clang, and the largest single identified opportunity
(CONSTLOOP-FOLD-1, a 4× instruction gap against Clang on `int_alu`) is documented
with a design and a reproducer rather than implemented — because implementing it
without the benchmark gates this VM is not allowed to run would be exactly the
unguessable, unvalidated change the method forbids. That is a real limitation of
this round, not a success.
