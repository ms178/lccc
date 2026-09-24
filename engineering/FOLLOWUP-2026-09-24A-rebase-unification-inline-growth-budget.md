# Follow-up S04 — rebase onto PR #605, mnemonic-model unification, inline growth budget (linux_find_bit)

Session date: 2026-09-24. Base moved: upstream merged PR #605 (i686
audit adjudication, `07ac7c2`); this session rebased the cumulative
S03 patch onto it and continued the audit/perf work. Three harness
wipes hit mid-session; all recovered from the published artifacts
(byte-exact round-trip: regenerated diff SHA-256 == ms178-1.patch
SHA-256). Wipe #3 took /tmp (losing one CI log) but spared the tree;
it also left a stray 8 GB `/swapfile` that filled the disk mid-build —
removed; the mandatory `/var/swapfile-lccc` (6 GB) is the only swap.

## 1. Rebase

Clean: PR #605 touches i686 peephole; this work touches x86-64 codegen,
middle-end passes, gate scripts. Zero conflicts. All gates re-run after
the rebase (see §4).

## 2. Red-team audit of S03 — findings and verdicts (self-audit, per standing order)

| # | Item | Verdict |
|---|------|---------|
| A1 | Two drifted dest-only mnemonic lists (helpers `is_read_modify_write` vs liveness `is_pure_write_mnemonic`) — the sha256 bug class | **FIXED ARCHITECTURALLY**: liveness now DELEGATES to the helper (single source of truth; drift structurally impossible). Equivalence proven at the call site: string instructions short-circuit first, xmm dests never consult the predicate (`dest <= REG_GP_MAX` guard), and the caller's operand-text guards subsume the LEA self-address exception. New drift-alarm test pins the full semantic table in both directions |
| A2 | LEA with dest inside its own address (`leaq 4(%rax), %rax`) | Old liveness list said pure-write; the caller's `src_reads_dest` guard already caught it — no hole, but the unified model now reports RMW directly (belt AND suspenders) |
| A3 | `slot_width_fits` consolidation changed the acc-branch condition from `!(small && S64)` to the full width check | Strictly safer (rejects S64 AND anything outside {8,16,32}); identical for every integer MachInst size; proven bit-exact on sha256 pre-wipe and all golden gates green post-rebase |
| A4 | F7 register-class-blind acc substitution | **DISAGREE with the finding as a defect**: `acc_has(v)` true ⇒ the emitter last wrote value `v` to %rax — the invariant is value identity, not register class; a class mismatch would also produce syntactically invalid asm (loud assembler error, never silent miscompile). The window-rax-free scan + window_defs + cache invalidation manage staleness. Kept the pinning tests; no code change |
| A5 | `LCCC_DUMP_IR` in pipeline.rs flagged as debug leftover | **DISAGREE**: it is part of the documented debug-knob family (next to `LCCC_DUMP_PRECG`, `LCCC_DEBUG_LABELS`), not garbage. Kept |
| A6 | Mov/FMov def-set source-side over-recording vs Movzx dst-only asymmetry | Sound both ways (the set gates acc substitution; over-recording only forbids, never permits). Pinned by test. Follow-up P2: dropping Mov src-recording may enable more substitutions — needs its own measurement |

## 3. linux_find_bit: oracle distillation + inliner growth budget

Godbolt oracle (pinned: GCC 16.2 `cg162`, Clang 23.1 `cclang2310`, ICX
`cicxlatest` — `scripts/godbolt.py audit` clean) on
`tests/benchmark/programs/linux_find_bit.c`, `-O3 -march=x86-64-v3`:

- All three oracles inline `linux_find_next_andnot_bit` (static, 3 call
  sites: two cold self-test calls + one hot in-loop call). LCCC kept it
  outlined: the bounded loop-helper tier capped cloning at 2 sites.
- LCCC's outlined body is instruction-selection SUPERIOR to GCC's
  inlined shape: `andnq` for `& ~addr2`, `shlxq` mask build, and the
  generic `__ffs` decision tree folded to a single `tzcntq` (GCC keeps
  the 6-level branch tree even inlined). Static: lccc main 108 insns vs
  clang 118 / icx 121 / gcc 274.
