# LCCC Engineering Backlog — "Maybach" edition

Rewritten 2026-10-24 against upstream `2070d23f` (`main`). It **supersedes** the
478-line live queue that preceded it, which is preserved unchanged at
[`engineering/archive/BACKLOG-2026-10-24-live-queue.md`](engineering/archive/BACKLOG-2026-10-24-live-queue.md);
the older 3,302-line queue stays at
[`engineering/archive/ROOT-BACKLOG-2026-10-17.md`](engineering/archive/ROOT-BACKLOG-2026-10-17.md).
Every open item of the superseded live queue is carried into §10 with its
acceptance criteria; nothing was dropped, only demoted. Closed entries, session
narratives and measurement journals live in the archives and in
[`engineering/journal/`](engineering/journal/README.md).
**GCC 16.2 parity register (added 2026-10-27, `dc878b3`).** §12 carries the
measured conformance and compatibility gap list produced by
[`docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`](docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md)
— 24 work items (`GP-01`…`GP-24`) covering attributes, macro environment,
builtins, warnings, tooling flags, inline asm, debug info, coverage, LTO and
sanitizers. Items that extend an existing MB entry are cross-referenced there;
nothing in §4–§10 is superseded.

Method doctrine: [`engineering/agent/RULES.md`](engineering/agent/RULES.md).
Do-not-retry grounds: [`engineering/DECISIONS.md`](engineering/DECISIONS.md).
Per-area knobs: [`engineering/subsystems/`](engineering/subsystems/README.md).

**This file holds open work only, written so an implementation agent can start
without asking questions.** An item with no reproducer or no stated acceptance
test does not belong here.

---

## 1. The target: what "Maybach" means for lccc / lccc-ld (x86-64 and i686)

A Maybach is not the fastest car; it is the one where nothing you touch feels
unfinished. For a C toolchain that means six properties, in this order:

| # | Pillar | Operational definition (testable) |
|---|--------|------------------------------------|
| 1 | **Trust** | Every accepted option either changes the artifact in the documented way or is refused with a diagnostic. No silent no-ops on semantics-, ABI-, or security-affecting flags. |
| 2 | **Completeness** | The flag set of a stock distribution build (Arch `makepkg.conf`, Debian `dpkg-buildflags`) compiles, links and *produces the hardening it asks for* (verifiable with `readelf`). |
| 3 | **Debuggability** | `-g` output is usable in gdb (frames, arguments, locals, types) and in `addr2line`/`perf annotate`; unwinding is correct through every generated function. |
| 4 | **Craft** | GCC-class diagnostics (precise, low false-positive), reproducible output, standard coverage up to the current default dialect (C23). |
| 5 | **Speed of the tool** | The compiler's own latency is never the reason a developer or CI job picks GCC. The linker is at least as fast as GNU ld and within 2× of lld/mold on the same inputs. |
| 6 | **Speed of the output** | Generated code beats GCC/Clang/ICX on the representative corpus; cost models are calibrated on the target microarchitecture. |

Pillars 1–3 are where lccc is furthest from the bar *today* and where the
effort/benefit ratio is best: they are mostly plumbing and policy, not research.
Pillar 6 already has a large, mature queue (see §6); this rewrite does not
re-litigate it.

### Evidence legend
- **R** reproduced on the baseline binary with the quoted command (fastbuild of
  `2070d23f`, rustc 1.91.1 + `-Zcrate-attr=feature(if_let_guard)` shim, 1 vCPU
  sandbox, GCC 13.3 / GNU ld 2.42 / mold / lld as comparison tools).
- **S** read from source; runtime consequence not demonstrated.
- **H** hypothesis; step 1 is the experiment.
- The i7-14700KF has **not** been measured by anything in this file. Absolute
  compile times below come from an `opt-level = 1` build of the compiler and are
  only used for *ratios and instruction-share profiles* (Callgrind), never as
  speed claims.

### Item format
`ID · class · evidence` — **Why** (impact), **Where** (files), **Plan**
(ordered, each step independently landable), **Gates** (tests to add; every new
gate must be mutation-verified: revert the fix, watch it fail), **Accept**
(measurable), **Risks**, **Effort/ROI**. Effort: S ≤ 1 day, M ≤ 1 week, L > 1 week
of agent time. ROI = impact × confidence ÷ effort, 1–5 scale in the table below.

### Standing constraints for every item
Build with the `fastbuild` profile, `-j2`, never concurrently with another heavy
job; run `scripts/ci_local.sh --fast` before declaring done; any change touching
code generation must also pass the identity/encoding gates named in
[`engineering/agent/RULES.md`](engineering/agent/RULES.md). Do not remove a
reproducer to close an item. The decision rules of the previous live queue still
bind every item here: correctness and evidence integrity outrank performance;
no runtime-speed claim from static instruction counts or Callgrind `Ir` alone
(they are simulated counts — in this file `Ir` is used **only** as a
deterministic proxy for the compiler's *own* cost, MB-20/MB-21, and every such
result must also be confirmed with paired wall time); never tune by workload
name; never weaken `-fwrapv`, alias, volatile or FP barriers to recover a
benchmark; a skipped oracle is missing evidence, not a pass.

---

## 2. Baseline snapshot (R, 2026-10-24, `2070d23f`)

Static census (`python3 scripts/audit_census.py`, as of `e8ae10a`; re-run for
the current numbers): ~530 source files; 888 panic-class sites (109 in
user-input-reachable code); 193/75/23 functions over 300/500/1000 lines.

**What the previous revision of this file reported and upstream has since fixed**
(re-measured on `2070d23f`; keep as regression rows): unknown or unimplemented
`-f`/`-m`/`-W` options now follow a measured, tiered policy documented at the top
of `src/driver/cli.rs` (hard error for contract flags — `-fstack-protector*`,
`-ftrapv`, `-fsanitize=`, and in practice `-fstack-clash-protection`,
`-fvisibility=`, `-flto`, `-fopenmp`; tolerated **with a warning** otherwise;
`LCCC_STRICT_OPTIONS=1` makes everything strict); `__builtin_cpu_supports` is now
a real runtime CPUID/XGETBV test with a cached result (`__lccc_cpu_ready`,
`__lccc_cpu_w7ebx`), no longer a Raptor Lake constant; C23 `constexpr`,
`nullptr`, keyword `bool`, digit separators, fixed-type enums and `typeof` under
`-std=c23` are accepted (40-snippet census: 39/40).

Dynamic probes on the current build (commands are runnable verbatim; `lccc` =
the built driver):

| Probe | Result | GCC reference |
|---|---|---|
| `-fstack-clash-protection`, `-fvisibility=hidden`, `-fsanitize=undefined`, `-flto`, `-fopenmp`, `-ftrapv`, `-fstack-protector` | hard error with a clear message (honest, but **not implemented**) | implemented |
| `-fno-plt` | accepted **silently**: no warning, no effect (inconsistent with the tolerated-with-warning tier) | indirect call via GOT |
| `-static-pie`, `-Wformat`, `-Werror=format-security` | accepted with a "tolerated but not implemented" warning | implemented |
| `__attribute__((target_clones("avx2","default")))` | accepted, no IFUNC symbol, no variants | IFUNC + resolver |
| `__attribute__((target("avx2")))` loop, `-O2` | **no `ymm`** (17 `xmm` refs); with `-mavx2` 13 `ymm` — the `e8ae10a` build emitted `ymm` for the same file: regression candidate, bisect first | GCC 13 `-O2` also emits no `ymm` (no vectoriser at `-O2`) |
| `-g -O0 -c` | **0** `.debug_*` sections | line + info |
| gdb on a linked `-g` binary | `bt` works; "No symbol table info available" for args/locals | full |
| `-Wall -Wextra` | still missing `-Wformat`, `-Wsign-compare`, `-Wunused-variable` | present |
| `_Float16` | rejected | accepted |
| default executable type | `ET_EXEC` (no ASLR), no build-id | `ET_DYN` (PIE), build-id |
| `-static-pie` (x86-64 and `-m32`) | warning, then silently `ET_EXEC` | `ET_DYN` static-pie |
| `-Wl,-z,pack-relative-relocs`, `--pack-dyn-relocs=relr`, `--image-base=0x10000000`, `-z max-page-size=65536`, `--oformat=binary` | accepted **silently**, no effect (no `DT_RELR`; entry stays `0x401450`; `LOAD` align stays `0x1000`; still ELF) | honoured (bfd) |
| `-m32` link | clear errors: `-pie`, `-Map`, `--version-script`, `--emit-relocs`, `--exclude-libs`, `--build-id=uuid`, `-r` | all work |
| 14 `-m32` run-and-compare programs (`-O0`, `-O2`) | all identical to `gcc -m32` (measured on `e8ae10a`; re-run as the first step of MB-32) | — |
| 800 objects (6.3 MB) link, 1 vCPU (`e8ae10a`) | lccc 0.08–0.09 s; bfd 0.15; mold 0.14; lld 0.045–0.09; outputs run, sizes within 1 % | — |
| Callgrind, 40-function TU, `-O1` (`2070d23f`) | 961 M instructions (was 1,232 M); **68 %** in `peephole_optimize_inner`; **32.5 % in `LabelRefs::build` called from `label_is_fallthrough_only` inside `combined_local_pass`**; `FileLiveness::new` fell from 41 % to 4.8 % and `fold_copy_into_lea_base` from 20 % to 2.3 % (upstream's liveness cache); `str::starts_with` ≈ 10 % | — |
| 300-function TU, `-O1`, `CCC_TIME_PEEPHOLE=1` | peephole 2.08 s of 2.81 s total (74 %); GCC `-O1` 0.77 s (ratio only: fastbuild compiler) | — |
| `-O0` vs `-O1`/`-O2` compile latency (`e8ae10a`, 80 TUs, twice) | `-O0` slower: 7.4 s vs 5.8–6.5 s; `.text` 4049 vs 2705 B — **re-measure** before acting (MB-21) | — |

**Re-measured 2026-10-27 on `dc878b3`** (this supersedes the stale rows above;
method and raw data in
[`docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`](docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md)):

| Probe | Result now | GCC 16.2 reference |
|---|---|---|
| `-Wall -Wextra` on ordinary code | `-Wformat`, `-Wsign-compare`, `-Wunused-variable`, `-Wformat-security`, `-Wint-conversion`, `-Wreturn-mismatch` present; `-Wuninitialized`, `-Wshadow`, `-Wconversion`, `-Wswitch`, `-Wvla`, `-Wreturn-local-addr`, `-Wmissing-prototypes`, `-Woverflow` absent | present |
| `-g -O1 -c` | `.debug_line` only (DWARF **v4**, empty directory table); **no** `.debug_info` → no types, locals or parameters in gdb | DWARF 5 with `.debug_info`/`.debug_abbrev`/`.debug_str` |
| `-ftest-coverage` / `-fprofile-arcs` (each alone) | accepted but **re-routed to the PGO path**: `lccc: PGO: instrumented N functions … into lccc-*.profraw`, **no `.gcno`/`.gcda`, no `__gcov_*`** — a coverage job gets a PGO profile | gcov data + `__gcov_*` runtime |
| `-fprofile-generate` | works, own `.profraw` format (GCC cannot read it) | `.gcda` |
| `_Float16`, `__bf16`, `_Decimal32/64`, `__float128`, `_Complex` | accepted (previously reported as rejected) | accepted |
| `-pedantic`, `-Ofast`, `-Og`, `-static-pie` | accepted with "not implemented" warning, no effect | implemented |
| `-O3` vs `-O2` | byte-identical output on the probe sample (no extra passes) | `-O3` adds unroll/vectorise |
| unknown/ignored/misapplied `__attribute__` | **silent** (no `-Wattributes`), e.g. `__attribute__((frobnicate)) int y;` | `warning: 'frobnicate' attribute directive ignored [-Wattributes]` |
| `__attribute__((ms_abi))` | silently ignored → **cross-compiler ABI break**: GCC caller + lccc callee prints `1216891234` instead of `47`; lccc caller + GCC callee segfaults | correct Microsoft ABI |
| `__attribute__((retain/noinit/persistent))` | silently ignored; objects land in `.data`/`.bss` | `.data.retain`, `.persistent` |
| `__has_builtin` differential | 607 visible vs 1 696 (`-march=x86-64-v3`: 607 vs 2 171); 129 names over-claimed | — |
| `-W` option surface | 18 of 549 positive options accepted; 12 live warning kinds; 535 tolerated | 549 |
| driver option surface (`--help=*` catalogue) | 157 accepted, 1 051 tolerated-no-op, 171 refused, 10 unrecognized (of 1 389) | — |
| `-dumpversion` / `__GNUC__` | `14` / `14` (`__VERSION__ "14.2.0"`) | `16` |
| `echo \| lccc -dM -E -x c -` | defines 211 **libc** macros GCC never defines (`BUFSIZ`, `EOF`, `NULL`, `CHAR_BIT`, `INT8_C(x)`, errno values) and `__STDC_NO_ATOMICS__`; misses 155 GCC macros | reference set |
| inline-asm constraints | no validation: `"j"`, `"V"`, `=` on an input are all accepted | `impossible constraint in 'asm'` / `input operand constraint contains '='` |
| digraphs / UCN identifiers | rejected (`%:`, `<:`, `int caf\u00e9`) | accepted |
| `_BitInt(N)` | rejected (unchanged; MB-11) | accepted |

What is **good** and must not regress: i686 code generation and ABI behaviour
on the 14-program differential set; link speed; object determinism across build
directories (R: identical objects from two directories with `-g`); the
diagnostic format (`file:line:col`, caret); `-Werror=` plumbing;
`--version-script`, `-Map`, `--wrap`, `--defsym`, `-rpath`, `--hash-style`,
`--dependency-file` in `lccc-ld`; `-ffile-prefix-map`.

## 3. Roadmap and ranking

ROI = (impact × confidence) ÷ effort, each 1–5; higher is better.

| Rank | ID | Title | Pillar | Effort | ROI |
|---:|---|---|---|---|---:|
| 1 | MB-01 | Finish the option contract: silent `-fno-plt`, OPTIONS.md, probe rows | 1 | S | 4.6 |
| 2 | MB-02 | Linker option contract + the five dead flags | 1 | M | 4.5 |
| 3 | MB-20 | Peephole compile time: hoist `LabelRefs`, decode once | 5 | S–M | 4.8 |
| 4 | MB-04 | PIE by default, real `-static-pie`, default build-id | 2 | M | 4.2 |
| 5 | MB-05 | `-fstack-clash-protection` (x86-64 + i686) | 2 | S–M | 4.0 |
| 6 | MB-03 | ~~`__builtin_cpu_supports`~~ fixed upstream; keep regression gate only | 1 | S | — |
| 7 | MB-10 | DWARF: line tables in the native assemblers, then DIEs | 3 | L | 3.8 |
| 8 | MB-13 | i686 linker parity (reuse `linker_common`) | 2 | M | 3.6 |
| 9 | MB-06 | Stack protector canaries (incl. kernel guard options) | 2 | M | 3.5 |
| 10 | MB-12 | Warnings: `-Wformat*`, unused, sign-compare | 4 | M–L | 3.4 |
| 11 | MB-11 | C23 conformance inventory and half-format edge cases | 4 | S–M | 3.3 |
| 12 | MB-07 | `-fvisibility`, `-fno-plt`, `-ftrapv`, trap-mode UBSan | 2 | M | 3.2 |
| 13 | MB-14 | x86-64 linker: DT_RELR, page-size, image-base | 2 | M | 3.2 |
| 14 | MB-16 | `target_clones` / IFUNC multiversioning | 6 | M | 3.0 |
| 15 | MB-32 | i686 differential gate (expand the 14 programs) | 3 | S | 3.0 |
| 16 | MB-15 | Whole-program mode (`-fwhole-program` / honest `-flto`) | 6 | L | 3.0 |
| 17 | MB-21 | `-O0` slower than `-O1` | 5 | S | 2.8 |
| 18 | MB-33 | Linker differential oracle (bfd/lld/mold/lccc-ld) | 1 | M | 2.8 |
| 19 | MB-30 | Raptor Lake cost-model calibration from published tables | 6 | L | 2.5 |
| 20 | MB-41 | Backlog format gate + archive discipline | — | S | 2.5 |

The GCC-16.2 conformance half of the programme is ranked separately in §12
(new items `GP-01`…`GP-24`, measured 2026-10-27). Its first five in ROI order are
GP-22 (option-effect ledger, 4.5), GP-01 (`-Wattributes` truth, 4.6 — but GP-22
first because it makes every later item a ratchet), GP-05 (macro hygiene, 4.4),
GP-23 (macro-diff gate, 4.0), GP-09 (warning registry, 4.0).

Suggested start for a single agent with one session: MB-20 (hoist `LabelRefs`:
a one-function change worth ~30 % of compile instructions) → MB-01 → MB-05; for two agents in parallel: [MB-01, MB-04, MB-13] on the driver/linker
side and [MB-20, MB-21, MB-05] on the backend side (disjoint files).

---

## 4. Tier M0 — Trust: promises must match artifacts

### MB-01 · P0 · R — finish the option contract (the policy exists; its edges do not)

**Why.** Upstream landed a measured, tiered policy (header of `src/driver/cli.rs`):
contract flags are hard errors, everything else unimplemented is tolerated with a
warning, `LCCC_STRICT_OPTIONS=1` makes it strict, and `scripts/lccc_capability_probe.py`
establishes capabilities by running the compiler. **Do not reopen the "unknown flag
must fail" question** — the policy's argument (16 of 93 flags that kernel, glibc,
zlib-ng, gzip and expat pass unconditionally had no arm) is recorded and measured.
The remaining defects are at the edges of that policy:

1. `-fno-plt` is accepted with **no warning and no effect** (R), unlike its
   neighbours in the tolerated tier.
2. The linker-side equivalent is wider: `-Wl,` options such as
   `-z pack-relative-relocs`, `--image-base`, `-z max-page-size`, `--oformat=binary`
   are swallowed silently (MB-02).
3. Nothing tells a user *which* options are implemented; the answer exists only
   inside the capability probe's manifest.
4. The "tolerated" tier mixes presentation namespaces (`-fdiagnostics-*`) with
   options that change the artifact (`-fno-plt`, `-fvisibility-*` family,
   `-ftls-model`, `-fzero-call-used-regs`, `-ftrivial-auto-var-init`); only the
   first kind is harmless to ignore.

**Where.** `src/driver/cli.rs` (tier arms), `scripts/lccc_capability_probe.py`,
[`engineering/SCRIPT-MEASUREMENT-MAP.md`](engineering/SCRIPT-MEASUREMENT-MAP.md).

**Reproducer.**
```bash
echo 'int f(int x){return x+1;}' > x.c
lccc -fno-plt -c x.c -o x.o; echo $?          # 0, nothing printed (siblings print a warning)
lccc -fPIC -fno-plt -S x.c -o - | grep -c GOTPCREL
```

**Plan.**
1. Give `-fno-plt` (and every other artifact-changing option found in step 2) the
   same tolerated-with-warning treatment, or implement it (MB-07b).
2. Enumerate the tolerated tier mechanically: `gcc -Q --help=common,target` option
   names × the capability probe; classify each as `presentation | artifact-changing`.
   Artifact-changing options must warn at least once per invocation and be listed in
   the document of step 3.
3. Generate an options document (to be created, e.g. docs/OPTIONS.md) from the
   probe manifest + the tier table: option, class, since-version, note. Add a gate
   that fails when the document is stale.
4. Add the probe's rows from §2 (flag → observable effect) to the capability probe
   as `effect:` capabilities, so "accepted" and "has the documented effect" are
   separate columns.

**Gates.** The probe runs in strict mode in `ci_local.sh --fast`'s existing probe
slot; new rows must be mutation-verified (turn the option into a silent no-op and
watch the `effect:` row fail). **Accept.** No artifact-changing option is accepted
silently; the options document is generated and current. **Effort** S · **ROI** 4.6.

**SSE option-contract progress (2026-10-25).** Independent SSE1 permission
now enables packed F32 maps under `-mno-sse2`, including either ordering with
`-msse`, without enabling integer/F64 SIMD. The focused native gate checks
observable packed instructions, exact results, aliases, page ends and scalar
ablation, not just option acceptance. SSE1 scalar parameter capture and the
store-forwarding cross-domain rewrites also respect that permission. Tiny
constant-trip maps now share the main entry's named-address-space veto;
a native GS-relative regression catches the former default-space load. See
[`SSE1 evidence`](engineering/evidence/sse1-f32-map-2026-10-25/README.md).
This is a bounded MB-01/UP-7 contribution, **not** closure of the option inventory,
all-architecture audit or full comparison backlog.

### MB-02 · P0 · R — linker option contract and no silent no-ops

**Status (re-audited 2026-10-27).** The no-op findings on `e8ae10a` and
`2070d23f` are historical, not the current checkout. The x86-64 path now emits
RELR for `-z pack-relative-relocs` / `--pack-dyn-relocs=relr`, carries the
`GLIBC_ABI_DT_RELR` version need, honors `--image-base` and 64-KiB
`-z max-page-size`, and writes `--oformat=binary` output identical to
`objcopy -O binary` of the same LCCC ELF. Regression cases now pin the RELR
loader contract, every `PT_LOAD` alignment, the image-base address, and raw
output bytes. The available GNU BFD accepted the 64-KiB page-size option but
rejected ELF `--image-base`; the image-base test therefore asserts LCCC's effect
without claiming BFD parity.

`--fatal-warnings` is parsed and now promotes deferred unimplemented-option
warnings regardless of argument order. `--warn-common` no longer disappears in
the benign-ignore list: standalone `lccc-ld --warn-common` and the compiler
driver's `-Wl,--warn-common` path explicitly warn that the requested
common-symbol diagnostic is not implemented; the linker test verifies fatal
promotion in both flag orders on both paths. The selected
`-z common-page-size[=N]` policy is also explicit:
warn that its separate layout granularity is ignored, validate BFD-compatible
zero/power-of-two spellings, reject malformed values, and promote the warning
under `--fatal-warnings`. Regression coverage spans x86-64 built-in/shared/
script and i386 built-in/shared/script links, plus the compiler driver's
`-Wl,` path. AArch64 and RISC-V built-in backends now share the validation and
warning policy; AArch64 was smoke-checked with the pinned assembler, while a
RISC-V cross-link fixture remains unavailable. The strict linker suite now
reports 313 pass, 0 fail, 0 warn, and 0 skip with GCC 16.2, BFD/mold oracles, and the
pinned BFD 2.47 fallback for the CODE_4_GOTPCRELX case. This is the focused
linker suite, not a completed `ci_local.sh --fast` run. The remaining issue is
the *whole linker option surface*: there is still no generated BFD/LLD/mold
option union or exhaustive `Implemented | BenignNoop | Refused | WarnUnimplemented`
inventory; the remaining benign-ignore entries and `--O1/--sort-common`
behavior need evidence and justification.

**Status (re-audited 2026-10-30, artifact-verified).** A 16-option sweep of the
x86-64 builtin path (`readelf -lW/-dW/-nW/-SW` on every product, each compared
against GCC 16.2.0's BFD link of the same objects) found exactly two options
that still accepted and then did nothing:

1. `-z execstack` was parsed (`args.rs`) and honoured by the i686 backend, but
   the x86-64 writers derived `PT_GNU_STACK` from the input `.note.GNU-stack`
   markers alone (`emit_exec.rs`) or hardcoded `PF_R|PF_W`
   (`emit_shared.rs`). The command line was therefore dropped in the one
   direction that changes a security property: a build that forced an
   executable stack got `RW`. Both writers now take the override, and the scan
   itself lives in `linker_common/stack_note.rs` so the two paths cannot
   drift. BFD parity is now exact, including the *diagnostic*: an input whose
   marker is executable widens the stack **and** is named
   (`<input>: requires executable stack (because the .note.GNU-stack section is
   executable)`), the line is suppressed once `-z execstack`/`-z noexecstack`
   settled the answer, and `--fatal-warnings` promotes it
   (`report_exec_stack_inputs` in `link.rs`). The shared-library path was the
   worse half of this bug: `-shared` from a trampoline-bearing PIC object
   produced `RW` where BFD produces `RWE`, i.e. a library that faults on the
   nested call it was built to support.
2. `--oformat=binary` through the **compiler driver** (`-Wl,--oformat=binary`)
   exited 0 and left an ELF file behind. The flat-image conversion already
   existed and `lccc-ld --oformat=binary` used it (and was tested); the driver's
   dispatch (`backend/common.rs::link_builtin_native`) simply never called it.
   It now does, producing bytes identical to `objcopy -O binary` of the same
   link. `-shared --oformat=binary` is refused (a flat image has no headers, so
   nothing can load it) and foreign encodings (`srec`, `pe-x86-64`, …) are
   refused **by name**, with the encoding this backend actually writes named in
   the message. BFD's deprecated alias `-b FORMAT` (which only ever arrives as
   `-Wl,-b,binary`) was not parsed at all and is now recorded by
   `parse_linker_args`, so it produces the same flat image instead of being
   dropped as an unknown token.

