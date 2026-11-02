# Gauntlet repro fixtures (2026-08-19/20 session)

Known-open reproducers kept verbatim so a fix can be verified by re-running
the exact commands. Both are -O0-stable => frontend/lowering/backend-emit
class, NOT optimizer passes (CCC_DISABLE_PASSES=all does not help).

## 1. zlib-ng 2.3.3 (+ ms178-1.patch) decompress-path crash

    cd zlib-ng-2.3.3 && ./configure --without-optimizations
    make CC=lccc CFLAGS="-O0 -w" minigzip
    echo "hello world" | ./minigzip | ./minigzip -d     # SIGSEGV

- non-compat O0 build: crash in gz_reset (`mov %r15d,(%r15)` — the zero
  materialization clobbered the live base pointer; same alias family as the
  i686 RMW fix, but in the x86-64 emit path).
- compat O2 build: compress OK, decompress crashes in inflateInit2_
  (`mov %rsi,0x50(%rbx)` with rbx = stale register after the zalloc
  function-pointer call — staging across indirect call).

## 2. SQLite 3.53.4 amalgamation crash (also -O0, also -O2)

    lccc -O0 -I sqlite-amalgamation-3530400 -o sqm sqmin.c \
         sqlite-amalgamation-3530400/sqlite3.c -lm && ./sqm   # SIGSEGV

- Crash in openDatabase: arguments to sqlite3SchemaGet already corrupted
  (rdi = 0x54e36f, a code/rodata-looking value, not the heap db pointer).
- Register corruption happens BEFORE the SchemaGet call site; the emitted
  sequence around `&db->aDb[0].pBt` (out-param to sqlite3BtreeOpen) plus the
  subsequent member chains is implicated. Minimal C repros of the isolated
  patterns (gep1/gep2/out1/base1 in the session transcript) all PASS, so
  the trigger needs more of openDatabase's context — next step is creduce
  on sqlite3.c restricted to openDatabase, or bisecting the amalgamation
  with !SQLITE_OMIT switches.

## Working (validated green) in the same session
- gzip 1.14: full build + `make check` 30/30 PASS (lccc CC).
- expat 2.8.2: full build; 7-document XML parse differential (events,
  attributes, char data, error codes) bit-identical vs gcc-built lib.
- zlib-ng 2.3.3: complete static+shared build with the ms178-1 patch
  applied and 0 errors; compression path verified round-trip at O2 compat.
