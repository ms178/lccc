# LINKGATE-1 — the relocation oracle-verdict predicate, disclosed and fail-closed

**Status:** shipped. **Scope:** `tests/linker/run_linker_tests.py` only — no
compiler code. **Upstream:** the predicate change landed inside PR #674
(`31f8f44e`); the fail-closed floor and the unit tests land here.

---

## 0. Why this document exists

A code-review pass (2026-09-29) raised one finding rated HIGH, and the rating
was about **packaging, not about the code**:

> An unrelated test-gate relaxation, absent from title, commit message and PR
> description.

The reviewer is right that the omission was a defect, and the omission was
mine. The predicate change rode along inside a PR titled "Vectorizer: omit
provably-dead scalar remainder loops; add causal slot census", and a reader of
that PR had no signal that cross-oracle validation had changed.

This document is the disclosure that should have shipped with it, plus the
hardening that makes the reviewer's second finding impossible.

**Verdict on the finding:** agreed in full. A gate that gets *weaker* must be
impossible to merge invisibly. Note also what the reviewer's summary did not
say: the same hunk **strengthened** one path. The old code was

```python
if agree and not all(ok for _, ok in agree):
    FAIL
```

so an empty `agree` — no oracle ran at all — was a **vacuous PASS**. The new
code fails on `not agree`. The change was therefore not purely a relaxation,
and any honest accounting has to say both halves.

---

## 1. The defect the change fixed

`reloc_pc32_out_of_range_diagnosed_{exec,script}_path` asks: *lccc refuses an
`R_X86_64_PC32` whose value does not fit; does the ecosystem agree?*

The old oracle rule was

```python
o.returncode != 0 and (label == "shared" or b"truncated" in o.stderr)
```

`"truncated"` is **bfd's spelling** ("relocation truncated to fit"). mold and
lld say **"out of range"**, and the psABI mandates neither. Consequence,
observed: merely *installing mold* flipped two passing tests to

> oracles disagree with the refusal … lccc is inventing a restriction

for a linker that is conforming — and printed no diagnostics, because the
mold run's stderr was never shown. Demanding one implementation's diagnostic
wording from all three is a conformance bug in the test, not in lccc.

The new rule classifies each oracle's answer instead of matching a substring:

| verdict | meaning | counts as |
|---|---|---|
| `accepted` | it linked the input | **disagreement** (lccc refuses what the ecosystem accepts) |
| `refused` | refused **and** named a range failure for this relocation type | agreement |
| `silent` | refused without naming the type or a range | **disagreement** |
| `inapplicable` | never **reached** the relocation | excluded from the agreement set |
| `errored` | exited outside `{0,1}` — crash, abort, OOM kill | **disagreement**, never excluded |

