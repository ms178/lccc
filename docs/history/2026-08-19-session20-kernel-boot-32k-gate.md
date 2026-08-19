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
