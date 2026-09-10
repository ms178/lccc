# FOLLOWUP-2026-09-10-param-alignment-and-f128-u64-rail.md

Closes the two outstanding defects from the i686 prologue audit
(`FOLLOWUP-2026-09-10-i686-prologue-fastcall-regparm-audit.md`): **Gap 1**
(over-aligned parameter allocas) and **Gap 2** (x86-64 legacy `%rax` head in
the F128 -> U64 cast). One additional pre-existing defect found and fixed on
the way (i686 peephole `ret $N` misclassification). All changes re-based onto
origin/main **28379879** (Merge PR #483), which re-integrates the S07 merge
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

## Validation summary (all on the re-based tree)

* fastbuild, `-D warnings`: clean (2m38s).
* `cargo test --lib`: 2307 passed, 0 failed.
* `scripts/ci_local.sh --fast`: 16/16 gates green.
* `cargo fmt --all`: clean.
* Matrices: pa1/pa2/pa3/fc + ovalign chains: all green on lccc-i686 and
  lccc-x86-64; oracle tables above.

## Follow-ups

* ARM/RISC-V ParamRef routing for over-aligned param allocas still uses
  the placeholder `_alloca_id` (no over-aligned effective-address load yet);
  same fix shape as x86-64 once ARM/RISC-V runtime oracles are available
  (qemu-user installed with the oracle toolchain).
* ICC classic: no 32-bit runtime (ia32_lin not shipped in the 2023.2.4
  oneAPI package), so ICC -m32 evidence is limited to compile-only; ICC
  additionally fails the over-aligned-parameter semantics outright.
* GCC 16.2: unavailable on Debian trixie (no gcc-16 package in stable or
  experimental); re-run the oracle tables when it lands.
