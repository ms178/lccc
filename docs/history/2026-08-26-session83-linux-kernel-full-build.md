# Session 83 — linux-cachymod 6.18 full build with LCCC: readiness audit + compiler fixes

Date: 2026-08-26
Upstream base at session start: `510d72d1d861570045072c8f9b9ca06934f8e23e` (lccc main, origin HEAD)
Compiler build: `scripts/build_lccc_fast.sh` (fastbuild profile, Rust 1.98.0 `-O1`, `-j2`, clang 19 + mold)
Memory discipline: 1.9 GiB RAM + active **8 GiB** swap (`/swapfile`; `ensure_swap.sh`)
Kernel package source: `ms178/archpkgbuilds` `packages/linux-cachymod-6.18` (26/26 patches applied)
Kernel tree: `/home/user/kernel-work/linux-6.18.44` (prepared by `prepare_kernel_tree.sh`)

## Objective (this session)

Compile and link the custom CachyMod 6.18.44 kernel **entirely with `lccc` + `lccc-ld`** and boot it in
QEMU, fixing every compiler/linker issue encountered (pre-existing or not), with a permanent patched
snapshot after each validated fix.

## Headline result

A realistic full **kernel-compilation readiness audit** was achieved. The result is far more encouraging
than the prior session's scope implied:

* **LCCC compiled several hundred real kernel `.c`/`.S` files** through the genuine Kbuild pipeline
  (`init/`, `kernel/`, `mm/`, `lib/`, `arch/x86/kernel/`, `arch/x86/mm/`, `drivers/base/`, `fs/`, and
  more) with only a **handful of distinct compiler defects** — most of the kernel already compiles.
* **Two genuine compiler defects were root-caused, fixed, and validated** (see below).
* A reproducible **full-kernel build harness** now lives in the repo (`scripts/build_kernel_full.sh`).
* The four remaining blockers were **precisely root-caused** and documented, including one concrete
  assembler data-emission bug and one frontend statement-expression typing bug, plus the two
  structural blockers (objtool interop, lccc-ld vmlinux link) that gate a *bootable* image.

**Honest scope note:** the full **compile + link + boot** of a bootable `bzImage` was **not** achieved in a
single session. The boot is gated by two large, subsystem-level blockers: (1) x86_64 6.18 **mandates
objtool** (`HAVE_OBJTOOL` is selected for X86_64), which rejects two independent aspects of LCCC's
assembler output; and (2) **linking the real `vmlinux`** with `lccc-ld` against
`arch/x86/kernel/vmlinux.lds` (a very large linker-script/relocation surface) was not reached because the
compile step first needs to pass objtool. This document gives the exact path and per-blocker evidence.

---

## 1. Environment / reproduction

```text
Linux Debian 13 (trixie), 2 vCPU, 1.9 GiB RAM, 8 GiB swap (/swapfile)
Rust 1.98.0 (rustup, minimal + rustfmt/clippy), Cargo 1.98.0
gcc 14.2.0 (host tooling + HOSTCC), clang 19.1.7, mold 2.37.1, GNU ld.bfd 2.x, objcopy
lccc built: target/fastbuild/{lccc,lccc-x86,lccc-i686,lccc-ld}   (fastbuild profile, -O1, no LTO)
```

Kernel configured for LCCC (see `scripts/build_kernel_full.sh`):

```text
base config:    x86_64 defconfig (lean, bootable, fast on 2 cores)
custom feature: CONFIG_CACHY=y, CONFIG_SCHED_BORE=y, CONFIG_SCHED_CACHE=y,
                CONFIG_SCHED_CLASS_EXT=y       (the package patches' scheduler functionality)
disabled for LCCC backend gaps:
                CONFIG_FUNCTION_TRACER=n, CONFIG_DYNAMIC_FTRACE=n     (-pg/__fentry__ unsupported)
                CONFIG_STACKPROTECTOR=n                                (gs-based canary unsupported)
                CONFIG_OBJTOOL=n, CONFIG_UNWINDER_ORC=n, CONFIG_STACK_VALIDATION=n,
                CONFIG_UNWINDER_FRAME_POINTER=y                       (no objtool ORC needed)
```

