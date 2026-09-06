# FOLLOWUP 2026-09-06 — MachInst Window Register Allocator (SSA-driven)

**Task ID**: machinst-window-allocator (session 2026-09-06)
**Scope**: x86-64 MachInst layer quality lift — multiple magnitudes, per the
"make MachInst even more powerful" directive.

## 1. What was delivered

### 1.1 The window register allocator (`machinst_alloc.rs`, NEW, ~700 lines + tests)

The MachInst layer was a well-tested *pattern library* but not a machine-IR
backend: `MachReg::Vreg` operands (SSA value IDs of main-RA-spilled values)
could only resolve to stack-slot memory operands, and **one spilled value in
a register-only position (two-address ALU dest, `Lea`/`SetCC`/`Movzx` dest,
memory base/index) replayed the ENTIRE buffered window through the
accumulator text path** — the "replay cliff". The dead scaffolding proved
the original intent was an allocator (`MACHINST_ALLOCATABLE_GPRS`,
`collect_vregs_in_inst`, `fold_spill_relays` — all never called; the
peephole was additionally unsound-if-wired: folding a store→reload pair
drops the store without proving the slot has no later reader).

The new allocator runs at flush, between resolution and emission:

1. **classify** (pre-resolution): vregs in register-only positions or memory
   bases → window-register class; the resolver skips substituting exactly
   those (mixing a slot-substituted read with a register-held def of the
   same SSA value in one window would read a stale slot image).
2. **resolve** memory-form positions to stack slots (unchanged, optimal).
3. **allocate** by live interval with interference against
   (a) physical-register operands inside the interval,
   (b) **whole-window-busy registers** — main-RA homes live across the
   window but never referenced inside it (derived from RA assignments ×
   liveness segments — the class operand-level interference cannot see),
   (c) implicit clobbers (`rax`/`rdx` of the division forms; `Raw` blocks
   the whole pool),
   (d) prior assignments.
4. **rewrite** + insert reloads (arriving values, before first reference)
   and stores (window-written values that still have IR uses after the
   window), both at the value's own width, clamped to the slot width.

The SSA contract is *checked*, not assumed: two pure writes, a read before
the local def, XMM-domain or float-typed vregs, slotless values, alloca
values, and vreg shift amounts all refuse the window → the replay remains
the fail-safe, now the exception it was meant to be. `rbp` is excluded from
the scratch pool (frame base); caller-saved registers are preferred.

Supporting integration: `Codegen.machine_reg_busy` (reg → liveness spans)
is derived in the prologue from the RA result × the same liveness the
allocator used (hole-aware `segments` preferred over fat `intervals`).

### 1.2 `LeaSlot` — the typed replacement for the AllocaAddr `Raw` hack

`Mov { src: AllocaAddr(id), dst }` used to resolve into a hand-formatted
`Raw("    leaq off(%rbp), %reg")` text string: untyped, untestable, and a
`Raw` escape hatch conservatively blocks the window allocator's entire
scratch pool. The new `MachInst::LeaSlot { slot, dst }` is frame-mode-aware
in the emitter (rbp and rsp addressing like every `StackSlot` operand),
golden-tested, and keeps the allocator's def/use scan exact.

### 1.3 The loop-gate raise (32 → 4096) — hot loops return to MachInst

The 32-instruction loop-body gate existed because ONE spilled value in a
register-only position replayed the whole loop body (the gzip ~3%
regression class) plus two forced-loop conformance failures. With the
window allocator the cliff is gone; the two historical failures
(`ra09_selfop_xor`, `range_check_fold`) now pass with the gate unlimited,
and the full 638-file regression suite (GCC oracle + A/B differentials) is
green at the new default: **PASS=630 FAIL=0 SKIP=15, 0 AB-diff failures**.

Two genuine codegen regressions the gate raise exposed were root-caused
and fixed rather than papered over (see 1.4/1.5). Post-fix focused A/B
(15 reps, paired, CPU-pinned): mandelbrot **+7.2% → −0.03%**, bitops
**+6.4% → +0.7%**, sieve −4.2%, loop_patterns −3.9%, spectral_norm −2.4%,
arith_loop +1.3%, total **−0.22%**. Known watch item: `expat_xml_scan`
+6.6% with *shorter* code (208 vs 217 instructions) — likely loop-layout
sensitivity (back-edge enters `.LBB45`; the added preheader reload is once
per entry); needs hardware counters on the real 14700KF to close.

### 1.4 isel: ADD vs LEA selection when the RA coalesced `dst == base`

MachInst's `Add` → `Lea` rule fired unconditionally, degenerating to
2-register LEA (`leaq (%rbx,%rsi,1), %rbx`) when the RA coalesced the dest
with the base — where the two-address `addq` is 1 uop on 4 ports vs LEA's
2 (p15) on every Intel P-core and Zen. Cost: +6.4% bitops, +7.2% mandelbrot
in A/B. Now: `dst == base` (Phys) selects the ADD form (with the memory-
operand folding a spilled RHS gets for free); otherwise LEA still replaces
the `mov+add` pair.

