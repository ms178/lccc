# The PR #733 review, adjudicated: the fourth audit, closed for real

An external reviewer audited PR #733 (commit `62c25fe4`, "Harden x86
differential oracle and corpus contracts", based on `27cffb80`) and
returned one blocking finding, three medium, three low — rating the PR
7.3/10 with B1 as the merge blocker. Every finding was re-derived here
against the pinned binutils 2.47.20260726 pair and the S14-rev fastbuild
encoder before a line was changed; all seven were agreed, and the fixes
landed as one revision with every fix validated before the next began.
Three additional defects in the same class were found here during the
re-derivation and fixed alongside.

## 0. Method

Re-based per standing instruction: the series sits on `27cffb80`
(latest `ms178/lccc` main at delivery — the merge of PR #732, whose
first parent is `be8b8569`, the merge of PR #731). The prior record's
§0 claimed `be8b8569` as this series' base; corrected in §6 there.
Baselines re-established before any edit: fastbuild 58 s / 0 warnings
(rustc 1.99.0 stable, gcc+bfd link — no clang/mold on this runner),
`test_encdiff.py` 55/55, `test_ci_gate_parity.py` 23/23,
`check_ci_gate_parity.py` PASS (132 commands), the encdiff corpus gate
green at the recorded baseline (208 rows, BEATS=175, ORACLE-INVALID=10,
ok=23, −559 B), both asm-diff corpora green against the pinned pair
(1418 + 672, 0 failed).

## 1. Verdicts on the seven findings

| # | Review's claim | Verdict | What decided it | Disposition |
| --- | --- | --- | --- | --- |
| B1 | `semantically_equal` still false-accepts: `_UNDISASM`'s `\b` can never match `(bad)` (non-word char before a required word/non-word transition), `_norm_disasm` drops the byte column so `06` and `0f 04` both normalise to bare `(bad)`, and `.stdout` is compared without `.returncode` — two failed disassemblies compare equal (`"" == ""`); exposure 25 `allow_better` groups / 7 casefiles / ~2,236 rows | **AGREE, BLOCKING** | re-derived mechanically: the regex probe (`\(bad\)\b` against the real rendering) never fires; `objdump -d` of `.byte 0x06` vs `.byte 0x0f,0x04` (probed this session, binutils 2.44 and the pinned 2.47) prints BOTH bytes before `(bad)` — the byte column is what the comparison drops; a `returncode=1` pair was fed through the shipped code and compared True | **landed, redesign**: the whole check is rebuilt fail-closed. `_parse_disasm` walks the listing with a total line grammar — headers/labels admitted, instruction lines split at the byte/instruction tab, undecodable fields (`(bad)`, `.byte`, any parenthesised synthetic) refuse the whole stream, a `...` gap of undecoded bytes refuses it, an unknown line shape refuses it, an empty instruction stream refuses it — and `_disasm_stream` gates it behind `returncode != 0 → None`, `OSError/Timeout → None`. `semantically_equal` is now `a is not None and b is not None and a == b`. The objdump is threaded explicitly through `run_case` (no module global), and the summary names both oracles' releases. Verified non-breaking for real this time: both corpora re-run green under the fixed oracle (1418/0, 672/0) — no legitimate `allow_better` verdict ever rested on the holes |
| M1 | no direct tests for the new asmdiff semantics — exactly why B1 survived | **AGREE** | zero test imports of `semantically_equal` existed | **landed**: new `scripts/test_asmdiff.py`, 19 cases: the eight contract cases (valid-equal with non-vacuity argv pinning, valid-diff, the 1-byte vs 2-byte `(bad)` regression pin, `.byte` fallback, one nonzero exit, two nonzero exits — the fail-open pin, empty output, timeout/OSError) plus the structural set (gap marker, unknown line shapes, byte-continuation fragments, headers, normalisation chain) and a real-toolchain leg anchoring the parser to genuine objdump output (scale-1 SIB fold is equality; `(bad)` prefixes refuse) |
| M2 | `ensure_gas_247.sh` validates presence, not version — `grep -q '2\.47'` accepts `2.470`, `12.47`, `wrapper-2.47-malicious`; no `as`/`objdump` pair-equality; post-install prints without asserting | **AGREE** | read the guard; the probe table was run through the shipped grep (all four lookalikes accepted) | **landed**: one `validate_pair` used by BOTH trust paths — anchored token extraction (`--version` line 1, last whitespace-delimited field) matched against `^2\.47(\.[0-9]+)*$`, `as`/`objdump` token EQUALITY (two builds both saying 2.47 is not "one 2.47 build"), a functional canary assembling and disassembling `mov %eax,%ebx` in both `--64` and `--32` modes for x86-family targets, and a validated post-install (`validate_pair \|\| FATAL`). Falsified by a 12-case `--self-test` matrix of fake tool pairs: correct 2.47 accept; 2.46, `2.470`, `12.47`, `wrapper-2.47-malicious`, mismatched tokens, failing/empty `--version`, canary-broken, as-refuses — all rejected; the dated `2.47.20260726` snapshot form (which the REAL pair prints) accepted on both halves. The `head -1` pipefail hazard is gone (`sed -n '1,1p'`) |
| M3 | byte-pin ingestion is not fail-closed: case-sensitive near-miss detection lets `# BYTE-EXACT 90` through as prose, and conflicting duplicate pins are last-write-wins | **AGREE** | fed all five shapes through the shipped `split_byte_pin`: the uppercase spellings returned `(text, None)` silently | **landed, exceeded**: the strict grammar AND the near-miss detector are `re.IGNORECASE` (`# BYTE-EXACT <hex>` parses as the same pin; `# Byte_Exact junk` is an error; uppercase hex is accepted by `bytes.fromhex`), and the harvest is a testable `harvest_byte_pins` that hard-errors on conflicting duplicate pins (`90` then `91`) and on the same text appearing both pinned and unpinned — the deduplicated row's contract is never decided by corpus ordering or a dict's last-write-wins. Pinned by four new tests |
| L1 | gate parity accepts lookalike paths: substring containment takes `.../bin/objdump-untrusted`, `/untrusted/.../bin/objdump`, `.../bin/objdumps`; the `--as` check carries the same weakness; so does the `ensure_gas` invocation check | **AGREE** | ran the lookalikes through the shipped checker — all passed | **landed, exceeded**: `PINNED_AS`/`PINNED_OBJDUMP` constants compared by EXACT token equality in both checkers, and `ensure_gas_invoked` requires the full token list `bash scripts/ensure_gas_247.sh x86_64-linux-gnu` (a mutated target, extra arguments, or the wrong target all fail). Falsified by a new mutation test: six lookalikes (suffixed/prefixed/sibling × both tools) on the asmdiff gate and on BOTH mirrors of the encdiff gate — where the old containment check passed — plus three installer lookalikes |
| L2 | baseline parsing tolerates ambiguity: duplicate digest lines and duplicate verdict names overwrite silently, negative counts pass `int()`, a misspelled zero-count class is discarded by the nonzero projection, and the `--expect-histogram` help still says "counts only, no bytes" while the digest now covers pins | **AGREE** | fed each shape through the shipped parser; all four were accepted as data | **landed**: the parse is strict end-to-end — `VERDICT <nonnegative integer>` (isdigit-gated), name validated against `SEVERITY` (unknown → error listing the supported set), duplicate class → error, duplicate digest line → error, missing classes → error (every class exactly once, zeros included), digest-line matcher case-insensitive like the pin grammar; the help text states the true contract (counts AND rows-sha256 row identity including byte-exact pins). Pinned by seven new tests including THE invisible case (misspelled zero-count class) |
| L3 | documentation overclaims: the record certifies the undecodable refusal CLOSED (it was not), repeats it in §5, confuses `be8b8569` with `27cffb8`, double-counts the test arithmetic ("8 changed-path tests + all other 53"), and leaves review-identifier archaeology in production comments | **AGREE** | every sub-point re-derived; the `\b`-probe and failed-objdump probe of B1 are what falsify the closure claims | **landed**: §6 of the prior record carries the corrections without rewriting history; this record states the true numbers. The archaeology sweep removed every `S1x`/`PR #7NN audit` identifier from the touched production files (asmdiff, encdiff, parity checker + tests, corpus, ci_local, ci.yml, encoder mod.rs) — the reasoning stands inline, the identifiers resolve in the engineering records. Out-of-scope debt noted in §4 |

## 2. Found here during re-derivation (the same class, beyond the review)

- **The `...` gap marker**: `objdump -d` prints a bare tab-ellipsis line
  where bytes exist that it declined to decode (probed: a 17-byte
  `.text` ends `d: 66 0f 07 \t data16 sysretl` then `\t...`). The
  pre-revision code silently dropped it, so two objects with DIFFERENT
  undecoded tails compared equal behind identical decoded prefixes.
  The parser now refuses the whole stream on it — undecidable content
  is not semantic evidence, in either direction.
- **Byte-continuation fragments**: objdump splits a byte column wider
  than one line across continuation lines that carry an address and
  bytes but NO instruction column (probed: `48 8b 04 3d ff ff ff ff`
  renders as 7 bytes + `7:\tff`). A naive strict parser would reject
  legitimate streams on it; a naive skip-everything parser would eat
  garbage. The parser admits exactly this one fragment shape (valid
  bytes, no instruction tab) and skips it — the instruction is anchored
  on the line that has the column. Pinned by test and by the real-
  toolchain scale-1 leg.
- **The `ensure_gas` presence check** was substring containment on a
  line of shell — the same class as L1's path pins. Covered by the
  L1 fix (`ensure_gas_invoked`).

## 3. Reproduced against the pinned toolchain (hard evidence)

Everything below ran here, against
`~/.cache/gas-2.47-x86_64-linux-gnu/bin/{as,objdump}`
(2.47.20260726, one build, validated by the new `validate_pair`) and
`target/fastbuild/lccc-x86`/`lccc-i686` (fastbuild, 0 warnings):

- The B1 probes: `\(bad\)\b` never fires on the real rendering
  (`0:\t06                   \t(bad)` — cat -A verified); the shipped
  `semantically_equal` accepted a `returncode=1` pair and a
  1-byte-vs-2-byte `(bad)` pair as EQUAL — both re-fed through the
  pre-revision tree before any edit landed.
- The M2 probe table: `2.470`, `12.47`, `wrapper-2.47-malicious` all
  pass the shipped `grep -q '2\.47'`; all three are rejected by the
  anchored token. The real pair's `2.47.20260726` form is accepted by
  design and by the self-test.
- The L1 probes: all six lookalike paths pass the shipped containment
  check on both mirrors of the encdiff gate; all six fail token
  equality, as do the three installer lookalikes.
- The M3/L2 probes: `# BYTE-EXACT 90` returned as prose through the
  shipped `split_byte_pin`; duplicate digest lines, duplicate names,
  `BEATS -3`, `okk 0` and a dropped class were all accepted as data by
  the shipped parser. Every one is now a hard error, pinned by test.
- **The three differential gates, green under the fixed oracles**:
  x86-64 asm-diff **1418/1418**, i686 asm-diff **672/672**, encdiff
  corpus **208 rows BEATS=175 ORACLE-INVALID=10 ok=23, −559 B** at the
  UNCHANGED recorded baseline (all 16 byte-exact pins live, digest
  accepted by the strict parser) — no verdict-baseline refresh, because
  no row, pin or count changed.
- Suites: `test_encdiff.py` **66/66** (55 + 11 new), `test_asmdiff.py`
  **19/19** (new), `test_ci_gate_parity.py` **24/24** (23 + 1),
  `ensure_gas_247.sh --self-test` **12/12**, `check_ci_gate_parity.py`
  **PASS (132 commands)**, `cargo fmt --all --check` clean,
  `cargo clippy --profile fastbuild --lib -- -D warnings` clean.

## 4. Soft citations (stated, not re-derived here)

- Hosted CI legs are beyond this harness; the parity checker and its
  mutation tests are the local proxy, per the standing gate-parity
  contract.
- The PR-body test-count phrasing (wherever it said "8 changed-path
  tests + all other 53") is corrected by §6 of the prior record; the
  GitHub PR body itself is not editable from here.
- Review-identifier references in files this series did not touch
  (`callgrind_ab.py`, `edg_changes_mine.py`,
  `test_differential_corpus_paths.py`, peephole/vectorize sources)
  are pre-existing upstream content from earlier series; a dedicated
  sweep is debt for a later PR, deliberately not smuggled into this
  one.

## 5. Verification summary (final tree)

`test_encdiff.py` 66/66 · `test_asmdiff.py` 19/19 ·
`test_ci_gate_parity.py` 24/24 · `ensure_gas_247.sh --self-test` 12/12 ·
`check_ci_gate_parity.py` PASS (132 commands) · encdiff corpus gate
green vs the pinned pair at the unchanged recorded baseline, 208 rows
BEATS=175 ORACLE-INVALID=10 ok=23, −559 B, digest-verified · x86-64
asm-diff 1418/1418 and i686 asm-diff 672/672 under the pinned pair and
the WORKING undecodable refusal · fastbuild 0 warnings · rustfmt clean ·
clippy (lib) clean · no swap possible on this runner (swapon EPERM,
unprivileged cgroup — 4G fallocate file created and removed after the
denial; the `-j2` recipe carried the build in 58 s).