# LCCC Licensing Guide

This document explains the dual licensing structure used in LCCC (Lev's Claude's C Compiler) and how to handle licensing for contributions.

## Quick Summary

| Code Type | License | Details |
|-----------|---------|---------|
| **Original CCC Code** | CC0 1.0 (Public Domain) | No copyright restrictions, attribution not required |
| **LCCC Improvements** | MIT OR Apache-2.0 OR BSD-2-Clause | Your choice, per contribution |
| **Mixed Files** | Both CC0 + your choice | Original sections CC0, new sections your license |
| **Third-party-derived benchmark kernels** | Per-file upstream license | Must retain upstream provenance and may not be relabelled as project-default-only |

## Licensing Structure

### CCC Code (Original)
**Licensed under**: CC0 1.0 Universal (Public Domain)

All code from the original Claude's C Compiler project:
- CCC frontend (lexer, parser, semantic analysis)
- CCC IR and SSA optimizer
- CCC backends (x86-64, ARM, RISC-V, i686)
- CCC assembler and linker

**What this means**: 
- Use freely for any purpose
- No attribution required
- No copyright restrictions
- More permissive than any open source license

See [`LICENSE-CC0-CCC`](./LICENSE-CC0-CCC) for full text.

### LCCC Code (New & Improvements)
**Licensed under**: MIT OR Apache-2.0 OR BSD-2-Clause (your choice)

All code authored as part of LCCC optimization efforts:
- `src/backend/live_range.rs`, RA policy in `src/backend/regalloc.rs`
- Optimizer / PGO / SIMD / linker work under `src/`
- `tests/benchmark/` runners (not third-party kernel *bodies*)
- Documentation under `docs/` and `lccc-improvements/`
- Any new modules or major rewrites

Paths are `src/…`. There is no `ccc/` submodule and no `linear_scan.rs`.

**What this means**:
- You pick ONE of three licenses for each contribution
- Standard open source protections
- Users can use your code under their chosen license
- Each license file available: [`LICENSE-MIT`](./LICENSE-MIT), [`LICENSE-APACHE`](./LICENSE-APACHE), [`LICENSE-BSD`](./LICENSE-BSD)

### Third-Party-Derived Benchmark Sources
**Licensed under**: the upstream license named in each file, plus any clearly
separable original harness material where applicable.

The benchmark suite intentionally contains narrow source-derived workload
kernels.  They are not CCC code and must **not** be treated as automatically
`MIT OR Apache-2.0 OR BSD-2-Clause` merely because their harness lives in this
repository.  Every such source has a license/provenance header and is listed in
[`tests/benchmark/WORKLOAD_PROVENANCE.md`](./tests/benchmark/WORKLOAD_PROVENANCE.md).
The corresponding upstream license texts are retained under
[`third_party_licenses/`](./third_party_licenses/README.md).

Current derived benchmark files are:

| File | Upstream-derived material | Applicable upstream license |
|---|---|---|
| `tests/benchmark/programs/gzip_crc32.c` | GNU gzip 1.14 `lib/crc.c` scalar table path | LGPL-3.0-or-later |
| `tests/benchmark/programs/zlib_ng_adler32.c` | zlib-ng 2.3.3 generic Adler-32 | Zlib |
| `tests/benchmark/programs/expat_xml_scan.c` | Expat 2.8.2 tokenizer specialization | MIT |
| `tests/benchmark/programs/sqlite_varint.c` | SQLite 3.53.4 varint encoder/decoder | Public domain |
| `tests/benchmark/programs/linux_find_bit.c` | Linux 6.18.42 bit-search kernel | GPL-2.0-or-later |
| `tests/benchmark/programs/glibc_memcmp.c` | glibc `memcmp` aligned-word strategy | LGPL-2.1-or-later |

The original benchmark runner, build script, provenance documentation, and
hotspot documentation remain LCCC contributions under the project default
unless their own header states otherwise.  Keep third-party benchmark sources
as test/measurement material; do not link them into the compiler/runtime or
claim the project-default license covers their upstream-derived portions.

### Hybrid Files (CCC Base + LCCC Improvements)
**Licensed under**: Both CC0 + your chosen license

When you modify existing CCC files to add improvements:
- Original CCC code sections remain CC0
- Your new/modified sections are under your license choice
- Both licenses apply to the file
- Users can use either license for their needs

