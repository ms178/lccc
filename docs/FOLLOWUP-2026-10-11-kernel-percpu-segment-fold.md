# Linux 6.18.55 on lccc + lccc-ld: per-CPU address spaces, `-fshort-wchar`, and the segment-operand fold (2026-10-11)

This note records one session of the standing "build the kernel with only lccc/lccc-ld,
no GCC anywhere in the kernel or its boot code" programme. Base: `ms178/lccc` main
`6dd8eee` (this work was rebased from `1334bb8a` onto it; `git range-diff` reports both
patches apply verbatim, no fuzzing). Oracle: **gcc 14.2.0**, never as the kernel compiler.
Host: 2-core research VM, 2 GiB RAM, 8 GiB swap, `$target/fastbuild` profile.

Session commits: `eef924` (`.if` register spellings, from the previous session, rebased),
`e586959` (kernel-prep: the four blockers below), `d54ba8a` (the two code-quality fixes
in §2).

---

## 0. What shipped

| # | Finding | Layer | Status |
|---|---------|-------|--------|
| 1.1 | `__seg_gs`/`__seg_fs` space lost on `&object` | sema + symbol table | fixed, kernel-wide |
| 1.2 | `-fshort-wchar` ignored | driver + preprocessor + literals | fixed |
| 1.3 | `enum *` rejected where the compatible integer pointer is expected | sema lattice | fixed |
| 1.4 | `%gs:disp(base)` not folded for register bases | backend emitter | fixed |
| 2.1 | lea→memory-operand splice refused every segment-prefixed consumer | x86 peephole | fixed |
| 2.2 | scalar FP segment loads crossed the GPR domain | backend emitter | fixed |
| 3.1 | the new gate asserted `+32` for a `+40` field | test | fixed |

The two §2 items are not kernel blockers: they are the difference between lccc's per-CPU
code and GCC's for the same source. Both were found by comparing against GCC on the
shapes the kernel actually produces, and both are covered by tests in both halves
(textual shape + runtime behaviour against a real GS base).

---

## 1. The four kernel-build blockers

### 1.1 A named address space belongs to the object, not to the type

