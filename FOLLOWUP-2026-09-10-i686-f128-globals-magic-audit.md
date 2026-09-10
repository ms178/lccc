# Follow-up: i686 F128 / globals / magic-division audit — adjudication of the Astra report and the perfected revision

Date: 2026-09-10 · Base: ms178/lccc @ 4721e8b (PR #478; includes the merged addressing-encoder series)

## 1. Scope

Red-team adjudication of the Astra audit of `float_ops.rs`, `globals.rs`
and `magic_div.rs` (`src/backend/i686/codegen/`), plus a full audit of the
related infrastructure those three files depend on: the F128 x87 stack
model (`emit_f128_load_to_x87`, `emit_float_binop`, the cast paths, the
memory store paths, call-result handling), the address-fold gates in
`generation.rs` (`is_foldable_mem_ty`, `rip_rel_blocked`,
`supports_global_addr_fold`), the register cache (`RegCache`), the
allocator's F128 slot policy (16-byte slots), and the PIC `%ebx`
reservation.

## 2. Adjudication of the Astra report

| # | Astra finding | Verdict | Why |
|---|---|---|---|
| 1 | F128 negation without a destination slot leaves the result and an extra entry on the x87 stack; repetitions overflow it | **Agree — and the mechanism is now proven, not assumed.** `fstpt` is GAS's store-AND-pop form (`db 38`, FSTP m80; the non-popping `fstt` is not even accepted by GAS 2.44). Every F128 op balances exactly when its store is emitted; the missing-slot branch is the one place the store is skipped. **Disagree with the remedy (panic).** A destination without a slot is a *dead result* under this backend's own convention (`store_eax_to` and `emit_f64_store_from_x87` treat a missing home the same way), and dead values are legal (-O0 codegen). A panic would crash the compiler on valid input. Fixed instead by resolving the slot *before any emission* and emitting nothing at all for a dead result: `fldt`/`fchs` have no side effects, so the dead case produces no code, pushes nothing, and is structurally incapable of unbalancing the x87 stack. |
| 2 | `fstpt` stores 80-bit x87 extended, not IEEE binary128; the name F128 proves no representation | **Agree.** Verified in-tree: i686 F128 is fully x87-backed (all producers/consumers use `fldt`/`fstpt`; `f128_direct_slots` marks x87-format slots; consts go through `f128_bytes_to_x87_bytes`; values live in 16-byte slots that `fstpt` fills 10 bytes of). Negation stays `fldt/fchs/fstpt` — `fchs` is defined on the x87 format itself, so no representation conversion or bit-flip assumption is involved. The contract is now documented in-file. |
| 3 | PIC prohibition on the absolute folds was documentation-only; a faulty caller could silently emit unsuitable absolute accesses | **Agree.** The upstream gates exist and work (`supports_global_addr_fold = !pic_mode`, `rip_rel_blocked` excludes GOT/TLS/absolute symbols, `is_foldable_mem_ty` excludes F128 and i128), but the failure mode of a leaked fold is a silently wrong address. Added release `assert!`s in both fold impls (second line of defense; one check per folded instruction). |
| 4 | The generic global-fold path would not transfer a full F128 value | **Agree.** Same release `assert!(ty != IrType::F128)` in both fold impls, on top of the `is_foldable_mem_ty` gate. |
| 5 | `%eax` missing from the byte/word partial-register store table | **Agree.** The fallback was correct (an eax-homed value costs nothing to stage: `operand_to_eax` is a no-op for it), but the table should be complete. Adopted the full mapping (`al/ax` for eax; esi/edi/ebp/sp for 16-bit; no sil/dil/bpl/spl — those need REX), with unit tests. |
| 6 | Global/label/TLS addresses always formed in `%eax` despite the existing direct-destination convention | **Agree.** Adopted the `dest_reg` direct path for all three, with the cache handled precisely: a direct write to `%eax` refreshes the accumulator cache; a write to any other register leaves it untouched (the cache only tracks `%eax`/`%edx`), instead of the blanket `invalidate_acc()`. Verified safe in every mode: in PIC+GOT functions the allocator reserves `%ebx` (it enters the clobber list), so the destination can never be the GOT base; `function_needs_got` guarantees the base exists whenever any GOTOFF form is emitted. |
| 7 | Magic division: no arithmetic error visible; preconditions were debug-only | **Agree, extended.** Preconditions are now release `assert!`s — and so are the *postconditions* (shift ranges): the emitter shifts by `s` / `s - 1` with no clamping, so an out-of-range shift would encode a masked immediate and silently miscompile. Cost is one check per compile-time divisor. |
| 8 | Documentation: ambiguous add-formula parenthesization; unverifiable godbolt-verification claim | **Agree.** Formula rewritten to `q = (hi + ((n - hi) >> 1)) >> (shift - 1)` (matches the emitter's `leal (%edx,%ecx)` shape); the "verified against the godbolt oracle" phrasing replaced with an honest statement: the pinned constants are regression pins for the immediates GNU/LLVM/ICX emit (all derive them from Hacker's Delight), evidence of parameter correctness, not of codegen superiority. |
| 9 | Tests: few divisors, no upper ranges, no `i32::MIN` in the original signed test | **Agree.** Adopted the deterministic property suite: quotient-transition probes, small exhaustive rectangles (2..=255 divisors), full-width divisors 2..=4096, near-powers-of-two and near-max divisor regions, 4096 deterministic full-width samples, and release-visible rejection tests. Suite runs in ~10 ms (fastbuild profile). |

### Findings beyond Astra (this session's own audit)

1. **The x87 balance invariant is exact and testable.** Since `fstpt`
   pops, the *only* imbalance sources are conditional stores. Beyond the
   audited `float_ops.rs`, three more sites had the identical defect and
   were fixed:
   - the F128 binop in `emit_float_binop` (two loads, `f{p}` pops one —
     a dead destination leaked one entry per op);
   - the three integer/float → F128 cast paths in `casts.rs`;
   - the F128 call-result store in `calls.rs`, where a dead destination
     must still pop the ABI-returned `st(0)` (the callee leaves the value
     on the stack in every case) — fixed with `fstp %st(0)`;
   - the F128 memory-store paths in `memory.rs` pop the orphan when the
     pointer address cannot be resolved (previously silent leak).
2. **A real pre-existing miscompile hazard in the F128 store path:**
   `memory.rs` loaded the stored value *before* resolving the pointer, so
   a pointer homed in `%eax` died when a constant (or any operand staged
   through `%eax`) was loaded — `fstpt (%eax)` then wrote through the
   bit pattern of the constant. The 64-bit pair path below it already
   stashed such a base into `%ecx`; the F128 paths now do the same
   (pointer-first resolution, stash before the value load).
3. **Cache hygiene on the pair paths:** the absolute 64-bit load/store
   folds clobber *both* halves of the accumulator pair, so both
   `invalidate_acc()` and `invalidate_sec()` are now issued (the old code
   invalidated only acc; i686 has no `sec_has` readers today, but the
   invariant is cheap to keep honest).
4. **Verified empirically:** a long double chain/cast program compiled
   with `lccc-i686 -O2` prints byte-identical results to `gcc -m32 -O2`
   (including 1e19-scale values), and the emitted F128 sequences are
   stack-balanced instruction for instruction; the global/label/TLS/
   load/store forms now match GCC's shapes exactly (`movl $extvar,%eax`
   vs `movl %eax,%reg`-staging eliminated; TLS LE = `movl %gs:0,%eax;
   addl $tv@NTPOFF,%eax`).

