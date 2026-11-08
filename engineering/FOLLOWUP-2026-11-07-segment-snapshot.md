# Follow-up: segment-relative large snapshot loads

## Scope and reasoning

PR #1133 merged into `7aafed6d13f5ce760113e702b57ba7c2b376ecd2`. Its `forward_large_memcpy_loads` optimization replaces loads from a temporary with source-rooted loads. The original `Memcpy` uses ordinary addressing; an IR `Load` carrying `SegFs` or `SegGs` applies an additional segment base. An alloca-derived numeric pointer is therefore not sufficient evidence that a segment-relative read observes the bytes written by the ordinary memcpy. Forwarding such a read changes its address when the two alloca addresses differ. The optimization now accepts only `AddressSpace::Default` loads. This is a conservative IR proof fix; no claim is made that a source-level program triggering it was reproduced.

A pre-fix unit test showed `SegFs` was incorrectly forwarded (one transformation rather than zero); after the fix, the same test exercises and refuses both FS and GS. Positive ordinary-address forwarding is still required by the existing exact-endpoint test. The expanded matrix adds a volatile intervening load, redefinition of the source, nested GEP signed overflow, and source-span preservation; it also retains atomic stores, fences, volatile source/destination and loads, bounds, and derived-value redefinition. A cheap candidate scan avoids building whole-function proof sets when no memcpy of at least 128 bytes is present; this is an operation-count observation, not an independently measured runtime or compile-time speedup.

## Evidence and limits

* Before the guard, the segment test failed with `SegFs: left 1, right 0`; it passes with the guard. Focused copy-propagation suite: 27 passed; vector-temp promotion: 44 passed. `cargo fmt`, `git diff --check` and the census ratchet passed. The initial `fastbuild` (`-O1`, two Cargo jobs) was performed with a 4-GiB swap file; the final binary still needs a rebuild after the guard.
* The preceding audit reproduced a separate 128-byte snapshot misscompile across an atomic store and added a C regression in #1133. Its fixed version passed LCCC/GCC 16.2/Clang 23.1.5 on x86-64 and i686 at five optimization levels, and AArch64/RV64 under QEMU. The earlier x86 32-bit runs needed an isolated sysroot and explicit dynamic loader, not a native host multilib installation. This evidence predates the new segment guard.
* The PR #1133 full local fast run on its then-head was **585 passed, 13 failed, 15 skipped**; oracle provisioning and host multilib deficiencies were diagnosed, and a later targeted strict linker run passed 341/341, but a subsequent full local pass was not obtained. Do not use these results as a final same-tree CI pass. Its GitHub checks passed on earlier commits; they do not establish the status of this branch.
* PR #1126 remains separately open with failing hosted jobs as of this audit; the available annotations do not establish their exact root causes. This branch does not change that PR.

## Outstanding

1. Rebuild the new head and run the targeted snapshot regression and cross-target controls. Establish a real source-level FS/GS reproducer if practical; the current regression is at IR level.
2. Run the complete local fast gate and GitHub CI on the final unchanged head; secure a matching gated snapshot. Recheck and rebase onto current main. Do not claim green based on partial gates or an older head.
3. Continue the broader alias/provenance and AVX-code-quality work documented in `engineering/FOLLOWUP-2026-11-07-snapshot-alias-interop.md`; its 157-vs-36 static instruction count is not a hardware speed measurement. Obtain actual hardware and ICX evidence, real 128-bit RV64 libatomic interop and concurrency/model tests before making corresponding claims. Hardware PMU and ICX were not available locally in the preceding environment.
