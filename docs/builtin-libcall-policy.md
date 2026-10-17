# Builtin / libcall synthesis policy

Several optimisation passes replace user code with a call to a standard
library function, and one pass rewrites a call to a *hardened* library entry
point into a plain one. Both are pure wins when the callee really is the
library function — and both are unsound when the symbol could resolve to code
the compiler is currently rewriting.

Two gates own that decision, and they share one symbol inventory:

| gate | question | code |
|---|---|---|
| libcall synthesis (A13) | may a pass *introduce* a call to `f`? | `src/passes/libcall.rs`, consumers `loop_idiom`, `loop_memset` |
| fortify-stdio fold (A14) | may a hardened call be folded to its plain equivalent? | `src/passes/fortify_fold.rs`, backend `backend/libcall_policy.rs` |

## The rule

**A pass may introduce a call to `f` only when this translation unit does not
provide `f`** — as a definition, as an assembler label redirect, or inside a
top-level `asm("…")` blob. The check is per *symbol*, not per enclosing
function, because inlining merges helper functions into `memcpy` before the
pass runs.

The rule is closed over *interposable* calls: a synthesised `memmove` may be
implemented by calling the public `memcpy` (newlib and the historical
BSD/glibc `memmove` forward the non-overlapping case that way), so a TU that
defines `memcpy` blocks `memmove` synthesis too. Only evidence-backed edges
belong in that table: an extra entry costs a rewrite in TUs that define the
reached symbol, a missing one costs correctness.

The three reproduced failures that motivated the gate (each was a real
self-call at `-O2`/`-O3`):

1. `void *memcpy(...) { for (...) d[i] = s[i]; }` — `loop_idiom` rewrote the
   loop inside `memcpy` into `call memmove@PLT`: unbounded self-recursion on
   libcs whose `memmove` calls `memcpy`.
2. `static void helper(...) { for (...) … } void *memcpy(...) {
   helper(d, s, n); }` — inlining merged `helper` into `memcpy` first.
3. `void *memset(...) { for (...) p[i] = 0; }` — the same self-call for
   `loop_memset`.

All three reference compilers already behave this way, measured with the
pinned oracles (GCC 16.2, Clang 23.1, ICX): with a TU-defined `memcpy` and a
copy loop, every compiler × flag row keeps the loop scalar.

## One inventory, one lock

`common::builtin::symbol_inventory(module)` collects, once per compilation:

* `defined` — function and object definitions, under their **emitted** names;
* `redirected` — C names that an `asm("label")` maps onto another symbol;
* `asm_blobs` — top-level `asm("…")` text, verbatim.

`SymbolInventory::blocks_library_assumption(name)` answers the whole question,
and both gates consume it. It is deliberately mention-based for assembler
text: the question is only ever asked for a handful of candidate names, and a
word-boundary scan of the blob cannot miss a spelling the way a partial
assembler parser can. The scanner lives in `common::asm_scan` and is shared
with the dead-static elimination, so the two callers cannot disagree about
what counts as a mention (`.` and `$` are identifier characters; `$g` is an
immediate reference to `g`).

The CLI side of the same gate — `-fno-builtin`, `-fno-builtin-f`,
`-ffreestanding` — is `BuiltinPolicy`, and the backend consults it with the
inventory through **one `RwLock` holding both** (`Live { policy, inventory }`
in `backend/libcall_policy.rs`). Two separate globals would let a pass observe
a policy that was already withdrawn next to an inventory that was not, which
is precisely the window the gates exist to close. The lock is taken
read-mostly; the write happens once, when the function/module context is
installed.

## stdio: the *original* callee decides

`fortify_fold` rewrites `__printf_chk`-style calls into `puts`/`fputs`/
`putchar`/`fputc`/`fwrite`. The permission question is asked about **the
callee that is being replaced**, not about the replacement:

* replacing `__printf_chk` with `puts` is allowed only when the TU neither
  defines nor redirects `__printf_chk` *and* does not define `puts`
  (the replacement names are checked through the same inventory);
* a `_FORTIFY_SOURCE`-hardened entry point the user redefined must keep
  receiving its calls, even when the *fold* would only call `putchar`.

Checking only the replacement name is the subtle failure — the empty-format
case (`__printf_chk(1, "")`) deletes the call entirely and therefore has no
replacement instruction to scan. The fold decision is therefore made from the
call's original callee plus the instruction list, and every fold arm is
gated, including the ones that emit nothing.

## `__*_chk` and `_FORTIFY_SOURCE`

The `__memcpy_chk` / `__printf_chk` family is a **glibc ABI surface**, not a
macro: a TU compiled with `-D_FORTIFY_SOURCE=2` (or a build system's default
`-O2 -D_FORTIFY_SOURCE=2`) calls those symbols directly, and lccc lowers them
to the checked equivalents.

`_FORTIFY_SOURCE` itself is **not** implemented: glibc's fortify headers
expand to `extern __inline __attribute__((__always_inline__))` wrappers built
on `__builtin_va_arg_pack()` and `__builtin_object_size()`, and a wrapper the
inliner refuses to inline has no out-of-line body — a link error rather than a
performance problem. The macro is therefore undefined for every translation
unit, and the driver **says so once** (`-D_FORTIFY_SOURCE=…` is diagnosed at
the option-policy layer): a build that asks for a fortified libc must not
discover from a security review that it did not get one. `LCCC_STRICT_OPTIONS=1`
turns that diagnostic into an error, so a capability probe can tell
"implemented" from "accepted".

## Option contract

Tolerating an optimisation flag is safe only while the optimisation it controls
does not exist here: `-fno-strict-aliasing` has nothing to switch off because
this compiler performs no type-based alias analysis, and `-fwrapv` /
`-fno-strict-overflow` have nothing to constrain because it carries no
signed-overflow or `inbounds` poison. That precondition is machine-checked —
`scripts/check_no_ub_exploitation.py` fails if such a concept appears in `src/`,
because landing one would silently turn those tolerated spellings into
miscompiles.

The wider fail-closed unknown-option contract (exit 1 for data-model flags
like `-funsigned-char` and for hardening requests, `LCCC_STRICT_OPTIONS=1` to
turn every tolerated spelling into an error, a one-time diagnostic for
`-D_FORTIFY_SOURCE=…`) is designed and measured in closed PR #757 but NOT yet
landed: today the driver rejects `-fstack-protector*` and accepts the
data-model and dialect spellings silently. A build system probing with
`cc-option` can therefore still read a tolerated flag as implemented — that
port is a tracked follow-up, and this paragraph must be replaced by #757's
measured table when it lands.

## Extending it

* New synthesised callee → add it to `libcall_reach` **with the measurement**
  that justifies the edge, plus a test that a TU defining the reached symbol
  blocks synthesis.
* New fold arm → gate it on the original callee *and* on every replacement
  name it emits; the abort-probe torture tests
  (`gcc.c-torture/execute/{printf,vprintf,fprintf,vfprintf}-chk-1.c`) define
  the hardened functions `noinline` and abort if a foldable call still reaches
  them, which is the end-to-end proof.
* New assembler spelling that can define a symbol → extend
  `common::asm_scan`, never a second scanner.
