# LCCC Engineering Backlog — "Maybach" edition

Rewritten 2026-11-20 against `dbdadc9` (`main`). Every status in this file was
re-checked on that tree: §2 lists the probes that were run, and an item whose
text says "measured" carries its own date. Closed work is deleted, not struck
through — `git log -- backlog.md` is the history, and per-session narratives live
in [`engineering/journal/`](engineering/journal/README.md).

Method doctrine: [`engineering/agent/RULES.md`](engineering/agent/RULES.md).
Do-not-retry grounds: [`engineering/DECISIONS.md`](engineering/DECISIONS.md).
Per-area knobs: [`engineering/subsystems/`](engineering/subsystems/README.md).
GCC 16.2 parity register: §12, based on
[`docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md`](docs/GCC16-PARITY-ASSESSMENT-2026-10-27.md).

**This file holds open work only, written so an implementation agent can start
without asking questions.** An item with no reproducer or no stated acceptance
test does not belong here. Re-run an item's reproducer before starting it; if it
no longer reproduces, delete the item in the same commit that proves that.

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
reproducer to close an item. These decision rules bind every item here: correctness and evidence integrity outrank performance;
no runtime-speed claim from static instruction counts or Callgrind `Ir` alone
(they are simulated counts — in this file `Ir` is used **only** as a
deterministic proxy for the compiler's *own* cost, MB-20, and every such
result must also be confirmed with paired wall time); never tune by workload
name; never weaken `-fwrapv`, alias, volatile or FP barriers to recover a
benchmark; a skipped oracle is missing evidence, not a pass.

---

## 2. Baseline snapshot (R, 2026-11-20, `dbdadc9`)

Every row below was produced today by running the `fastbuild` binaries built from
`dbdadc9` (x86-64 rows on the host; i686 rows with header-free sources because the
audit sandbox has no 32-bit libc headers). A row is either **works** (accepted *and*
the documented effect observed) or **open** (the item that owns it). Re-run the
probes before starting an item; the status of a row is only as old as its date.

| Probe | Result | Owner |
|---|---|---|
| `-fno-plt`, `-fvisibility=hidden`, `-ftrapv`, `-fsanitize-trap=undefined`, `-ftls-model=`, `-fzero-call-used-regs=`, `-ftrivial-auto-var-init=zero` | works on x86-64 (GOTPCREL calls, `.hidden`, `jo`/`ud2`, zeroing); `-fno-plt`, visibility, `-ftrapv` also works on i686 | MB-07 (residual) |
| `-fstack-clash-protection`, `-fstack-protector*` with `-mstack-protector-guard{,-reg,-offset,-symbol}` | works on x86-64 and i686 (probe loop `subq $4096` + `orl $0,(%rsp)`; `%gs:64`, `%fs:__stack_chk_guard`) | — |
| `target_clones("avx2","default")` | works: `f.avx2`, `f.default`, `f.resolver`, `IFUNC f` | — |
| `-z pack-relative-relocs` (PIE), `-z max-page-size=65536`, `--image-base=`, `--oformat=binary`, `--build-id`, `-Map`, `--version-script`, `--icf=all`, `-pie` on `-m32` | works | MB-02 / MB-14 |
| `-Wformat`, `-Wunused-variable`, `-Wsign-compare`, `-Wattributes` (unknown attribute) | works | MB-12 |
| `lccc a.c -o a` | `ET_EXEC`, no build-id (open) | MB-04 |
| `-static-pie`, `-flto`, `-fsanitize=address`, `-ftest-coverage` | hard error with an honest message (open) | MB-04, MB-15, GP-15, GP-13 |
| `--emit-relocs` | x86-64: warns "ignored" outside `-T` script links; i686: refused. `-m32 -r`: requires the `gcc_linker` feature (open) | MB-14 |
| `-Wl,--print-gc-sections` | accepted, **prints nothing** although a section was discarded (open) | MB-02 |
| `-Wshadow -Wconversion -Wswitch -Wvla`, `-Wmaybe-uninitialized` at `-O2 -Wall` | tolerated with a "nothing will act on it" warning / silent (open) | GP-11, GP-12 |
| `_BitInt(37)` under `-std=c23` | rejected (open) | GP-06 |
| `-mstackrealign`, `-mfsgsbase`, `-gsplit-dwarf`, `-gz`, `-g3` macro section | refused or warned as unsupported; `-g3` emits no `.debug_macro` (open) | GP-19, GP-20 |
| `__asm__("..." : "=Z"(x) : "q"(1))` | accepted, garbage operand emitted; GCC: `impossible constraint in 'asm'` (open) | GP-17 |
| mutation fuzz (`tests/fuzz/mutate_frontend.py`): the 1 500-mutant sequence that found the old `#define` hang, plus 2 500 fresh mutants (`--seed 424242`), `-O1` / `-O2 -mavx2` | 0 internal errors, 0 signals, 0 hangs | AU-02 |
| run-to-run determinism, 12 benchmark programs × 4 configs × 4 runs (35 compilable pairs) | assembly and object bytes identical | AU-06 |

---

## 3. Roadmap and ranking

ROI = (impact × confidence) ÷ effort, each 1–5; higher is better.

| Rank | ID | Title | Pillar | Effort | ROI |
|---:|---|---|---|---|---:|
| 1 | MB-20 | Peephole compile time: decode once (76.9 % → ≤ 35 % of instructions) | 5 | M | 4.5 |
| 2 | MB-02 | Linker option contract (generated inventory, silent `--print-gc-sections`) | 1 | M | 4.5 |
| 3 | MB-04 | PIE by default, real `-static-pie`, default build-id | 2 | M | 4.2 |
| 4 | AU-02 | Nightly lane for the frontend mutation fuzzer; add `cargo-fuzz` targets | 1 | S–M | 4.0 |
| 5 | MB-10 | DWARF: line tables in the native assemblers, then DIEs | 3 | L | 3.8 |
| 6 | AU-03 | Miri lane and a coverage report | 1 | M | 3.6 |
| 7 | MB-01 | Decide the 766 tolerated options (grow the generated inventory) | 1 | M | 3.5 |
| 8 | MB-12 | Warnings that real builds rely on (flow, shadow, conversion) | 4 | M–L | 3.4 |
| 9 | MB-11 | C23 conformance inventory and half-format edge cases | 4 | S–M | 3.3 |
| 10 | MB-14 | Linker residue: `--emit-relocs`, ELF32 `-r`, common-page-size | 2 | M | 3.2 |
| 11 | AU-07 | One linker emit skeleton instead of two 4 k-line copies | 5 | M | 3.1 |
| 12 | MB-32 | i686 differential gate (expand the 14 programs) | 3 | S | 3.0 |
| 13 | MB-15 | Whole-program mode (`-fwhole-program` / honest `-flto`) | 6 | L | 3.0 |
| 14 | AU-09 | Parked optimisations: root-cause or delete | 5 | M | 3.0 |
| 15 | MB-07 | `-fno-strict-overflow` pointer half; UBSan handler mode | 2 | M | 2.9 |
| 16 | MB-33 | Linker differential oracle (bfd/lld/mold/lccc-ld) | 1 | M | 2.8 |
| 17 | AU-04 | Narrow the crate-wide `allow(dead_code, …)` | 5 | M | 2.7 |
| 18 | MB-30 | Raptor Lake cost-model calibration from published tables | 6 | L | 2.5 |

The GCC-16.2 conformance half of the programme is ranked separately in §12
(`GP-01`…`GP-24`). GP-17 gained an R reproducer on 2026-11-20 (invalid output
constraint accepted) and should be read before the rest of that register.
Audit items `AU-xx` are in §14 (whole-tree audit, 2026-11-20); AU-05, AU-06,
AU-08, AU-10, AU-11 and AU-12 are P2–P3 and unranked here.