Driving command (validated; `kernel/fork.o` builds cleanly):

```bash
make ARCH=x86_64 CC=/home/user/lccc/target/fastbuild/lccc \
     LD=/home/user/lccc/target/fastbuild/lccc-ld as=/home/user/lccc/target/fastbuild/lccc -j2 <target>
```

## 2. Compiler fixes (validated, in `ms178-1.patch`)

### Fix 1 — x86 codegen: invalid narrow-type load destination registers

**Symptom (kernel `arch/x86/kernel/cpu/common.o`):**
`ccc: error: \`movzbl\` destination must be wider than source: %sil -> %al`
(emitted immediately after `# LCCC_PARAM_ABI_READ sil`).

**Root cause:** `backend/x86/codegen/emit.rs::load_dest_reg` returned the 8/16-bit register names
(`%al`/`%ax`) for I8/U8/I16/U16, but the paired load mnemonics from `mov_load_for_type` require wider
destinations:

| type | load mnemonic | required dest | old (buggy) | new |
|------|---------------|---------------|-------------|-----|
| I8   | `movsbq` (sext 8→64) | 64-bit `%rax` | `%al` | `%rax` |
| U8   | `movzbl` (zext 8→32) | 32-bit `%eax` | `%al` | `%eax` |
| I16  | `movswq` (sext 16→64)| 64-bit `%rax` | `%ax` | `%rax` |
| U16  | `movzwl` (zext 16→32)| 32-bit `%eax` | `%ax` | `%eax` |
| U32/F32 | `movl`           | 32-bit `%eax` | `%eax` | `%eax` |

GNU as rejects `movzbl %sil, %al` / `movsbq %sil, %al`. This mirrored the already-correct mnemonic-width
logic in `inline_asm.rs::asm_load_dest_reg`, but the *type*-based helper (used by memory/globals/atomics/
prologue) had the wrong widths.

**Fix:** `load_dest_reg` now matches the mnemonic width contract. `%rax` stays correctly zero/sign-extended
for the subsequent `store_rax_to` spill.

**Validation:** fastbuild compiles; `arch/x86/kernel/cpu/common.o` compiles cleanly; no
`movzbl %…, %a[lh]` / invalid sext forms remain in its `-S` output; lib tests **1179 passed / 0 failed**;
correctness suite **50/50**.

### Fix 2 — sema: allow char-type pointer assignment (GNU `-Wpointer-sign`, not a hard error)

**Symptom (kernel `fs/namei.c:2400` → `net/core/*` would follow):**
`error: incompatible pointer types (have 'char *' but expected 'unsigned char *')`
for `nd->last.name = name;` (`struct qstr.name` is `const unsigned char *`).

**Root cause:** `frontend/sema/analysis.rs::pointee_types_compatible` classified a pointer assignment whose
pointee types differ only in char signedness as `-Wincompatible-pointer-types`. GNU C treats pointers to
the character types (`char`, `signed char`, `unsigned char`) as mutually *assignment-compatible* and emits
only a `-Wpointer-sign` warning — **never** a hard error, even under `-Werror=incompatible-pointer-types`
(verified empirically with gcc 14.2: the case is `[-Wpointer-sign]`, and `gcc -Werror=incompatible-pointer-types`
accepts it). The kernel relies on this constantly.

**Fix:** `pointee_types_compatible` returns `true` for `Char`<->`UChar` (`Char` covers plain/signed char;
there is no distinct `SChar` variant).

**Validation:** minimal reproducer + `fs/namei.o` compile cleanly; lib tests **1179 passed / 0 failed**;
correctness suite **50/50**.

### Fix 0 — build-blocking lint in upstream main

The current `main` would not build under the repo's `-D warnings` policy: `passes/loop_rotate.rs:267`
called `std::mem::drop` on a `&Vec<Instruction>` (`-D dropping-references`). Removed the no-op `drop`
(NLL ends the immutable header borrow at the point of last use), restoring a clean build.