`__seg_gs`/`__seg_fs` (the kernel's `__percpu`) is a property of the *storage*. GCC
measured behaviour:

```c
static __seg_gs int v;          /* type of &v is  __seg_gs int *        */
static __seg_gs int *p;         /* type of &p is  __seg_gs int **       */
```

so a declaration-level space is absorbed by the declarator's **pointee** when the
declarator has a pointer level, and otherwise belongs to the object itself.
`type_builder::apply_declaration_address_space` now reports which of the two happened
(returns `bool`), and sema records the unabsorbed space in the new
`Symbol::address_space`; `lvalue_address_space`/`pointee_address_space`
(`src/frontend/sema/type_checker.rs`) give `&E` the storage space of what `E`
designates, mirroring the lowerer's `get_addr_space_of_struct_expr`.

Symptom without the fix: `__tasklet_schedule_common(t, &tasklet_vec, TASKLET_SOFTIRQ)`
(kernel/softirq.c) rejected as an address-space mismatch, and — if the space had simply
been stamped on the type instead — `include/linux/percpu-refcount.h` rejected with <!-- dl-skip: kernel-tree path, resolved via KERNEL_DIR -->
`have '__seg_gs __seg_gs unsigned long **'`.

Reproducers: `tests/regression/seg_gs_percpu_address_of.c`,
`tests/regression/seg_gs_address_of_percpu.c` (both `-O2`, both byte-identical stdout to
GCC), plus the unit test `object_space_is_absorbed_by_pointer_declarators`
(`src/common/type_builder.rs`).

### 1.2 `-fshort-wchar`

The kernel builds with `-fshort-wchar` (its `wchar_t` must match EFI's 16-bit `CHAR16`;
`include/linux/efi.h` passes `L"SecureBoot"` to a function taking `unsigned short *`). <!-- dl-skip: kernel-tree path, resolved via KERNEL_DIR -->
lccc accepted the flag and ignored it. Now: the driver records it
(`set_short_wchar`), `wchar_ctype()`/`__WCHAR_TYPE__`/`__SIZEOF_WCHAR_T__`/
`__WCHAR_MAX__`/`__WCHAR_MIN__`/`__WCHAR_WIDTH__` follow GCC (5 macros change 4→2 bytes
or the signedness), and wide literals retarget at parse time.

Reproducers: `tests/regression/short_wchar_16bit_literals.c`,
`tests/regression/short_wchar_wide_literals.c` (and the matrix
`wchar_short.c`/`wsz.c`/`w16.c`/`w32.c` in the session scratch: GCC and lccc
byte-identical where both accept).

### 1.3 An enumeration is compatible with its underlying integer

C11 6.7.2.2p4: each enum is compatible with `char`, a signed or an unsigned integer
type. GCC's choice on x86-64 is `unsigned int` when no enumerator is negative, `int`
when one is, and a 64-bit type otherwise — this is *not* pedantry, because pointers to
compatible types are compatible. `arch/x86/mm/pat/set_memory.c` passes
`enum pg_level level` to `lookup_address(addr, unsigned int *level)`; GCC accepts it
under `-Werror=incompatible-pointer-types`, lccc stopped at `set_memory.o`.

Fix: `EnumType::compatible_ctype()` (`src/common/types.rs`) + `enum_matches_integer()`
in `pointee_compat` (`src/frontend/sema/analysis.rs`). Reproducer `/tmp/enumptr.c`
reduced from the kernel error; unit test
`enum_is_compatible_with_its_underlying_integer_pointer`.

### 1.4 `%gs:` loads with a constant displacement fold into the operand

Kernel per-CPU field access is `base + constant`, where `base` is `per_cpu_ptr()`'s
register-held result. GCC folds the displacement into the segment operand; lccc
materialised `base + off` into a second GPR first.

```
struct S __seg_gs *getp(void);
long f(void) { return getp()->b; }

GCC:   call getp@PLT            lccc (before):  call getp@PLT
       movq %gs:8(%rax), %rax                  leaq 8(%rax), %rcx
                                               movq %gs:(%rcx), %rax
```

Fix: `emit_seg_load_with_const_offset` (trait hook + `memory.rs` impl) reached from a
relaxed `retain_ptr_only_uses` Load arm, gated by the *same* `can_const_addr_fold`
contract the default-space fold uses (the base's live interval is extended to the
access, and the emitter re-reads it there). The `!is_wide_int_type(ty)` guard is
exactly `is_foldable_mem_ty`, so the segment path cannot accept a type the
default-space path refuses.

Kernel relevance: this is `this_cpu_ptr(p)->field` and `per_cpu(x, cpu).field`
everywhere in the tree, and the shape GCC emits is one instruction shorter with one
fewer live GPR.

---

## 2. Two code-quality fixes found by red-teaming the first four

### 2.1 The lea splice refused segment-prefixed consumers

`relay_and_lea::splice_lea_into_mem_operand` — the pass that turns
`leaq 8(%rax), %rcx; movq (%rcx), %rax` into `movq 8(%rax), %rax` for *non*-segment
consumers — failed closed on `%gs:`/`%fs:` consumers, because the override lands in the
displacement slot where `parse_disp` reads `%gs:8` as a *symbol*. That is why the
default-space twin of §1.4's case folded (`call getq@PLT; movq 8(%rax), %rax`) while the
segment one did not — including after §1.4, because §1.4 covers register-homed bases
only and a call result lives in the accumulator with no home and no slot.

Fix (four surgical edits in `relay_and_lea.rs`):

* `segment_prefix_len()` — validates exactly `%cs: %ds: %es: %fs: %gs: %ss:`
  (case-insensitive, two letters, one colon, nothing else);
* `at_mem_operand_boundary()` — the window scan now recognises `%gs:(%T)`, `%gs:(%T,`
  while still refusing `*%gs:(%T)`, `foo:(%T)`, `%gsx:(%T)`;
* `compose_indexed_relay()` strips the validated prefix before parsing the consumer
  address (the caller keeps it: the rewrite only replaces the `(...)` operand);
* `splice_lea_into_mem_operand()` strips it for the parse and re-attaches it in front
  of the *composed* address, and its contract now documents the one prefix that is
  composed through.

Measured (`gcc -O2` vs `lccc -O2`, same source):

| source | lccc before | lccc now | GCC |
|--------|-------------|----------|-----|
| `long f(void){return base()->b;}` | `call; leaq 8(%rax),%rcx; movq %gs:(%rcx),%rax` | `call; movq %gs:8(%rax),%rax` | same |
| `p[i]` (indexed) | `leaq 16(%rax,%rdx,8),%rcx; movq %gs:(%rcx),%rax` | `movq %gs:16(%rax,%rdx,8),%rax` | same |
| `void w(struct S __seg_gs *p,long v){p->b=v;}` | `leaq 8(%rdi),%rcx; movq %rsi,%gs:(%rcx)` | `movq %rsi, %gs:8(%rdi)` | same |

The store case is the reason no backend store-fold was needed: the peephole composes
it, which keeps the store path's conservative liveness handling (stores still mark
their segment pointers in `retain_ptr_only_uses`) untouched.