### 1.5 emitter: FMov reg-reg copy form

`vmovsd s, s, d` (scalar VEX 3-operand) lost the rename-time move
elimination the mature path's `movapd` gets — +7.2% in mandelbrot's float
loop. FMov reg-reg now emits `vmovapd`/`vmovaps` (VEX packed, full-register
semantics — still no merging false-dependence). Matches GCC's copy form.

### 1.6 Release-sound immediate staging (Cmp/Test/Alu/Imul3)

The `%rax`-staging of wide immediates was guarded only by
`debug_assert!` (`assert_scratch_free`): in release, a staged immediate
could clobber a same-instruction rax operand silently, and TWO wide
immediates in one `Cmp`/`Test` compared `%rax` with itself — an
always-equal compare that assembles perfectly. The emitter now
stages lhs→rax, rhs→rcx (and single stagings divert to rcx when the other
operand is rax), the mem-to-mem relay avoids a base register it must
re-read, and Alu/Imul3 wide-imm staging never clobbers the accumulation
register. The debug assert is retired; the invariants are now structural.

### 1.7 Shift-amount resolution hole closed

`resolve_stack_vregs` used to rewrite a Vreg shift amount to a StackSlot —
which the emitter then **silently ignored** (it emits `%cl` for any
non-Imm amount): a dropped shift count. Unreachable from the isel (which
stages counts into `%rcx`), but one defensive caller away from a silent
miscompile. The resolver no longer rewrites amounts; a Vreg amount trips
the unresolvable gate and the allocator refuses it — both paths replay
correctly.

### 1.8 Emitter width discipline (5 defects found by the extended corpus)

The randomized corpus was extended from 10 to 16 instruction families
(Imul3, Neg/Not, Lea full cross-product, Cmov all condition codes, Movzx/
Movsx widening matrices, FMov/FAlu XMM shapes) with **wide immediates**
(i64::MIN/MAX, imm32-window edges, the historical bug constant) and the
pre-colored scratch registers. It immediately found:

1. `Movzx` dst named at `to_size` → `movzbl %al, %bl` (assembler reject).
2. `Movsx` incomplete (from,to) table → `movslq src, %eax` (size mismatch).
3. `Movzx` S64→S64 identity → `movl %rax, %ebp` (size mismatch) — now `movq`.
4. `Cmov` S8 → `cmovneb` (no 8-bit form exists) — promoted to the 32-bit
   form (identical low bits, zero-extended).
5. `Imul3` S8 / S16-with-wide-imm → unencodable — widened to S32 (same
   discipline as the two-address Alu `imul` arm).

All five are latent-bug classes the isel never triggers today — exactly
the "(op, width) pair no hand-written golden had instantiated" failure
mode this suite exists to catch. Narrowing movz/movs now traps loudly.

### 1.9 Execution differential, layer two

New runtime probes (assemble + link + execute + Rust-computed reference):
`Imul3` (incl. the imm32-window edge), `Neg`/`Not`, `Lea`
(base+index×scale+offset), `Div` (the rax:rdx discipline: quotient checked,
including `i64::MIN/2`), `Cmov` (operand order — the trap no assembler can
catch), `ShiftX` (BMI2 3-operand form), `Mov128` (16-byte transfer
bit-exactness), `FAlu` (all four ops at F64, including the
non-commutative Sub/Div operand orders), and `LeaSlot` golden shapes
(both frame modes). Every probe skips loudly (never vacuously) without a
toolchain.

### 1.10 Toolchain: pin removed everywhere