## 3. Files changed

| File | Change |
|---|---|
| `src/backend/i686/codegen/float_ops.rs` | Dead-destination contract: slot resolved before any emission; dead result emits nothing; representation contract documented in-file |
| `src/backend/i686/codegen/globals.rs` | Release asserts (PIC, F128) on both fold impls; direct `dest_reg` paths for global/label/TLS with precise cache handling; complete `i686_store_register` table incl. eax; pair paths invalidate acc+sec; 4 unit tests |
| `src/backend/i686/codegen/magic_div.rs` | Release asserts for pre- and postconditions (shift ranges); unambiguous formulas; honest verification claims; 19-test property suite |
| `src/backend/i686/codegen/emit.rs` | F128 binop: dead-destination early skip |
| `src/backend/i686/codegen/casts.rs` | int/float → F128 casts: dead-destination early skip (3 paths) |
| `src/backend/i686/codegen/memory.rs` | F128 stores: pointer-first resolution with the `%eax`-homed base stash into `%ecx`; orphan pop when the address cannot be resolved (2 sites) |
| `src/backend/i686/codegen/calls.rs` | F128 call result: dead destination pops the ABI `st(0)` |

## 4. Validation record

- 2283/2283 lib tests (fastbuild), 23 new tests (19 magic_div, 4 globals).
- Magic suite also run under `--release`: all pass, incl. the rejection
  tests (release asserts fire as designed).
