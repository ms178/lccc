# Frontend follow-up: bounded `#embed` offset reads (2026-11-02)

## Finding and evidence

Red-team review of `src/frontend/preprocessor/includes.rs::read_embed_bounded`
found that the old `offset.saturating_add(limit)` calculation bounded the
allocation to the file size, but did not bound it to the bytes actually emitted.
For `offset >= file_size`, adding a positive limit could saturate to
`usize::MAX`; clamping that value to `file_size` then read and retained the
entire resource even though `#embed` emits no resource bytes. For a valid
high-offset/small-limit request, both memory and I/O were O(offset + limit),
not O(limit). A sparse 8 MiB file with a one-byte range provides a bounded,
reproducible witness: the old helper returned the 8 MiB prefix; the new helper
returns only the selected byte.

The actual result of `#embed` had often remained correct because
`expand_embed_bytes` discarded the prefix afterward. That did not make the
resource handling safe or performant: an attacker-controlled C source could
make the compiler read and allocate a huge prefix unnecessarily.

## Change and design rationale

`read_embed_bounded` now:

1. Opens the file once and obtains its size from that open handle.
2. Converts the offset to `u64` without wrapping; computes available bytes with
   `file_size.saturating_sub(offset)` (no `offset + limit` arithmetic).
3. Clamps the requested count to available bytes, converts that bounded count
   to `usize`, and uses `try_reserve_exact` so capacity/allocation failure is an
   ordinary error rather than an allocation panic.
4. Seeks directly to the requested offset and reads only the selected range,
   retaining the short-read and `Interrupted` handling.
5. Returns the selected range, so expansion no longer stores and then slices
   away the discarded prefix. The existing limit clamp in expansion remains a
   defensive cap; `if_empty`, `prefix`, and `suffix` behavior is unchanged.

Keeping the prefix read was rejected: it preserves an unnecessary O(offset)
resource cost and makes saturating addition easy to mishandle. Memory mapping
was rejected as a larger portability and failure-mode change (including
truncation/SIGBUS behavior); one seek plus a bounded read loop is the smaller
and safer design. The size snapshot caps allocation, and a file shortened
concurrently can only yield a short read, which is truncated to bytes received.

Read failures now distinguish a genuinely missing file from other I/O or
allocation errors; the latter keep their diagnostic context instead of being
misreported as “file not found.”

## Regression and validation

- Extended `embed_bounded_read_slices` to check nonzero offsets with and without
  a limit, end-of-file truncation, `usize::MAX` offset, and a sparse 8 MiB file
  whose last byte alone is selected. The sparse test checks the returned
  buffer is one byte, so the former prefix-buffer behavior fails directly.
- The existing C23 embed regression corpus (five tests, including the maximal
  offset/limit fixture) passed with fastbuild LCCC. That fixture is explicitly
  LCCC-only: GCC 16.2 rejects Clang's `clang::offset` extension, so GCC was not
  counted as an oracle for this case. Clang 23.1.4 independently compiled and
  ran the maximal-offset fixture successfully with its large-literal warning
  disabled; this validates the fixture's expected observable result, not LCCC
  internals.
- Existing include-next regression also passed: all 14 base cases and the 12
  mixed-GCC wrapper cells passed with LCCC, GCC 16.2, and Clang 23.1.4.
- `cargo test --offline --locked --profile fastbuild`: 7,525 passed, 0 failed,
  10 ignored. `cargo clippy --offline --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, `git diff --check`, and the Rust 1.97.0 MSRV
  `cargo check --offline --locked` passed.
- `scripts/build_lccc_fast.sh` built `lccc` and `lccc-ld` with Rust 1.99.1,
  fastbuild `-O1`, and 4 GiB active swap.
- The full `scripts/ci_local.sh --fast` was run with the cached Rust/GCC/Clang
  toolchains and restored Debian multilib/YAML/Valgrind prerequisites, but did
  not finish within the 30-minute command limit. Its output showed the initial
  gates and i686 runtime checks passing; no completion summary or matching CI
  stamp was produced. This is not a full local CI pass. Hosted PR CI is the
  remaining acceptance gate.

## Scope and limits

This fixes the demonstrated range-read issue; it does not claim an exhaustive
frontend audit or universal optimality. The allocation-failure path is handled
without panic but was not fault-injected. No hardware performance claim is
made: the measurable bound is requested/read buffer size (8 MiB before versus
1 byte after for the sparse witness), not a timing result. The tree was based
on `origin/main` at `abb7708da583a6ab0453acde2d49380a53820b1b`; it was current
when the work began. Hosted CI must pass on the pushed commit before calling
this change green.
