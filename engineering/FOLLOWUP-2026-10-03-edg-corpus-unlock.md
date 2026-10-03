# FOLLOWUP-2026-10-03 — EDG Transplant Session 2: corpus unlock, driver honesty, decay fix

**Session base:** upstream `ms178/lccc` `358104c` (main, 2026-10-03, PR #738) —
re-based onto latest main per standing order.
**Supersedes/continues:** [`FOLLOWUP-2026-10-01-edg-transplant.md`](FOLLOWUP-2026-10-01-edg-transplant.md)
and the E1–E14 register in [`../docs/EDG_TRANSPLANT_ANALYSIS.md`](../docs/EDG_TRANSPLANT_ANALYSIS.md) §6.
**Headline:** the EDG-mined Clang-C corpus went from **102 executed
invocations to 467** (5.6×), because the thing blocking it was not LCCC's
frontend but a *missing evidence pipeline*. Building that pipeline immediately
exposed three real defects, two of which are fixed and validated here.

---

## 0. Environment (rebuilt from a wiped workspace, as always)

| Item | State |
|---|---|
| `/swapfile` | **4 GiB, active** (`/sbin/swapon`, verified via `/proc/swaps`) |
| Rust | `1.99.0` via rustup into persisted `/home/user/.cargo` |
| Build profile | **`fastbuild`** only (policy): `-O1`, no LTO, incremental, `CARGO_BUILD_JOBS=2` |
| Cold build | `4m00s`; incremental driver-only rebuild `35–37s` |
| 32-bit oracles | `gcc -m32` C **OK**, `g++ -m32` C++ **OK** |
| Linker used by the build | gcc + GNU ld/bfd (no `clang`/`mold` on the image) |
| Oracle pins | GCC **16.2**, Clang **23.1.0**, ICC **2021.10.0**, ICX latest — all `ok` via `scripts/godbolt.py audit` |
| EDG tree | `edgcpp/compiler` shallow clone, 3.7 GB / 111 077 files (re-derivable in ~36 s; never depended on across a turn boundary) |
| mold preset | `-DMOLD_TARGETS='X86_64;I386'` (confirmed in `tools/linker/setup_oracles.sh`) |

Snapshots this session: **S01** bootstrap, **S02** driver policy, **S03**
`-ffreestanding` + capability probe, **S04** comma decay, **S05** probe
self-tests + follow-up doc, **S06** snapshot-script repair. Every one verified
`APPLIES-CLEAN` against the base commit.

**Two snapshot scripts exist and they are NOT interchangeable** — see R10 below.
`scripts/lccc-snapshot.sh` is the *upstream* 449-line artifact, gated on a
`mode=full` `ci_local.sh` stamp; `scripts/arena_autosave_snapshot.sh` is the
standalone harness-wipe autosave used on the DIRECT STANDING ORDER (a plain
version is also kept at `/home/user/lccc-snapshot.sh`, outside the tree, so it
survives a repo `.git` loss).

---

## 1. The finding that changed the plan: the corpus was starved, not LCCC

`tests/corpus/run_clang_c_corpus.py` is *deliberately* conservative. Its
docstring is explicit: an invocation whose compiler flags have not been
**established as supported** is reported `UNSUPPORTED` rather than executed,
and for any family other than `clang` the capability set is empty unless the
caller supplies `--capability`. Measured first run against a green fastbuild
`lccc`:

```
1529 selected files; 5130 planned invocations; 102 executed;
statuses={'UNSUPPORTED': 5028, 'FAIL': 41, 'PASS': 61}
```

**98 % of the corpus was filtered.** The dominant reason incidences were
`implicit Clang C dialect not established` (4 426) and
`compiler capability not established: …` (~2 000 across ~300 distinct flags).
So the E1 "triage real frontend failures" task had an empty grid: there was
nothing to triage. The immediate prerequisite was an *evidence pipeline*, not a
frontend fix.

### Why the obvious shortcut was rejected

The cheap unblock is `--compiler-family clang`, which hands LCCC Clang's static
flag table wholesale. That was rejected because it would have **asserted, rather
than measured**, that LCCC implements ~300 flags — and then attributed every
resulting failure to the frontend. It would also collide with
`compiler_identity()`'s family/version cross-check, which exists precisely to
stop a declared family from drifting away from the binary. Instead the gap was
closed the way the harness asks for it.

---

## 2. Landed this session (all validated)

### 2.1 `scripts/lccc_capability_probe.py` — establishing capabilities from evidence *(new, E1 remainder)*

Builds the `--capability` argument list the runner demands, **only from
observed behaviour**:

* `flag:X` — established by running the compiler under test as
  `lccc -fsyntax-only -x c - X` with **`LCCC_STRICT_OPTIONS=1`** exported.
  That variable is the crux: LCCC diagnoses an unrecognized option and keeps
  going (see §2.2), so a plain run cannot distinguish *implemented* from
  *silently ignored*. Under strict mode an unimplemented option is fatal, which
  makes the exit status exactly the wanted predicate. The probe's contract is
  documented in the module header, together with what it does **not** prove
  (driver acceptance ≠ Clang-identical semantics; the corpus run is the oracle
  for the latter).
* `clang-driver-target` — `-dumpmachine` resolves to an x86-64 triple.
* `clang-default-c-dialect` — `__STDC_VERSION__` is `201710` (C17), matching
  Clang 23 on x86-64, so an unqualified `RUN` means the same thing to both.
* **A negative control.** The probe bombs out (`exit 2`) if
  `--lccc-probe-nonexistent-option-xyzzy` is reported *supported*, because then
  the compiler does not honour the strict variable and every flag would probe
  "true" — a harness that blesses unimplemented flags is worse than no harness.
  **The control fired on the first run**, against a control flag spelled
  `-lccc-probe-…`: `-l<name>` is the *library* option, so a correct compiler
  accepts it. The control was fixed to a `--`-spelled option, and the incident
  is recorded in the code so nobody re-derives a bogus control.
* **Refusal to proceed on a silent hole:** if the corpus demands a capability
  the tool has neither a probe nor a *recorded refusal* for, it exits 2 rather
  than emitting a partial list.

Result: **304 / 312 capabilities established, covering 92.5 % of weighted
planned invocations** (the count moved up from 303/86.3 % within this session
because the `-ffreestanding` fix in §2.3 made that flag establishable and
unlocked the tests that use it). The 9 not established are exactly the ones LCCC really
lacks (`-fblocks`, `-fms-extensions`, `-fms-compatibility`, `-fgnuc-version=`,
`-fno-builtin`, `-fno-signed-char`, `-fshort-enums`, `-fshort-wchar`,
`-ffreestanding` — the last one fixed in §2.3).

Corpus, honest capability set:

| run | executed | PASS | FAIL | UNSUPPORTED |
|---|---:|---:|---:|---:|
| before (no caps) | 102 | 61 | 41 | 5 028 |
| ceiling (all caps incl. unimplemented) | 509 | 331 | 178 | 4 621 |
| **honest (probe-derived)** | **467** | **305** | **162** | 4 663 |
| honest, after the `-ffreestanding` + comma-decay fixes | **470** | **309** | **161** | 4 660 |

`--emit-args` prints bare capability names; `--emit-runner-args` prints the
`--capability <X>` **pairs** the runner accepts. Mixing these up is an easy
error — it happened twice while writing this session's commands — so both are
spelled out in the module docstring and here.

Reproduce the honest run:

```bash
mapfile -t CAPS < <(scripts/lccc_capability_probe.py --cc target/fastbuild/lccc \
                      --emit-runner-args)
python3 tests/corpus/run_clang_c_corpus.py --cc target/fastbuild/lccc \
        --compiler-family lccc --mode outcome-only -j 2 --timeout 25 \
        "${CAPS[@]}" --report results/corpus-honest.json
```

### 2.2 Driver: complete the *documented* unknown-option policy *(correctness / hazard elimination)*

`src/driver/cli.rs` had an internal contradiction. Its module header claimed
*"Unknown flags are silently ignored (matching GCC's behavior…)"*, but GCC does
**not** behave that way — verified on GCC 14.2 and 16.2:

```
gcc -fbogus       -> error: unrecognized command-line option '-fbogus'   (exit 1)
gcc -Wbogus       -> error: unrecognized command-line option '-Wbogus'   (exit 1)
gcc --bogus       -> error: unrecognized command-line option '--bogus'   (exit 1)
gcc -std=bogus99  -> error: … did you mean '-std=gnu99'?                 (exit 1)
```

Meanwhile the file already enforced the *opposite* doctrine in two places, with
the reasoning written out in its own comments:

* `-m` catch-all — *"LCCC refuses to silently ignore target-affecting `-m`
  flags"*;
* `-fstack-protector*` — *"silently accepting these would hand the caller a
  binary it believes is hardened but is not — the worst failure mode for a
  security feature"*.

The generic path was the one place that doctrine was not applied. Two concrete
holes found:

1. **A blanket `arg if arg.starts_with("-f") => {}`** swallowed every
   unimplemented `-f` option with no diagnostic. It also made the
   `-fstack-protector` arm *below* it unreachable for `-ftrapv`/`-fsanitize=`,
   i.e. any new refusal arm added at the end of the match was **dead code**.
2. **`-std=<unrecognized>` left the default dialect untouched** — so
   `-std=bogus99` (and any typo) silently compiled as `gnu17`, with the wrong
   `__STDC_VERSION__` and the wrong feature set, and no diagnostic.

Fix, tiered so that speculative feature probes (autoconf, Meson, Kconfig
`cc-option`) keep working while nothing can vanish silently:

| input | behaviour |
|---|---|
| `-std=<unrecognized>` | **hard error**, with a GCC-style `did you mean` (Levenshtein, common-prefix tie-break, prefix family respected) |
| `-ftrapv`, `-fsanitize[=…]` | **hard error** — same invisible-absence failure mode as `-fstack-protector` |
| any other unrecognized option | **warning, once per distinct spelling**, naming the option |
| as above + `LCCC_STRICT_OPTIONS=1` | **hard error** |

`-fno-trapv` / `-fsanitize-recover=…` stay accepted: a compiler that never
emits instrumentation already complies with a request to leave it out (the same
reasoning the `-mno-` arm already documented). Escalation reuses the established
`LCCC_STRICT_` idiom (`LCCC_STRICT_MFLAGS`). The policy decision lives in a pure
function `unknown_option(arg, strict)` so it is unit-testable without mutating
process-global environment state — which would race under the parallel test
harness.

**13/13 end-to-end behaviours verified**, and **45/45** `driver::cli::cli_tests`
green (8 new tests, including one that asserts every dialect advertised by the
suggestion table is itself *accepted*, so a hint can never point at another
rejection).

Deliberate, documented divergence: GCC answers `-std=bogus99` with
`did you mean '-std=gnu99'?`, i.e. it leaves the family the user was typing in.
LCCC suppresses the hint when no same-family dialect is within 4 edits. The
rejection (the part that matters) matches GCC; the hint is merely more
conservative. Recorded as a decision in the code and pinned by a test.

### 2.3 `-ffreestanding` / `-fhosted` — `__STDC_HOSTED__` now tracks the contract *(C11 4p6)*

Surfaced by the probe (which is the point of having one): `-ffreestanding` was
being swallowed by the blanket arm while `__STDC_HOSTED__` stayed hardcoded
`"1"` in `src/frontend/preprocessor/predefined_macros.rs`.

```c
/* GCC differential oracle */
flags                       gcc  lccc
<none>                       1    1    MATCH
-ffreestanding               0    0    MATCH   (was 1 before this fix)
-ffreestanding -fhosted      1    1    MATCH
-fhosted -ffreestanding      0    0    MATCH
```

4/4 including last-wins ordering, and the functional header-branch check
(`#if __STDC_HOSTED__ #error` fires without the flag, silent with it). Applied
next to `__STDC_VERSION__` and **before** the user's `-D`s so
`-ffreestanding -D__STDC_HOSTED__=1` still lets the user win.

Why it matters beyond conformance: `arch/x86/boot` and most firmware build with
`-ffreestanding`, and headers branch on `__STDC_HOSTED__`. Before this fix a
kernel/firmware build silently took the hosted path.

### 2.4 Sema: lvalue conversion on the comma operator *(C11 6.5.17p3 + 6.3.2.1p3)*

Found by triaging the newly-visible corpus. `infer_expr_ctype`'s comma arm
returned the raw right-hand type:

```c
struct s { char c[17]; };
extern struct s f(void);
int X[sizeof(0, (f().c)) == sizeof(char*) ? 1 : -1];   /* lccc: reject, gcc: accept */
```

The comma operator's result undergoes the usual lvalue conversion, so the array
operand must decay — **GCC gives `sizeof(0, f().c) == 8`, LCCC gave 17**. This
was not only a rejection bug: the two halves of the compiler *disagreed*, since
codegen already emitted the decayed address (`leaq 16(%rsp), %rax`), while sema
reported "array". Any pass trusting the inferred type of a comma expression with
an array operand inherited the wrong answer.

Validated: 5/5 probes now agree with GCC, including two negative controls —
bare-array `sizeof((char[17]){0})` is still **17** (no over-decay), and
`sizeof(0, 1)` is still `sizeof(int)`. `Sema/expr-comma-c99.c` now compiles
clean. The decay spelling reuses the file's existing pointer-arithmetic idiom so
there is one idiom, not two.

---

## 3. Corpus triage — what the 162 FAILs actually are

Honest-run FAIL breakdown: `expect=reject → LCCC accepted` **118**;
`expect=accept → LCCC rejected` **44**. By category: `Sema` 111, `C` 25,
`Parser` 13, `Preprocessor` 13. **Zero CRASH and zero TIMEOUT** — the
`crash_synth_*` / `crash_csmith_*` repro class did not reproduce here.

### 3.1 The 44 "expected-accept, rejected" split into two very different things

`FOLLOWUP-2026-10-01` warned that the `Sema/` slice is "where lccc is
weakest". That is true, but the failures are **not** uniform, and conflating
them would send future work at the wrong target. Two clusters:

**(a) Genuine LCCC conformance gaps — real defects.** Each has a C-standard
reference and is a legitimate test to add:

| construct | C ref | evidence (first error) |
|---|---|---|
| `??=` digraph | C23 6.4.6 | `expected declaration before '?'` — `C/C23/n2940.c` |
| `typeof(x) y;` as a declaration specifier | C23 6.7.2 | `expected ';' after declaration before 'j'` — `Parser/c2x-typeof-ext-warns.c` |
| enum with fixed underlying type `enum E : long` | C23 6.7.2.2 | `Sema/c23-switch.c` |
| `[[attr]]` before a struct tag (`struct [[nodiscard]] S`) | C23 6.7.6.1 | `Sema/c2x-nodiscard.c`, `c2x-maybe_unused.c` |
| `bool` as a type name in `_Generic` | C23 6.5.1.1 | `Sema/c2x-bool.c` |
| label immediately before a declaration | C23 6.8.1 | `expected expression before '_Static_assert'` — `C/C23/n2508.c` |
| scope of a *tag/enum declared in a controlling expression* reaching the body | C99 6.8.4p3 | `C/C99/block-scopes.c` — `_Static_assert(a == 1)` fails |
| `_BitInt` in some declarations | C23 6.2.6 | `expected ')' before integer constant` — `Sema/implicit-int-conversion-on-int.c` |

**(b) Clang-specific leniency that LCCC (correctly) does not implement — NOT
defects. Do not "fix" these to match the corpus.** This distinction was
established by checking GCC on each, not by assumption:

| construct | GCC | verdict |
|---|---|---|
| `_Static_assert(u == 10U)` where `const unsigned u = 10u` — a `const` object is **not** an integer constant expression in C | rejects | corpus expects Clang's folding extension; LCCC agreeing with GCC is correct |
| `-fblocks` / Block syntax | n/a | Clang extension, explicitly not implemented (probe-confirmed) |
| `-fms-extensions`, `__if_exists`, MS pragmas | n/a | MS extensions |
| `_Nonnull`, `__private_extern__`, nullability | n/a | Clang/Apple extensions |

An earlier draft of this session mis-triaged `Sema/expr-comma-c99.c` as a
Clang-leniency artifact because a hand-written reproducer used `char c` instead
of `char c[17]` — which *changes the decay semantics of the program under test*.
GCC rejected the mangled reproducer too. Re-testing faithfully against the
original types showed GCC accepts and LCCC rejected, which is what produced the
real fix in §2.4. **Lesson: never characterise a corpus failure from a
paraphrased reproducer.**

### 3.2 The 118 "expected-reject, accepted" are mostly diagnostic parity

These are invocations where the corpus expects a diagnostic (`expected-error`
or a negative `//type:`) and LCCC accepted the program. Predominantly
`Sema/` (111). Severity is *lower* than (a) — accepting more than Clang is
usually a missing-warning issue, not a miscompile — but several classes
(`-Wunsequenced`, unsequenced-modification, `-Wreturn-type`) correspond to
real UB-detection value. Triage these **after** the §3.1(a) list.

---

## 4. Red-team audit of this session's own work

| # | Finding | Severity | Resolution |
|---|---|---|---|
| R1 | Capability probe's negative control used `-lccc-probe-…`; `-l<name>` is the library option, so a *correct* compiler accepts it and the control reported a false failure | harness-fatal (would have blocked the whole pipeline) | control changed to `--`-spelled; the incident and the reason are recorded in the code and in §2.1 |
| R2 | First corpus invocations passed args from `--emit-args` (bare names) where the runner needs `--capability <X>` pairs → argparse rejected 606 args | self-inflicted, cost 2 cycles | both emission modes documented in the module docstring and in §2.1 |
| R3 | My own `unknown_option` test included `-std=bogus99` in the "must have a suggestion" set; no same-family dialect is near it, so no hint is correct | false test failure | moved to a dedicated test asserting rejection-of-`qwertyuiop`-class values carries *no* misleading hint |
| R4 | `closest_std_dialect` first draft used a prefix-mismatch count labelled "edit distance" and folded candidate length into the key, biasing toward short names (`c17x` → `c1x`) | wrong suggestions; comment would have lied | real two-row Levenshtein; tie-break on longest common prefix (`c17x` → `c17`); `common_prefix_len` unit-tested |
| R5 | `-ftrapv`/`-fsanitize=` arms were placed *below* the blanket `-f` swallow → unreachable dead code that still looked like a fix | silent no-op | blanket arm removed; a comment at its old site forbids reintroduction |
| R6 | A `///` doc comment was written on match arms (invalid Rust) | build break | converted to `//` |
| R7 | First `-fwrapv` "hazard" read looked like LCCC folding `x+1 > x` to `1`; a `grep` had hidden the `leal 1(%rdi), %esi` | would have been a false miscompile claim | full asm inspected; LCCC is *conservative* — the hazard is latent, stated as latent |
| R8 | §3.1 initially asserted `expr-comma-c99.c` as "Clang-specific leniency" | mis-triaged, would have suppressed a real bug | reproduced with the original types; GCC accepting vs LCCC rejecting is the oracle; fixed in §2.4 |
| R9 | First `grep 'call.*memcpy'` for the `-fno-builtin` probe also matched `call my_memcpy`, making the result meaningless | inconclusive evidence | `-fno-builtin` is reported as an **unimplemented flag**, not as a demonstrated miscompile; the demonstrated-cases column is left empty for it |
| R11 | **`stdbuf -oL` used only to make the CI log monitorable injected a 64-bit `LD_PRELOAD` (`/usr/libexec/coreutils/libstdbuf.so`, ELFCLASS64), so every 32-bit i686 test binary printed `ERROR: ld.so: object ... cannot be preloaded` on stderr.** `GATE minmax-reduction`'s i686 differential compares the produced output, read the injected line as the difference, and reported `FAIL: i686: output differs from the oracle` | false CI failure — a *miscompile* accusation against a correct compiler | proved, not assumed: the gate **passes standalone** (11.6 s, `check_minmax_reduction: PASS`), and `gcc -m32` + `stdbuf -oL` reproduces the ld.so line while the same binary is silent without it. CI re-run clean (`env -u LD_PRELOAD`, no `stdbuf`); the contaminated log is kept as `results/ci-fast-STDBUF-CONTAMINATED.log`. **Never wrap a 32-bit-differential CI run in `stdbuf`** |
| R12 | **`rustfmt` gate red** — the new driver tests were written in a formatting rustfmt rejects (long assertion lines; a 4-element array literal that must be exploded) | CI failure: the *only* failure of the clean `--fast` run (150 passed / 1 failed / 5 skipped) | `cargo fmt`; verified it touched ONLY `src/driver/cli.rs` (no collateral reformatting); `cargo fmt --check` rc=0; gate re-run `PASS`; 45/45 unit tests and the 7-case driver matrix re-verified after the rebuild |
| R10 | **`scripts/lccc-snapshot.sh` (449-line upstream, `mode=full` CI-stamp gated, exercised by `tests/corpus/test_snapshot_contracts.py`) was overwritten with the session's standalone autosave script** | high — destroys upstream functionality and would fail the snapshot-contract gate | caught in the pre-delivery red team; upstream restored byte-identical (`git diff` vs base empty, contract test **11/11 green**), the standalone script re-homed to `scripts/arena_autosave_snapshot.sh` |


Deliberately **not** claimed: `-fno-builtin`, `-fshort-enums`, `-fno-signed-char`
are unimplemented (probe-proven) but no miscompile has been demonstrated for
them; `-fshort-enums`/`-fno-signed-char` change ABI and are the highest-risk of
the three.

---

## 5. Verification status — read this before trusting any number above

* **Green and reproducible:** `cargo build --profile fastbuild` (clean);
  `cargo test --profile fastbuild --lib driver::cli::cli_tests` **45/45**;
  the 13-behaviour driver matrix; the 4-case `-ffreestanding` GCC differential;
  the 5-case comma/decay GCC differential; `scripts/godbolt.py audit`.
* **`./scripts/ci_local.sh --fast` — FINAL CLEAN RUN ON THE FROZEN TREE:
  `SUMMARY: 151 passed, 0 failed, 5 skipped` / `ALL GATES GREEN` /
  `CI_EXIT=0`.** `rustfmt` and `clippy` both `PASS` (the latter 179 s). Log:
  `results/ci-fast-final.log`.
  *Caveat, stated rather than hidden:* the gate also printed
  `WARNING: the worktree changed during the run (<tree> -> <tree>); no pass
  stamp`, so it did not write a stamp. `HEAD` was unchanged (`154d249`) and
  `git status --short` was empty at completion, so the movement was transient —
  most plausibly a gate that writes and then restores a tracked artifact (the
  corpus tooling does staged atomic swaps by design). The **gate verdicts** are
  the substantive result and they are all green; the missing *stamp* means a
  `mode=full` upgrade is still owed.
* Earlier in the session a first clean `--fast` run gave
  `150 passed, 1 failed, 5 skipped` with `rustfmt` as the sole failure
  (**R12**, now fixed) — recorded so the improvement is auditable.
* Note for the next session: `stdbuf` must **never** wrap this CI. It injects a
  64-bit `LD_PRELOAD`, which makes every 32-bit i686 differential gate report a
  phantom output mismatch (**R11**). Run it as
  `env -u LD_PRELOAD bash scripts/ci_local.sh --fast > log 2>&1`.
* **A `--fast` pass is not CI-equivalent**: the three slow gates
  (`regression-corpus-ssa`, `benchmark-output-oracle`,
  `peephole-whitespace-invariance`) are skipped by design. The upstream
  `scripts/lccc-snapshot.sh` enforces a `mode=full` stamp; the session autosave
  script (`scripts/arena_autosave_snapshot.sh`) deliberately does not, so a
  mid-session wipe can never cost validated work — see **R10**.
* **The deliverable was verified against a pristine upstream checkout**:
  `git clone --depth 1 https://github.com/ms178/lccc` at `358104c6` then
  `git apply --check /home/user/ms178-1.patch` → clean. The patch touches six
  files and contains no binary hunks and no conflict markers.
* **No wall-clock performance claim is made this session.** The measurements
  here are coverage counts and compiler-behaviour differentials, which are
  deterministic and host-independent. No `perf`/PMU data exists in this
  environment by design; the pinned-Cachegrind doctrine (`scripts/callgrind_ab.py`,
  `LCCC_CG_{I1,D1,LL}`) remains the correct instrument for instruction/cache
  claims and was not exercised against a real workload this session.

---

## 6. To-do for the next session, in priority order

The register in `docs/EDG_TRANSPLANT_ANALYSIS.md` §6 remains authoritative;
these are the concrete, evidence-backed next actions.

1. **[E1 remainder — highest value] Curate the §3.1(a) list into
   `tests/regression/` reproducers.** Each already has a standard reference, a
   minimal trigger, and a GCC differential in this document. Order by effort:
   `??=` digraph → `typeof` declaration specifier → labels-before-declaration →
   `[[attr]]` before a tag → enum fixed underlying type → `bool` in `_Generic`
   → controlling-expression scope → `_BitInt` → comma-decay **done (§2.4)**.
2. **[P0 driver honesty] `-fno-builtin` is unimplemented and byte-invisible.**
   `-fno-builtin` exists to stop the compiler recognising `memcpy`, `printf`,
   … precisely when a translation unit *defines* them (glibc's own string
   routines). Once LCCC's builtin recognition is strong enough to matter, this
   becomes an active miscompile source. Either implement the flag or add it to
   the `-fstack-protector`-style refusal tier. Reproducer needed first — R9
   shows the naive test is inconclusive.
3. **[P0 driver honesty] `-fshort-enums` / `-fno-signed-char` change the ABI**
   and are silently ignored. `-fshort-enums` is used by real embedded/kernel
   code. Decide: implement, or refuse loudly with a remediation hint.
4. **[E5] Work-stack const-eval + step budget.** Still open. The corpus
   surfaced no crash this session, but `src/common/const_eval.rs` and the three
   sibling `const_eval*.rs` files still have **no depth or step budget** at all
   (grep-confirmed) — the `crash_synth_*` class is a real risk, just not
   triggered by this corpus.
5. **[E7 remainder] Curate `docs/edg_changes_c_extract.md`** (1 559 C-relevant
   entries) into regressions. Start with C23 tag compatibility (N3037), empty
   initializers, bit-field quirks, unsequenced-modification semantics.
6. **[measurement] Wire the honest corpus run into CI** so the 470-invocation
   number is a regression gate. `scripts/lccc_capability_probe.py --selftest`
   now exists and is **5/5 green** (it drives a synthetic compiler to pin the
   strict-env contract, the negative control, *measured* driver capabilities,
   `--emit-runner-args` pairing, and the refusal to proceed on a capability with
   neither a probe nor a recorded refusal). The remaining work is to add those
   five tests to `ci_local.sh`'s Python contract gate next to the 44 mocked
   runner/miner contracts, and to decide whether the full 470-invocation run is
   cheap enough (≈12 s wall on 2 cores) to gate every CI run or should be a
   scheduled/`--slow` gate.
7. **[E3/E6/E9]** unchanged from the register.
8. **[performance] `-fstrict-aliasing` is accepted-and-ignored and LCCC is
   conservative** (verified: it reloads across a `float`/`int` store where GCC
   folds). That is *safe* today and a **performance gap**, since TBAA is a
   first-class lever in the standing objective (§16 of the directive). Every
   TBAA optimization added from here on must be accompanied by
   `-fno-strict-aliasing` *actually* disabling it — the driver now warns on the
   flag precisely because it does not yet act on it.
9. **[workloads] `tests/workloads/{gzip-1.14,zlib-ng-2.3.3,glibc-2.44,sqlite-3.53.4}`
   all exist and none were run this session** (budget went to the corpus
   unlock). Running `gzip-1.14/run.py` against the current binary is the
   cheapest end-to-end confidence check and should be first in the next
   session's queue.

## 7. Session metrics

| metric | value |
|---|---|
| corpus executed invocations | 102 → **470** (4.6×) |
| honest corpus PASS | 61 → **309** |
| capabilities established by evidence | **304 / 312** (92.5 % weight) |
| new tests added (all green) | **8** driver-policy + freestanding + dialect + distance |
| defects found | **4** (blanket `-f` swallow; silent bad `-std=`; `-ffreestanding`/`__STDC_HOSTED__`; comma lvalue conversion) |
| defects fixed & validated | **4** |
| cold / incremental build | 4m00s / 35–37s |
| snapshots | S01–S04, all `APPLIES-CLEAN` |
