# Follow-up: the CachyMod kernel now builds and boots on lccc + lccc-ld alone

Date: 2026-09-21 (session 58)
Base: `c160d11` (`ms178/lccc` main).

The standing goal for this program was "build the user's custom kernel with
lccc and lccc-ld only, and boot it in QEMU". **That goal is now met**, and this
document records the measurement, the two codegen fixes that landed on the way,
one measurement-methodology trap that produced a false result mid-session, and
the one runtime defect that is still open.

---

## 1. The milestone, with its evidence

`scripts/build_kernel_vm.sh` on the real package tree
(`/opt/kwork/linux-6.18.52`, linux-cachymod 6.18.52, 28/28 CachyMod patches,
the package's own config — `SCHED_BORE`, `SCHED_CACHE`, `HZ_800`, `CACHY`,
`TCP_CONG_BBR`, `PREEMPT`, `SMP`, `KERNEL_ZSTD` all `=y`):

* 1363 `# CC` lines, zero compiler errors, objtool accepted every object.
* `arch/x86/boot/compressed/vmlinux`, `arch/x86/boot/setup.elf` and
  `arch/x86/boot/bzImage` (5071872 B) all linked by **lccc-ld**.
* Kernel banner in the guest:

```
Linux version 6.18.52-2.1-cachymod (user@e2b.local) (lccc (a high performance
Claude's C Compiler fork, GCC-compatible) 14.2.0, GNU ld (LCCC built-in) 2.42)
#1 SMP PREEMPT Mon Sep 21 20:53:15 UTC 2026
```

`scripts/qemu_boot_test.sh` — **16/16 PASS, exit 0**, 596 lines of dmesg,
`reboot: Power down` at guest time 1.791611 s. The checks are not a smoke test:
they cover the banner naming lccc, each CachyMod feature being compiled in,
`bbr` present in `tcp_available_congestion_control`, `CACHE_HOT_BUDDY` in the
sched features, BORE stats, 2 CPUs online, two verbatim serial-integrity
sentinels (which catch a console that drops or splits bytes), and a clean
poweroff.

`scripts/run_regression_suite.sh`: **PASS=737 FAIL=0 SKIP=8, AB-diff failures: 0**.
`cargo test --profile fastbuild --locked --lib`: **3092 passed, 0 failed,
7 ignored**.

---

## 2. Fix A — i686 narrow-value facts were dropped for every indexed load

### The defect

`eliminate_redundant_zext_i686` and `eliminate_redundant_sign_ext_i686` split an
instruction's operands with `rest.split_once(',')`. For a SIB (indexed) memory
operand that splits at the *wrong* comma:

```
movsbl (%edi,%ebx),%esi
       └── split_once(',') gives src="(%edi"  dst=" %ebx)"
```

`register_family(" %ebx)")` returns `REG_NONE`, so the load's destination was
never recorded as holding a sign-extended byte. Every narrow-value fact
(`is_byte` / `is_16` / `sign_ext`) that an indexed load establishes simply
evaporated, and the redundant extension that follows it survived:

```
movsbl (%edi,%ebx),%esi      movsbl (%edi,%ebx),%esi
movl   %esi,%eax        →    movl   %esi,%edx
movsbl %al,%eax
movl   %eax,%edx
```

This was **fail-closed** — a missed optimization, never a miscompile — which is
why nothing caught it: no wrong answer, no assertion, just code that a 1990s
peephole would have removed.

### The fix

`src/backend/i686/codegen/peephole.rs` gains `split_operands_last_comma`
(delegating to the existing, paren-aware
`peephole_common::last_top_level_comma`), used at the four narrow-value sites
(`movzbl` ×2, `movsbl`, `movzbl`). The other 34 `split_once(',')` call sites in
that file were triaged by mnemonic: they are two-operand register/immediate
forms where no operand can contain a comma, so they are correct as written and
were deliberately left alone.

Guard test `indexed_load_never_leaks_a_fact_into_its_index_register` pins the
*direction* of the fix: a looser parse would hand `%ecx` a byte fact it does
not have and delete a genuine `movzbl %cl,%ecx`. The test fails if anyone
"fixes" the split by taking the first comma instead of the last.

### Measurement

Corpus A/B over `tests/regression/*.c` compiled as i686 (`-m32`, identical
command lines, sizes from the ELF section table), old binary vs new:

| Opt | Total `.text` | Delta | Files changed |
| --- | --- | --- | --- |
| `-Os` | 862107 → 861248 | **−859 B (−0.10 %)** | 60 of 706 |
| `-O2` | 1006391 → 1005772 | **−619 B (−0.06 %)** | 52 of 706 |

