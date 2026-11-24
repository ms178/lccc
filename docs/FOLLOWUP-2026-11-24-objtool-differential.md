# objtool as a second opinion on lccc's objects (2026-11-24)

## What it is

`objtool` is the Linux kernel's object-file validator. It decodes every
instruction of an x86-64 object with the kernel's decoder and follows every
code path of every function with the stack state the instructions imply.
`objtool --orc --dry-run FILE.o` therefore reports, without running anything:

* a `ret` reached with a moved stack pointer;
* two paths that meet with different stack depths;
* a tail call taken with a non-entry stack;
* a function without an ELF size, which it then cannot validate;
* instructions no path reaches.

These are the properties the x86 peephole passes that reason about `%rsp`
have to preserve (`eliminate_never_read_stores`, frame compaction, push/pop
pairing, tail-call rewriting), and no test of lccc's own asks about them
directly: a program that happens not to read the damaged slot still passes.

## How to get it

```
scripts/ensure_objtool.sh            # prints the binary's path; cached under ~/.cache/lccc/objtool
```

It needs no root and no network host other than github.com: a sparse,
blob-less clone of `tools/objtool` from `torvalds/linux` at v7.3, plus
`libelf.h`/`gelf.h` from the elfutils release that matches the installed
`libelf.so.1` when the development package is absent. A cold build takes
about 3.5 minutes (the clone), a warm call none; the script proves the
binary on a sound and a broken function before it prints the path.

## How to use it

```
scripts/objtool_differential.py --lccc target/fastbuild/lccc --check
```

objtool was written for kernel code, so on ordinary programs it also
reports what it merely does not model (stack realignment, `alloca`, calls
that do not return): GCC 16.2's objects of the regression corpus draw 594
warnings. The tool is therefore differential. Every x86-64 fixture is built
by lccc and by GCC with the fixture's own flags plus the kernel's `-fno-pie`
(a RIP-relative jump table is "a GCC quirk" to objtool, which then stops
reporting unreachable code for the whole object), and a warning counts when
lccc's object draws it in a function for which GCC's draws none.

## First run (1,407 x86-64 fixtures, lccc at `802100a`)

| Warning only lccc draws | Count | Verdict |
|---|---|---|
| `__lccc_cpu_init() is missing an ELF size annotation` | 11 | **defect, fixed** |
| `unreachable instruction` | 6 | targets of computed gotos, `__builtin_setjmp` receivers, a fixture's own assembly jump table |
| `falls through to next function` | 6 | a nested function leaving through a non-local goto (GCC's code draws a different warning for the same jump) |
| stack-state warnings of any kind | 0 | |

The defect: the CPUID probe helper that `__builtin_cpu_supports` appends to
a translation unit defined its 18 cached words and the `ready` flag without
a type and a size, and its initialiser as a function of size 0. A readelf
sweep of the same objects found the same omission in the profile writer of
`-fprofile-generate`, whose paths and format strings are string literals
under symbol names. Both are fixed (`common/cpu_probe.rs`,
`backend/common.rs::emit_data_sections`) and pinned on four targets by
`tests/regression/check_symbol_annotations.py`, which needs readelf only
and runs in CI. The 12 remaining warnings are listed with their reasons in
`TRIAGED`; `--check` fails on a new one and on a listed one that stopped
occurring.

The absence of any stack-state warning is the result that matters for the
`popcnt` fix of this pull request (the pass that deletes never-read stack
stores took `popcnt` for a `pop` and misplaced `%rsp` by 8 bytes): across
1,407 fixtures at their own optimization levels, no function of lccc's
returns with a moved stack pointer or joins two paths at different depths.

## Why it is not a CI gate

The runners have no objtool, and building it costs a kernel clone per cold
cache. `check_symbol_annotations.py` carries the one class of finding that
readelf can see. Run the differential after a change to frame layout,
prologue or epilogue emission, tail calls, or any pass that tracks `%rsp`.

## What was not obtainable in the sandbox

* **pahole** needs libdw and its headers; neither a package nor a buildable
  source (elfutils wants autotools) is reachable. `gdb`'s `ptype /o` prints
  the same offsets, sizes and holes and is what layout questions were
  answered with.
* **ICX 2026.2**: `scripts/ensure_icx_2026.sh` needs Intel's APT host,
  which the sandbox does not route (`TLS/SSL connection has been closed`),
  and no cache tag of this repository carries it. Comparisons in this pull
  request are against GCC 16.2 and Clang 23.1.5; the gates that have a
  hosted ICX contract run it in CI.
