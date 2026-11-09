# Red-team audit of the last 20 merged PRs — 2026-11-08

Branch: `arena/d3289d35-lccc`.  Base: current `origin/main` at the start of the
audit.  Scope: the twenty most recently merged pull requests in `ms178/lccc`,
not merely the files touched by the newest one.

## Method

1. Fetch each PR's metadata and complete diff (`gh pr view`, `gh pr diff`).
2. Read the changed production paths and the adjacent invariants, then run the
   PR's own regression gate plus a targeted adversarial probe where the gate
   could be vacuous.
3. Reproduce every claimed defect against GCC 16.2 and Clang 23.1.5 before
   changing code.  A finding without an executable witness is not a finding.
4. Fix the confirmed defect at the shared invariant, add a regression that
   fails on the pre-fix tree, and re-run the local CI mirror.

Toolchains: GCC 16.2.0, Clang 23.1.5, Rust 1.99.1, GDB 18.1, binutils 2.47,
mold 3.0.0.  Build policy: fastbuild, two Cargo jobs, exactly 4 GiB swap.

## Confirmed findings and fixes

### F1 — Query operands could not cross physical-line trivia

PRs: #1154, #1160, #1162.

`int x=__has_feature\n(tls);` is valid: newline is whitespace between a
function-like preprocessing operator and its operand.  The pending-line
accumulator recognized only user `MacroDef` function-like macros, so a
compiler-owned query was resolved before the next line arrived.  The resolvers
then skipped only spaces/tabs, mis-absorbed the `(` as the missing-operand
token, and emitted `int x=0\n(tls);` plus a false `missing '('` diagnostic.
Blank lines and trailing comments had the same failure.

Fix: `ends_with_funclike_macro` now recognizes active query operators and
trivia-only continuations; the accumulator keeps the line pending under the
existing bounded newline limit; every query whitespace skip uses the shared
comment/newline-aware helper.  The candidate is validated outside literals and
comments, and only the last physical line is inspected.

### F2 — Missing-`(` token boundaries ignored the compiler persona

PR: #1162.

`pp_token_len` always lexed `::` as one punctuator.  Clang and C23 do, but
GCC's pre-C23 C lexer sees two `:` tokens.  Recovery after a malformed query
therefore absorbed the wrong token boundary in `-std=c11`.

Fix: `pp_token_len(bytes, i, scoped_colon)` takes the lexer fact explicitly.
Clang-persona operators pass `true`; GCC-persona operators pass `c23_dialect()`.
The differential matrix now covers both dialects.

### F3 — Token-length scanning was linear in the wrong dimension

PR: #1162.

The punctuator path probed a 29-entry prefix table, and an unknown non-ASCII
token validated the **entire remaining line** as UTF-8.  A run of unknown
non-ASCII tokens was quadratic.

Fix: first-byte dispatch, shared maximal-munch digraph helpers, and at most
four bytes of UTF-8 validation per unknown scalar.

Measured on this VM (best of five, `rustc -O`, black-boxed inputs; not a PMU
claim):

| input | old | new | speedup |
|---|---:|---:|---:|
| 340,000 punctuator tokens | 0.016638 s | 0.000924 s | 18.01x |
| 20,000 unknown UTF-8 scalars | 0.289217 s | 0.000096 s | 3026.55x |
| 200,000 mixed tokens | 0.005055 s | 0.000574 s | 8.80x |

### F4 — C23 digit separators hid a trailing query candidate

PRs: #1154, #1162.

The new literal-aware candidate validator treated the `'` in `1'000` as a
character-literal opener, so `int x=1'000 + __has_feature\n(tls);` failed even
though the query token was real code.

Fix: skip the whole pp-number once during candidate validation.  This also
avoids probing every separator backwards, which would be quadratic on a long
separated number.

### F5 — Recovery coverage was hand-picked and persona-blind

PRs: #1154, #1160, #1162.

The lifetime gate had 244 checks but no batched malformed-token matrix, no
newline/blank/comment operand shapes, and no C23-number-tail case.  The fix
adds a 28-tail x 7-operator x 3-context x 2-dialect oracle matrix, ten
newline/trivia shapes per operator, balanced-comment operands, and C23 number
tails: **367 checks**.  Preprocessor units grew to **131**, including
panic-safe dialect restoration and UTF-8 boundary properties.

