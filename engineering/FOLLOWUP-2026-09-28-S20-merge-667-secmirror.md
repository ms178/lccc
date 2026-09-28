# S20: merge onto upstream main (PR #667) + %rcx shadow-epoch mirror (2026-09-28)

Sequel to `FOLLOWUP-2026-09-28-S19-rebase-660-r2-redteam.md` (S19). Upstream
landed PR #667 (PF-TLS-1 + CC-O0CALL-1: the per-argument
`previous_arg_wrote_rcx` deferred invalidate) at `81055cab`. This session
re-anchored the delta there, adopted the canonical fix, dropped what the
evidence falsified, added the one hole upstream's fix does not cover
(F-CC11), and built the %rcx shadow-epoch validator (the SEC mirror of the
S19 %rax machinery). All verdicts below are executed; proofs are inline.

## Merge surgery (executed)

- Base: `81055cab` (PR #667 merge). Our WIP overlapped upstream in 4 files
  (ci.yml, ci_local.sh, traits.rs, memory.rs) — a merge, not a replay.
- **Adopted upstream's per-arg fix, dropped our phase hammer.** Our
  `invalidate_all` at call-phase boundaries (traits.rs) cannot catch the
  intra-Phase-3 park→clobber→consume and costs legitimate cross-phase SEC
  hits; upstream's deferred per-argument invalidate is the scalpel. Both
  traits.rs widenings reverted (file now byte-identical to upstream).
- **memory.rs triage:** guards #1 (add_offset, no x86-64 callers), #2
  (alloca_addr_to, i686-only callers) and #4 (copy_addr_to, rdi/rsi-only
  today) dropped as dead/speculative; #3 (acc_to_secondary, live on the
  shared over-aligned-GEP + memcpy-staging path) kept with an S20 comment.
- **F-CC11 (new, narrow, real):** Phase-2 dynamic stack realignment
  (`movq %rsp,%rcx` + the %rax modulo chain) overwrites BOTH cached
  registers with no exit invalidate, and upstream's fix covers only
  intra-Phase-3 writes. Fixed with `invalidate_all()` INSIDE `if dyn_align`
  (calls.rs) — non-realigned calls pay nothing, not even a metadata store.
  Trigger needs over-aligned dynamic realignment + a live park + a Phase-3
  consume; proven uncovered by exhaustive audit of the Phase-2/3 regions.
- Full upstream audit of the fix's soundness (FloatReg/StructSseReg %rcx
  freedom, Stack-class loop skip, F128 staged arms, SplitRegStack
  x86-never, post-loop clobber healing): all closed, `_=>false` sound.

## Tooling forensics: parallel same-file edits race (rule)

Two `edit_file` calls to traits.rs and four to memory.rs, issued in
parallel blocks, produced cross-contaminated files (a stray 8-line test
duplicate at traits.rs:3208 that broke the build, a stray `ert!(` fragment
at memory.rs:4479, and two silently-dropped hunks). Cause: read-modify-write
race on the same file. RULE: never parallelize edits to one file (one edit
per file per block), verify every edit with `git diff` immediately, prefer
asserted python replacements for surgical work. All damage found by diff
review and repaired; `rustfmt --check` parses every touched file.

## SEC mirror: %rcx shadow-epoch validator (the deliverable)

The secondary cache (`movq %rcx,%rax` fast path, no slot fallback) now has
the same mechanical enforcement the ACC cache got in S19:

- **Classifier** (`classify_rcx_line`, common.rs): shares the prefix chain,
  operand splitter and both-operand/last-two tables; differs where the ISA
  differs. Implicit writers: call/syscall/sysenter/cpuid/rdtscp/loop-family
  (NOT div/idiv/mul/lods/cmpxchg/rdtsc/xgetbv/rdmsr/cwtl/cmpxchg8b/fstsw —
  all %rax-only). `rep*` on string-op bases (movs/stos/cmps/scas/ins/outs/
  lods) writes the counter; `rep ret`/`rep nop` fall through (ignored hint).
  One-operand mul/div/idiv and one-operand imul report FALSE (their explicit
  operand is the source; the dest is the implicit rax:rdx pair) — mirrored
  inversely from the %rax side. `bt` reads (false); `bts/btr/btc` write
  (generic dest-last true). x87/jump/prefetch never write %rcx.
- **Scan** (`debug_scan_tail`): both epochs ride ONE line walk; the
  unclassified vocabulary is the union of both classifiers. New
  `last_rax_write`/`last_rcx_write` records name the actual clobberer in
  assertion context (closes the triage gap where `last_unclassified` is
  None for a plain classified write — wired into BOTH messages).
