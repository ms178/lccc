# Follow-up: kernel-boot C6 — `-Os` aggregate-frame inlining, boot-stage bisection, and the remaining upstream work

Session date: 2026-09-07
Base: `adecdaeda2c9363011ec4e4baefc60995cbb6454` (upstream `main`, PR #434 merged — re-verified this
session with `git fetch origin main`; `FETCH_HEAD` == base, so the series is current, not rebased).
Snapshot head: `396faeb375424e2afb7d333a4481797ebb911a6d` (`S03-os-agg-frame-veto`)
Deliverable: `/home/user/ms178-1.patch` — 60197 bytes,
sha256 `5390d3c1b2546d3f5d1a0a6ac347e7ee1442a2b926c9ee35ffa84901ff5a753f`, verdict APPLIES-CLEAN.

---

## 1. What landed this session (validated)

### 1.1 C2 SIMD/ISA gate (snapshot `S02`, unchanged)

The vectorizer and the FMA fold now honour the TU's ISA flags. Verified again this session:
`check_vectorize_isa_gate.sh` → PASS; default `-O2` still emits 2× `vfmadd` for `fma_isa_gate.c`,
and 0 under `-mno-sse -mno-mmx -mno-sse2 -mno-avx`.

### 1.2 `-Os` aggregate-frame inlining veto (snapshot `S03`, new)

**Root cause.** The `-Os` cost model counts callee instructions and never looks at the callee's
stack frame. A plain-static callee whose *only* cost is a large non-promotable alloca is therefore
cheap to inline — and each call site materialises the frame again.

**Symptom** (`arch/x86/boot/tty.c`, real-mode, `-m16 -Os -mregparm=3`): `getchar_timeout` inlined
`gettime()` and `kbd_pending()`, each carrying a 44-byte `struct biosregs`, producing 5 frame
copies and `sub $0x10c,%esp` — 290 bytes against GCC's 71. GCC keeps both callees outlined
(`nm` shows them as `t`); LCCC emitted no out-of-line copy at all.

**Fix** (`src/passes/inline.rs`). `CalleeData` gains `agg_frame_bytes` (sum of alloca sizes that
cannot be promoted to SSA) and `single_call_site`. `aggregate_frame_should_stay_outlined()` vetoes
a site when the TU is size-optimised, the callee is multi-site, `agg_frame_bytes >=
AGG_FRAME_SLOT_BYTES` (16), and nothing forces the inline (always_inline / gnu_inline /
PGO-forced / single call site all exempt). Both `select_inline_site` admission chains consult it,
so the `-Oz` and PGO paths stay consistent.