## PR coverage ledger

| PR | title | files | merge |
|---|---|---:|---|
| #1162 | Preprocessor red-team successor: missing-paren recovery and compiler-persona parity | 5 | `a41cee0fe4` |
| #1160 | Preprocessor red-team follow-up: raw query operands and i686 biarch fallback | 4 | `5e9a06a2fa` |
| #1159 | [SLP] Perfect the exact two-tier dedup: red-team audit, hardening, zero-warning CI | 4 | `6efad73129` |
| #1157 | vec: read every claim at its own width; pin six store-source laws | 7 | `1f783b0f3c` |
| #1155 | x86 peephole: forward vector reloads of a just-stored aligned frame home (-36.8%) | 9 | `2e5b854dc2` |
| #1154 | Truthful __has_feature/__has_extension (GCC 16.2 parity) + capability execution gates + cached mold oracle | 26 | `d3cd669726` |
| #1153 | SV-03: sink conditional loads by speculating the address, never the load | 18 | `b81f5d801c` |
| #1152 | x86 vector homing: one source of truth for register-held values; inliner refuses block-sparse bodies | 6 | `f35994f0b6` |
| #1151 | [SLP] Object-identity store streams, measured cost model, 4-lane feedstock | 7 | `9f8883ec3d` |
| #1150 | simd: fix SA-05/SA-33/SA-36, audit all 67 EVEX arms, and wire the audit driver CI never ran | 17 | `d1ee0bfb8c` |
| #1149 | i686: fix the doc comment #1146 orphaned + make its fail-closed argument an executable test | 1 | `a7f2c9bc49` |
| #1148 | Audit GCC torture evidence and diagnostic scaling | 30 | `1f442b8f90` |
| #1147 | [OPT] Forward memcpy chains through disjoint vector writes (avx_fp_private 7.23x -> 1.54x vs GCC) | 2 | `dcd1822c4c` |
| #1146 | peephole: borrow instead of copy (-17.069% allocations) + fix the ratchet's 23-site blind spot | 18 | `baaa0199fa` |
| #1145 | maskfacts: eliminate the runtime vpcmp* vector HOME roundtrip (7403 -> 4523) | 20 | `ced92d0071` |
| #1144 | SIMD: make vector forwarding and store motion alias-safe | 7 | `35a1e23e42` |
| #1143 | Red-team round 10: EVX register-claim fix, redundant mask-cast elimination, set1 signature parity | 7 | `782b9f4f43` |
| #1142 | i686 peephole: fix the alloc ratchet's 3-backend blind spot and the 5 sites it hid | 6 | `c847a670e3` |
| #1141 | SIMD perfection: audited correctness fixes, red-team soundness proofs, 117 driver self-tests | 28 | `df2fdfbb80` |
| #1140 | Red-team audit: close a latent stale-slot fold, remove debug instrumentation, add differential tests | 10 | `fe3706467c` |
## Validation receipts (this session)

- Fastbuild: warning-free, `BUILD_RC=0`.
- Preprocessor units: 131 passed, 0 failed.
- Query lifetime gate: 367 checks, 10 operators.
- Frontend/preprocessor gate: 1346 oracle checks / 18 modes / 21 names,
  capability execution, 538-case fuzz, lifetime gate: PASS.
- Extra query fuzz: 1088 cases each at seeds 7, 99, 20261109: PASS.
- SIMD gates re-run for the merged SIMD PRs: aligned-vector-reload (4
  configurations) + 26 contract tests; vec-store six laws + 18 mutation
  self-tests; commutativity agreement (86 operations / 139 width rows) + 27
  tests; SLP object identity (34 tests); audit pack (19 tests): PASS.
- rustfmt and doc-link gate: PASS.
- Full `scripts/ci_local.sh --fast` and hosted CI: recorded in the PR body
  after the final run; no claim is made from a historical receipt.

## Honest limits

ICX 2026.2, objtool and pahole are not available in this sandbox unless the
hosted authenticated lane supplies them; local results do not infer their
outcome.  The VM microbench above is a deterministic scanner measurement, not
a hardware-counter claim.  Open backlog items outside the reproduced findings
are not silently reclassified as fixed.