Tests: `segment_prefix_validation_is_exact`,
`segment_override_is_an_operand_boundary_for_the_window_scan`,
`lea_folds_into_segment_prefixed_load`,
`lea_folds_into_indexed_segment_prefixed_load`,
`lea_folds_into_segment_prefixed_store`,
`segment_prefixed_consumer_composes_displacements` (unit), plus the existing
`splice_fails_closed_on_unrepresentable_or_ambiguous_operands`, whose segment case is
now a *positive* assertion (it used to pin the refusal) with `%gss:` added as the
malformed-prefix negative.

### 2.2 Scalar FP segment loads stay in the SSE domain

`mov_load_for_type(F64)` is the GPR form, so an FP per-CPU load went
`movq %gs:8(%rdi), %rax` → `movq %rax, %xmm` → `movsd %xmm, %xmm0` while the
*default-space* twin already loaded into the destination's XMM home. All three segment
load paths (folded constant offset, generic register-held pointer, `%gs:sym(%rip)`) now
use one shared `emit_seg_load_xmm` helper: `movsd %gs:8(%rdi), %xmm2`.

The behavioural half of the test caught a real bug in the first version of that helper
— it printed the destination register without its `%` (`movsd %gs:8(%rdi), xmm2`), which
the integrated assembler rejected with `error: unsupported SSE mov operands`. That is
the whole argument for keeping a runtime/execution test next to a grep-shaped gate.

---

## 3. Gate corrections (found by running the gate, not by reading it)

### 3.1 The gate's own expected constant was wrong

`tests/regression/check_seg_const_offset_fold.sh` asserted `%gs:32(%rdi)` for
`long read_c3(struct S __seg_gs *p){ return p->c[3]; }` with
`struct S { long a, b; long c[4]; }`. `c[3]` is at `16 + 8*3 = 40`. GCC emits
`movq %gs:40(%rdi), %rax`; the gate was wrong, the compiler was right. Fixed, and the
header comment now spells the arithmetic out.

### 3.2 Instruction counts cannot see a wrong constant

The GCC cross-check compared instruction *counts* only. It now extracts the `%gs:`-operand
GCC folds and requires lccc to emit that operand character for character, for every
shape in the file (`read_b`, `read_c3`, `read_a`, `fold_after_call`, `read_d`, `read_f`).
GCC spells a zero displacement the same way lccc does (`%gs:(%rdi)`), so no shape needs
an exemption.

---

## 4. Verification inventory (this session)

* Unit tests, `backend::x86::codegen::peephole::passes::relay_and_lea`:
  **141 passed / 0 failed** (6 new).
* `tests/regression/check_seg_const_offset_fold.sh` — **PASS**
  (`%gs:disp(base)`, no `leaq`, GCC instruction parity *and* exact-operand parity).