The concrete `inapplicable` case is real and named: mold 2.37 cannot parse the
minimal `-T` linker script this fixture uses at all ("unknown linker script
token"), so it has no opinion about the relocation.

---

## 2. What genuinely weakens, and the floor added for it

The reviewer's second finding is the load-bearing one, and it is correct:

> `inapplicable` has no floor; the gate degrades silently from N-way to 1-way.
> `inapplicable` is *inferred* from the absence of a substring in stderr — an
> oracle that crashes, OOMs, times out, or formats the relocation name
> differently is indistinguishable from one that never reached the relocation.
> A flaky oracle is silently converted from a red test into a green one.

With the normal three-oracle set, losing two to `inapplicable` left **one**
linker deciding the whole test. A "cross-check" resting on a single opinion is
not a cross-check.

Three changes close it, all in `reloc_oracle_agreement`:

1. **`errored` is a verdict of its own**, classified *before* the `shared`
   shortcut. This matters more than it looks: the `shared` path accepts *any*
   refusal (GNU ld words its `R_X86_64_32` refusal its own way), so a
   segfaulting oracle on that path used to be scored as agreement. The
   reviewer's proposed placement — checking the exit status only in the
   non-shared arm — would have missed exactly that path.
2. **Applicability floor** `len(applicable) >= min(2, len(oracles))`, not a
   flat 2. A flat 2 would turn every single-linker host (this sandbox has only
   bfd) into a spurious failure; `min(2, …)` keeps the weaker-but-real
   conformance check where only one oracle exists and demands a real
   cross-check where more than one is configured.
3. **The reference must have an opinion.** `oracles[0]` is always bfd, the
   psABI reference. If the reference never reached the relocation, the fixture
   did not exercise what the test claims, and no amount of agreement from the
   others repairs that.

And degradation is no longer *silent*: a PASS that rested on fewer oracles than
were configured says so in the result detail
(`agreed by 2 of 3 oracles; inapplicable: mold(inapplicable)`), so a narrowing
gate is visible in the log rather than invisible by construction.

---

## 3. Making the classifier testable instead of only exercisable

The reviewer's proposal left `_oracle_verdict` as a closure inside the test
body. That is the wrong place for a function that decides PASS/FAIL: on a host
with one linker installed, the interesting branches — two oracles going
`inapplicable`, one crashing, the reference having no opinion — **never execute
at all**, so an end-to-end-only suite reports coverage it does not have.

Both halves are therefore lifted to module scope and unit-tested:

* `reloc_oracle_verdict(oerr, orc, kind, label)` — the five-way classifier.
* `reloc_oracle_agreement(agree, notes, oracles)` — the floor, the reference
  requirement, and the PASS detail.

`tests/linker/test_reloc_oracle_verdict.py` pins **30 known-answer cases**
(20 verdict, 10 agreement) whose expectations come from the psABI contract,
not from the implementation: exit statuses 0/1/2/127/134/137/139, bfd vs mold
vs lld wording, the `shared` shortcut, a diagnostic naming the *wrong*
relocation type, and each of the three fail-closed paths. It is wired into
both `ci.yml` and `scripts/ci_local.sh` as gate `linker-oracle-verdict`, and it
runs **outside** the `if [ -x target/fastbuild/lccc-ld ]` guard because it
needs no built linker and no oracle linker — which is precisely why it can
cover the paths the fixture route cannot.

`check_ci_gate_parity.py`: **PASS (91 commands)**.

---

## 4. Separability

This is, and always was, reviewable independently of any compiler delta: it
touches one test-harness file and no compiler code. It is documented here
rather than split into its own PR only because it is already merged upstream
inside `31f8f44e`; from here on it is a self-contained change with its own
evidence and its own gate.

## 7. Environment prerequisites the linker suite actually needs

The suite exercises an i386 path (`LCCC_REQUIRE_I386=1`) and a `c++ lib link`
case, so a host that can only compile 64-bit cannot run it. On a stock Debian
trixie container the missing pieces are not obvious from the failure messages,
because they surface as preprocessor errors inside system headers rather than
as "no 32-bit toolchain".

Fully green (`301 pass, 0 fail, 0 warn, 0 skip`) needs four things:

| # | requirement | symptom if absent | provisioning |
|---|---|---|---|
| 1 | 32-bit libc headers | `fatal error: bits/libc-header-start.h: No such file` | `apt install libc6-dev-i386` |
| 2 | 32-bit gcc multilib | `gnu/stubs-32.h: No such file` | `apt install gcc-multilib` |
| 3 | 32-bit libstdc++ headers | `fatal error: bits/c++config.h: No such file` (from `<stdexcept>`) | `apt install libstdc++-14-dev-i386-cross`, then symlink `/usr/i686-linux-gnu/include/c++/14/i686-linux-gnu/{bits,ext,64,x32}` into `/usr/include/i386-linux-gnu/c++/14/` |
| 4 | 32-bit libstdc++ library | `cannot find -lstdc++` at link time | symlink `/usr/i686-linux-gnu/lib/libstdc++.{a,so.6.0.33,so}` into `/usr/lib/gcc/x86_64-linux-gnu/14/32/` |

Item 3 is the awkward one: Debian trixie ships no native
`libstdc++-14-dev-i386` package, only the `-cross` variant, which installs into
`/usr/i686-linux-gnu/...` instead of the multiarch path `g++ -m32` searches.
The symlinks bridge the two layouts. Without them the suite reports 300/301
with a single, entirely misleading `i386_dso_emit_semantics` failure whose text
blames a system header rather than the host configuration.

The same gap (items 1 and 2 only) is what makes four *unrelated* gates fail on
a bare container — `nocfi-peephole-parity`, `reassoc-latency`,
`map-i64-two-lane` and `i686-integer-isa-parity` — because each of them shells
out to `gcc -m32`. Their failures look like C compiler bugs and are not: every
line is a preprocessor error about `bits/*.h`.

Worth stating plainly because it cost real time this session: a red `-m32`
gate is far more often "this container has no 32-bit libc" than "the compiler
regressed". Check for preprocessor errors about `bits/` headers before
suspecting the pass.
