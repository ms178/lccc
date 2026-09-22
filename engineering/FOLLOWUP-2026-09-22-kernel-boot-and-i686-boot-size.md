# Follow-up: kernel boots clean on 6.18.52, and the i686 boot-size gap finally has a number and a name

Date: 2026-09-22 (session 59)
Base: `2874963` (`ms178/lccc` main).

The standing goal — "build the user's custom kernel with lccc and lccc-ld
alone and boot it in QEMU" — is met again on the current base, and this time
**the boot is completely clean**: zero `WARNING`, zero `Call Trace`, zero
ACPI errors. The `free_large_kmalloc` defect that session 58 left open did
not reproduce (§2). The second half of the session turned the i686 boot-size
gap from an anecdote into a measured, attributed number (§3), falsified one
proposed fix with a paired A/B (§4), and landed one real improvement (§5).

---

## 1. The milestone, with its evidence

Host: 2 cores, 1.9 GiB RAM, single 25.8 GiB disk, 12 GiB `/swapfile`.
Kernel tree `/opt/kwork/linux-6.18.52`, linux-cachymod 6.18.52, all 28
CachyMod patches in PKGBUILD `source=` order, config =
`make allnoconfig` + `scripts/kernel-vm.fragment` + `olddefconfig`.

`scripts/build_kernel_vm.sh`:

* **`build: SUCCESS in 1798s`**, 1374 `# CC` lines, zero compiler errors,
  objtool accepted every object.
* `vmlinux` 20577712 B; `arch/x86/boot/bzImage` 5071872 B — every link
  (`setup.elf`, `compressed/vmlinux`, `vmlinux`) by **lccc-ld**.
* `size vmlinux` → text 11901398, data 3217216, bss 2228224.
* **`setup _end: 0x00007370 (29552 bytes, ok of 32768)`** — the 32 KiB gate
  passes with **3216 bytes** of headroom.

`scripts/qemu_boot_test.sh` → **16/16 PASS, exit 0**, `reboot: Power down`
at guest time 1.524993 s. Banner:

```
Linux version 6.18.52-2.1-cachymod (user@e2b.local) (lccc (a high performance
Claude's C Compiler fork, GCC-compatible) 14.2.0, GNU ld (LCCC built-in) 2.42)
#1 SMP PREEMPT Tue Sep 22 14:05:55 UTC 2026
```

Severity scan of the 285-line serial log: `WARNING` 0, `Call Trace` 0,
`BUG:` 0, `Oops` 0, `UBSAN` 0, `KASAN` 0, `general protection` 0,
`invalid opcode` 0. Congestion list `reno bbr bic cubic westwood htcp`;
`CACHE_HOT_BUDDY` present in the sched features; `borescore-ok:` emitted;
2 CPUs online; both serial sentinels verbatim.

---

## 2. Session 58's open defect did not reproduce — and what that means

Session 58 (`FOLLOWUP-2026-09-21-i686-operand-split-and-kernel-boot.md` §6)
left one runtime defect open: a `WARN_ON_ONCE` in `free_large_kmalloc`
reached from `acpi_ds_create_operand`'s `ACPI_FREE(name_string)` error path,
with the freed pointer proven to be a stale **return address** read off the
stack (`0xffffffff8173b8c6` = `acpi_ut_trace_ptr+54`).

On this base and this config it is **absent**, and so is its trigger: the
log contains **zero** occurrences of `Failure resolving symbol`,
`Could not execute arguments`, `AE_AML` or `AE_BAD`. Session 58 recorded
that QEMU's SeaBIOS DSDT provoked that error path *every boot*. It did not
this time.

Two candidate explanations, and the session did not separate them:

1. **A codegen fix between `c160d11` and `2874963`** removed the lost
   initializer / lost allocation result that put a stale return address in
   `*out_name_string`. The window contains at least
   `05c9a6b Fix unsound coalescing of narrow call results into wide stack
   slots`, which is exactly the right shape of fix for "a stack slot held a
   value nobody stored there".
2. **The trigger moved.** The QEMU/SeaBIOS here is
   `1.16.3-debian-1.16.3-2`; a different DSDT would not provoke the
   `AE_AML_NAME_NOT_FOUND` path at all, and the latent defect would simply
   not be reached.

**Do not close the item on this evidence.** A defect that is not *triggered*
is not a defect that is *fixed*. The cheap discriminator, for whoever picks
this up: run the session-58 gdbstub probe
(`scripts/qemu_gdbstub_probe.py`, break on `free_large_kmalloc`, filter on
the `object` argument) against this image. If the breakpoint never lands on
an in-image pointer, the path is not reached and explanation 2 holds; the
standalone `acpi_ex_get_name_string` reproducer is then still owed.

---

## 3. The i686 boot-size gap, measured and attributed