* Regression corpus, `--filter seg_gs`: **3 passed / 0 failed**, compared against GCC.
* `tests/regression/seg_gs_const_offset_fold.c` stdout **byte-identical to GCC**,
  covering: constant offsets through parameters (`read_b(0)`, `read_b(24)`,
  `read_c0(8)`, `read_c3(8)` → storage[6]), call-returned bases (`call_b`,
  `call_c3raw`), indexed accesses (`read_ci` — 4 cases), and FP members whose bit
  patterns are pi and e.
* Boot-code 32 KiB gate (`KERNEL_DIR=/opt/kernel/linux-6.18.55
  ./scripts/build_kernel_boot.sh`): **PASS** — `_end = 0x7980 = 31104`,
  headroom **1664 bytes**, section total 29441, and the `ld.bfd` oracle produced a
  **byte-identical flat setup image**
  (`sha256 7e309840b57a28060ba208aba7843a1db7338ebf120640eeed03797d73ab91c5`).

---

## 5. Bounded divergences and open items

### 5.1 FP loads land in the value's XMM home, not in the ABI register

```c
double f(struct F __seg_gs *p){ return p->d; }
GCC:  movsd %gs:8(%rdi), %xmm0
lccc: movsd %gs:8(%rdi), %xmm2 ; movsd %xmm2, %xmm0
```

The load itself is now one instruction in the SSE domain (§2.2); the second move is the
*pre-existing* return-register placement, and it is not segment-specific — the
non-segment twin does the same (`movsd 8(%rdi), %xmm2; movsd %xmm2, %xmm0`). Fixing it
means making the allocator prefer the ABI return register for a value returned by value
(a coalescing preference, not a correctness issue), which is a register-allocator
change with its own A/B evidence requirements. Recorded here with the reproducer so it
is not lost.

### 5.2 The 64-bit case of `compatible_ctype()` is target-dependent

For an enumerator outside `[i32::MIN, u32::MAX]`, `packed_size()` is 8 and
`compatible_ctype()` answers `long long`/`unsigned long long`. GCC's choice among
64-bit types depends on the data model (`unsigned long` on LP64, `long long` on ILP32),
so a *fully* faithful answer needs the target width, which `common/types.rs` currently
has no channel for (`char_is_unsigned()` is the precedent: a process-global the driver
installs). No kernel enumeration reaches this range (`packed_size()`'s own comment
documents the same 4/8 split), and any implementation must consult the data model, so
this is written down rather than guessed at. Reproducer:

```c
enum E { X = 1ULL << 40 };
void take(unsigned long *);
void f(void) { enum E e = X; take(&e); }   /* GCC accepts; lccc rejects */
```

### 5.3 Remaining kernel surface

The boot-code gate is green and the blockers above are gone; the full VM kernel build
and boot results are recorded in §7.

---

## 6. Reproductions

```sh
# 1. Compiler
scripts/build_lccc_fast.sh                     # fastbuild profile, -O1, -j2

# 2. The shapes, against GCC
cat > /tmp/shapes.c <<'EOF'
struct S { long a; long b; };
struct S __seg_gs *getp(void);
long f1(void){ return getp()->b; }
EOF
lccc -O2 -S -o /tmp/l.s /tmp/shapes.c && gcc -O2 -S -o /tmp/g.s /tmp/shapes.c
grep %gs /tmp/l.s /tmp/g.s                    # identical operands

# 3. Gate + behaviour
bash tests/regression/check_seg_const_offset_fold.sh
python3 tests/regression/run_regression.py --filter seg_gs

# 4. Boot code (32 KiB gate + ld.bfd oracle)
KERNEL_DIR=/opt/kernel/linux-6.18.55 ./scripts/build_kernel_boot.sh
```

## 7. ISA portability: the QEMU triple fault, root-caused (session 3, 2026-10-11)

### Symptom