- CI-exact test gate (`CARGO_BUILD_JOBS=2 RUSTFLAGS="-D warnings" cargo
  test --all-targets`): green. Clippy (`-- -D warnings`): green. rustfmt:
  clean.
- `ci_local.sh --fast`: 16/16 gates green. Full `ci_local.sh`: **19/19
  gates green, 0 failed, 0 skipped** (build, toolchain selector, CI-exact
  test gate with `RUSTFLAGS=-D warnings`, regression corpus with SSA
  validation, benchmark-output oracle, differential correctness, loop
  alignment, fuzz wiring/smoke, inline-asm UTF-8, recip codegen,
  MachInst window, i686 atomics, i686 asm-diff, cross-backend atomics,
  linker fuzzes, codegen-quality gate, rustfmt, clippy).
- F128 runtime differential vs `gcc -m32 -O2`: identical output.
- F128 regression corpus (7 tests): 7/7 pass.

## 5. Remaining follow-ups (recorded, not hidden)

1. **F128 codegen quality (pre-existing, not introduced here):** every
   F128 operation round-trips through stack slots (`fldt X; fstpt Y` even
   for copies), producing the long load/store chains visible in §2. GCC
   keeps live values on the x87 stack. A slot-to-slot copy elision /
   x87-stack value cache (the F64 path has one: `x87_pending`) is the
   right next step, with `-Os` sizes in mind.
2. **F128 semantics beyond the x87 subset:** the backend is x87-backed
   (64-bit significand), while `_Float128` promises binary128. This is an
   upstream ABI decision; the audit only preserves it.
3. **`lstore` frame bounce:** the 64-bit store fold stages the pair from
   the parameter slots through an extra slot round trip before
   `movl %eax,lv; movl %edx,lv+4`; a direct slot→acc/edx pair load at
   the fold would drop the bounce (candidate for the pair-load fold).
4. **Astra's release gates 2–5:** full PIC/TLS link tests (shared-object
   initial-exec round trip), F128 ±0/∞/NaN round trips, and hardware
   measurement were not run in this harness; the differential and
   regression coverage above is the stand-in.

---

# Part 2: casts.rs audit — adjudication of Astra v1/v2 and the perfected revision (2026-09-10, session 2)

## 7. Adjudication of the Astra casts audits

Astra produced two candidate designs: v1-final (retain SSE fast paths +
representation-based unsigned handling + `fisttp`) and v2 (all-x87,
control-word save/restore, no SSE3 assumption). Both audits claimed the
same defect list. **Every valid-input defect was reproduced empirically
before fixing** on the current tree (differential battery vs `gcc -m32`,
1263 lines, i686 runtime):