Suggested start: MB-20 (the largest measured compile-time lever) → MB-02 → MB-04.
For two agents in parallel: [MB-02, MB-04, MB-01] on the driver/linker side and
[MB-20, AU-09] on the backend side.

**2026-11-18 performance worklist:** see the [evidence-ranked ten-item queue](engineering/BACKLOG-ROI-2026-11-18.md).
The Nov 18 VM screens favor bounded match extension and Huffman heap codegen over
reusing the historical nbody/LZ4 order. For compiler throughput, re-profile after
the next MB-20 PR before choosing between decode-once work and a fresh-solver contract
for liveness memoization. Scope-review metric-pair sharing and screen global
allocators on byte identity and paired wall time. Passing a compile-cost gate is
not a runtime win.

---

## 4. Tier M0 — Trust: promises must match artifacts

### MB-01 · P1 · R — decide the 766 tolerated options

**Why.** The driver's tiered policy (header of `src/driver/cli.rs`) and
`docs/OPTIONS.md` — generated by `scripts/lccc_options_inventory.py`, whose 37 rows are
re-measured against the binary (accepted / warned / refused, plus the observable
effect) on every `--check` — cover the options with a decided class. The rest is
undecided. R, measured on `4d28473` by probing GCC 13.3's option names bare with
`lccc <opt> -fsyntax-only`: of 1 140 names, **181 are accepted silently, 766 are
accepted with a "tolerated but not implemented" warning, and 156 are refused**
(names that need a value are over-counted as refused). Each of the 766 is either a
presentation or diagnostic option (keep tolerated) or changes the artifact and must
be implemented or refused: today `-fstrict-aliasing`, `-fstack-reuse=none`,
`-fvtable-verify=std`, `-fzero-init-padding-bits=all` and `-fcheck-pointer-bounds`
all produce a normal exit status and an artifact that ignores them. The 181 silent
acceptances were spot-checked (`-fcf-protection`, `-fpack-struct`, `-funsigned-char`,
`-fshort-wchar`, `-pg`, `-fno-omit-frame-pointer`, `-fno-asynchronous-unwind-tables`,
`-ffast-math`, `-fpatchable-function-entry`): all have their effect and are rows now.

**Where.** `src/driver/cli.rs` (tier arms), `scripts/lccc_options_inventory.py`
(`ROWS`, `--discover`), `docs/OPTIONS.md` (generated).

**Reproducer.**
```bash
python3 scripts/lccc_options_inventory.py --discover --lccc target/fastbuild/lccc
# 1140 GCC option names; not yet rows: {'accepted': 181, 'warned': 766, 'refused': 156}
lccc -fstrict-aliasing -c x.c      # rc 0, "unrecognized command-line option ... is tolerated but not implemented"
```

**Plan.**
1. Teach `--discover` to read each option's value form from `gcc -Q --help=...`, so
   options that need a value stop inflating the "refused" count, and make it print the
   three lists to a file for triage.
2. Triage the 766 by what the option does to the artifact: presentation / diagnostic
   (stay tolerated, no row needed), optimisation hints with no semantic effect (stay
   tolerated, one summary line in the document), and artifact-changing (ABI, layout,
   security, output format) — implement it with an effect row, or move it to the
   refused tier with a message. `-fstrict-aliasing` and the `-fsanitize-*` /
   `-fvtable-verify` family go first.
3. Add the decided rows to `ROWS`; the checker enforces class and effect, and
   `--write` regenerates the document.
4. Ratchet the tolerated count: `--discover --json` pins `warned` and fails when it
   grows.

**Gates.** `python3 scripts/lccc_options_inventory.py --self-test` and `--check`
(mutation-verified: a wrapper that turns `-fno-plt` and `-fstack-clash-protection`
into silent no-ops fails three rows). **Accept.** No artifact-changing option is in
the tolerated tier; the document lists every class decision. **Effort** M · **ROI** 3.5.

### MB-02 · P0 · R — linker option contract and no silent no-ops

**Current state (R, 2026-11-20).** On x86-64 RELR (`-z pack-relative-relocs`,
`--pack-dyn-relocs=relr`), `--image-base`, 64-KiB `-z max-page-size`,
`--oformat=binary` (also through the driver's `-Wl,` path, byte-identical to
`objcopy -O binary`), and `-z execstack` / `-z noexecstack` (`PT_GNU_STACK` on
both the executable and shared-library paths) take effect and are pinned by
`tests/linker/run_linker_tests.py`. `--fatal-warnings` promotes
unimplemented-option warnings in either argument order (`--warn-common`,
`-z common-page-size`). The strict linker suite reported 313 pass / 0 fail on
2026-10-27. What is missing is the whole linker option surface: there is no
generated BFD/LLD/mold option union and no exhaustive
`Implemented | BenignNoop | Refused | WarnUnimplemented` inventory, and some
accepted options do nothing visible (R: `--print-gc-sections` prints nothing
although a section was discarded; `--O1`, `--sort-common` need evidence or a
justified `BenignNoop`).

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

### MB-04 · P0 · R — PIE by default, a real `-static-pie`, default build-id

