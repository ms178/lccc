# Calling-convention attributes: the measured contract (2026-11-01)

GP-02 of [`GCC16-PARITY-ASSESSMENT-2026-10-27.md`](GCC16-PARITY-ASSESSMENT-2026-10-27.md)
is the only entry in the campaign register whose defect class is *silently
wrong executables*. Every other gap loses an optimisation, a warning or a
diagnostic sentence; a calling convention that one compiler honours and the
other ignores loses the ABI. The caller and the callee still link, still start,
and disagree about where every argument lives.

Measured on the pre-fix tree, with a four-integer function and its prototype
carrying the same attribute:

| program built by | `__attribute__((ms_abi))` callee | result |
| --- | --- | --- |
| GCC caller, GCC callee | GCC | correct |
| GCC caller, lccc callee | lccc (ignored the attribute) | returned `42`, not the sum |
| lccc caller, GCC callee | GCC | `SIGSEGV` |

Both mismatches compile without a diagnostic. That is the failure mode this
document describes the removal of.

## Why the old classification could not express the answer

The attribute registry sorts every GCC-known name into three classes: unknown
(gcc has no such name), *known but parsed without applying*, and implemented.
A convention attribute needs four answers, because "we do not apply it" is a
different thing in each of four situations:

1. the attribute names the convention this target already emits, so applying
   it is applying nothing (`sysv_abi` on x86-64);
2. gcc itself does not honour it on this target and says so (`stdcall` on
   x86-64), so the parity answer is gcc's sentence, not silence and not an
   error;
3. gcc knows the name and rejects the target for it in a fixed sentence
   (`'callee_pop_aggregate_return' attribute only available for 32-bit`);
4. the attribute selects a convention or a record layout this compiler does
   not implement, where the only safe answers are "implement it" or "refuse".

Folding (1) into "parsed without applying" is what produced the silent
miscompile: `ms_abi` was accepted, discarded, and the default convention was
emitted under a foreign-ABI declaration.

## The table

`src/frontend/parser/attribute_registry.rs` now carries
`ABI_VERDICTS: &[(&str, AbiEffect, AbiEffect)]` — one row per name, one column
per x86 target — and the parser asks it *before* the classification funnel, so
the funnel never sees these names. The five effects are

| effect | meaning | diagnostic |
| --- | --- | --- |
| `Default` | the target already emits this convention | none (gcc is silent too) |
| `GccIgnored` | gcc ignores it here and says so | gcc's own sentence, default on |
| `OnlyAvailable32Bit` | gcc's wrong-word-size sentence | gcc's own sentence |
| `Unsupported` | a foreign ABI or layout | error naming attribute, convention, target |
| `Implemented` | lccc lowers it (`regparm`, `fastcall` on i686) | none |

The verdicts were measured, not chosen. The full method is repeated in the
docstring of the module, and the table itself is the machine-checked form: a
gate re-derives every cell from a live gcc on every CI run, so a gcc that
changes its mind fails the gate instead of silently redefining what parity
means.

Two verdicts are deliberately *not* "ignore it because gcc does":

* `preserve_none` and `no_callee_saved_registers` are refused on both x86
  targets. Preserving extra registers is not a sufficient ABI proof: measured
  GCC 16.2 x86-64 `preserve_none` passes the first arguments in R12/R13, not
  the native RDI/RSI. Separate caller/callee translation units, identically
  named functions, and EBX/ESI/EDI clobber controls expose contracts hidden by
  arithmetic-only or differently named twins. Both preservation attributes
  therefore select `Unsupported`, never `GccIgnored`.
* `no_caller_saved_registers` is the opposite promise: the callee promises not
  to clobber caller-saved registers, and the caller may then stop saving them.
  lccc cannot yet prove that of a body it emits (gcc enforces it by diagnosing
  forbidden `xmm`/`x87` use, which we do not track), so honouring it silently
  could hand a caller a clobbered register. It is refused.

## Placement

`ms_struct` and `gcc_struct` are record-layout attributes: gcc honours them on
a `struct`/`union` tag and ignores them everywhere else, with the ordinary
`'x' attribute ignored` sentence. Measured: a record
`{char; int:5; int:3;}` is 4 bytes by default and 8 bytes under `ms_struct`, so
the tag placement is a real layout decision, not a no-op. The parser raises a
placement flag for exactly the tag-level attribute parse
(`Parser::parse_record_attributes`), so the table can answer both positions.

