# Round 12 — review follow-ups on the #789 findings, and what measuring did to my own work

Scope: the eleven findings raised against PR #789 (round-11 content, reviewer score
7.2/10, 0 P0 / 5 P1 / 4 P2 / 2 P3), re-checked on `arena/round12` on top of upstream
`6dd8eeea`. Each entry states the verdict, the measurement that produced it, and the
gate that keeps it from rotting. Three of them changed *my own* round-11 conclusion;
one of them (F09) turned out to be a live over-read in the x86-64 backend and is now a
fix rather than a test request.

## Verdict table

| # | finding | verdict | outcome |
|---|---|---|---|
| F01 | GNU nested-`auto` / shadowing in the macro preprocessor | **disputed, unresolved** | see §1 |
| F02 | `.p2align` cap law generalised from one section | **agree, fixed (again)** | the law has an input I had hard-wired wrong — §2 |
| F03 | reference killed by a signal treated as "ineligible" | **agree, fixed** | `reference-fail` for negative rc and 124/125/126/127 in *both* phases |
| F04 | torture evidence compared by counts, schema-gated validation | **agree, fixed both sides** | shared `case_key`/digest in `tools/corpus/`, strict contract, 15 witnesses |
| F05 | Expat workload compared on `len(cases)` | **agree, fixed** | identity-multiset + status + configure-manifest diff, 13-check `--self-test` |
| F06 | include-graph digest could miss a header | **agree, fixed** | transitive quoted-include digest, pinned pre-run, drift re-checked post-matrix |
| F07 | a crashed worker silently shrank the matrix | **agree, fixed** | guarded `future.result()` → `fatal`, futures cancelled, `complete:false`, exit 2 |
| F08 | dispatcher goldens ratchet certified a hole | **agree, fixed** | structural scan on both sides, empty-set is a loud failure, 12 new fixtures — §3 |
| F09 | more aggregate-ABI fixtures wanted | **agree, and it found a bug** | new partial-eightbyte fixture + boundary sizes; over-read fixed — §4 |
| F10 | duplicate multilib provisioning step in CI | **agree, fixed** | one step, comment says why the other was deleted |
| F11 | (round-11 leftover) | closed by `7a9b6c1d` | — |

## 1. F01 — conceded in part, and the part that was real was a fail-open

The finding was framed as a scoping difference for nested `auto` symbols and shadowed
macro parameters. There is no such scoping in this backend to differ *about*, which is
why I could not build the counterexample -- but the search found something worse next to
it, and that is now fixed.

Measured, with a valid altmacro fixture (`.altmacro`, a macro containing `local lbl`,
invoked twice):

```
$ riscv64-linux-gnu-as f.s -o f.o          # assembles
$ lccc --target=riscv64-unknown-elf -c f.s
ccc: error: unhandled instruction: local [Memory(Symbol("lbl"))]
```

`local` is not implemented -- it reaches the writer as an unknown *instruction* -- and
`.altmacro` was sitting in `riscv/assembler/parser.rs` under "Other ignorable
directives", beside `.file` and `.ident`. That combination is the defect: a directive
whose whole purpose is to change how every macro argument in the file expands (`\()`,
quoted arguments, `!` repeat counts, `%` expressions) was accepted as a no-op. A file
that uses none of those features assembles identically either way, which is what made
it look safe; a file that uses them assembles into bytes the author did not ask for,
with no diagnostic at any stage.

`.altmacro` is now refused at the switch, naming both the mode and `local` as the thing
the user hits next (`altmacro_mode_is_refused_not_ignored`); `.noaltmacro` stays
ignorable, because non-altmacro mode *is* this backend's mode. `.purgem` stays lenient
with a comment saying so: forgetting a purge can only let an input GAS rejects assemble,
never change the bytes of a valid one. The narrow scoping question stays unanswered --
I still have no executable `auto`-nesting counterexample, and I am not inventing one.

## 2. F02 — the law has an input, and I had it wrong