| # | Claim | Verdict |
|---|---|---|
| 1 | `fisttpq` (signed) used for F64/F128 → U64: `[2^63, 2^64)` mishandled | **Confirmed.** `3·2^62 → 0x8000000000000000` (indefinite) + FE_INVALID, GCC gives `0xc000000000000000`. Same for F128 (2^63+1, 2^64-1). |
| 2 | signed `cvttss2si` used for F32 → U32 | **Confirmed.** `3221225472.0f → 0x80000000` + FE_INVALID (GCC: `0xc0000000`, no exception). |
| 3 | U8→I8 / U16→I16 register no-ops break the canonical form | **Confirmed structurally** (the scalar emitter emitted nothing). Runtime-visible only via canonical-form consumers, which normalize at use — fixed anyway (one `movsbl`/`movswl`; the canonical contract is now airtight for future consumers). |
| 4 | I8→U16 sign-extends only (0xffffffff instead of 0x0000ffff) | **Confirmed structurally; fixed** (re-canonicalize the destination width). |
| 5 | U64→float correction (`fadds 2^64`) depends on the ambient x87 PC | **Confirmed, with a twist:** at PC=53 the current code and GCC lose bits identically (parity, not divergence). Replaced anyway with the exact native-encoding load (`fldt`, exponent word 0x403e = bias+63): strictly better than GCC under every PC, ~12% faster, and no addition at all. |
| 6 | Unconditional `fisttp` needs SSE3 | **Factually true, but moot in-tree:** the i686 backend already emits SSE/SSE2/SSE3 unconditionally in ~230 sites (intrinsics alone: 183) and has no target-feature gating interface. v2's all-x87 rewrite would have been *inconsistent* with the backend's own floor, slower, and control-word traffic on top. The real gap — target-feature gating — is recorded as a follow-up (item 2 below). |
| 7 | Missing F128 dest slot leaks an x87 entry | **Confirmed** (fstpt = store-and-pop, verified via GAS). Resolution per session 1: dead-result skip, not a panic — empirically `dead_neg` compiles to a bare `ret` like GCC. Applied to all F128-dest cast paths. |
| 8 | Wide identity casts fall through scalar code | **Confirmed live:** `(f128)3.999L` identity hit the eax path; `(int)(f128)3.999L` printed 0 in the battery. F64/F128 identity now pair-copy / slot-copy. |
| 9 | Cache invalidation inconsistent across paths | **Audited per-path.** New code invalidates exactly the registers each sequence writes (acc for eax, sec for edx, none for pure-x87/push-pop paths that leave eax untouched) — more cache hits, not fewer. |
| 10 | `fstpt` = 10-byte store into a 12-byte ABI object | **Confirmed** (FSTP m80); documented; identity copies move the full 12-byte object including padding. |

### v2's retraction of the globals direct-register paths — REJECTED, with proof