- **API** (state.rs): `park_sec` + `sec_has_verified` (mirror of
  `park_acc`/`acc_has_verified`, same fail-loud message shape naming value,
  epochs, clobbering line and vocabulary). Release-inert by construction
  (cfg'd fields/bodies; pure classifier uncalled in release).
- **Migration (x86-64):** the 1 park (emit.rs) + all 6 consumes (5 emit.rs,
  1 calls.rs) moved to the verified API; zero raw uses remain on x86-64.
  i686 keeps raw `set_sec` x4 (producer-only: no `sec_has` consumer exists)
  and RISC-V keeps its raw `sec_has` (foreign ISA, scan inert) — documented
  policy in `park_sec` docs, not an oversight.
- **Tests (7):** 3 pure classifier tables (release-safe), 1 shared-scan
  integration (epoch independence + both-operand + last-write records), 3
  validator (unstaged-clobber fire with catch_unwind, repark healing,
  opt-in inertness). Scan/validator mods are `cfg(all(test,
  debug_assertions))` (fastbuild inherits release — the plain `cfg(test)`
  broke the build; matches the ACC mods' gating).

## Validation (executed)

- `cargo test --profile fastbuild --all-targets`: **3853 passed, 0 failed,
  0 warnings** (3 pure RCX tests green).
- Same with `debug-assertions=true` (CI's slow-gate overrides): **3867
  passed, 0 failed** — all 7 SEC tests green, ZERO validator fires across
  the suite (no stale-SEC in existing codegen), 0 warnings.
- `cargo clippy --all-targets`: exit 0, zero warnings. `cargo fmt --check`:
  clean. Targeted retest after the lods totality fix: 3/3.
- Upstream 8-shape test + 20-arm gate
  (`check_call_secondary_cache.sh`, 5 opts x 4 modes): **20/20 PASS** at
  all of -O0/-O1/-O2/-O3/-Os.
- `ci_local.sh --fast`: 89 passed, 7 failed -> all 7 triaged and rerun
  green with the EXACT gate commands (see below). Clippy gate PASS in-CI.
- A/B (5 benchmark programs, -O2 .s, baseline 6f8ace9c vs this tree):
  **byte-identical** (822/822 instructions) — the fix + hardening are
  codegen-neutral off the buggy shape (no-regressions proof).

## CI triage: 2 mine (1 root cause, fixed) + 5 environmental (fixed)

- **ci.yml block-scalar break (mine).** The carried S19 hunk had its comment
  continuation line at column 0 (dedented somewhere in the S20 carry chain;
  S19 was green so the damage is S20's). Inside a `run: |` literal block
  that terminates the scalar -> YAML parse error -> `ci-gate-parity` AND
  `ci-asm-diff-parity-self-test` (which parses ci.yml, 4 errors) failed.
  One-line fix (re-indent), `YAML-OK`, both gates rerun PASS.
- **Missing 32-bit toolchain (environmental, 5 gates).**
  `nocfi-peephole-parity`, `reassoc-latency` (-m32 arms), `map-i64-two-lane`
  and `linker-suite` (i386_*) failed on missing `bits/*.h`; `i686-integer-
  isa-parity` on `cannot find -lgcc`. Installed `gcc-multilib`,
  `libc6-dev-i386`, `g++-multilib` (sudo works; needed `apt-get update`
  first). Linker suite needed a second pass for 32-bit C++ headers
  (`bits/c++config.h`): final **277/277 PASS**. All rerun green.

## Decisive experiment: our test is a REAL repro (vacuity FALSIFIED)

S20 theory said `cc_o0call1_stale_sec.c` was likely vacuous (passes
everywhere). Baseline 6f8ace9c built pristine in a worktree (+2 staged test
files) and run:

- baseline + our test: **SIGSEGV (139)** — predicted PASS. FALSIFIED.
- baseline + upstream test: SIGSEGV (139) — predicted FAIL. Confirmed.
- our tree + upstream test: exit 0. Confirmed fixed.

Attribution (not just correlation): baseline -O0 emits `movq %rax,%rcx`
(arg3 staging clobbers the parked &u) then `movq (%rax),%r8` (byval union
through the dead register; the `movq %rcx,%rax` SEC consume was folded by
the -O0 peephole into the staging move). Fixed tree: `movq -8(%rbp),%r8`
(slot rematerialization via the deferred invalidate). The test's shape
(16-byte INTEGER byval, %r8+stack split, after a scalar %rcx clobber) is
distinct from all 8 upstream shapes (trailing 8-byte union always) — KEPT,
with the attribution comment rewritten (the old comment described the
dropped phase hammer; it now documents the per-arg fix + validator).

## Closed without code: T1/T4/T8 + F-CC3/4/5

The S20 battery's open items (shape anomalies T1/T4/T8 incl. the T8
float-spill `movaps` hang confusion, and fix candidates F-CC3/4/5) do not
reproduce on the fixed tree: all 8 shapes pass at all opts/modes, the
validator is silent across the whole unit suite, and the only red observed
anywhere traced to the ci.yml typo (fixed) or the sandbox toolchain
(fixed). T8's hang was already identified as a red herring (interrupted
tool call, and `push %rbp; mov %rsp,%rbp` prologue shape, not a codegen
hang). Class covered by: upstream per-arg fix + F-CC11 + #3 + SEC validator.

## Open (next session)

- Perf campaigns (S13 "beat every compiler" bar): this session proves
  no-regressions; wins are future work (A/B harness: baseline worktree +
  `-S` diff pattern above, seconds per program now that multilib is in).
- `cargo test` (dev profile, asserts on) locally: CI covers dbgassert via
  the slow gate; a local dev-profile run is cheap insurance if time allows.
- i686 SEC consumers: if a `sec_has` consumer is ever added there, migrate
  its 4 parks to `park_sec` (the flag already arms i686).
- Snapshot discipline held: worktree removed post-experiment; patch below
  regenerated after every validated step.