Round 11 fixed the headline defect (`.text`-only measurement generalised to `.data`,
plus `parser.rs` zeroing `tail.max_pad`) and left the cap decision at
`fill.is_none() && section_is_exec`. That is still two conjuncts short, and the missing
ones are not cosmetic.

GNU as reaches the "cap is dropped" behaviour only through `tc-riscv.h`'s `md_do_align`
hook, which fires for `fill == NULL && subseg_text_p && !need_pass_2`, and whose body
(`riscv_frag_align_code (int n)`) returns false — putting the directive back on the
generic `frag_align (n, *fill, max)` path that *does* honour the cap — when
`n == 0`, when `1 << n <= insn_alignment`, or when `!riscv_opts.relax`. The
`insn_alignment` there is `riscv_opts.rvc ? 2 : 4`.

I had that second constant as 2 on the reasoning "lccc assumes the C extension". It
assumes the opposite: `apply_option` accepts `rvc`/`norvc` and ignores them, because
this backend cannot emit compressed instructions, so the emitter's instruction width is
4. The dependency is measurable without any of that argument — on the local
`riscv64-linux-gnu-as` 2.44, `.text; .byte 1; .p2align 2,,1; .byte 2` pads **0** bytes
at `-march=rv64im` and **3** at `-march=rv64gc`. Same assembler, same input, one flag.

So the predicate is now `fill.is_none() && exec && align_bytes > 4 && !self.no_relax`,
and the `.option norelax` leg is reachable from source (`Directive::ArchOption`) rather
than only from a command line. `riscv_align_cap_matrix_matches_the_hook` pins 22 rows —
`.text`/`.data` × fill/`fill,max` × relax/`norelax` × four exponents × several caps —
with the expected padding taken from measurement, not re-derived in the test. Two rows
carry the reasoning that a table cannot express: `.p2align 1` is swept over every cap to
state that the hook's `n != 0` condition is *unobservable* at `n == 1` (the pad is 1
byte and no expressible cap can cancel it), and the same sweep is deliberately in
`.data`, because `.text` at that exponent depends on the mapping-state pre-align that is
still open in §6.

Mutation check: restoring `INSN_ALIGNMENT = 2` fails the new test on the
`.p2align 2,,1` row; the fixed constant passes.

## 3. F08 — a ratchet that reads structure, not layout

Both copies of the dispatcher goldens enumeration (`scripts/gen_aarch64_dispatcher_goldens.py`
and `dispatcher_goldens_ratchet_one_row_per_arm` in `encoder/mod.rs`) delimited the
region by "first `}` in column 0" and matched arm heads against raw text. Three ways
that is a hole, all now pinned by `tests/aarch64/test_dispatcher_region_contract.py`
(12 tests, toolchain-free, gated as `dispatcher-region-contract` in `ci_local.sh` and
mirrored in `ci.yml`):

* a raw string containing a line-initial `}` truncates the region — to zero arms, which
  a ratchet reports as "all 0 rows verified";
* a commented-out `"cas" =>` head counts as an arm;
* a `b`/`r` immediately before a quote opens a phantom literal, so masking must require
  a token boundary. The third one was not a fixture invention: `"xzr" } else { "wzr"`
  contains exactly that shape, and my first scanner read the `r` of `"xzr"` as a raw
  prefix, blanked `} else {`, unbalanced every depth downstream and lost 403 of the 526
  arms *while the header still looked plausible*.

Both sides now close the region by brace depth over a `code_view` (comments and literal
contents blanked, offsets and line breaks preserved so slices of the view index `src`
directly), match heads in that view while reading the *key text* out of the original,
and treat an empty arm set as a hard failure. `code_view` asserts its own length against
its input: a misaligned view shifts every offset the ratchets depend on, which is a
silent-wrong-answer bug, so padding was replaced with `assert_eq!`.

