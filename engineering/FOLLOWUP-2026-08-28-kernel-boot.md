# Follow-up work document — 2026-08-28

**Task:** build and boot the user's custom kernel (`linux-cachymod-6.18`, i.e. `linux-6.18.46`
+ the 26 `archpkgbuilds/packages/linux-cachymod-6.18` patches) with **lccc + lccc-ld**,
using a default config. Hard gate: `arch/x86/boot` setup must satisfy `_end <= 0x8000`
(32768 B).

**Status at end of session**

| Requirement | Status |
|---|---|
| Compiles end-to-end with lccc + lccc-ld | **DONE** — 1015 TUs, 0 errors |
| 32 KiB setup gate | **PASS** — `_end = 0x7600` (30208 B) |
| objtool clean | **4 warnings left** (was an "empty alternative entry" flood); GCC reference = 0 |
| Boots under QEMU | **NOT YET** — root-caused to a single precise defect (below) |

---

## 1. Accomplished this session

### 1.1 Two real assembler bugs found and fixed

Both live in `src/backend/elf_writer_common.rs`, in the deferred `.skip` machinery that the
kernel's `ALTERNATIVE()` macro depends on (a `.skip` whose size is a symbolic expression
involving labels that are only known after the section layout is final).

**Bug A — deferred `.skip` fill bytes dragged preceding same-offset labels forward.**
`resolve_deferred_skips` spliced the fill bytes at `offset`, then advanced every label with
`loff >= offset`. A label defined *immediately before* the `.skip` also has `loff == offset`,
so it got pushed forward with the fill. In the kernel's
`.byte 773b-771b` idiom (the recorded source length of an ALTERNATIVE region) this collapsed
the difference to **0**, so objtool reported "empty alternative entry" and the boot-time
alternative patcher would have patched **zero bytes** — the SMAP/lfence-style alternatives
would silently never take effect. No diagnostic was emitted anywhere.

*Fix:* `deferred_skips` became `Vec<DeferredSkip>`; the new struct carries
`labels_at_offset` / `numeric_at_offset` snapshots taken (via `snapshot_labels_at`) at the
moment the skip is deferred. Both label-adjust loops now shift a label only if it is strictly
after the gap, **or** sits at the gap start and is **not** in the snapshot. Relocations,
jumps, alignment markers and deferred diffs keep the original `>=` semantics.

**Bug B — two deferred skips at the same offset were spliced in the wrong order.**
The sort was written as
`skips.sort_by(|a,b| a.sec.key().then(a.offset.cmp(&b.offset)).reverse())`.
`.reverse()` was applied to the *comparator's `Ordering`*, not to the vector, so the sort was
descending but **stable** — ties kept source order. Since each splice *inserts* at `offset`
(pushing already-inserted bytes right), source order puts the second gap's fill **before**
the first gap's fill, silently permuting the section contents.

Evidence (`/tmp/asmtest/two_marks.S`, two consecutive empty-original ALTERNATIVEs with
different fill bytes):

```
GAS  .text:  90 eb 04 | 90 90 90 | cc | c3
LCCC .text:  90 eb 04 | cc | 90 90 90 | c3      <- gaps swapped
```

Critically, **every recorded length still looked correct** — only the byte pattern revealed
it. *Fix:* sort ascending (stable) then `skips.reverse()`, which yields descending offsets
with ties in reverse source order.

### 1.2 Validation of the fixes

* `/tmp/asmtest/two_marks.S` — LCCC `.text` now byte-identical to GAS (`90eb0490 9090ccc3`).
* `/tmp/asmtest/two_dbg.S` — gap lengths `00 03 03 / 00 03 03`, matching GAS exactly
  (previously `00 06 06 / 00 00 00`).
* `/tmp/asmtest/two_blocks.S` — `.altinstructions` now `3/3` and `3/3` (previously `6/3`, `0/3`).
* `/tmp/asmtest/diag.S`, `negskip.S`, and the full `matrix.py` differential harness: all ok.
* **Real kernel file:** `cmp_alt.py` on `fs/readdir.o` (12 `alt_instr` entries) went
  **12 mismatching → 0 mismatching** against the GCC-built reference object.
