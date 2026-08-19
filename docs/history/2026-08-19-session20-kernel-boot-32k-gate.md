# 2026-08-19 (session 20) — kernel boot: exact 32 KiB gate arithmetic + register-pressure evidence

**Base:** `origin/main` @ `b03191f` (Merge PR #135 — FPO alignment fix)
**Snapshot:** `/home/user/ms178-1.patch`

## Environment restore (harness wiped everything again)

swap 6G → /swapfile; rustup 1.97.1 stable minimal under **/opt/rust**
(RUSTUP_HOME=/opt/rust/rustup, CARGO_HOME=/opt/rust/cargo — keeps ~15k
toolchain files out of the 10k-file workspace snapshot cap);
`cargo build --profile fastbuild --locked -j2` (1m37s clean);
apt: cmake ninja clang-19 lld-19 flex bison bc gcc-multilib libc6-dev-i386
libelf-dev. Kernel re-extracted: linux-6.18.44 + all 26 patches from
ms178/archpkgbuilds (PKGBUILD order, `patch -Np1 -s`, zero rejects), cachymod
`config` + `make olddefconfig` + `make prepare`. Boot stubs: cpustr.h via
mkcpustr (mkcapflags.sh OUT cpufeatures vmxfeatures — arg order!), stub
zoffset.h honoring `ZO_efi32_stub_entry == ZO_efi64_stub_entry - 0x200`
(CONFIG_EFI_MIXED=y), utsversion.h stub.

## The EXACT 32 KiB gate arithmetic (corrected; supersedes session 12's budget)

lccc-ld links setup.ld and reports the asserts faithfully. Baseline layout
(23 objects, -m16 -Os; object text total **33445**, data 216, bss 4915):

```
.bstext..​.initdata  0x000–0x4a0   (1184 B fixed: header/entry/init)
.text                0x4a0–0x7d96  (30966 B)
.text32              +0x1e
.pecompat            ALIGN 4096 → 0x8000   (588 B padding lost)
.rodata 0x554 + .videocards 0x54 + .data 0x8c + .signature
setup_size = ALIGN(0x8650, 4096) = 0x9000 > 0x8000  → "at most 64 sectors" FAILS
.bss 0x1364 (4964 B: main.o 4108 = boot_params-adjacent statics)
_end = 0x99c0 (39360) > 0x8000                    → "Setup too big!" FAILS
```

The `_end` gate is BINDING, and session 12's ".text ≤ ~27449" budget only
satisfied `setup_size`, NOT `_end`: with pecompat @ 0x7000 the post-signature
position is ~0x7650 and + bss(0x1364) lands at ~0x89b4 > 0x8000. Correct
budget: pecompat must fit at **0x6000** →

```
0x4a0 + text_total + 0x1e ≤ 0x6000  →  text_total ≤ 23330 bytes
```

Target: **33445 → ≤ 23330 = −10115 bytes (−30.2 %)**. GCC's corpus text is
12846 (fits pecompat @ 0x4000, _end ≈ 0x5a70). bss/rodata/data are fixed by
kernel source; the WHOLE gap is C codegen.

## Where the bytes go (measured, 19-file corpus -Os)

lccc 30846 B vs gcc 12846 B (2.40×). Instruction census (lccc vs gcc):
esp-relative slot refs **2171 vs 373**, mov* 3431 vs 1190, pushl 258 vs 175,
popl 386 vs 147. Slot-ref decomposition: `movl %eax,SLOT` 668,
`movl SLOT,%eax` 422, `movl SLOT,%reg≠eax` 243, `movl %reg,SLOT` 177,
movb/w 159, leal-slot 192, folded non-mov refs 197. Consecutive RMW-collapse
opportunities: only ~20 left (the peephole already takes the rest) — the
remaining traffic is STRUCTURAL: values whose home is a slot because the
allocator gave them none.

CCC_DEBUG_RA_INTERVALS (new this session, regalloc.rs) proves it in
`__cmdline_find_option`: 111 values, 9 in registers; ten function-spanning
values (v5/v6/v34/v70/v82..v85/v88/v103..v106, intervals ~[16..43,127]) fight
over 4 callee-saved + 2 caller-saved homes; switch-state phi webs (v103–v106,
four overlapping copies of the same C variable) each get their own slot.

## Allocator knob sweep (corpus text bytes, -Os)

| config | bytes | Δ |
|---|---:|---:|
| baseline | 31966 | — |
| CCC_NO_PHI_COALESCE=1 | 32286 | +320 (phi coalescing helps) |
| CCC_NO_COALESCE=1 | 32034 | +68 (copy coalescing helps) |
| CCC_CALLER_SAVE_SPANNING=1 | 31773 | −193 **but UNSOUND on i686** |
| CCC_NO_LOOP_PIN=1 | 31966 | 0 |

**CCC_CALLER_SAVE_SPANNING hazard (found this session):** Phase 2b assigns
call-spanning values to caller-saved regs and returns `caller_save_spans`;
the x86-64 backend consumes them (spill slot + save/restore around spanned
calls, prologue.rs:939) but the i686 prologue DISCARDS them
(`_caller_save_spans`), so on i686 the −193 "win" is the size of the MISSING
save/restore — the values are clobbered across calls. Must NOT be enabled on
i686 until the emitter grows span save/restore. Documented, not shipped.

## Baseline validation (all on this machine, fastbuild binary)

- 922/922 unit tests
- 50/50 correctness
- regression --compare-gcc: 347 passed + 6 known gcc-14-oracle mismatches
  (builtin_cpu_supports_raptor, code16_realmode_encoding, fp_domain_crossing,
  has_attribute_in_code, kernel_flags_and_builtins, sqrt_vex_scalar) —
  byte-identical set to upstream session 19
- m32 differential fuzz: (running)

## Shipped this session

- `regalloc.rs`: CCC_DEBUG_RA_INTERVALS dump (value, interval, callspan,
  use-count, home) — compiler explainability (§57); zero codegen change.

## Ranked levers for the −10 KB (evidence-backed, updated)

1. **CFG-aware coalescing of loop phi webs** — v103..v106-class overlapping
   copies of ONE C variable each take a slot; graph-coloring-style web
   merging (or SSA-level copy propagation through phis) collapses N slots
   to 1. Biggest single structural lever; touches regalloc + (carefully)
   the deferred-slot machinery.
2. **%eax allocatable** (+1 register, all caller-saved quality): every
   accumulator path (operand_to_eax/store_eax_to/casts/calls) must tolerate
   %eax live. Prototype behind a flag; measure.
3. **i686 caller-save-span save/restore** (makes Phase 2b sound): bound the
   gain at ≤ −193 corpus-wide; implement only after #1/#2.
4. Leaf-param ABI homes on i686 (x86-64 has CCC_NO_LEAF_PARAM_GPR; i686
   prologue captures params to callee-saved/slot homes today).

## Session 20 continued — landed fixes (each fully validated)

### S02: redundant `testl` elimination (−57 B boot text)
`andl`/`orl`/`xorl %R,%R` set exactly the flags `testl %R,%R` would produce
(ZF/SF/PF from result, CF=OF=0). A directly following self-test is pure
overhead → deleted (2 B/site, 29 sites in the corpus: cpucheck's
`andl $536870912,%eax; testl %eax,%eax; je` etc.). Flag contract verified
for the sign-flag consumer too (`js`-style). Gated regression check:
`check_redundant_test_elimination.sh`.

### S03: missing `#include` in `.S` is now fatal (GCC parity, correctness)
The assembly preprocess paths (both `run_preprocess_only` and the `-c`
builtin path in `external_tools.rs`) dropped preprocessor errors on the
floor: `arch/x86/boot/header.S` compiled happily against an EMPTY
`voffset.h`, silently evaluating the `VO_*` `#if` guards to 0. Both paths
now emit the diagnostics and fail. Regression:
`check_asm_missing_include_fatal.sh`. Kernel stubs documented:
`arch/x86/boot/{zoffset.h,voffset.h}` must exist (stubs provided in the
kernel-work tree; values are immediates → size-neutral).

### S05/S06: CRITICAL — unary RMW classification fix (silent miscompile)
`classify_line` gave single-operand unary RMWs (`notl %eax`, `negl %eax`,
`incl %eax`, `decl %eax`) `dest_reg = REG_NONE` (no comma → parse_dest_reg
fails). `propagate_reg_copies` then let a `movl %edx,%eax` alias SURVIVE
`notl %eax` and rewrote the consumer back to the pre-not register:
`v0 = ~v0` returned the OLD v0 — at every -O level, because the PEEPHOLE
(not codegen) introduced it (`LCCC_NO_PEEPHOLE=1` hid it). Pre-existing on
upstream `53b4254` (PR #136 head), confirmed by clean-worktree build.
Fix: unary RMWs on a register operand now classify with their register as
dest (memory operands keep REG_NONE). Found by the NEW differential fuzzer
`tests/fuzz/slot_rmw_differential.py` (address-escaped locals + RMW shapes;
the m32 fuzz never covered unary ops): 126 mismatches → 0 (750/750 cases).
Pinning regression: `check_unary_rmw_copy_propagation.sh`.

### S05 also: slot-RMW collapse peephole (−22 B corpus)
`movl S,%R; OP %R; movl %R,S` → `OP S` when %R is provably dead before its
next read (forward scan; barrier = label/jump/call/ret/push/directive;
`popl %R` counts as a pure def, `popl %other` is skipped). Soundness hinge
is the dead-register proof — the corpus's 8 surviving sites all keep %eax
live after the store (result reused), so they correctly do NOT collapse;
the fired sites are the dead-result shapes (printf/video counters).

### Reverted experiment (documented, §69): leaf-param relax + Cast-clean
Two allocator changes intended to put params/long-lived values in %ecx/%edx:
(i) lift `param_restricted` for call-free i686 functions, (ii) mark ≤32-bit
int/ptr Casts clean in the scratch-hazard scan. A/B matrix (corpus bytes):
baseline 31966 / leaf-only 32013 (+47) / cast-only 32063 (+97) / both
32154 (+188). Mechanism: register homes whose CONSUMERS stage through %eax
pay a home→%eax relay that outweighs the saved slot traffic (check_cpu:
`movl err_flags,%edx; movl %edx,%eax`; frame +24 B elsewhere). **Reverted
completely.** Lesson: home selection must be consumer-shape-aware; this is
the design core of the remaining −10 KB (§ "Remaining levers").

### Re-base on PR #136 (S04)
Upstream moved `b03191f → 53b4254` ("Preserve volatile memory semantics").
Session commits rebased cleanly; full re-validation green: 931 unit, 50/50
correctness, 350 + 6 known-gcc14 regression, boot corpus base now 32007
(+98 B from PR #136's own volatile conservatism), boot objects 33492.

### Gate status after session 20 (final)
boot object text **33450** (33492 baseline: −57 redundant-test already in,
−42 directive-aware slot-RMW collapse). Budget ≤ **23330**. Gap
**≈ −10.1 KB (−30 %)**. GCC corpus reference 12846 (fits pecompat @ 0x4000).

Note: the slot-RMW collapse initially only fired without `-g` because `.loc`
directives broke the load/op/store adjacency; the final version skips
NOP/Directive lines between the three instructions and in the dead-register
forward scan (labels still block — single-block invariant preserved).

## Validation ledger (session 20 final)
- unit: 931/931 (fastbuild, -D warnings clean)
- correctness: 50/50
- regression --compare-gcc: 350 passed + 6 known gcc-14 oracle mismatches
  (byte-identical set to session 19)
- m32 differential fuzz: 900/900 (seeds 0:300 × O0/O2/Os)
- slot_rmw differential fuzz (NEW): 750/750 (seeds 0:250 × O0/O2/Os)
- boot build pipeline: 23 objects compile, lccc-ld evaluates setup.ld
  asserts faithfully ("at most 64 sectors" = the live gate)

## Remaining levers (updated evidence, ranked)
1. **Consumer-shape-aware home selection** — the reverted experiment proved
   the win exists but needs the emitter's consumer shape in the cost model
   (register home only pays if the consumer reads the register directly:
   memory-folded cmp, direct-dest ALU, register-source binop). Design:
   classify each value's consumers at allocation time; gate ecx/edx homes
   on ≥1 register-friendly consumer; else keep the acc-cache flow.
2. **CFG-aware phi-web coalescing** — switch-state webs (cpucheck
   `__cmdline_find_option` v103–v106 class) each take a slot; merging
   copies of ONE C variable collapses N slots to 1. Needs the deferred-slot
   machinery respected (session 16 root cause).
3. **%eax allocatable** — +1 register; every accumulator path must tolerate
   %eax live. Biggest structural change; prototype behind a flag.
4. **Load ecx-hazard refinement** — Load is ecx-dirty unless ptr is a direct
   alloca; when the ptr is register-resident at emit time no %ecx staging
   happens. Two-pass allocation (ptrs first, hazards recomputed) could
   unlock ecx across loops. Soundness hinges on allocation-aware staging.

---
**Session 21 executed this roadmap** — see
`2026-08-19-session21-lever-implementation.md`: L1 shipped (−592 B boot),
L4 shipped (−200 B corpus), L2 machinery shipped (0 delta, root cause
documented), L3 ceiling proven (~0.8 KB bound; deferred with evidence).
Gate now: boot text 32 858 vs budget 23 330.
