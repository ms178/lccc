# Session 85 — Red-team audit of "Agent Z" kernel-bringup patch (ms178-1.patch.txt)

**Base audited:** `ms178/lccc` main @ `be227056` (PR #251, 2026-08-26).
**Patch:** 974 lines, 23 files (Linux-kernel bringup for the ms178 6.18.46 Cachymod kernel).
**Method:** content-level diff vs main (the blob hashes in the patch do not exist in any
fetched ref — Agent Z's base was a dirty worktree — so every hunk was re-derived against
main), plus empirical reproduction of every claimed defect with the freshly built
`target/fastbuild/lccc`, plus reference-compiler cross-checks with GCC 14.2.0 (Debian) and
GAS/binutils, plus kernel v6.18 source cross-checks (Makefile CC_FLAGS_FTRACE logic).

---

## Verdict table

| # | Hunk | Verdict | Action |
|---|------|---------|--------|
| 1 | `scripts/prepare_kernel_tree.sh`: tolerate tar rc≤2 | **AGREE w/ strengthening** | Adopt + sentinel-file post-check |
| 2 | `generation.rs`: `invalidate_text_section()` after PFE section | **AGREE (bug is REAL on main)** | Adopt verbatim |
| 3 | `generation.rs`+`state.rs`+`mod.rs`+`emit.rs`+`cli.rs`+`pipeline.rs`: mcount `-pg/-mfentry/-mrecord-mcount/-mnop-mcount` | **AGREE w/ 3 fixes** | Adopt fixed (naked guard, classic-mcount placement, test update) |
| 4 | `generation.rs` BinOp Add → `emit_leaq_sym_index` fold (per_cpu_ptr) | **AGREE** | Adopt (mirrors upstream GEP fold, PR #245) |
| 5 | `traits.rs`: `MIN_JUMP_TABLE_CASES` 4→5 | **AGREE, comment REWRITTEN** | Adopt with corrected rationale |
| 6 | `gp_integer.rs`: `movabs $sym_a-sym_b,%reg` (size 8) | **AGREE (needed, reproduced)** | Adopt |
| 7 | `parser.rs`: `parse_sym_addend`-first probe for `sym - (. + 4)` | **AGREE (bug REAL, silent zero reproduced) + probe GUARD required** | Adopt with paren-only guard (unconditional probe flipped trailing-addend signs — broke kernel_altinstr_layout) + 2 unit tests |
| 8 | `emit.rs`: `LCCC_BEST_EFFORT_NO_HOME` zero-fabrication mode | **DISAGREE — REJECTED** | Hard gate stays |
| 9 | `macro_defs.rs`: `_Pragma` rewrite | **OBSOLETE on main** | Drop (main already rewrote it) |
| 10 | `no_instrument_function` plumbing (ast/parse/decl/module/lowering) | **AGREE (required by #3)** | Adopt |
| 11 | struct-literal updates (gvn/inline/outline_switch/redundant_loads/pgo×3) | **AGREE (mechanical)** | Adopt, indentation fixed |
| 12 | `cli.rs` test update (comments only) | **DISAGREE — BROKEN** | Rewritten: mcount flags now assert ACCEPT |

---

## Evidence log

### #2 PFE section leak — reproduced on pristine main

`lccc -O2 -fpatchable-function-entry=2,0 -S` on a 2-function TU emits BOTH function
bodies inside `__patchable_function_entries,"awo"` (writable, linkonce, not executable):
no `.text` re-selection happens because the raw `.section` directive does not update
`current_text_section`, and `emit_switch_to_section()` early-returns when the requested
section equals the stale cached one. GCC reference: `.section …,.LPFE0` → `.quad .LPFE0`
→ `.text` → body. Any kernel/user build that passes `-fpatchable-function-entry`
currently produces non-executable function bodies. **Real, severe, still present.**

### #3 mcount family — needed, GCC semantics verified empirically

* main refuses `-pg` outright (`cli.rs:1505`): kernel must set
  `CONFIG_FUNCTION_TRACER=n`. Cachymod 6.18 ships it ON.
* Kernel v6.18 top Makefile: `CC_FLAGS_FTRACE := -pg`; `+= -mrecord-mcount` when
  `CONFIG_FTRACE_MCOUNT_USE_CC`; `+= -mnop-mcount` if `CONFIG_HAVE_NOP_MCOUNT` and
  cc-option passes; `+= -mfentry` if `CONFIG_HAVE_FENTRY` (x86_64 selects it). x86 also
  selects `HAVE_OBJTOOL_MCOUNT`/`HAVE_OBJTOOL_NOP_MCOUNT`. Agent Z's flag contract
  (−pg is the trigger; the −m sub-modes are inert without it) matches the kernel's
  `CFLAGS_REMOVE_… = -pg` VDSO pattern exactly. **Correct design.**
* GCC 14.2 reference shapes (measured):
  * `-pg -mfentry`: `call __fentry__` is the FIRST instruction (before prologue).
  * `-pg -mfentry -mrecord-mcount [-mnop-mcount]`: site label + `call`/5-byte NOP,
    then `.section __mcount_loc,"a",@progbits` + `.quad 1b` + `.previous`.
  * `-pg` (classic): frame IS set up first (`push %rbp; mov %rsp,%rbp`), THEN
    `call mcount`. `-pg -fomit-frame-pointer` is rejected by GCC.
* Agent Z defects found (fixed in our integration):
  1. Guard `!func.is_inline && !func.no_instrument` misses `!func.is_naked` although
     the comment claims naked is skipped → naked functions would get an entry call.
  2. Classic `call mcount` before the prologue violates the mcount frame ABI
     (GCC measures: call comes after frame setup). Fixed via a deferred prologue hook
     + forced frame pointer in classic mode.
  3. The cli refusal test `unimplemented_hardening_is_refused_not_ignored` still
     asserts `try_flag("-pg").is_err()` → patch as given FAILS `cargo test`.

### #4 Add-fold — consistent with upstream direction

main already folds the identical shape for `GetElementPtr` (PR #245, commit 804ce8c:
"per_cpu()-style GEP … Fold the symbol base into a SIB leaq directly"). Agent Z extends
the same mechanism to `BinOp Add` (the `per_cpu_ptr()` Cast+Add shape). The guard set
(`Add`, non-float, size 8, `global_addr_map` hit, `rip_rel_blocked` check) mirrors the
GEP path; fallback to `emit_binop` unchanged. The minimal C repro does not ICE on main
(the ICE needs the exact RA state of `workqueue_prepare_cpu`), but the fold is
semantically equivalent and closes the documented producer/consumer gap class.

### #5 MIN_JUMP_TABLE_CASES 4→5 — GCC parity PROVEN, objtool rationale WRONG

Measured with GCC 14.2 -O2, side-effecting dense switches:
* 4 cases → compare chain (NO table); 5 cases → jump table; 6+ → jump table.
So the effective GCC x86 threshold is 5 — Agent Z's number is right (matching
`targetm.case_values_threshold()` on i386). But their comment claims objtool "cannot
follow the lea+movslq+add+jmp*%reg pattern" — that is false: the kernel is built with
GCC 4-case… no, with GCC's own 5-case tables and objtool validates them daily. The
correct rationale is GCC-parity + small-N branch-prediction economics. Also noted: the
constant is shared by the i686/ARM/RISC-V switch lowering; GCC's generic default is 4,
so this is a documented benign deviation there (4-case chains are never wrong, only a
different size/speed point).

### #6 movabs — reproduced

`movq $(.Lend - start), %rcx` fails on main: "symbol-difference mov immediate only
supported at 32-bit width". Needed by `arch/x86/mm/mem_encrypt_boot.S`. Encoding
(REX.W + B8+rd + R_X86_64_64 diff reloc + 8-byte addend) is correct.

### #7 static_call `.long func - (. + 4)` — reproduced SILENT ZERO

main: assembles with rc=0 and emits `00000000` (no relocation). GAS reference:
`R_X86_64_PC32 target_fn-4`. The naive ` - ` split grabs `(. + 4)`; the rfind probes
fail (`(."` is not label-like); the fall-through pushes `SymbolDiff("func", ". + 4")`
which the writer cannot resolve → silent zero. This is the worst failure class
(misassembly, no diagnostic). Agent Z's fix (probe `parse_sym_addend(rhs_full)` first,
which strips parens and yields `(".", 4)` → `SymbolDiffAddend(lhs, ".", lhs_addend-4)`)
is correct; the downstream writer already handles `a - .` diffs (proven by the passing
`asm_quad_pcrel_key.c` regression).

### #8 LCCC_BEST_EFFORT_NO_HOME — REJECTED

Fabricates `xorl %eax,%eax` for any value without a home, behind an env var. Violates
the correctness-first charter: silent miscompiles in any build whose operator happens to
set the variable; hides RA bugs instead of fixing them; the motivating case
(`workqueue_prepare_cpu` value 65) is addressed at the root by hunk #4. Hard gate stays.

### #9 macro_defs.rs `_Pragma` — obsolete

main's `handle_pragma_operator` already implements balanced-paren scanning with string
skipping + C11 §6.10.9 full macro replacement of the argument (doc comment explicitly
cites the kernel `__diag_str` case). Empirical: the `__diag(push)` reproducer compiles
and runs on pristine main. Hunk dropped.

### #1 prepare_kernel_tree.sh — agree with strengthening

The premise (GNU tar rc=2 on benign "directory renamed" metadata races under load) is
plausible, but tolerating exit-2 with only a top-level `Makefile` check is weak — a
corrupt partial tree that still has `Makefile` would fail later with confusing errors.
Strengthened with a sentinel-file integrity check (Makefile, init/main.c,
arch/x86/Makefile, kernel/sched/core.c, include/linux/sched.h) before proceeding.

## Validation performed this session

* fastbuild (`scripts/build_lccc_fast.sh`, -O1, -j2) clean; warnings-as-errors.
* `cargo test --lib`: 1197 tests, 1191 passed, 0 failed, 6 ignored —
  including NEW `test_static_call_parenthesized_dot_addend`,
  `test_diff_trailing_addend_sign_preserved`, `mcount_flag_family_parses`,
  and the rewritten `unimplemented_hardening_is_refused_not_ignored`.
* New regression scripts: `tests/regression/check_{pfe_section,mcount,static_call_pcrel,movabs_symdiff,switch_threshold,percpu_add_fold}.sh` — all PASS.
* `scripts/run_regression_suite.sh` full sweep: **PASS=467 FAIL=0 SKIP=11**,
  AB-diff failures 0.

### Red-team catch during integration (important)

The FIRST version of hunk #7's probe (applied unconditionally, as Agent Z
proposed) REGRESSED `kernel_altinstr_layout`: `.long 760b - 770b + 5` was
read as `760b - (770b + 5)` — the trailing addend flipped sign
(neg_addend came out -11 instead of -1). Root cause: `parse_sym_addend`
swallows a `sym ± N` tail even when the parens are absent. Fix: the probe is
gated on `rhs_full.starts_with('(')` (the shape it exists for) and rejects
digit-only "symbols" (`a - (1 + 2)` must not diff against `1`). Both shapes
are now unit-tested. This is exactly why the audit builds and runs the suite
instead of reviewing prose.

### Cross-compiler evidence collected

| Claim | Evidence |
|---|---|
| GCC x86 jump-table threshold = 5 | GCC 14.2 -O2: 4-case side-effecting switch → compare chain; 5/6-case → `jmp *%rax` table |
| Classic `-pg` needs the frame | GCC 14.2: `push %rbp; mov %rsp,%rbp` BEFORE `call mcount`; `-pg -fomit-frame-pointer` → hard error |
| fentry shape | GCC 14.2: `call __fentry__` as first instruction |
| record/nop shape | GCC 14.2: `.section __mcount_loc,"a",@progbits` + `.quad 1b`; NOP = `0f 1f 44 00 00` |
| static_call site | GAS: `R_X86_64_PC32 target_fn-4`; lccc pre-fix: silent `00000000`, post-fix: identical reloc + runtime pointer-exact |
| kernel flag contract | linux v6.18 Makefile: `CC_FLAGS_FTRACE := -pg` + `-mfentry` (HAVE_FENTRY) + `-mrecord-mcount` (FTRACE_MCOUNT_USE_CC) + `-mnop-mcount` (HAVE_NOP_MCOUNT + cc-option) |