* `tests/regression/kernel_altinstr_layout.c` extended with **Case 2c** (two consecutive
  empty-original ALTERNATIVEs). Because length checks alone cannot catch Bug B, the case uses
  **distinct fill bytes (`0x90` / `0xcc`) and verifies the byte pattern at runtime**.
  Passes under both lccc and gcc. (Negative control — reverting the fix and re-running the
  test — has **not** been done yet; it was deliberately skipped because rebuilding lccc
  mid-kernel-build would have swapped the binary out from under the running build.)

### 1.3 Kernel build results (lccc, after both fixes)

```
1015 translation units, 0 errors, ~22 min (-j2)
vmlinux            20017016 B   (gcc reference: 15483712 B -> lccc is +29 %)
bzImage             4318208 B   (gcc reference: 3593216 B)
setup.bin             25248 B   (gcc reference:   14748 B)
setup _end       0x7600 = 30208 B <= 32768  -> GATE PASS
objtool warnings          4     (gcc reference: 0)
```

Remaining objtool warnings (all present only in the lccc build; addresses stripped):

```
arch/x86/kernel/signal_64.o: x64_setup_rt_frame+: return with UACCESS enabled
init/main.o:                 start_kernel+:       rest_init() missing __noreturn
kernel/exit.o:               __do_sys_waitid+:    return with UACCESS enabled
kernel/sched/fair.o:         dequeue_entities+:   unreachable instruction
```

### 1.4 Boot-failure bisect (the main result)

QEMU (`-m 512 -smp 2 -accel tcg,thread=multi`, serial log) produced zero bytes for the lccc
image while the GCC reference produced 157 lines including the validation markers. Bisecting
by swapping image halves:

| Image | Result |
|---|---|
| gcc setup + gcc payload | **BOOTS** (157 lines) |
| **lccc setup + gcc payload (hybrid A)** | **BOOTS** (157 lines) |
| **gcc setup + lccc payload (hybrid B)** | **HANGS** (0 lines) |
| lccc setup + lccc payload | HANGS (0 lines) |

=> **lccc's real-mode setup code (`arch/x86/boot`, 16-bit) is correct.** The fault is in
`vmlinux.bin`, i.e. `arch/x86/boot/compressed/*` and/or the decompressed kernel.

Live register dump (QMP `human-monitor-command 'info registers'`, 25 s into the boot):

```
RIP=0x221bdf3  HLT=1  CR0=0x80050033 (paging on)  CR3=0x225a000
code at RIP:  eb fd    jmp 0x221bdf2        ; preceded by f4 (hlt)
```

`f4 eb fd` is the `while (1) asm("hlt")` infinite halt GCC-style compilers emit for the
kernel's `error()`/dead-end paths. Searching vmlinux for that 3-byte signature and scoring
candidates by how many sampled stack words fall inside the image localised it to:

```
HALT LOOP IS IN: fpu__init_system+0x11e
stack: fpu__init_system+0xfb, detect_art+0x1a1, early_cpu_init+0x162,
       tsc_early_init+0x161, alternative_instructions+0x11
```

Disassembling `fpu__init_system` in both vmlinux images shows **identical control flow**:

```
GCC :  mov __pi_boot_cpu_data+0x30,%rax ; test $0x1,%al ; jne <continue>
       ... else: _printk("..."); hlt ; jmp -3
LCCC:  mov __pi_boot_cpu_data+0x30,%rax ; and $1 ; cmp $0 ; setne ; test ; je <panic>
       ... panic path: _printk("..."); hlt ; jmp -3
```

Both halt iff `boot_cpu_data.x86_capability[0]` bit 0 (`X86_FEATURE_FPU`) is clear.
`__pi_boot_cpu_data` and `boot_cpu_data` are correctly aliased at the same address with the
same size (`0x150`) in **both** builds, and the static initializer is **all zeros in both**
builds — so the bit is set at *runtime* by the CPUID path.