The 32 KiB x86 setup corpus is unchanged (`.text` 20335) — the real-mode boot
objects contain no indexed narrow-load sites, which is worth knowing: **the
boot gate is not a sensitive instrument for i686 peepholes**, and a change can
be a real win there and invisible in `arch/x86/boot`.

## 3. New tooling: `scripts/i686_size_ab.py`

The measurement above is now reproducible rather than an ad-hoc loop:

```
scripts/i686_size_ab.py                        # tests/regression, -Os and -O2
scripts/i686_size_ab.py --corpus 'arch/x86/boot/*.c' --kernel-dir $K --opt -Os
```

It compiles every source with both binaries, reports per-file `.text` deltas
and the aggregate, and never disassembles. The gap it fills: the boot gate is a
*gate*, not a *signal*, and until now there was no way to answer "what did this
peephole do to 32-bit code size in general?" without writing a throwaway script
that gets the flags wrong. (The first version of that throwaway script did
exactly that — see §5.)

## 4. A measurement trap: `-O2` file-level sizes carry ±60 B of alignment noise

One file appeared to regress: `simd_avx2_defer_chain.c` **+147 B** at `-O2`.
It did not. Its instruction count went *down* by 8, and all four hunks are the
same 3-instructions-to-1 win from Fix A:

```
- movzbl 188(%esp,%edi),%eax     + movzbl 188(%esp,%edi),%esi
- movzbl %al,%eax
- movl   %eax,%esi
```

