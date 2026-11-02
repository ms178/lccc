# The external review, adjudicated: verdicts, evidence, and what testing found that reading did not

An external reviewer audited this patch and returned eight findings (`F1`–`F8`)
plus a follow-up list (`T1`–`T7`). The reviewer had no ability to run anything:
every finding is an argument from reading. This document is the other half — each
finding was re-derived by running code, and the verdict below is either
*confirmed with evidence*, *confirmed with a correction*, or *rejected*. Where the
review was right, the fix is in the tree and named by file and line. Where the
review was right about a symptom and wrong about the mechanism, both are stated.
Where testing found defects the review did not, they are listed in §2 — including
four false quantitative claims in *this repository's own* documents and one bug in
the reproduction harness written to check them.

## 0. Method, and an honest note on provenance

**Re-base.** Per instruction the work was re-based on the current `origin/main`
(`56858cb`, the merge of PR #559) before anything else. Two commits sit on top of
it: `2a675b3` (46 files, +3318/−407) and `400ea84` (7 files, +1122/−8); the
delivered diff is 49 files, +4440/−415. Some fixes the review asked for had
already landed upstream in #559 (F7's named register constants are one); those are
marked as such rather than claimed twice.

**Reconstruction.** The finding ledger (`F1`–`F8`) survived intact. The verbatim
follow-up list (`T1`–`T7`) did not: it was lost when the working context was
compacted mid-session. `T1`–`T7` as named below are *reconstructed* from the
findings they belonged to, and every one is labelled `[reconstructed]`. Nothing in
this document claims to quote the original list, and no work is justified by a
reconstructed item alone — each is tied to a finding that is quoted.

**Evidence standard.** No verdict here rests on reading. Each names the command
that decides it or the `file:line` that does. All twelve quantitative claims this
tree's documents make are reproduced by `scripts/repro_claims.sh`, which prints
the command next to the measurement and classifies the claim (§3); `--list` prints
the map, `--require-all` turns any skip into a failure so a release check cannot
degrade into a no-op.

**Host.** 2 CPUs, ~2 GB RAM, gcc 14.2 present, clang absent (installed packages do
not survive between sessions on this host). Where a claim needs clang, the harness
says so and reports rather than asserts — see §7.

## 1. Verdicts on the eight findings

| # | Review's claim | Verdict | What decided it | Disposition |
| --- | --- | --- | --- | --- |
| F1 | major, latent soundness: `%ch` counted as *not* mentioning the `%rcx` family, and a test codified the wrong answer | **AGREE** | `local_patterns.rs:3582-3625`; the private four-spelling scan disagreed with `scan_register_refs` | fixed: one shared oracle + the test inverted and pinned |
| F2 | doc claim false: the guard is a prefix/name union, not a set of predicates | **AGREE** (doc-only) | `helpers.rs:505-511`, `flag_peepholes.rs:1756-1766`, `types.rs:222`, `types.rs:831-887` | §2 of the 09-19C document corrected in both places |
| F3 | the trailing-whitespace class was left unfixed at ~46 sites | **AGREE in principle; the count is not the point** | `whitespace_invariance.rs` (+747), `check_peephole_whitespace.sh` (+218), `peephole_common.rs` (+99) | fixed by making whitespace unobservable and enumerating the sites mechanically — **and then fixed again: as committed it cost 24% of compile time on a kernel TU (§2.10)** |
| F4 | test env restore was an unconditional `true`, no RAII | **AGREE** | `src/test_support.rs` (+299), wired `#[cfg(test)]` in `lib.rs:113-116` | fixed: RAII guard, one implementation, used by every env-touching test |
| F5 | five `FIXME` env markers remained, and `backedge_pre.rs` was missed | **AGREE** | `check_env_test_hygiene.sh` (+89); `src/passes/backedge_pre.rs` (+45) | fixed, and pinned by a permanent regression script |
| F6 | ~155 LOC of duplicated integer grammar in `parser.rs` | **AGREE** | `parser.rs` −76 lines of duplicated grammar in `is_label_like`, +390 lines of exhaustive equivalence tests | fixed: single-source recognizer, proven equivalent by enumeration |
| F7 | magic `1`s standing for the `%rcx` family | **AGREE** (cosmetic, real) | `const RCX_FAMILY: RegId = 1; const RCX_FAMILY_MASK: u16 = 1 << RCX_FAMILY;` | fixed in the production pass; partly already upstream in #559 |
| F8 | eight quantitative claims with no reproduction path | **AGREE — the most valuable finding** | `scripts/repro_claims.sh` (new) | fixed; running it found four false claims in our own docs, one bug in the harness itself, and one wrong invariant (§2) |

### F1 — `%ch` and the kill that deleted a live copy

The review was right, and right about the part that matters most: the defect was
*codified*. A private scanner in `local_patterns.rs` tested four spellings
(`%rcx`, `%ecx`, `%cx`, `%cl`) and asserted that `%ch` was a miss. `%ch` is bits
8..15 of the same architectural register, so `movzbl %ch, %ecx` reads the value a
`movq %rdi, %rcx` copy defined — and the address-copy kill deleted that copy.

Latent rather than live: it needs a high-byte read of the copied register inside
the fold's window, which the 772-program regression corpus does not contain and
the boot stage does not emit. "Latent" is not "harmless" — the test made the wrong
answer load-bearing, so any future fix would have failed CI.

The fix is not a fifth spelling. `mentions_rcx_family` now delegates to the shared
`mentions_family(bytes, RCX_FAMILY)` oracle, which is the same mapping
`scan_register_refs` has always used (`%ch` → family 1), so the three independent
scanners that disagreed — this one, `compare_branch`'s `%ah` rule and
`relay_and_lea`'s `%dh` rule — cannot drift apart again. The test asserts hits for
`%ch`, `movzbl %ch` and `%ch,%ecx` (so the *source-side* mention is covered too,
which was the one hole left open in an earlier iteration) and misses for neighbours
a coarser `%c` search would over-match.

### F2 — the guard is a union of two different kinds of test

The review said the document described predicates where the code has a prefix/name
union. Verified in the source:

* `dest_operand_is_full_width` (`helpers.rs:505-511`) has a classification arm —
  `LineKind::Pop { reg } => reg == fam` — which *is* exact, and a fallback that
  compares the text after the last top-level comma against exactly two spellings,
  `REG_NAMES[0][fam]` and `REG_NAMES[1][fam]`. That fallback is a name union, not a
  width analysis: it answers false for `%cx`, `%cl`, `%ch` and for any decorated
  destination. Conservative, and safe for a kill test, but not what the document
  claimed.
* Of the four callers, three gates are classification-based (`is_barrier`
  membership at `types.rs:222`; the `LineKind::Other` requirement in the copy-swap;
  the explicit `Pop { reg: 1 }` arm in the rcx walk). The fourth, in
  `flag_peepholes.rs`, is `mentions_exact_name` (`:1756-1766`) — a byte-substring
  scan with an alphanumeric-boundary check. Textual, not a predicate.

Both paragraphs of §2 in the 09-19C document now say this, with the line numbers.
One detail the review did not raise and testing settled: the document's
"`popl`/`popw` stay `Other` with an unparsed destination" is **correct for x86-64**
— `parse_dest_reg_fast` returns `REG_NONE` for a comma-less line that is not one of
the in-place modifiers it knows (`types.rs:831-887`) — and **false for i686**,
whose classifier matches `popl ` and produces a real family. That is §9.4, below.

### F3 — the whitespace class

The review counted ~46 sites where trailing whitespace still changed peephole
behaviour and called the class unfixed. Agreeing with the finding did not require
agreeing with the remedy: patching 46 call sites leaves the 47th to be written
tomorrow. `400ea84` instead makes the class unobservable — trimming lives once in
`peephole_common.rs`, the peephole is total on any operand, and
`whitespace_invariance.rs` (+747) plus `tests/regression/check_peephole_whitespace.sh`
(+218) enumerate the sites mechanically and shrink any failure to the smallest
reproducing line window, revealing significant blanks (`_` space, `>` tab, `~` CR)
so a regression is reported as two lines of assembly instead of a 511-line file.
The harness covers four paths: the peephole, the SSA validator, the assembler and
the CRLF variant (28 pairs compared byte-for-byte). Two harness bugs were found and
fixed while writing it, and are documented where they were: `str::lines()` strips
the CR that the CRLF variant exists to test, and comparing a window against a
whole-file baseline never shrinks.

### F4 + F5 — one shared fix

Both findings are the same defect seen from two ends: tests that mutate process
environment and restore it by hand. `src/test_support.rs` (+299, `#[cfg(test)]`,
wired at `lib.rs:113-116`) provides an RAII guard, so restore cannot be skipped by
an early return or a panic, and `tests/regression/check_env_test_hygiene.sh` (+89)
is a permanent pin: it fails if a *pass* reads the environment or a test mutates it
without the guard. `src/passes/backedge_pre.rs` — the file the review said was
missed — is in the commit (+45), and its hand-rolled `assert_eq!(…, false)` was
also the reason `cargo test` passed while `cargo clippy -D warnings` did not
(`bool_assert_comparison`): the lesson recorded in §7 is that clippy, not the test
run, is the gate.

### F6 — one grammar, proven equivalent by enumeration

`is_label_like` carried its own copy of the integer grammar the library parsers
already implement. The duplicate is gone (−76 lines) and the shared recognizer is
now the only one. Because "they behave the same" is exactly the claim that a
duplicate silently breaks, the commit adds ~390 lines of tests that do not sample:
`shared_grammar_matches_the_library_parsers_exhaustively` enumerates strings over
the relevant alphabet up to a bounded length, `is_label_like_matches_the_original_definition_exhaustively`
differs the new code against the old definition kept verbatim in the test, and
`the_public_evaluator_is_unchanged_by_the_shared_grammar` plus a width-boundary test
pin the edges. No wall-clock win is claimed for this change; the win is that the
two grammars can no longer disagree.

**Defect in my own work, recorded rather than hidden:** `2a675b3`'s commit message
does not mention `parser.rs` at all, despite +501 lines changed there. The message
describes the environment-hygiene work and stops. A reviewer reading the log alone
would not know the parser dedup happened.

### F7 — naming the family

`local_patterns.rs` now carries `const RCX_FAMILY: RegId = 1` and
`const RCX_FAMILY_MASK: u16 = 1 << RCX_FAMILY`, with the reason in the comment: a
bare `1` in four call positions that hand a family to family-generic predicates
reads as a count, and the day a second family gets the same treatment the literal
becomes a bug. The `1 << 1` occurrences that remain in the tree are expected-value
literals inside tests (`types.rs:2606-2616`, `fp_liveness.rs:1284`,
`vex_promote.rs:894`), which should stay spelled out: a test that computes its
expectation from the constant under test proves nothing.

### F8 — eight claims, now twelve, each with its command

This was the finding worth the most, because it is the one that cannot be argued
with: a number without a command is rhetoric. `scripts/repro_claims.sh` reproduces
every quantitative claim in this tree's follow-up documents and prints, for each,
the document section, the documented value, the command, the measured value and a
verdict. Claims are classified rather than treated as one kind of thing:

* **EXACT** — an invariant of the tree (a sha256, an `_end`, a section total, an
  object size, an `entries == refusals` identity). Measured ≠ documented is a
  failure.
* **MONOTONE** — counts that only grow as tests are added. Measured < documented
  fails; measured > documented is reported as drift.
* **REPORT** — a property of the host or a sample, not the tree (a timing
  distribution, another compiler's code size, a file-mode census). Printed next to
  the documented value, asserted never. Asserting a timing delta the document
  itself calls below the noise floor would be the dishonesty F8 objected to.
* **SKIP** — a prerequisite is absent. The prerequisite and the command that
  installs it are printed; `--require-all` turns any skip into a failure.

Results are in §3. Running the harness is what produced §2.

## 2. What testing found that the review did not

The review could not run anything, so it could not find these. All five are now
fixed in the tree, and all five were in *our* documents or *our* harness — the
place a reviewer is least able to check.

1. **The `mm/page_alloc.c` object size did not reproduce.** Documented 136896 B;
   measured **141504 B** with the command Kbuild itself recorded
   (`mm/.page_alloc.o.cmd`), replayed verbatim under this compiler. The figure was
   measured before the peephole work landed. The document now carries the measured
   value, the exact flag transform, and the reason gcc's own object for the same TU
   is 2121560 B (`CONFIG_DEBUG_INFO` DWARF, which this backend does not emit — the
   two sizes are not comparable). Claim 4 pins it as a codegen-stability gate.
2. **The ISA-gate census did not reproduce.** Documented "814 entries, 814
   refusals"; measured **3256 trace lines = 1628 entries and 1628 refusals over 586
   distinct functions** (623 unique (function, CFG shape) pairs, 350 entries with at
   least one loop). The *invariant* — every traced entry ends in the ISA refusal —
   holds exactly, and that is what claim 6 asserts; the absolute count follows the
   kernel config and is reported, not asserted.
3. **The "37-TU sweep" list had 36 entries.** The count in the document was one
   ahead of the list it described. `mm/page-writeback.o` is the 37th.
4. **`kernel/lockdep.o` is not buildable under this config** (`CONFIG_LOCKDEP` off:
   Kbuild has no rule for it), and because the sweep's first phase ran `make`
   without `-k`, that one bad name aborted the run and destroyed 35 in-flight
   records — a single unbuildable target cost the whole sweep. Replaced with
   `kernel/rcu/tree.o` (verified buildable), phase 1 now passes `-k`, and the
   harness distinguishes "not buildable under this config" from "a compiler
   rejected it".
5. **The reproduction harness's own first parser was wrong.** Claim 2 read the boot
   log's `.text` total with a substring match and `tail -1`, which picked up the
   `.text32` row and reported **32 B** instead of 22890 B. Found by running it. The
   parse now matches the first field exactly (`awk '$1 == ".text"'`).

And two that only an A/B run could produce, because both sides of the comparison
shared the defect:

8. **The replay harness mis-tokenized Kbuild's recorded command, and the failure
   looked exactly like a compiler regression.** `kernel_tu_command` split the
   recorded line with `read -r -a`, which does not interpret shell escaping, so
   `-DKBUILD_MODNAME=\"skbuff\"` reached the compiler with its backslashes intact.
   Eight of the 37 sweep TUs (`net/core/dev`, `net/core/skbuff`, `net/ipv4/tcp`,
   `net/ipv4/tcp_input`, `mm/vmscan`, `kernel/rcu/tree`, `crypto/sha256`,
   `crypto/aes_generic`) then died in the parser: `printk_index_wrap` expanded to
   `'"skbuff"'`, a multi-character constant, and lccc reported
   `include/linux/gfp.h:183:54: error: expected ')' before integer constant`.
   The sweep reported "post-change compiler rejected it" for all eight. It was
   reported for the *post* side only because the harness tries that side first;
   running the pre-change binary by hand produced the identical error, which is
   what proved the defect was shared. Two lessons are now written into the
   harness: tokenize with the shell (`set -f; eval "toks=( $raw_cut )"`) so the
   compiler sees what make would have shown it, and when an A/B reports a
   one-sided failure, run the other side by hand before believing the side.
   The same bug perturbed claim 4: the faithful `mm/page_alloc.o` is 141592 B, not
   the 141504 B the mangled define produced.
9. **"Identical objects" was the wrong invariant for this patch.** `400ea84`
   deliberately broadens the peephole (trim once, be total on any operand), so a
   sweep that demands byte identity is a sweep that must fail. Claim 5 now asserts
   what the change actually promises: every listed TU compiles under both
   compilers, no object grows, and every differing pair is *saved*
   (`/tmp/repro-differ/<tu>.{post,pre}.o`) so a difference can be read in the
   disassembly instead of being reported as a count. A number you cannot inspect is
   not a finding.

10. **The whitespace fix cost 24% of compile time on real kernel code, and the
    A/B harness is what said so.** Claim 8 interleaves the two compilers on
    `mm/page_alloc.c` and is REPORT-class, because a compile-time delta on a
    2-core box is usually noise. This one was not: over six interleaved rounds the
    ranges did not overlap at all (pre median 35681 ms, max 36545; post median
    41019 ms, min 40861). Bisecting against a build of the intermediate commit
    `2a675b3`, order-rotated so drift cancels:

    | binary | median on `mm/page_alloc.c` | object |
    | --- | --- | --- |
    | `origin/main` (`56858cb`) | 33343 ms | 141592 B |
    | `2a675b3` (R4-4: env hygiene + ISA-gate hoist) | 32982 ms (**−1.1%**) | 141592 B |
    | `400ea84` (F3 as committed) | 40881 ms (**+24%**) | 141592 B |
    | F3 + shared ASCII scan in `trimmed()` | 35053 ms (+6%) | 141592 B |
    | F3 + the redundant trim removed | **33269 ms (−18.4% vs F3, at baseline)** | 141592 B |

    The cause was one line. `LineInfo::trimmed()` — called from **473 sites**
    inside the peephole's fixed-point loop — had gained a `str::trim_end()`,
    which walks the line backwards decoding UTF-8 through `char::is_whitespace`.
    At the call volume a 7764-line TU reaches, that is 7.8 s. Replacing it with
    the ASCII byte scan the store already uses cut it to 1.9 s; noticing that the
    trim was *unreachable work* removed the rest. `LineStore`'s entire API is
    `new`, `get`, `len`, `replace`, and both writers already strip trailing
    blanks, so no pass can obtain padded text and `trimmed()` can go back to
    being a pure slice. The object is byte-identical at every step (141592 B), so
    this was pure overhead. The invariant is now pinned twice: by
    `the_store_never_hands_a_pass_trailing_whitespace` (seven paddings, CRLF,
    `new`/`get`/`build_result`/`replace`), and by a `debug_assert` in `trimmed()`
    that names the store as the only legal source — with the caveat written down
    that `fastbuild` inherits `release`, so `debug_assertions` are off in the
    shipped profile and the test, not the assert, is the load-bearing guard.

    The lesson is the reason this section exists: `400ea84` was validated for
    *correctness* (byte-identical output, harness green) and never for *time*. A
    whitespace fix that costs a quarter of the compile on real code is a bad
    trade, and it was invisible until F8's demand for a reproduction path
    produced an instrument that measured it. A REPORT-class claim is not a
    claim to ignore: non-overlapping ranges are a finding.

Two more, from the same sweep over our own prose:

6. **§9.4's pop claim was not imprecise but false.** It said only the `popq `
   spelling becomes `LineKind::Pop { reg }` and that `popl`/`popw` fall through to
   `Other`. In the i686 backend the classifier matches `popl `/`pushl `
   (`i686/codegen/peephole.rs:884-894`) and the codegen emits 117 `popl` sites
   against zero `popq`/`popw` — so the 32-bit form *is* classified with a real
   family, and is a barrier (`:153`). The genuinely unclassified spellings are
   `popw`/`pushw` and unsuffixed `pop`/`push`, which the i686 assembler accepts
   (`encoder/mod.rs:495-498`) but no codegen path emits; they can only arrive inside
   inline asm, where every line of a `#APP`/`#NO_APP` region is forced to
   `LineKind::InlineAsm` — pinned, all `reg_refs` set, `has_indirect_mem=true`,
   `kill_all()` on the offset trackers (`types.rs:116-122`; `peephole.rs:11449-11455`,
   `:3310`). The deferral stands; the reason for it does not, and the wrong reason
   implied a live hole where there is none.
7. **The kernel harness was not reproducible across sessions.** The workspace
   snapshot truncates a 55k-file kernel tree between turns while leaving the
   `.lccc-prepared` marker behind, so the next session sees a prepared tree that is
   29 files; and `arena_session_restore.sh`'s package list was missing `zstd`/`kmod`
   and friends, so `--with-kernel` failed *after* downloading the 155 MB tarball.
   `prepare_kernel_tree.sh` now verifies canaries and regenerates a damaged tree,
   and the restore script covers the full preflight.

## 3. The twelve claims, reproduced

`scripts/repro_claims.sh --list` prints this map; the table records what each
returned on this host, at `HEAD`, against the tree at
`/home/user/kernel-work/linux-6.18.52` (linux-6.18.52 + the 28 CachyMod patches).

| # | Class | Claim | Documented | Measured on this host | Verdict |
| --- | --- | --- | --- | --- | --- |
| 1 | EXACT | boot stage fits the 32 KiB gate | `_end=31168`, headroom 1600 B | `PASS (_end=31168, headroom=1600 bytes)` | ok |
| 2 | EXACT | boot `.text` total | 22890 B | 22890 B | ok |
| 3 | EXACT | `setup.bin` sha256 | `bf9f0e9f5f19…29cffd53` | byte-identical | ok |
| 4 | EXACT | `mm/page_alloc.c` under Kbuild's own flags | 7764 lines → 141592 B | 141592 B (gcc's own object: 2121560 B, DWARF) | ok |
| 5 | EXACT | 37-TU sweep, pre vs post | all compile; no TU emits more real instructions | 37/37 compile both sides; 20 byte-identical, 17 differing; 332475 → 332420 real instructions (−55); 0 TUs worse; 1 object +16 B of alignment padding | ok |
| 6 | EXACT | ISA-gate census on a real TU | 1628 entries / 1628 refusals / 586 functions | 3256 trace lines, 1628 entries, 1628 refusals, 586 distinct functions, 623 unique CFG shapes, 350 with ≥1 loop | ok |
| 7 | EXACT | ISA-gate trace byte-identical across five env combos | stderr 18 / 1526 / 9 / 18 / 0 and identical asm | `check_vectorize_isa_gate.sh` exit 0, PASS | ok |
| 8 | REPORT | interleaved compile-time A/B | pre 33343 ms, `2a675b3` 32982 ms, HEAD 33269 ms | pre median 33381 ms, post median 32976 ms (n=6, interleaved) | report — direction only |
| 9 | REPORT | boot size oracle vs two assemblers | lccc / gcc / clang rows | oracle exit 0; lccc `_end=31168`, headroom 1600, PASS; gcc `_end=22880`, headroom 9888, PASS; **clang not installed here** | report — clang row not reproducible on this host |
| 10 | MONOTONE | unit + integration totals | 2953 passed, 0 failed, 7 ignored | 2953 passed, 0 failed | ok |
| 11 | MONOTONE | regression corpus totals | 773 passed of 784 | exit 0; 773 passed, 0 failed, 11 skipped-compare, 784 total, 103 s | ok |
| 12 | REPORT | tracked-file mode census | 151 shebang-and-644, 91 of them `check_*.sh` | 151 / 91 | report — the census follows the tree; the rule it justifies is what §7 states |

`repro_claims: PASS (every measured claim agreed with its document; skips name
their prerequisite)`. Four cosmetic reporting defects visible in that run's own
output — claim 5 printing `0 1` for a counter because `awk` emitted four fields
into three variables, claim 7 counting `^ok` lines a harness that indents its
output does not produce, claim 11's `PASS=`/`FAIL=` pattern matching nothing and
falling back to an empty line, and claim 10 asserting a total three revisions
stale — were fixed after it and re-verified on claims 11 and 12; no verdict
depended on them. A harness that prints an empty measurement has the same defect
as a document that prints an unreproducible one.

## 4. Numbers that moved, and why

| Documented | Measured | Why it moved | Where it is pinned now |
| --- | --- | --- | --- |
| `mm/page_alloc.o` = 136896 B | **141504 B** | measured before the peephole work landed; codegen moved | claim 4 (EXACT) + 09-19C §0 |
| ISA-gate census 814/814 | **1628 entries / 1628 refusals / 586 functions** | pre-hoist compiler, different trace grammar | claim 6 asserts the identity, reports the count + 09-19C §5 |
| unit + integration 2931 | **2953 passed, 0 failed, 7 ignored** | tests added by F3/F4/F5/F6 | 09-19C §10 |
| regression corpus 772/783 | **773 passed of 784**, 11 skipped-compare, 103 s | corpus programs added | 09-19C §10 |
| "37-TU sweep" | 36 listed | the list lagged the prose | claim 5's frozen list (37) |
| `.text` total 22890 B | 22890 B | did not move — the harness misparsed it as 32 B | claim 2 (EXACT), parser fixed |
| boot `_end` 31168 / headroom 1600 | unchanged | — | claim 1 (EXACT) |
| `setup.bin` sha256 `bf9f0e9f…` | unchanged | — | claim 3 (EXACT) |

A number that moves is not automatically a defect, but it must move *on purpose*:
each EXACT claim fails loudly, and the failure text says to re-measure with the
harness and update the document in the same commit rather than loosen the gate.

## 5. Validation

Every row was run on this host at `HEAD` after the last source change, not
carried over from an earlier round.

| Gate | Command | Result |
| --- | --- | --- |
| Format | `cargo fmt --check` | clean |
| Lint | `cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings` | exit 0 |
| Unit + integration | `cargo test --profile fastbuild --all-targets --locked -j 2` | 2952 passed, 0 failed, 7 ignored in the lib target; 2953 across all targets |
| Whitespace invariance | `CCC=$LCCC bash tests/regression/check_peephole_whitespace.sh` | PASS — in-tree corpus, operand totality, generated corpus, assembler path |
| ISA gate | `CCC=$LCCC bash tests/regression/check_vectorize_isa_gate.sh` | PASS |
| Env/test hygiene | `bash tests/regression/check_env_test_hygiene.sh` | PASS — guard-only env mutation, migrated passes clean, reads not growing, no markers |
| Regression corpus | `CCC_VALIDATE_SSA=1 python3 tests/regression/run_regression.py --lccc $LCCC -j 2` | 773 passed, 0 failed, 11 skipped-compare, 784 total, 100 s |
| Boot gate | `KERNEL_DIR=… LCCC=$LCCC bash scripts/build_kernel_boot.sh` | PASS, `_end=31168`, headroom 1600 B, `.text` 22890 B, `ld.bfd` oracle byte-identical, `setup.bin` sha256 `bf9f0e9f…ffd53` **unchanged after the `trimmed()` performance fix** |
| Local CI | `bash scripts/ci_local.sh --fast` | 47 passed, 0 failed, 4 skipped |
| Claim reproduction | `LCCC_PRE=… bash scripts/repro_claims.sh` | 12/12: 8 ok, 4 report, 0 skipped, 0 failed (§3) |
| Kernel TU sweep | claim 5 of the above | 37/37 compile under both compilers; −55 real instructions; no TU worse (§8a of the 09-19C document) |

The boot sha256 row deserves the emphasis it has: the `setup.bin` image is the
strictest artifact this repository produces, and it is byte-identical before and
after removing the per-access trailing trim. That is the end-to-end proof that the
performance fix in §2.10 changed no codegen anywhere, which is a stronger
statement than the object-size equality on one TU.

## 6. Performance

Every performance statement here was measured on this host with order-rotated
interleaved runs, and one of them (§2.10) is a regression this patch introduced
and then removed. The numbers, in the same unit — median wall-clock on
`mm/page_alloc.c` (7764 lines) under the kernel's own recorded flags, three
binaries built from the same profile:

* **The ISA-gate hoist** (09-19C §5) removes the vectorizer's CFG-forest
  construction from every non-SIMD translation unit of a kernel whose
  `KBUILD_CFLAGS` disable SSE globally: 1628 entries per TU on `mm/page_alloc.c`,
  every one of which ended in a refusal. The traced path still computes the forest
  and prints the same lines in the same order — that is claim 7's byte-identity
  across all five environment combinations, and it is why the trace did not have to
  be sacrificed for the speed.
* **Net compile time on a real kernel TU is back to baseline**: `origin/main`
  33343 ms, `2a675b3` 32982 ms, `HEAD` 33269 ms — the R4-4 hoist's ~1% is retained
  and F3's 24% is gone (§2.10). Claim 8 stays REPORT-class because a compile-time
  delta on a 2-core box is normally below the noise floor; it earned an exception
  here by producing non-overlapping ranges, which is the condition under which a
  REPORT should be read as a finding. Re-run it with `--rounds` on a quieter host.
* **The shared recognizer (F6) and the shared mention oracle (F1)** are
  de-duplications, not optimisations: no wall-clock win is claimed. The measured win
  is that three scanners which disagreed now cannot.
* **The whitespace harness** costs what a regression script costs and runs in
  `ci_local.sh --fast`; it is not in the compiler's hot path. Trimming moved into
  `peephole_common.rs` once instead of per pass.

## 7. Residual risk, deferred work, and lessons

1. **Real-mode boot size is the largest remaining risk.** lccc's boot `.text` is
   22890 B against gcc 14.2's 13479 B and clang 19's 14740 B; the 32 KiB gate passes
   with 1600 B of headroom, and the per-object table shows the gap is broad
   (`printf` +1598, `video` +1343, `string` +1027, `cpucheck` +852, `cmdline` +839)
   rather than one bad object. That is a codegen-quality programme for a 16-bit,
   `-Os`, `-march=i386`, `-mregparm=3` closure — not a defect with a fix, and not
   something this patch should pretend otherwise about.
2. **`bzImage` is still out of reach on this host.** The harness cannot build
   objtool-acceptable objects, does not emit `__fentry__` or the `%gs` canary, so it
   runs with tracing, objtool and the stack protector disabled and stops at the boot
   gate — which is `setup.ld`'s own `_end <= 0x8000` assert, i.e. the condition that
   must hold before a `bzImage` can exist at all.
3. **The clang row of the size oracle cannot be reproduced here** (clang is not
   installed and packages do not persist between sessions). Claim 9 reports this
   instead of asserting the documented clang numbers.
4. **Clippy is the gate, not `cargo test`.** `assert_eq!(x, false)` compiles and
   passes; `cargo clippy -D warnings` rejects it (`bool_assert_comparison`). Every
   "green" claim in this document was checked with clippy and `cargo fmt --check`,
   not with the test run alone.
5. **The workspace snapshot is a hazard for kernel work.** See §2.7: a marker file
   outliving a truncated tree is worse than no marker, because it lies. Canary files
   are the authority now.
6. **A commit message that omits 501 lines of its own diff** (§1, F6) is a defect in
   the work, not in the code. Recorded here because the diff is the deliverable and
   the log is how a maintainer reads it.

## 8. Reproducing this document

```sh
scripts/prepare_kernel_tree.sh              # kernel tree, self-sufficient, canary-verified
cargo build --profile fastbuild -j 2        # the compiler under test
scripts/repro_claims.sh --list              # the claim -> command map
scripts/repro_claims.sh                     # all twelve claims
scripts/repro_claims.sh --claims 4,6        # the two that moved
scripts/repro_claims.sh --require-all       # no claim may skip
CCC=target/fastbuild/lccc bash tests/regression/check_peephole_whitespace.sh
CCC=target/fastbuild/lccc bash tests/regression/check_vectorize_isa_gate.sh
bash tests/regression/check_env_test_hygiene.sh
cargo clippy --all-targets --profile fastbuild --locked -j 2 -- -D warnings
```

Claims 5 and 8 additionally need a pre-change binary to A/B against
(`LCCC_PRE=<path>`); without one they skip and name the prerequisite. Building it
from the patch base costs one `cargo build` in a worktree at `origin/main` and reuses
the dependency artifacts (3m07s here).