`scripts/boot_size_oracle.sh` (both arms on the identical
`scripts/boot_flags.sh` command lines):

| | `.text`-class bytes | `_end` | headroom |
|---|---:|---:|---:|
| lccc | 24968 | 31168 | 1600 |
| gcc 14.2 | 14270 | 22880 | 9888 |
| clang 19 | 15555 | 22768 | 10000 |

lccc is **+10698 B vs gcc (+75 %)** and **+9413 B vs clang**. Worst objects:
`printf` +1597, `video` +1332, `string` +974, `cpucheck` +919,
`early_serial_console` +775, `cmdline` +767.

**New tool: `scripts/boot_insn_census.py`.** The oracle says *how much*;
this says *what kind of instruction*. Over the 21 TUs both toolchains
compile:

```
total instructions: lccc=5127  gcc=3183  delta=+1944 (+61.1%)

mnemonic        lccc   gcc   delta
movl            1949   773   +1176      <- 60% of the entire surplus
leal             209    88    +121
jmp              264   144    +120
movzwl           154    45    +109
cmpl             245   144    +101
movzbl           137    46     +91

structural defect-class counters      lccc   gcc  delta
store-to-frame-slot                    546    41   +505
load-from-frame-slot                   342    61   +281
reg-to-reg-movl                        443   213   +230
total-frame-bytes                     5116  2708  +2408
functions-with-frame                    70    30    +40
reg-to-reg-narrow-extend                99    30    +69
```

The attribution is unambiguous: **gcc opens a frame in 30 of 67 functions,
lccc in 70 of 70, and lccc's frames total 2408 more bytes.** 505 extra
stores to frame slots and 281 extra loads back out of them. `movl` alone
accounts for +1176 instructions — roughly 4.1 KiB of the 10.7 KiB.