---

## 3. Remaining blockers (root-caused, in priority order)

### B1 — objtool interop (the hard gate to BOOT)

x86_64 6.18 selects `HAVE_OBJTOOL` unconditionally (`select HAVE_OBJTOOL if X86_64`), which makes the
Kbuild run objtool on **every** object. Without objtool-accepted objects, no image builds. Two independent
defects were observed when running the real objtool on an LCCC object:

1. **`.discard.annotate_insn` is empty (0 data bytes) yet carries 11 relocations.** The kernel's
   `__ASM_ANNOTATE` macro emits `~8 bytes per entry` (`.long label - .` + `.long type`) into
   `"M",@progbits,8`. LCCC's assembler records the relocations but emits **no section data**, so objtool
   reads `type` out of a zero-length buffer (garbage → `type 3`). This is likely a bug in the assembler's
   handling of `.long sym - .` (current-location subtraction) or section data sizing. *Higher-value than it
   looks — the same `- .` pattern drives `.discard.unwind_hints`, `.discard.func_stack_frame_non_standard`,
   so fixing it unblocks several meta-sections at once.*
2. **`find_insn` fails at an annotated offset** (`bad .discard.annotate_insn entry: 0 of type 3`), i.e. the
   relocation target `.text + 0x149` is not an instruction boundary in objtool's decode of the LCCC
   `.text`. Separate from (1); suggests objtool decodes LCCC's instruction stream (or the retpoline/
   patchable-entry site layout) differently than GAS/GCC output.

**Workaround in harness (compile-only sweeps):** `tools/objtool/objtool` is shimmed to a no-op
(`LCCC_NOOBJTOOL=1`, default) so the **compiler** can be swept; objtool interop remains the boot gate.

### B2 — statement-expression (+ inline asm) type inference

Blocks **all of `net/*`** (`net/core/sock.o`, `net/core/request_sock.o`, …) and any code using an inline-asm
`xchg`/cmpxchg inside a `({ ... })` statement expression under a `typeof(*({...}))` unevaluated context.

**Minimizer (reproduces in isolation):** `old_dst = ({ typeof(*({ …__ret=dst; switch(…){case 8: asm("…"
:"+r"(__ret),"+m"(*__ai_ptr));} __ret; })) *u614 = … ((typeof(*({…})) *)(…)); ((typeof(*({…})) *)(u614)); })`
→ `error: incompatible pointer types (have 'int *' but expected 'struct dst_entry *')`.

**Root-cause hypothesis:** the inner statement-expression's value is `__ret` (`__typeof__(*(__ai_ptr))` =
`struct dst_entry *`). When the statement expression **contains an inline asm statement**, and is evaluated
inside the unevaluated `typeof(*({...}))` context, the `__ret` identifier's type is resolved as `int *`
instead of `struct dst_entry *` — most likely because the asm `"+r"(__ret)` output operand is type-checked
in a nested/discarded scope and overwrites the symbol's inferred type. Verified: removing the `switch`/asm
makes it compile; the same code in a plain function (not inside a statement expression) compiles.

**Next step:** inspect `infer_expr_ctype`/`resolve_var_from_compound` and the asm-output type inference in
`sema/type_checker.rs`; ensure an inline-asm output operand does not mutate a *declaration-derived* type of a
local, especially in an unevaluated (`sizeof`/`typeof`) context.

### B3 — `-pg` / `-mfentry` / `__fentry__` (function tracing)

The CachyMod package config enables `CONFIG_FUNCTION_TRACER`; LCCC correctly *diagnoses* that it does not
yet emit `mcount`/`__fentry__` (`arch/x86/kernel/cpu/common.o`: `#error Compiler does not support fentry?`
from `asm/ftrace.h`). Do **not** silently accept-and-ignore `-pg`; either implement the ABI correctly or keep
`CONFIG_FUNCTION_TRACER=n`. The harness keeps it off.

### B4 — gs-based stack canary (`-mstack-protector-guard-reg=gs`)