Both files were re-measured to confirm the *other* fourteen options are real:
`-z now` (DT_FLAGS BIND_NOW + DT_FLAGS_1 NOW), `-z ibt`/`-z shstk`
(GNU_PROPERTY), `-z nodlopen` (FLAGS_1 NOOPEN), `-z noexecstack`, `--hash-style=gnu`,
`--build-id=sha1`, `--no-undefined` (hard error), `--fatal-warnings`, `--image-base`,
`-z max-page-size`, `-z pack-relative-relocs`, and `--sort-common` (effective —
the earlier "no delta" reading was a probe artifact: it only reorders *common*
symbols, so the probe needs `-fcommon` input). `-z stack-size` and `-z muldefs`
warn correctly. `-z text`, `-z separate-code` and the lld-spelling
`--no-rosegment` produce no delta, which matches BFD itself (`-z separate-code`
has a zero delta on BFD, and BFD reports `-z no-rosegment ignored`); pinning
them as `BenignNoop` is the correct classification, not a fixed defect. DT_RELR
on PIE and `-shared` was re-verified as working end to end (`RELR`/`RELRSZ`/
`RELRENT` + `GLIBC_ABI_DT_RELR`, binaries exit identically to GCC's) — the
earlier "missing" reading came from probing without the flag.

Gates: the linker runner grew two assertions — `expect_gnu_stack` (emitted
`PT_GNU_STACK` flags, cross-checked against the BFD oracle for the same command
line) and `expect_diag` (a failing link must *name* the unsupported request).
Rows: `gnu_stack_default_is_nonexec`, `z_execstack_requested`,
`z_noexecstack_requested`, `gnu_stack_follows_input_marker`,
`z_noexecstack_overrides_input_marker`, `shared_gnu_stack_follows_input_marker`,
`oformat_binary_with_shared_is_refused`, `oformat_unknown_name_is_refused`,
`oformat_native_is_accepted`, `exec_stack_warning_and_header`, and the driver
flat-image byte comparison folded into `binary_output_matches_objcopy`. Every
one of the five new code paths was mutation-verified (fix reverted → the row
fails with the expected message).

**Why.** Without that inventory, additional semantic, ABI, and diagnostic
options can still be silently swallowed. Arch's default LDFLAGS include
`-Wl,-z,pack-relative-relocs` and `-Wl,-O1 --sort-common`; RELR is now covered,
but the broader option contract remains open.

**Where.** `src/bin/lccc_ld.rs` (`is_benign_ignorable`, `warn_unimplemented`),
`src/backend/linker_common/args.rs`, `parse_linker_args` users in the driver.

**Current regression reproducer.** With the fastbuild binary, the focused
linker cases pin the accepted effects and the warning-policy edge:
```bash
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter pack_relative_relocs
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter pack_dyn_relocs_relr_alias
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter max_page_size_64k
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter image_base_sets_first_load
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter binary_output_matches_objcopy
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter warn_common
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter common_page_size_warns_and_honors_fatal_warnings --strict
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter emit_relocs_warns_when_unimplemented --strict
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter exec_stack --strict
python3 tests/linker/run_linker_tests.py --lccc target/fastbuild/lccc --filter oformat --strict
```

**Plan.**
1. Same discipline as MB-01 for the linker: one table, classes
   `Implemented | BenignNoop(justification) | Refused | WarnUnimplemented`.
2. Harvest the option universe mechanically: `ld.bfd --help`, `ld.lld --help`,
   `mold --help` (all installable in the sandbox) → union list checked into a
   data file; the gate fails on any option that is in the union but absent from
   the table (no un-triaged options).
3. Finish classifying the probed flags: the MB-14 RELR, image-base, max-page-size
   and binary-output contracts now have focused x86-64 gates. `common-page-size`
   has an explicit warn-and-ignore policy, value validation, and cross-path
   fatal-warning coverage; continue auditing `-N/-n/--omagic/--nmagic`,
   `--print-memory-usage`, `--trace/-t`, and the actual `--warn-common` warning.
   Implement each or assign a justified `BenignNoop`, `Refused`, or
   `WarnUnimplemented` status.
4. Keep every `WarnUnimplemented` diagnostic deduplicated and explicit, and make
   final `--fatal-warnings` state promote it regardless of argument position;
   `warn_common_fatal_order` and the common-page-size gate cover both orderings.
   Expand this gate as the option table is populated.

**Gates.** Option-surface test (to be created, e.g.
scripts/ld_option_surface.py (to be created)): for each option with an `Implemented` class, run
a mini-link and assert the effect via `readelf` (examples in the reproducer);
for `Refused`, assert rc≠0 and message.
**Accept.** 0 silent no-ops among options present in bfd/lld/mold; the generated
surface inventory is mutation-verified, and every accepted/refused/warned option
has an effect or diagnostic gate. The focused RELR, image-base, page-size,
binary-output, and fatal-warning regressions remain green. **Effort** M ·
**ROI** 4.5.

### MB-03 · closed upstream — keep a regression gate for `__builtin_cpu_supports`

The previous revision of this file reported a Raptor Lake constant fold. On
`2070d23f` the builtin lowers to a real runtime test (CPUID leaves 1, 7, 0xd +
XGETBV, cached in `__lccc_cpu_ready`/`__lccc_cpu_w7ebx`). Residual work, all small:
(a) a gate asserting that `-march=x86-64` **and** `-mno-avx2` builds contain the
runtime path while `-march=raptorlake` may fold (`readelf`/asm shape), (b) check
`OSXSAVE`/XCR0 gating for the AVX/AVX-512 bits explicitly, (c) AArch64/RISC-V:
`__builtin_cpu_supports("avx2")` must be a diagnostic, not a value (re-run
`lccc-arm`, which earlier folded `avx2 + 10*bmi2` to `11`). **Effort** S.

### MB-04 · P0 · R — PIE by default, a real `-static-pie`, default build-id

**Why.** `lccc a.c -o a` yields `ET_EXEC` (R) where every mainstream GCC
configuration yields `ET_DYN`: no ASLR for any binary built by lccc. `-static-pie`
now warns "tolerated but not implemented" and still produces a non-PIE executable
on x86-64 **and** i686 (R): that tier is wrong for a hardening flag — it belongs in
the contract tier (hard error) until implemented. No `.note.gnu.build-id` by default (GCC distributions pass `--build-id`);
without it, debuginfod, `perf`, coredumps and `eu-unstrip` cannot match binaries.
`-pie` itself works on x86-64 (`ET_DYN`); i686 refuses it (see MB-13).

**Where.** `src/driver/cli.rs` (the `-pie`/`-no-pie`/`-static`/`-static-pie`
arms and `parse_linker_args` handoff), `src/driver/external_tools.rs`
(start-file selection), `src/backend/x86/linker/emit_exec.rs` (PIE emission),
`src/backend/linker_common/build_id.rs`.

**Plan.**
1. Policy switch `default_pie` (compile-time constant, overridable by env
   `LCCC_DEFAULT_PIE=0/1` for bootstrap scripts): when linking an executable and
   none of `-no-pie`, `-static`, `-shared`, `-r`, `-nostdlib`+`-T` is given, add
   `-pie` and use `Scrt1.o` instead of `crt1.o`.
2. `-static-pie`: start files `rcrt1.o`, flags `-static -pie --no-dynamic-linker
   -z text`; the linker must emit `R_X86_64_RELATIVE` (and, with MB-14,
   `DT_RELR`) for every absolute pointer in data, apply `IRELATIVE` entries in
   `.rela.iplt` through `__rela_iplt_start/end` as glibc's `_dl_relocate_static_pie`
   expects, and define `_DYNAMIC`/`_GLOBAL_OFFSET_TABLE_`. If this is not done in
   the first change, `-static-pie` must be **Refused** (MB-01 class), never
   silently non-PIE.
3. Default `--build-id=sha1` when the driver links (the linker already
   implements `sha1`/`tree`; add `uuid`/hex-string forms for parity, shared code
   with i686).
4. Make the object code PIE-clean: audit that `-fPIE` codegen of lccc never
   emits absolute 32-bit relocations in `.text` (`R_X86_64_32S`) — GCC's PIE
   mode guarantees this; verify with `readelf -r` over the benchmark corpus
   linked with `-pie -Wl,-z,text`.

**Gates.** `check_default_pie` (to be created): `readelf -h` type DYN, `PT_INTERP`
present, `FLAGS_1 PIE`; static-pie: `file` says "static-pie linked", runs, and
`setarch -R` vs default produce different `&main` printed addresses; build-id
note present and stable across two builds in different directories.
**Accept.** Corpus (zlib-ng, gzip, expat, sqlite harness) builds, runs and
passes with default PIE; kernel builds keep using `-no-pie`/`-T`.
**Risks.** Code with absolute addresses in asm; start-file availability
(`Scrt1.o`/`rcrt1.o` on the host). **Effort** M · **ROI** 4.2.

### MB-05 · P1 · R — `-fstack-clash-protection` (x86-64 and i686)

**Why.** Now a hard error ("Stack probing is not implemented", R) — honest, but
Arch's default CFLAGS and Debian hardening include it, so those builds fail
outright. Implementation is mechanical and well-specified.

**Where.** `src/backend/x86/codegen/prologue.rs` (frame allocation and
`calculate_stack_space_impl`), the dynamic `alloca`/VLA lowering in the x86 and
i686 codegen, CFI emission (`src/backend/cfi_synth.rs`).

**Plan.**
1. Static frames: if `frame ≤ 1 KiB` (no probe needed beyond the callee-pushed
   return address) leave as is. Else probe every 4 KiB: unrolled up to 4
   steps (`subq $4096,%rsp; orq $0,(%rsp)`), loop beyond
   (`mov %rsp,%r11; sub $N,%r11; L: sub $4096,%rsp; orq $0,(%rsp); cmp %r11,%rsp; jne L`), then allocate the residue and
   probe it if it is > 1 KiB. i686: same with `orl $0,(%esp)`.
2. Each `sub` adjusts the CFA: emit `.cfi_adjust_cfa_offset 4096` per step (loop:
   describe once with `DW_CFA_def_cfa_expression` or save-restore state), so
   unwinding is exact mid-prologue.
3. Dynamic allocation (`alloca`, VLA): allocate in 4 KiB steps with a probe each
   iteration, final partial block probed; never use a single `sub %reg,%rsp`.
4. Leaf functions using the red zone: keep the red zone only when the frame is
   ≤ 128 bytes (already true); no probes needed.
5. Interaction: `-mno-red-zone` (kernel) unchanged; `-fcf-protection` `endbr64`
   remains first; make sure peephole passes cannot delete or reorder the
   `orq $0,(%rsp)` probes (treat as a memory write with side effect, pinned).

**Gates.** (a) asm-shape test for 512 B / 8 KiB / 1 MiB / VLA frames on both
targets; (b) runtime test: run a 64 KiB-frame function on an `mmap`ed stack
(`makecontext`) whose lowest page is `PROT_NONE` with a writable victim region
below it — with probes `SIGSEGV` must occur; without them the victim region is
corrupted (this proves the probes work, which asm shape alone cannot); (c)
`gdb` backtrace taken *inside* the probe loop is correct (CFI).
**Accept.** Both tests green on x86-64 and i686; `-fstack-clash-protection`
moves from Refused (MB-01) to Implemented. **Effort** S–M · **ROI** 4.0.

### MB-06 · P1 · S — stack-protector canaries, including the kernel's guard options

**Why.** Refused today (correct, honest; `-mstack-protector-guard*` too), which means Debian/Ubuntu/Arch default
flags fail outright (`-fstack-protector-strong`) and the kernel's
`CONFIG_STACKPROTECTOR` is unavailable. A complete toolchain implements it; the
design below keeps it small.

**Where.** `src/backend/stack_layout/` (slot ordering), `src/backend/x86/codegen/prologue.rs`
and `returns.rs`, `src/backend/i686/codegen/`, `src/driver/cli.rs`.

**Plan.**
1. Eligibility per function: `-fstack-protector` = char arrays ≥ 8 bytes (or
   `alloca`); `-strong` = any local array, any local whose address is taken,
   VLA/`alloca`, local register variables; `-all` = every function;
   `-explicit` = `__attribute__((stack_protect))`; `__attribute__((no_stack_protector))`
   opts out.
2. Layout: place the canary directly below the saved frame pointer/return
   address; sort protected arrays *above* scalar locals so an overflow hits the
   canary before other variables (GCC's layout rule); spilled arguments copied
   below the arrays.
3. Prologue: `movq %fs:0x28,%rax; movq %rax,-8(%rbp-or-rsp);  xorl %eax,%eax`
   (i686: `%gs:0x14`). Epilogue: reload, `subq %fs:0x28,%rdx` (or `xor`), `jne` to
   one shared cold block per function calling `__stack_chk_fail` (noreturn).
   No tail call out of a protected function; no check before a `noreturn` call.
4. Options: `-mstack-protector-guard=tls|global`, `-mstack-protector-guard-reg=`,
   `-mstack-protector-guard-offset=`, `-mstack-protector-guard-symbol=` (the
   kernel passes `tls`, `gs`, `offset` and, on newer trees, a symbol).
5. Remove the refusal arms in `cli.rs` and flip MB-01 rows.

**Gates.** Smash test (`char b[8]; strcpy(b, argv[1])`) aborts with
`*** stack smashing detected ***` under both lccc and GCC for `-strong`,
`-all`; not for a function with only scalars under `-strong`; asm test that the
check is not removable by the peephole passes (mutation: add a pass that drops
the epilogue compare and watch the gate fail); kernel-style options accepted
and emitted. **Accept.** Debian and Arch flag sets compile; `scripts/` corpus
overhead measured (expect < 2 % on the benchmark set; report it).
**Effort** M · **ROI** 3.5.

### MB-07 · P1 · R — `-fvisibility`, `-fno-plt`, `-ftrapv`, trap-mode UBSan, `-fwrapv`

Split into independently landable parts; each ends by flipping its row in MB-01.

**MB-07a `-fvisibility=hidden|protected|internal|default` + `#pragma GCC visibility`
+ `__attribute__((visibility))` precedence.** Apply to definitions *and* to
`extern` declarations (hidden externs get direct calls/PC-relative accesses with
no PLT/GOT, a measurable size and speed win for shared libraries). Where: symbol
emission in `src/backend/x86/codegen/globals.rs` and the i686 equivalent; the
attribute parser in the frontend. Test: `readelf -sW` visibility column over a
matrix of flag × attribute × pragma; link a shared lib and verify `--dynsym`
exports shrink; `-fvisibility-inlines-hidden` is C++ only (Refused).

**MB-07b `-fno-plt`.** With `-fPIC`: calls to external functions become
`call *foo@GOTPCREL(%rip)` (i686: `call *foo@GOT(%ebx)`); the linker must keep
the `GOTPCRELX` relaxation (`call *foo@GOTPCREL(%rip)` → `addr32 call foo` when
`foo` binds locally; already supported in `src/backend/x86/linker/plt_got.rs`
if relaxation is implemented — verify first). Test: asm shape + runtime with
`LD_BIND_NOW` unset to prove lazy PLT is bypassed (`LD_DEBUG=bindings`).

**MB-07c `-ftrapv` and `-fsanitize=undefined` (trap mode first).** Stage 1 needs
no runtime library: `-ftrapv` lowers signed `+ - *` to the existing
`__builtin_*_overflow` machinery followed by `jo`/`ud2`; `-fsanitize-trap=undefined`
adds: signed overflow, shift out of range, unreachable, array-bounds (known-size
only), null-pointer deref where statically known. Stage 2: emit calls to
`__ubsan_handle_*` (link `libubsan` when present) with source-location records.
`-fsanitize=address|thread|memory|leak`: Refused (MB-01) — they need runtime
libraries and shadow memory; do not fake them. Test: seeded-bug programs per
check, rc/trap observed, no false trap on the corpus (all 39 benchmark workloads
must pass with `-fsanitize-trap=undefined`).

**MB-07d `-fno-strict-overflow` pointer half** (A14-16 tail). The archived
reproducer was **re-run on 2026-10-31 and no longer reproduces**:
`engineering/evidence/pr824-review/accepted-wrap-product.c` (`p[i * 4]` with `i`
from 2^30) exits 0 with the expected answer at `-O0`, `-O1`, `-O2` and `-O3`
under `-fwrapv` and under `-fno-strict-overflow`, byte-for-byte with GCC 16.2 —
the `-11` that
[`accepted-wrap-product-open.json`](engineering/evidence/pr824-review/accepted-wrap-product-open.json)
recorded is gone. It is now a running leg of
`tests/regression/check_iabs_cmp_fold.sh`, so a regression fails a gate instead
of waiting for a backlog review. What remains open is the *other* half of the
umbrella: `-fno-strict-overflow` also requests pointer-overflow semantics
(GCC documents `-fno-strict-overflow` as implying `-fwrapv` **and** disabling
pointer-overflow UB), and no pointer-overflow assumption is established or
audited here. That is why the flag still warns once per translation unit and
why `LCCC_STRICT_OPTIONS=1` refuses it; closing this item means auditing every
pointer comparison and pointer-plus-offset fold against `-fno-strict-overflow`
and then dropping the warning. `-fstrict-overflow` is not part of the item: it
asks for an assumption this compiler never makes, so normal mode accepts it
silently and only strict mode refuses it (see
[`docs/builtin-libcall-policy.md`](docs/builtin-libcall-policy.md)).

**MB-07e `-ftls-model=`, `-fzero-call-used-regs=`, `-ftrivial-auto-var-init=zero|pattern`.**
`-ftls-model` is cheap (select GD/LD/IE/LE per variable; the i686 linker already
has TLS relaxations in `src/backend/i686/linker/tls.rs`). The other two are
security options: either implement (zeroing used caller-saved registers before
`ret`; zero/pattern-fill of uninitialised automatic storage at declaration) or
refuse — never ignore. **Effort** M (a–e together L) · **ROI** 3.2.

---

## 5. Tier M1 — Completeness and debuggability

### MB-10 · P1 · R — DWARF line tables landed; DIEs and gdb remain

**Status (re-audited 2026-10-27).** The old "no `.debug_*` sections" claim is
stale for line tables. `tests/regression/check_dwarf_line_tables.py` passes with
GCC 16.2 and LCCC on x86-64, plus LCCC i686: it checks `.debug_line` rows,
`.rela.debug_line` / `.rel.debug_line`, multi-object address relocation,
non-allocated debug sections, `--strip-all`, and preservation/relocation of
GCC's full DWARF through executable and shared links. This is not evidence that
LCCC emits full DIEs for its own code: GCC's `.debug_info` is only checked for
preservation. `gdb`, pahole and objtool are unavailable in this harness, so no
locals/arguments/CFI/BTF inspection is claimed.

**Why.** LCCC-generated line tables now support basic source/address mapping,
but its own `-g` output still lacks a demonstrated usable DIE tree for gdb
(types, parameters, locals), and optimized location lists remain open. The
remaining debug gap is substantially narrower than the historical baseline.

**Where.** x86 and i686 assemblers (`src/backend/x86/assembler/`, i686 equivalent),
`src/backend/x86/assembler/elf_writer.rs` (section + relocation emission),
`src/driver/pipeline.rs` (`asm_contains_debug_records` honesty check),
`src/backend/cfi_synth.rs`; DIE generation in a new backend-independent module
fed by `CType`.

**Plan (each stage is shippable alone).**
1. **Line table (basic DWARF emission and relocation now landed).** Keep the
   existing `.file`/`.loc` mapping and x86-64/i686 relocation gates; finish
   coverage for `.loc` flags (`is_stmt`, `prologue_end`, `discriminator`,
   `view`), `.debug_line_str`, and `-gdwarf-4` directory/file tables. The current
   gate verifies decoded rows and multi-object address relocation, not every
   line-program feature.
2. **Minimal DIE tree at -O0 (M).** `.debug_info`/`.debug_abbrev`/`.debug_str`:
   compile unit (producer, language `DW_LANG_C11`, name, comp_dir, `low_pc`/`high_pc`
   or `DW_AT_ranges`, `stmt_list`); base, pointer, const/volatile, typedef,
   struct/union/enum, array (subrange), subroutine types; `DW_TAG_subprogram`
   (name, external, decl line, type, pc range, `frame_base = DW_OP_call_frame_cfa`);
   formal parameters and locals with `DW_OP_fbreg <slot offset>`; lexical blocks.
   Emit them as assembler directives (`.uleb128`, `.long label`, `.quad`) so
   stage 1's assembler consumes them.
3. **Optimised code (L).** Location lists from register-allocator live ranges
   (`DW_AT_location` → `.debug_loclists`), `DW_OP_piece` for split values,
   `DW_TAG_inlined_subroutine` with `call_file/call_line`, honest
   `<optimized out>` where no range exists.
4. **Extras.** `.debug_aranges`, `.debug_rnglists`, `-gz` section compression,
   `-fdebug-prefix-map`/`-ffile-prefix-map` applied to `DW_AT_name/comp_dir/decl_file`
   (must keep objects reproducible across directories — the current
   cross-directory determinism is a property to protect), `-gsplit-dwarf`
   either implemented or Refused, BTF generation for the kernel
   (`pahole`-compatible) as a follow-up.

**Gate status (2026-10-27).** `tests/regression/check_dwarf_line_tables.py`
now covers the decoded-line, object-relocation, multi-object, strip, executable,
and shared-link portions of (a)/(e); the full DWARF preservation checks also
run `readelf --debug-dump=info`. `gdb` and `llvm-dwarfdump` were unavailable,
so the remaining debugger/verification claims are still untested.

**Remaining gates.** (a) exact decoded `.loc` rows for every supported flag;
(b) `addr2line -e a.out <pc>` for every function entry of a 3-file program;
(c) gdb batch: `break g`, `run`, `bt`, `info args`, `info locals`, `print *ptr`,
`ptype struct S`, `finish` at `-O0`; (d) the same on i686 via `gdb` + `-m32`;
(e) mutation: corrupt one relocation and see the gate fail; (f)
`llvm-dwarfdump --verify` where available.
**Accept.** The line-table stage is already independently gated; the DIE stage
must land with a stable gdb test. Stage 3 has its own acceptance on the benchmark
corpus (≥ 90 % of parameters inspectable at `-O1`). **Effort** L · **ROI** 3.8.

### MB-11 · P2 · R — C23 conformance inventory and half-format edge cases

**Status (re-audited 2026-10-27).** The earlier 39/40 count and statement that
`_Float16`/`__bf16` were rejected are stale. The current tree implements both
half formats, their literals/conversions and ABI lowering, with targeted GCC 16
regressions including `gcc16_half_bfloat16_abi.c`, `gcc16_half_f16c.c` and the
half-math builtin tests. Those tests do not constitute the previously proposed
exhaustive 2^16-pattern × rounding-mode proof; no exhaustive coverage claim is
made. The default C dialect now follows GCC 16.2's `gnu23` (`__STDC_VERSION__`
202311L); the no-flag fixture and explicit `-std=gnu17` override are checked by
the regression corpus. Prototype status is preserved independently of an empty
parameter vector: default GNU23 `f()` rejects extra arguments, GNU17 `f()` keeps
its unspecified-parameter behavior, and `(void)` remains a zero-parameter
prototype in both. Conditional function-pointer composites retain a compatible
prototype from either arm. The external GCC compatibility macros/version banner remain
at 14.2.0: this dialect change is not a full-parity claim.

The refreshed GCC 16.2 matrix has 24 probes and one C23 difference:
`_BitInt(17)` (GCC accepts; LCCC rejects). This is an explicit, deferred feature
gap, not a false expected result or a syntax-only acceptance target. The current
fixed-width `CType` model cannot carry `_BitInt` precision and signedness through
integer conversions, constants, layout, ABI lowering, codegen and debug metadata.
The inventory documents the measured rejection and the end-to-end acceptance
criteria; until that work lands, keep the row visible and do not publish a C23
coverage percentage. The 2026-10-27 report is saved at
`/home/user/lccc-c23-inventory-2026-10-27.{json,md}`; the same run records six
C17-mode differences where LCCC accepts extensions that GCC 16.2 rejects:
`alignof-keyword`, `auto-type-inference`, `elifdef`, `elifndef`,
`one-argument-va-start`, and `static-assert-keyword`. See
`docs/C23-COMPATIBILITY-INVENTORY.md` for the dated method and classifications.

The historical per-function AVX2 vectorization candidate was remeasured with
`tests/regression/per_function_target_avx2_reduction/reduction.c`: at `-O2`
LCCC emits YMM instructions for the `target("avx2")` reduction without a
translation-unit-wide `-mavx2`; the adjacent baseline function stays YMM-free.
GCC 16.2 and
Clang 23.1.3 show the same function-level ISA boundary; the target function's
results match the scalar reference for trip counts 0–35 on all three. The
current regression is pinned in `check_vectorize_isa_gate.sh`. The earlier
`2070d23f` no-YMM observation does not reproduce on this branch, so no bisect
or performance claim is justified from that historical row.

Remaining work:

1. **Full `_BitInt` support** (deferred; see `docs/C23-COMPATIBILITY-INVENTORY.md`).
   Add a width-bearing type that preserves signedness and precision through
   integer promotions/conversions, constant evaluation, object/aggregate
   layout, ABI classification, lowering/codegen, and debug output. Cover widths
   around the built-in integer boundaries; arithmetic, shifts and conversions;
   arrays, aggregates and bit-fields; and calls/returns on each supported ABI.
   Keep `bitint-17` as the visible inventory difference until this end-to-end
   contract is implemented and differentially tested.
2. **Exhaustive half-format proof.** The existing targeted `_Float16`/`__bf16`
   regressions do not cover all 2^16 encodings across rounding modes. Add that
   proof before making exhaustive conversion claims.

**Gates.** Keep the 24-probe GCC 16.2 accept/reject matrix, default/gnu17
prototype differential, and the per-function AVX2 ISA-boundary/runtime test;
new `_BitInt` support must add compiler-oracle differential and ABI/layout
coverage rather than weakening the existing known-gap row. **Effort** M–L ·
**ROI** 3.3.

### MB-12 · P1 · R — warnings that real builds rely on

**Status (re-audited 2026-10-27).** The earlier missing-warning list is stale:
`-Wunused-variable`, core `-Wformat`, `-Wsign-compare`, and now
`-Wformat-security` have focused compatibility cases. The `-Wformat-security`
implementation diagnoses a non-literal printf-family format with no format
arguments, requires both `-Wformat` and `-Wformat-security` (GCC 16.2
behavior), and supports suppression, demotion, and `-Werror=format-security`.
`tests/regression/check_warning_compat.py` passes these cases against GCC 16.2
and LCCC. Clang 23.1.3 also warns for the same source (different diagnostic
text); the requested Clang 23.1.4 and ICX 2026.2 toolchains are unavailable in
the current harness, so no exact-version claim is made.

**Why.** The current gate is still a narrow compatibility sample, not a
complete warning inventory or corpus-wide precision proof. `-Wunused-but-set-variable`,
`-Wunused-function`, `-Wunused-parameter`, the full format subfamilies, warning
group expansion/defaults, and unknown `-W` naming need broader coverage before
real builds can rely on the entire GCC warning contract.

**Where.** `src/frontend/sema/analysis.rs` (semantic checks), warning kinds and
configuration in `src/common/error.rs` / `src/driver/cli.rs`, and the
`tests/regression/check_warning_compat.py` parity gate.

**Plan.**
1. Warning registry: finish a single table `(name, default-on, in -Wall, in
   -Wextra, description)` driving `-W<name>`, `-Wno-<name>`, `-Werror=<name>`,
   `-Wno-error=<name>`, the `[-Wname]` suffix, and `-Wall/-Wextra/-pedantic`
   expansion; unknown `-W` names follow MB-01. The focused `FormatSecurity`
   kind/config is now present, but the whole registry/group matrix is not done.
2. Extend unused diagnostics beyond the gated `-Wunused-variable` case:
   `-Wunused-but-set-variable`, `-Wunused-function` (static, not referenced),
   and `-Wunused-parameter` (`-Wextra`) need correct `used`/`set` state,
   respecting `(void)x`, `__attribute__((unused))`, `[[maybe_unused]]`, and
   `sizeof`/unevaluated contexts.
3. Expand the existing `-Wsign-compare` and core `-Wformat` coverage. For the
   format family (`printf`/`scanf`/`strftime`/`strfmon`), cover libc names plus
   `__attribute__((format(printf,m,n)))`, conversion/argument mismatch,
   missing/extra arguments, `%n`, and width `*` types. `-Wformat-security`
   (non-literal format, no arguments) and `-Werror=format-security` are now
   implemented; `-Wformat-nonliteral` and `-Wformat-overflow` remain later work.
4. Later: `-Wmaybe-uninitialized` (needs flow analysis before `mem2reg`),
   `-Wshadow`, `-Wmissing-prototypes`, `-Wstrict-prototypes`,
   `-Wimplicit-fallthrough`, `-Wcast-qual`, and `-Wdeprecated-declarations`.

**Gates.** Per-warning positive/negative tests; **precision gate**: zero new
warnings over `tests/benchmark/programs`, the zlib-ng/gzip/expat sources and
the kernel corpus for each warning that is enabled by `-Wall` (compare counts
with GCC on the same files and list every lccc-only warning as a bug).
`-Werror=format-security` on `printf(argv[0])` fails (R reference: GCC does).
**Effort** M–L · **ROI** 3.4.

### MB-13 · P1 · R — i686 linker parity (reuse `linker_common`)

**Why.** The i386 userspace linker rejects `--version-script`, `-Map`, `-pie`,
`--emit-relocs`, `--exclude-libs`, `--build-id=<other than sha1/tree>` and `-r`
(`src/backend/i686/linker/options.rs`, `unsupported(...)` arms;
`src/bin/lccc_ld.rs` rejects ELF32 `-r` and `-pie`/`-static-pie` outright;
the driver's `-r` for `-m32` demands the `gcc_linker` feature). All errors are
clear, which is why this is "parity" not "trust". `linker_common` already
contains the shared building blocks: `version_script.rs`, `mapfile.rs`,
`build_id.rs`, `defsym.rs`, `archive.rs`, `eh_frame.rs`, `cet.rs`.

**Where.** `src/backend/i686/linker/{options,emit,shared,link}.rs`,
`src/backend/linker_common/*`, `src/bin/lccc_ld.rs`.

**Plan (one PR each, in this order).**
1. `--version-script`/`--exclude-libs`: parse via `linker_common::version_script`,
   apply when building `.dynsym` and `.gnu.version*` in `emit_shared`/`shared.rs`.
2. `-Map`/`--print-map`/`--cref`: feed `linker_common::mapfile` from the i686 layout.
3. `--build-id=uuid|0xHEX|md5` shared implementation (also used by MB-04).
4. `-r` for ELF32: implement relocatable output (merge sections, keep relocations,
   COMDAT handling); mirrors `src/backend/x86/linker/emit_rel.rs`.
5. `--emit-relocs` for the regular (non-script) path.
6. `-pie`/`-static-pie` for ELF32: `R_386_RELATIVE`, `.rel.dyn` (and `DT_RELR`
   via MB-14), `__x86.get_pc_thunk` based PIC code is already generated.
   Without this, MB-04 cannot make i686 default-PIE; until then the driver must
   say "default PIE is x86-64 only".

**Gates.** The same option-surface script as MB-02 run with `-m32`, plus the
MB-32 corpus linked with each feature; `readelf --dyn-syms`, `--version-info`
comparisons against `ld -m elf_i386`. **Accept.** The seven rows in §2's
"`-m32` link" line turn into passes. **Effort** M · **ROI** 3.6.

### MB-14 · P1 · R — x86-64 linker: DT_RELR, page size, image base, small flags

**Progress (re-audited 2026-10-27).** The old "five flags are dead" baseline
is stale. The current x86-64 linker emits `DT_RELR`/`DT_RELRSZ`/`DT_RELRENT`
and the `GLIBC_ABI_DT_RELR` version need for both `-z pack-relative-relocs`
and `--pack-dyn-relocs=relr`; the new regression cases also run the PIE.
`-z max-page-size=0x10000` sets every `PT_LOAD.p_align` to 64 KiB and matches
available BFD; `--image-base=0x10000000` moves the first `ET_EXEC` load segment
as requested (GNU BFD in this sandbox rejects the ELF option, so no BFD-parity
claim); `--oformat=binary` matches `objcopy -O binary` byte-for-byte. These
contracts now have focused tests in `tests/linker/run_linker_tests.py`.

`common-page-size` is not implemented as a separate layout granularity; valid
requests now warn that they are ignored, and `--fatal-warnings` promotes that
warning in either option order. BFD 2.47 measurement accepts zero or a
power-of-two value and rejects malformed/non-power-of-two values; LCCC validates
that contract before warning. The regression gate covers x86-64 built-in,
shared, and script links plus i386 built-in, shared, and script links, including
invalid values. The x86-64 shared path also promotes `--emit-relocs` under
`--fatal-warnings`. The full strict linker suite passes 313/313 with no warns or
skips (BFD/mold oracles; the pinned BFD 2.47 linker is used for the
CODE_4_GOTPCRELX reference case). Remaining work is implementing and measuring a
separate common-page-size layout granularity, adding image-base alignment/error
cases and reference behavior where an oracle accepts it, residual small flags,
and broader RELR/ICF differential validation.

**Why.** `DT_RELR` remains a high-impact ABI contract: Arch ships with it; it
shrinks PIE/shared-object relocation tables and is only safe when the matching
glibc version need is emitted. The unimplemented page-size and small-option
edges still need explicit policy.

**Where.** `src/backend/x86/linker/emit_exec.rs`, `emit_shared.rs`,
`plt_got.rs`, layout constants in `layout_plan.rs`; `src/backend/linker_common/`.

**Remaining plan.**
1. **RELR audit/coverage.** The encoder and loader-version dependency are in
   place. Preserve the contract: collect only encodable `R_X86_64_RELATIVE`
   entries whose place is 8-byte aligned and whose addend is stored at the
   place; encode sorted offsets as even address entries and odd 63-bit bitmaps;
   leave unencodable entries in `.rela.dyn`; emit `.relr.dyn`, `DT_RELR`,
   `DT_RELRSZ`, `DT_RELRENT=8`, and `GLIBC_ABI_DT_RELR`; enable only on
   `-z pack-relative-relocs` / `--pack-dyn-relocs=relr`, off by default. Add
   bitmap-boundary/mixed-relocation mutation tests and compare loader behavior
   and relocation-section size with BFD/mold.
2. **Page size.** `-z max-page-size=N` is implemented and its 64-KiB
   `PT_LOAD.p_align` is differentially tested. A separate
   `-z common-page-size=N` is not implemented as a separate granularity: valid
   values warn as ignored, malformed/non-power-of-two values are rejected, and
   `--fatal-warnings` promotes the warning across x86-64 and i386 built-in,
   shared, and script paths. If the separate RELRO/protection granularity is
   implemented later, test its boundaries independently from maximum segment
   alignment; i386 max-page-size/RELR parity remains tracked in MB-13.
3. `--image-base=ADDR` sets the first `PT_LOAD` base for `ET_EXEC` and `ET_DYN`
   in the focused test. Validate alignment/error cases and retain the caveat:
   the available GNU BFD rejects this option for ELF, so do not claim BFD parity
   unless a reference linker later accepts it.
4. Small options: `--oformat=binary` is implemented and byte-compared with
   `objcopy`; `--fatal-warnings` promotes both parser- and backend-generated
   warnings independently of argument order. Still classify/implement or refuse
   `--print-memory-usage`, `--trace/-t`, `-N/-n/--omagic/--nmagic`, and
   `-z keep-text-section-prefix`.
5. Verify (not assume) `--icf=all|safe` correctness: the flag is accepted by
   lccc-ld; bfd 2.42 rejects it, lld/mold implement it. Add a run-and-compare
   test that function pointers to merged functions still compare unequal under
   `--icf=safe` and that `-Wl,--icf=all` does not break address-significant
   data (`.llvm_addrsig` equivalent: lccc has none, so `safe` must be
   conservative — document the rule).

**Gates.** `readelf -d`/`-r --use-dynamic`, a test binary run with
`LD_DEBUG=reloc` on the host loader, comparison of `-z pack-relative-relocs`
output size with bfd 2.47 (`ld.bfd --pack-dyn-relocs=relr` once available);
exercise glibc 2.36+. **Accept.** Reproducer lines in MB-02 flip; size of
`libc`-linked PIE with N relocations within 5 % of bfd's. **Effort** M · **ROI** 3.2.

### MB-15 · P2 · S — whole-program mode instead of a no-op `-flto`

**Why.** `-flto` is accepted and does nothing (R: accepted; no IR is retained —
treat as S until the archive is inspected). Cross-TU inlining and constant
propagation are among the largest wins GCC/Clang/ICX have on multi-file programs
(gzip, expat, zlib-ng are all multi-file), and the benchmark corpus is the stated
yardstick. Honest staging:

1. **Stage 0 (S, do now).** `-flto` Refused or `BenignNoop` *with a warning that
   no LTO is performed* (MB-01).
2. **Stage 1 (M).** `lccc -fwhole-program -O2 a.c b.c c.c -o app`: the driver
   parses all TUs, lowers each to IR, **merges the modules in memory** (statics
   renamed `TU#name`, extern declarations resolved to definitions, duplicate
   `inline`/COMDAT handling, tentative definitions merged) and runs the normal
   pipeline once. Only accepted when the whole program is on one command line
   (no `-c`). Expected effect: cross-TU inlining of small functions, global
   constant propagation, dead-function elimination with `-fwhole-program`
   semantics (non-`main` globals become internal unless exported by
   `-Wl,--export-dynamic` or attribute).
3. **Stage 2 (L).** `-flto` with `-c`: serialise IR into a `.lccc.ir` section
   (versioned, deterministic), and make `lccc-ld`/the driver perform the merge
   at link time (Stage 1 machinery). Needs a stable IR serialisation format
   and a symbol-resolution handshake (which symbols are preserved).

**Gates.** Differential run-and-compare on gzip/expat/zlib-ng corpus: outputs
identical to non-LTO; counted instructions on hot paths drop (Callgrind, the
repo's paired harness); build with an *unrelated* TU defining a same-named
static to prove renaming. **Effort** M (stage 1) / L (stage 2) · **ROI** 3.0.

### MB-16 · P2 · R — function multiversioning and IFUNC

**Why.** `target("avx2")` works (R: `ymm` emitted) but `target_clones` produces
no IFUNC and no variants (R). zlib-ng, zstd, xz, FFmpeg style dispatch is the
standard way to ship fast portable binaries; with MB-03 fixed, `target_clones`
is the other half.

**Plan.** (1) Parse `target_clones("avx2","bmi2","default")`; (2) emit one
clone per target string through the existing per-function ISA machinery;
(3) emit an IFUNC symbol plus a resolver that tests the same runtime feature
bits as MB-03 in priority order (most capable first), `STT_GNU_IFUNC`,
`R_X86_64_IRELATIVE` handled by the linker for static binaries (MB-04 step 2);
(4) i686: IFUNC needs `-fPIC` and a GOT-relative resolver; (5) `__attribute__((ifunc("resolver")))`
explicit form; (6) `target_version` is Refused.
**Gates.** Dispatch test runs both clones depending on a forced `GLIBC_TUNABLES`
mask or `qemu -cpu`; `readelf -sW` shows `IFUNC`; static-binary variant works.
**Effort** M · **ROI** 3.0.

---

## 6. Tier M2 — speed of the tool

### MB-20 · P1 · R — peephole compile time: one quadratic lookup, then decode once

**Progress:** exact lazy reference tracking and the per-invocation scan ratchet
are implemented; pinned branch-rewrite commits are guarded. Evidence and
mutation results: [label-reference journal](engineering/journal/2026-10-24-label-reference-cost.md).
The new 40-function profile puts `LabelRefs::build` at 1.77%, but the peephole
still costs 73.46%; allocation/decode/fixpoint work remains open. Deferring
copy-mask liveness construction preserves all 94 assembly outputs but saves
only 0.247% of simulated instructions on the same input, not a major latency
win (see the journal follow-up). The historical
reproducer and staged plan below are retained as the acceptance specification,
not a claim that all five steps are complete.

The FP promoter now also indexes loop headers instead of rescanning every
jump prefix. A 40-function measurement saves 4.00% of simulated instructions;
40/120-function final paired compiler-time medians improve by 1.048×/1.134×. This
removes another quadratic lookup but does not establish the ≤35% peephole
criterion. The accompanying audit fixes partial-store and pinned-participant
rewrites; its evidence is in `engineering/evidence/fp-loop-headers-2026-10-24/`.

**Why (measured on `2070d23f`).** Callgrind, 40-function TU, `-O1`: 961 M
instructions total, **68 % in `peephole_optimize_inner`**. Inside it:

- **32.5 % of all compiler instructions are `LabelRefs::build`**, reached via
  `label_is_fallthrough_only` → `combined_local_pass` (`local_patterns.rs`, two call sites near
  lines 1771 and 1854). `label_is_fallthrough_only` is the *one-shot* form
  (`dead_writes.rs`, "a whole-file scan per call"); calling it per candidate label
  is quadratic in file size. Its own doc comment names the cure: build a
  `LabelRefs` once per pass invocation (`memory_fold.rs` already does, with
  `get_or_insert_with`), and states the staleness contract that makes it safe for
  passes that only delete or rewrite lines.
- `FileLiveness::new` fell from 41 % to 4.8 % and `fold_copy_into_lea_base` from
  20 % to 2.3 % since `e8ae10a` (upstream's liveness cache) — those earlier
  hypotheses are closed.
- `str::starts_with`/`slice::starts_with` ≈ 10 %; 80 `trimmed(..).to_string()` sites
  remain in `peephole/passes/` (was 71): opcode tests on text, allocation before
  the filter.

On a 300-function TU the peephole is 2.08 s of 2.81 s (74 %) in the fastbuild
compiler; GCC `-O1` takes 0.77 s on the same file (ratio only).

**Where.** `src/backend/x86/codegen/peephole/passes/{local_patterns,dead_writes,memory_fold,relay_and_lea,load_op_fuse}.rs`.

**Reproducer.**
```bash
CCC_TIME_PEEPHOLE=1 lccc -O1 -c big.c -o /dev/null
valgrind --tool=callgrind --callgrind-out-file=cg.out lccc -O1 -c big40.c -o /dev/null
callgrind_annotate --inclusive=yes cg.out | grep -E 'LabelRefs::build|combined_local_pass|peephole_optimize_inner'
```

**Plan (measure after every step; output must stay byte-identical).**
1. **Hoist `LabelRefs` (S).** In `combined_local_pass` create the index lazily once
   (`Option<LabelRefs>`), pass it to both call sites, and keep the one-shot function
   only for tests. Confirm the staleness contract holds for every transformation in
   the pass (deletes/rewrites only; if one *adds* a label reference, rebuild after
   that edit). Expected: the 32.5 % collapses to a single scan per invocation.
2. **Search for siblings (S).** `grep -rn 'label_is_fallthrough_only(' passes/` and
   any other "one-shot" helper whose doc says "whole-file scan per call" — apply the
   same fix; add a debug-assertion counter (tests only) that fails when
   `LabelRefs::build` runs more than once per pass invocation.
3. **Allocation before filter (S).** For each of the 80 `trimmed(..).to_string()`
   sites test the opcode on `&str` first; prioritise by Callgrind inclusive cost.
4. **Decode once (M).** Extend `LineInfo` with an interned opcode id and operand
   kinds computed on creation/edit so the `starts_with` ladders become table lookups.
5. **Per-function dirty bits (M).** Re-run only functions changed in the previous
   iteration of the fixpoint loop; skip a pass when its trigger opcode never occurs.

**Gates.** Byte-identical `.s` over the benchmark programs, the kernel corpus and the
asm-diff corpus before/after each step (existing identity scripts); a compile-cost
ratchet on two fixed inputs using Callgrind instruction counts with a 3 %
tolerance (deterministic proxy, per the decision rules above) **plus** a paired
wall-time check reported separately. **Accept.** `LabelRefs::build` below 2 % of
instructions; peephole ≤ 35 % of the total; output byte-identical. **Risks.** Stale
index after a pass that adds a label reference — hence step 1's explicit audit and
the identity gates. **Effort** S for steps 1–3, M for 4–5 · **ROI** 4.8.

### MB-21 · P2 · R — `-O0` compiles slower than `-O1`

**Why.** Reproduced twice on `e8ae10a` (**re-measure on current `main` first**: MB-20 step 1 may erase the difference): 80 small TUs take 7.4 s at `-O0` vs 5.8–6.5 s at
`-O1`/`-O2`; `-O0` text is 50 % larger (4049 vs 2705 bytes) and `CCC_TIME_PHASES=1`
shows 72 ms vs 66 ms in `codegen` for the same file, i.e. the same peephole
cost on more lines. The edit-compile-debug loop is exactly where `-O0` is used.
**Plan.** (1) Measure which peephole phases run at `-O0` (`CCC_TIME_PEEPHOLE`);
(2) run only the cheap local passes and skip the liveness-driven ones at
`-O0` — but **only if** debugging behaviour and `-g` line tables are unaffected;
(3) re-measure after MB-20. **Gate.** `-O0` compile instructions ≤ `-O1`'s on
the ratchet inputs; run-and-compare corpus identical at `-O0`.
**Effort** S · **ROI** 2.8.

### MB-22 · P2 — measure the tool honestly

`fastbuild` (`opt-level = 1`) is for REACT cycles and is **not** a basis for any
speed claim about the compiler. Add a documented recipe and script (to be
created) that builds a measurement binary (`--release` equivalent without thin
LTO if memory-bound), records its hash, and reports both Callgrind instruction
counts (deterministic, for gates) and paired wall times (for humans). Keep
instruction counts as the only CI-gated number.

---

## 7. Tier M3 — i686 and linker verification

### MB-32 · P1 · R — i686 differential gate (promote the 14 programs, then widen)

**Why.** The 14-program run-and-compare set (64-bit integer ops, FP, struct
return, varargs, `setjmp`, `qsort` callbacks, 64-bit atomics, TLS,
`regparm`/`fastcall`/`stdcall`, bit-fields, switch lowering, PIC data, float↔int
conversion, `alloca`/VLA) is currently clean against `gcc -m32` at `-O0` and
`-O2` and runs natively as 32-bit code even on an x86-64 host. That is the
best i686 asset we have and it is not a repo gate yet.
**Plan.** (1) Commit the 14 programs under `tests/` with a runner comparing
stdout and exit code to `gcc -m32` and to expected-output files (so the gate
works where GCC multilib is absent); (2) widen: x87 `long double` (arithmetic,
`printf("%Lf")`, `-mfpmath=387|sse`), `-malign-double`, `-mstackrealign`,
`-mpreferred-stack-boundary=2..4`, `-mregparm=3`, `-mrtd`, struct passing
and return (≤8 B in `eax:edx`, `sret` otherwise, `-freg-struct-return`),
`va_arg` of structs and `long long`, `signal`/`sigaltstack` frames, unwinding
through generated functions (`_Unwind_Backtrace`, `__attribute__((cleanup))`
with `pthread_cancel`), all four TLS models, `-fPIC` shared library + `dlopen`,
`-static`, `-O1/-O2/-Os`, `-fcf-protection`; (3) add each defect found to this
file as its own item.
**Gates.** Runs in `ci_local.sh --fast` only for the small subset; full set in
the slow job. **Effort** S · **ROI** 3.0.

### MB-33 · P2 · S — linker differential oracle

Link the same object sets with `ld.bfd`, `ld.lld`, `mold` and `lccc-ld`; assert
(1) identical runtime behaviour, (2) a normalised set of structural properties
is equal: `PT_*` flags and alignment, `PT_GNU_RELRO` page-exactness,
`PT_GNU_STACK`, `.eh_frame_hdr` table sorted and complete (`_Unwind_Find_FDE`
works for every function), `DT_NEEDED` order, `DT_RUNPATH`, symbol versions,
`IFUNC`/`IRELATIVE`, TLS template sizes (`PT_TLS`), init/fini array order and
constructor priorities, weak-undefined resolution to 0, COMDAT dedup,
copy-relocation placement, `--gc-sections` keep rules. Differences allowed only
via an explicit allowlist with a reason. This subsumes A14-07 and A14-06's
linker-oracle part (archive). The 800-object synthetic link (§2) is the
performance input; keep its generator in `scripts/` and record
`lccc-ld` vs the three linkers' wall time **and** `LCCC_LD_TIME` phase output.
**Effort** M · **ROI** 2.8.

---

## 8. Tier M4 — code quality of generated code (carry-forward, re-ranked)

This tier is deliberately thin: the performance queue is large and was
produced under a measurement discipline this file does not repeat (paired,
order-balanced, Callgrind-backed; see the archive). Priorities below are by the
archive's own evidence, **not re-measured here**.

- **MB-30 · L · H — calibrate the cost model from published Raptor Lake data.**
  Replace hand-tuned latencies/ports in the scheduler/regalloc cost functions with a
  generated table (instruction → µops, latency, throughput, ports) built from a
  checked-in snapshot of the uops.info measurement data for Alder/Raptor Lake P
  cores, keyed by the mnemonic classes the backend already enumerates; the
  generator script lives in `scripts/`, the output in a generated Rust file with a
  version header. Gate: table lookup equals the old constants for the
  instructions where they were already right (regression), and the benchmark
  corpus A/B shows no >1 % regression per workload. This is archive item
  A14-25 made concrete.
- **Carry-forward performance items** (unchanged scope, see archive):
  RA-GLA-04, RA-CSAVE-1, RA-PRESSURE-3/4, PF-SN-1, PF-LZ4-1, PF-MB-1, PF-FB-1,
  PF-CHACHA-1, PF-SCHEDULER-1, PF-CLS-1, PERF-1 (byte-width reductions), PERF-2
  (loop interchange; matmul 1.399×, worst kernel), PERF-4 (descending-loop
  IV strength reduction), PERF-5 (redundant-extension elimination), PERF-6,
  SETMEM-SHAPE-1, RANGEFOLD-OR-1, CONSTLOOP-FOLD-1, WR-COND-RETEST,
  MI-CLOBBER-1/MI-XMM-1/MI-PARAM-1/MI-ROTATE-1/MI-ENCODE-1, IVOPTS-1,
  LOOP-PREHEADER-3. Use MB-15 (whole-program) as the next *measurable*
  cross-cutting performance lever for the multi-file corpus members.
- **Metric traps still in force:** METRIC-TRAP-1 and METRIC-SPLIT-1 in the archive
  (`Ir` vs wall clock disagreement of up to 74 points) — never accept an
  optimisation on static instruction counts alone.

---

## 9. Tier M5 — engineering system

### MB-40 · P1 · S — keep the census ratchet honest
`scripts/audit_census.py` exists upstream with a baseline; its remaining
weaknesses are recorded as A14-01 in the archive (suffix-blanking after the
first `#[cfg(test)]`, malformed-function reporting). Close those, then add the
two metrics this rewrite found useful: "functions over 1,000 lines" is already
tracked (23); add **`.to_string()` calls on a `trimmed(...)` result inside
peephole passes** (MB-20 step 1, currently 71) and **catch-all option arms**
(`=> {}` after `starts_with("-f")`, MB-01) so the specific regressions cannot
return.

### MB-41 · P2 · S — backlog format gate
This file follows the item format of §1. Add a gate (to be created, e.g.
scripts/check_backlog_format.py (to be created)): every `### ID · class · evidence` heading
has Why/Where/Plan/Gates/Accept fields; every `R` item contains a fenced
reproducer or a command in backticks; every referenced repo path resolves
(the existing doc-link gate already covers paths); closed items must be moved to
the archive directory rather than left in place. New closed entries go to
[`engineering/journal/`](engineering/journal/README.md), not here.

### MB-42 · P1 · S — promote the probe set to a standing gate
The probes behind §2 (flag effects, linker option effects, debug-info presence,
C23 census, 14 i686 programs, link-speed corpus, compile-instruction ratchet) were
ad-hoc scripts. Extend `scripts/lccc_capability_probe.py` (it already runs the
compiler and records command + exit status per capability) with `effect:` rows
rather than adding a parallel tool; keep a machine-readable result file and a
checked-in expectation table; each MB item above flips its rows from GAP to OK as it
lands. This makes "Maybach progress" a number instead of an opinion.

### MB-43 · P1 · S — keep ISA cross-references strict; audit model boundaries
`scripts/sdm_isa_crossref.py --backend all --strict` verifies every declarative
row of both encoders against Intel's ISA descriptions: 2595 rows, 1436 classes,
4992 encodings from 39 XED datafiles, **zero findings and zero unmodelled rows**
(i686 1081, x86 1514). `tests/corpus/test_sdm_isa_crossref.py` pins the parsing
(`0b1011_0 SRM[rrr]` opcodes, `+rd`/`+cc`, EVEX `EVV`/`W0/W1`, maps 4..7), the
modelling rules (group digits, operand-size agreement, x87 base-byte digits,
VEX/EVEX family and prefix of every helper) and the reference provenance.
Two invariants must not decay: a row the model cannot read is a *defect*, not
noise (`--strict` fails on UNMODELLED), and a class XED does not model goes into
`SDM_ONLY_CLASSES` with a provenance comment **and** a byte-level assertion in
`tests/asm-diff/` — `PREFETCHW` (`0F 0D /1`) is the only entry today. The
as-2.47 reference gap is closed: the distiller used to read a lone `f2`/`f3`
opcode byte as a mandatory prefix and drop the whole row, which is why `andn`,
`blsi`, `blsr` and `blsmsk` were reported as UNCOVERED; the opcode is now taken
as the *last* byte of the field. The upstream `0F 01 <ModRM>` repair is
retained: the fixed ModR/M byte contributes its `/digit` instead of being
dropped as an extra opcode. Only the four `movs*` overloads remain documented
snapshot limitations. The parameterised `setcc` resolves through
`SNAPSHOT_ALIASES` to the `setc` row it encodes. `gas_encoding_crossref.py
--verify --strict --backend all` therefore checks 1638 rows with 0 findings, 0
unmodelled rows and an **empty** UNCOVERED list — a ratchet that
`tests/corpus/test_gas_encoding_crossref.py` pins by asserting both lists are
empty and that the four BMI rows carry `VEX.NDS.LZ.0F38 F2/F3 /digit` in the
snapshot. A subsequent red-team audit found the empty list insufficient:
substring family inference, unconditional v aliases and an early ModR/M return
could accept invalid families/W bits, and strict mode did not fail for an
uncovered-only result. The repair derives family/naming from exact GAS
attributes, checks both known constraints, and fails closed on uncovered names.
The same-source snapshot is now2,195 names/3,975 forms, with the same1,638
encoder rows checked. Eighteen GAS tests,22 SDM tests,11 native GAS
positive/refusal cases and a1,024-expression three-compiler oracle cover the
new boundaries. C precedence is repaired as well:116 synthetic disagreements,
but **zero** precedence-dependent changes in5,223 accepted evaluations from the pinned
current table. Details and remaining row-existence/operand-model limitations:
[`red-team audit`](engineering/evidence/gas-reference-redteam-2026-10-25/README.md).
Accept: clean-tree gates pass; wrong-family, W, digit and uncovered mutations
fail. Full operand/feature and architecture audits remain separate obligations.

### MB-44 · P0 · R — close the runtime gaps the ranker can now report
The corpus ranking tool could not read five of the six checked-in evidence
reports (they predate the nested `runtime` schema), so it wrote them off as
"INCORRECT/FAILED" and the worst-workload list has been derived by hand since.
`scripts/bench_rank.py` now normalises the older flat schema and refuses an
unknown one instead of labelling it a failure, which makes the recorded corpus
usable again. Ranking `engineering/evidence/benchmarks/2026-09-27-s13-685da054`
(ratios are lccc/GCC, >1 = lccc slower, 95 % paired bootstrap):

| # | workload | lccc | GCC | ratio | CI |
|---|---|---|---|---|---|
| 1 | `lz4_match_extend` | 2332.0 ms | 1250.6 ms | **1.937** | [1.787, 2.059] |
| 2 | `nbody` | 527.7 ms | 374.4 ms | **1.414** | [1.407, 1.548] |
| 3 | `expat_xml_scan` | 52.2 ms | 41.5 ms | **1.246** | [1.045, 1.306] |
| 4 | `hash_table` | 15817.2 ms | 13412.7 ms | **1.176** | [1.076, 1.223] |
| 5 | `linux_find_bit_scaled` | 1058.0 ms | 932.9 ms | **1.162** | [1.078, 1.235] |

The 2026-09-25 red-team run ranks the same kernels the same way over a wider
set (`linux_find_bit` 1.312, `mandelbrot` 1.285, `binary_trees` 1.186,
`sha256_transform` 1.177, `sqlite_varint` 1.131, `zstd_count` 1.119,
`switch_dispatch` 1.109, `nbody` 1.078) and puts `spectral_norm` at 27.7×, which
the 2026-10-17 archive records as later repaired to ≈1.0 — re-measure before
quoting it. Work order: `lz4_match_extend` (byte-compare match extension: the
classic "GCC widens the compare, we go one byte at a time" shape) and `nbody`
(FP-heavy, no vectorisation win) first, then `expat_xml_scan` (`sqlite_varint`
overlap) and `hash_table` (probe loop).

**Fresh revalidation (2026-10-25):** the earlier Rust blocker is obsolete.
`rust-cache-1.97.0` supplies rustc/cargo/rustfmt/Clippy, verified against its
manifest; current main builds warning-clean with `fastbuild -j2`,8GiB swap.
Checking only `harness-cache-1` was not an inventory of all available caches.

A fresh15-round CPU-pinned, output-checked VM screening of the two priority
workloads gives `lz4_match_extend` **1.0007x GCC [0.9647,1.0164]** (parity, not
an established regression) and `nbody` **1.4361x GCC [1.4268,1.4521]** /
**1.4948x Clang [1.4835,1.5229]**. Ratios are LCCC/reference; all measured arms
exceed237ms. These are current-main measurements, **not wins from the oracle
repair**. Investigate nbody first rather than uncritically tuning the historical
1.937x LZ4 number. Reduced-size Callgrind at a separately stated v3 profile
retains1.1841x/1.1579x whole-program simulated Ir ratios for nbody/LZ4 versus
GCC; do not equate them with native cycles or the default-size native ratios.
Full receipts, toolchain limits and review rationale:
[`gas reference red-team audit`](engineering/evidence/gas-reference-redteam-2026-10-25/README.md).

Runtime follow-up: the computed-reciprocal reduction now has a strict SSE2
spelling, not just AVX2. The canonical default-size `spectral_norm` case falls
from 200.22 to 158.23 ms: after/before paired ratio **0.7974 [0.7881,0.8100]**
in31 randomized rounds (20.26% lower runtime). After/GCC is1.0087
[1.0032,1.0237], still slightly behind; after/Clang is0.8080 [0.8011,0.8227].
The nbody/F32/F64/I32 controls have byte-identical `.text` before/after: no
speedup is attributed to them. Reduced-size baseline-SSE2 Callgrind reports
24.92% fewer whole-program simulated instructions for spectral norm, and an
unchanged nbody count. The FP unroll-budget, SSE-copy and register-streaming
experiments did not outperform the selected lowering and are not shipped.
This closes a demonstrated SSE2 legalization gap, **not MB-44 as a whole**;
nbody remains an open priority. Full numerical/ISA boundaries, failure
witnesses and receipts:
[`SSE2 runtime follow-up`](engineering/evidence/sse2-strict-recip-2026-10-25/README.md).

**The table above is the 2026-09-27 ranking and is kept only as the schema
witness; it is not the live list.** Two of its rows are repaired and must not be
re-quoted: `spectral_norm` was 1.13 by 2026-10-22 (the 27.7× figure was a
transient regression, repaired to ≈1.0), and `hash_table` gained −18.0 % from
CFG-priced block placement in the round-2 layout work. The current, witnessed
worklist lives in [`docs/PERFORMANCE-BACKLOG.md`](docs/PERFORMANCE-BACKLOG.md)
(load-operand commutation in `linux_find_bit`, value-table switch lowering in
`switch_dispatch`, affine loop-PRE for `IV + k` in `spectral_norm`, scalar
unrolling before `mandelbrot`/`fannkuch`); add to that file, not here.

### MB-45 · P1 · S — make the ISA "gap" counts mean what they say

**Why.** `scripts/gas_encoding_crossref.py` has two directions: ``--verify`` asks
"is every declarative encoder row a legal `as` 2.47 encoding" (0 findings, 1638
rows, both backends) and the other three ask "what does `as` accept that the
encoder has no row for".  The second question is not the coverage question it
is read as, because the 64-bit backend encodes most instructions *in code*:
`encode_alu` derives the opcode arithmetically (`0x01 + op * 8`,
`encoder/gp_integer.rs`), the `jcc`/`cmov*`/`setcc`/`movbe`/BMI2 families
dispatch on a suffix or a table built in a helper, and the VEX/EVEX emitters
share helpers.  Measured on the current tree with ``--backend all``:

| invocation | count (re-derived 2026-10-25) | what it actually is |
|---|---|---|
| ``--verify --strict`` | 1638 rows + 224 code-model encodings, 0 findings | every *row* and every code-derived family is a legal encoding (sound) |
| ``--missing`` | **806** encodings / 438 mnemonics | forms `as` accepts that no row model covers after per-spelling model expansion |
| ``--absent`` | 1354 mnemonics | names with no row; includes APX/AVX-512 spellings and the alias spellings |
| ``--absent --triage`` | 740 UNMENTIONED / 614 MENTIONED | spelling census, not coverage |

Every number in that table moved when the snapshot learned the direction bit
(below), so none of the figures published before PR #965's snapshot may be
quoted: the old 349/286 pair was measured against a snapshot that had `and
0x22` missing *and* `movdqa 0x66 0f 6e` (which is `movd`) present.  The counts
are only comparable within one snapshot; the tool prints the snapshot's own
totals (2195 mnemonics, 4022 encodings) in ``--summary`` so a stale figure is
visible.

The list is still not a gap list: `add`, `call`, `movbe`, `rorx`, `pdep`,
`pext`, `mulx`, `andn` and the ALU family are all *encoded*, and `call`/`add`
are not even in the ``--absent`` set (a row names them) — the model simply
could not see `0x01 + op * 8`.  Reading the counts as a worklist invites
"implementing" encodings that already work, the one failure mode a coverage
gate must not have.  That these spellings also assemble to the same bytes as
`as` 2.47 is the `as`-diff suite's claim, not this tool's: it is measured on
the corpus, in CI, and is not assumed here.

**Progress (2026-10-25, second pass): four code-driven families, and the gate
that checks them.**  `code_models()` now reads four code paths, not one:

* `encode_alu` — the `self.encode_alu(ops, mnemonic, <op>)` dispatch arms out
  of `encoder/mod.rs`; `alu_forms(op)` derives the nine legacy encodings from
  the dispatch index (`add` `00..05` + `80`/`81`/`83 /0`, `and` `20..25`, `cmp`
  `38..3D`).
* `cc_from_mnemonic` — the 30-alias condition table in *both* backends'
  `registers.rs`, giving `j<cc>` `0x70+cc`, `cmov<cc>` `0f 4x`, `set<cc>`
  `0f 9x /0`.
* `inc`/`dec` — the one-byte `40+rd`/`48+rd` forms, which only the **i686**
  backend can use (REX owns `40..4F` in 64-bit mode, so `x86` must go through
  `FE`/`FF`, which are declarative rows there).
* `fma3_opcode` — the `vfm{,n}m{add,sub}{132,213,231}{ps,pd,ss,sd}` grid, whose
  opcode lives in the mnemonic's digits and in no row at all.

The claims are no longer trusted: `verify_code_models()` requires every one of
the 224 encodings in 160 mnemonics to be a form `as` 2.47 actually accepts, in
both directions (`CODE-UNKNOWN` for a mnemonic the snapshot does not have,
`CODE-EXTRA` for an encoding it does not accept).  It is wired into `--verify`
and into `--strict`, so a model that drifts from the encoder fails CI instead of
quietly shrinking the gap counts.  That gate is what made the snapshot's own
defects visible: the first run reported 16 findings, all of them the ALU `reg`
forms, and the cause was in the *oracle* (see the next paragraph), not in the
model.

**The direction bit was transcribed wrong, and that is fixed.**  `D` does not
mean "XOR 1": GNU as picks one of four schemes (`Opcode_D` 0x2 for the base
map, `Opcode_ExtD` 0x1 for the extended maps, `Opcode_SIMD_IntD` 0x10 for the
compact SIMD moves, `Opcode_FloatD` 0x4 for x87) and three cases — `Ugh` rows,
FMA4/XOP rows, and commutative `C` rows — change no opcode at all.  The old
transcription therefore **invented** encodings (`movdqa 66 0f 6e`, which is
`movd`; `test 86`, which is `xchg`; `faddp da`, which is `fiadd`) and **missed**
real ones (`and (%rax),%al` is `22 00`).  Two more gaps closed with it: `D|W`
compose, so `mov` has all four of `88/89/8a/8b`, and rows with no ModR/M keep
their register in the opcode (`push %rcx` is `51`), which the snapshot now
expands over all eight registers.  Snapshot: 3975 → 4022 encodings, with 105
wrong entries removed and 151 right ones added; every single change was
cross-checked against `objdump -D` on synthesized bytes
(`tests/corpus/test_gas_snapshot_decode.py`, which decodes the whole
direction-sensitive subset in one pass and pins the six encodings the decoder
itself spells differently, each with its reason).  Pinned hermetically by
`GasDirectionModelTests` and `GasCodeModelGateTests`; the ALU forms by
`test_the_alu_code_model_derives_the_opcode_from_the_dispatch_arm` and
`test_a_code_modelled_family_leaves_the_gap_list`, the second of which also
asserts that non-instruction pseudo-mnemonics (`cs`, `ds`, `es`) stay in the
list, so the model cannot silently swallow everything.

The docstring and the three summary lines now say exactly this, and ``--triage``
splits the list by whether the *spelling* appears anywhere in the backend
sources.  Neither half is a verdict: a mention can be a comment or a deny-list
entry, and a family that composes its mnemonic (`format!`) leaves no literal, so
`vfmadd132pd` lands in UNMENTIONED while being supported.  Both limits are
pinned by tests in `tests/corpus/test_gas_encoding_crossref.py`.

**Where.** `scripts/gas_encoding_crossref.py` (`triage_absent`, the docstring's
"What a row cannot see"), `tests/corpus/test_gas_encoding_crossref.py`.

**Plan (each step independently landable).**
1. ~~`encode_alu`~~, ~~`cc_from_mnemonic`~~, ~~`inc`/`dec`~~, ~~`fma3_opcode`~~
   **done** (above).  Next, in this order: BMI/BMI2 (`pdep`/`pext`/`mulx`/
   `rorx`/`sarx`/`shlx`/`shrx`/`andn`/`bls*`), then `movbe`/`movdir*`, then
   `crc32`'s EVEX forms.  Each extractor reads the helper's own table, the
   pattern `vcvt_base_params` and `HELPERS` already use, and each one lands
   with a `verify_code_models()` claim so it cannot drift.
2. ~~Wire the check into `--strict`~~ **done**: `--verify --strict` fails on a
   `CODE-*` finding, and ``--missing``'s own exit status is still interactive
   (the list is a reading aid until step 1 reaches zero).
3. Delete the ``--absent`` list from the docstring's promise set once step 1
   covers the families it names.

**Gates.** For each family: a test that the extractor reproduces the bytes the
`as`-diff corpus already pins for it (mutation-verified: break the extractor,
watch the row count fall), plus ``--verify`` staying at 0 findings.

**Accept.** ``--missing`` reaches 0 for both backends with ``--strict`` and the
code-driven families modelled; no count in this file is quoted as coverage
until then.  Current distance after per-spelling expansion and opcode-register ranges: 806 encodings over 438 mnemonics. The current list still includes extension encodings, alias/schema collisions and model gaps; it is not yet a validated count of missing compiler functionality. Treat 813 as a report output, not a completion claim.  **Effort** M (the families are the work).  **ROI** 3.0 — it buys no
runtime, but it is the difference between a gate and a rumour, and three
sessions have now spent time re-deriving that these numbers are not gaps.

#### Campaign state, 2026-10-25 (what the manuals can and cannot give here)

* **Intel SDM.** The machine-readable form is XED's `datafiles/*.txt`, and
  `scripts/sdm_isa_crossref.py` consumes 39 of them for encodings (2595 rows,
  1436 classes, 4992 encodings, 0 findings).  The one attributes axis still
  unexploited is `FLAGS:` on each ISA line: it would validate
  `flag_peepholes::flags_written_mask` (the table that decides when a flag write
  may be dropped) against the SDM's own per-instruction flag lists.  That is the
  next gate worth building, and it needs the SDM's operand-*width* refinements
  (`inc`/`dec` preserve CF, `bt` preserves ZF) modelled explicitly, not guessed.
* **AMD 2026 SOG.** Not reachable from this sandbox (`www.amd.com` refuses
  connections), so no Zen tuning claim may be sourced from its prose here.
  The reachable machine-readable cross-check is LLVM `X86.td` at `main`
  (`b24cdd737024be039fc644d802e677af9e91945c`), whose resolved
  `ProcessorFeatures` give: Zen3 ⊇ {FSRM, PKU, INVPCID, CLWB, RDPRU, WBNOINVD},
  Zen4 ⊇ AVX-512 CD/BW/DQ/IFMA/VBMI/VBMI2/VL/VNNI/BF16/BITALG/VPOPCNTDQ/GFNI +
  VAES/VPCLMULQDQ/SHSTK, Zen5 ⊇ {AVXVNNI, MOVDIRI, MOVDIR64B, PREFETCHI,
  VP2INTERSECT}.  lccc tracks **none** of `fsrm`, `pku`, `invpcid`, `clwb`,
  `rdpru`, `wbnoinvd`, `movdiri`, `movdir64b`, `prefetchi` as a CLI feature bit,
  and `src/common/x86_features.rs` states the rule that forbids adding them as
  flag-only surface: a `-m` flag with no codegen permission bit is a silent
  no-op, and a `__builtin_cpu_supports` answer that promises an instruction the
  backend cannot emit is the AUD-CPU-1 bug.  Closing those gaps means adding the
  permission bit *and* its consumer — that is AUD-CPU-3, not a flag-table edit.
* **Resolved earlier in the campaign, do not re-open:** the `0F 01 <ModRM>`
  system group is distilled (#964, 1638 rows, UNCOVERED 0); `lea3_latency`,
  `lea3_uops` and `lea3_rtp_x100` *are* consumed (they derive
  `X86Tune::avoid_lea3_on_critical_path`, `cpu_model.rs`); Zen 5's
  `tzcnt` false dependency is modelled and pinned by
  `false_dependency_rows_match_uops_info`; the scaled-index LEA is composed into
  a `lea` consumer by `fold_scaled_index_temp_into_mem`
  (`relay_and_lea.rs`, including the `leaq 0(,%I,s)` + `leaq (%B,%T,1)` shape).

---

## 7a. asm-goto fusion: three measured codegen gaps it exposes (2026-10-30)

The `=@cc` fusion in this branch is correct and it removes the
setcc -> store -> reload -> test round trip, but the function around it
still carries three instructions that serve nothing.  Measured on

```c
__attribute__((noinline)) static int cc_branch(unsigned x) {
    _Bool err;
    asm goto("testl %1, %1; jz %l[label]" : "=@ccz"(err) : "r"(x) : "cc" : label);
    if (err) return 1;
    return 0;
label:
    if (err) return 2;
    return 3;
}
```

at `-O2` on x86-64 (lccc fastbuild vs gcc 16.2, same source):

| | per-call uops on the `x != 0` path | what it is |
|---|---|---|
| gcc 16.2 | 4 | `testl/jz`, `sete`, `movzbl`, `ret` |
| lccc (this branch) | 8 | `subq $8,%rsp`, `movl %edi,%eax` (dead), `movl %edi,%ecx`, `testl/jz`, `jne` (always taken), `xorl`, `addq $8,%rsp`, `ret` |

1. The frame is empty: the fused path stores nothing, yet the prologue still
   runs `subq $8, %rsp` / `addq $8, %rsp`.  Frame sizing happens before the
   fusion decision, so eliding it needs either a second sizing pass for
   `asm goto` functions or a deferred-offset scheme.
2. `movl %edi, %eax` is dead: it is the asm output's register home, allocated
   before the fusion proved the home unused.  Freeing it is the same
   "allocate after the fusion decision" change.
3. `.LBB1: movl $1, %eax; ...; ret` is unreachable: on the label edge the
   fusion emits `jne` whose target is statically never chosen (the edge is
   only reachable through `jz`), but the block is still emitted.  A
   reachability pass over the emitted CFG would delete it.

Runtime consequence, measured: 40M calls of the function above take 51.07 ms
with this branch, 50.95 ms with `origin/main` (0.2%, noise) and 38.73 ms with
gcc 16.2.  The fusion is performance-neutral on this shape — it trades a
setcc/movzbl pair for a taken branch — and the 32% gap to gcc is those three
instructions plus the extra branch.  The correctness win is the real one: the
same program prints 2 (correct, gcc-equal) here and 3 (stale taken-edge
output) with `origin/main`.

## 8a. Declaration-parser tail split (measured 2026-10-30)

The attribute/cleanup verdict work feeds one declaration-level verdict pool, and
the three declaration entry points each carry the tail that turns the collected
flags into a `Declaration`.  They cross the 300-line ratchet:

| function | file | lines | main |
|---|---|---|---|
| `parse_declaration_rest` | `src/frontend/parser/declarations.rs` | 355 | 315 |
| `parse_local_declaration` | same | 342 | 291 |
| `parse_external_decl` | same | 317 | 277 |
| `lower` | `src/ir/lowering/lower.rs` | 514 | 500 |

`scripts/audit_census_baseline.json` was raised by those three and one count in
the commit that added them, which is the ratchet's own escape hatch.  The
refactor that pays it back is mechanical and is the next item here: extract the
`Declaration::new` + `set_*` tail into one `finish_declaration(...)` helper
shared by the three entry points, and split `lower`'s call lowering from its
statement walk.  Nothing in the current change depends on the split, and the
semantics are already covered by `tests/regression/check_attribute_diagnostics.*`
plus the attribute option ledger.

## 9a. SSE/AVX audit follow-through (2026-10-26)

These are **open obligations**, not an ISA/manual sign-off. Start with
`scripts/audit_sse_avx_public_api.py --check` and the pinned GCC 16.2 header
name/reference sets in `engineering/isa/`. Name presence is not evidence that
an implementation is correct. The initial scope is `_mm_*`/`_mm256_*` in the
eight named SSE–AVX2 leaf headers plus bundled `lcccsimd.h`; uppercase control
macros/constants, MMX aliases and other `immintrin.h` extension headers need
separate inventories. Conditional alternatives are retained, not preprocessed.

| ID | Reproducer / starting point | Acceptance criterion |
|---|---|---|
| SIMD-API-1 | `missing_reference_names` in `sse-avx-public-api.json`; compare against the SHA-pinned GCC resource headers, not just existing bundled wrappers | Classify every reference-only name as implemented, explicit supported-scope exclusion, alias or diagnosed gap; native typedef/layout and direct/indirect mixed-compiler ABI tests on i686/x86-64 at O0/O2; inventory control macros, constants and remaining headers separately. Bundled-only names are not automatically nonstandard (reference scope excludes FMA/AVX512VL headers). |
| SIMD-PERM-1 | `expr_builtins_lcccsimd.rs`: broad `v*`/`*512` handling cannot express subset conjunctions. A first instruction-specific permission table uses independent AVX512F/BW/DQ/VL/VNNI/BITALG/VPOPCNTDQ feature bits; the currently mapped operations exercise these requirements: key examples are `vpcmpub128/256` (F+BW+VL), ternary logic (F+VL), qword min/max/abs (F only; lane width does not imply the DQ subset) and masked byte loads (F+BW+VL). DQ remains modeled but no currently mapped helper exercises a DQ-only instruction. The i686 backend does not yet implement these EVEX helpers and now diagnoses instead of silently reaching an absent emitter. | **Not closed.** Run the GCC 16.2 positive/negative matrix on x86-64; implement or formally scope every remaining subset (CD/IFMA/VBMI/VBMI2/BF16/FP16/ER/PF/VP2INTERSECT), audit all individual encodings and `-mno-*` last-option behavior, and decide/implement i686 support. The public-name inventory now compares the bundle against 6,169 names parsed from 49 GCC 16.2.0 release-source headers (pinned commit and per-header SHA-256 in `engineering/isa/gcc16-sse-avx-reference.json`); it finds 5,365 reference names absent from the current bundle. This is a mechanical inventory, not semantic/manual coverage or proof that all reference declarations apply to this project. The 512-bit compare audit also caught and fixed missing BW requirements for `vpcmpeqb/w` and `vpcmpgtb/w`; continue checking each mapped helper against Intel encodings. Verify function-local targets and both option orders; unsupported architectures diagnose. No EVEX width shortcut or host-CPU inference. |
| SIMD-FMA-1 | The generic helper gate now classifies FMADD/FMSUB/FNMADD/FNMSUB and fused addsub/subadd spellings as FMA3; AVX-512 FMA is gated by AVX512F, and local `target("fma")`/AVX-512 feature permissions are recognized. `_mm512_fmadd_ps/pd` now route to native EVEX helper calls (the previous lane loops did not guarantee single-rounding fused semantics). | **Not closed.** Enumerate every public/raw FMA3 and FMA4 name and distinguish unsupported FMA4; cover 128/256/512 signed forms including fmsub/fnmadd/fnmsub and addsub/subadd with direct intrinsic emission; test exact rounding under `-ffp-contract=off`, all global/local override orders, both inlining directions and isolated callers against GCC 16.2/Clang 23.1.4. Ensure generic contraction remains separately controlled. |
| SIMD-TYPE-1 | Header-derived arity table and generated-helper metadata test cover resolved prototypes; the exact unmapped ratchet contains `512_i_vpcmpeqb512` and `512_i_vpcmpeqw512` (valid calls now diagnose); operand class/type/width, malformed width/class prefixes and unknown mnemonic paths remain separate gaps | Generate positive and missing/extra-operand controls for every helper including aliases, scalar/store/zero-operand families; enforce vector lane/width, pointer constness and scalar/immediate ranges with precise diagnostics, never silent truncation/zero/ICE. Cover undeclared and forged prototypes and foreign targets. |
| SIMD-MANUAL-1 | Intel combined SDM 325462-093; AMD 26568 rev 3.27 (July 2026), 24594 rev 3.38. Selected Intel 2B MIN/MAX instruction sections are reviewed; combined Intel/AMD retrieval established edition/front matter only. A 30-page extraction is **not full-manual coverage** | Obtain full primary instruction sections, retain hashes/edition/page or instruction anchors and extraction method; map every public API, raw builtin, lowering and assembler encoding to actual instruction requirements. Check register/memory/immediate forms, widths, destructive/upper-lane effects, masks, alignment, fault behavior, rounding/MXCSR, NaNs/signaling/signed-zero/subnormals, memory tails and numerical edge cases. Both x86 modes; GAS/LLVM cross-checks are not substitutes for manuals. Publish per-row reviewed/untested/unsupported status; never infer whole-family coverage from selected runtime legs. |
| SIMD-PERF-1 | `legacy_sse_native_chain.c`, generic helper chains, and YMM ABI carrier frames; `aggregate_abi.rs` documents the hidden-pointer IR shape even for register returns | Remove carrier/copy bridges only with an IR/inliner/ABI proof. Compare exact outputs against GCC 16.2 in all four caller/provider directions; guard pages, volatile/address spaces and inline/noinline controls; paired output-checked SSE2/AVX/AVX2 corpus runtime plus Valgrind/Callgrind, instruction/frame receipts, O2 native host screening and hosted EPYC results. Static or Callgrind counts alone are not a runtime speed claim. |

## 9b. SIMD assessment and programme (measured 2026-10-31, `219f74a`)

Scope: automatic vectorisation, vector codegen, intrinsics, ISA coverage and the
correctness of all three, for x86-64 and (where stated) i686. This section is the
SIMD *performance and design* programme; §9a above remains the SSE/AVX **API**
audit (header names, permission tables, manual review) and is not repeated.

### 9b.1 How to read the evidence
- **R** = reproduced with the commands below on a fastbuild of `219f74a`
  (rustc 1.91.1 + `if_let_guard` shim, 1 vCPU Xeon-class VM with AVX-512, GCC 13.3
  as the *only* available reference; the GCC 16.2 / Clang 23.1 / ICX oracles must
  rerun every number before a claim leaves this file).
- Runtime numbers are **screening**, not target-hardware evidence: single VM,
  minimum of three runs, `N = 4096` elements, one kernel per function. The
  i7-14700KF (AVX2, FMA, AVX-VNNI, **no AVX-512**) has not been measured
  (UP-6 in §10 stands). Static counts and Callgrind `Ir` are not runtime wins
  (decision rules, §1).
- The 44-kernel corpus (float/double/int/short/byte elementwise, conversions,
  reductions, widen/narrow, selects, strides, gathers, aliasing, recurrences and a
  matmul) is committed as tests/simd/scoreboard_kernels.c and the F1 shape matrix
  as tests/simd/aos_pair_shapes.py; the runner/gates of SV-02 below are still to be
  built, so the reproducers in each item remain authoritative.

### 9b.2 Findings

**F1 — wrong code and a crash in the vectoriser/unroller path (P0, R).**
A loop that reads and writes two interleaved fields per iteration segfaults at
`-O2`/`-O3` and is correct at `-O1`:
```c
#define N 64
static float x[3*N+8], y[3*N+8], o[3*N+8];
__attribute__((noinline)) void k(void){
  for (int i = 0; i < N; i++) {              /* complex multiply, AoS */
    float ar=x[2*i], ai=x[2*i+1], br=y[2*i], bi=y[2*i+1];
    o[2*i]=ar*br-ai*bi; o[2*i+1]=ar*bi+ai*br; } }
```
Shape matrix (7 loop shapes × `-O1/-O2/-O3`, each compared with `gcc -O0` output):
only the two shapes with *cross-field* uses fail — `o[2i]=x[2i]+y[2i+1];
o[2i+1]=x[2i+1]+y[2i]` and the complex multiply above (rc 139 at `-O2`/`-O3`).
Same-field shapes (`o[2i]=x[2i]*y[2i]`, 3-strided, plain copy, AoS add) pass.
The `-O2` assembly shows the unrolled second copy indexing with `%r9`
(`4(%rsi,%r9,8)`, `4(%rdx,%r9,8)`, `4(%rdi,%r9,8)`) while nothing in the function
ever defines `%r9` (the loop only does `addq $2,%r8`): an index value (`i+1`) that
the unroller/strength-reducer references but whose definition was lost. In the
larger corpus (`N = 4096`) the same loop did not crash but produced a wrong
checksum (the only mismatch among 44 kernels with `-ffp-contract=off`), i.e. the
defect can be silent. Disabling `CCC_NO_TWO_BLOCK_UNROLL=1` or compiling with
`-fno-tree-vectorize` makes the output correct (**not** root cause: both gate the
path); `CCC_NO_FIXED_SLP`, `-fno-slp-vectorize`, `-fno-unroll-loops` do not.
Everything else in the corpus (43 kernels × 5 configurations: `-O2` baseline,
`-O2 -mavx2 -mfma`, `-O3 -mavx2 -mfma`, `-O2 -march=native`) is bit-identical to
`gcc -O0` when FP contraction is off, which is a real positive result for the
existing pattern passes.

**F2 — the vectoriser is a collection of pattern recognisers, not a general
loop vectoriser (design, R/S).** `src/passes/vectorize.rs` (38,513 lines) opens
with "AVX2/SSE2 vectorization pass for matmul-style loops … Only handles
matmul-style patterns"; the satellites are recogniser families
(`vectorize_byte_count_normalize`, `vectorize_generator_map`,
`vectorize_strict_recip`, `vectorize_strided_i32_reduce`,
`vectorize/strict_i32_pack`, `arx_vectorize`, `vec_interleave`,
`vec_mask_shifts`, `vec_shuffle`) plus the basic-block SLP
(`slp_vectorizer.rs`, 6,895 lines). There is no shared legality layer
(dependence distances, alias versioning), no VF selection, no cost model, no
if-conversion and no masked-memory model. The measured consequences are F3–F6.
Maintenance consequence: more than sixty `CCC_NO_*` kill switches in
`src/passes/mod.rs`, each a different recogniser's escape hatch.

**F3 — large, uniform performance gaps on ordinary loops (P1, R).** Per-kernel
ns/element, lccc `-O2 -mavx2 -mfma` divided by `gcc -O3 -mavx2 -mfma` (higher =
lccc slower; geometric mean over 44 kernels **1.96×**, 17 kernels above 2×):

| Class | Kernel (ratio) |
|---|---|
| not vectorised **and** not if-converted | `select_f32` 35.4, `condstore` 10.6, `abs_f32` 5.8, `min_i32` 5.3, `clamp_i32` 2.4 |
| conversions | `f2i` 7.2, `i2f` 7.1, `f2d` 3.8 |
| width change / shifts | `narrow_i16_u8` 8.8, `widen_u8_u16` 6.4, `shift_i32` 5.0 |
| permutes and access shapes | `reverse` 6.9, `aos_soa` 2.6, `cmul` 2.5, `stride2` 2.0 |
| dependence distance | `overlap_dist1` (`a[i]=a[i+1]+b[i]`) 8.5, `alias_overlap` 4.0 |
| idiom reductions | `sad_u8` 1.8, `satadd_u8` 1.7 |
| near parity | `add_f32` .93, `fma3` .97, `stencil3` .97, `dot_i32` 1.0, `sum_f64` 1.0, `add_i16` .96, `matmul` 1.5 |
| lccc faster | `count_eq_u8` .19 (byte-compare idiom), `mul_i32` .72, `unknown_n` .79, `add_i32` .82, `alias_ptr`/`restrict` ~.9, `gather`/`scatter` .9 |

`select_f32` is the sharpest datum: the loop is a scalar `ucomiss`/`jbe` diamond
(`movss` in both arms) on random data, so it is limited by branch mispredictions
(3.9 ns/element vs 0.11) — it is neither vectorised nor converted to a
select/blend/`cmov`. Excluded from the table as invalid or unexplained:
`saxpy` (re-initialises inside the timed region), `sqrt_f32` (negative inputs →
NaN path in both), `dot_f32_fast`/`sum_f64_fast` (compiled without fast-math in
both; the 2× on the dot product is unexplained and needs a look at the strict-FP
chain), `popcnt` (parity with scalar `popcnt`; GCC with AVX-512 `vpopcntd` is
~11× faster but the Raptor Lake target has none).

**F4 — `-O3` and `-march=native` change nothing for SIMD (P1, R).** `-O3` is
indistinguishable from `-O2` on all 44 kernels (within noise) and
`-march=native` on an AVX-512 host emits **0** `zmm` references and no opmask
use, identical to `-mavx2` (107 `ymm` references total; GCC `-O3 -mavx2`: 657;
GCC `-O3 -march=native -mprefer-vector-width=512`: 429 `zmm`, up to 2× faster on
the same kernels, e.g. `add_f32` 0.040 → 0.024, `popcnt` 0.26 → 0.023 ns/element).
Without any `-m` flag the baseline emits no `ymm` (SSE2 only), as designed.

**F5 — intrinsics API breadth is small beyond SSE/AVX (P1, R screening).** A
name-presence census of the GCC 13 headers against `include/*.h`
(`_mm*_name(` definitions or `#define`s; counts only — presence is not
correctness, and macro-generated names may be undercounted, so the first step of
SV-05 is the compile-based version of this test): total **534 of 5,173**
reference names (10.3 %). By group (reference/present): AVX 205/132, AVX2
208/92, SSE4.x 84/44, FMA 32/6, F16C 4/0, AVX-VNNI **8/0**, GFNI 30/3, VAES
8/4, VPCLMULQDQ 2/2, SHA 7/7, AES/PCLMUL 7/7; AVX-512F 1412/107, BW 284/64,
VL 1260/10, VL+BW 449/15, DQ 212/13, VNNI 12/2, VBMI2 51/1, BITALG 27/5,
VPOPCNTDQ 7/3, FP16 601/7. The bundled headers define 512-bit types and
generated wrappers (`gen_lcccsimd.py`); §9a covers the audited SSE–AVX2 subset.
For the stated primary target the relevant, currently missing extensions are
AVX-VNNI (`_mm256_dpbusd_avx_epi32` family), F16C, 256-bit GFNI/VAES forms and
the AVX2/FMA remainder (about half of AVX2 and 80 % of FMA names).

**F6 — AVX/SSE transition discipline is incomplete (P1, R in asm, impact
unmeasured).** In the `-O2 -mavx2` corpus, 2 of 18 functions that use `ymm`
registers return without `vzeroupper` (`k_matmul`, `k_alias_ptr`); the other 16
emit it (16 `vzeroupper` in total; GCC: 45). On Intel cores since Skylake a dirty
upper state turns later legacy-SSE instructions (libm, memcpy via SSE paths,
callers compiled without AVX) into merge-dependency stalls. Whether the omission
is a deliberate decision must be established first (search `DECISIONS.md`); if
not, it is a cheap, measurable fix.

**F7 — design notes from reading the code (S).**
- AVX-512 is present only as types, wrappers and a few native EVEX helper calls;
  there is no opmask register class in the allocator and no masked-tail or
  masked-condition model, so every AVX-512 gain in F4 is unreachable for the
  autovectoriser today.
- `lcccsimd.h` is generated and `-march`/`target()` gating is a per-instruction
  permission table (§9a SIMD-PERM-1) — good — but the table is not derived from
  one authoritative ISA description shared with the assembler encoder
  (`encoder/avx.rs`, 7,042 lines), so API, encoder and cost model can drift apart.
- Vector value passing is expensive through carrier frames (§9a SIMD-PERF-1),
  which also taxes small hot helpers and the intrinsics themselves.

### 9b.3 Programme

Goal: no corpus kernel more than 1.2× slower than the best oracle on AVX2, zero
SIMD miscompiles on the matrix below, and a measured AVX-VNNI/AVX-512 story, in
this order. Each item has a reproducer, gates (mutation-verified) and an
acceptance number. Effort S ≤ 1 day, M ≤ 1 week, L > 1 week.

| ID | Pri | Item | Effort | Depends |
|---|---|---|---|---|
| SV-01 | P0 | Fix the two-block-unroll / vectoriser index loss (F1) | S–M | — |
| SV-02 | P0 | SIMD scoreboard: correctness matrix + runtime table in the repo | M | — |
| SV-03 | P1 | Scalar if-conversion: `select`/`min`/`max`/`abs`/`clamp` (F3) | S–M | — |
| SV-04 | P1 | Vector conversions, widen/narrow, shifts, integer idioms (F3) | M | SV-02 |
| SV-05 | P1 | Intrinsics: compile-based coverage + execution differential (F5) | M | — |
| SV-06 | P1 | General loop vectoriser framework, staged (F2, F3) | L | SV-02, SV-04 |
| SV-07 | P1 | `-O3` / `-march=native` / vector-width policy (F4) | S | SV-02 |
| SV-08 | P1 | `vzeroupper` audit and policy (F6) | S | — |
| SV-09 | P2 | AVX-512 enablement: opmask class, ZMM, masked tails (F4, F7) | L | SV-06 |
| SV-10 | P2 | AVX-VNNI / F16C / GFNI / VAES for the Raptor Lake target (F5) | M | SV-05 |
| SV-11 | P2 | One ISA description shared by headers, encoder, cost model (F7) | M | SV-05 |
| SV-12 | P2 | Split `vectorize.rs` by recogniser family; one registry (F2) | M | SV-06 |

#### SV-01 · P0 — fix the lost index in the unrolled interleaved-access loop
**Reproducer.** The `k()` function above (also the `a_swapadd` shape);
`lccc -O2 f.c -o f && ./f` → segmentation fault; `CCC_NO_TWO_BLOCK_UNROLL=1`
makes it pass; `-O1` passes.
**Plan.** (1) Dump IR before/after the passes in the order of
`src/passes/mod.rs` for the 12-line reproducer (the repo's IR dump knobs; bisect
with the existing `CCC_NO_*` switches by pass rather than by source) and find the
instruction that defines `i+1` for the second unrolled copy; (2) establish
whether the definition is created and then deleted (DCE/CSE after a rewrite that
changed the use but not the value map) or never created (the unroller's
induction-variable substitution for GEPs with *two* index forms `2*i` and `2*i+1`
in the same body); (3) fix at the source of the invariant, not by a guard that
declines the unroll — then add the invariant check (every operand of a cloned
instruction is defined on every path to it) as a debug assertion in the cloner;
(4) run the full matrix of SV-02.
**Gates.** The 7-shape matrix as unit tests at `-O1/-O2/-O3`; a differential
generator for AoS-pair loops (strides 2–4, 2–3 fields, cross-field uses, trip
counts 0,1,2,3,7,8,9,31,33, element types f32/f64/i32/i16, with and without
`restrict`) compared with `gcc -O0` and under a page-guard harness (arrays placed
so that a one-element over-read or write faults); mutation: revert the fix and
see the matrix fail.
**Accept.** Zero failures on the generator for a day's seed range; reproducer
and shape matrix permanent. **Risk.** The same root cause may affect scalar
unrolling outside the vectoriser; extend the generator to scalar loops.

#### SV-02 · P0 — SIMD scoreboard (correctness first, then speed)
**What.** Commit the 44-kernel corpus (`kernels.c` as used here, split into one
file per class so failures localise) and a runner (to be created, e.g.
scripts/simd_scoreboard.py) that: (a) compiles with lccc in five configurations
(`-O1`, `-O2`, `-O3`, `-O2 -mavx2 -mfma`, `-O2 -march=native`) and with the
reference compilers (`gcc -O3 -mavx2 -mfma`, and `-march=native` with
`-mprefer-vector-width=512`); (b) checks checksums against `gcc -O0` with
`-ffp-contract=off` (and separately against a contracting reference when
`-mfma` is on, since fused results legitimately differ); (c) sweeps trip counts
{0,1,2,3,4,5,7,8,9,15,16,17,31,32,33,63,64,65,N−1,N}, source/destination
misalignments {0..63 bytes} and partial overlaps; (d) places each array at the
end of a page followed by `PROT_NONE` and at a page start preceded by
`PROT_NONE`, so over-reads/over-writes by vector loops fault; (e) runs the timing
table with paired, order-balanced repetitions, min and median, CPU frequency and
affinity recorded, and **writes machine-readable output**. Timing only informs;
the pass/fail gate is correctness + "no kernel regresses more than 3 % against
the checked-in baseline on the same host".
**Gates.** Fast lane: correctness over the reduced sweep (≈ 1 min). Slow lane:
full sweep + timing. **Accept.** F1 is caught by the fast lane; the table in F3
is reproduced from the committed corpus. **Effort** M.

#### SV-03 · P1 — scalar if-conversion (the 35× case)
**Why.** Independent of the vectoriser, branchy selects on unpredictable data
cost an order of magnitude (F3: `select_f32` 3.9 vs 0.11 ns/element; `abs_f32`,
`min_i32`, `clamp_i32` likewise). Doing this first is the highest ROI item in
the programme: it fixes scalar code everywhere and is the legality foundation for
vector blends.
**Plan.** (1) A CFG pass on SSA before out-of-SSA: for each triangle/diamond whose
arms contain only speculation-safe, cheap instructions (no loads that may fault
unless dominated by an access to the same object, no division, no calls, no
volatile/atomic), replace the join phi by a `select`; cost guard ≤ 2–3 cheap
instructions per arm, never across loop back edges that carry a loop-exit
branch; use profile data when present (PGO) and skip well-predicted branches
only when a profile says so. (2) Lowering: integer → `cmov`/`pminsd/pmaxsd`
(vector) / `pabsd`; float → `minss/maxss` only when NaN/signed-zero semantics
match the C source (the `x < y ? x : y` form maps exactly to `minss`; other forms
use compare + `blendv`/`and/andn/or`); `abs` → `andps` with sign mask. (3) Teach
the scalar peephole to keep `cmov` chains short (a long dependent `cmov` chain is
slower than a well-predicted branch on Raptor Lake — gate with the profile cost).
**Gates.** Random-data microbenchmarks (misprediction-bound) and sorted-data
controls (branch-friendly) so a pessimisation is visible; asm-shape tests;
differential on edge values (NaN, ±0, INT_MIN, INT_MAX); mutation: disable the
legality check for loads and watch a guard-page test fault.
**Accept.** `select_f32`, `abs_f32`, `min_i32`, `clamp_i32`, `condstore` scalar
versions ≥ 5× faster on random data, ≤ 3 % slower on sorted data. **Effort** S–M ·
**ROI** highest.

#### SV-04 · P1 — vector conversions, widen/narrow, shifts, integer idioms
**Why.** F3 rows: conversions 3.8–7.2×, widen/narrow 6.4–8.8×, `shift_i32` 5×,
`sad_u8`/`satadd_u8` 1.7–1.8×. Each is a single-instruction (or two) vector
operation whose scalar form is a 6–9× slower loop.
**Plan.** Add recognisers *through the framework of SV-06 stage S1* where
possible; before that exists, extend the elementwise pattern pass with a table:
`cvttps2dq/cvtdq2ps/cvtps2pd/cvtpd2ps` (VEX 128/256), `pmovzx/pmovsx` +
`packus/packss` + `vpermq` fix-up for widen/narrow, `psll/psrl/psra` immediate
and `vpsllv*` variable forms, `pabs*`, `pmin/pmax[su]*`, `psadbw`,
`paddus*/psubus*`, `pavgb`, `pmulhw/pmulhuw`, `pmaddwd`. Semantics must follow C
exactly: float→int conversion of out-of-range values is undefined in C but the
hardware gives `INT_MIN`; the scalar code does the same, so vector and scalar
agree — assert this in the differential sweep rather than assuming it.
**Gates.** SV-02 sweep, full 2^16 exhaustive checks for 16-bit element ops,
random + edge sweeps for 32-bit; asm-shape gate that the loop contains no scalar
`cvtsi2ss`/`cvttss2si`. **Accept.** Each listed kernel ≤ 1.2× of GCC AVX2.
**Effort** M.

#### SV-05 · P1 — intrinsics: compile-based coverage and execution differential
**Plan.** (1) A coverage test that, for every `_mm*`/`_mm256_*`/`_mm512_*` name
in the SHA-pinned GCC 16.2 headers (the inventory already exists for SSE–AVX2 in
`engineering/isa/`; extend to every `*intrin.h`), compiles a translation unit
referencing the name under the minimal target flags and records
*implemented / diagnosed / missing*; this replaces F5's regex census. (2) An
**execution differential**: generate, from the GCC header prototypes, a program
per intrinsic that applies it to random and edge inputs (including all-ones,
zero, NaN payloads, denormals, shift counts at and beyond the lane width) and
compares the bytes with GCC's result on the same host; run the AVX-512 subset on
an AVX-512 host only. (3) Order by value to the target: AVX2/FMA remainder,
AVX-VNNI, F16C, GFNI/VAES/VPCLMUL 256-bit, then AVX-512F/BW/VL/DQ/VNNI.
(4) Every intrinsic must lower to **one** native instruction (or the documented
sequence); add an asm-shape ratchet over the corpus so no helper call or carrier
copy creeps back (links to §9a SIMD-PERF-1).
**Accept.** AVX/AVX2/FMA/F16C/AVX-VNNI at 100 % *implemented or diagnosed* with
execution differentials green; AVX-512 coverage ≥ 90 % of F/BW/VL/DQ names.
**Effort** M (coverage + differential harness), then incremental.

#### SV-06 · P1 — a general loop vectoriser, introduced behind the existing passes
**Design.** One pass, five layers, each testable alone:
1. **Loop forms.** Counted innermost loops (and the trip-count algebra
   already in `loop_bound`/`loop_analysis`): canonical IV, bounds, runtime trip
   counts, exit conditions; reject early with a *reason code* (see below).
2. **Memory model and legality.** Affine subscripts per base object; for each
   store/load pair compute dependence distance (GCD/Banerjee-lite is enough for
   the corpus); classify: independent, loop-carried with distance ≥ VF (safe),
   distance < VF (unsafe, or safe when the vectorised order preserves the
   read-before-write relation — `a[i]=a[i+1]+b[i]` is the canonical safe case),
   unknown (emit a **runtime alias/distance check** with a scalar fallback loop;
   the pointer-pair check cost enters the cost model). Reductions: associative
   integer ops always; FP only under `-fassociative-math` (strict FP keeps the
   existing multi-accumulator interleave, which preserves order only per lane —
   verify that this is what the code does before relying on it).
3. **Plan and cost.** A `Plan {vf, interleave, epilogue, masks}` per loop; cost =
   µops/throughput per instruction class from the calibrated tables (MB-30),
   shuffle and gather penalties, register-pressure estimate, trip-count break-even
   against versioning and epilogue overhead; choose among 128/256/(512) widths by
   ISA and by policy (SV-07).
4. **Widening.** Elementwise arithmetic, compare → mask, select → blend, masked
   stores (`vmaskmov`, AVX-512 opmasks later), conversions (SV-04), induction
   variables as vectors, interleaved/strided/reverse access via shuffles
   (`vpermd/vpermq/vpshufb/unpck*`; AoS↔SoA, complex multiply with
   `addsub`/`fmaddsub`), gather/scatter only when the model says so (lccc's scalar
   gather already ties GCC).
5. **Emission and epilogues.** Scalar remainder (existing), vector epilogue at
   half width, masked epilogue when AVX-512 or `vmaskmov` is cheaper; alignment
   peeling only when measured to pay.
**Staging (each stage ships behind `CCC_GENERIC_VEC=1`, then default-on after
the identity and scoreboard gates pass; the existing pattern passes stay as
fast paths and are removed only when the generic path matches or beats them on
their own kernels):** S1 elementwise unit-stride (includes SV-04 conversions);
S2 if-conversion and masked stores (`condstore`, `select`, `abs`, `clamp`);
S3 reductions (int add/min/max, `sad`, `dot` via `pmaddwd`, FP with
`-fassociative-math`); S4 strided/reverse/interleaved; S5 runtime alias and
dependence-distance versioning (`overlap_dist1`, `alias_overlap`); S6
gather/scatter.
**Explainability.** Every loop that is not vectorised records a reason code
(`-Rpass-missed=vec`-style output or `-fopt-info-vec-missed` equivalent, tied to
the compiler-explainability goal of the original charter): "unknown dependence
a/b", "non-affine subscript", "unsupported cast", "cost: not profitable (vf=8,
est 1.4× slower)". These feed the scoreboard so every remaining gap has a
named cause.
**Gates.** SV-02 matrix on every stage; byte-identical output when the knob is
off; an IR-level legality fuzzer (random affine loop nests with seeded
dependences, compared against scalar execution); mutation of the dependence test.
**Accept (end of S5).** 44-kernel corpus: no kernel > 1.2× GCC AVX2 except
documented exclusions; geometric mean ≤ 1.05×; zero correctness failures.
**Effort** L (S1+S2 deliver most of F3).

#### SV-07 · P1 — `-O3`, `-march=native` and vector-width policy
**Plan.** (1) Decide and document what `-O3` means (more aggressive
unrolling/interleave limits and cost thresholds, never different semantics) or
make it an explicit alias; add a scoreboard assertion so it cannot silently stay
identical. (2) `-march=native` must select the host ISA (it currently produces
the same code as `-mavx2` on an AVX-512 machine): detect through the existing
`cpu_model.rs`, and honour `-mprefer-vector-width=128|256|512` and
`-mno-avx512…`. (3) GCC 16 vectorises at `-O2` with a conservative model; decide
the lccc policy (the baseline emits SSE2 only without `-m` flags, deliberately;
record the decision and its measured basis). **Accept.** Policy documented in
`docs/`, scoreboard rows for each flag combination. **Effort** S.

#### SV-08 · P1 — `vzeroupper` audit
**Reproducer.** `lccc -O2 -mavx2 -S kernels.c`: functions using `ymm` and
returning without `vzeroupper` (`k_matmul`, `k_alias_ptr`).
**Plan.** (1) Find out whether the omission is deliberate (a loop that ends with
upper halves already zero, or an ABI argument); (2) implement the standard rule —
`vzeroupper` before `ret`, before calls to functions not known to be VEX-clean,
and before inline asm that may use legacy SSE, unless the function takes or
returns a 256/512-bit value; (3) a dirty-upper-state data-flow so the
instruction is emitted once per exit, not per use. **Gates.** Asm-shape test over
the corpus; a mixed legacy-SSE/AVX callee test measured with a paired runner on
the target hardware (the Intel stall is hardware-dependent — report per CPU).
**Accept.** Zero ymm-using functions without it unless justified in
`DECISIONS.md`. **Effort** S.

#### SV-09 · P2 — AVX-512 enablement (oracle hosts, EPYC, Sapphire Rapids; not Raptor Lake)
**Plan.** (1) Allocator: an opmask register class (`k0–k7`, `k0` special),
ZMM16–31 and YMM16–31 availability with the EVEX 32-register file; (2) EVEX
encoder coverage audit against the SDM tables for every instruction the
generator may emit (including `{k}{z}` masking, embedded broadcast `{1toN}`,
disp8*N compression, `vpternlogd`); (3) codegen: masked tails (`vmovdqu8/16/32/64
{k}{z}`), compare-into-mask, `vpcompress/vpexpand`, `vpopcnt*`, `vpermt2*`,
`vpconflict*` where idioms exist; (4) policy: prefer 256-bit EVEX (VL) on cores
where 512-bit lowers frequency, 512-bit where measured profitable (SV-07);
(5) never emit AVX-512 without `-mavx512*` or `-march` that implies it.
**Gates.** SV-02 on an AVX-512 host; execution differentials (SV-05);
encoder round trip against GAS 2.47 for the full emitted set. **Accept.**
`-march=native` AVX-512 builds beat `-mavx2` on ≥ 50 % of corpus kernels with no
regression above 3 %. **Effort** L.

#### SV-10 · P2 — extensions the Raptor Lake target actually has
AVX-VNNI (`vpdpbusd/vpdpwssd` VEX, 0 of 8 header names present), F16C (0 of 4),
256-bit GFNI/VAES/VPCLMULQDQ, SHA, BMI2. **Plan.** Intrinsics (SV-05), plus
idioms: `dot_i32`-style int8/int16 multiply-accumulate loops → `vpdpwssd`/`vpdpbusd`
(semantics: `vpdpbusd` is u8×s8 with *no* saturation, `vpdpbusds` saturates —
select by the C expression, never by taste), `_Float16` ↔ float with F16C
(MB-11 item), byte-LUT popcount with `vpshufb` where profitable (no
`vpopcnt` on this core), `crc32`/`pclmul` folding left to intrinsics.
**Gates.** Execution differentials; paired timing on the 14700KF. **Effort** M.

#### SV-11 · P2 — one ISA description for headers, encoder and cost model
Today the wrapper generator (`scripts/gen_lcccsimd.py`), the permission table
(§9a SIMD-PERM-1), the assembler encoder (`encoder/avx.rs`) and the scheduler's
cost constants each describe instructions separately. **Plan.** A single
checked-in machine-readable instruction table (mnemonic, operand forms, encoding
class VEX/EVEX, ISA feature set, latency/throughput/ports from the MB-30 tables,
C prototype(s), semantics tag) from which generators emit the header wrappers,
the permission table, encoder tables and cost entries; a gate that regenerating
produces no diff. **Accept.** Adding an instruction touches the table once.
**Effort** M.

#### SV-12 · P2 — split `vectorize.rs` and unify kill switches
Structure only, byte-identical output (identity gates from MB-20 apply): one
module per recogniser family behind a `Recogniser` trait (`matches(loop) →
Option<Plan>`, `apply(plan)`), one registry, one `CCC_NO_VEC_<NAME>` convention
generated from it and documented through the knob registry (AUD-KNOB-1).
Do after SV-06 S1 so the new framework defines the trait. **Effort** M.

### 9b.4 Milestones
- **SM-1 (correct):** SV-01 and SV-02 landed; the F1 matrix and sweep green in
  the fast lane.
- **SM-2 (no cliffs):** SV-03 and SV-04; no corpus kernel above 3× GCC AVX2.
- **SM-3 (general):** SV-06 S1–S5 with reason codes; no kernel above 1.2× GCC
  AVX2; geometric mean ≤ 1.05×.
- **SM-4 (wide):** SV-07/SV-08/SV-10 and SV-09; AVX-512 hosts beat AVX2 builds;
  AVX-VNNI/F16C on the Raptor Lake target measured on real hardware.
- **Beating GCC/Clang/ICX** means passing the *oracle* comparison on the
  committed corpus with GCC 16.2, Clang 23.1 and ICX on identical flags, on real
  hardware, with the raw pairs published — not the VM screening above.

### 9b.5 Implementation follow-up (2026-10-31)

- **SV-01 root cause fixed; extended acceptance still open.** The AoS crash
  is caused by backend dead-offset suppression, not a missing unroller IR
  definition. A value peeled through by one address remains the selected SIB
  index of another. Protect all selected indices across the function, retain
  store-fallback offset chains, and make shared-offset deletion a worklist
  fixed point. No unroll/vectorization kill switch is needed. Five Rust
  regression cases pin these invariants.
- **SV-01/SV-02 regression foundation:** the seven-shape runner now fails
  closed on compilation/runtime/reference errors, compares every output
  element, uses private scratch directories, emits optional JSON and supports
  18 trip counts, start/end guard pages, four element types, pair strides 2–4
  and bounded mixed-sign inputs. Local GCC 16.2 / Clang 23.1.4 differentials:
  2,268 trip/layout checks plus 1,008 mixed-sign/type/stride checks passed.
  GCC and authenticated ICX CI receive permanent smoke gates. See
  `tests/simd/README.md` for commands, scope and oracle limitations.
- **Still open:** SV-01's randomized one-day seed/page/misalignment/overlap
  acceptance and scalar-loop extension; SV-02's hardened 44-kernel scoreboard,
  controlled paired timing and full sweep. SV-03–SV-12 remain open; this fix
  does not implement a general vectorizer, extend ISA coverage, or establish
  target-hardware speedups. The offline Godbolt cross-add witness still shows
  a substantial static-code gap against GCC/Clang; instruction counts are not
  runtime wins. SM-1 is not yet complete.

### 9b.6 Red-team follow-up (2026-10-31)

The address-fold audit repaired alias over-crediting, alias fallback root
protection, missing-count handling and effectful-producer suppression. A
256-case independent edge oracle and target/store-profile tests protect the
new proof boundary. x86-64's explicit guaranteed-store contract recovers
`glibc_memcmp` from 156 instructions / 18 stack operands to 154 / 17 without
changing the golden gate. Other targets retain fallback offsets.

SV-02 now also has a strict **44-name checksum-only gate**. Signed sink overflow
(reproduced with GCC 16.2 UBSan), negative signed left-shift, truncated FP
receipts, cross-row pointer traversal and zero matmul repetitions were repaired.
GCC and authenticated ICX CI run this gate; controlled timing is still open.
AoS probe compilation now denies warnings and process timeouts kill compiler
descendants as well as their parent. The complete audit and its measured limits
are in `engineering/SIMD-ADDRESS-FOLD-AUDIT-2026-10-31.md`.

## 10. Carried over from the superseded live queue (full text in the archives)

Everything open in the 478-line queue that this file replaces is listed here with
its acceptance criterion. Statuses are the previous queue's own (revalidated there
only where its gates were rerun); re-verify a reproducer before starting. Full
narrative and evidence links:
[`engineering/archive/BACKLOG-2026-10-24-live-queue.md`](engineering/archive/BACKLOG-2026-10-24-live-queue.md).

| ID | Item | Accept |
|---|---|---|
| UP-1 (was P0.1) | `ext_vector_type` layout: both historical mismatches re-measured **closed** (receipts: layout-matrix gate, 12 types × 10 lane counts vs Clang 23.1.3, `-m64`/`-m32`, `-O0`/`-O2`). Remaining: cross-compiler receipt for the bit-packed `bool` rule from a Clang/ICX hosted lane | Hosted-lane receipt; invalid forms diagnose |
| UP-2 (was P0.2) | Option acceptance: policy landed (see MB-01). Remaining: classification review of `-flto`/sanitizers (now hard errors — verify intent), and a local replay of the `-fwrapv` byte-address boundary witnesses at `-O1`/`-O2` on every supported target | Modular result preserved across levels and targets; wrapping and non-wrapping controls kept |
| UP-3 (was P0.4) | RISC-V relocation error/preservation contracts: replay archived malformed-expression and mutated-relocation fixtures; enumerate applied/retained/relaxed/rejected classes for exec and shared outputs | No unsupported expression becomes zero; malformed metadata rejected in both output modes; valid symbolic subtraction stays a positive control |
| UP-4 (was P1.1) | Fannkuch runtime regression: explain before changing code | Matched paired runner; mechanism identified before any edit |
| UP-5 (was P1.2) | Zstandard code quality: `ZSTD_decompressSequencesLong_default` 2517 instructions / 871 frame refs vs GCC 1786 / 401; `ZSTD_safecopy` 211 / 58 vs 152 / 0; no victory claim from instruction counts | Output-checked paired runtime on useful durations; all regressions retained |
| UP-6 (was P1.3) | Validate on real target hardware (Raptor Lake 14700KF PMU, EPYC 7763): VM numbers are screening only | Raw pairs, CPU/microcode, affinity, frequency policy, hashes, PMU availability; no universal "best compiler" claim |
| UP-7 (was P1.4) | Fresh current-tree comparison and required local gates after any rebase | Frozen main vs candidate, recompiled corpus, immutable identities |
| UP-8 (was P1.5) | Scalar min/max/abs lowering beyond the landed `max(0, a-b)` sign-test CMOV case (6→5 instructions; GCC 4, Clang/ICX 5): enumerate remaining shapes legal from exact flag semantics | Exact differential over the arithmetic domain incl. wrapping edges; mutation-sensitive gates; same-width and intervening-flag barriers kept |
| UP-9 (was P1.6) | IPA-RA known-callee clobber sets: `hot_web_loop` 1.549× GCC 16.2 (99.8 ms median), 18 stack refs vs 0, caused by assuming every call clobbers all caller-saved registers around a `static noinline` helper | Per-callee written-register summaries in reverse topological order; conservative for recursion, indirect, escaped and external-ABI calls; no allocator miscompile; paired runtime on the 49-program corpus |

**Rejected/stale claims carried forward** (do not revive without a new failing
control): the old A14/PR834/PR837 dated statuses are historical; `oracle_asm.py`
and `tune_oracle.py` returning success on failure is stale (nonzero now, 40 offline
tests); the BZHI header-asm optimisation is rejected on measured negative evidence;
Callgrind `Ir`, static counts and single-run VM timings are not runtime wins; the
10k-line SIMD-stack PRs are **unvalidated, not accepted** (no matching 16-byte XMM
stack operands in the 49-program corpus); keep the `CCC_LAYOUT_TIE_ORIG=1` rollback
and the strict stack rule; the layout-tie benchmark number is not portable across a
rebase.

**Older item families** (IDs live in the archives; re-verify before use):
evidence/benchmark integrity A14-02/03/04/06/07/22/23/26/27/28/32/33; structure
A14-01/08/09 (typed AST identity — includes the pointer-address `ExprId` hazard)
/10/11/12 (giant functions)/18/24/25; correctness A14-15/17 (RISC-V), A14-31
(lossy ELF string decoding), ANON-KEY-1, PARSER-ALIGN-1, CONST-QUAL-1, INTPTR-1,
REDEF-1, IVSR-OFFSET-1; performance PF-*, RA-*, PERF-1..6, SETMEM-SHAPE-1,
RANGEFOLD-OR-1, CONSTLOOP-FOLD-1, WR-COND-RETEST, MI-*, IVOPTS-1,
LOOP-PREHEADER-3; infrastructure INF-HARNESS-1, INF-BENCHGATE-1, INF-FRESHCLONE-1,
INF-GAS-1, INF-LINK-1, INF-PGOPID-1, INF-M32HDRS-1, EDG-TRANSPLANT.

## 11. Definition of done for the whole programme

Maybach is reached for x86/i686 when the MB-42 harness shows: **zero GAP** rows
in pillars 1–3 for the Arch and Debian default flag sets; C23 census 40/40; gdb
and `addr2line` gates green on x86-64 and i686; the linker differential oracle
(MB-33) green against bfd/lld/mold for the curated structural set; compile
instructions at `-O1`/`-O2` within 2× of GCC's on the 300-function input
(measured with a release build, MB-22); and the performance corpus gates of §8
unchanged or better.

Added 2026-10-27 for the §12 register (these are the conditions under which the
phrase "GCC 16.2 compatible" is allowed in any document):

* the option-effect ledger (GP-22) has no accepted-and-inert row in the priority
  set, and the ledger gate runs in the fast lane;
* the attribute inventory (GP-01) is a pinned list with zero *silently* ignored
  attributes — every GCC 16.2 attribute is implemented or diagnosed;
* the macro differential (GP-23) reports an empty forbidden list and an empty
  missing list for the essential set, with `__GNUC__`/`-dumpversion` consistent;
* the cross-compiler ABI matrix (GP-02) is green for every accepted calling
  convention, and no ABI-selecting attribute is accepted unobserved;
* `__has_builtin` has zero over-claims (GP-18), and inline-asm constraint
  verdicts match GCC on the full matrix and `-pedantic` (GP-10) is load-bearing;
  both are gate-enforced.

Until then, report progress per row of §2 or per row of §12.1, never as a single
adjective.

---

## 12. GCC 16.2 parity register (measured 2026-10-27, `dc878b3`)

Source of the original 2026-10-27 measurements below:
[`docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`](docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md)
(oracle GCC 16.2.0, Clang 23.1.4). The report carries the raw numbers, the
reproducer commands and the risk ranking; this section carries the work.

Scope note: "parity" means **the GCC 16.2 C contract on x86-64 and i686** —
language, diagnostics, attributes, builtin namespace, driver/warning surface,
tooling flags, debug info and object ABI. GCC's other back-ends, C++, and
Objective-C are out of scope and are not tracked here.

Ordering note: the rank column is **execution order**, not raw ROI — GP-22 is
sequenced ahead of GP-01 (4.6) because the ledger is what turns every later item
into a permanent ratchet; where two items are interchangeable the higher ROI comes
first.

Class legend: **W** = wrong code / silent semantic change (highest priority),
**S** = silent no-op (accepted, does nothing), **D** = missing diagnostics,
**F** = missing feature, **T** = missing tooling substrate, **P** = process gate.

### 12.0 Bounded GP-01–GP-24 sentinel verdicts (2026-10-28)

These are **sentinels**, not the full acceptance suites. Unless noted,
GCC is 16.2.0, target is x86-64, LCCC is the integrated fastbuild, and each
row reports only the specified independent fixtures and named targeted gates. A matching sentinel is not a claim of item
completion. `OPEN` means the acceptance criterion has not been met or tested;
`PARTIAL` means some narrower evidence exists.

| Item | Verdict | Observed evidence / remaining acceptance gap |
|---|---|---|
| GP-01 | PARTIAL | 40 attribute cases and two glibc headers at four optimization levels pass; 127 GCC-known names, 83 known-but-unimplemented inventory entries, and 48 implemented names (four are Clang-only). Semantic implementation of the 83 is not implied; function-typedef attribute diagnostics remain open. |
| GP-02 | OPEN, wrong code | `ms_abi` four-int sentinel: GCC/GCC returns success; GCC caller + LCCC callee returns 42 instead of success; LCCC caller + GCC callee dies SIGSEGV. No complete mixed-ABI matrix. |
| GP-03 | OPEN, wrong code | The C `retain` sentinel still emits plain `.data` with `WA`, versus GCC `.data.retained` with `WAR`; frontend propagation is absent. The five integrated assemblers now preserve a uniquely named `R` section and GNU OSABI, and x86-64 `ld -r` preserves R/non-R separation. Same-named assembler variants are refused, and LCCC i686 userspace GC remains unimplemented. This is infrastructure, not attribute support. |
| GP-04 | OPEN, wrong code | x86 interrupt sentinel under `-mgeneral-regs-only`: GCC ends in `iretq`; LCCC ends in `ret`, without a diagnostic. Other contract attributes remain unaudited here. |
| GP-05 | PARTIAL | The wrong predefines are gone and the missing compiler-owned family landed, measured on one x86-64 GNU23 empty TU: GCC-only shrank 153 → **17** and LCCC-only 211 → **33** (GCC 408 / LCCC 424 macros; `EOF`, `BUFSIZ`, `NULL`, limits, `PRI*`, errno/signals, `memory_order_*` and `__STDC_NO_ATOMICS__` no longer predefined; 136 GCC values added verbatim, incl. the `__U<INT…>_MAX__`/`_C()` limit family, atomics lock-free/compare-exchange claims and the `_FloatN` capability probes). Remaining: the 17 GCC-only names are deliberate skips (decimal FP, unverified `__GCC_*` asm/CFI claims, HLE, C++ ABI), 33 pinned known-deltas (keyword-vs-macro mechanisms, GCC's `-dM` omission of `__FILE__`/`__DATE__`, PIC/PIE and C23 decimal/width claims) and 34 value spellings; `__GNUC__` 14 stays a documented deliberate choice. One target/dialect only; ratchet = GP-23. |
| GP-06 | OPEN | GCC accepts one `_BitInt(7)` parameter/return sentinel; LCCC rejects it. No width, ABI or runtime matrix. |
| GP-07 | PARTIAL | Digraph punctuation, `%:` directives and `%:`/`%:%:` macro operators now have a GCC 16.2 differential: 22 native/i686 executions over five dialects, five integrated-target object emissions, 40 overlapping-punctuator cases and negative controls pass. Stringification preserves spelling. This is bounded, not a full preprocessing or pedantic audit; GCC accepts the UCN identifier sentinel and LCCC still rejects it. |
| GP-08 | PARTIAL | One C23 empty-prototype call sentinel is rejected by both compilers; this does not establish the full strictness/over-acceptance inventory. |
| GP-09 | PARTIAL | The registry now drives a 17-kind × 13-case matrix (207 rows), the format-level rows, the MB-01 rows and a 77-program zero-LCCC-only precision sweep, all two-sided against GCC 16.2.0 and wired to hosted CI next to the 57 precedence cases (both green locally). Remaining: the Pedantic *group* slot (no members; the leaf `-Wpedantic` is GP-10), corpus-wide (`tests/corpus`, 1 434 files) warning precision beyond the benchmark sweep, and the documented divergences (format-family `-Werror=format` promotion, `undeclared` compat switch). |
| GP-10 | PARTIAL | The sentinel class is implemented: `({ ... })` statement expressions now warn `ISO C forbids braced-groups within expressions [-Wpedantic]` under `-pedantic`/`-Wpedantic`, error under `-pedantic-errors` (plain `[-Wpedantic]` label) and under `-Werror=pedantic` (`[-Werror=pedantic]`), silent by default and under `-Wall`/`-Wextra` - all measured against GCC and pinned in the 17-kind matrix; `-pedantic` is no longer a tolerated no-op. Remaining: the rest of the extension inventory (case ranges, `$`, old-style asm, pre-C23 dialect differences) and the reserved Pedantic group slot. |
| GP-11 | OPEN | GCC reports one definite uninitialized use under `-O2 -Wall -Wuninitialized`; LCCC has no corresponding warning. |
| GP-12 | PARTIAL | `-Wvla` landed: default-off, no group membership (measured), exact message `ISO C90 forbids variable length array '<name>'` once per declarator, `-Werror=vla` promotion verified, matrix green in CI. The sweep also exposed an unpinned family gap: GCC's `-Wunused-but-set-variable=` (trailing `=` label) has no LCCC counterpart. Remaining: shadow, conversion/sign-conversion, overflow, switch{,-enum}, missing/strict-prototypes, return-local-addr, array-bounds, stringop-*, cast-qual, deprecated-declarations, format=2. |
| GP-13 | PARTIAL, honest refusal | Both coverage spellings are now named refusals with the gcov gap spelled out (`-fprofile-arcs is not implemented: lccc emits no .gcno/.gcda and no __gcov_* counters...`), replacing the earlier silent LCCC-profile substitution. GCC's side is recorded as the ledger divergence and re-measured every run: `-ftest-coverage` writes a `.gcno` notes file, bare `-fprofile-arcs` does not, and neither exists in LCCC. Remaining: emit `.gcno`/`.gcda` for gcov or keep the refusal permanent and documented. |
| GP-14 | PARTIAL | The no-profile sentinel is now warned by LCCC (`no lccc profile files`) and GCC (`-Wmissing-profile`); `.gcda` interop and version/round-trip contracts remain unverified. |
| GP-15 | OPEN, honest refusal | One `-fsanitize=address` compile is accepted by GCC and refused by LCCC by name. No ASan runtime support claimed. |
| GP-16 | PARTIAL, honest refusal | `-flto` refusal re-verified in the ledger gate (`LTO IR production and link-time consumption are not implemented`), and `-fwhole-program` moved from accepted to **proven effective**: the symbol loses `.globl` under the flag in both compilers (ledger row, byte-differing assembly). `-static-pie` no longer tolerated-inert: it is now a named driver refusal (was accepted-and-inert), with GCC's `-###` forwarding recorded as the divergence. Remaining: real LTO/`-ffat-lto-objects` handling. |
| GP-17 | PARTIAL | Inline-asm constraint *direction* validation matches GCC 16.2 exactly: input `=`/`+` (`input operand constraint contains '='/'+'`), output without a write modifier (`output operand constraint lacks '='`), and a write modifier after a comma (`... incorrectly positioned '+' or '='`), with 7 valid shapes still accepted - a 12-row two-sided gate in CI. Measured residuals, deliberately not half-built: the `%`-commutator position rule (`'%' constraint used with last operand`) and the alternative-count rule (`differ in number of alternatives`). The full 65-case matrix from the original register still stands as the acceptance bar. |
| GP-18 | PARTIAL | The over-claim is fixed and ratcheted: all **246** registered `__builtin_ia32_*` names × **14** flag sets (default, sse3, ssse3, sse4.1, sse4.2, avx, bmi, bmi2, avx2, fma, f16c, aes, pclmul, avx512f) now answer exactly like GCC 16.2 - 110 gated names (16 SSSE3, 5 SSE4.1, 4 SSE4.2, 16 AVX, 6 BMI2, 63 AVX2) via a provenance-documented table plus macro-implication rows (both compilers define the same gating macros per flag). Remaining: the historical 5 501-name audit outside the ia32 namespace and non-x86 builtin gating. |
| GP-19 | OPEN, honest refusal | GCC accepts `-mstackrealign`; LCCC refuses it by name. Other ISA/mitigation flags not measured in this sentinel. |
| GP-20 | PARTIAL | With `-g -O0`, LCCC now emits compile-unit, subprogram and unspecified-type DIEs, plus line tables; GCC's same fixture also has two base types, one parameter, two members, one structure and two variables. The earlier line-only statement is stale; full DIE/debugger acceptance remains open. |
| GP-21 | PASS (sentinel closed) | All 4 checked queries answer like GCC 16.2.0 and a 10-row gate runs in CI: `-dumpfullversion` (LCCC identity `14.2.0`, dotted like GCC's own), `-print-multiarch` (exact, incl. `-m32` → `i386-linux-gnu`), `-print-multi-lib` (byte-exact `.;` + `32;@m32`, same under `-m32`), `-print-sysroot` (0 bytes, like GCC with no `--sysroot`). Bonus fixes: `-print-multi-directory` was silently wrong under `-m32` (`.` vs GCC's `32`) and is now target-aware; the `-dumpmachine` configured-vs-canonical tuple difference and the `--sysroot=` refusal are documented divergence rows. |
| GP-22 | PARTIAL | The priority-set ledger exists and is wired to hosted CI: 10 rows prove effect-or-refusal for all 7 priority options - `-Ofast` (= `-O3 -ffast-math`, byte-identical) and `-Og` (= `-O1`, byte-identical; GCC's own `-Og`==`-O1` re-measured on the scalar probe) are now *implemented* instead of tolerated, `-Wattributes` fires in both compilers, `-pedantic` accepts and acts, `-fprofile-arcs`/`-ftest-coverage` are refused by name with GCC's gcov-notes behaviour pinned as the divergence, and `-static-pie` joined the named-refusal class (`-flto`, `-fsanitize=address` refusals re-pinned). `-fno-delete-null-pointer-checks` (passed by every kernel translation unit) joined as an Inert row with `probe-nullptr-checks-inert`, probed by `check_nullptr_check_policy.sh`, and the kernel harness now refuses to build a configuration whose package features it drops. Remaining: the wider accepted-silent backlog (the historical 157), fail-closed `LCCC_STRICT_OPTIONS` coverage as a gate, and every other accepted spelling getting a row. |
| GP-23 | PARTIAL | The ratchet gate landed: `check_macro_parity.py` pins the complete GNU23/x86-64 differential as three reviewed budgets (17 GCC-only, 33 LCCC-only, 34 value spellings), fails on any new divergence in either direction, and also fails on *stale* pins so the budgets can only move through a reviewed edit. Wired to CI next to the warning gates (gate-parity check green, 434 commands). Remaining: more targets/dialects (single target×dialect now) and converging the 34 value spellings. |
| GP-24 | PARTIAL | Seven derived gzip/zlib-ng/Expat kernels passed bounded GCC/Clang/LCCC checks; compare256 covers four optimization levels, two ABIs and a detected mutation. Verified zlib-ng 2.3.3 configure/Make static builds passed upstream `alltests` under GCC and LCCC, followed by 48 cross-compiler roundtrips and 24 system-gzip decodes. Manually configured upstream Expat 2.9.0 passed 5,196 checks with GCC, Clang and LCCC libraries, including an LCCC-compiled runner. Its bounded paired benchmark on this VM measured LCCC/GCC median CPU-time ratio 1.3161 [1.3077, 1.3206], not a speedup. Expat configure/CMake/CTest, other configurations and kernel-style diagnostic matrices remain open. |

**2026-10-30 progress (attribute applicability, `noplt`, two warning families).**

* GP-01 — placement verdicts are now taken once per *declaration* instead of
  per attribute position, matching GCC's rule that `int x __attribute__((noreturn));`
  and `__attribute__((noreturn)) int x;` are the same mistake. Concretely:
  specifier-position and post-declarator records survive to a single
  declaration-level verdict; a trailing attribute on a later comma sibling
  (`int a, b __attribute__((noreturn));`) is diagnosed too, which the previous
  code dropped entirely. Verdicts report at the declaration's own location
  (a leading `__attribute__` list included), which is what GCC prints
  (`__attribute__((packed)) int x;` → column of the declaration, not of the
  name). Two dedicated wordings measured and pinned: `'naked' attribute only
  applies to functions`, `'noplt' attribute is only applicable on functions`.
  Duplicate parser/sema verdicts are collapsed in `DiagnosticEngine::emit` on
  (file, span, message); see the second-pass note below for the per-list salt
  that keeps two real lists from collapsing into one. Remaining known
  micro-divergence: when the attribute sits on a later *line* than the
  declaration start, GCC's caret lands on that line's first token while LCCC
  reports the declaration start; and `'x' attribute directive ignored`
  (unknown names) still points at the name token. Both are recorded, not
  silently accepted.
* `noplt` is implemented rather than inventoried: `-Wattributes` was a false
  positive on functions, and the semantic was missing. It is now a per-symbol
  `-fno-plt`: calls dispatch through the GOT (`call *f@GOTPCREL(%rip)`),
  verified byte-for-byte against gcc-16 on extern declarations, comma
  siblings, post-declarator placement, block-scope declarations and
  same-TU definitions (where GCC deliberately keeps the PLT call — so does
  LCCC), plus leak controls proving the flag does not spread to neighbouring
  declarations.
* GP-09/GP-12 — `-Wdiv-by-zero` (default-on, `-Wall` label, `-Wno-`
  effective) and `-Wstrict-prototypes` (opt-in, silent under `-std=c23`
  because `()` is a prototype there) both match GCC's wording, gating and
  diagnostic shape. `-Wdiv-by-zero` follows GCC's syntactic rule: it fires
  inside `if (0)` and in both arms of a non-constant `?:`, but not for the
  right operand of a deciding `&&`/`||`, the untaken arm of a constant `?:`,
  or inside `sizeof`/`_Alignof`.
* GP-22 — the option-effect ledger gained three rows (`-Wdiv-by-zero`,
  `-Wstrict-prototypes`, `noplt`) and its priority set grew to 10; every row
  was mutation-verified (removing the verdict fails the gate) and the
  `-Wdiv-by-zero` row compares the warning *count* against GCC on seven
  short-circuit/unevaluated shapes so the suppression rule cannot drift.
* Stale claims corrected in this register: the GP-03 `retain` sentinel now
  passes (measured: plain `__attribute__((retain))` → `.data.<name>`, flags
  `WAR`, identical to gcc-16; `noinit`/`persistent` likewise), and
  `-fstack-clash-protection` really does emit the probe loop for an 8 KiB
  frame. `tests/regression/check_attribute_diagnostics.sh` had pinned the old
  false negatives (`int x __attribute__((pure));` and friends expected silent,
  while GCC warns); it now probes each attribute in a position GCC accepts and
  keeps a separate misapplied class.

**2026-10-30, second pass (declaration-level verdict anchors and `cleanup`).**
Measured against gcc-16.2.0 with a differential harness (51 probes over
`-Wattributes`, `-Wdiv-by-zero` and `-Wstrict-prototypes`); the exact-match
score went from 32/51 to 41/51 with the remaining ten accounted for below.

* One verdict per attribute *list*: the engine's warning dedupe keyed on
  (file, span, message), which folded
  `__attribute__((noreturn)) __attribute__((noreturn)) int x;` into a single
  warning where GCC prints two. `Diagnostic` now carries a `dedup_salt` that
  the attribute verdict sets from the list's own span, so repeats inside one
  list still collapse while two lists report twice.
* `noreturn` on a function *type* is silent, as in GCC:
  `extern int (*fp)(void) __attribute__((noreturn));` and
  `typedef void (*handler)(int) __attribute__((noreturn));` produced a false
  `'noreturn' attribute ignored` because the object-target rule treated the
  declarator as a plain object. The verdict is skipped whenever the
  declarator's derived part names a function or a function pointer.
* `cleanup` is checked, not merely parsed. GCC's matrix (measured, all
  anchored at the declaration): zero or two arguments →
  `error: wrong number of arguments specified for 'cleanup' attribute` plus
  `note: expected 1, found N`; a non-identifier argument →
  `error: cleanup argument not an identifier`; a well-formed attribute on an
  automatic object → silent; on a function, or on an object with static
  storage duration → only `'cleanup' attribute ignored`, with the shape
  checks suppressed. The old parser took one identifier and left the rest of
  the list in the token stream, so `cleanup()` produced cascading
  `expected ';'` errors and `cleanup(1)` was silently accepted.
* `-Wstrict-prototypes` is derived from the finished declarator tree instead
  of transient parser state, which fixes the two shapes GCC reports and LCCC
  did not: an unprototyped type nested in a prototype
  (`void g(int (*h)());`) and a K&R definition. Locations now match GCC
  column for column: a declaration reports at its first token (a leading
  attribute list included), a definition at its name (`int k() {}` → the
  `k`). Pinned by the option-effect ledger, which compares the verdict
  locations against GCC on a three-shape fixture.
* The C23 attribute specifier runs the same placement check as
  `__attribute__`: `int b [[gnu::noreturn]];` and `int c [[gnu::packed]];`
  report `'noreturn' attribute ignored` / `'packed' attribute ignored`, and
  the standard spelling keeps its own measured sentence,
  `standard 'noreturn' attribute can only be applied to functions`. The
  legal placements stay silent, including `[[gnu::packed]] struct S { ... };`.
* Coverage: `check_attribute_diagnostics.py` grew to 49 probes (two
  `noreturn`-on-a-function-type silences, three `cleanup` verdicts, one
  silent well-formed `cleanup`), and `DiagnosticEngine::note` makes the
  `expected 1, found N` note a first-class emission.

Remaining measured deltas, kept explicit: `cleanup` on a block-scope object
with static storage duration, or on a block-scope function declaration, takes
`'cleanup' attribute ignored` in GCC while LCCC is silent (the ignored-target
rule covers file-scope objects only), and neither `cleanup argument not a
function` nor the prototype-arity/pointer-compatibility checks are emitted —
they need the symbol table; when an attribute sits on a later
line than the declaration, GCC's caret lands on that line's first token while
LCCC reports the declaration start (the same rule decides
`'x' attribute directive ignored`, which still points at the name token);
a macro
expansion adds a `note: in expansion of macro` that GCC omits; and
`int a[1 / 0];` prints its warning after the semantic error in LCCC
(GCC: warning first), with the counted driver summary as a third line that
GCC does not print at all (the summary is LCCC's own contract, pinned by
`tools/corpus`).

The `ms_abi`, `retain`, and `interrupt` findings are correctness/ABI hazards,
not performance opportunities. Independent `typeof` red-teaming also found
wrong `__builtin_types_compatible_p` constants and eliminated volatile reads;
see `engineering/evidence/typeof-qualifier-gaps-2026-10-28/`. Those remain
open despite the bounded const-propagation fix. The raw GP audit is in
`~/.cache/lccc/gp-sentinel-audit.{py,json,txt}`;
the checked-in attribute, warning, retain and workload regression gates provide
repeatable coverage for the corresponding narrower claims. Older measurements
elsewhere in the register are historical, not freshly re-measured verdicts.


### 12.1 Ranked register

| Rank | ID | Item | Class | Effort | ROI |
|---:|---|---|---|---|---:|
| 1 | GP-22 | Option-effect ledger: accepted ⇒ proven effective | P | M | 4.5 |
| 2 | GP-01 | Attribute registry + real `-Wattributes` | D/W | S | 4.6 |
| 3 | GP-05 | Macro-environment hygiene (`__GNUC__`, libc predefines, missing macros) | W | S–M | 4.4 |
| 4 | GP-02 | ABI attributes (`ms_abi` family) + cross-compiler ABI gate | W | L | 4.2 |
| 5 | GP-23 | Macro-parity gate (`-dM` diff budget) | P | S | 4.0 |
| 6 | GP-09 | Warning registry and group expansion | D | M | 4.0 |
| 7 | GP-17 | Inline-asm constraint grammar and validation | W | M | 3.8 |
| 8 | GP-18 | Builtin coverage + honest `__has_builtin` | F/W | L | 3.9 |
| 9 | GP-03 | Object/section attributes (`retain`/`noinit`/`persistent`/`nocommon`/`no_icf`/`noplt`) | W | S–M | 3.8 |
| 10 | GP-20 | Debug info DIEs, DWARF v5 lines, `-gdwarf-N`/`-gsplit-dwarf`/`-gz` | T | L | 3.8 |
| 11 | GP-12 | Missing warning families (shadow/conversion/overflow/switch/vla/…) | D | L | 3.8 |
| 12 | GP-04 | Codegen-contract attributes (`interrupt`, `nocf_check`, `musttail`, `assume`, …) | W/D | M–L | 3.6 |
| 13 | GP-11 | `-Wuninitialized` / `-Wmaybe-uninitialized` | D | L | 3.6 |
| 14 | GP-13 | Real gcov (`.gcno`/`.gcda` for `-ftest-coverage`/`-fprofile-arcs`) | S→T | M | 3.6 |
| 15 | GP-19 | ISA option surface (retpoline, `-mstackrealign`, `-mfsgsbase`, AVX10 policy) | F | M–L | 3.5 |
| 16 | GP-10 | `-pedantic` | D | M | 3.4 |
| 17 | GP-07 | Digraphs and UCN identifiers | F | S | 3.4 |
| 18 | GP-24 | Real-world adoption matrix (gzip/zlib-ng/expat/sqlite + kernel-style TUs) | P | M | 3.7 |
| 19 | GP-06 | `_BitInt(N)` end-to-end (extends MB-11) | F | L | 3.2 |
| 20 | GP-21 | Driver query parity (`-dumpfullversion`, `-print-multi*`, `-print-sysroot`) | F | S | 3.2 |
| 21 | GP-14 | PGO interop and honesty | T | M | 3.0 |
| 22 | GP-08 | C23 dialect strictness sweep (`()` prototype, `__fp16`, over-acceptances) | F | S | 3.0 |
| 23 | GP-16 | LTO (extends MB-15) | T | L | 3.0 |
| 24 | GP-15 | Sanitizer runtimes (extends MB-07) | T | L | 2.8 |

### 12.2 Items

#### GP-22 · P0 · P — option-effect ledger

**Why.** 157 of 1 389 GCC options are accepted by lccc with no diagnostic, and at
least three of those are provably inert: `-ftest-coverage`/`-fprofile-arcs` (no
`.gcno`/`.gcda`, no `__gcov_*`), `-Wattributes` (never fires), and `-Ofast`/`-Og`
beyond the tolerated tier. Pillar 1 forbids exactly this, and today only the
`-flto`/`-fopenmp`/`-fsanitize` neighbourhood is covered by
`tests/regression/check_driver_option_contract.sh`.
**Where.** `src/driver/option_table.rs`, `src/driver/cli.rs`,
`tests/regression/check_driver_option_contract.sh`, `scripts/ci_local.sh`.
**Plan.**
1. Extend the option contract table to a *ledger*: every accepted spelling carries
   a probe id and an expected artifact/diagnostic delta.
2. Generate the probe as a data-driven test (`option → source fixture → observable
   (section list, symbol, diagnostic line, asm hash)`), one row per option.
3. Fail closed: a newly accepted option with no ledger row fails the gate;
   `LCCC_STRICT_OPTIONS=1` makes the tolerated tier an error.
**Gates.** The ledger gate itself, plus a negative control (delete a row from an
implemented option, watch the gate fail). **Accept:** zero accepted-and-inert
options for the priority set (`-fprofile-arcs`, `-ftest-coverage`, `-Wattributes`,
`-Ofast`, `-Og`, `-static-pie`, `-pedantic`); the gate runs in the fast lane.
**Effort** M · **ROI** 4.5.

#### GP-01 · P0 · D/W — attribute registry and a real `-Wattributes`

**Why.** 62 of GCC 16.2's 142 C attributes are parsed and discarded in silence, and
`-Wattributes` (accepted since forever) never fires — not for an unknown name
(`__attribute__((frobnicate))`), not for a misapplied one (`packed` on an `int`),
not for a nonsensical one (`noreturn` on an object). GCC warns in all three cases.
This is the entry point for every other attribute item: until it exists, an
unimplemented attribute is invisible.
**Where.** `src/frontend/parser/parse.rs` (`dispatch_gcc_attribute`),
`src/frontend/sema/analysis.rs`, `src/common/error.rs` (`WarningKind::Attributes`).
**Plan.**
1. Turn the dispatcher into a table `name → { applicability, handler, implemented }`.
2. Emit `'<name>' attribute directive ignored [-Wattributes]` for unknown names and
   `'<name>' attribute ignored [-Wattributes]` for known-but-inapplicable ones
   (GCC's two phrasings, caretted at the attribute name).
3. Route every not-yet-implemented semantic attribute through a single
   `unimplemented-attribute` site so the audit list is generated, not maintained by
   hand.
**Gates.** the new regression gate `check_attribute_diagnostics` (added under `tests/regression/`): gcc-16 verbatim
expectations for unknown / misapplied / accepted forms, plus the generated
"known but unimplemented" inventory pinned by name (same ratchet pattern as the
`verify_after_pass` inventory in `check_verify_ir_clean.sh`).
**Accept:** lccc and gcc-16 agree line-for-line on the probe set; the inventory of
still-unimplemented attributes is a pinned list that only shrinks.
**Effort** S · **ROI** 4.6.

#### GP-02 · P0 · W — ABI-selecting attributes

**Why.** `ms_abi` is silently ignored; the measured consequence is cross-object
garbage: GCC caller + lccc callee returns `1216891234` instead of `47`, and the
reverse direction segfaults. `sysv_abi`, `stdcall`, `thiscall`, `cdecl`,
`sseregparm`, `force_align_arg_pointer`, `callee_pop_aggregate_return`,
`preserve_none`, `no_caller_saved_registers`, `no_callee_saved_registers`,
`ms_struct`, `gcc_struct` are in the same silent state.
**Where.** `src/frontend/parser/parse.rs`, `src/backend/call_abi.rs`,
`src/backend/x86/`, `src/backend/i686/`, `src/frontend/parser/ast.rs`
(`FunctionAttributes`).
**Plan.**
1. Add the calling-convention field to the function type/attribute set and thread it
   through call lowering and definition emission (x86-64 first: argument registers
   `rcx/rdx/r8/r9` for integer, `xmm0..xmm3` for float, 32-byte shadow space in the
   caller, `rax` return).
2. `ms_struct`/`gcc_struct` layout flag in `src/common/types.rs` (bit-field packing and
   member alignment) with layout diff tests against GCC.
3. For anything not implemented yet: hard error naming the attribute (pillar 1), never
   silence.
**Gates.** The decisive one is a **cross-compiler ABI matrix**: for each attribute and
each signature class (int/float/mixed/aggregate/variadic), build callee with GCC and
caller with lccc and vice versa, execute, and compare to GCC/GCC. Plus `readelf -s`
layout checks for `ms_struct`.
**Accept:** the matrix is green for the implemented set; everything else is refused
with a named error and covered by GP-01's inventory.
**Effort** L · **ROI** 4.2.

#### GP-03 · P1 · W — object-placement attributes

**Why.** `retain`, `noinit`, `persistent`, `nocommon`, `no_icf`,
`nodirect_extern_access`, `noplt` are accepted and ignored. Measured: GCC places
these objects in `.data.retain` / `.persistent`; lccc puts them in plain `.data`.
`retain` is a *security* contract (the object must survive section garbage
collection), so silence is a correctness bug for binaries built with `--gc-sections`.
**Where.** `src/frontend/parser/ast.rs` (`DeclAttributes`), `src/backend/generation.rs`,
`src/backend/elf_writer_common.rs`, `src/backend/linker_common/`.
**Plan.** Model the flag per object; emit `SHF_GNU_RETAIN` and the `.noinit`/
`.persistent` section names exactly as GCC does; diagnose misuse (GCC already warns
for `persistent` on an uninitialized variable).
**Gates.** `readelf -S`/`readelf -s` table vs GCC for each attribute, plus a
`--gc-sections` link in which a `retain`ed unreferenced object must survive.
**Accept:** section name, flag and survival match GCC for all seven.
**Effort** S–M · **ROI** 3.8.

#### GP-04 · P1 · W/D — codegen-contract attributes

**Why.** `interrupt`, `nocf_check`, `cf_check`, `no_caller_saved_registers`,
`no_callee_saved_registers`, `assume`/`assume_aligned`, `musttail`,
`indirect_return`, `no_reorder`, `noipa`, `noclone`, `flatten`, `leaf`, `nothrow`,
`strict_flex_array`, `warn_if_not_aligned`, `designated_init`, `flag_enum`,
`unavailable`, `warn_unused`, `hardbool`, `signed_bool_precision`, `tainted_args`,
`target_version`, `ms_hook_prologue`, `strub` are silent. The first six change
guarantees a caller is entitled to rely on; `interrupt` changes the prologue of a
function GCC refuses to treat as ordinary.
**Where.** `src/frontend/parser/parse.rs`, `src/backend/cfi_synth.rs`,
`src/backend/x86/`, `src/frontend/sema/analysis.rs`, `src/passes/`.
**Plan.** Split by risk: (a) prologue/epilogue contracts (`interrupt`,
`ms_hook_prologue`, `nocf_check`/`cf_check` with `-fcf-protection`, `strub`) —
implement or refuse; (b) register-contract attributes — implement or refuse;
(c) optimisation hints (`flatten`, `leaf`, `nothrow`, `noipa`, `noclone`,
`no_reorder`, `assume`) — implementing as *no-ops* is acceptable only if GP-01
reports them as ignored, which is GCC's own behaviour for several of them;
(d) diagnostic/metadata attributes (`unavailable`, `warn_unused`, `designated_init`,
`flag_enum`, `warn_if_not_aligned`, `strict_flex_array`) — small, self-contained
diagnostics.
**Gates.** Per-attribute positive/negative diagnostics (gcc-16 verbatim) and asm
checks for the prologue contracts; `-fcf-protection` `endbr64` presence/absence for
`nocf_check`/`cf_check`.
**Accept:** no attribute in the list is both accepted and unobserved by a gate.
**Effort** M–L · **ROI** 3.6.

#### GP-05 · P0 · W — macro-environment hygiene

**Why.** Three defects in one place: (1) 211 libc macros (`BUFSIZ`, `EOF`, `NULL`,
`CHAR_BIT`, `INT8_C(x)`, errno values, …) are predefined, so `#ifdef` feature tests
take the wrong branch (reproduced with `#ifdef EOF`); (2) `__STDC_NO_ATOMICS__` is
predefined although `_Atomic` works — an autoconf capability denial; (3) 155 GCC
macros are missing (`__BIGGEST_ALIGNMENT__`, `__BITINT_MAXWIDTH__`, the
`__FLT32/64/128*` and `__DEC*` families, `__CHAR8_TYPE__`, `__FINITE_MATH_ONLY__`,
`__FLOAT_WORD_ORDER__`, `_STDC_PREDEF_H`, `__ATOMIC_HLE_*`), and `__GNUC__`/`-dumpversion`
say 14 while the compiler targets 16.2 parity.
**Where.** `src/frontend/preprocessor/builtin_macros.rs` (the `def(macros, …)` block,
lines 22–216), `src/driver/cli.rs` (`-dumpversion`, `--version`).
**Plan.**
1. Delete the libc-macro predefines; the real headers must provide them (they do —
   lccc uses the system glibc headers). Where a header is missing, ship it in
   `include/` instead of predefining its macros.
2. Drop `__STDC_NO_ATOMICS__`; audit the other `__STDC_NO_*` claims against what the
   compiler actually implements.
3. Add the missing GCC macros with byte-identical spellings (hex vs decimal matters
   for `-dM` diffs; `((double)…)` casts matter for `__builtin_types_compatible_p`).
4. Decide the `__GNUC__` policy explicitly: claim 16 with a documented risk note, or
   keep 14 and provide `-fgnuc-version=`; either way `-dumpversion` must agree.
**Gates.** GP-23 (the `-dM` diff gate) plus a feature-detection test set
(`#ifdef BUFSIZ`, `#ifdef EOF`, `#ifdef __STDC_NO_ATOMICS__` with `_Atomic` in use).
**Accept:** zero lccc-only macros on an empty TU; the missing-macro list is empty for
the essential set; `__GNUC__`/`__VERSION__`/`-dumpversion` are consistent and
documented.
**Effort** S–M · **ROI** 4.4.

#### GP-06 · P2 · F — `_BitInt(N)` end-to-end

**Why.** The last C23 hole (MB-11 keeps the row visible): `unsigned _BitInt(7)`,
`_BitInt(17)`, `_BitInt(200)` and `1wb`-suffixed literals are rejected while GCC
16.2 accepts them. Not a syntax patch: the fixed-width `CType` model cannot carry
precision/signedness through conversions, constant evaluation, layout, ABI
classification, codegen and debug info.
**Where.** `src/common/types.rs` (new `CType::BitInt{bits, signed}`),
`src/frontend/sema/` (conversions, promotions), `src/common/type_builder`,
`src/backend/call_abi.rs`, `src/ir/`.
**Plan.** (1) Type + spelling + `_Generic`/`typeof` identity; (2) conversions and
usual arithmetic conversions with the exact C23 rules; (3) constant evaluation and
layout (non-power-of-two widths, alignment rules); (4) ABI classification
(≤64 bits in an integer register, >64 in memory/aggregate per the SysV rules GCC
uses); (5) codegen for widths ≤64 via the existing integer paths, wider values via
multi-word sequences; (6) debug metadata.
**Gates.** Compiler-oracle differential (gcc-16 accept/reject + runtime values) per
width class, ABI cross-link tests against GCC, and a layout table test.
**Accept:** differential green up to `_BitInt(4096)`; no regression in the existing
C23 census.
**Effort** L · **ROI** 3.2.

#### GP-07 · P1 · F — digraphs and universal character names

**Why.** Two standard-C conformance holes that GCC has had for decades: digraph
punctuators `%:` `<:` `:>` `<%` `%>` (C95) are rejected, and UCNs in identifiers
(`int caf\u00e9;`, C99) are rejected while UCNs in string literals work.
**Where.** `src/frontend/lexer/` (token spelling), `src/frontend/parser/`.
**Plan.** Fold the six digraphs onto their primary tokens in the lexer (before
parsing, so `%:include` and `a<:2:>` work), and accept `\uXXXX`/`\UXXXXXXXX` in
identifiers by mapping to the identifier's identifier-charset form. Both are
`-std=` independent, as in GCC.
**Gates.** the new regression gate `check_digraph_ucn` (added under `tests/regression/`) with gcc-16 verdicts for every
digraph position (`#`, `{`, `}`, `[`, `]`) and UCN identifiers, plus `-pedantic`
behaviour once GP-10 exists.
**Accept:** the probe set compiles and executes identically to GCC.
**Effort** S · **ROI** 3.4.

#### GP-08 · P3 · F — C23 dialect strictness sweep

**Why.** lccc is *more* permissive than GCC in at least two measured places: C23
`int f();` stays an unspecified-prototype declaration (GCC rejects redeclaring it
with parameters), and `__fp16` is accepted on x86-64 (a GCC AArch64 type). Over-
acceptance is a compatibility risk of its own: code that only builds with lccc is
code that cannot be merged upstream.
**Where.** `src/frontend/sema/analysis.rs` (prototype handling),
`src/common/types.rs` (`__fp16`).
**Plan.** Run the C23 inventory as a *bidirectional* differential (accept *and*
reject, per dialect) and fix every "lccc accepts, gcc rejects" row; keep a pinned
allow-list for deliberate extensions.
**Gates.** Extend `docs/C23-COMPATIBILITY-INVENTORY.md`'s generator to fail on any
new over-acceptance row.
**Accept:** zero unexplained over-acceptances across gnu90…gnu2y.
**Effort** S · **ROI** 3.0.

#### GP-09 · P1 · D — warning registry and group expansion

**Why.** 18 of GCC's 549 positive `-W` options are accepted and 12 `WarningKind`s
exist; `-Wall`/`-Wextra`/`-Werror=` work only for that handful. (MB-12 covers the
first tranche; this item is the structural half.)
**Where.** `src/common/error.rs`, `src/driver/cli.rs`,
`tests/regression/check_warning_compat.py`.
**Plan.** One table `(name, default_on, in_wall, in_wextra, in_pedantic,
description)` generating `-W<name>`, `-Wno-<name>`, `-Werror=<name>`,
`-Wno-error=<name>`, the `[-Wname]` suffix and group expansion; unknown names follow
the MB-01 policy (diagnose, not silence); `-Wno-` prefix matching for GCC's
`-Wno-unknown-warning` idiom.
**Gates.** Per-warning positive/negative pairs, group-expansion matrix, and a
precision gate: zero lccc-only warnings over `tests/benchmark/programs` + the
workload sources (every lccc-only line is a bug).
**Accept:** the registry drives every emitted warning; `-Wall`/`-Wextra`/`-pedantic`
membership matches GCC for the implemented set.
**Effort** M · **ROI** 4.0.

#### GP-10 · P2 · D — `-pedantic`

**Why.** `-pedantic` is accepted and does nothing; GCC diagnoses GNU extensions in
strict `-std=cNN` modes (statement expressions, `typeof`, zero-length arrays,
`asm`, nested functions, …). Projects that gate on `-pedantic` cannot use lccc.
**Where.** `src/driver/cli.rs`, `src/frontend/sema/`, `src/frontend/parser/`.
**Plan.** Mark every extension site with a `pedantic` diagnostic kind that is silent
unless `-std=c*` (not `gnu*`) and `-pedantic`; take GCC's message per extension.
**Gates.** Extension battery × {`c17`,`gnu17`,`c23`,`gnu23`} × {`-pedantic`,
`-pedantic-errors`}, gcc-verbatim.
**Accept:** verdict and message parity; no diagnostic in `gnu*` mode.
**Effort** M · **ROI** 3.4.

#### GP-11 · P1 · D — flow warnings

**Why.** `-Wuninitialized` and `-Wmaybe-uninitialized` do not exist; measured: GCC
warns on `int x; if (x > 0) …` at `-O2 -Wall`, lccc is silent. These two are the
highest-value warnings in the catalogue and are in every distro flag set.
**Where.** `src/frontend/sema/` (definite-assignment lattice), `src/passes/`
(`mem2reg`, SSA availability), diagnostics in `src/common/error.rs`.
**Plan.** A forward must-def / may-def dataflow over the CFG at `-O1+` with the
"maybe" wording chosen by the same rule GCC uses; honour `(void)x`, unused
attributes, `sizeof` operands, and `-Wno-` plumbing from GP-09.
**Gates.** A curated positive/negative corpus (including the kernel's classic
false-positive shapes), plus a zero-false-positive requirement over the workloads.
**Accept:** parity on the curated corpus with no lccc-only warning on the workloads.
**Effort** L · **ROI** 3.6.

#### GP-12 · P2 · D — missing warning families

**Why.** Measured absent on ordinary code: `-Wshadow`, `-Wconversion`,
`-Wsign-conversion`, `-Woverflow` (constant conversion overflow), `-Wvla`,
`-Wswitch`/`-Wswitch-enum`, `-Wmissing-prototypes`, `-Wstrict-prototypes`,
`-Wreturn-local-addr`, `-Warray-bounds`, `-Wstringop-overflow`/`-truncation`,
`-Wcast-qual`, `-Wdeprecated-declarations`, and the `-Wformat=2` extras.
**Where.** `src/frontend/sema/` (most), `src/passes/` (bounds/stringop),
`src/common/error.rs`.
**Plan.** Land in cheapness order: (`-Woverflow`, `-Wvla`, `-Wswitch`,
`-Wstrict-prototypes`, `-Wmissing-prototypes`, `-Wreturn-local-addr`) then
(`-Wshadow`, `-Wconversion`, `-Wcast-qual`, `-Wdeprecated-declarations`), then the
analysis-backed pair (`-Warray-bounds`, `-Wstringop-*`).
**Gates.** Per-family positive/negative pairs against gcc-16; precision gate as in
GP-09.
**Accept:** each family matches GCC on its gate; no new lccc-only diagnostics on the
workloads.
**Effort** L · **ROI** 3.8.

#### GP-13 · P1 · S→T — real gcov

**Why.** `-ftest-coverage` and `-fprofile-arcs` are accepted but re-routed to the
compiler's own PGO path: each flag alone prints `lccc: PGO: instrumented N
functions … into ./lccc-*.profraw` and emits that profile, while no `.gcno`/
`.gcda` and no `__gcov_*` symbols appear. A project's coverage job therefore
reports 0 % (or fails) *and* gets a misleading indication that instrumentation
happened. Re-measured 2026-10-27 on `dc878b3`, `-ftest-coverage` alone and
`-fprofile-arcs` alone behave identically.
**Where.** `src/driver/`, `src/ir/`, `src/backend/` (counter insertion),
new `.gcno` writer; `src/pgo/` for the profile side.
**Plan.** First remove the current alias so the flags stop pretending to be PGO.
Then either (a) implement `-ftest-coverage`: note file emission in the gcov
format `gcov-16` reads, per-block counters, `.gcda` writing via a small runtime
shared with the existing PGO instrumentation; or (b) refuse both flags with a named
error and document the supported `-fprofile-generate` alternative. (a) is the parity
answer; (b) is the honest one if effort is unavailable. Never silence.
**Gates.** `gcov-16` parses the emitted `.gcno`, runs the program, reads `.gcda`, and
the reported line/branch percentages match GCC's for the same source.
**Accept:** `gcov` round-trips and matches GCC within one counter of the same
program, on x86-64 and i686.
**Effort** M · **ROI** 3.6.

#### GP-14 · P2 · T — PGO interop and honesty

**Why.** lccc's PGO works but uses an incompatible `.profraw` scheme, accepts
`-fauto-profile`/`-fprofile-use` silently when no profile exists, and does not
document the format. Build systems that ship GCC `.gcda` files get no optimisation
and no warning.
**Where.** `src/pgo/`, `src/driver/cli.rs`.
**Plan.** Version and document the profile format; warn (GCC's
`-Wmissing-profile` equivalent) when `-fprofile-use` finds nothing; either import
GCC `.gcda` or refuse it explicitly.
**Gates.** `-fprofile-use` without a profile warns; a mismatched-version profile
warns and is ignored; the documented format round-trips.
**Accept:** no silent "used a profile" claim without a profile.
**Effort** M · **ROI** 3.0.

#### GP-15 · P3 · T — sanitizer runtimes

**Why.** ASan/TSan/LSan/MSan are refused (honest) and UBSan only works in trap mode.
Kernel-address, `-fsanitize=pointer-*` and `-fsanitize-coverage` are refused as well.
Adoption in fuzzing/CI pipelines requires at least ASan+UBSan run modes.
**Where.** `src/driver/`, `src/ir/` (instrumentation), runtime cooperation with
`libasan`/`libubsan` from the host GCC or compiler-rt.
**Plan.** Stage 1: UBSan run mode by emitting libubsan-compatible handler calls
(reuse the trap-mode site list). Stage 2: ASan via the standard shadow-map protocol
and host `libasan`. Stage 3: `-fsanitize-coverage=trace-pc-guard` for fuzzing.
**Gates.** Each sanitizer: a positive (bug detected, correct report) and a negative
(clean program, zero reports), plus ABI interop with GCC-compiled objects.
**Accept:** UBSan and ASan usable in a real CI recipe; everything else refused by
name.
**Effort** L · **ROI** 2.8.

#### GP-16 · P2 · T — LTO

**Why.** `-flto` is refused (honest) and `-fwhole-program`/`-fno-lto` are accepted
with no effect. Distro default flags include `-flto=auto` in several recipes, so this
blocks "drop-in replacement" for those builds.
**Where.** driver + IR serialisation + link-time pass driver.
**Plan.** Either real LTO (serialise lccc IR, run the existing pass pipeline at link
time) or a documented "no LTO" mode in which `-flto`/`-flto=auto` are *refused* and
`-fno-lto`/`-ffat-lto-objects` are accepted as no-ops with a rationale (MB-15).
**Gates.** `-flto` round-trip: two TUs, cross-TU inlining must be observable in the
asm; refusal path must be gate-tested too.
**Accept:** whichever branch is delivered, no option is silently inert.
**Effort** L · **ROI** 3.0.

#### GP-17 · P0 · W — inline-asm constraint grammar

**Why.** lccc validates nothing: `"j"(i)`, `"V"(i)`, `"=f"(d)` as an *input* are all
accepted; GCC rejects all three. 15 of 65 probe cases diverge, every one in the
"lccc accepts what GCC rejects" direction. Hand-written SIMD and kernel code lives in
this space.
**Where.** `src/backend/inline_asm.rs`, `src/frontend/sema/` (operand binding).
**Plan.**
1. Per-target constraint table (letter → register class / immediate range / memory /
   symbol) for x86-64 and i686: `r,m,i,n,g,X,o,V,p,a,b,c,d,S,D,A,f,t,u,x,v,y,q,Q,R,l,
   I,J,K,L,M,N,O,e,Z,s,G,C,B,Yz` with GCC's target meanings.
2. Validate placement prefixes (`=`, `+`, `&`, `%`) and reject `=` on inputs.
3. `%`-modifier table (`%b,%h,%w,%k,%q,%c,%a,%A,%l,%P,%p,%z`) with GCC's arity rules.
4. Diagnostics: `impossible constraint in 'asm'`, `input operand constraint contains
   '='`, `'asm' operand N probably does not match constraints`,
   `-Wasm-operand-widths`.
**Gates.** The 65-case matrix as a gate with gcc-16 verdicts; plus a
"matches-GCC-when-it-accepts" runtime set (assembled and executed).
**Accept:** verdict parity on the matrix; zero accepted-impossible constraints.
**Effort** M · **ROI** 3.8.

#### GP-18 · P1 · F/W — builtin coverage and `__has_builtin` honesty

**Why.** 607 visible builtins vs GCC's 1 696 (2 171 with `-march=x86-64-v3`), and
129 of lccc's answers are *over-claims* — worse than missing, because portable code
dispatches on `__has_builtin`.
**Where.** `src/frontend/sema/gcc16_builtins.rs`, `src/frontend/sema/builtins.rs`,
`scripts/check_intrinsic_arity.py`, `include/`.
**Plan.**
1. Make `__has_builtin` answer from the lowering table (a builtin is "there" only if
   it lowers or is a declared intrinsic), never from the name list alone.
2. Add the ~550 `floatN` math builtins (`__builtin_acosf16/f32/f64/f128/f32x/f64x/l`
   and `…pi` forms) — they lower to the same libm calls with a type suffix.
3. Add the generic families: `__builtin_object_size`/`__builtin_dynamic_object_size`
   variants, `__builtin_*_overflow` gaps, `__builtin_stdc_*`, `__builtin_apply*`,
   `__builtin_classify_type`, `__builtin_alloca_with_align_and_max`.
4. x86 ISA naming follows `engineering/isa/gcc16-sse-avx-reference.json` (5 365
   names absent today).
**Gates.** The `__has_builtin` differential as a gate (bidirectional: no over-claims,
missing-set pinned and shrinking), plus lowering tests for each added family.
**Accept:** zero over-claims; missing set empty for the generic families; the ISA gap
count only shrinks.
**Effort** L · **ROI** 3.9.

#### GP-19 · P2 · F — ISA option surface

**Why.** 77 of 234 `-m` options are accepted; refusals include flags that hardening
and kernel builds require: `-mindirect-branch=`, `-mfunction-return=` (retpoline),
`-mstackrealign`, `-mfsgsbase`, `-msha`, `-mrdseed`, `-madx`, `-mclflushopt`,
`-mwaitpkg`, and `-mavx10.1` (no AVX10 policy exists).
**Where.** `src/driver/cli.rs`, `src/backend/x86/`, `src/backend/i686/`.
**Plan.** Land the retpoline pair first (thunk + `lfence` variants, x86-64 and
i686), then the instruction-enable flags that only gate existing encoders
(`-mfsgsbase`, `-msha`, `-madx`, `-mrdseed`, `-mclflushopt`, `-mwaitpkg`,
`-mstackrealign`), then decide AVX10 (refuse permanently with a rationale, or
implement).
**Gates.** Per-flag: asm shows the gated instruction appear/disappear; retpoline
gates check `call __x86_indirect_thunk_*` and `lfence` placement.
**Accept:** the mitigation flags work; the rest are either implemented or refused by
name (never tolerated).
**Effort** M–L · **ROI** 3.5.

#### GP-20 · P1 · T — debug information (extends MB-10)

**Why.** `-g` emits `.debug_line` only: DWARF **v4**, empty directory table, no
`.debug_info`/`.debug_abbrev`/`.debug_str`. gdb shows no types, no locals, no
parameters, no inlined frames; `addr2line` cannot resolve header lines.
`-gdwarf-N`, `-gsplit-dwarf` and `-gz` do not exist as options.
**Where.** `src/backend/cfi_lower.rs`, new DIE builder, `src/backend/elf_writer_common.rs`,
`src/driver/cli.rs`.
**Plan.** (1) CU DIE, `DW_TAG_subprogram`, `DW_TAG_formal_parameter`,
`DW_TAG_variable`, `DW_TAG_base_type`/`pointer_type`/`structure_type` (bounded
scope first), `DW_AT_location` from the existing live-range data. (2) Line tables
→ DWARF v5 with directory/file tables so header lines resolve. (3) `-gdwarf-N`
selection, `-gsplit-dwarf`, `-gz=zlib`.
**Gates.** gdb batch scripts (`ptype`, `info locals`, `info args`, `bt`, `list` on a
header line), `addr2line` on a known header call site, and a DIE-inventory diff
against GCC for a fixture bounded by scope (not an "all of DWARF" claim).
**Accept:** the gdb and addr2line gates are green on x86-64 and i686; the DIE
inventory covers the gates' fixtures exactly.
**Effort** L · **ROI** 3.8.

#### GP-21 · P3 · F — driver query parity

**Why.** `-dumpfullversion`, `-print-multiarch`, `-print-multi-lib`,
`-print-sysroot` are hard errors; `-print-prog-name=cc1` prints a bare `cc1`;
`-print-libgcc-file-name` resolves to an unrelated system `gcc-12` libgcc;
`-dumpversion` prints `14`.
**Where.** `src/driver/cli.rs`.
**Plan.** Implement the queries with GCC's exact output shapes (paths, empty
`-print-sysroot`), and make `-print-libgcc-file-name` either honest (the libgcc lccc
actually links) or refuse.
**Gates.** the new regression gate `check_driver_query_parity` (added under `tests/regression/`): shape comparison against
gcc-16 for all nine queries.
**Accept:** every query answers with the right shape and a truthful value.
**Effort** S · **ROI** 3.2.

#### GP-23 · P0 · P — macro-parity gate

**Why.** The macro environment drifted silently (211 wrong predefines, 155 missing,
`__GNUC__` 14) because nothing measured it. This is the gate that makes GP-05 a
ratchet.
**Where.** `tests/regression/`, `scripts/ci_local.sh`, `.github/workflows/ci.yml`.
**Plan.** For each `(target, dialect)` pair (x86-64/i686 × gnu90…gnu2y), diff
`lccc -dM -E` against `gcc-16 -dM -E`, with three pinned lists: *forbidden*
(defined by lccc only), *missing* (GCC-only, shrinking), *spelling* (value
differences, with an allow-list for numeric-synonym spellings).
**Gates.** It is the gate. Positive control: re-add `BUFSIZ` and watch it fail.
**Accept:** forbidden list empty; missing list empty for the essential set; spelling
allow-list justified row by row.
**Effort** S · **ROI** 4.0.

#### GP-24 · P1 · P — real-world adoption matrix

**Why.** The strongest parity statement is "these projects build and pass their
tests", and today that evidence lives in four CI workloads (gzip, zlib-ng, expat,
sqlite) plus the corpus. It does not cover kernel-style idioms (inline asm
constraints, `__attribute__` coverage, retpoline, `-mno-red-zone`,
`-fno-strict-aliasing`) where most of this register's findings live.
**Where.** `tests/workloads/`, `tests/regression/`, `.github/workflows/`.
**Plan.** Add a kernel-style TU corpus (extracted shapes, licence-checked) to the
existing workload harness, and make the harness compare **diagnostics** as well as
outputs against GCC (today it compares execution results).
**Gates.** The matrix runs in the hosted CI lanes it already owns.
**Accept:** every workload builds with zero lccc-only diagnostics and identical
runtime output; every lccc-only diagnostic is triaged into an item of this register.
**Effort** M · **ROI** 3.7.

### 12.3 Mapping to existing entries

| GP item | Extends / overlaps | Note |
|---|---|---|
| GP-01, GP-03, GP-04 | — (new) | attribute surface, not covered by MB-01's option contract |
| GP-02 | MB-01 (ABI flags) | attributes, not flags; adds the cross-compiler ABI gate |
| GP-05, GP-23 | — (new) | macro environment has no owner today |
| GP-06 | MB-11 (`_BitInt` row) | same item, now with a plan and gates |
| GP-09, GP-10, GP-11, GP-12 | MB-12 | MB-12 keeps the warning contract; GP-09 is its structural half |
| GP-13, GP-14 | — (new) | coverage/PGO formats |
| GP-15 | MB-07 | sanitizer run modes beyond trap mode |
| GP-16 | MB-15 | LTO |
| GP-17 | — (new) | inline-asm grammar |
| GP-18 | §9a `SIMD-PERM-1`, `SIMD-MANUAL-1` | adds the generic builtin families and `__has_builtin` honesty |
| GP-19 | §9a (`SIMD-PERM-1` i686 gaps) | adds the mitigation flags |
| GP-20 | MB-10 | DWARF DIEs, v5 lines, split/gz |
| GP-21, GP-22 | MB-01 | driver queries and the option-effect ledger |
| GP-24 | MB-42 (probe set as gate) | adds diagnostics comparison to workloads |

Nothing in §4–§11 loses priority status by appearing here; the register exists so
that "GCC 16.2 compatible" is a measurable statement rather than an adjective.