Assembling the compiler's own `-S` output with GAS gives `.text.main`
649 → 628 B (−21 B), exactly as predicted. The +147 comes from the integrated
path, which additionally emits `.lccc_tight_loop` markers so the integrated
assembler can insert **unconditional** 16/32/64-byte loop alignment that `-S`
deliberately omits (`generation.rs:4534`, "Only present in code destined for
the integrated assembler"). Removing 12 bytes shifts every loop's start offset,
and padding to a 64-byte boundary can grow by up to 63 B per aligned loop.

Consequences for future work, worth more than the fix itself:

* **`-Os` is the clean i686 code-size signal** (loop alignment is empty there);
  `-O2` per-file numbers are noisy at ±60 B and must not be read as a
  regression without checking the instruction count.
* `-S` output is *not* size-equivalent to `-c` output on this backend. Any
  oracle comparison built on `-S` under-measures the integrated path — here by
  184 B on a single function.

## 5. Fix B — a pre-existing test the epilogue-merge pass had silently broken

`unused_callee_saves_refused_when_interleaved_label_edge_prealloc` asserted a
literal count of `addl` instructions in the epilogue. The epilogue-merge pass
legitimately cross-jumps textually identical epilogues via
`jmp .L__lccc_epilogue_N`, so the count changed from 1 to 2 with identical
semantics — verified by diffing the instruction sequence, not assumed. The
assertion now counts *deallocation sites* (`addl $16,%esp` **or**
`jmp .L__lccc_epilogue_`), which is the property actually under test.

The lesson is procedural and is the reason this section exists: the break was
landed in an earlier session and survived because the validating run was
filtered (`cargo test --lib epilogue_merge`). **A pass change must be validated
with the full `--lib` suite.** Three other tests that count `addl $N,%esp`
textually (peephole.rs:14582, 14627, 14899) were checked and are unaffected.

## 6. Open: one boot-time WARNING, and what has already been ruled out

The boot is clean except for one `WARN_ON_ONCE`, and it is not yet attributed:

```
WARNING: CPU: 1 PID: 1 at free_large_kmalloc+0x99/0x190
 kfree+0xb6/0x370
 acpi_ds_create_operand+0x26c/0x580
 ... acpi_ns_evaluate ... acpi_init
page dumped because: Not a kmalloc allocation   (pfn:0x173b, refcount:1)
```

Evidence gathered:

* The condition is `!folio_test_large_kmalloc(folio)` (mm/slub.c:6816), i.e.
  `kfree()` received a pointer that is neither slab nor large-kmalloc.
* `pfn 0x173b` → phys `0x173b000`, which lies inside vmlinux's first `PT_LOAD`
  (phys `0x1000000`–`0x1d98454`, `R E`). So `kfree` was handed **a pointer into
  the kernel image**.
* The call site is `ACPI_FREE(name_string)` in `acpi_ds_create_operand`
  (drivers/acpi/acpica/dsutils.c), reached only on the `AE_AML_NAME_NOT_FOUND`
  error path — which QEMU's SeaBIOS DSDT provokes every boot
  (`Failure resolving symbol [\_SB.PCI0.PRES._INI.D]` immediately precedes it).
* The source is clean: `name_string` comes from
  `acpi_ex_get_name_string` → `acpi_ex_allocate_name_string` → `ACPI_ALLOCATE`,
  and every path allocates. There is no source-level way to get a `.text`
  pointer there, so this is memory corruption or codegen — not ACPICA logic.

### The bad pointer, captured live

`scripts/qemu_gdbstub_probe.py` (new this session) breaks in the guest over
QEMU's `-s` stub, no gdb required. Breaking on `free_large_kmalloc` and
filtering on the `object` argument until it lands in the kernel image:

```
[MATCH] rip=0xffffffff814fe040 rsi=0xffffffff8173b8c6 rdi=0xffffea000005cec0 ...
=== rsi=0xffffffff8173b8c6 ===
nearest symbol: acpi_ut_trace_ptr 0xffffffff8173b890 (+54)
first 64 bytes at rsi: 448b15f30978004181e200002000743f...  (i.e. instructions)
return-address candidates from rsp:
  rsp+0x000: kfree+0xb6
  rsp+0x090: acpi_ds_create_operand+0x26c
  rsp+0x0f0: acpi_ds_evaluate_name_path+0x92
  rsp+0x140: acpi_ds_exec_end_op+0xfe
  rsp+0x190: acpi_ps_parse_loop+0x2f3
```

`0xffffffff8173b8c6` is the instruction immediately after
`call acpi_ut_track_stack_ptr` inside `acpi_ut_trace_ptr` — it is a **return
address**, i.e. `kfree()` was handed a value read off the stack, not a pointer
anyone ever allocated.

### Where the value comes from — and where it does not

Both translation units were disassembled at the exact sites:

* `acpi_ds_create_operand` passes `lea -0x38(%rbp),%rdx` as `out_name_string`
  and frees with `mov -0x38(%rbp),%r9; mov %r9,%rdi; call kfree`. **Store
  target and load source are the same slot** — correct.
* The status check survived optimization: `mov %eax,%r14d; test %r14d,%r14d;
  je …` — correct, so the free is not on a failure path.
* `acpi_ex_get_name_string` spills `out_name_string` to `-0x40(%rbp)` and its
  success epilogue is `mov -0x40(%rbp),%rcx; mov %r13,(%rcx)` followed by the
  length store through `-0x48(%rbp)`. **Correct, and not confused with the
  length store.**
* The caller's frame cannot be reached by a callee: `-0x38(%rbp)` is
  `0xffff8880024d34a0` while `rsp` at the fault is `0xffff8880024d3450`, so no
  called function writes there legitimately.

So the value in `*out_name_string` is whatever `r13` (`name_string`) held
inside `acpi_ex_get_name_string`. A stale return address from the ACPICA trace
functions — which run at the *entry* of these very functions — is the signature
of reading an **uninitialized stack slot**. `name_string` is `= NULL` in the
source and every allocation path assigns it, so either the initializer or one
of the five inlined `acpi_ex_allocate_name_string` results is being lost. That
is the concrete next target: a standalone reproducer for
`acpi_ex_get_name_string`'s allocation paths, not another kernel rebuild.

### Ruled out

The global-location-allocation / rematerialization pass (GLA) was the prime
suspect, because lccc prints
`[GLA] acpi_ex_get_name_string: rewrite failed structural verification (φ in
block 246 incoming set mismatches CFG preds (missing [270], extra []));
aborting plan with zero edits` at `-O0` for this exact function. Recompiling
both TUs at `-O2` with the documented kill switch `CCC_RA_GLOBAL_LOCATION=0`
produces **byte-identical objects** (6672 B and 12792 B): GLA does not touch
either TU at `-O2`, so it is not the cause.

**An experiment that did not produce a result.** The plan was to rebuild both
objects at `-O0`, relink, and boot, to separate "optimization-dependent codegen
bug" from "corruption originating elsewhere". It is invalid: the relink turned
into a full rebuild (the earlier `build_kernel_boot.sh` runs had refreshed
generated headers), and although the two objects were `touch`ed to look
up-to-date, make recompiled them anyway at 21:29 (`md5` differs from the `-O0`
build). **`touch` is not sufficient to keep a hand-built object out of a kernel
rebuild**; a per-file `Makefile` override or `KBUILD_` flag is. Do not cite any
`-O0` result from this session — there isn't one.

Also noted, both non-lccc: `ACPI Error: Could not execute arguments for
[_FDI]/[_S3_]/[_S4_]/[_S5_]` and the `_CRS.D` / `_STA.D` symbol failures are
SeaBIOS/BOCHS DSDT limitations, and `tsc: Unable to calibrate against PIT` is
a QEMU TCG artifact.

## 6a. Three harness defects behind one "ALL CHECKS PASSED"

The boot test reported 16/16 while one of its checks was reading nothing. All
three are fixed in `scripts/qemu_boot_test.sh` and `scripts/kernel-vm.fragment`,
and the suite is 16/16 again on the rebuilt image with the check now genuine.

1. **A vacuous check.** "BORE stats in sched_debug" was `expect "bore|BORE"`.
   The dmesg line "BORE CPU Scheduler" matches that, so the check passed while
   its own `grep` was failing. Worse, the check could *never* have matched its
   intended file: `kernel/sched/debug.c:970` prints the BORE score as an
   unlabelled column (`SEQ_printf(m, " %2d", p->bore.score)`) and the
   "runnable tasks:" header at `debug.c:988` has no BORE token, so the word
   "bore" does not occur anywhere in sched_debug. The replacement reads
   `/proc/self/sched`, where `proc_sched_show_task` (`debug.c:1372`) does label
   it under `CONFIG_SCHED_BORE`, and the guest emits `borescore-ok:` only on a
   real match (`borescore-missing:` cannot satisfy the expect):

   ```
   PASS: BORE score exposed per task
   borescore-ok: bore.score                                   :                    0
   ```

2. **A wrong path.** `/proc/sched_debug` does not exist on 6.18 — upstream
   moved it to debugfs (`debug.c:744`,
   `debugfs_create_file("debug", 0444, debugfs_sched, …)`), i.e.
   `/sys/kernel/debug/sched/debug`. The guest confirms the path resolves
   (`scheddbg-file: /sys/kernel/debug/sched/debug`) and prints the file head
   when a grep finds nothing, so this class of failure is visible in the log
   instead of silent.

3. **A phantom config symbol.** `kernel-vm.fragment` sets
   `CONFIG_SCHED_DEBUG=y`, but `grep -rn '^config SCHED_DEBUG' --include=Kconfig*`
   finds no such symbol in this tree, so `olddefconfig` drops it and `.config`
   never mentions it — and it is not in the script's `REQUIRED` verification
   list, so nothing complained. Annotated in the fragment.

The procedural lesson is the point: **a green verdict is only as good as the
patterns behind it, and those patterns are only as good as the paths and
symbols they name.** A check that can be satisfied by a line the guest prints
for a different reason is not a check.

## 6b. Final validation on the rebuilt image

The tree was fully rebuilt after §1 (the relink cascaded into a whole-kernel
rebuild because the boot-corpus runs had refreshed generated headers). The
final `bzImage` (#2, 5067776 B, 21:39) was re-tested rather than assumed:

`scripts/qemu_boot_test.sh` → **16/16 PASS**, `reboot: Power down` at
1.725358 s. Severity scan of the serial log: one `WARNING`/`Call Trace` pair —
the §6 `free_large_kmalloc` warning, unchanged (32 `dump_page` lines) — and no
`BUG:`, `Oops`, `UBSAN`, `KASAN`, general-protection or invalid-opcode events.

## 7. Next steps

1. **Build a standalone reproducer for `acpi_ex_get_name_string`.** §6 narrowed
   the defect to a lost initializer or a lost allocation result on one of the
   five `acpi_ex_allocate_name_string` paths, observed as a stale return
   address in the out-parameter. That needs a differential runtime test (the
   `tests/regression` AB harness already does the comparing), not another
   kernel rebuild — the kernel is 40 minutes per iteration and the `-O0`
   attempt this session produced nothing because make recompiled the objects.
2. Land the constant/copy rematerialization work in the register allocator. The
   i686 spill ratio is still 42.7 % and the boot corpus still shows 48
   `cmp $imm,slot` sites (466 B); both are symptoms of over-slotting, and no
   peephole can reach them. This is a bigger job than any pass landed so far
   and is the largest remaining lever on i686 code quality.
3. `scripts/qemu_gdbstub_probe.py` needs its `--stack-scan` heuristics tuned
   (it lists stale frames as return addresses) and a `--until-range` that can
   test memory rather than a register. The breakpoint/continue/`vCont` path is
   solid and is what produced §6.
4. Re-run the boot corpus A/B (`scripts/i686_size_ab.py --corpus
   'arch/x86/boot/*.c'`) after the rematerialization work lands; §2 records
   that the boot gate is insensitive to i686 peepholes, so it should be read
   alongside the regression corpus, not instead of it.

## 8. Gate status at the end of this session

`scripts/ci_local.sh --fast`: **55 passed, 0 failed, 4 skipped, exit 0**
(cargo-test 447 s). Two gates failed on the first run and both were real:

* `rustfmt` — two hunks in `src/backend/i686/codegen/peephole.rs` (one from the
  epilogue-merge pass, one from the test rewritten in §5). Fixed with
  `cargo fmt`; the only file it touched is that one.
* `doc-link-integrity` — this document named a suite that does not exist
  (`scripts/regression_local.sh` <!-- dl-skip -->); the real one is
  `scripts/run_regression_suite.sh`.

That the link gate caught a wrong script name in a brand-new document is the
gate doing its job, and it is the same failure mode as §6a: a reference that
reads plausibly and points at nothing.
