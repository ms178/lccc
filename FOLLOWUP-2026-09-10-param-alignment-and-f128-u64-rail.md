# FOLLOWUP-2026-09-10-param-alignment-and-f128-u64-rail.md

Closes the two outstanding defects from the i686 prologue audit
(`FOLLOWUP-2026-09-10-i686-prologue-fastcall-regparm-audit.md`): **Gap 1**
(over-aligned parameter allocas) and **Gap 2** (x86-64 legacy `%rax` head in
the F128 -> U64 cast). Two additional pre-existing defects found and fixed on
the way (i686 peephole `ret $N` misclassification; see also the CI-red
section below, which fixes one defect the PR itself introduced plus three
codegen traps it exposed). All changes re-based onto origin/main
**28379879** (Merge PR #483), which re-integrates the S07 merge
(PR #482) plus PRs #477-#479.

Every claim below was reproduced or refuted empirically. Oracles:
GCC 14.2.0-19, Clang 23.1.2 (apt.llvm.org), ICX 2023.2.4 and classic ICC
2021.10.0 (Intel oneAPI apt repo). GCC 16.2 could not be installed: Debian
trixie/experimental has no `gcc-16` package yet (documented in the installer
log); GCC 14.2.0-19 is the newest stable GCC on this host.

## Gap 1 — over-aligned parameter allocas

**Defect.** `typedef int A32 __attribute__((aligned(32))); int f(A32 x){ &x }`
must observe `&x` 32-aligned (the parameter OBJECT carries the declared
alignment; C11 6.2.8p3 / GCC semantics: the callee homes the incoming value
at the declared alignment, dynamically realigning `%esp` when > 16). lccc
instead stored and reloaded the parameter from a 4-aligned slot, so `&x`
violated the declared alignment — silent UB for any user relying on it.

**Reproduction.** `ovalign3.c/ovalign3_sink.c/ovalign3_drv.c` (cross-TU
escape), `ovalign4.c` (+sink/drv, checks `&x % 32 == 0`), `ovalign5.c`
(in-TU `void *volatile` escape): GCC -m32 prints `78`/rc 0; the pre-fix
lccc printed -1/rc 1. Escape is essential: without it the alloca is folded
away (`ovalign2_l.s`).

**Design.** Two regimes, mirroring GCC's split:

* `align <= 16` — static homing at the ABI-inherited alignment (frame
  already satisfies it; GCC-identical, zero cost).
* `align > 16` — the alloca slot is padded by `align - 1` and everything
  routes through the *effective* aligned address (`lea base(%ebp),%ecx;
  add $31; and $-32`):

  * the prologue capture stores the incoming argument **directly at the
    effective address** — the capture IS the homing store;
  * `&x` and every alloca load/store use the effective address
    (`alloca_over_align` / `emit_alloca_aligned_addr_impl` in all four
    backends);
  * the ParamRef materialization loads from the effective address;
  * the separate `store %p, %a` homing store and the ParamRef copy are
    **elided** when the ParamRef dest's only use is that store
    (Phase 7.5 census in `stack_layout/mod.rs`; `param_homing_stores` /
    `dead_param_ref_dests` in `state.rs`; checks in `emit_store_impl` and
    `emit_param_ref_impl`, i686 + x86-64).

Result: capture-as-single-home. Disassembly of
`h(A32 x){ void *volatile p=&x; sink_ptr_rw(p); return x; }` is ONE aligned
store plus reads straight from the effective address — strictly fewer memory
ops than GCC, which captures to a temporary and copies into a realigned home.

**Frontend.** `ParamDecl` gained `alignment` / `alignas_type`
(`ast.rs`); `parse_param_list` now captures pre-name GCC attributes, parses
POST-name attributes (`int x __attribute__((aligned(32)))` — previously
silently skipped), and merges `_Alignas` via `.take()` (no cross-parameter
leakage). `build_ir_params` folds the declared alignment into `param_align`
(scalars), `struct_align` (struct/union/vector params), and the `_Float128`
carrier branch (previously hardcoded `struct_align: Some(16)`, silently
dropping `aligned(32)` on `_Float128` params). lccc ACCEPTS direct
`aligned`/`_Alignas` parameter spellings that GCC and Clang reject
("alignment may not be specified for 'x'"), so honoring them is mandatory —
accepting-but-ignoring would be silent UB.

**Guards added.** `slot_assignment.rs`: the ParamRef-dest/alloca slot share
is now refused for over-aligned allocas (the dest's accesses would use the
raw base while the alloca's use the padded effective address). x86-64
`emit_param_ref_impl` loads over-aligned param allocas via the effective
address (was raw `slot_ref`); i686 already did via
`emit_param_slot_load`.

**Validation.** `pa1_matrix.c` (24 cases: alignment x type x ABI-shape,
mutation/escape, register interaction), `pa2_direct.c` (9 direct-spelling
cases), `pa3_conv.c` (fastcall/regparm(1..3)/stdcall x over-aligned
scalars and structs): all PASS on lccc-i686 and lccc-x86-64, 20-run
determinism (1 unique summary). Oracle standings for over-aligned
parameters (ovalign6g + pa1): GCC 14.2 PASS, Clang 23.1.2 PASS, ICX
2023.2.4 PASS, **classic ICC FAILS at -O0, -O2 and -O2 -align-all**
(`&x` misaligned for every typedef-aligned parameter — ICC drops parameter
alignment entirely), lccc PASS. lccc is strictly stronger than ICC here.

## Gap 2 — x86-64 legacy `%rax` head in F128 -> U64

`emit_f128_to_u64_cast` opened with `subq $16; movq %rax,(%rsp);
fldl (%rsp)` — a legacy rail that treats the low 8 bytes of an F128 as an
f64 and was only reachable from `emit_f128_to_int_cast`, which itself is
only called when `emit_f128_to_int` (cast_ops.rs) fails to intercept a cast.
A 16-byte F128 can never ride the `%rax` accumulator, so the rail was
structurally wrong — a latent miscompile for any shape that reached it.

**Empirical rail proof.** An emission marker was inserted at the head and
the compiler was run over a 400-file corpus (repo tests + scratch harnesses)
on BOTH x86-64 and i686: **0 hits in 800 compilations** (plus the S07
battery: f128u64.c, x64f128.c, battery1.c).

**Fix.** The legacy functions (`emit_f128_to_u64_cast`,
`emit_f128_to_int_cast`, `emit_fisttp_from_f64_via_stack`) are deleted; the
call site in `emit_cast_instrs_x86` now **fails closed** with a descriptive
panic instead of silently emitting the wrong sequence. Every real
F128 -> int conversion flows through `emit_f128_to_int`'s dispatch
(memory / f128-source / constant) into the canonical x87 ST0 path
(`emit_f128_st0_to_int`, the S07 0x403e threshold implementation).

**Validation.** `fc_matrix.c` (bit-exact truncations, boundary values,
precision-control invariance at PC 24/53/64, exact F128 -> F64/F32 narrows,
U64 -> F128 -> U64 round trip): **all eight toolchain/mode combinations
PASS** (lccc-i686, lccc-x86-64, GCC -m32/-m64, Clang -m32/-m64, ICX,
ICC). The long-double PC matrix remains the discriminator: f128u64.c
(lccc-i686 ALL OK; **GCC -m32 FAILS PC53 case 8**, got 0 want 2^64-1) and
x64f128.c (lccc-x86-64 ALL OK; **GCC x86-64 FAILS pc=1 case=2**) — lccc is
strictly stronger than GCC where precision control meets exact payloads.

Matrix note: `(_Float128)N.N` casts are NOT valid test vectors — the
unsuffixed literal is parsed as double and ROUNDS first
(`(_Float128)9223372036854775807.0` is 2^63, not 2^63-1; GCC, Clang, ICX,
ICC and lccc all agree on this). The matrix therefore uses exact `f128`
(GCC/lccc) / `q` (Clang/ICX/ICC) literal suffixes.

## New defect found and fixed — i686 peephole `ret $N`

`pa3_conv.c` fastcall case f2 returned garbage (a value left in `%ecx` at
`ret $8`). Root cause: `classify_line` recognized only the bare `ret` as
`LineKind::Ret`; `ret $N` (fastcall/stdcall/sret callee-pop) fell through as
a generic non-barrier line, so `eliminate_dead_reg_moves` scanned PAST the
`ret $8`, saw no read of `%eax`, and **deleted the return-value move**
(`movl %ecx, %eax`) immediately before the return. Reproduced in isolation:
`peephole_optimize("movl %ecx,%eax\nret $8")` returns just `ret $8`.

Fix: new `LineKind::RetN` classification for `ret $N`, wired into all 11
consumers (is_barrier, slot invalidation, CFG liveness in/out, edx
return-value liveness, dead-reg-move Ret arm, ret-observes census,
guarded-label scan, block-terminator scan). Tail-call conversion is
deliberately NOT given RetN (its suppression "falls out naturally" and
remains intact). Regression test added in the peephole test module.

## Test assets

`/home/user/scratch/paramalign/` — pa1_matrix.c (+pa_sink.c), pa2_direct.c,
pa3_conv.c, fc_matrix.c (self-checking, unique negative failure codes,
PASS/FAIL summary, 20-run determinism verified). `/home/user/scratch/prologue/`
— ovalign3-6 chains incl. the GCC-clean typedef oracle ovalign6g.c.

## Validation summary (all on the re-based tree, post-CI-fix)

* fastbuild, `-D warnings`: clean (2m38s).
* `cargo test --lib`: 2308 passed, 0 failed.
* `scripts/ci_local.sh --fast`: 16/16 gates green (incl. the
  benchmark-output oracle gate, run explicitly: green).
* `CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py --lccc
  target/fastbuild/lccc -j 2`: **729 passed, 0 failed** — the exact CI-red
  step, green (3 consecutive full runs; `fabsf128` and
  `f128_global_carrier` individually re-verified).
* `cargo fmt --all`: clean.
* Matrices: pa1/pa2/pa3/fc + ovalign chains: all green on lccc-i686 and
  lccc-x86-64; oracle tables above.
* New: `ovalign4` green at ALL opt levels (default/-O0/-O1/-O2/-O3/-Os/-Oz)
  on lccc-x86-64 — the shape that exposed the typed-call-arg trap.
* New: arm (aarch64) + riscv64 under qemu — `ovalign6g` and the LD32
  capture probe green at ALL opt levels, outputs ≡ cross-GCC. AArch64
  regression suite: identical pass/fail/skip counts vs the base compiler
  (514/53/127 — zero regressions).
* New: over-aligned-param value+alignment probes (`livepref.c`,
  `livepref2.c`, `one.c`, `cap_o2.c`) green on lccc-x86-64/-i686/-arm/-riscv
  at every opt level and ≡ GCC; arm/riscv disassembly verified: captures
  write the effective address (x11/t2 bases), reads resolve it, no pad
  access.
* Oracle anomaly recorded: GCC 14.2 at -O1 (and default) FAILS the
  over-aligned long-double param case of `ovalign6g.c` (prints -1000);
  lccc is correct at every level. GCC on riscv64/aarch64 does not honor
  typedef-aligned scalar params at all (alignment checks fail); lccc now
  honors them on all four backends.
* Benchmark output oracle gate: 180/180 PASS.

## CI red — root cause and fixes (PR #484)

**Symptom.** GitHub job 103013467965 (PR #484), "Run regression corpus
with SSA validation" (`CCC_VALIDATE_SSA=1 … run_regression.py -j 2`):
`FAIL fabsf128`, `f128_global_carrier: 3 check(s) failed`, exit 3. The SSA
validator itself was innocent: it exercised the FULL corpus at -O2, the
configuration the local gates skipped, and the failures were plain runtime
miscompiles.

**Root cause (one line).** `emit_capture_store_movdqu` in
`src/backend/x86/codegen/prologue.rs` (new in this PR) used the LOAD-form
emitter `emit_instr_rbp_reg` ("movdqu off(%rbp), %xmmN") in its Raw arm
instead of the STORE-form `emit_instr_reg_rbp`. The 16-byte `_Float128`
parameter capture became a garbage load: the incoming register is clobbered
and the parameter slot is never written. At -O2, mem2reg removes the
IR-level homing store, making the prologue capture the sole writer of the
parameter slot — every parameter read returns uninitialized memory. This is
why the lower-opt matrices (address-taken params, IR homing store still
present) passed. A/B disassembly against the base compiler pinned the
delta exactly: base emits `movdqu %xmm0, -16(%rbp)`; the PR emitted two
loads and no store. Exhaustive audit of every emitter added/rewritten in
the PR (x86-64/i686 prologue + memory paths, arm/riscv diffs) confirmed
this was the ONLY polarity error. Fix restores the exact pre-PR
instruction; zero performance cost. Regression test:
`tests/regression/f128_param_capture_o2.c` (red on the broken build, green
after the fix).

**Latent traps exposed by the same audit, fixed in the same PR (all
x86-64 typed-MachInst admission):**

1. *Typed call arguments for over-aligned allocas* (`arg_src` in emit.rs):
   `TypedCallSrc::AllocaAddr` resolved to a raw-slot `LeaSlot` while the
   capture writes the EFFECTIVE align_up'd address — `sinkp32(&x)` received
   the pad base. Live miscompile, maskable by frame-layout luck (that is
   why `ovalign4` only failed at -O1/default). Fix: over-aligned allocas
   fall back to the mature path (the established OverAligned-rejection
   doctrine), which computes the effective address.
2. *Typed ParamRef materialization of a dead destination*: the census
   looked only at `value_use_counts == 0`, not at `dead_param_ref_dests`
   (the homing-elision set) — a dest whose sole use was the elided homing
   store still materialized a raw-slot load of the alignment pad. Fix:
   mirror the text path's early-out.
3. *Typed ParamRef "Case 1" raw-slot load for over-aligned param allocas*:
   the `_alloca_id` tuple element existed but was unused; a live ParamRef
   of an over-aligned param would have loaded the pad. Fix: reject to the
   text path (effective-address load), same doctrine.

**Cross-backend audit fallout (arm/riscv), fixed in the same PR:**

4. *aarch64 SIGSEGV on over-aligned long-double params*:
   `f128_softfloat.rs` Path-2 OverAligned arm computed the effective
   address into x9 but loaded through x17 (the mask scratch) — the
   `f128_move_aligned_to_addr_reg` bridge (`mov x17, x9`) was missing at
   one of five sites. Base compiler does not crash; the PR's param
   over-align plumbing made this path reachable. Fix: add the bridge.
5. *arm/riscv prologue captures ignored the effective address*: with the
   PR, over-aligned scalar parameters now get padded slots on all four
   backends, but the arm/riscv captures still wrote the raw slot while all
   readers resolve the effective address — deterministic silent value
   corruption on riscv64 (LD32 probe printed 5 instead of 82) and
   frame-luck corruption on aarch64. Fix: new `emit_param_home_store_impl`
   in both backends; the capture loops compute the effective base once per
   parameter (arm: x11, riscv: t2) and store through it; the arm/riscv
   `emit_param_ref_impl` alloca-load paths now route over-aligned param
   allocas through the effective address (the former `_alloca_id`
   placeholders). All store sites covered (GP/FP/F128/stack/by-ref
   classes). Verified by qemu runtime + disassembly at all opt levels,
   outputs ≡ cross-GCC.

## Follow-ups

* ICC classic: no 32-bit runtime (ia32_lin not shipped in the 2023.2.4
  oneAPI package), so ICC -m32 evidence is limited to compile-only; ICC
  additionally fails the over-aligned-parameter semantics outright.
* GCC 16.2: unavailable on Debian trixie (no gcc-16 package in stable or
  experimental); re-run the oracle tables when it lands.
* GCC -O1 over-aligned long-double param defect (ovalign6g case 5) is an
  oracle anomaly, not a lccc defect; re-check against a future GCC point
  release.
