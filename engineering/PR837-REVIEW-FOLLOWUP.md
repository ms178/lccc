# Review follow-up: temporary leases, vector transfers and receipt diagnostics

## Integration and scope

This follow-up is rebased onto main
`2f477ce01749824ccab73ead7ccac90abd77fa08`, which includes both PR837 and
PR836. Only the focused review delta remains above that base. Deliveries must
remain direct diffs against the latest main, with exact-tree application checks.

The review is useful, but not an authority. Its final merge condition is now
historical: the merged PR's head has thirteen successful checks. That does not
certify this follow-up. Compilation, native execution, MSRV and formatting are
CI's authority; no local C/Rust compilation or native witness runs are claimed.

The reported 40 files / +6,453 / -285 are confirmed by the PR files API.
The changed-line categories sum to 6,738, so 5,223 non-src/scripts/tests lines
are **77.52%**, not 81%. The reviewability concern remains valid; the incorrect
percentage is not evidence of a production defect.

## Adjudication

| Finding | Decision and action |
|---|---|
| M1 / FU-3: native prerequisites | Agree on contributor coupling; retain the owner's explicit hard-fail execution contract, not an opt-out or a successful skip. A single AVX/GCC preflight names both requirements and reports both results. An unconditional sibling runs the real peephole pipeline and its self-copy positive control without AVX or external tools. The native test still builds and executes both original and optimized fixtures. |
| M2 / FU-5: retained archive size | A larger file alone is not a correctness or security defect. Add a 1 MiB default external-storage threshold and explicitly grandfather the frozen offline-verifiable runtime archive. Do not prune receipts, change their hashes or turn an expiring Actions URL into the only permanent evidence. |
| M3 / FU-1: prefixed transfers | Agree: literal-only call matching misses staged upper-register observations. The suggested mask-only change is insufficient: local prefixed branches also need correctly decoded CFG successors. One allocation-free decoder serves both paths. Near/far call widths, stacked legacy/segment/size prefixes and opaque far jumps are conservative; a local label cannot prove a far transfer stays in the same code segment. |
| L1/L2 / FU-2: lifetime and ownership prose | Agree. Describe the managed-lease / persistent-debug split, pending reservations, kept files, best-effort empty-directory cleanup and exclusive creation rather than globally unique timestamp names. The active code already replaced `make_temp_path` with fallible reservations, so do not resurrect that removed API in documentation. Unix mode 0700 is user-private, not an isolation boundary against other processes with the same uid. |
| L3 / FU-6: brittle policy tests | These are intentional design-budget and immutable-history tripwires, not wall-time or correctness proofs. Annotate their purpose and require a justified content/design change and evidence when updating a pin or limit. Keep the existing hash and refresh ceiling unchanged. |
| L4 / FU-4: secondary receipt error | Reproduced, not merely hypothetical. A missing/corrupt native receipt masks a launch exception; the analogous compile-receipt read also loses a compiler failure. Non-object JSON can raise an uncontrolled AttributeError. Validate the object shape and share a receipt-retention rule: preserve the primary error, record the secondary error without inventing a successful receipt/hash, and still fail when evidence alone is missing after a successful process. |

## Regression authorities

- The original call-boundary tests are unchanged. New Rust coverage checks
  **31 prefix patterns x 8 near/far call mnemonics x 16 tracked upper aliases**,
  with both blocked-promotion and full-overwrite-positive arms. Explicit direct,
  indirect and far review examples check the full conservative mask.
- Backward local jmp/jcc edges must preserve a reachable wide read. Forward
  unconditional branches that skip a read must still allow promotion. Fixing
  only the observer check would fail both directions. Operand substrings are
  not prefixes. Tests deliberately include structural-only width/prefix forms;
  they do not claim long-mode GAS accepts every combination. This decoder is
  local to final promotion, not a general GAS parser or arbitrary-prefix
  support in other peephole passes. The emitter currently uses unprefixed
  transfers; adding emitter forms needs those passes' contracts and full-pipeline
  witnesses too. The existing inline-assembly veto is unchanged.
- Mocked real Callgrind orchestration exercises both arms, OSError/ValueError
  spawn failures, missing/malformed/non-object receipts, compiler nonzero exits
  and receipt loss after a successful process. No compiler/program/Valgrind is
  executed by these tests. With the first three new methods installed against
  the unmodified implementation, 23 fault states produced 8 assertion failures
  and 14 errors; the already-rejected malformed-JSON reader state passed.
  These are manifestations of the related diagnostic defects, not 22 bugs.
- Four new diagnostic methods now cover 29 fault states. Missing evidence after
  success is still fatal; no numerical result or successful hash is fabricated.
- Affected local suites: **146 unittest methods passed**, including 42 Callgrind
  methods and 73 gate-parity mutation methods. The separate cache self-test
  passed 18 checks. Gate parity verifies 234 commands / 24 invocation contracts.
- `verify_evidence_hashes.py`: **69 files**, unchanged closed inventory.
  `verify_corpus_receipts.py`: **14,639 files / 225 program pairs / 3,600 linked
  process receipts**, 44 ELF-equal pairs and stored program-Ir range [1,1].
  This is integrity/linkage verification, not fresh execution or proof of
  current performance; captured compiler-input checks are intentionally false.

## Preserved invariants and limits

The production `PropagationLiveness` implementation, its eager/lazy
64-seed differential test, managed directory lease design, `_job_reachability`
fail-closed parser and hash-locked receipt content are unchanged. A finite
64-seed differential test is strong equivalence evidence, not a formal proof
of all CFGs. Caller-saved clobbers still occur after argument observation;
recovering narrower call masks later requires an ABI/staging proof, not a
claim that precision changes are risk-free. An upper-vector return ABI is not
established by the call-argument witness.

The prefix decoder changes a safety proof, not a measured optimization win.
Its positive controls protect legitimate promotion; no elapsed-time gain,
tenfold advantage, universal compiler ranking or exhaustive correctness is
claimed. CI must validate the final source, not inherit the merged PR's green
checks or reinterpret old receipts as new measurements. Measurement-claims
language and historical data remain intact.

Reproduce the local read-only/mocked checks (PyYAML is required):

```sh
python3 scripts/verify_evidence_hashes.py
python3 scripts/verify_corpus_receipts.py
python3 -m unittest tests.corpus.test_callgrind_contracts \
  tests.corpus.test_evidence_integrity tests.corpus.test_pr834_review_documentation \
  tests.corpus.test_pr834_corpus_receipts tests.corpus.test_ci_followup_collection \
  tests.corpus.test_parallel_ci_layout scripts.test_ci_gate_parity
python3 scripts/check_ci_gate_parity.py
python3 scripts/test_godbolt_cache.py
```