`arch/x86/boot/bzImage` built by `scripts/build_kernel_vm.sh` (lccc + lccc-ld)
produced **no serial output at all** under QEMU — the guest triple-faulted
before the first `pr_info`.  `-d int` showed the classic double→triple exit:

```
check_exception old: 0x8 new 0xd          # #DF escalated to #DF -> triple
RIP=0000000002255dad ... CS=0010 ...      # early 64-bit startup
```

### Evidence chain

1. RIP → symbol: load base `0x1000000` (`LOAD_ADDR`), decompressor ELF link
   base 0 → `0x2255dad` = `kernel_ident_mapping_init + 0x8d`.
2. Disassembly of that exact function: `c4 62 ab f7 c0  shrxq %r10,%rax,%r8`
   — a **BMI2** instruction.
3. Instruction census: **435** BMI2-class mnemonics in
   `arch/x86/boot/compressed/vmlinux` (`.text`) and **552** in the kernel
   `vmlinux` — in builds that were supposed to be x86-64 baseline.
4. That is enough: QEMU's default CPU model (`qemu64`) has no BMI2, so the
   first `shrx` raises `#UD`; in 64-bit startup with no IDT the result is a
   triple fault and zero console output.

### Root cause (two independent flag paths, one of them ours)

* **Decompressor.** `arch/x86/boot/compressed/Makefile:27` *replaces*
  `KBUILD_CFLAGS` (`KBUILD_CFLAGS := -m$(BITS) -O2 $(CLANG_FLAGS)`), so on
  x86-64 the compressed TUs are built with **no `-march` at all**.  For GCC
  that means `-march=x86-64` (baseline).  For lccc the absent-flag default is
  the **integer-v3 profile** — a deliberate, documented design decision
  ("integer-v3 default ISA") — so lccc's effective ISA here was v3, i.e.
  BMI2/AVX2, with `-mno-sse` correctly suppressing the SIMD half.
  `scripts/build_kernel_compressed.sh` hand-writes the compressed-TU flag set
  (by design: invoking `make bzImage` leaks `-march=native` into the PIE
  decompressor) and omitted `-march` — so the script silently inherited v3.
* **Kernel proper.** `CONFIG_X86_NATIVE_CPU=y` (the CachyMod package config)
  makes `arch/x86/Makefile:170` append `-march=native`; that is correct for
  the owner's machine and for GCC alike, but a kernel built that way only runs
  on the build host's CPU class — by definition not on QEMU's `qemu64`.

Neither is a lccc correctness bug, and the ISA *gate* is sound: with an
explicit `-march=x86-64` the same TUs, the full benchmark corpus and the
BMI-triggering probes (57 files) contain **zero** BMI2/BMI/AVX-class
instructions (measured; now a gate, see below).

### Fixes (all in this patch)

| # | Site | Change |
|---|------|--------|
| 1 | `scripts/build_kernel_compressed.sh` | pass `-march=x86-64` **explicitly** in `CF` and `ST_CF` (GCC parity for a Makefile that replaces `KBUILD_CFLAGS`), with the reasoning in the header |
| 2 | `scripts/kernel-vm.fragment` | `# CONFIG_X86_NATIVE_CPU is not set` + `CONFIG_GENERIC_CPU=y`: the VM kernel is built for the generic x86-64 baseline, so the boot gate is reproducible on any host and any QEMU CPU |
| 3 | `scripts/qemu_boot_test.sh` | explicit `-cpu qemu64` (override via `QEMU_CPU=`) — the ISA contract is now visible and host-independent; a `-march=native` kernel #UDs loudly instead of passing on one host and failing on another |
| 4 | `tests/regression/check_isa_gate_escape.sh` (**new**, fast CI + hosted CI) | an explicit `-march=` is a ceiling: no BMI/BMI2/AVX-class mnemonic in `-march=x86-64` or `-march=x86-64-v2` TUs (corpus + probes), `-mno-bmi2` wins over `-mavx2`, a positive control proves the check is not vacuous, and the integer-v3 **default** is pinned so a change to it cannot silently break the kernel harnesses again |