`dispatcher_arms()` reproduces `# arms: 526` from `encoder/mod.rs`, which is the latch
between generator and consumer — measured both ways: with the correct scanner the number
comes out of the file, and the Rust assertion `!arms.is_empty()` fired during
development precisely when my depth walk stopped at the *inner* `}` of
`encode_instruction` (123 arms) instead of the enclosing `mem_rm` item the TSV actually
covers. The two enumerations disagree about the region *only* by intent: the TSV's
contract is the wider region, and the header count is what makes an accidental
narrowing loud in both gates instead of quietly reducing coverage.

One deliberate non-fix: a behavioural case *per three-same leaf* would need whole-crate
reachability (these leaves are reached via `encode_logical` and friends — `"and"` maps to
`encode_logical`, not to `encode_neon_logical`), so the census says so in a comment
instead of carrying a hand-maintained table with per-leaf exemptions. The census's own
check was still made structural: `neon_shapes_agree(operands, 3` must be live code in the
leaf *and* must precede the leaf's first `Ok(`, so a commented-out call or one placed
after the encoding word is built no longer satisfies it.

## 4. F09 — extra fixtures, and what they caught

`tests/regression/check_aggregate_call_abi.sh` swept `BLOB_BYTES` from 16 upward, so the
partial-word cases below the 16-byte register-class boundary were never built, and the
fixture's aggregates are `unsigned char[N]`, which is *always* INTEGER-classified — the
SSE and mixed classes could not be reached at any size.

Widening the sweep to 1, 7, 8, 9, 15 immediately failed: with lccc as the **caller**,
`-m64 -O0` at sizes 1, 7, 9 and 15 SIGSEGVs (rc=139), while size 8 and every
`gcc`-built caller were fine. That profile is qword rounding, and it is in
`emit_call_reg_args_impl`: a register-classified aggregate was loaded one whole eightbyte
at a time (`movq 0(%rax)`, `movq 8(%rax)`), so a 9-byte struct read 7 bytes past the end
of the C object. The sibling path for *stack*-passed aggregates already refused to do
this — its comment says ABI rounding "does not license a read past the C object" — which
is what made the omission worth fixing rather than documenting: the rule was already
accepted in the same file, in the other half of the same function.

Fixed by `emit_small_aggregate_eightbyte_load`, which takes the in-bounds whole-eightbyte
fast path when `len >= 8` and otherwise stages the 1..7 defined bytes through a balanced
16-byte transient in the outgoing area, using no register but %rax (parked and restored,
because every other GP is either an argument register or already staged). Applied to all
four register-side classes: `StructByValReg`, `StructSseReg`, `StructMixedIntSseReg`,
`StructMixedSseIntReg` — the last two also stop being able to ignore their `size` field,
so their `#[expect(dead_code)]` attributes are gone rather than left to rot.

`tests/regression/small_aggregate_partial_eightbyte.c` is the fixture for the classes the
byte-array sweep cannot reach: a 12-byte `float[3]` ([SSE, SSE], 4 defined bytes in the
tail eightbyte), a 9-byte packed `{double; short}` ([SSE, INTEGER], 1 byte), a 7-byte
byte array (single partial eightbyte) and a 13-byte one ([INTEGER, INTEGER], 5 bytes),
each placed flush against a `PROT_NONE` page so an over-read is a fault rather than a
neighbour read, with the callee folding **exactly `sizeof` bytes** and neighbouring
scalars riding between them. `packed` is load-bearing: unpadded-free `{double; short}`
is 16 bytes with 6 unspecified padding bytes, which both probes nothing and lets the two
compilers differ by design.

The same defect was present on i686, one layer over: regparm passes a small aggregate as
`ceil(size/4)` words and the caller-side loader did `movl k*4(%eax)` per word, so the
last word read up to 3 bytes past the object. That is what the widened `-m32` sweep
caught *after* the x86-64 fix had landed -- the gate's `-m32 -O0 size=1` pair SIGSEGVed
with rc=139 while every `-m64` cell passed -- and it contradicts the last line of commit
`5dee9fe4` ("i686 needed no change"), which was true of the fixture at the two
optimisation levels I had tried and false of the backend. Fixed the same way: a partial
final word is staged through a transient word of our own stack using nothing but %eax,
parked and restored so the descending order that keeps an `%eax` destination last still
holds.

