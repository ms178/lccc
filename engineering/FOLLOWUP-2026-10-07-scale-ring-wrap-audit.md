# Follow-up 2026-10-07 — scale-ring wrap miscompiles: audit of PR #772 and two fixes

Rebase base: `6a9690c3` (merge of PR #772, "Teach IVSR LCCC's own addressing form:
two wrap miscompiles fixed, byte-walks accelerated").

This document is the red-team audit of that work plus the two live miscompiles
the audit found and fixed. Every number here was produced on this VM
(`rustc 1.99.0`, `gcc 14.2.0`, 2-vCPU Xeon, 6 GiB swap); every claim names the
tool that made it. Nothing is quoted from a stale tree: the binaries are pinned
by path in §5.

## 1. Scope and method

* Read the full `c3259492` diff (`iv_strength_reduce.rs` +983, `common/types.rs`
  +173, `backend/generation.rs` ±19/4, `backend/regalloc.rs`, `slot_assignment.rs`,
  `ir/instruction.rs`, `passes/mod.rs`, six regression tests, five evidence files).
* Rebuilt the new baseline (`-O1 -j2`, 3m26s) and re-ran their own gates and
  regression corpus *before* touching anything: `check_ivsr_domains.sh`,
  `check_fp_extract_homes.sh`, `check_va_arg_pack_len.sh`, lib tests — all green.
* Attacked the *new proof obligations* rather than the new spellings, on the
  principle that a transform's soundness lives in its predicates, not in its
  intent. Two hypotheses came straight out of reading the code:

  | # | hypothesis | test | outcome |
  |---|---|---|---|
  | F1 | `unsigned_iv_bound` reads the loop limit with `IrConst::to_i64()`, but constants are stored in the operand's *own ring* (`const_for_int_ty`); a `uint32_t` bound ≥ 2^31 is a **negative** `I32`, so an almost-2^32 trip count is handed to `offset_product_cannot_overflow` as a negative maximum | differential with a `uint32_t` bound of `0x80000003u` | **live miscompile**, §2 |
  | F2 | the back end's SIB peel (`resolve_index`) folds `shl`/`mul`/`add(v,v)`/`add(iv,const)` into `disp(base,index,scale)` — evaluated in the *pointer* ring — without asking whether the peeled instruction's own ring is narrower and wraps | same differential with `CCC_NO_IVSR=1` (isolates the back end) | **live miscompile**, §2 |

## 2. The two defects

Both are the same C semantics defect reached by two independent paths:
`buf[i * 2]` with `uint32_t i` must compute the product in **32 bits, wrapping**
(C17 6.5/5 unsigned arithmetic) and only then zero-extend it for the address.
Linearising that product into pointer-ring arithmetic is value-preserving only
while the product cannot wrap.

### 2.1 Reproducer

`tests/regression/ivsr_scale_ring_wrap.c` (added by this change; LP64 only, skips
loudly on ILP32 in the style of `ivsr_unsigned_sparse_wrap.c`). 4 GiB + 16 bytes
of index space mapped `MAP_NORESERVE` with three committed pages, so both the
wrapped offsets `{0xFFFFFFFE, 0, 2, 4}` and the linear offsets
`{0xFFFFFFFE, 0x100000000, 0x100000002, 0x100000004}` are inside the object and
a wrong choice is a *wrong sum*, not a fault. The program is fully defined: every
index computation is unsigned, every access is in bounds. Wrapped sum `110`,
linear sum `264`.

| compiler, `-O2` | `walk_const` | `walk_param` | `walk_dynamic` |
|---|---|---|---|
| lccc before this change (`lccc_prefix_6a9690c3_s26`) | 209 ✗ | 209 ✗ | 209 ✗ |
| lccc after this change | 110 ✓ | 110 ✓ | 110 ✓ |
| gcc 14.2.0 `-O2`/`-O3` | 110 ✓ | 110 ✓ | 110 ✓ |
| lccc `-m32` (all builds) | SKIP (span not representable in ILP32) | | |

`walk_param` takes the bound as a parameter: interprocedural constant propagation
specialises the clone back into the constant-bound case, which is why a *dynamic*
C source still reaches the constant-bound proof. `walk_dynamic` keeps the bound
opaque so no exact no-wrap proof exists; both transforms must then refuse on
their own.

### 2.2 F1 — the new no-wrap proof certified the wrap it exists to refuse

`unsigned_iv_bound` (new in #772) proves a narrow unsigned IV cannot wrap and
`offset_product_cannot_overflow` turns that into a licence for the pointer
recurrence. The limit was read as a mathematical value:

```rust
let hi = match other {
    Operand::Const(c) => c.to_i64().map(|n| n as i128 - 1).unwrap_or(max),
```

but a `uint32_t` limit of `0x80000003u` is stored as `IrConst::I32(-2147483645)`
— the ring-correct encoding, the same one `const_for_int_ty` writes and the same
one the *same commit* decodes correctly for `init_offset`
(`IrType::U32 => i64::from(v as u32)`). So `hi = -2147483646`, and for
`mul_ty = U32, stride = 2`: `lo = 0 >= 0` ✓, `hi = -4294967292 < 2^32` ✓ —
**certified**. The IVSR side of the differential then walks `p += 2` in the
pointer ring.

Introduced by #772: before it, that path had no numeric bound reasoning at all.

### 2.3 F2 — the peel's ring obligation was never stated

`resolve_index` peels `shl(idx,k)`, `mul(idx,k)`, `add(v,v)` (the canonicalizer's
`i*2`) and `add/sub(iv,const)` into the SIB form. The CPU evaluates that form as
`base + extend(index, index's own type) * scale + disp` **in the pointer ring**,
so the peel is value-preserving only when the peeled instruction's ring is not a
narrower wrapping one. Nothing asked. `CCC_NO_IVSR=1` isolates it: pre-fix
`buf[i*2]` reached memory as

```
movl %edx, %edx                 # zero-extend the index
movzbl (%rdi, %rdx, 2), %eax    # 64-bit index*2: no wrap
```

versus the value-preserving form the same compiler emits with the peel refused
(`CCC_NO_GEP_FOLD=1`) and after this change:

```
leal (%rdx, %rdx), %r8d         # 32-bit product: wraps, as C requires
movzbl (%rdi, %r8), %r11d
```

Pre-existing: the peel code is untouched by #772. #772's own message claims the
SIB peel "cannot drift again" because it now shares
`IrType::cast_preserves_integer_value` — that predicate governs the **cast** half
of the same address expression (and does so correctly, §3.1); the **scaling**
half carries the same obligation in a different form (a ring width, not a cast
relation) and stayed unstated. That is the one place I disagree with the
reasoning, not with the code it produced.

## 3. What I agree with, and why

1. **`cast_preserves_integer_value` / `cast_preserves_offset_value`.** The
   value-only predicate is exactly domain containment, and the exhaustive
   64-pair test against a domain model (rather than against the implementation)
   is the right way to freeze it. The pointer-ring arm is correctly scoped:
   same-width reinterpretation at or above the pointer width cannot change the
   pattern a `GetElementPtr` consumes, narrower ones can. Diffing the old inline
   peel condition against the new predicate by hand over all 8×8 pairs shows them
   provably identical, so the refactor is neutrality-preserving as claimed.
2. **ILP32 exclusion of the address-`Add` scan.** The measurement they quote
   (frame 142 → 191 stack refs, +49, for +2 instructions on
   `simd_crc_adler -O2 -m32`) is the right kind of evidence for a register-file
   constraint, and the 6-GPR explanation is consistent with this host's
   `i686` behaviour. Gating on `target_is_32bit()` rather than on a guess about
   the shape is correct; the differential still runs at `-m32` (§5) and skips
   loudly where the shape cannot be exercised.
3. **Domain-correct `init_offset`.** `(v + add_offset) * stride` with `v`
   re-interpreted through the IV's ring and checked arithmetic is the right
   repair for the `p[(int32_t)i]` family; my F1 is the same rule applied one
   function away.
4. **Callgrind `Ir` as the decision metric, wall clock rejected.** Their stated
   reason ("7 of 11 kernels moving beyond 3% with byte-identical codegen — a
   ~15% layout-noise floor") is the same phenomenon my loop-inversion work
   measured from the other side: on this host, `min`-of-N invents 5–25% effects.
   `Ir` is deterministic, allocation-free and cannot flatter either arm. Where a
   *runtime* claim is unavoidable, the interleaved same-binary A/B at ≥101
   samples with median + paired win rate is the protocol that survives here
   (`docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md` §6.4).
5. **Kill switches read once in the pipeline.** `CCC_NO_IVSR_PTR_ADD` resolved in
   `run_ivsr_ptr_add` and threaded through as a parameter is what the env-read
   ratchet asks for, and it is what made F2 isolable in one command.
6. **The new differentials.** `ivsr_address_add`, `ivsr_unsigned_sparse_wrap`
   and `ivsr_signed_wrap_impldef` map 16 GiB of virtual address space with
   guard pages instead of asserting over small buffers — the only way to make a
   wrong recurrence a *fault* rather than a silently plausible integer.

## 4. Fixes

Both fixes are one rule — *a ring narrower than the pointer ring that can wrap
must not be linearised* — applied where each path reads it.

* **`src/passes/iv_strength_reduce.rs`** — *landing note.* The F1 half of this
  fix is **already in this tree** in a different shape: `const_in_iv_domain`
  decodes a constant in the IV's own domain (its doc names the same
  `0xFFFFFFFF`-as-`-1` certificate), the descending ceiling follows the init,
  and `UnsignedIvBound` carries `bounded` rather than `exact`. That
  implementation enforces the same rule and carries its own decode tests, so
  this delta keeps it and lands no frontend rewrite: no `const_in_ring`, no
  `max` field, no `UnsignedIvBound` literal churn. The one F1 detail left out is
  the `max` well-formedness net: on this implementation a `bounded` interval is
  well formed inside its ring by construction (`hi` is a decoded limit minus
  one, or the init), and the `hi < lo` shape the net also refuses can only come
  from a zero-trip loop, where no address is ever computed. What this delta
  *adds* here is the end-to-end test for the decoded limit,
  `unsigned_wrap_certificate_uses_the_decoded_limit`.
* **`src/backend/generation.rs`**
  * `scale_ring_is_linear(ty)`: pointer-width-or-wider, or a narrower **signed**
    ring (no wrap in a defined program — the same theorem
    `offset_product_cannot_overflow` rests on), or a non-integer ring ⇒ refused.
  * Applied to all five peels that can carry scaling or displacement: `shl`,
    `mul` (both operand orders), self-`add`, `add(iv,const)`, `sub(iv,const)`.
* Regression test `tests/regression/ivsr_scale_ring_wrap.c`, wired into the
  `ivsr-integer-domains` fast gate (which already runs every test at `-O0..-O3`
  and `-m32`).
* Unit tests: `unsigned_bound_decodes_the_limit_in_the_iv_ring`,
  `unsigned_wrap_certificate_uses_the_decoded_limit` (IVSR),
  `peel_requires_a_ring_the_address_arithmetic_can_reproduce` (back end).

## 5. Verification

| check | command | result |
|---|---|---|
| differential, all levels/ISAs | `lccc -O0..-O3 -march=x86-64{,v3} ivsr_scale_ring_wrap.c` | OK ×8 |
| differential, `-m32` | same | SKIP loudly |
| differential vs the pre-fix binary | `lccc_prefix_6a9690c3_s26 -O2 …` | exits 5, `FAIL param: got 209, want 110` |
| differential vs gcc | `gcc -O2 …` | OK |
| no legal fold lost | pre-fix vs post-fix asm of `w64` (`size_t` index stride 2, `int j+1` disp, `u32` stride-1 walk) | **byte-identical** |
| corpus blast radius | `scripts/census_full_delta.sh post pre 1` (all TUs, `-O0/-O1/-O2/-O3/-Os`) | §5.1 |
| IVSR/lane/va gates | `check_ivsr_domains.sh` (+ my test), `check_fp_extract_homes.sh`, `check_va_arg_pack_len.sh` | PASS |
| lib tests | `cargo test --lib --profile fastbuild -j2` | §5.1 |
| fmt / clippy | `cargo fmt --check`, `cargo clippy --all-targets -j2` | clean |
| fast CI | `scripts/ci_local.sh --fast` | §5.1 |

### 5.0 Callgrind kernel A/B (deterministic; `valgrind --tool=callgrind`)

Protocol: each kernel compiled `-O3` by both binaries, linked against the shared
`tests/bench/driver.c` driver object built once by gcc, run as `./k <inner=5>`
under Callgrind; `Ir` read from the `totals:` line. All 11 kernel **checksums are
identical** between arms (a different answer would be a failure, not a win).

| kernel | pre-fix Ir | post-fix Ir | delta |
|---|---|---|---|
| k_adler32 | 476 612 | 476 643 | +0.01% |
| k_adler32_do8 | 356 775 | 356 789 | +0.00% |
| k_classify | 1 561 334 | 1 561 343 | +0.00% |
| k_hashmix | 310 169 | 310 202 | +0.01% |
| k_map64_sub | 159 668 | 159 681 | +0.01% |
| k_matchlen | 2 528 724 | 2 527 086 | −0.06% |
| k_memchr | 705 059 | 705 043 | −0.00% |
| k_namechars | 1 289 895 | 1 289 749 | −0.01% |
| k_strcmp_signed | 5 605 055 | 5 605 080 | +0.00% |
| k_strlen_scan | 7 008 019 | 7 008 011 | −0.00% |
| **k_varint** | **979 339** | **899 665** | **−8.14%** |

Ten kernels move by at most a handful of setup instructions (the ≤0.01% rows are
prologue/exit differences, not loop differences). `k_varint` — the kernel whose
index ring is exactly the shape this change stops linearising, and the kernel
PR #772 itself targeted — *gains* 8.1% of its retired instructions: keeping the
product in `uint32_t` lets the same address be re-formed in 32-bit arithmetic
instead of a 64-bit recurrence, and the checksum is unchanged. Absolute `Ir`
here is not comparable to `engineering/evidence/2026-10-07-perf-ivsr-ptradd/callgrind-kernel-ab.json`
(different inner count and driver linkage); the *delta* is the measurement.

### 5.0b What the fix costs (measured, not hidden)

Static asm over the benchmark corpus at `-O2` (90 TUs: 55 programs + 11 kernels
+ 24 kernel-corpus files): **86 byte-identical, 4 changed** —
`k_varint`, `k_matchlen`, `histogram`, `glibc_strstr`.

| TU | static insns | dynamic Ir (Callgrind, whole program) |
|---|---|---|
| k_varint | 63 → 63 | 979 339 → 899 665 (−8.14%) |
| k_matchlen | 40 → 39 | (kernel row: −0.06%) |
| histogram | 52 → 60 (+8) | 3 261 440 → 3 523 573 (**+8.04%**) |
| glibc_strstr | 173 → 174 (+1) | 25 005 791 496 → 26 030 175 693 (**+4.10%**) |

Both regressions are the **same defect being refused**. In `histogram`'s hot
loop the pre-fix code emitted

```
movzbl 1(%rdi, %rsi), %edx      # bytes[i + 1]: add folded into the displacement
```

i.e. it evaluated `zext32(i) + 1` when the C source (and the U32 IR add) says
`zext32((i + 1) mod 2^32)`. Those differ for exactly one input, `i = UINT32_MAX`
— which this program cannot reach, and which *no* analysis in the compiler
proves it cannot reach. Post-fix the add is materialised in its own ring
(`leal 1(%rsi), %edx` + `movzbl (%rdi, %rdx)`), one extra instruction per
element, plus the register pressure it implies. `glibc_strstr` shows the same
shape at +4.10%.

**Why this is the right trade, and what it opens.** The pre-fix fold is not a
design choice to be optimised but a miscompile: per the project's own ordering,
correctness is a hard constraint and a fast wrong compiler is worthless. It is
also *provably* recoverable: the only missing input is a bound on the index, and
`unsigned_iv_bound` (§4) already derives exactly that, per loop, from the
header's exit test. Widening a narrow-unsigned index add/mul into the pointer
ring **when that proof holds** (the same licence `iv_widen` uses, extended to
`add`/`sub`/`mul` on a bounded IV) makes `add u32` ring-exact, after which the
existing peel folds it again — recovering `histogram` and `glibc_strstr` while
keeping the wrap. That is the first thing the next session should implement;
the numbers above are its acceptance criteria, and `histogram`'s self-check plus
`ivsr_scale_ring_wrap.c` are its correctness gates.

### 5.1 (filled in by the run that produced this tree)

* corpus blast radius: recorded in the session ledger; the only TUs that change
  are those whose address expressions contain an unsigned narrower-than-pointer
  scaling or displacement — that *is* the fix's contract. Tuples whose index
  ring is the pointer ring are byte-identical (see "no legal fold lost" above).
* lib tests: run by the `cargo-test` gate of `ci_local.sh --fast` on this tree
  (the count is in that run's log; the pre-change baseline recorded for
  `f8aa3456`/S26 was 4137 passed / 0 failed / 7 ignored). This delta adds two
  unit tests (`peel_requires_a_ring_the_address_arithmetic_can_reproduce` in the
  back end, `unsigned_wrap_certificate_uses_the_decoded_limit` in IVSR) and
  touches no `UnsignedIvBound` literal (see the landing note in §4).
* `ci_local.sh --fast`: green on the frozen tree — see the snapshot ledger entry
  for seq 27, which names the run and its pass/fail counts.

## 6. Oracle evidence (Compiler Explorer, cached via `scripts/godbolt.py`)

Source: a standalone `oracle_wrap` function with the F1/F2 shape
(`for (uint32_t i = 0x7FFFFFFF; i < lim; i++) s += buf[i * 2];`), compiled
`-O2 -march=x86-64-v3`, AT&T, function body only, via
`scripts/oracle_asm.py` (cache namespace shared with `codegen_oracle.py`, so no
extra requests and no rate-limit risk).

| compiler | insns | load | store | spill | branch | shape that matters |
|---|---|---|---|---|---|---|
| lccc (post-fix) | 12 | 1 | 0 | 0 | 3 | `leal (%r8,%r8),%r9d` + `movzbl (%rdi,%r9)` — 32-bit product, wrap preserved |
| gcc 16.2 | 16 | 1 | 0 | 0 | 4 | `addl %esi,%esi` + `movzbl (%rdi,%rcx)` — 32-bit doubling, wrap preserved |
| icc 2021.10.0 | 12 | 1 | 0 | 0 | 3 | `addl $2,%edx` — the *offset* counter itself lives in 32 bits and wraps |
| clang 23.1.0 | 61 | 9 | 0 | 0 | 12 | trip-count restructure (`r8 = lim − 0x7FFFFFFF`), 8-way tail mask |
| icx (latest) | 62 | 7 | 0 | 0 | 8 | vectorised main loop (`vpbroadcastq`/`vpaddq`/`vpand`) |

What this establishes: lccc's *fixed* shape is the same idea as GCC 16.2's (fold
the product to a 32-bit doubling, then extend), and ICC 2021.10 independently
reaches the same semantics by keeping the offset in 32 bits — three vendors, one
ring discipline. It also settles the performance question that put lccc 12
instructions ahead of GCC 16.2 on this kernel, and shows ICX vectorising a shape
neither GCC nor lccc vectorises.

### 6.1 Runtime adjudication (executor pool, `--execute`)

Shape arguments are not results, so the same differential was *run* on all four
vendors. `scripts/godbolt.py` gained an `--execute` flag for this (the CE API's
`filters.execute`; the result carries `execResult.code/stdout`), and the probe
was made CE-safe (`engineering/evidence/2026-10-07-scale-ring-wrap/wrap_probe.c`)
with a loud SKIP if the shared executor refuses the 4 GiB sparse mapping.

| arm | exit | stdout | verdict |
|---|---|---|---|
| gcc 16.2 | 0 | `sum 110  (wrapped=110, linear=264)` | wrap preserved |
| clang 23.1.0 | 0 | `sum 110  (wrapped=110, linear=264)` | wrap preserved |
| icc 2021.10.0 | 0 | `sum 110  (wrapped=110, linear=264)` | wrap preserved |
| icx (latest) | 0 | `sum 110  (wrapped=110, linear=264)` | wrap preserved **and vectorised** |
| lccc pre-fix | — | `209` (local) | miscompile, both halves |
| lccc post-fix | — | `110` | matches all four vendors |

So the competition does not merely *look* ring-correct on this shape: every
vendor, including the one that vectorises it, reproduces the wrapped C
semantics. That is the standard the fix is held to, and it also answers the
performance question in the same breath — ICX vectorising a loop that GCC, ICC
and lccc keep scalar is a concrete target for §6.7 of the loop-rotation
follow-up.

## 7. Follow-ups this opens

1. ~~Four-vendor *execution* probe~~ — **done**, §6.1. What remains: run the
   *whole* wrap corpus (`ivsr_address_add`, `ivsr_unsigned_sparse_wrap`,
   `ivsr_signed_wrap_impldef`, `ivsr_scale_ring_wrap`) through the executor so
   every differential has a vendor column, and let `--execute` reach the corpus
   tools (`codegen_oracle.py --rank`) rather than only the `compile`
   subcommand.
2. **Recover the refused folds with a proven index domain** (§5.0b): extend the
   `iv_widen`/`unsigned_iv_bound` licensing to `add`/`sub`/`mul` on a bounded
   narrow-unsigned IV so the ring-exact widening happens in the IR and the back
   end's `scale_ring_is_linear` sees a pointer-ring instruction again. Acceptance
   criteria: `histogram` and `glibc_strstr` back to pre-fix `Ir` (or better),
   `ivsr_scale_ring_wrap.c` still green at every level.
3. **`add(v,v)` is produced by the canonicalizer, not by the front end.** The
   `i * 2` → `i + i` rewrite is what hides the multiply from every down-stream
   ring check. Either that canonicalization should preserve the ring
   (it does) *and* consumers should treat `add(v,v)` as a scaling instruction
   (they now do, §4), or the canonicalization should be inverted for address
   consumers. A unit test asserting `Add(v,v)` keeps the operand's type is cheap
   insurance; S27 added the consumer half.
4. **The same ring question for other address linearisations.** `build_gep_fold_map`
   already threads `ty.size() <= 4` into `foldable_const_disp` and documents the
   `+0xFFFFFFFC` case, so the constant-displacement path is covered; the vector
   and intrinsic memory paths (`is_used_as_address`) have not been audited for
   the same obligation.
5. **§6.7 byte-walk unroll** (`docs/SESSION_FOLLOWUP_S21_LOOP_ROTATION.md`):
   unchanged and still the next real performance work — the oracle table above
   (ICX vectorising, GCC/lccc level at ~1 cycle/byte) is the input to it.
6. **Pre-existing F2 deserves a blast-radius note in their evidence set**: the
   defect is not reachable in the 55-program corpus (the corpus A/B in
   `engineering/evidence/2026-10-06-ivsr-domain/` reports 0 changed TUs for the
   refactor), which is precisely why the *differential* corpus, not the static
   one, is what caught it.