**Example structure**:
```rust
// File: src/backend/regalloc.rs
//
// Original CCC code: CC0 1.0 Universal
// LCCC waves / coalescing / XMM / i686 policy: MIT (or your choice)
// See `git log --follow` — line numbers from 2026-03 are obsolete.
```

## How to Determine Which License Applies

For any code in LCCC, ask these questions in order:

### Question 1: Is this code new or substantially rewritten?
**YES** → Use MIT/Apache/BSD (your choice)
- You are the author of new optimization work
- Pick the license that best fits your preference

**NO** → Go to Question 2

### Question 2: Is this code from the original CCC project?
**YES** → CC0 applies (no copyright restrictions)
- Original CCC frontend, IR, backends, assembler, linker
- No license action needed - already public domain
- Respect the original author's CC0 dedication

**NO** → Go to Question 3

### Question 3: Is this derived from a third-party upstream source?
**YES** → Preserve and apply that upstream license
- Add a per-file source/provenance header
- Record exact release/commit and source digest in `WORKLOAD_PROVENANCE.md`
- Do not describe the complete file as project-default-only
- Keep it out of compiler/runtime distribution paths unless license review approves it

**NO** → Go to Question 4

### Question 4: Is this a hybrid (CCC base + LCCC improvements)?
**YES** → Both licenses apply
- Identify which sections are CCC (CC0)
- Identify which sections are new (your choice)
- Document clearly in git commits and code headers

**NO** → You found something else
- Document appropriately and ask maintainers

## Practical Examples

### Example 1: Creating a New Module (Pure LCCC Code)

File: `src/backend/live_range.rs`

```rust
// Linear Scan Register Allocator (scan kernel)
// Licensed under: MIT
//
// Production allocator — not a Week-2 prototype.

pub struct LinearScanAllocator {
    // ...
}
```

### Example 2: Modifying Existing CCC File

File: `src/backend/regalloc.rs` (CCC origin + LCCC policy)

```rust
// ORIGINAL CCC CODE (CC0 1.0 Universal): eligibility helpers, PhysReg
// LCCC ADDITION (MIT): allocate_registers waves, coalescing, XMM, i686
```

There is **no** feature-flag fallback to the 574-line 3-phase greedy allocator.

### Example 3: Major Rewrite of Existing File

If you substantially rewrite a CCC file (>80% new code):

Option A - Keep both licenses:
```rust
// File mostly rewritten for LCCC improvements
// Original CCC portions: CC0 1.0 Universal
// LCCC rewrite: MIT
// See git log for exact attribution
```

Option B - Replace entirely with new module:
```rust
// This module was deprecated and replaced by:
// src/backend/<new_module>.rs
// 
// If you need original CCC code, see git history
```

## License Choice Guide

### MIT License
**Choose if you**:
- Want broad industry compatibility
- Are used to MIT-licensed projects
- Want simple, concise language
- Prefer permissive without restriction

### Apache-2.0 License
**Choose if you**:
- Want explicit patent grant protection
- Are contributing to an Apache project later
- Want more legal specificity
- Need broader liability/warranty clauses

### BSD-2-Clause License
**Choose if you**:
- Want BSD heritage/compatibility
- Prefer concise non-attribution variant
- Are in BSD/academic communities

**Recommendation**: Default to MIT if unsure. All three are compatible with CC0 code and each other.

## Contributing to LCCC

### Your Responsibilities

1. **Clearly mark new code as LCCC** with license header
2. **Preserve CCC code** as CC0 - don't try to relicense it
3. **Document mixed files** with git commit messages
4. **Choose a license** for each contribution (or use default)
5. **Check git history** if unsure about code origin

### In Your Commits

Always include license information:

```
[LCCC] Phase X: Brief description

Details about what was changed and why.

Files modified:
- new_file.rs (NEW, MIT)
- existing_file.rs (CCC code + MIT additions)

License: MIT (or Apache-2.0 or BSD-2-Clause)
```

### In Your Code

Add headers to new files:

```rust
// Linear Scan Register Allocator
// Licensed under: MIT
// Part of LCCC Phase 1

// This module implements a linear scan register allocation algorithm
// to improve upon CCC's overly conservative 3-phase allocator.

pub struct LinearScanAllocator { ... }
```