v2 withdrew the `dest_reg` direct paths for global/label/TLS addresses for
lack of repo access ("whether %ebx is excluded from destinations in PIC
mode" etc.). With the repository in hand:

1. `function_needs_got` reserves `%ebx` (pushes it into the allocator's
   clobber list) for *every* function that can emit a GOT reference, and
   `%ebx` is never in the caller-saved pool — so `dest_reg` can never be
   the GOT base in a GOT-live function, and no GOT form is ever emitted
   in a function where `%ebx` is allocatable.
2. A live PIC link+run test was executed: a 32-bit shared object compiled
   with `lccc-i686 -O2 -fPIC` (global addr via `sym@GOT`, local via
   `GOTOFF`, TLS via `GOTNTPOFF`, PLT call), linked with `ld`, `dlopen`ed
   from a 32-bit host, functions called, TLS verified across a thread —
   all results correct.
3. That test exposed and fixed a real assembler defect: `@GOTNTPOFF`
   emitted `R_386_TLS_IE`, which forces `DT_TEXTREL` in shared objects
   (GAS emits `R_386_TLS_GOTIE`). Mapper, README, and unit test updated.

## 8. Defects found beyond Astra (this session, all reproduced first)

1. **Constant-fold collapse of float→int cast chains** (lowering, not
   codegen): `(uint32_t)(int)3.999L` folded to **0**. Root cause:
   `eval_const_cast` re-derived source bits by walking the expression;
   `irconst_to_bits` on a float leaf falls back to `to_i64().unwrap_or(0)`.
   Fix: prefer the evaluated value's own bits. Verified by the battery and
   a new regression (`tests/regression/casts_roundtrip.c`).
2. **Assembler silently drops trailing `+k` after a base+disp operand**:
   `movl %edx, 128(%esp)+8` assembled as `128(%esp)` (both operand roles).
   GAS rejects the form; the parser now rejects it identically (with a
   regression test), and the new codegen emits proper `slot_ref_offset`
   displacement strings. This bug was first surfaced by the new F128 GP
   copy/neg paths and would have mis-assembled any user code written that
   way.
3. **F32 intermediate-rounding policy divergence** (pre-existing,
   recorded, not fixed): `(u32)(f + 3e9f)` differs from GCC by one ulp on
   ~25% of inputs because lccc rounds the f32 add to storage before the
   cast while GCC keeps x87 extended precision. Cast semantics are exact
   given their input; the divergence is in f32 arithmetic storage policy.
4. **GP sign-flip F128 negation rejected on measurement**: the sequence
   microbenchmark favoured it 2.6x (3.24 vs 8.50 ns), but end-to-end it
   lost ~35% (17.0 vs 24.8 ns) — every F128 consumer reloads with `fldt`,
   and three partial `movl` stores defeat store-to-load forwarding where a
   single `fstpt` forwards cleanly. Kept the x87 path; documented in-file.

## 9. Design decisions with measured numbers (Xeon @2.6 GHz, 32-bit, CLOCK_MONOTONIC)

Sequence-level microbenchmarks (isolated inline-asm, 3e6 iterations):

| Sequence | ns/op | Choice |
|---|---|---|
| x87→u64: plain fisttpq (old, wrong) | 10.65 | — |
| x87→u64: fucomip-branch (new) | **10.42** | **adopted** |
| x87→u64: GCC-style CW save/fistpq/restore | 10.25 | rejected (CW traffic, slower in context) |
| x87→u64: fstpt+cmpw (Astra v1-final) | 12.69 | rejected |
| u64→x87: fildq+fadds 2^64 (old/GCC) | 10.04 | — |
| u64→x87: fildq/fldt(0x403e) (new) | **8.81** | **adopted** (exact at every PC) |
| f128 neg: x87 fldt/fchs/fstpt | 8.50 | **adopted** (see §8.4) |
| f128 neg: GP sign-flip copy | 3.24 | rejected end-to-end (§8.4) |
| f128 copy: push/pop (Astra) | 3.01 | rejected |
| f128 copy: 3×movl | **2.56** | **adopted** |

End-to-end conversion workloads (lccc-i686 -O2, volatile inputs, vs the
pre-audit compiler and gcc -m32 -O2): d→u64 parity (11.5 ns), u64→long
double parity-or-faster (13.4 vs 13.5), f32→u32 +12% (2.9 vs 2.6 — the
correctness fix's only real cost), long double→u64 +4% (15.8 vs 15.2),
f128 neg parity after the revert (17.0 vs 16.5), identity copy parity.
GCC remains 2–5x faster on most conversions because it chains values in
registers/x87 while this backend round-trips every value through stack
slots — the slot-bounce optimization remains the biggest lever (item 1
below), not the cast sequences.

## 10. Validation record (session 2)

- 2294/2294 lib tests (fastbuild) incl. 9 new casts.rs unit tests, the
  parser rejection test, and the updated TLS-relocation test.
- CI-exact gate `CARGO_BUILD_JOBS=2 RUSTFLAGS="-D warnings" cargo test
  --all-targets`: green. Clippy `-- -D warnings`: green. rustfmt clean.
- 1263-line differential cast battery vs `gcc -m32` (all cast families,
  256-pattern sweeps, boundary values, all four rounding modes, PC=53,
  exception flags, NaN payloads, deep stack stress): identical except the
  four intended T17 lines (lccc exact under PC=53 where GCC loses bits).
- PIC shared-object link+run: dlopen, global/label/TLS addr, PLT call,
  cross-thread TLS — all correct, no DT_TEXTREL.
- New regression `tests/regression/casts_roundtrip.c` (host corpus) and
  `tests/asm-diff/i686/casts.casefile` (encoder oracle).
- Full `ci_local.sh`: **19/19 gates green, 0 failed, 0 skipped** (build,
  toolchain selector, CI-exact `-D warnings` test gate, regression corpus
  with SSA validation, benchmark-output oracle, differential correctness,
  loop alignment, fuzz smoke, inline-asm UTF-8, recip codegen, MachInst
  window, i686 atomics, i686 asm-diff (now incl. casts.casefile),
  cross-backend atomics, linker fuzzes, codegen-quality gate, rustfmt,
  clippy).

## 11. Remaining follow-ups (recorded, not hidden)

1. **Slot-bounce elimination for F128/F64 chains** (pre-existing): every
   F128 op round-trips through slots; a deferred x87-stack value cache
   (like the existing F64 `x87_pending`) is the highest-leverage codegen
   win, ~2-5x on conversion chains.
2. **Target-feature gating for the i686 ISA floor**: the backend emits
   SSE2/SSE3 unconditionally (documented in casts.rs); a real
   `-march`/feature interface is needed before "generic i686" claims are
   meaningful.
3. **TLS model selection** (upstream responsibility): PIC code uses
   initial-exec unconditionally; general-dynamic for shared libraries
   (GCC's default) remains unimplemented.
4. **F32 storage-rounding policy** (§8.3): decide between GCC's extended
   intermediate precision and storage-rounded semantics.
5. **Internal assembler hardening**: the trailing-`+k` parse is fixed;
   audit the remaining GAS-divergence classes for silent mis-assembly.
