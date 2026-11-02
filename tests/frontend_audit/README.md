# Frontend differential audit corpus

The frontend is audited **data-driven**: every case here is run through lccc and
through independent oracles, and the *observable behaviour* is compared. A case
is a small C file plus, implicitly, the answer to "what must all three compilers
agree on"; a divergence is either a defect in lccc or a defect in the case, and
both get fixed.

```
python3 scripts/frontend_differential_audit.py            # every family
python3 scripts/frontend_differential_audit.py --family pp --verbose
python3 scripts/frontend_differential_audit.py --levels 0,1,2,3,s
```

The script needs `gcc-16` and/or `clang-23` (found through `PATH`, overridable
with `--oracle name=exe`) and nothing else: no pytest, no `column`, no `file`.
Run it after any frontend change — it takes a few seconds.

## Families

| Directory | Observable compared | What it can catch |
|-----------|--------------------|-------------------|
| `pp/` | `-E` output, reduced to a **token stream** (line markers dropped, every run of whitespace -- newlines included -- collapsed) | macro expansion, rescanning, blue paint, stringification, pasting, `#if` arithmetic, `__VA_ARGS__`/`__VA_OPT__`, line splicing, missing/extra tokens anywhere |
| `run/` | the program's stdout, stderr and exit status at every `-O` level | constant folding, integer promotions and conversions, `sizeof`/`_Alignof`/bitfield layout, enum values, character-set mapping, type selection (`_Generic`), designated initialisers, and anything else that changes a value |
| `diag/` | accept vs reject (`-fsyntax-only`), plus "no internal error" | missing diagnostics, spurious diagnostics, crashes |

`diag/` cases can carry a first line of `/* expect: accept */` or
`/* expect: reject */`. The expectation only matters when the oracles agree with
each other — when they disagree the case is not a conformance probe and must be
fixed or removed, exactly like a code defect.

## Why a token stream and not a text diff

`a+b` and `a + b` are the same program; `f(2 * (~ 5))` and `f(2 * (~)) 5)` are
not. A whitespace-normalised text diff cannot tell those apart — it reports both
and buries the real one. Tokenizing both sides makes the comparison exact, which
is what lets the corpus stay small and still be trusted.

## Adding a case

1. Put the smallest input that shows the behaviour in the right directory.
   Prefer the *standard's* example when one exists (C11 6.10.3.5 EXAMPLE 3 and 4
   are in `pp/`), because the expected output is then not our opinion.
2. Write a comment saying what the oracles do and why — the case is
   documentation of a rule, not just a screenshot of today's behaviour.
3. Run the script. If lccc disagrees, decide which side is wrong *before*
   changing anything; if the oracles disagree with each other, drop the case.
4. If the fix is in code, add a case that would have caught it at the level a
   user sees (usually `run/`), and keep the `pp/`/`diag/` case as the precise
   statement of the rule.