Measured gating matrix (`/tmp/v/probe.c`: and-not fusion, rotates, variable
shifts, mask-to-bzhi, count-zero, clz/ctz, mulhi):

| flags | BMI-class mnemonics |
|-------|--------------------:|
| `-march=x86-64 -O2` | 0 |
| `-march=x86-64-v2 -O2` | 0 |
| `-march=x86-64 -mavx2 -mno-bmi2 -O2` | 0 |
| `-mavx2 -mbmi -mbmi2 -O2` (positive control) | 8 |
| `-O2` alone (the v3 default, pinned) | 8 |

### Verification

* 32 KiB boot-code gate: **PASS** (`_end=31104`, headroom 1664 bytes) with the
  flat setup image **byte-identical** to the ld.bfd oracle
  (`setup.bin` sha256 `7e309840…ab91c5`).
* VM kernel rebuilt from the fixed fragment; boot result recorded below.
* `tests/regression/check_isa_gate_escape.sh`: PASS (positive control 8).
* `tests/regression/check_seg_direct_home.sh`: PASS (8 instructions in the
  per-CPU walk loop; the pre-fix compiler FAILS it with the `%rax` relay
  quoted in §3).

---

## 8. The in-tree `make` path: the same ISA wipe in two more Makefiles (session 4, 2026-10-11)

### Why §7 was not sufficient

§7 fixed the *hand-written* decompressor harness
(`scripts/build_kernel_compressed.sh`) and the *config* path
(`kernel-vm.fragment`).  It did not cover the artifact the boot test actually
consumes: `build_kernel_vm.sh` runs **`make … bzImage`** with `CC=lccc`, so
`arch/x86/boot/compressed/` is compiled by the kernel's own Makefile — and
that Makefile *replaces* the flag set:

```
arch/x86/boot/compressed/Makefile:27   KBUILD_CFLAGS := -m$(BITS) -O2 $(CLANG_FLAGS)
```