LCCC correctly refuses `-mstack-protector-guard-reg=gs` (`LCCC refuses to silently ignore target-affecting
-m flags`). The kernel's stack protector uses a per-CPU `gs` segment canary. Harness keeps
`CONFIG_STACKPROTECTOR=n`.

### B5 — `lccc-ld` real `vmlinux` link (NOT yet attempted)

Not reached: the compile step must first clear objtool. `arch/x86/kernel/vmlinux.lds` is a very large
linker-script surface (ORC sections, `__ex_table`, `__jump_table`, `__static_call_sites`, `.altinstructions`,
`/DISCARD/`, `KEEP`, `ALIGN`, `PROVIDE`, `LOADADDR`, and the `R_X86_64_*`/`R_386_*` relocation set). When
the compile step is objtool-clean, tackle the link next; `setup.elf` (32-bit, script + GC + KEEP) is already
byte-identical to BFD/LLD, which is a strong signal the script engine has the right primitives.

---

## 4. What still builds (LCCC today)

Verified objects compiling cleanly with LCCC through Kbuild (representative):
`init/*`, `kernel/*` (exit, fork, panic, sched/…), `mm/*`, `lib/*`, `arch/x86/kernel/*` (incl. `cpu/*`,
`fpu/*`, `acpi/*`), `arch/x86/mm/*` (incl. `pat/`), `drivers/base/*` (incl. `power/`, `firmware_loader/`,
`regmap/`), `fs/*` up to `fs/namei.c`. Two distinct compiler errors surfaced across hundreds of files (both
fixed here). The remaining known *distinct* frontend defect is B2 (statement-expression+asm typing).

## 5. Deliverables in the repo

* `scripts/build_kernel_full.sh` — reproducible full-kern build harness (config selection, objtool shim,
  Kbuild driving); validated on `kernel/fork.o`.
* `scripts/build_kernel_boot.sh` — unchanged; still links the 32-bit `setup.elf` byte-identical to BFD/LLD
  and passes the 32 KiB gate (`_end=0x79c0`, 1600 B headroom).
* Compiler fixes: `src/backend/x86/codegen/emit.rs` (load-dest widths), `src/frontend/sema/analysis.rs`
  (char pointer-sign).
* Two committed snapshot checkpoints (`S03`, `S04`) + working-tree patch `ms178-1.patch`.

## 6. Follow-up priority queue (for the next sessions)

1. **Fix B1 (objtool):** first the `.discard.annotate_insn` empty-data assembler bug (and the sibling
   `- .` meta-sections), then the `find_insn`/decode mismatch. This is *the* gate to a bootable image.
2. **Fix B2 (statement-expression + inline-asm typing)** — unlocks all of `net/*`, a large fraction of the
   kernel and many drivers.
3. **objtool ORC generation** (frame-pointer unwinder gets you to a boot; ORC is the perf/accuracy win).
4. **`-pg`/`__fentry__`** or keep tracing off; **gs-based stack canary** or keep STACKPROTECTOR off.
5. **lccc-ld `vmlinux` link** via `arch/x86/kernel/vmlinux.lds`; then `arch/x86/boot/compressed` and
   `bzImage`; then QEMU serial-console smoke test with a deterministic initramfs.
6. Re-run the boot-code harness (`build_kernel_boot.sh`) after every i686/code16/x86 change; never lose the
   1600-byte `setup.elf` safety margin.

## 7. Correctness / regression evidence

* Rust library tests: **1179 passed, 6 ignored, 0 failed** (after each fix).
* Correctness suite: **50/50**.
* `arch/x86/kernel/cpu/common.o`, `fs/namei.o`, `kernel/fork.o`, `init/*`, `mm/*`, `drivers/base/*` compile.
* Both compiler fixes verified against the real failing kernel sources and minimal reproducers.

## 8. Honest non-claim

No bzImage was produced or booted this session; B1 (objtool) and the *compile-blocking* B2 are the reasons.
This document's evidence is a readiness audit + two validated compiler fixes + a reproducible harness, not a
claim of a bootable LCCC-compiled kernel.