Concrete shape (`arch/x86/boot/cpucheck.c`, `check_cpuflags`, 155 B vs
gcc's 70 B):

```
movl req_flags(,%ebx,4), %eax      movl %eax, 4(%esp)     ; value -> slot
movl $cpu, %edx                    movl cpu+12(,%ebx,4), %eax
notl %eax                          movl %eax, %edx         ; copy, then
...                                                       ; not the SOURCE
movl 4(%esp), %eax                 andl %edx, %eax
movl %eax, 0(%esp)                 cmpl $0, 0(%esp)        ; compare memory
```

Every intermediate has a frame slot *and* is staged through `%eax`. `LCCC_DBG_RA=1
LCCC_DBG_RA_FUNC=check_cpuflags` shows why: `values=22 assigned=10`, and the
12 unassigned include short-lived values marked **`elig=true`** (v10, v13,
v16, v21) — eligible for a register, given a slot anyway. With
`available_regs = {ebx, esi, edi, ebp}` (4) plus `%ecx`/`%edx` handed out
only where the scratch-hazard whitelist proves them clean, the pool is real
but the greedy start-point scan spends it on long-lived values first.

This is `engineering/subsystems/i686.md` §2.2 ("Leaf-function prologue &
argument homing … ≈1.7× static cost vs GCC") and §5 item 1, now with a
corpus-wide number attached.

---

## 4. Falsified: relaxing the i686 accumulator no-home policy makes it *worse*

`src/backend/i686/codegen/prologue.rs` denies a register home to every
"immediately-consumed" value, forcing it through `%eax`. The stated premise
is that a home would only add a `movl %R,%eax` relay. That premise is false
for the ops `try_emit_int_binop_direct` handles, which read a register-homed
operand **in place** — so the policy looked like the direct cause of the
slot round trips, and the obvious experiment was to lift it.

It is now a switch, `CCC_NO_I686_ACCUM_NOHOME`
(`RaConfig::no_i686_accum_nohome`, mirroring the existing
`CCC_NO_X64_IMMED_NOHOME`), and the measurement says **keep the policy**:

```
scripts/boot_ab_size.py --a <old> --b <new> --b-env CCC_NO_I686_ACCUM_NOHOME=1
  TOTAL  A=22111  B=22502   +391 bytes (+1.77%)   17 of 20 objects worse
```

The homes cost more (extra callee-save push/pop pairs, and the direct path
does not in fact fire for most of them) than the round trips they replace.
The default stays ON; the switch stays in the tree so the next person does
not have to rebuild to re-derive this. **This is a do-not-retry.**

---

## 5. Landed: `fold_copy_narrow_ext_pairs` (i686 peephole)

The census's `movzwl` +109 is 45 copies of one shape:

```
movl   %ebx, %eax        ; 2 B — the copy
movzwl %ax, %eax         ; 3 B — the narrow type invariant
```

`movzwl %bx, %eax` (3 B) is exactly equivalent: the `movl` wrote `%eax=%ebx`,
so the extension's source `%ax` held `%bx`, and the extension overwrites
`%eax` completely. New pass `fold_copy_narrow_ext_pairs` in
`src/backend/i686/codegen/peephole.rs`, covering all four narrow-extend
mnemonics (`movzbl`/`movsbl`/`movzwl`/`movswl`), wired in **before**
`eliminate_redundant_zext_i686` so the fused form becomes that state
machine's input shape rather than competing with it.

Fail-closed rules, each with a concrete wrong-code shape behind it: the
source must be spelled as a 32-bit register (`%ah`/`%al` share a family, so
a family-only comparison would read the wrong byte); `%esp` excluded;
`narrow_reg_name` returning `None` refuses the rewrite (`%sil`/`%dil` are
64-bit only); a barrier or label between the lines kills the fold.

Measured, both arms same tree, same command lines:

| corpus | A (before) | B (after) | delta |
|---|---:|---:|---:|
| `arch/x86/boot`, 20 objs (object sum) | 22195 | 22091 | **−104 B (−0.47 %)**, 9 objects better, 0 worse |
| `arch/x86/boot` **gate**, paired | `_end` 28144 | `_end` 28048 | **−96 B**, headroom 4624 → 4720 |
| `tests/regression`, 742 srcs, `-Os` | 883363 | 883209 | **−154 B (−0.02 %)** |
| `tests/regression`, 742 srcs, `-O2` | 1042988 | 1042554 | **−434 B (−0.04 %)** |

The in-suite boot gate agrees: `.text` 20335 → **20329**,
`pre-pecompat end 21525 ≤ 24576`. Three files improved by more than 20 B
(`video-mode` −39, `cmdline` −21, `string` −11); nothing regressed by more
than the `-O2` alignment noise documented in session 58 §4 (largest +48 B,
`bb_slp_v4.c`, against a corpus net of −434 B).

### 5a. Measurement trap: the tree state moves the gate by 3 KiB, so a gate number is worthless unpaired

The first "after" gate reading was `_end=28048` against the session's
opening reading of `_end=31168` — an apparent 3120-byte win from a 104-byte
peephole. It is not a win; the two readings are **different tree states**.
`build_kernel_vm.sh` leaves the tree with real generated headers (and a
different `.config`), and `build_kernel_boot.sh` then re-stubs `zoffset.h` /
`voffset.h` via `ensure_boot_offset_stubs`, which changes `header.S` and
whether `.pecompat` exists at all:

| tree state | `.text` | `.inittext` | `.rodata` | `.pecompat` | `_end` |
|---|---:|---:|---:|---:|---:|
| before any kernel build | 22598 | 406 | 1387 | 9 | 31168 |
| after `build_kernel_vm.sh` | 20329 | 397 | 1328 | — | 28048 |

Same compiler family, 3120 bytes apart. **A gate number is only meaningful
paired against the same tree state** — re-run both arms back to back
(`build_kernel_boot.sh` with `LCCC=`/`LCCC_LD=`/`OUT=` overridden per arm,
as the table above was produced), or use `scripts/boot_ab_size.py`, which
compiles both arms in one process and cannot see a stale header.

**New tool: `scripts/boot_ab_size.py`** — paired LCCC-vs-LCCC A/B over the
boot corpus. `boot_size_oracle.sh` compares against a *reference compiler*,
whose 10.7 KiB gap swallows any 100-byte movement; this compares two lccc
binaries (or one binary under two environments) on the identical command
lines, which is the only way a codegen change of this size is visible at
all. Self-A/B (same binary both arms) returns `+0` on all 20 objects, so
the harness is deterministic.

### 5b. The defect this pass shipped with, and the gate that now catches it

The first version emitted `movzwl%bx, %eax` — `prefix.trim_end()` removed
the mnemonic's trailing space and the format string never put it back. **It
passed every text-level assertion**, including the new positive test,
because the peephole's own classifier files an unparseable line under its
`Other` catch-all. It was caught only by compiling the boot corpus: eight
translation units died with
`unhandled i686 instruction: movzwl%bx, [Register(...)]`.

The lesson is the same one session 58 §6a recorded for the boot harness, in
a new place: **a check that inspects the producer's own output with the
producer's own vocabulary cannot detect output the consumer rejects.** The
fix is a third test, `folded_copy_extension_text_reaches_the_assembler_parseable`,
which routes `peephole_optimize`'s result through
`crate::backend::i686::assembler::assemble` — the actual next consumer.
Mutation-checked: re-introducing the missing space makes that test fail and
only that test.

### 5c. An environment note that changes what the suite means

`gcc-multilib` / `libc6-dev-i386` / `libc6-i386` / `lib32gcc-14-dev` were
absent, so every `-m32` regression test **skipped** — the vacuous-pass trap
`engineering/subsystems/i686.md` §"Host contract" warns about (PR #426's CI
head was exactly this). Installing them moved the suite from
**737/0/8 → 742/0/8**. Any i686 claim made on a host without multilib is
worth nothing; check `dpkg -l | grep libc6-dev-i386` before believing one.

---

## 5d. Bootstrap traps for the next session (the harness wipes everything)

Verified on this host, in the order they bite:

* **Size the swap file against the disk, not against ambition.** The disk
  here is a *single* 25.8 GiB `/dev/vda`; there is no second drive to move
  to. An 18 GiB `/swapfile` left ~5 GiB for the toolchain, the kernel tree
  and the build, and the very first `rustup` run died with
  `failed to extract package: No space left on device`. **12 GiB is the
  measured-good size**: swap is on, and ~5 GiB stays free for a full
  `bzImage` build (which peaks around 3.5 GiB of tree growth).
* **A sparse swap file does not work.** `swapon` refuses it outright —
  verified: `swapon: /tmp/holetest: skipping - it appears to have holes`,
  exit 255. `truncate -s` is not a way to get a big swap on a small disk;
  `fallocate`/`dd` is.
* **`/sbin` is not on `PATH`** in this image, so bare `swapon`/`mkswap` are
  "command not found" even as root. `scripts/ensure_swap.sh` already calls
  them by absolute path; ad-hoc shell work needs `PATH=$PATH:/usr/sbin:/sbin`.
* **`/tmp` is a 993 MiB tmpfs**, not disk. Anything large (rustup staging,
  kernel tarballs, A/B object trees) must live under `/home/user` or `/opt`.
* **Keep the ~55k-file kernel tree outside the workspace snapshot.** It is
  capped (~128 MiB / 10k files) and a truncated tree fails deep inside a
  build. `/opt/kwork/linux-6.18.52` +
  `PKG_ROOT=/opt/archpkgbuilds` is what this session used; the same tree on
  `/dev/root`, so it is a snapshot-scope decision, not a space one.
* **Install `gcc-multilib libc6-dev-i386 libc6-i386 lib32gcc-14-dev` before
  believing any i686 result** (§5c), and `zstd dwarves busybox-static`
  before starting a kernel build — `zstd` missing fails at
  `vmlinux.bin.zst` ~25 minutes in, and `busybox-static` is what
  `qemu_boot_test.sh` builds its initramfs from.
* Add `/swapfile none swap sw 0 0` to `/etc/fstab` so a re-created root
  filesystem does not need the swap re-derived by hand.

## 6. Open items, in the order the evidence supports

1. **i686 register-allocation quality is the boot-size gap.** Not a
   peephole problem: +505 slot stores, +281 slot loads, 70/70 functions with
   a frame vs gcc's 30/67, and `elig=true` values given slots anyway. The
   two candidate levers, in ascending risk: (a) make the greedy scan
   pressure/priority aware so short-lived in-loop values win over
   function-spanning ones; (b) shorten live ranges by emitting
   register-direct ALU more often, which needs the emitter to stop routing
   every operand through `%eax`. Do (a) first; it is contained in
   `regalloc.rs`. **Do not** retry the accumulator no-home relaxation (§4).
2. **x87 ↔ GP pair lowering** (`subsystems/i686.md` §2.1) remains the
   dominant *runtime* i686 item and is untouched by this session.
3. **The `free_large_kmalloc` question is not closed** (§2). Run the gdbstub
   probe against the current image before writing it off.
4. **`reg-to-reg-movl` +230** is the next census line after slotting. 205 of
   549 such moves in the `-S` output are dead on arrival by a straight-line
   scan, but only 26 of 74 survive into the *integrated* object — i.e. the
   existing `eliminate_dead_reg_moves` already removes most of them, and the
   `-S` count is not the available win. Measure on objects, not on `-S`
   (session 58 §4's rule, re-confirmed).
5. **`compare-against-frame-slot` +29** (`cmpl $imm, N(%esp)`): each is a
   reload that a register home would remove. Same root cause as 1.

## 7. Gate status at the end of this session

| gate | result |
|---|---|
| `cargo test --profile fastbuild --locked --lib` | **3111 passed, 0 failed, 7 ignored** (incl. the 3 new peephole guards) |
| `scripts/run_regression_suite.sh` | **PASS=742 FAIL=0 SKIP=8**, AB-diff 0 |
| boot gate (in-suite) | **PASS**, pre-pecompat end 21525 ≤ 24576, `.text` 20329 |
| `scripts/build_kernel_vm.sh` | **SUCCESS 1798 s**, bzImage 5071872 B |
| `scripts/qemu_boot_test.sh` | **16/16 PASS**, clean poweroff, no WARNING |
| `scripts/ci_local.sh --fast` | **57 passed, 0 failed, 3 skipped — ALL GATES GREEN** |
