# Session follow-up: red-team audit of PRs #396–#400 + peephole-oracle fixes

Base: `447ea90e` (ms178/lccc main, merge of PR #400). Worktree HEAD: `5455235e`.
Snapshot: **S03-peephole-oracle-audit** → `/home/user/ms178-1.patch` (44 594 B,
APPLIES-CLEAN vs `447ea90e`; ledger + series + tarball in `/home/user/artifacts`).

Validation: `cargo test --profile fastbuild --locked -j2 --lib` →
**1854 passed / 0 failed / 6 ignored** (baseline 1842; +12 new tests).
Full regression suite → **PASS=606 FAIL=0 SKIP=15** (baseline had 1 fail).

## What was found and fixed

All fixes are in the implicit-operand oracle surface introduced by #398/#400,
plus one test-harness correction. Each fix carries a non-vacuous unit/e2e test
(verified by temporarily reverting the classification: the old model fails the
new test, the fixed model passes).

1. **Any-width vs full-width implicit writes (the core #398/#400 soundness
   hole).** The oracle classified every implicit write as if it redefined the
   whole register. On x86-64 a partial write (`movb $1,%al`, `lahf`, `xlat`,
   `divb`, `lodsb`, `fnstsw`, the conditional accumulator reloads of
   `cmpxchg*`/`xbegin`) preserves the upper bits, so treating it as a full
   redefinition let `narrow_dead_sign_extension` drop a needed `movslq`,
   `eliminate_dead_reg_moves` drop a needed `movq`, and the three-line-copy
   pattern of `local_patterns` delete a copy whose upper bits were still live.
   Fix: `implicit_full_write_refs` (the subset of implicit writes that is
   guaranteed ≥32-bit) + `writes_family_full` in `helpers.rs`, width-exact
   `is_full_write` in `relay_and_lea.rs`, `FileLiveness` now models partial
   implicit writes as *reads* (never kills), and `dead_code`'s redefinition
   acceptance switched to `writes_family_full`. Same defect class is why
   #400's x86 prologue moved its scratch register `%rcx → %r11` (upstream
   change, correct direction).

2. **`flags_effect` misclassification.** BMI1 (`andn/bextr/bzhi/blsi/blsmsk/
   blsr`) were treated as flag writers, and every SSE/AVX *data* op spelled
   `add*/sub*/mul*/div*/and*/or*/xor*` (`addsd`, `vaddps`, …) classified as
   an ALU flag writer because the table matched by mnemonic prefix. An FP op
   between `cmp` and `jcc` hid the reader and got the flag producer deleted.
   Fix: exact size-suffix matching in the WRITERS table, an FP/VEX-data
   Neutral table, and exact writers (`sahf/stc/clc/cld/std/shld/shrd/bts/btr/
   btc/ptest/popf/comiss/ucomiss`) classifying Writes.

3. **`pop`-prefix collision.** The push/pop carve-out used
   `t.starts_with("pop")`, which also matched `popcntq` — a genuine EFLAGS
   writer — so `flags_dead_after` reported flags dead across `popcntq` and
   `movq; addq; popcntq; je` folded wrongly. Fix: exact token matching
   (`t == "pop" || t.starts_with("pop ")`, likewise `push`), which also
   inherently keeps `pushf*`/`popf*` out of Neutral.

4. **`local_patterns` one-instruction flag peek.** The `movq+addq→leaq`
   rewrite checked only the immediate next instruction for a flags consumer;
   an intervening flag-neutral line hid a later `jcc`. It now uses the
   central `flags_dead_after` scan (shared, single source of truth).

5. **Oracle table row corrections.** `sysenter`/`sysexit`/`sysret` no longer
   fabricate RAX|RCX|RDX clobbers (sysenter: none; sysexit: reads RCX|RDX;
   sysret: reads RCX|R11); `cpuid` reads its `%rcx` subleaf; `xlat` reads AL;
   `pushf/popf/iret` track `%rsp`; `xbegin`/`sahf`/`fnstsw`/`int` rows added
   (`int` claims EAX but is masked to *no* full writes — conservative).

6. **`tests/regression/vec_alias_versioning.c` (+2B case).** The partial-
   overlap case dereferences a misaligned `float*` (UB). GCC exploits the UB
   and vectorizes unguarded; lccc correctly falls back to the scalar
   remainder, so the bytes legitimately differ and the oracle comparison is
   ill-posed — this was the suite's one failure. The case now stays in the
   test (it is the only exercise of the guard-fallback path) but no longer
   prints or hashes its UB-dependent bytes. Suite is green again.

## Audit findings that needed no change (reviewed, deliberate)

- **#396** guard arithmetic verified: `(dst+dst_disp) − (src+disp_min)` in
  `(0, W·e)` ⇒ `(distance−1) <u window_bytes + (disp_max−disp_min) − 1`;
  remainder is built before mutation and committed after; map vectorization
  requires IV start = const 0. IR for the guarded loop is correct.
- **#397** headline decision re-verified against live measurement with
  `scripts/uops_info_probe.py SHL_R64_CL`: 3 µops on SKL..CLX, 2 µops on
  ICL..RPL, latency 1 on AMD ZEN+ — exactly what the model's
  `prefer_shlx`/`shlx_saves_move` split assumes. The `redundant_ext` additions
  (`shlxl/shrxl/sarxl`) are width-correct (SHLX is a pure 32-bit write of the
  destination, doesn't read it, doesn't touch flags).
- **#399** tuning data carries per-field provenance (uops.info page, Agner
  Fog 2024, Intel ARK, glibc thresholds, LLVM sched) and a working verifier
  script (ran it live, correct output).
- **#400 arm64 oracle** is correct for the emitted vocabulary: the `lea`/`ls`
  carve-outs in `writeback_base` are inert dead code (no A64 mnemonic starts
  with `lea`; `ls` is a condition code, never a bare line) — left in place.
  `cas` is not emitted (atomics use ldxr/stxr loops) and the copy-prop guard
  already treats `cas` as a barrier.
- **#400 riscv oracle**: `tail` claiming a write of `ra` is conservative by
  design (tail preserves ra; the false write only blocks copy propagation
  through an opaque barrier). Accepted as documented.
- **#400 i686 oracle**: family-granular and consistent (byte div/mul forms
  partial, `cdq`/`cltd` full EDX, `rep ret`/`rep nop` → (0,0)).

## To-do / next session

- Continue the audit backlog: read the remaining unread diff chunks of #396
  and #399 for completeness (all correctness-relevant parts are read; this is
  bookkeeping).
- Godbolt/codegen-oracle comparison runs on representative workloads
  (standing goal: beat GCC/Clang/ICX; host has gcc 14.2 only — oracle uses
  GCC 16.2).
- mold i686 build preset (exact cmake option still to look up).
- Re-base on latest ms178/lccc main at session start (routine).
- After any further validated fix: `./scripts/lccc-snapshot.sh` (S04…).
