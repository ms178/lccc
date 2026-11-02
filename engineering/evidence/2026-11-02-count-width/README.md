# A count's width is the width of what it counts — 2026-11-02

The design and the reasoning live in
[§20 of `SCALAR-PREDICATE-FOLDS.md`](../../llvm-missed/SCALAR-PREDICATE-FOLDS.md);
this directory holds what a re-run needs.

## The pass

`src/passes/bit_idioms_count_width.rs`, dispatched from `bit_idioms_facts::fold`
right after `count_merge`.  Two families, one rewrite shape each:

| shape | example | rewritten to |
| --- | --- | --- |
| cross-width mask | `popcountll(x) & 1` (count `U64`, mask `I32`) | `Parity@U64(x)`, truncated |
| promoted narrow count | `__builtin_parity(u8)` → `count@U32(zext u8)` | `Parity@U8(x)`, zero-extended |

Both emit at most one temporary: the operation at the narrow width into the
driver's `fresh` id, then a `Cast` back to the consumer's type.  Refusals: a
shared count (`uses != 1`), a sign-extending chain, a mask that is not 1, and
the same-width shape (which `fold_popcount_mask_to_parity` owns — the two rules
compare *sizes*, so an `unsigned` count masked by an `int` literal is still
same-width and stays with the simplifier).

## The gate

```sh
CCC=target/fastbuild/lccc bash tests/regression/check_parity_and_popcount.sh
```

Eight parts, ~2 s: the x86-64 parity shape (`setnp`, no `imul`, no branch) and
`-mpopcnt`; the byte count's two-multiplier form (no `push`/`pop`, no
`0x55555555`); the attribution (`CCC_SIMPLIFY_SKIP=parity` restores the
same-width count but must *not* restore the cross-width one); the AArch64 and
RISC-V shapes and pinned counts, including the narrow shapes staying at their
pre-round numbers; the RISC-V count being branch-free; pinned x86-64/i686 counts;
and value agreement with `tests/regression/parity_and_popcount.c`
(`failures=0 hash=09dc4aea9979cf33`) at `-O0`/`-O1`/`-O2`/`-O3`/`-Os` and
against GCC 16.2 at `-O2`.

## The before/after table

```sh
./ab.sh <baseline-bindir> <candidate-bindir>
```

`<baseline-bindir>` is a driver directory built from `origin/main` (here, a
worktree of `d2e6814` built with `CARGO_TARGET_DIR=/tmp/maintarget`); both
directories must hold `lccc`, `lccc-i686`, `lccc-arm` and `lccc-riscv`.
`count.py` counts instruction lines per function in a `-S` dump — no
directives, labels or comments, as every table in the design doc does.

`-O2`, instructions inside the function body including `ret`:

| probe | x86-64 | i686 | AArch64 | RISC-V |
| --- | --- | --- | --- | --- |
| `popcountll(x) & 1` | 22 → **11** | 27 → **16** | 15 → **14** | 29 → **18** |
| `__builtin_parityll(x)` (control) | 11 = 11 | 16 = 16 | 13 = 13 | 17 = 17 |
| `__builtin_parity(u8)` | 8 → **5** | 10 → **6** | 14 = 14 | 16 = 16 |
| `__builtin_parity(u16)` | 8 → **5** | 10 → **7** | 14 = 14 | 18 = 18 |
| `__builtin_popcount(u8)` | 17 → **8** | 19 → **11** (10 when the argument load's mask and the narrow prelude's `andl $0xff` coincide) | 13 = 13 | 24 = 24 |
| `__builtin_popcount(u16)` | 17 = 17 | 19 = 19 | 13 = 13 | 26 = 26 |
| `__builtin_popcount(x) & 1` (control) | 8 = 8 | 13 = 13 | 13 = 13 | 16 = 16 |
| `1u << (32 - clz(x - 1))` (control) | 9 = 9 | 17 = 17 | 16 = 16 | 32 = 32 |

The two controls are the point: the first row lands on exactly the body the
`__builtin_parityll` control already had, and the two shapes this round does not
touch (same-width mask, `next_pow2`) do not move.  For reference, GCC 16.2 and
Clang 23.1.4 at `-O2` on the x86-64 column: 10 / 10, 10 / 10, 4 / 4, 5 / 5,
5 / 6, 5 / 18, 7 / 7, 6 / 7 — GCC's five for the `u8`/`u16` counts are a
`__popcountdi2` call, which is the linking decision §18 documents.

The AArch64/RISC-V columns are the *reason* the narrow-count rule is gated to
the x86 back ends: their cast emitters mask and copy a zero extension, and their
narrow count paths still run the full-width sequence, so narrowing there would
cost the extension and save nothing.  `other_targets_keep_the_promoted_form` in
the unit tests pins that gate, and the gate script pins the resulting counts.

## The proof

`src/passes/bit_idioms_count_width_tests.rs` evaluates each rewrite with an
independent IR evaluator — binops ring at their type, casts extend by the
source's signedness, counts read their `ty` bits — exhaustively:

* every width pair (`U8`/`U16`/`U32`/`U64` count × mask) over `0..256` and the
  64-bit edges, against `popcount(x) & 1`;
* the composed shape (`popcountll(zext u8|u16(x)) & 1`) over `0..=0xffff` for
  every `from` type, which is the measured `popcountll & 1` family;
* every zero-extension pair for both operations (a parity narrows at any
  narrower width; a popcount only where the algorithm changes);
* the refusals (sign extension, `uses != 1`, mask ≠ 1, same width) and the
  fresh-id contract (one temporary, in the driver's `fresh` slot);
* the target gate: on AArch64 the narrow rule declines and the cross-width rule
  still folds, without narrowing.

The byte count's five-instruction multiplier form is exact for all 256 inputs
(`imul $0x08040201`, `shr 3`, `and $0x11111111`, `imul $0x11111111`, `shr 28`),
checked before it was emitted and again by the gate's value section, whose hash
is unchanged from before the round.