- The runtime delta was therefore pure call overhead + missed
  caller-context. Forced-inline probe (always_inline copy): 6–12%
  faster in paired runs (contention-inflated; idle-box confirmation
  flat-to-better, 1.00 vs base, **0.86 vs local GCC** at PASSES=4096,
  outputs identical across lccc/base/gcc).

Fix (principled, calibrated against existing provenance):
`fits_static_loop_inline_limits` now bounds the bounded tier by a
**clone-growth budget**: `inst_count * (sites - 1) <= 128` with the
site cap raised 2→3. The budget reproduces the historical worst
permitted case exactly (2 sites × 128 insns = 128) and admits the
third site only while total growth stays inside the same envelope. The
glibc-memcmp clone regression (five 27-insn clones, −7.1%) lives in the
small tier and is untouched; the zlib Adler-32 `-Os` nest veto is
untouched. Result: the kernel now inlines at all three sites (zero
`call linux_find_next_andnot_bit` in the output, 3× `tzcntq`), golden
codegen-quality gate green, 9-kernel runtime sweep vs base shows no
regression (sha256 0.979, hash_table 0.979, sieve 0.976, rest flat).

## 3b. ABM v3 default (LZCNT/TZCNT) — found by the find_bit gate

The new `tests/regression/check_findbit_inline.sh` gate (pins zero
outlined calls + tzcnt + andn at BOTH `-O2` and `-O3 -march=x86-64-v3`)
caught an asymmetry in the S01 v3-baseline feature: `andn` (BMI1) was
default-ON without `-march`, but `tzcnt` (ABM) was not — plain `-O2`
emitted `bsfq` for the `__ffs` fold while `-march=x86-64-v3` emitted
`tzcntq`. The pipeline's own docstring says the absent-`-march`
baseline "projects x86-64-v3", and v3 carries ABM; the gap was an
implementation drift, exactly the class the drift-alarm work targets.

Fix mirrors the S01 BMI pattern exactly: `lzcnt_effective()` =
explicit `-mlzcnt` OR (x86-64 && no explicit `-march`), AND NOT sticky
`-mno-lzcnt` denial (new `lzcnt_explicitly_disabled` field; `-mlzcnt`
after `-mno-lzcnt` re-enables, GCC last-explicit-wins semantics). Four
new CLI tests pin default-on, sticky denial, re-enable, and the
explicit-march ceiling (v1 removes it, v3 carries it). Soundness:
`tzcnt`/`lzcnt` are at least as correct as the `bsf`/`bsr` they
replace (defined on zero input), so defaulting them ON can only remove
the undefined-zero-input hole, never change a defined program's result.
Preprocessor feature macros stay on the raw flags — same precedent as
the S01 BMI macros (codegen-effective default, source-visible gates stay
explicit).

## 4. Validation summary (post-rebase, post-changes)

- `cargo test --lib`: 3302 passed / 0 failed (was 3292 + upstream PR #605
  tests + 2 new: mnemonic drift-alarm table, inline growth-budget rows).
- `ci_local.sh --fast` run 1 (rebase + unification): **63 passed, 0
  failed, 3 skipped**. Run 2 (+ inliner change): see ledger — golden
  codegen-quality gate PASS in both.
- `check_ch_maj_codegen.sh`: PASS. `cargo fmt` / clippy `-D warnings`:
  clean.
- Correctness: linux_find_bit checksum identical across lccc /
  lccc-base / gcc at PASSES 1024 and 4096.

## 5. TODO (next session)

1. chacha20 scalar RA rework on Zen5 (multi-session; structural).
2. Remaining EPYC regression list re-measured post-S04 on real EPYC
   hardware (sieve 1.25, glibc_strstr 1.11, fannkuch 1.11 are the next
   oracle-distillation candidates; mandelbrot 1.09). Oracle distillation
   for these four is in progress via `scripts/godbolt.py compare`.
   Secondary finding from the find_bit oracle run: GCC vectorizes
   `make_sparse_bitmaps` + the scan loop with AVX2 where LCCC stays
   scalar — candidate for the vectorizer cost-model work.
3. A6: measure dropping Mov/FMov src-side def over-recording (potential
   extra acc substitutions).
4. F4: re-strengthen `check_store_alu_cross_join.sh` beyond the registry
   grep (P2).
5. aarch64 execute suite + i686 torture under the inliner growth budget
   on hosts that have QEMU/multilib CI parity (the 3 skipped local
   gates).
