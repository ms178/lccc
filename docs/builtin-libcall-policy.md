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
* `objects` — source-level object declarations as well as definitions (an
  `extern` object named `memcpy` must not be mistaken for a callable function);
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

The policy interacts with the driver's unknown-option contract, which is
fail-closed and measured against GCC 14.2 in-tree plus the pinned oracles
GCC 16.2 / Clang 23.1 / ICC 2021.10 / ICX:

| spelling | gcc 16.2 | clang 23.1 | icc 2021.10 | icx | lccc |
|---|---|---|---|---|---|
| `-std=<unknown>` | exit 1 | exit 1 | varies | exit 1 | exit 1 |
| `-fstack-protector*`, `-ftrapv`, `-fsanitize=` | accepted | accepted | accepted | accepted | exit 1 (contract: LCCC would not harden) |
| `-funsigned-char`, `-fsigned-char` | accepted | accepted | accepted | accepted | accepted and implemented (selects plain-`char` signedness) |
| `-fshort-enums`, `-fshort-wchar`, `-fpack-struct*` | accepted | accepted | accepted | accepted | exit 1 (contract: ignoring silently changes the data model) |
| `-g4`, `-ggdb9`, `-gdwarf-9`, `-gz=bogus`, `-gno-bogus`, `-gbogus` | exit 1 | exit 1 | accepted | exit 1 | exit 1 |
| `-Wno-<unknown>`, `-gno-<feature>` | accepted | accepted | accepted | accepted | accepted silently (satisfied off-request) |
| `--<unknown>`, `-f<unknown>`, `-m<unknown>`, `-W<unknown>`, `-Werror=<unknown>`, `--param`, GNU-accepted but unimplemented `-g` selectors, presentation namespaces | exit 1 for `--`/`-f`/`--param`/`-W`, accepted elsewhere | accepted | accepted | accepted | accepted + diagnosed; `LCCC_STRICT_OPTIONS=1` → exit 1 |

A build system decides whether a flag exists from the **exit status**
(`cc-option`, Meson `has_argument`, Kconfig), so succeeding on a flag lccc
does not implement turns "does the compiler support this?" into a false yes —
and for a hardening or optimisation flag that is a silent misbuild.

LCCC fails closed wherever ignoring a request would change what the program
*means*: the hardening contract flags (`-fstack-protector*`, `-ftrapv`,
`-fsanitize=`), unsupported data-model flags (`-fshort-enums`, `-fshort-wchar`,
`-fpack-struct*` — ignoring one silently returns objects whose layout disagrees
with the libraries they link against), a dialect it cannot parse
(`-std=<unknown>`), and the debug selectors GNU itself rejects (that grammar
is closed and was measured exhaustively). Plain-`char` signedness is
implemented and is therefore not in the refusal set.

Everything else the driver has no arm for is **diagnosed, not fatal**, and the
reason is a measurement rather than taste: of the 93 flag spellings the Linux
kernel, glibc, zlib-ng, gzip and expat build systems pass unconditionally, 16
had no arm in the driver (`-fno-strict-aliasing`, `-fwrapv`,
`-fno-strict-overflow`, `-fno-plt`, `-flto`, `-funroll-loops`, `-Og`,
`-ansi`, `-fno-ident`, `-fuse-ld=*`, `-fvar-tracking-assignments`, …) — all of
them optimisation, reporting or debug-info preferences whose being ignored
cannot change the program's meaning. LCCC also has no table to validate the
open namespaces against (six implemented warning names against GNU's 409,
`gcc -Q --help=warnings`), so it cannot tell a typo from a spelling a newer GNU
added. The diagnostic names the option, and `LCCC_STRICT_OPTIONS=1` — the
variable the capability probe exports — turns every tolerated request into a
failure, so no probe can read one as an implemented feature.

"Switch X off" requests (`-Wno-<unknown>`, `-gno-<feature>`) are the
one class accepted in silence: when X does not exist, the request is already
satisfied, and the same reasoning is why all four reference compilers accept
`-Wno-<unknown>`.

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