**Why.** `lccc a.c -o a` yields `ET_EXEC` (R) where every mainstream GCC
configuration yields `ET_DYN`: no ASLR for any binary built by lccc. `-static-pie` is a hard error (R, 2026-11-20: "-static-pie is not
supported by the LCCC driver") — honest, but a hardening flag every distribution
toolchain accepts. No `.note.gnu.build-id` by default (GCC distributions pass `--build-id`);
without it, debuginfod, `perf`, coredumps and `eu-unstrip` cannot match binaries.
`-pie` itself works on x86-64 and on `-m32` (`ET_DYN`); the default is still `ET_EXEC` on both.

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
   expects, and define `_DYNAMIC`/`_GLOBAL_OFFSET_TABLE_`. Until this lands `-static-pie`
   stays a hard error, never silently non-PIE.
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

### MB-07 · P2 · R — `-fno-strict-overflow` pointer half and UBSan handler mode

`-fvisibility`, `-fno-plt`, `-ftrapv`, trap-mode UBSan, `-ftls-model`,
`-fzero-call-used-regs` and `-ftrivial-auto-var-init` are implemented (§2). Two
parts remain.

**MB-07a `-fno-strict-overflow` pointer half.** The integer half is gated by
`tests/regression/check_iabs_cmp_fold.sh` (the former `accepted-wrap-product.c`
reproducer passes at `-O0`…`-O3`). GCC documents `-fno-strict-overflow` as implying
`-fwrapv` **and** disabling pointer-overflow UB; no pointer-overflow assumption is
established or audited here, which is why the flag still warns once per translation
unit and why `LCCC_STRICT_OPTIONS=1` refuses it. Closing this means auditing every
pointer comparison and pointer-plus-offset fold against the flag, then dropping the
warning. `-fstrict-overflow` is not part of the item (see
[`docs/builtin-libcall-policy.md`](docs/builtin-libcall-policy.md)).

**MB-07b UBSan stage 2.** Trap mode works (`-fsanitize-trap=undefined`).
`-fsanitize=undefined` without trap mode is refused ("requires trap mode"); stage 2
emits `__ubsan_handle_*` calls with source-location records and links `libubsan`
when present. `-fsanitize=address|thread|memory|leak` stay refused (GP-15): do not
fake runtimes.

**Reproducer.**
```bash
echo 'int a(int x,int y){return x+y;}' > t.c
lccc -O2 -fsanitize=undefined -c t.c; echo $?      # 1: requires trap mode
lccc -O2 -fno-strict-overflow -c t.c               # warns once per TU
```

**Gates.** Seeded-bug programs per check with the handler mode (rc and message);
no false report over the 39 benchmark workloads. **Accept.** Pointer-overflow audit
documented per fold, warning removed. **Effort** M · **ROI** 2.9.

---

## 5. Tier M1 — Completeness and debuggability

### MB-10 · P1 · R — DWARF: DIE coverage, locations and the remaining line-program features

**Current state.** `-g` output carries line tables with relocations on x86-64 and
i686 (gated by `tests/regression/check_dwarf_line_tables.py`) and a partial DIE tree
(§13 lists what is measured and open; `tests/regression/check_dwarf_quality.py`
decodes it and reports unimplemented DIE kinds as named gaps). The `-O0` internal
definition rule is GCC 16.2's: an unreferenced `static` function or object is
emitted at `-O0` and dropped at `-O1`+. `gdb`, pahole and objtool are unavailable
in the audit harness, so no locals/arguments/CFI/BTF inspection is claimed. The
campaign plan with file anchors and gate assertions is
[`docs/DWARF-CAMPAIGN.md`](docs/DWARF-CAMPAIGN.md).

**Why.** Locals, lexical blocks, location lists for optimized code and inlined
subroutines are still missing, so gdb cannot show most variables and `perf annotate`
cannot attribute inlined code. GCC and Clang provide all of it.

**Reproducer.**
```bash
lccc -g -O0 -c p.c -o p.o && readelf -S p.o | grep -c debug   # sections exist
python3 tests/regression/check_dwarf_quality.py --lccc target/fastbuild/lccc   # lists the named DIE gaps
```

**Where.** x86 and i686 assemblers (`src/backend/x86/assembler/`, i686 equivalent),
`src/backend/x86/assembler/elf_writer.rs` (section + relocation emission),
`src/driver/pipeline.rs` (`asm_contains_debug_records` honesty check),
`src/backend/cfi_synth.rs`; DIE generation in a new backend-independent module
fed by `CType`.

**Plan (each stage is shippable alone).**
1. **Line-program remainder.** Keep the
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
**Accept.** The line-table stage is independently gated; the DIE stage
must land with a stable gdb test. Stage 3 has its own acceptance on the benchmark
corpus (≥ 90 % of parameters inspectable at `-O1`). **Effort** L · **ROI** 3.8.

### MB-11 · P2 · R — C23 conformance inventory and half-format edge cases

**Current state.** `_Float16`/`__bf16` (literals, conversions, ABI lowering) are
implemented with targeted GCC 16 regressions, but there is no exhaustive
2^16-pattern × rounding-mode proof. The default dialect is `gnu23`: `f()` rejects
extra arguments there, GNU17 keeps unspecified parameters, and `(void)` is a
zero-parameter prototype in both. `_BitInt(N)` is rejected (R, 2026-11-20:
`_BitInt(37)` under `-std=c23`); it is an explicit, deferred feature gap because
the fixed-width `CType` model cannot carry precision and signedness through
conversions, constants, layout, ABI lowering, codegen and debug metadata. Keep the
row visible and publish no C23 coverage percentage. `tests/regression/check_c23_inventory.py`
(43 probes in `c17`, `gnu17`, `c23`, `gnu23`) finds no other difference from
GCC 16.2; method, classifications and the four `-std=c89` deviations (item 3
below) are in `docs/C23-COMPATIBILITY-INVENTORY.md`.

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
3. **`-std=c89` revision keywords** (found by the 2026-11-04 matrix, outside the
   C17/C23 scope of that change). LCCC accepts four C99/C11 forms that GCC 16.2
   rejects in `-std=c89`: the `u8"..."` string prefix (C11), `restrict` (C99),
   `inline` (C99) and `//` comments (C99). The dialect capability layer
   (`src/common/dialect.rs`) decides these today from a single `c23` bit, which
   cannot express "C99 or later"; carrying the standard revision into the lexer
   and preprocessor as a number closes all four in the same place the C23
   keywords are decided. `c17`, `gnu17`, `c23` and `gnu23` have no deviations,
   so this item does not block them.

**Gates.** Keep the 43-probe × 4-dialect GCC 16.2 accept/reject matrix
(`tests/regression/check_c23_inventory.py`, which fails on any undeclared
difference), the diagnostic-text/severity and DWARF dialect-identity gate
(`tests/regression/check_c_std_diagnostics.py`), the default/gnu17
prototype differential, and the per-function AVX2 ISA-boundary/runtime test;
new `_BitInt` support must add compiler-oracle differential and ABI/layout
coverage rather than weakening the existing known-gap row. **Effort** M–L ·
**ROI** 3.3.

### MB-12 · P1 · R — warnings that real builds rely on

**Current state.** `-Wunused-variable`, core `-Wformat`, `-Wsign-compare` (R,
2026-11-20) and `-Wformat-security` have focused GCC 16.2 compatibility cases in
`tests/regression/check_warning_compat.py`; `-Wformat-security` needs both `-Wformat`
and `-Wformat-security`, and supports suppression, demotion and
`-Werror=format-security`. Clang 23.1.3 warns for the same source with different
text; Clang 23.1.4 and ICX 2026.2 were unavailable, so no exact-version claim is
made.

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

### MB-14 · P1 · R — linker residue: `--emit-relocs`, ELF32 `-r`, common-page-size, print options

`-z pack-relative-relocs` (RELR), `-z max-page-size`, `--image-base`,
`--oformat=binary`, `--build-id`, `-Map`, `--version-script`, `--icf=all` and `-pie`
work on x86-64 and on `-m32` (§2). What remains (R, 2026-11-20):

1. `--emit-relocs`: x86-64 warns "ignored: relocation retention is only implemented
   for -T script links" on the regular path; the i686 linker refuses it.
2. ELF32 `-r` requires the `gcc_linker` feature ("Relocatable linking (-r) requires
   the gcc_linker feature").
3. `-z common-page-size` is validated and then ignored with a warning (no separate
   layout granularity).
4. `--print-gc-sections` prints nothing (MB-02).

**Where.** `src/backend/i686/linker/{options,emit,shared,link}.rs`,
`src/backend/x86/linker/`, `src/backend/linker_common/*` (shared code such as
`mapfile.rs`, `version_script.rs`, `build_id.rs` already exists), `src/bin/lccc_ld.rs`.

**Reproducer.**
```bash
echo 'void _start(void){for(;;);}' > st.c
lccc -m32 -nostdlib -O1 st.c -o st32 -Wl,--emit-relocs   # error: not supported by the elf_i386 userspace linker
lccc -nostdlib -O1 st.c -o st64 -Wl,--emit-relocs        # warning: --emit-relocs ignored
lccc -m32 -O1 -c st.c -o a.o && lccc -m32 -nostdlib -r a.o -o r.o   # needs the gcc_linker feature
```

**Plan (one PR each).** (1) `--emit-relocs` for the regular layout path on both
linkers: keep the input relocations, rebased to output sections, in `.rela.*`/`.rel.*`
(compare `readelf -r` with `ld.bfd --emit-relocs`). (2) ELF32 `-r`: merge sections,
keep relocations and COMDAT groups; mirror `src/backend/x86/linker/emit_rel.rs.`
(3) Decide `common-page-size`: implement a layout granularity measured against
BFD 2.47, or document it as a permanent `BenignNoop`.

**Gates.** The linker suite (`tests/linker/run_linker_tests.py --strict`) gains one
case per step with BFD/lld as oracle where they accept the option. **Accept.** The
three reproducer lines no longer warn or fail. **Effort** M · **ROI** 3.2.

### MB-15 · P2 · R — whole-program mode; `-flto` is a hard error today

**Why.** `-flto` fails with "LTO IR production and link-time consumption are not
implemented" (R, 2026-11-20; honest, but distribution flag sets that carry `-flto`
fail outright). Cross-TU inlining and constant
propagation are among the largest wins GCC/Clang/ICX have on multi-file programs
(gzip, expat, zlib-ng are all multi-file), and the benchmark corpus is the stated
yardstick. Honest staging:

1. **Stage 1 (M).** `lccc -fwhole-program -O2 a.c b.c c.c -o app`: the driver
   parses all TUs, lowers each to IR, **merges the modules in memory** (statics
   renamed `TU#name`, extern declarations resolved to definitions, duplicate
   `inline`/COMDAT handling, tentative definitions merged) and runs the normal
   pipeline once. Only accepted when the whole program is on one command line
   (no `-c`). Expected effect: cross-TU inlining of small functions, global
   constant propagation, dead-function elimination with `-fwhole-program`
   semantics (non-`main` globals become internal unless exported by
   `-Wl,--export-dynamic` or attribute).
2. **Stage 2 (L).** `-flto` with `-c`: serialise IR into a `.lccc.ir` section
   (versioned, deterministic), and make `lccc-ld`/the driver perform the merge
   at link time (Stage 1 machinery). Needs a stable IR serialisation format
   and a symbol-resolution handshake (which symbols are preserved).

**Reproducer.**
```bash
echo 'int main(void){return 0;}' > p.c
lccc -flto -c p.c -o p.o; echo $?     # error: -flto: LTO IR production ... not implemented
```

**Gates.** Differential run-and-compare on gzip/expat/zlib-ng corpus: outputs
identical to non-LTO; counted instructions on hot paths drop (Callgrind, the
repo's paired harness); build with an *unrelated* TU defining a same-named
static to prove renaming. **Effort** M (stage 1) / L (stage 2) · **ROI** 3.0.

## 6. Tier M2 — speed of the tool

### MB-20 · P1 · R — peephole compile time: decode once

**Current state.** The quadratic label scan, the allocate-then-reject sites and the
pass-level re-verification are fixed and gated (lazy `LabelRefs`, the
allocation-before-filter ratchet `tests/regression/check_peephole_alloc_before_filter.py`,
pass-level dirty ticks with `CCC_PEEPHOLE_NO_DIRTY_SKIP=1` / `CCC_PEEPHOLE_VERIFY_DIRTY=1`
as the revert and honesty switches), and the liveness solver reads a branch target
with `helpers::second_token` instead of `SplitWhitespace` (byte-identical output;
`SplitWhitespace::next` fell from 622.6 M to 261.9 M instructions, total −1.94 %).
What remains, measured with Callgrind on `tests/regression/int_cmp_const_select_cross.c`
at `-O1` on `4d28473` (14.29 G instructions after the step above; the baseline
before it was 14.57 G): the peephole is **76.87 %
inclusive** against the ≤ 35 % criterion. Where it goes: the dual-layout metric pair
(`metric_pair::run_metric_pair`, two full peephole runs per function, 28.8 %),
`FileLiveness::solve_function` with its inlined slice/str helpers (about 11 %),
`LabelRefs::build` (1.7 %) and `adjust_counts` (2.2 %), `is_structural` (1.8 %, 3.7 M
calls), `FileLiveness::hash_function` (2.3 %), and diffuse `starts_with` (2.3 %) /
`LineKind::eq` (1.9 %) traffic over hundreds of sites. `analyse_function` runs 28 188
times for 1 128 functions: each pass re-solves functions the previous pass edited.

**Where.** `src/backend/x86/codegen/peephole/passes/{liveness,local_patterns,dead_writes,memory_fold,relay_and_lea,load_op_fuse}.rs`,
`src/backend/metric_pair.rs`.

**Reproducer.**
```bash
valgrind --tool=callgrind --callgrind-out-file=cg.out \
  lccc -O1 -S -o /dev/null tests/regression/int_cmp_const_select_cross.c
callgrind_annotate --inclusive=yes cg.out | grep -E 'peephole_optimize_inner|run_metric_pair|solve_function'
```

**Plan.** In order of measured cost: (1) cut the 28 188 re-solves — key the solve on the
function's edit generation so a pass that changed nothing in a function does not
re-run `analyse_function` for it (the dirty-tick machinery exists at pass level);
(2) share the decode between the two metric candidates: they differ only in block
order, so lines that are identical in both are classified once; (3) give `LineInfo` an
interned opcode id and operand kinds computed on creation or edit, then convert one
recogniser family per PR, most expensive first; (4) `is_structural` as a precomputed
`LineInfo` bit. Do not route `LineKind::eq` through the packed word: measured 16× worse
at fastbuild opt-level 1 (pinned in the enum's doc comment). Re-profile after each PR.

**Gates.** Byte-identical `.s` over the benchmark programs, the kernel corpus and the
asm-diff corpus before and after (existing identity scripts); a compile-cost ratchet
on the reproducer TU using Callgrind instruction counts with a 3 % tolerance (the
deterministic proxy) **plus** a paired wall-time check reported separately.
**Accept.** Peephole ≤ 35 % of total compiler instructions with output byte-identical.
**Risks.** A table that disagrees with the text it caches — hence the identity gates.
**Effort** M · **ROI** 4.5.

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
via an explicit allowlist with a reason. The 800-object synthetic link (§2) is the
performance input; keep its generator in `scripts/` and record
`lccc-ld` vs the three linkers' wall time **and** `LCCC_LD_TIME` phase output.
**Effort** M · **ROI** 2.8.

---

## 8. Tier M4 — code quality of generated code (carry-forward, re-ranked)

This tier is deliberately thin: the performance queue is large and was
produced under a measurement discipline this file does not repeat (paired,
order-balanced, Callgrind-backed; see `engineering/BACKLOG-ROI-2026-11-18.md`).

- **MB-30 · L · H — calibrate the cost model from published Raptor Lake data.**
  Replace hand-tuned latencies/ports in the scheduler/regalloc cost functions with a
  generated table (instruction → µops, latency, throughput, ports) built from a
  checked-in snapshot of the uops.info measurement data for Alder/Raptor Lake P
  cores, keyed by the mnemonic classes the backend already enumerates; the
  generator script lives in `scripts/`, the output in a generated Rust file with a
  version header. Gate: table lookup equals the old constants for the
  instructions where they were already right (regression), and the benchmark
  corpus A/B shows no >1 % regression per workload.
- **Metric trap.** `Ir` and wall clock have disagreed by up to 74 points in earlier
  campaigns — never accept an optimisation on static instruction counts alone.
  MB-15 (whole-program mode) remains the next *measurable* cross-cutting lever for
  the multi-file corpus members.

---

## 9. Tier M5 — engineering system

### MB-41 · P2 · S — extend the backlog gate: fields and dangling references

`scripts/check_backlog_roi.py` already checks that item headings parse and that every
`R` item carries a reproducer (`mb41_item_headings_parse`, `mb41_reproducers`).
Extend it with: every item has Why / Where / Plan / Gates / Accept (or Reproducer)
fields; every `ID` mentioned in the text has a heading or a table row (a mention of
a deleted item such as the ones this rewrite removed is a dangling reference); no
text says "archive" or "see the archive" (closed work is deleted from this file, and
its history lives in git and in [`engineering/journal/`](engineering/journal/README.md)).
Both new checks need a negative control in `--self-test`.

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
`switch_dispatch` 1.109, `nbody` 1.078). Work order: `lz4_match_extend` (byte-compare match extension: the
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

## 7a. asm-goto fusion: three codegen gaps (measured 2026-10-30, reproduced 2026-11-20)

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

## 8a. Declaration-parser and lowering tails (measured 2026-11-20)

Three functions on the declaration/lowering path exceed the 300-line ratchet and
are mechanical to split:

| function | file | lines |
|---|---|---:|
| `lower` | `src/ir/lowering/lower.rs` | 482 |
| `build_function_sig` | `src/ir/lowering/lower.rs` | 428 |
| `parse_declaration_rest` | `src/frontend/parser/declarations.rs` | 404 |

Extract the `Declaration::new` + `set_*` tail into one `finish_declaration(...)`
helper, and split `lower`'s call lowering from its statement walk. Semantics are
covered by `tests/regression/check_attribute_diagnostics.*` plus the attribute
option ledger; use `scripts/check_codegen_refactor_identity.py` as the
byte-identity gate. Do not raise `scripts/audit_census_baseline.json` to make room.

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

### 9b.5 Open from the 2026-10-31 follow-ups

* SV-01: randomized one-day seed/page/misalignment/overlap acceptance and the
  scalar-loop extension.
* SV-02: the hardened 44-kernel scoreboard needs controlled paired timing and a
  full sweep (the checksum-only gate exists).
* SV-03–SV-12 are open. The offline Godbolt cross-add witness still shows a static
  code gap against GCC/Clang; instruction counts are not runtime wins.

## 9c. SIMD implementation audit register

The implementation-level follow-up to §9b lives in
`engineering/SIMD-IMPLEMENTATION-AUDIT-2026-11-08.md` (54 findings SA-01…SA-54,
all **S** — source-anchored, none reproduced). It is a defect register, not a
confirmed-bug list: run the reproducers (`tests/simd/audit_repro_2026-11-08.c`,
`check_audit_2026-11-08.py`) before relying on a finding. SA-03, SA-04, SA-05,
SA-33 and SA-36 are fixed. Edit the audit document, not a copy of the register here.

## 10. Open performance and verification items (carried from the 2026-10-24 queue)

Historical source: [October 24 live queue](https://github.com/ms178/lccc/blob/4d28473646f9a549ad6c2e74852ee2f07b3d9f1a/engineering/archive/BACKLOG-2026-10-24-live-queue.md).

Each row keeps the measurement it was filed with; re-verify the reproducer before
starting, and delete the row when it stops reproducing.

| ID | Item | Accept |
|---|---|---|
| UP-3 | RISC-V relocation error/preservation contracts: construct malformed-expression and mutated-relocation fixtures; enumerate applied/retained/relaxed/rejected classes for exec and shared outputs | No unsupported expression becomes zero; malformed metadata rejected in both output modes; valid symbolic subtraction stays a positive control |
| UP-4 | Fannkuch runtime regression: explain before changing code | Matched paired runner; mechanism identified before any edit |
| UP-5 | Zstandard code quality: `ZSTD_decompressSequencesLong_default` 2517 instructions / 871 frame refs vs GCC 1786 / 401; `ZSTD_safecopy` 211 / 58 vs 152 / 0; no victory claim from instruction counts | Output-checked paired runtime on useful durations; all regressions retained |
| UP-6 | Validate on real target hardware (Raptor Lake 14700KF PMU, EPYC 7763): VM numbers are screening only | Raw pairs, CPU/microcode, affinity, frequency policy, hashes, PMU availability; no universal "best compiler" claim |
| UP-7 | Fresh current-tree comparison and required local gates after any rebase | Frozen main vs candidate, recompiled corpus, immutable identities |
| UP-8 | Scalar min/max/abs lowering beyond the landed `max(0, a-b)` sign-test CMOV case (6→5 instructions; GCC 4, Clang/ICX 5): enumerate remaining shapes legal from exact flag semantics | Exact differential over the arithmetic domain incl. wrapping edges; mutation-sensitive gates; same-width and intervening-flag barriers kept |
| UP-9 | IPA-RA known-callee clobber sets: `hot_web_loop` 1.549× GCC 16.2 (99.8 ms median), 18 stack refs vs 0, caused by assuming every call clobbers all caller-saved registers around a `static noinline` helper | Per-callee written-register summaries in reverse topological order; conservative for recursion, indirect, escaped and external-ABI calls; no allocator miscompile; paired runtime on the 49-program corpus |

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

**R, 2026-11-20.** An invalid *output* constraint is accepted and a garbage operand
is emitted; GCC 16.2 stops with `impossible constraint in 'asm'`:
```bash
echo 'int main(void){int x; __asm__("mov %0, %1" : "=Z"(x) : "q"(1)); return x;}' > c1.c
lccc -S c1.c -o -     # emits "mov %ecx, %edx"-style text, rc 0
gcc  -S c1.c -o /dev/null   # error: impossible constraint in 'asm'
```

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

---

## 13. DWARF campaign (measured 2026-11-05)

Full analysis, design and per-item file anchors:
[`docs/DWARF-CAMPAIGN.md`](docs/DWARF-CAMPAIGN.md). Instrument and gate:
`tests/regression/check_dwarf_quality.py` (`--self-test` validates its decoders;
`--lccc`, `--gcc`, `--json`, `--markdown` mirror the other gates' contract). The
gate reports unimplemented DIE kinds as **named known gaps**, so the list below
cannot rot: a gap that stops reproducing is reported and its entry must be
deleted in the same commit that closes the item.

Baseline before the series (18 DIE kinds at zero, 1 752 DIEs):
`variable` 0/417, `formal_parameter` 0/485, `base_type` 0/480, `pointer_type`
0/122, `typedef` 0/103, `const_type` 0/98, `subrange_type` 0/78, `array_type`
0/74, `member` 0/59, `lexical_block` 0/127, `structure_type` 0/14, `subprogram`
283/338 (the deficit was the `-O0` statics, **closed**), `unspecified_type` 49/0.
Structural audit (line program, address map, `.eh_frame`) over 65 files × 4
configurations: **0 failures**. Correct and gated already: DWARF 2–5 line tables
with version-gated `prologue_end`/`epilogue_begin`/`discriminator`/`isa`,
per-unit `.debug_aranges` sets, post-peephole synthesized CFI, dialect identity.

| ID | P | Effort | Item | Gate added |
|---|---|---|---|---|
| D1b | P1 | M | a parameter's `DW_AT_location` (needs the frame layout as a `.dwarf_param` directive) | every parameter's `DW_OP_fbreg` reads the value the program prints |
| D3b | P1 | M | parameter and local locations (`DW_OP_fbreg` at `-O0`, location lists when optimized) | the location evaluates (CFA walk) to the value the program printed |
| D6 | P1 | L | `DW_TAG_inlined_subroutine` with `call_file`/`call_line` | two inlining instances, ranges inside the caller, abstract origin declared |
| D7 | P1 | M | `-S -g` carries the DIE tree (GCC puts the sections in the `.s`; lccc puts `.loc` only) | `-S`+`as` object's tree equals the `-c` object's |
| D8 | P2 | M | `-g3`: `.debug_macro` | two macros with line and replacement text; nothing at `-g` |
| D9 | P2 | S | `-gz` compresses (today: warned as unsupported, emitted plain) | every `.debug_*` carries `SHF_COMPRESSED` and decodes |
| D10 | P2 | S | `-fdebug-prefix-map`/`-ffile-prefix-map` on `DW_AT_name`/`comp_dir`/`decl_file` | two builds in different directories are byte-identical |
| D11 | P3 | L | `-gsplit-dwarf`: skeleton unit + `.dwo` | skeleton names the `.dwo`; the tree resolves `pq_global` |
| D12 | P3 | M | `.debug_names` for DWARF 5 | every subprogram/variable indexed once; lookup equals a full walk |
| D14 | P1 | L | local variables and `DW_TAG_lexical_block` (GCC 695 variables, 207 blocks; lccc 68, 0) | every executable local's value equals what the program prints |

Ordering is ROI: D4 is a day's work and removes a whole class of "no debug info"
answers; D1+D2+D3 are the centre (they are what "godlike
debugging" means) and land as a series, each with executable assertions; D6–D12
are the polish that makes the artifact GCC-shaped rather than merely valid.

Standing constraints from §1's item format apply unchanged: `fastbuild`, `-j2`,
`scripts/ci_local.sh --fast` before declaring done, mutation-verified gates
(revert the fix, watch the gate fail), no reproducer removed to close an item.

---

## 14. Whole-tree audit (2026-11-20, `dbdadc9`)

Method: the repo's own static gates (`scripts/audit_census.py`, doc-link,
conflict-marker, backlog-ROI), source reading of every non-test `unsafe` block,
and runtime probes on a `fastbuild` binary (§2), including a mutation fuzz
(1 500 mutants, 3 400 compilations) and a run-to-run determinism sweep. No clippy and no CI run was used. Items
marked **S** are source-anchored and have not been reproduced: their first plan
step is to turn them into **R** or delete them. Census at this commit: 769 Rust
files, 1 195 507 lines, 859 panic sites (94 in frontend/lowering/driver), 27
`unsafe` sites, 233 / 97 / 26 functions over 300 / 500 / 1 000 lines, 576 knob
names (248 undocumented).

### AU-02 · P1 · R — run the frontend mutation fuzzer nightly and add coverage-guided targets

**Why.** The existing fuzzers (`tests/fuzz/differential_fuzz.py`,
`phi_cfg_fuzz.py`) generate *valid* programs and compare output; nothing feeds the
frontend invalid or hostile input, although it has 94 panic sites in user-reachable
code. `tests/fuzz/mutate_frontend.py` (added with this audit) mutates the repo's own
C files (token insertion, truncation, byte flips, duplication, NUL and 0xFF bytes)
and classifies results as diagnostic / ICE / signal / timeout. The first run on
`4d28473` (1 500 mutants, 3 400 compilations over `-O1`, `-O2 -mavx2`, `-m32 -O1`)
found one hang (a malformed `#define` parameter list, fixed and pinned by
`tests/regression/check_malformed_define.sh`); afterwards the same sequence plus 2 500
fresh mutants gave 0 internal errors, 0 signals, 0 hangs (R). That is a good baseline,
and it is blind to anything the mutators cannot reach.

**Where.** `tests/fuzz/mutate_frontend.py`, `tests/fuzz/README.md`; new
`fuzz/` crate for `cargo-fuzz` targets (not present today).

**Reproducer.**
```bash
python3 tests/fuzz/mutate_frontend.py --lccc target/fastbuild/lccc \
  --seeds 'tests/**/*.c' --iterations 900 --out /tmp/lccc-mutfuzz -- -O1
```

**Plan.** (1) Run the mutator nightly with a rotating `--seed`, at `-O0`, `-O2
-mavx2`, `-m32 -O1` and `-S -g`; any finding is minimised and committed as a
regression case. (2) Add coverage-guided targets: preprocessor (`preprocess(&str)`),
parser (`parse(&str)`), the x86 assembler's `parse_asm`, the ELF object reader in
`linker_common` and `.a` archive parsing. Seed from `tests/corpus/clang-c`. (3)
Add a dictionary of C tokens and GNU extensions (`__attribute__((`, `asm goto`,
`_Generic(`, `typeof(`, `_BitInt(`) to both fuzzers.

**Gates.** A target must run 10 minutes in CI-free local mode without a crash before
it is committed. **Accept.** 100 000 mutated inputs per night: 0 ICE, 0 signal, 0
timeout; every past finding has a regression case. **Effort** S–M · **ROI** 4.0.

### AU-03 · P1 · S — no Miri, no coverage measurement

**Why.** `ci.yml` and `scripts/ci_local.sh` contain no Miri, `llvm-cov`, sanitizer or
`cargo-fuzz` lane. The riskiest code is exactly what Miri checks: raw-pointer
partitioning in `src/backend/x86/linker/parallel_reloc.rs` (`from_raw_parts_mut`
over a pointer round-tripped through `usize`), the memory-mapped input in
`src/backend/linker_common/filemap.rs`, and the string table in `symstr.rs`. The
IR lowering layer (about 35 k lines in `src/ir/lowering/`) has no in-file tests, so
its protection is the integration corpus, and nobody can say which lowering paths
that corpus never reaches.

**Where.** `.github/workflows/`, `scripts/ci_local.sh` (a new optional lane),
`src/backend/linker_common/`, `src/backend/x86/linker/parallel_reloc.rs`.

**Plan.** (1) `cargo +nightly miri test` over the `linker_common` and
`parallel_reloc` unit tests (add tests where there are none; replace the `usize`
round trip with `split_at_mut`/`chunks_mut`, which Miri accepts without exposed
provenance). (2) Add a `cargo llvm-cov` report over `tests/regression` and the
corpus, published as a table per module; set a ratchet per top-level module,
starting at today's numbers. (3) List the lowering functions with zero coverage and
file one test each.

**Gates.** The Miri lane is allowed to run only on the linker-support subset
(minutes, not hours). **Accept.** Miri clean on that subset; a coverage table exists
and a ratchet blocks regressions. **Effort** M · **ROI** 3.6.

### AU-04 · P2 · S — crate-wide lint suppression hides dead code

**Why.** `src/lib.rs` carries `#![allow(dead_code, unused_imports, unreachable_code)]`
and a block that allows 75 distinct `clippy::` lints, among them `if_same_then_else`,
`unnecessary_unwrap` and `needless_range_loop` (lines that often mark a copy-paste
bug). In a 1.18 M-line tree built from many agent sessions, "dead" code accumulates
silently: the parked peephole Phase 4 (AU-09) is one known case. Nobody can say how
much is dead.

**Where.** `src/lib.rs` (the allow blocks at lines 11 and 44 onwards) and the 50 `#[allow(clippy::too_many_arguments)]`
sites.

**Plan.** (1) Measure: `RUSTFLAGS='-W dead_code -W unused_imports -W unreachable_code'
cargo check --profile fastbuild` (a rustc lint run, no clippy) and count warnings
per module. (2) Delete what is truly dead; for deliberate keep-alive code add an
item-level `#[allow]` with a reason. (3) Remove the crate-level allows module by
module, with `audit_census.py` ratcheting the number of remaining allows.

**Accept.** The three crate-level allows are gone or each remaining one has a named
owner in `engineering/DECISIONS.md`. **Effort** M · **ROI** 2.7.

### AU-05 · P2 · R — three diagnostic spellings and an ICE that looks like an error

**Why.** One driver run prints `ccc: error: …` (hard errors), `lccc: warning: …`
(`-gz`, `-gsplit-dwarf`), bare `warning: …` (tolerated options) and
`lccc-ld: warning: …` (linker). Tools that match the program name prefix, and users
searching logs, see three products (R, 2026-11-20). In `compiler_main`
(`src/lib.rs`) a panic is reported as `ccc: internal error: <message>` with exit
status 1 (S): the same status as an ordinary diagnostic, so build systems and
`cc-option` probes cannot tell "your code is wrong" from "the compiler crashed", and
the message names neither the input file, the function nor the pass.

**Where.** `src/lib.rs` (`compiler_main`, panic handler), the diagnostic emitters
under `src/driver/`, `src/bin/lccc_ld.rs`.

**Reproducer.**
```bash
echo 'int main(void){return 0;}' > p.c
lccc -static-pie p.c 2>&1 | head -1     # "ccc: error: -static-pie is not supported ..."
lccc -gz -c p.c 2>&1 | head -1          # "lccc: warning: compressed debug sections ..."
lccc -Wshadow -c p.c 2>&1 | head -1     # "warning: unrecognized command-line option ..."
```

**Plan.** (1) One prefix, `lccc:`, for the compiler driver and `lccc-ld:` for the
linker, through a single emitter; update every golden file and test that matches the
old spelling. (2) A distinct exit status for internal errors, documented in
`docs/`. (3) Carry the current input file, function and pass name in a thread-local
guard that the panic handler prints, plus a one-line bug-report hint and the
command line. (4) On an ICE write the preprocessed input and arguments to a temp
directory and print its path.

**Gates.** A test-only knob that panics in a named pass; the test asserts status,
prefix and the file/function/pass fields. **Accept.** No `ccc:` prefix remains; ICE
and diagnostic statuses differ. **Effort** S–M · **ROI** 3.2.

### AU-06 · P3 · S — `std` hash maps in hot backend code

**Why.** 34 non-test files, among them the i686 peephole (27 uses), the ARM peephole (5),
`passes/copy_prop.rs` (3) and `linker_common` (`defsym.rs`, `eh_frame.rs`), use `std::collections::HashMap`
/ `HashSet` (randomly seeded SipHash) while the rest of the tree uses `FxHashMap`.
Output order is not at stake today (R: 12 programs × 4 configurations × 4 runs gave
byte-identical assembly and objects), but SipHash costs compile time that MB-20
chases elsewhere, and one iteration over such a map in an emit path would make
output nondeterministic without any test noticing.

**Where.** `grep -rn "std::collections::{.*HashMap\|std::collections::HashMap" src/backend/i686 src/backend/arm src/backend/linker_common src/passes`.

**Plan.** (1) Replace with `FxHashMap`/`FxHashSet` behind the existing alias. (2)
Wherever a map is iterated into output, sort or use an ordered map. (3) Turn the
determinism sweep into a script under `scripts/` (compile each program N times,
compare hashes of `-S` output and the object) and run it before and after.

**Gates.** `scripts/check_codegen_refactor_identity.py` (byte identity) and the new
determinism script. **Accept.** Byte-identical output; measured compile-time delta
reported with Callgrind per MB-22. **Effort** S · **ROI** 2.3.

### AU-07 · P2 · S — two 4 k-line linker emitters that are half the same code

**Why.** `emit_executable` (4 262 lines, `src/backend/x86/linker/emit_exec.rs`) and
`emit_shared_library` (4 254 lines, `emit_shared.rs`) share 51 % of their
substantive lines verbatim (600 of 1 182 lines longer than 25 characters). A fix to
layout, TLS, relocation or note handling has to be made twice.
`link_with_script_machine` adds a third
3 340-line function; the i686, ARM and RISC-V linkers repeat the structure.

**Where.** `src/backend/x86/linker/{emit_exec,emit_shared,emit_script}.rs`,
`src/backend/linker_common/`.

**Plan.** Extract the shared phases (section layout, PT_LOAD construction, dynamic
section, `.note.*`, relocation application, RELR packing) into functions in
`linker_common` parameterised by a small `OutputKind` enum, one phase per PR, each
gated by byte identity of the produced ELF over `tests/linker/` and the benchmark
programs (`readelf -a` diff plus `cmp`).

**Accept.** Both emitters under 1 500 lines, byte-identical outputs. **Effort** M ·
**ROI** 3.1.

### AU-08 · P2 · S — 26 functions over 1 000 lines

**Why.** The census lists them: `emit_intrinsic_impl` (7 131, x86 and 2 725 on
i686), `allocate_registers` (3 686), `run_passes` (3 233), `encode_mnemonic`
(3 147) and the three linker functions of AU-07. `i686/codegen/peephole.rs` is a
single 27.8 k-line file whereas the x86 peephole is already split into `passes/`.
Giant functions defeat review, make per-function compile cost unmeasurable and are
where merge conflicts and miscompiles concentrate.

**Where.** `scripts/audit_census.py --top-fns 30`.

**Plan.** One split per PR, in this order: `run_passes` (a pass table with
declarative ordering), `emit_intrinsic_impl` (per-ISA-family modules), `encode_mnemonic`
(per-mnemonic-class tables), `allocate_registers` (phases), then the i686 peephole
mirroring the x86 `passes/` layout. Each PR gates on
`scripts/check_codegen_refactor_identity.py`; never raise the census baseline.

**Accept.** Functions over 1 000 lines: 26 → at most 10. **Effort** L · **ROI** 2.6.

### AU-09 · P2 · S — parked optimisations are dead or opt-in code with a known miscompile

**Why.** Four features are switched off because they miscompile, and each carries
code and tests nobody exercises: peephole Phase 4 (register renaming) runs only
when `CCC_PEEPHOLE_PHASE4` is set and `!no_machinst` (two `#[ignore]`d coalescing
tests in `peephole/passes/mod.rs` are blocked on it); `src/passes/loop_rotate.rs` is
opt-in after 16 miscompiles on default-enable (`CCC_LOOP_ROTATE=1`,
`engineering/DECISIONS.md`); `CCC_SROA_COPYOUT=1` did not transform the case it was
written for; the ARM GSF pass is parked on a complex float-array miscompile (four
`#[ignore = "AUD-ARM-GSF-1 …"]` tests). Loop rotation is a standard optimisation
GCC applies at `-O2`, so the parked state also costs generated-code quality.

**Where.** `src/passes/loop_rotate.rs`,
`src/backend/x86/codegen/peephole/passes/mod.rs`, `src/backend/arm/codegen/peephole.rs`.

**Plan.** For each feature: (1) reproduce the miscompile with `CCC_VERIFY_IR` and the
differential fuzzers; (2) either fix the root cause and enable it behind a corpus
A/B (no workload slower by more than 1 %), or delete the code and its ignored tests
in the same commit; (3) record the verdict in `engineering/DECISIONS.md`. Start with
`loop_rotate` (largest generated-code upside).

**Accept.** No `#[ignore = "blocked on …"]` test and no default-off pass without an
owner item. **Effort** M per feature · **ROI** 3.0.

### AU-10 · P2 · S — 576 environment knobs, 248 undocumented, all read in release builds

**Why.** The compiler reads the environment 651 times (`passes/mod.rs` 93,
`vectorize.rs` 41, `driver/pipeline.rs` 39). Any `CCC_*` variable left in a build
environment changes code generation without appearing on the command line or in a
build log, which is the opposite of reproducible output; and each undocumented knob
is a decision nobody can review. The census already counts undocumented knobs; the
ratchet only stops the number growing.

**Where.** `scripts/audit_census.py --list-undocumented-knobs`,
`engineering/subsystems/`.

**Plan.** (1) A single knob registry (name, default, owner, documented effect) from
which the documentation table is generated. (2) Gate dev-only knobs behind a cargo
feature (`dev-knobs`) so a release `lccc` ignores them. (3) Print the effective
non-default knobs under `-v`. (4) Document or delete the 249.

**Accept.** Undocumented knobs: 248 → 0; a release binary built without
`dev-knobs` ignores a `CCC_*` variable (test it). **Effort** M · **ROI** 2.8.

### AU-11 · P3 · S — `unsafe` hygiene

**Why.** 27 non-FFI `unsafe` sites, all small, but three need work: `parallel_reloc.rs`
builds `&mut [u8]` slices from an address stored as `usize` (AU-03 replaces it);
`src/backend/riscv/codegen/emit.rs:710` dereferences a raw `*const IrFunction`
(`current_func`) without a `SAFETY` comment, whereas the x86 equivalent
(`src/backend/x86/codegen/emit.rs:4047`) is centralised and documented; `pgo/mod.rs`
leaks a boxed table per unit to hand out `&'static` references.

**Plan.** Centralise the `current_func` accessor for riscv, arm and i686 like x86
(or pass `&IrFunction` through the emit context and delete the pointer); write the
missing `SAFETY` comments; replace the PGO leak with an arena owned by the unit.

**Accept.** Every remaining `unsafe` block has a `SAFETY` comment; Miri (AU-03)
covers the linker ones. **Effort** S · **ROI** 2.0.

### AU-12 · P3 · S — repository weight and dated session logs

**Why.** Documentation volume exceeds what a reader can keep current: 708
Markdown files; `docs/` holds 87, 42 of them dated session reports; top-level
`engineering/` holds 229 dated files; `engineering/evidence/` is 175 directories and
77 MB of committed JSON. `ci.yml` is 4 195 lines and `scripts/ci_local.sh` 4 068.
Dated logs go stale the day after they are written, and new agents spend context
reading them.

**Plan.** (1) Move session logs older than 30 days out of the tree (git history keeps
them) except those still linked as evidence from an open item here. (2) Store bulk
JSON evidence as release assets with a checksum manifest in the tree. (3) Split
`ci.yml` into reusable workflows and generate the shell lanes from one table shared
with `ci_local.sh` (`scripts/check_ci_gate_parity.py` exists to hold them together).
(4) Add a gate that fails when a dated doc is not referenced from a live document.

**Gates.** The doc-link gate stays green after each removal. **Accept.** `docs/` and
`engineering/` contain only documents referenced from the live tree; the repository
shrinks by at least 70 MB. **Effort** M · **ROI** 2.2.

---

## 15. Register-homing campaign (opened 2026-11-12, base `3964191`)

**Mandate (owner, standing).** Demand *perfect* homing and register allocation
for every vector type and shape: **zero memory/stack round trips**, for 128-
and 256-bit, integer and floating point, parameters, temporaries and returns.
No scope limit, no regressions. Supersede, but do not re-litigate, the
individual defect rows in §9a/§9b/§9c and `docs/AUDIT-2026-11-05-open-simd-prs.md`:
those remain the evidence base; this section is the ordered queue.

**Method.** Every claim needs four receipts: (1) a differential value proof
against libm/GCC at run time, (2) the emitted-assembly count and shape,
(3) a gate that fails on the tree that motivated the fix (measured, not
asserted — disable the pass and watch the gate go red), and (4) no regression
in `ci_local.sh --fast`. Instruction counts are taken between the function
label and `.size` so `ret` is inside the budget on every side of a comparison;
mixing counting conventions is how a 28-instruction body gets reported as 26.

### 15.2 Queue (measured gaps, ROI order)

Every row is a *measured* defect on the current tree, not a hypothesis.

1. **RH-11 (P1) — intrinsics with NO packed encoding at all.**
   Found while measuring packed-integer homing: intrinsics that have no packed encoding fall back
   to scalar lane loops in `include/*intrin.h`.  Open: the six builtins that cost 5 for
   lack of any twin (`_mm_andnot_si128`, `_mm_avg_epu8`, `_mm_mulhi_epu16`,
   `_mm_adds_epi8`, `_mm_packs_epi16`, `_mm_unpacklo_epi8`): one `Vec*` twin
   and one emitter arm each; and `_mm_max_epu16`/`_mm_min_epu16` stay at 3
   because `pmaxuw`/`pminuw` have no pre-AVX-512 encoding (the
   saturating-difference identity is the floor there).  Numbers and both
   traps: §6.6/§6.7/§6.8 of `engineering/AUDIT-VECTOR-ABI-2026-11-10.md`.
1a. **RH-13 (P2) — i686 twin coverage.**  The 32-bit backend asserts "only
   the dest_ptr form is supported here" for a subset of the `Vec*` twins, so
   the packed-integer pointer-form fix is x86-64 only and `-m32` keeps the pointer path.  The gate already
   runs the full value matrix for i686.
1b. **RH-12 (P1) — the by-value vector parameter is still an `Sse,SseUp`
   AGGREGATE passed as a pointer.**  §6.7 removed the inliner's `memcpy`
   bridge, so the `VecLoad` off a parameter home is forwarded into the
   incoming `%xmm`/`%ymm`; what remains is the layer below: the parameter is
   modelled as a 16/32-byte struct argument, so the ABI prologue still writes
   a home that the ordinary case never reads, and the memory-operand shapes
   stay one instruction above GCC (§6.6).  The fix is in lowering/ABI (a real
   vector parameter class whose `arg_types` is the vector type), not in the
   backend; it subsumes RH-04 and needs the same cross-TU ABI discipline.
2. **RH-04 (P0) — vector parameter homes.** Incoming `__m256`/`__m128`
   parameters are always stored into a 16/32-byte home by the ABI prologue,
   even when the only use is a memory operand.  Acceptance: a parameter that
   does not escape never touches memory; `check_param_home_contracts.py` stays
   green; no cross-TU ABI change.
3. **RH-05 (P1) — FMA3 operand rotation costs register copies.**
   `_mm256_fmadd_ps(a, *(p+8), c)` = 5 vs GCC 2 (two `vmovaps`/`vmovdqa`
   copies) and `fma_nested` = 5 vs 3.  The destructive form must be chosen so
   that one operand already lives in the return register.
4. **RH-06 (P1) — address arithmetic is not folded into the memory operand.**
   `*(p+8)` emits `addq $32,%rdi` then `(%rdi)`; GCC emits `32(%rdi)`.
5. **RH-07 (P1) — memory addend.** `_mm256_fmadd_ps(a,b,loadu(p))` = 3 vs 2:
   needs the 213 spelling with the memory operand as the addend.
6. **RH-08 (P1) — 128-bit double wrapper.** `_mm_fmadd_pd(loadu(p),b,c)` =
   11 vs 2: the I128 carrier bridge documented in
   `try_native_packed_fp_helper` must become a native bridge.
7. **RH-09 (P2) — `fmsub`/`fnmadd` header loop bodies** (13 instructions),
   lane extract `a[3]` (32-byte home round trip), `v8 f(v8)` 12 vs 6 (budget
   gate `tests/regression/vector_abi/`).
   DONE (vector-return marshalling): `v8 f(v8)` 7→5 (budget 6) and
   `v4 f(v4)` 12→5 via (a) the `call_arg_realignment_pins_frame` FPO gate,
   (b) the `vecabialign` Phase 11d' pass, (c) the adjacent-`%ymm`
   self-reload peephole (per-function AVX-512 screen), (d) the one-store
   `[Sse, SseUp]` return home in `expr_calls.rs`.  STILL OPEN, measured at
   `-O2 -mavx2` against GCC 16.2: (i) lane extract still homes through the
   frame (`a[3]` = 4 insns vs 2: needs register-lane `vshufps`/`vextractps`
   extraction); (ii) float scalars live across a call round-trip through GP
   (`movd %xmm0,%eax; movq %rax,SLOT` — IPA-RA knows the callee's clobbers
   but the allocator never considers keeping FP temporaries in unclobbered
   XMM registers); (iii) aggregate zero-init followed by a full-width
   I128/vector overwrite with a call in between survives DSE
   (`struct wrap w = { mk4(s) };` keeps two `movq $0` stores).
8. **RH-10 (P2) — benchmark coverage.** Add the intrinsic-homing kernels
   (load/FMA/integer map/parameter/return) to the integrated benchmark section
   with instruction counts AND wall-clock, so RH-04 and the other homing rows are ranked by real
   workload impact rather than by count alone.  Slices to include: zlib-ng
   `adler32`/`crc32` and `compare256`, gzip `longest_match`, expat UTF-8
   scanning, SQLite bytecode dispatch.

### 15.3 Sandbox prerequisites (do this first, every session)

`engineering/verification/prepare_local_env.sh` (idempotent, symlinks only,
no network, no live-root package mutation) is now required for a green local
mirror: it publishes the offline i386 sysroot (headers, crt, libgcc,
`/lib/ld-linux.so.2`, `gnu/stubs-32.h`), `/usr/lib/debug` and — added in this
session — `/usr/libexec/valgrind` plus the Debian suppression file, without
which valgrind cannot start `memcheck` and the hexadecimal-literal oracle
fails rather than reporting a Memcheck result.  PyYAML
(`python3 -m pip install --break-system-packages pyyaml`) is required by
`check_ci_workflow_shell.py`.
