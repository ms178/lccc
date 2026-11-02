# PR #683: why CI was red, and what the OBS-4 alarm actually meant

This records the round that fixed PR #683's failing `Test Suite` job, and a
measurement that contradicts the review audit's reading of OBS-4.

## 1. The red job, and why it was red

`Test Suite` failed at one step: the linker suite, run with `--strict`. The
failing case was `reloc_pc32_out_of_range_diagnosed_on_script_path`:

```
only 1 of 2 oracles could express an opinion, need 2:
  bfd:  ('refused', 1, ...)
  mold: ('inapplicable', 1, "mold: fatal: t.ld:1: ENTRY(probe) unknown linker script token")
```

This was **my own design contradiction**, not a host quirk. I had written a
verdict classifier whose docstring says a linker that cannot parse the script
is `inapplicable` and therefore excluded — and a floor that demands two
opinions. Those two rules cannot both hold on a host with mold installed, and
the losing side was CI.

The first fix I tried was removing `ENTRY(probe)` from the fixture. That is
correct on its own merits (see below) but it did **not** fix the test, and I
checked rather than assumed. Reducing the script to a bare `SECTIONS { ... }`
still failed:

```
mold: fatal: t.ld:1: SECTIONS {  ^ unknown linker script token
```

So the honest diagnosis needed a direct probe of each oracle:

| oracle | `-T` minimal script | note |
|---|---|---|
| `ld.bfd` | rc=0 | psABI reference |
| `ld.lld` | rc=0 | parses scripts |
| `mold` 2.37.1 | **rc=1** | advertises `-T`/`--script`, parser cannot read one |

mold is not missing one script *feature*; it cannot consume a linker script at
all. The fixture was a red herring.

## 2. The fix, and why each half is load-bearing

Two changes, because they fix two different host configurations. Both were
mutation-tested — I checked that each is actually doing work rather than
getting lucky.

**a. `lld` joins the oracle set.** The registry was `bfd` + `mold` + `wild`;
`lld` was never added, so the script-path cross-check had exactly one
script-capable oracle by construction. Adding it is the substantive fix: the
quorum becomes bfd + lld, two real opinions.

**b. A structural script-capability probe.** `oracle_script_capable()` links a
trivial object with a trivial script through the compiler driver and caches
the result. This replaces an *inference*: `inapplicable` used to be deduced
from the **absence** of a relocation-type substring in stderr, which cannot
distinguish "this linker cannot parse scripts" from "this linker parsed the
script but never reached the relocation". Those have different owners, and
collapsing them is what made a harness defect look like a linker defect.

`incapable` is now its own class. It is excluded from the opinion set, and the
quorum is taken over the oracles that *could* answer. The floor is never
relaxed for opinions.

### Mutation evidence

| configuration | probe | result |
|---|---|---|
| bfd + lld + mold | on | **PASS** (2 opinions) |
| bfd + lld + mold | forced on (disabled) | PASS — lld alone carries it |
| bfd + mold (no lld) | on | **PASS**, detail says `agreed by 1 of 2` |
| bfd + mold (no lld) | off | **FAIL** — exactly the original CI error |

So `lld` is what fixes a normal host, and the probe is what keeps the test
honest on a host without `lld`. Neither is decoration.

### Also fixed

* The fixture no longer uses `ENTRY(probe)`. It is a GNU ld script feature,
  and `probe` is not even defined by the fixture object, so it contributed
  nothing to a test about relocation range checks.
* `ci.yml` now installs `mold` and `lld` **explicitly**. The oracle set was
  whatever the runner image happened to ship, which means the strength of
  every cross-check silently varied between runs. A gate whose evidence
  depends on the image is not a gate.
* The suite's summary line printed a hardcoded `(oracles: bfd mold)`. It now
  prints the set actually used.

### Verification

```
== linker tests: 297 pass, 0 fail, 0 warn, 4 skip (oracles: bfd lld mold) ==
```

39 classifier cases (up from 32). The new ones pin the dangerous shapes:
`incapable` must not rescue a disagreement, must not rescue a *silent*
refusal, all-incapable must not be a vacuous PASS, the reference being
incapable must still fail, and — the subtle one — an `inapplicable` oracle
still counts against the quorum while only `incapable` frees a seat.

## 3. OBS-4: the review audit is right that it barely fires, wrong about why

The audit rates `loop_preheader` "medium risk: 528 lines CFG surgery
default-on with ~0 firing at default" and suggests measuring insertion rate.
Measuring it is easy to do badly, and my first attempt did exactly that.

The pass had one observable: a line per **insertion**. "Inserted nothing" and
"inserted nothing because the gate rejected everything" are different
findings, so I added a per-function census of rejection reasons
(`CCC_DEBUG_LOOP_PREHEADER`).

**My first census was wrong and said `loops=0` everywhere** — including on the
pass's own fixture, where an insertion visibly happened two lines earlier. The
counters were incremented through `&mut cell.get()`, which mutates a
*temporary* copy of a `Copy` value and discards it. Every counter read zero.
Silently-wrong instrumentation is worse than none, and the code now does a
read-modify-write-back. Flagging this because the wrong number was
superficially convincing and I nearly reported it as a finding.

With working counters, over `-O2` on the whole benchmark corpus:

| corpus | loops seen | inserted | already dedicated | no-profit | no single pred |
|---|---|---|---|---|---|
| `tests/benchmark/programs` (51) | 70 | **0** | **70** | 0 | 0 |
| `tests/bench` (11 kernels) | 9 | **0** | **9** | 0 | 0 |

**Every single loop the pass sees already has a dedicated preheader.** It is
not mis-tuned and not blind: the admission test `is_dedicated_to(pred, header)`
is satisfied by the ordinary CFG shape, so there is nothing to do. The pass
only acts when a loop's predecessor is *conditional* (a guard), which is why
its own fixture's `guarded_sum` fires and real code does not.

So the audit's reading — "possibly mis-tuned, consider gating it off" — is the
wrong inference from the right observation. The pass is a correctly
conservative safety net for a shape that rare code produces. Gating it off
would delete the only thing that handles it. What it deserves is the
measurement, which now exists, and the documentation of the number.

## 4. Duplication: the audit understated it

The audit records MINMAX-5/LOW-3 as "~110-line hoist duplicated verbatim,
~220 LOC". Measured by contiguous-match between
`transform_reduction_avx2` and `transform_reduction_sse2`:

* both functions are ~1350 lines
* **610 lines are byte-identical**, in 15 contiguous blocks
* the largest single block is **226 lines**

That is a far larger clone than reported. It remains deferred, deliberately,
and is not touched by this round: it sits in the reduction rewrite, and mixing
a 2700-line refactor into the commit that turns CI green would make the CI
fix impossible to review. It is recorded here with the real number so the
backlog estimate is corrected.
