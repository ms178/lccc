# 2026-08-19/20 (session 20) — volatile access correctness + boot-gap accounting

**Base:** `origin/main` @ `93a09491` (Merge PR #134) — sessions 17–19 (alu
audit, magic div, cast audit) already landed upstream.
**Snapshot:** `/home/user/ms178-1.patch` (tags `S01-alu-audit-prep`,
`S02-volatile-semantics`), auto-saved after every validated win.

## Landed upstream during this session (read, audited, built on)

PR #131 (i686 ALU rewrite — my audit line, session 17), PR #132 (const
div/rem strength reduction — session 18), PR #133 (pointer↔float cast,
session 19), PR #134 (variadic ABI + struct alignof). All verified present
and clean after rebase; my work was rebased onto `93a09491` without
conflicts.

## Accomplished (all validated: 931 unit / 50 correctness / 120-case m32
fuzz / 20 regression scripts / alu torture MATCH O2+Os)

1. **`tests/fuzz/alu_torture_m32.py`** — permanent i686 ALU differential
   oracle (full 32-bit hash, not exit-code folding): narrow bitops with
   dirty-high inputs, f32 neg bit patterns, mul-by-const family, const
   div/rem over the signed/unsigned boundary numerators, mem/LEA/shift
   shapes. Found and fixed its own UB (ctz of truncated-zero) — no compiler
   bug; clz/ctz/popcount/bswap confirmed GCC-equal at O2/−Os.
2. **C `volatile` access semantics — the kernel-blocking correctness hole**
   (see commit message for the full fix). Reproduced first
   (`counter = 7; return counter;` folded the read), then fixed end-to-end:
   IR `Load/Store{volatile}`, qualifier flow from Declaration/ParamDecl
   through DeclAnalysis into LocalInfo/GlobalInfo, the
   `expr_access_is_volatile` predicate, LValue-carried volatility for every
   typed load/store/assign/bitfield-RMW/deref/member path, mem2reg flag
   propagation (it silently rebuilt loads with `volatile:false` — found by
   dumping per-pass IR), and gates in store_load_forward, load_forward,
   gvn, dce, licm, loop_memory_promote, and the i686 load+cmp memfold.
   `check_volatile_access_semantics.sh` locks it in at O0/O1/O2/Os.
3. **Inliner: single-call-site statics** (GCC `-finline-functions-called-once`
   parity): address-not-taken soundness scan
   (`collect_value_referenced_functions`: GlobalAddr, pointer tables, alias
   targets, constructors, inline-asm templates and `i`-constraint symbols —
   the old budget exemption had a latent miscompile hole here), DoS-bounded
   admission (4000 inst / 800 blocks) replacing the 24-block cap that kept
   display_menu/get_entry/parse_earlyprintk outlined, and a measured
   loop-nest exemption ceiling (160) that keeps the zlib-ng Adler-32 policy
   intact (262-inst body; its final-site inline was a runtime regression).
4. **i686 peephole: ALU slot-source forwarding** —
   `imull 32(%esp),%edx` → `imull %eax,%edx` (+ dead store) under the same
   window/alias/implicit-write proofs as the reload forwarding; 4 unit
   tests incl. the negative cases (src rewritten mid-window, live
   mid-slot reader).
5. **Corpus/oracle honesty + infrastructure**: realmode_corpus.sh sums ALL
   exec sections (GCC `.text.startup` undercounted main.o by 481), fails
   loudly on truncated trees, and `prepare_kernel_tree.sh` now generates
   cpustr.h (cpu.o was silently ERR) and validates the tarball with `xz -t`
   (snapshot truncation left a 28 MB stub that broke extraction).
   `arena_session_restore.sh` = one-command sandbox bring-up (swap, rust,
   apt, git remote/identity/exec-bits, kernel tree, fastbuild).

## State of the 32 KiB boot gate (measured, this session's tree)

```
setup.elf _end = 0x99c0 = 39360   (gate: 0x8000 = 32768 → 6592 over)
realmode C-object text: lccc 30950 vs gcc 13327 (2.32×)
```

Per-object excess (lccc−gcc): printf +3013, string +2306, video +2187,
cmdline +1189, cpucheck +1038, early_serial +1223, edd +927, video-mode
+979, video-bios +815, cpuflags +589, a20 +508, cpu +421 … The .S objects
are byte-identical to GCC; the whole gate gap is C-object codegen.

## Follow-up work (ranked; the identified root causes)

1. **i686 register pressure on merged/large bodies — THE structural item.**
   `set_video` (display_menu+get_entry+mode_menu inlined, GCC-parity
   structure) emits 3056 bytes vs GCC 1204: ~600 SSA values of which only
   70 are register-homed (56 in %edx for short ranges); everything else is
   define-in-%eax → store-to-slot → use-from-slot. Concrete sub-items:
   - **%eax allocatability** (the funnel: operand_to_eax/store_eax_to,
     casts, div, 8-bit ops) — per-point accumulator hazards, multi-session.
   - **Caller-saved-first ordering for leaf functions** (ebx/esi/edi push/pop
     tax: store_mode_params pays 6 pairs). The RA already prefers ecx/edx
     for non-spanning values, but hazard exclusion (ecx staging) plus
     Phase-1 callee-saved preference for anything call-spanning leaves
     leaves with spills. Measure with CCC_DEBUG_RA on the corpus.
   - **Mem-destination ALU** (`addl %reg, mem` when dest==lhs and old-lhs
     is dead) — needs use-count at codegen (detect_load_cmp_mem_fold
     already computes use_counts; same plumbing).
2. **printf number()/vsprintf (+3013)** — per-function dissection still
   open; the base-conversion loop and the flag/dispatch switch dwarf GCC's.
3. **string.c kstrtoull (+1049)** — 64-bit parse loop: __div_u64_rem should
   become the 2-divl inline (GCC inlines x86's asm/div64.h; we emit the C
   fallback loop, 217 bytes).
4. **cmdline (+1189), early_serial (+1223)** — mostly the same leaf-loop
   register tax; will move with item 1.
5. **store_mode_params partial inlining** (2 sites, GCC keeps a `.part.0`
   clone) — a general 2-site-static "inline hot site + share cold part"
   transform; design sketched in session-11 notes.
6. **Volatile phase 2** (documented in the predicate): casts to
   volatile-qualified pointer types (`*(volatile u32*)addr` — the MMIO
   idiom; needs qualifier retention in TypeSpecifier or a cast-side flag),
   volatile struct FIELDS, `int * volatile p`, volatile aggregate
   initializers.
7. **asm-level dead-load deletion audit**: IR-level DCE now keeps volatile
   loads; verify no x86/i686 peephole deletes a load whose result is dead
   (empirically green today via the regression script, but a dedicated
   asm-level check is cheap insurance).

## Environment protocol (unchanged; now scripted)

`scripts/arena_session_restore.sh [--with-kernel]` restores swap, /opt rust,
apt, git remote/identity, exec bits, the kernel tree, and the fastbuild
compiler after any harness wipe. Snapshot after every validated win
(`scripts/lccc-snapshot.sh`); ms178-1.patch is regenerated on each call.