Unsupported ABI/layout verdicts deliberately reject some programs GCC accepts.
For example, lccc has no Microsoft bit-field layout yet; silently emitting the
SysV layout under an `ms_struct` tag would reproduce the ABI failure this work
removes. Likewise x86-64 `force_align_arg_pointer` is refused until entry-stack
realignment is implemented. On x86-64 `regparm` is ignored with GCC's warning,
not claimed as an implemented register-argument convention. GCC's own behaviour on the conflicting-attribute form is recorded for
whoever implements the layout: first attribute wins, the later one produces
`warning: 'ms_struct' incompatible attribute ignored`, exit status 0.

## Escape hatch

`CCC_ALLOW_UNSUPPORTED_ABI=1` downgrades every `Unsupported` verdict to a
warning. It is correct exactly when every object in the program is built by
this compiler — then no caller can be relying on the foreign convention. It is
a build-system escape hatch, not a compatibility mode: mixing objects across
compilers under it reintroduces the mismatch the table exists to prevent. The
knob is named in every message it produces, so a build that uses it also
documents that it does.

## What proves it

`tests/regression/abi_attribute_matrix.py` (wired into the attribute-diagnostic
gate, which both CI mirrors run):

* parses `ABI_VERDICTS` out of the registry,
* re-measures every cell against the live gcc — integer, floating,
  aggregate-return and record-layout twin bodies in both targets and both
  placements — and derives the expected verdict from *what gcc does*, so the
  table cannot rot,
* asserts lccc's side of every cell: silence for `Default`, gcc's verbatim
  sentence for the ignored classes, a named refusal for `Unsupported`, and the
  documented downgrade under the escape hatch,
* proves the two conventions lccc actually lowers with a cross-compiler
  runtime sentinel: gcc builds the callee and lccc the caller, then the
  reverse, and the freestanding program must exit with the value only an
  agreeing ABI can produce.

The registry's "known but not applied" inventory drops the thirteen names that
no longer belong to it, and the shell gate pins the new classes end to end.

## How the gate itself is kept honest

A gate that reports PASS while measuring nothing is worse than no gate, so the
matrix is held to the same standard as the compiler it polices.

**The oracle is pinned, not whatever gcc is installed.** `ORACLE_GCC = (16, 2)`
is the release every cell was measured on. On that release — or a newer one — a
disagreement is a failure; on an older gcc it is a note that names both
verdicts. This is not leniency, it is the only behaviour that can be correct:
gcc changes its mind about these names between releases (measured: `regparm` is
silent on x86-64 in 12.2 and emits `'regparm' attribute ignored` in 16.2), so a
table that followed the local gcc would flap instead of pinning a contract,
while a developer on an older release would see a red gate that no code change
could fix. Detection of oracle drift is unaffected — a *newer* gcc is still
checked strictly, which is how a future release changing a verdict gets caught.

**Every probe is bounded in wall-clock time.** Compiles get 120 s, links 60 s,
executed sentinels 30 s. A hung or thrashing compiler used to consume the job's
multi-hour budget and report nothing at all; now a timeout is a *failure with a
reason*, and a missing binary is a 127 result rather than a traceback. Gate
scripts must never crash, because a crash names no cell.

**A mismatch carries its evidence.** `verdict disagrees with gcc` alone names a
cell but no cause, which is unactionable from a check annotation where the log
is not readable. The observation now carries the two ABI signatures that
differ — `attr=[...] plain=[...]` — so the annotation usually names the
convention that changed.

**One resolved compiler per run.** The gate takes the driver and the oracle from
its caller (`LCCC`, `GCC`) instead of guessing a second path internally: two
independent guesses could measure one binary and name another.

**The census ratchet cannot lie about itself.** `scripts/audit_census.py` now
asserts that `knob_literals` equals `len(knob_names)` — two views of one set. A
baseline that disagrees with itself makes a metric look like it grew while every
diagnostic that explains a growth stays silent, which is exactly how a phantom
CI failure cost a cycle; `--check` and `--update` both refuse such an artefact
now, and a growth line names the new knob inline.

`tests/corpus/test_abi_contract_tools.py` pins all of the above without a
compiler: the parsed table's shape and measured cells, oracle strictness on a
pinned/newer/older release, the evidence renderer, bounded-subprocess behaviour
(timeout, missing binary), every lccc-side verdict branch against a specification
stub, and the census invariants.
