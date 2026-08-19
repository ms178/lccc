# 2026-08-19 (session 10) — re-base onto latest upstream main

**Base:** `origin/main` @ `0ce9329d` (Merge PR #124 — session 8 landed upstream)
**Snapshot:** `/home/user/ms178-1.patch` (now the session-9 delta only: `origin/main..HEAD`)

## Why the old patch stopped applying

Upstream `ms178/lccc` merged session 8 as **PR #124**, squash-committed as
`4b7c301d` ("backend: fix call-arg/callee-clobber miscompiles + latent
stack-align fault") and merged as `0ce9329d`. The squashed tree is
**byte-identical** to this line's session-8 HEAD (`00c8c48c`):

```
0ce9329d^{tree} == 4b7c301d^{tree} == 00c8c48c^{tree} == 4755d69c…
```

So the old cumulative deliverable (`base 31b8e404`) now overlapped upstream:
every session-8 hunk failed (content already present) and the session-8 doc
file already existed. The fix is exactly the user's diagnosis — re-base the
remaining delta (session 9) on top of latest main.

## What was recovered

The harness wipe had (a) removed the `origin` remote from `.git/config`,
(b) dropped the session-9 commits `a2228aad` / `e09b5d5f` from the object
store, and (c) stripped `+x` from 37 files, while leaving the session-9
*content* in the working tree. All of it was restored from
`/home/user/artifacts/lccc.bundle` (which contains `refs/heads/main →
e09b5d5f`), not re-typed:

1. Re-added `origin` → `https://github.com/ms178/lccc.git`; fetched latest
   main (`0ce9329d`).
2. `git fetch artifacts/lccc.bundle main:recovered-session9` → recovered
   `a2228aad` + `e09b5d5f` with correct authors/modes.
3. Verified the working tree is content-identical to `e09b5d5f`
   (`git diff --cached e09b5d5f` shows only the 37 mode changes).
4. `git rebase --onto origin/main 00c8c48c main` — replay was conflict-free
   *because* the onto-tree equals the original parent tree (see above).

Result:

```
5808871c9 Land the corrected segments prototype + adopt Agent B's valid fixes (code size -537)   ← was e09b5d5f
be4815971 Fix Grok code-size regressions: FP global load/store, param/coalesce/GEP-base priority  ← was a2228aad
0ce9329db Merge pull request #124 from ms178/arena/01a017a0-lccc   ← origin/main
```

`5808871c9^{tree} == e09b5d5f^{tree}` — the rebase is a pure history
operation; **no code changed**, so the −537-byte code-size result is preserved
by construction.

## Deliverable

- `.base_ref` updated: `31b8e404` → `0ce9329db`.
- `ms178-1.patch` regenerated as `git diff 0ce9329db..5808871c9` = the 14-file
  session-9 delta (752 insertions / 75 deletions), verified **APPLIES-CLEAN**
  on a fresh `origin/main` worktree and to produce a tree identical to `main`.

## Validation (re-run after the re-base)

| metric | result |
|---|---:|
| Rust library tests | 910 passed, 0 failed |
| Correctness | 50/50 |
| Runtime regressions | 339 passed + 6 diagnosed GCC-oracle mismatches (unchanged set) |
| i686 runtime | lccc-i686 −m32 == gcc −m32 (smoke + 30-seed differential fuzz, 0 mismatch) |
| Warning-free fastbuild (`-D warnings`) | clean, 1m39s |

## Environment restored this turn

- 8 GiB swap (`/swapfile`), Rust 1.97.1 in `/opt` (`RUSTUP_HOME=/opt/rustup`,
  `CARGO_HOME=/opt/cargo`), apt deps (gcc-multilib, libc6-dev-i386, kernel
  tooling, qemu, etc.).
- Pruned stale worktrees `/tmp/baseline`, `/tmp/bisect-liveness`, `/tmp/pre-grok`.

## Not done / next

- No push: the harness wipe also dropped GitHub credentials; the re-based
  branch is local. Provide a token (or push manually) to open PR #125.
- Code size is unchanged by construction (tree byte-identical to the −537
  state); a fresh baseline rebuild + 28-file corpus re-measure can be run on
  request.