Measured, all three compiler pairings (`lccc:lccc`, `lccc:gcc`, `gcc:lccc`): 240
blob-matrix executions (`-m64`/`-m32` x five `-O` levels x sizes 1, 7, 8, 9, 15, 16, 17,
2049) plus the partial-eightbyte fixture at `-O0`/`-O2` all pass. Mutation checks:
restoring the unconditional `movq` on x86-64 makes the new fixture SIGSEGV (rc=139), and
removing the bound on i686 makes `-m32` SIGSEGV at sizes 1 and 7 (rc=139).


The script also gained a latch of the kind this review keeps finding missing: the header
advertises what the fixture probes (guard pages, two aggregate arguments, scalar/F64
neighbours, regparm → cdecl, fastcall, over-aligned locals), and those claims are now
`grep`-checked against the fixture, so a comment cannot outlive the code it describes.

## 5. Evidence-pipeline findings (F03–F07) — one pattern

Every one of these was the same defect wearing different clothes: **a differential gate
proved by comparing a count or a substring instead of an identity, and an error class
recognised in one phase but not the other.**

* F03: a reference killed by a signal during the *run* phase was recorded as
  `reference-run-skip` — "not eligible" — while the same signal in the *compile* phase was
  a failure. Both phases now classify negative rc and 124/125/126/127 as
  `reference-fail`; an ordinary `exit 1` carrying eligibility text stays a skip.
* F04: producer and consumer each had their own idea of what a case was. They now share
  `tools/corpus/evidence.py` (`case_key`, `case_set_digest`) so the digest cannot be
  computed two ways, and the consumer reports *all* defects instead of the first:
  unknown status, undeclared flag, duplicate `(test, flags)`, missing or mismatched
  `case_set_sha256`, non-zero `fatal`, `complete:false`, count mismatch. `schema ∉ {1,2}`
  is a hard refusal that `--allow-legacy-schema1` cannot open; schema 1 is marked
  `verified:false`; publication is atomic and a merely *failing* report is still written
  (exit 1), while a structurally unusable one is exit 2.
* F05: replaced `len(cases)` equality with testcase *identity* multisets, per-testcase
  status, and a CMake configure-manifest comparison, with an allowlist for the keys that
  legitimately differ (compiler, paths). The validator is compiler-free and gated.
* F06: the dependency digest is now transitive over quoted includes, pinned before the
  matrix runs and re-checked after it, with system headers named out of scope and the
  bound surfaced as `dependency_digest_truncated`.
* F07: a worker that raised is a `fatal`, remaining futures are cancelled, an
  `complete:false` report is published atomically, and the exit code is 2.

347 tests in `tests/corpus` cover these (all green), with 15 witnesses added for the
specific shapes the old code mis-classified.

## 6. Open, on purpose

1. riscv `.text` creation alignment and the mapping-state pre-align
   (`frag_align_code (rvc ? 1 : 2, 0)`): lccc does not emit the byte GAS's pre-align
   produces for a `.byte`-only text section. It is why §2's `n == 1` sweep lives in
   `.data`. Pinning it here first would freeze a half-modelled law.