`rust-toolchain.toml` now pins the **channel `stable`** (was: version
1.98.0 — already stale: stable is 1.98.1). All five scripts that
duplicated the literal (`build_lccc_fast.sh`, `build_lccc_o1_j2.sh`,
`arena_session_restore.sh`, `lccc-bootstrap.sh`,
`bisect_boot_size.sh`) install/select `stable`, and the policy is recorded
in `scripts/README.md` and `engineering/DECISIONS.md`: stable-only
regressions are fixed at the source. Two such regressions were fixed
(clippy `filter_next` in `split_ranges.rs`; the 68-file rustfmt drift from
stable's formatter). The tree is warning-clean and clippy-clean
(`-D warnings`) and rustfmt-clean against stable 1.98.1.

## 2. Evidence (data-driven)

| metric | before | after |
|---|---|---|
| unit tests (`cargo test --lib`) | 1984 pass | **2000 pass** (16 new) |
| regression suite (638 files, GCC oracle + A/B) | 633/0/7* | **630/0/15** (same set; 15 = i686 runner unavailability, not code) — with the loop gate UNLIMITED |
| MachInst pipeline instructions (census, -O2) | 14880/30170 | **25729/52026** (+73% of instruction volume; 71 more files produce census) |
| whole-window replays (MI-FALLBACK) | 7 windows / 13 IR insts | 7 / 13 — all slotless phi-copy class (allocator correctly declines; text path owns them) |
| loop-gate A/B (15-rep paired) | gate 32 | total **−0.22%**; bitops +6.4→+0.7; mandelbrot +7.2→−0.03; sieve −4.2; loop_patterns −3.9 |
| clippy (stable, -D warnings) | 2 errors | **0** |
| rustfmt (stable) | 68 files drifted | **clean** |

\* last recorded historical run (FOLLOWUP-2026-09-05d).

Benchmark JSON evidence: `/tmp/ab3_g32.json` vs `/tmp/ab3_gopen.json`
(focused 13-benchmark paired A/B, current binary, 15 reps each) and
`/tmp/bench_gate32.json` vs `/tmp/bench_gateopen.json` (full-suite first
pass). Long (>1s) benchmarks on this VM are noise-dominated (hash_table
swung +24% between two identical-config runs) — treat only the focused,
back-to-back numbers as signal, per the benchmark lab's own screening
discipline.

## 3. Design decisions worth recording

* **Memory-operand substitution stays optimal**: vregs in memory-form
  positions keep the folded stack-slot access (1 µop load+op fusion). The
  allocator only fires for the register-only class — deliberately not
  "registerize everything".
* **Per-vreg uniformity**: a vreg is either slot-substituted everywhere in
  the window or register-homed everywhere — never mixed (a def in a
  register would leave the slot image stale for a substituted later read).
* **Store-back requires a window write**: arriving read-only values are
  never stored (the slot already holds them); only pure writes or RMWs
  that still have IR uses after the window drain the scratch to the slot.
  Use counts come from the function-level `value_use_counts`; a missing
  count is conservatively live-out.
* **`rbp` excluded from the scratch pool** unconditionally — one less
  frame-mode invariant to track in the allocator.
* **Replay is the fail-safe, not the cliff**: every refusal class
  (domain, discipline, pool exhaustion) falls back to the mature path,
  which is correct for everything.

## 4. Follow-up backlog (ranked)

1. **Slotless phi-copy values (the last 7 replay windows)**: the
   `Copy dest=V, src=V` chains whose source never materialized (no slot,
   no register). The emit-side gate misses `Copy` sources; the text path
   materializes them. Route: admit them at the gate or materialize at
   flush.
2. **`expat_xml_scan` +6.6%**: shorter code, likely layout (loop-header
   alignment / uop-cache set shifts). Needs `perf` on the 14700KF
   (branch-misses + frontend stalls first). If layout: alignment
   experiments on the two loop headers.
3. **Allocator XMM bank**: XMM-domain vregs still replay. The design
   (reload/store via `movq` slot↔xmm, interference from Phys xmm
   operands) mirrors the GPR work; unlocks slot-homed float
   Load/Store/Copy on the typed path (the remaining `Store(float)`
   rejections).
4. **`FCmp` migration** (`Cmp(float)` — 12–36 rejections): `ucomiss/d`
   + parity-aware setcc/jcc; needs the xmm0/xmm1 scratch representation
   the XMM allocator item would own anyway.
5. **Loop-gate → remove entirely** once 1–2 are closed and the kernel-boot
   gate has run (boot-code size budget re-check).
6. **`machine_reg_busy` refinement**: whole-window granularity today; a
   point-granular span check (intersect the vreg interval with each busy
   span) would free registers for windows inside holes.
7. **Select lowering** (dead `lower_select`): the emit-side blanket
   refusal can now be revisited — the rax-relay hazard it feared is the
   same class 1.6 made structural.
8. **Mov128 in the random corpus** + execution probe of the GPR fallback
   (four-S64 form) for i128 paths.

## 5. Reproduction

```sh
# unit tests (2000) — includes the 9 allocator unit tests, layer-two
# execution differentials, and the 16-family randomized corpus
cargo test --profile fastbuild --lib

# full regression (630 PASS, GCC oracle + A/B differentials)
./scripts/run_regression_suite.sh

# census (25729/52026 with the raised loop gate)
CCC_BIN=target/fastbuild/lccc OPT=-O2 JOBS=2 python3 scripts/isel_census.py

# loop-gate A/B
CCC_MI_MAX_LOOP_INSTS=32     python3 tests/benchmark/run_benchmarks.py \
    --compilers lccc --reps 15 --only bitops,mandelbrot,sieve,... --json g32.json
CCC_MI_MAX_LOOP_INSTS=999999 python3 tests/benchmark/run_benchmarks.py ... --json gopen.json

# window-allocator tracing on a spill-pressure kernel
CCC_MI_FN_FORCE=pressure CCC_MI_STREAM=1 CCC_MI_DEBUG=1 \
    target/fastbuild/lccc -O2 -S probe.c -o probe.s
```