A replacement also discards the `-march=` that `arch/x86/Makefile:185-190`
derives from `CONFIG_GENERIC_CPU` / `CONFIG_X86_64_VERSION` — which is exactly
the pair `kernel-vm.fragment` pins so the kernel *proper* is built for the
baseline.  Result: an in-tree VM kernel whose body is `-march=x86-64` and whose
decompressor is **integer-v3** (lccc's documented absent-flag default: "no
explicit `-march`: project baseline x86-64-v3", `src/driver/pipeline.rs`) —
`shrxq` in `kernel_ident_mapping_init` is `#UD` on QEMU's `qemu64`, and a
`#UD` in early 64-bit startup is a triple fault with no serial output at all.
Same symptom as §7, through a path §7 did not touch.

### Evidence chain

1. The replacement is a genuine wipe, not an append: `KBUILD_CFLAGS :=` in the
   sub-makefile overrides the inherited (exported) value; `Makefile:1145`
   (`KBUILD_CFLAGS += $(KCFLAGS)`) runs in the *top-level* make only, so
   `KCFLAGS=` cannot reach this directory either.  Only `CC` itself survives
   into every recipe — which is why a `CC="lccc -march=…"` floor was rejected
   as the mechanism: it would also apply `-march=x86-64` to the `-m16`
   real-mode TUs, i.e. impose a 64-bit ISA on 16-bit code.
2. The kernel already states this ISA per mode *in the same two files*:
   `cflags-$(CONFIG_X86_32) := -march=i386` (compressed `Makefile:33`, EFI stub
   `Makefile:12`), and `REALMODE_CFLAGS` carries `-march=i386 -mno-mmx
   -mno-sse`.  The x86-64 branches were the only ones left implicit.
3. `scripts/build_kernel_compressed.sh` reproduces the compressed flag set
   verbatim; with `-march` absent it produced **435 BMI2 mnemonics** in
   `compressed/vmlinux .text` (measured, §7), and 0 with `-march=x86-64`.  Same
   flags + same compiler ⇒ same ISA gating, so the in-tree directory would have
   produced the same bytes.
4. Measured after the fix, on the *live* in-tree build (the tree was patched
   before `make` had reached that directory, so nothing was rebuilt):
   `misc.o`'s command line contains `-march=x86-64 -mcmodel=small -mno-red-zone`
   and `arch/x86/boot/compressed/*.o` (13 objects) contains **0**
   BMI/ABM/VEX/SSE4.2-class instructions.

### Fixes (all in this patch)

| # | Site | Change |
|---|------|--------|
| 1 | `scripts/kernel_boot_isa.sh` (**new**) | `patch` / `check` / `scan` for the kernel boot path's ISA contract. `patch` names the baseline ISA in the two flag-wiping Makefiles, idempotently, and **refuses to guess** when the file has been refactored (a missing `-march=i386` sibling or an unrecognized x86-64 line is an error, never a silent skip); `scan` audits built `*.o` per object (the linked stage-2 ELF carries the compressed payload as data, which an image-wide text scan would misread) |
| 2 | `scripts/build_kernel_vm.sh` | runs `patch` before the build and `scan arch/x86/boot arch/x86/boot/startup arch/x86/boot/compressed` after it — the flag half and the artifact half of the same contract |
| 3 | `scripts/build_kernel_boot.sh`, `scripts/build_kernel_compressed.sh` | run `scan` on their own object directories before declaring success (the 16-bit setup objects are `-march=i386`; the decompressor/startup objects are `-march=x86-64`) |
| 4 | `tests/regression/check_kernel_boot_isa.sh` (**new**, fast CI + hosted CI) | fixture-driven: patch correctness (only the x86-64 line changes, byte-compared), idempotence, a **reverted line must fail `check`** (the gate can fail), two refactor cases must fail loudly without touching the file, and the scanner itself gets real objects as controls — a `-march=x86-64-v3` object and a `-mavx2 -mno-bmi*` object must be rejected with the mnemonic named, a baseline object must pass, a directory/missing-path/empty-directory argument must behave |

The tree edit itself (applied by `patch`, verified by `check`, and covered by
CI on fixtures) is one token per file:

```
arch/x86/boot/compressed/Makefile: cflags-$(CONFIG_X86_64) := -march=x86-64 -mcmodel=small -mno-red-zone
drivers/firmware/efi/libstub/Makefile: cflags-$(CONFIG_X86_64) := -march=x86-64 -mcmodel=small
```

For GCC/Clang/ICC this is a no-op (it is the baseline they already default to)
and the flag sets stay GCC-exact; for lccc it is the difference between a
bootable and a triple-faulting kernel.

### Verification

* `scripts/kernel_boot_isa.sh check <prepared tree>`: **PASS** (both branches of
  both files name their ISA).
* Live in-tree build: `arch/x86/boot/{,startup/,compressed/}*.o` = 28 objects,
  **0** BMI/ABM/VEX/SSE4.2-class instructions; `misc.o` flags quoted above.
* `tests/regression/check_kernel_boot_isa.sh`: **PASS** (patch, idempotence,
  refactor refusal, artifact scan with all controls).
* `python3 scripts/check_ci_gate_parity.py`: **PASS** (157 commands, 19
  contracts) — the new gate is wired into `ci_local.sh --fast` *and* hosted CI;
  `scripts/check_pipefail_sigpipe.py`: PASS.
* Boot result of the in-tree kernel: recorded in §9.

### Open items

1. The EFI-stub file is patched as a *contract*, not as a measurement: the VM
   config has `CONFIG_EFI` off, so those TUs are not compiled here.  The
   mechanism is identical to the decompressor case (same line shape, same
   compiler behavior), and the patch is content-asserted by `check`; a config
   with EFI on would give the direct measurement.
2. `arch/x86/boot/startup/Makefile` and `arch/x86/purgatory/Makefile` only
   *filter* the inherited flag set, so they keep the kernel's `-march=` — no gap
   (verified by reading, and the startup objects are covered by the scan).
3. If upstream refactors either file, `patch` fails loudly by design; the
   failure names the exact expected line so the fix is mechanical.