2. `writer_base.rs` still carries stale wording about the same floor.
3. #786's `.macro` 2×-billing question is still open upstream.
4. F01, above.
5. The per-leaf behavioural latch for the NEON census (§3).
6. The four PRs (#781 adopt-perfect, #777, #776, #725) — the loop-invert census gate from
   #777 and the parser-level classification from #725 are the structural fixes for exactly
   the class F08 belongs to; both are still pending adoption.

## 7. Gate and verification record

Filled in after the round-12 verification run: `ci_local.sh --fast`, `cargo test --lib`,
`clippy -D warnings`, `rustfmt`, `check_ci_gate_parity.py`, `check_doc_links.py`,
`tests/corpus` discovery, `tests/aarch64` discovery, and the Expat self-test.

## 8. The four open PRs, re-checked against this tree (measured, not remembered)

| PR | state in HEAD | evidence |
|---|---|---|
| #781 x86 ABI lowering + torture coverage | **adopted** | `.github/workflows/ci.yml` provisions `gcc-multilib` (2 hits), which is exactly what makes the `-m32` half of the ABI gates runnable — this round needed it locally too, and installing it is what turned `check_aggregate_call_abi.sh` from an infra failure into a real `-m32` result. |
| #776 assembler parity + directive safety | **adopted** | `scripts/ci_local.sh:1165` runs `tests/linker/run_linker_tests.py --strict`, and the parity checker reports the "strict full linker suite" as a mirrored contract. |
| #725 guard x86 flag flow / observable RMW loops | **adopted** | `tests/regression/check_volatile_access_semantics.sh` exists and is gated at `scripts/ci_local.sh:433`. The note that "parser-level classification is still pending" was stale: `src/frontend/sema/analysis.rs` contains no asm string-sniffing at all — its four `contains(` calls are `defined_structs` set lookups — so the classification the PR asked for is not sitting in that file waiting to be rewritten. |
| #777 iv_widen soundness half | **partly adopted** | The soundness half is present and pinned (the scale-ring wrap fixture and its gate). Not adopted: `scripts/loop_invert_census.py` (absent from the tree, so no gate can call it) and the default-off memory licence. |

Why #777's census is the piece worth taking next: F02, F08 and F09 are the same bug
wearing three outfits — a gate whose predicate is a *textual proxy* for a property of
the program, checked by counting or by substring presence. A census that enumerates
loops through `syn` and asserts "every inversion the pass performs has a witness in the
executed suite" is the structural version of what I did by hand here (blank non-code,
delimit by depth, refuse an empty set, latch the count to the thing that consumes it).
Adopting it also removes the last judgement call from the NEON census described in §3.

## 7. Gate and verification record

Run on `arena/round12` with the tree clean at `HEAD`, cargo 1.99.0, `--profile
fastbuild`, in a sandbox recreated since round 11 (so the toolchain provisioning below
is part of the record, not an afterthought):

| check | result |
|---|---|
| `cargo test --profile fastbuild --lib` | **4285 passed, 0 failed, 7 ignored** |
| `cargo clippy --profile fastbuild --lib --tests -- -D warnings` | clean (two `code_view` lints fixed in `6a8a560a`) |
| `cargo fmt -- --check` | clean |
| `python3 -m unittest discover -s tests/corpus` | **Ran 347 tests, OK** |
| `python3 -m unittest discover -s tests/aarch64` (the new `dispatcher-region-contract` gate) | **Ran 12 tests, OK** |
| `tests/workloads/expat-2.8.5/junit_check.py --self-test` | 13 checks passed |
| `scripts/check_ci_gate_parity.py` | PASS (153 commands, 19 invocation contracts) |
| `scripts/check_doc_links.py` | OK, all markdown references resolve |
| `tests/regression/check_aggregate_call_abi.sh` | widened matrix green: `-m64`/`-m32` x 5 opt levels x 18 sizes x 3 pairings, plus the partial-eightbyte fixture |
| `bash scripts/ci_local.sh --fast` | 178 passed / 1 failed / 5 skipped; the failure was `linker-suite` -> `i386_dso_emit_semantics`, `fatal error: bits/c++config.h: No such file or directory` |

That `linker-suite` failure is infrastructure, and the record says so because a green
run in a warm sandbox would have hidden the fact: the sandbox was recreated between
rounds, so the apt-installed multilib packages were gone. `302 pass, 1 fail` was one
missing 32-bit C++ header, not a linker defect. After `apt-get install gcc-multilib
g++-multilib`, the same gate run alone reports `1 passed, 0 failed` with `302 pass, 0
fail` in its own tally. Both packages are what CI provisions (the `gcc-multilib` step is
#781's, see §8), which is why the fast lane is expected to need them.

A full `--fast` re-run on the final tree is what earns the pass stamp for the snapshot;
if it lands after this document is committed, its result is reported in the round
message rather than rewritten here.