**Conclusion:** the lccc-built kernel never sets `boot_cpu_data.x86_capability[0]` bit 0
before `fpu__init_system()` tests it. The defect is therefore in the **CPU feature
detection path** — `early_cpu_init()` / `identify_cpu()` / `get_cpu_cap()` and the `CPUID`
inline-asm / capability-bit-setting helpers in `arch/x86/kernel/cpu/common.c`,
`arch/x86/lib/cpu.c` (or their callees) — not in the alternative machinery and not in the
real-mode setup.

---

## 2. To-do (ordered by expected impact)

1. **Fix the CPU-feature-detection miscompile (blocker for booting).**
   Narrow it further with a targeted A/B: compile *only* `arch/x86/kernel/cpu/common.c`,
   `arch/x86/kernel/cpu/intel.c`, `arch/x86/lib/cpu.c` etc. with gcc into the otherwise
   lccc-built vmlinux, bisecting which TU restores the FPU bit. Then reduce that TU.
   Prime suspects: `cpuid`/`cpuid_count` inline asm constraints, `set_cpu_cap()` bit
   manipulation, `memcpy` of `struct cpuinfo_x86`, and any `__builtin_cpu_supports`-style
   construct. The `f4 eb fd`-signature + QMP register-dump technique in §1.4 is reusable and
   cheap (~30 s per experiment).
2. **Negative control for Bug B.** Temporarily revert the `skips.reverse()` change, rebuild
   lccc, and confirm the new Case 2c in `tests/regression/kernel_altinstr_layout.c` actually
   FAILS. This must be done **when no kernel build is running** (rebuilding swaps the binary
   out from under an in-flight build).
3. **Zero out the 4 remaining objtool warnings.** "return with UACCESS enabled" is
   potentially a real correctness issue (stac/clac pairing), not just noise.
4. **Code-size gap.** lccc `vmlinux` is +29 % vs gcc. Once it boots, profile the biggest
   deltas — with the icount harness now proven, size is a good proxy for the perf work.
5. **Then** the in-QEMU A/B performance work (`scripts/in_vm_codegen_bench.sh`,
   `scripts/qemu_icount_plugin/`) that the boot gate has been blocking.

## 3. Gaps, risks and technical debt

* The deferred-`.skip` subsystem still has **no negative-control coverage for Bug A** either
  (Case 2b passes, but has not been shown to fail on the pre-fix compiler).
* No unit tests exercise the `DeferredSkip` snapshot logic directly; all coverage is
  end-to-end assembly-level.
* `resolve_deferred_skips` is called, then `relax_jumps()` runs again afterwards. The
  interaction between the two passes is only covered indirectly; the reasoning in the
  session notes about it did not match observed behaviour, so it deserves a careful re-read
  with instrumentation rather than more inference.
* The kernel can only be built **in-tree** in this tree (`O=` is rejected: "source tree is
  not clean", and `make clean` does not satisfy it), so gcc-reference and lccc builds must
  run **sequentially** with a `make clean` between them, copying outputs out first.
* Disk is tight: 25 G root, ~6 G free. The 1.7 G kernel tree plus two 15–20 MB vmlinux
  copies per configuration is near the practical limit.

## 4. Artifacts

* `/home/user/artifacts/kernels/lccc-fixed-boot-bisect/` — `vmlinux.lccc`, `bzImage.lccc`,
  `setup.bin.lccc`, `hybA.bin`, `hybB.bin`, `build.log`.
* `/home/user/artifacts/kernels/` — GCC reference: `bzImage.gcc`, `vmlinux.gcc`,
  `setup.bin.gcc`.
* `/home/user/artifacts/repro/` — the assembly reducers (`two_marks.S`, `two_dbg.S`,
  `two_blocks.S`, `two_distinct.S`, `diag.S`, `negskip.S`) plus `matrix.py` and `cmp_alt.py`.
* Build logs: `/home/user/kernel-build-lccc3.log`, `/home/user/gcc-kbuild.log`,
  `/home/user/build-lccc5.log`.
* Boot logs: `/tmp/boot-gcc.log` (working GCC boot, 157 lines), `/tmp/bisect/o.*.log`.
* Auto-saved patch: `/home/user/ms178-1.patch` (snapshot S05).