**Measurement.** Same-config A/B, 23-object real-mode corpus, GCC oracle, identical flags. The
"before" compiler was rebuilt from `git show adecdae:src/passes/inline.rs` (S02 did not touch
`inline.rs`, so upstream's copy *is* the pre-veto state) rather than compared against numbers
taken under a different `.config`:

| metric | pre-veto | with veto | GCC |
|---|---|---|---|
| `getchar_timeout` | 290 B | **119 B** | 71 B |
| `tty.o` `.text` | 721 B | **629 B** | — |
| `video-vga.o` `.text` | 1723 B | **1259 B** | — |
| boot `.text` total (23 objects) | 24876 B | **24320 B** (−2.2 %) | 13479 B |

The gap to GCC on `getchar_timeout` narrowed from +219 B to +48 B. Only `tty.o` and `video-vga.o`
changed; their −556 B accounts for the whole total delta.

**Gates (all re-run this session).** `cargo fmt --check` clean · `clippy --all-targets -D warnings`
clean · `cargo test --all-targets` **2020 passed / 0 failed / 6 ignored** (one more than S02 — the
new `aggregate_frame_veto_is_size_optimized_and_multisite_only` unit test) ·
`run_regression_suite.sh` **PASS=648 FAIL=0 SKIP=15, AB-diff failures 0** · boot gate
**PASS (pre-pecompact end 23910 ≤ 24576; .text 22714)** · `check_vectorize_isa_gate.sh` PASS.

### 1.3 Tooling and infra fixes

* **`scripts/boot_stage_bisect.sh` (new).** Relinks only `arch/x86/boot` against the already-built
  payload using the commands Kbuild recorded in `arch/x86/boot/.*.cmd`, then boots under QEMU.
  A compile → link → verdict cycle costs ~20 s instead of a ~12 min kernel rebuild, and per-file
  compiler selection (`BOOT_BISECT_ORACLE_FILES`) plus per-linker selection (`BOOT_BISECT_LD`)
  and per-assembler selection (`BOOT_BISECT_ASM_CC`) turn a silent bzImage into a binary search.
* **`prepare_kernel_tree.sh` host preflight.** `make olddefconfig` died on a missing `flex`
  *after* the 155 MiB download and the 26-patch apply. The preflight now runs first.
* **`build_kernel_vm.sh` preflight** gains `flex`/`bison` (kconfig) and `pahole`
  (only when `CONFIG_DEBUG_INFO_BTF=y`).
* **`fma_isa_gate.c` gains a `main()`** pinning fused vs unfused semantics
  (`(1+2^-12)^2 - (1+2^-11) = 2^-24` fused, `0` unfused) plus a `-lm` `.flags` sidecar, so the
  regression suite can build, run and A/B-compare it. It previously failed the suite with
  `undefined reference to 'main'` — a defect in the S02 test, found and fixed here.

---

## 2. Boot status: builds, passes the size gate, **still does not boot**

State at the point of the harness wipe (all reproduced once; re-verification needed on the
restored tree):

* Full kernel builds with `CC=lccc LD=lccc-ld`: 1057 objects LCCC-compiled (the 3 `gcc` objects
  are `arch/x86/tools/relocs_*`, HOSTCC by design), `vmlinux` 17,905,584 B,
  setup `_end` 30368 B of 32768 → **boot size gate PASS**.
* **C2 audit PASS**: 0 `xmm/ymm/zmm` across all 1011 LCCC-compiled C TUs. `vmlinux.o`'s ~5055
  SIMD refs are fully accounted for by 11 hand-written `.S` crypto/CRC objects.
* QEMU: serial log is 370 B (SeaBIOS → iPXE → "Booting from ROM..." → silence). 0 triple faults.
* Control: `/boot/vmlinuz-6.12.107+deb13-amd64` boots in 2.4 s on the identical QEMU line, so the
  harness is not at fault.

### 2.1 A claim from earlier this session was **wrong** and is retracted

I reported that substituting GCC for `arch/x86/boot` only — keeping LCCC's `vmlinux` and
decompressor — produced a booting kernel, and concluded the setup stage was the defect.
**That was false.** `make CC=gcc LD=ld.bfd bzImage` rebuilt **1015 translation units** with GCC
(verified from the build log: 92 in `lib`, 61 in `net/ipv4`, 58 in `mm`, …) and relinked the whole
payload. It was a full GCC kernel, so it proved nothing about LCCC's `vmlinux`.

### 2.2 What the corrected evidence actually shows

`scripts/boot_stage_bisect.sh` compiles the boot `.c` files with the flags read back from Kbuild's
own `.cmd` records. Those flag sets are **identical** for both compilers (43 flags each, set
difference empty), so the A/B differs in exactly one variable.

| setup stage | payload (`arch/x86/boot/vmlinux.bin`) | result |
|---|---|---|
| LCCC | GCC-built | **BOOTS** (kernel banner, initrd, both with and without `-initrd`) |
| LCCC | LCCC-built | **SILENT** |

⇒ **LCCC's real-mode setup stage is exonerated; the defect is in the compressed payload**
(`arch/x86/boot/compressed/*` and/or `vmlinux` itself).

Also worth recording because it invalidates an earlier line of reasoning: the VM/package config has
`# CONFIG_EDD is not set`, and even a *successful* boot prints nothing before the kernel banner
(verified: the booting arm has 0 occurrences of "Probing EDD"). The 32-byte silent serial log is
therefore the **expected** output for any failure at or after the decompressor, and says nothing
about the setup stage.

### 2.3 Next step (the open blocker)

Isolate decompressor vs `vmlinux` inside the payload. Both changed between the two rows above, so
the 2×2 is incomplete. The cheap decisive pair, now that `boot_stage_bisect.sh` exists:

1. LCCC decompressor + GCC-built inner image → if SILENT, the decompressor is fine and `vmlinux`
   is broken.
2. GCC decompressor + LCCC-built inner image → the converse.

Mechanically: `arch/x86/boot/compressed/vmlinux` is a link of ~10 objects plus `piggy.S`
(which `.incbin`s `vmlinux.bin.zst`), and `arch/x86/boot/vmlinux.bin` is an `objcopy` of it. Both
commands are recoverable from the build log, so the two arms can be assembled by hand in seconds
once one GCC-built inner image exists. Note that `make arch/x86/boot/compressed/vmlinux` is **not**
a valid top-level target ("No rule to make target") — descend with `make -C` or drive the link
directly.

---

## 3. Remaining upstream work

### 3.1 Blocker

* **[B1] Compressed-payload boot failure** — §2.3. Everything else in the kernel-boot thread is
  downstream of this.

### 3.2 Real-mode / `-Os` codegen (measured, not yet fixed)

All figures are `-m16 -Os -mregparm=3`, LCCC vs GCC, from `scripts/boot_size_oracle.sh`. After the
§1.2 fix the boot corpus is still 1.80× GCC (24320 vs 13479 B). Instruction-mix delta
(LCCC − GCC): `mov` **+1138 (≈55 % of the delta)**, `pop` +183, `lea` +131, `add` +100,
`movzw` +94, `movzb` +85, `jmp` +79 — i.e. the bulk is address/value materialisation, not control
flow.

| # | defect | evidence | size |
|---|---|---|---|
| D2 | Unused stack frame not elided. `memcmp` allocates a 44-B frame it never touches; the elision at `prologue.rs:1792` requires `space == callee_save_reserve`, so any other reservation blocks it. | `memcmp` frame present, unused | 40 vs 28 B |
| D3 | No leaf caller-saved register preference. `strlen` spills `%ebx`/`%esi` (callee-saved ⇒ push/pop pairs) where GCC uses `%ecx`/`%edx`. Explains part of the `pop` +183. | `strlen` prologue | 32 vs 24 B |
| D4 | Redundant reload across the loop backedge — missed GVN/load-PRE. | `strcmp` | 81 vs 53 B |
| D5 | Narrow compare not folded: `movsbl`×2 then a 32-bit `cmp`, where GCC emits `cmp %dl,%cl`. Part of the `movzb`/`mov` delta. | `strchr` | 70 vs 20 B |

D3 and D5 are not real-mode specific; they should pay off in the general `-Os` corpus too.
The largest single functions by absolute gap are `printf.vsprintf` (+1591 in `printf.o`),
`video.set_video` (+1372 in `video.o`), `string.o` (+1130) and `cpucheck.o` (+852).

### 3.3 Backend

* **[B2] Scalar FP under `-mno-sse` still emits `%xmm`** (pre-existing TODO,
  `src/backend/mod.rs:108`). Reproduced this session: `fma_isa_gate.c` under the kernel ISA flags
  has 44 `xmm/ymm/vfmadd` refs, 0 of them `vfmadd`. It needs an x87 path; it is why
  `vectorize_isa_gate.c` is integer-only. This is a latent kernel hazard of the same class as C2.

### 3.4 Carried over from `FOLLOWUP-2026-09-06` §2

2. Per-point fixed-GPR hazard table for `x86_inst_fixed_scratch`.
3. `Memcpy` / `AtomicRmw` / `Cmpxchg` operand eligibility.
4. `divconst` I128 `mulhi` bloat.
5. `live_range.rs` `register_touches_only` doc triplicated.
6. `scripts/` consolidation; 11 failing `check_*.sh`; LK-28 objtool; LK-30 encodings.
7. Unit tests for `x86_inst_fixed_scratch` and `stage_copy_operands`.

### 3.5 Test infrastructure (found this session)

* `run_regression_suite.sh` reports only the aggregate `SKIP=` count — no per-test reason. The
  count moved 7 → 15 between the S02 and S03 runs on this machine and I could not determine why
  from the output alone. Emitting `SKIP <name> (<reason>)` would make that diff legible.
* The regression suite links every `tests/regression/*.c`; a corpus file without `main()` fails as
  `undefined reference to 'main'` rather than being skipped. Worth asserting up front with a
  clearer message (see §1.3 — this is what caught the `fma_isa_gate` defect).

---

## 4. Harness notes (cost real time this session)

* The workspace was wiped mid-session. `/home/user/ms178-1.patch` and `artifacts/` survived;
  `.git`, `target/`, `/var/tmp` (kernel tree + tarball), the Rust toolchain, all apt-installed host
  tools, and swap did not. Recovery: `fallocate`+`swapon` the 8 G `/swapfile`, `chmod +x
  ~/.cargo/bin/*` (the wipe strips exec bits — `rustup` itself was unrunnable),
  `rustup toolchain install stable`, `git init` + `git fetch origin main` +
  `git reset --soft FETCH_HEAD`, restore the 6 files the snapshot dropped with
  `git checkout FETCH_HEAD --`, and `git update-index --chmod=+x` for the 77 scripts whose exec
  bits were stripped.
* **A wipe strips exec bits, so a naive `git add -A` afterwards stages 77 mode
  deletions (100755 → 100644).** Check `git diff --cached --summary | grep 'mode change'` before
  committing.
* `artifacts/lccc.bundle` from the previous snapshot was **truncated** ("did not send all
  necessary objects"); recovering history from GitHub worked. Do not rely on the bundle alone.
* Kernel-side: `make` does not rebuild when the compiler *binary* changes — delete the `.o` files.
  Kbuild `.cmd` sidecars are hidden and named `<dir>/.<obj>.cmd`. `objdump -d` on a `-m16` object
  decodes as 32-bit; use `objcopy -O binary --only-section` then `objdump -D -b binary -m i8086`.
  Debian's `awk` is `mawk` (no `strtonum`).