For modifications to existing files:

```rust
// LCCC ADDITION (MIT):
// New integration point for linear scan allocator
// Original CCC code above/below this section remains CC0

fn integrate_linear_scan(...) { ... }
```

## FAQ

### Q: What if I don't specify a license for my code?

**A**: The default in `Cargo.toml` is `MIT OR Apache-2.0 OR BSD-2-Clause`. Your code uses that by default.

```toml
[workspace.package]
license = "MIT OR Apache-2.0 OR BSD-2-Clause"
```

### Q: Can I relicense CCC code to MIT?

**A**: No. CCC code must remain CC0. However, you don't need to! 
- CC0 is MORE permissive than MIT
- Any code using CC0 parts is already unrestricted
- Relicensing would reduce freedom, not increase it

### Q: Can I use LCCC code in a proprietary project?

**A**: Yes! All LCCC code is under MIT/Apache/BSD, which permit proprietary use. The CC0 parts have even fewer restrictions.

### Q: What if multiple people contribute to one file?

**A**: Document in commit messages and file headers:

```rust
// File: src/backend/regalloc.rs
//
// Original CCC code: CC0 1.0 Universal
// LCCC additions: MIT (or Apache/BSD)
// See git log for detailed attribution
```

### Q: Can I contribute code under a different license?

**A**: Better to stick with MIT/Apache/BSD for consistency. If you really need GPL/AGPL/etc:
- Discuss with maintainers first
- Creates incompatibility issues
- Makes distribution more complex
- Generally not recommended for library code

### Q: What about documentation and non-code files?

**A**: These typically follow the project default (MIT OR Apache-2.0 OR BSD-2-Clause). If unclear, add a header:

```markdown
# Documentation
Licensed under: MIT OR Apache-2.0 OR BSD-2-Clause
```

### Q: If I use LCCC in my project, what do I need to do?

**A**: 
1. Include a copy of the licenses you use (CC0 and/or MIT/Apache/BSD)
2. For MIT/Apache/BSD, include copyright notices (optional for CC0)
3. Follow the license terms you choose

### Q: Is this licensing setup unusual?

**A**: Not really! It's similar to:
- LLVM (has both permissive and restricted code)
- GCC (combines FSF code with other licenses)
- Chromium (combines MIT, Apache, BSD, etc)

Layered licensing is common for large compiler projects.

## File Locations

- **CCC License**: [`LICENSE-CC0-CCC`](./LICENSE-CC0-CCC)
- **MIT License**: [`LICENSE-MIT`](./LICENSE-MIT)
- **Apache License**: [`LICENSE-APACHE`](./LICENSE-APACHE)
- **BSD License**: [`LICENSE-BSD`](./LICENSE-BSD)
- **Project README**: [`README.md`](./README.md) - Overview with licensing structure
- **Contributing Guide**: [`CONTRIBUTING.md`](./CONTRIBUTING.md) - How to contribute

## Summary Table

| Scenario | Action | License |
|----------|--------|---------|
| New optimization module | Create new file | MIT (or your choice) |
| Modify existing CCC file | Add clear comments | Both CC0 + your choice |
| Major rewrite (>80% new) | Consider new module | Your choice |
| Question about origin | Check git history | See `git log --follow` |
| Not sure which license | Use default | MIT OR Apache-2.0 OR BSD-2-Clause |
| Contributing | Mark clearly in commit | Your choice of MIT/Apache/BSD |

---

## Questions?

If you're unsure about licensing for a specific contribution:
1. Check similar files in the codebase
2. Review git history: `git log --follow filename`
3. Ask in issue or PR discussion
4. Default to MIT if still uncertain

The goal is to **maximize freedom** for everyone while **respecting original authors**.

## Related Documents

- **README.md** - Project overview including licensing summary
- **CONTRIBUTING.md** - How to contribute while respecting licenses
- **LICENSE-MIT** - Full MIT License text
- **LICENSE-APACHE** - Full Apache 2.0 License text
- **LICENSE-BSD** - Full BSD 2-Clause License text
- **LICENSE-CC0-CCC** - Full CC0 1.0 License text (CCC code)

---

**Last updated**: 2026-03-19  
**Version**: 1.0

This guide ensures everyone understands the LCCC dual licensing model while respecting the original CCC project's CC0 dedication.
