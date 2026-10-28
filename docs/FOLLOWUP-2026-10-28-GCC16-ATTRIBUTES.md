# GCC 16.2 diagnostics: session follow-up (2026-10-28)

## Recovery and integration

The previous diagnostic work was recovered from the open PR and replayed on
this session's branch, based on `main` at `3f2f6a9`. Main's shared, pure-Rust
zlib/zstd ELF-section decoder supersedes the previous branch's private FFI
implementation. An automatic replay initially retained both implementations
and failed to compile; the duplicate implementation and its elevated census
baseline were removed, retaining main's ELF64/i686 and legacy `.zdebug` path.
The current census passes without raising any baseline.

The missing default-on header diagnostics were caused by GCC-known attribute
names incorrectly falling into the unknown-name class. The registry now has
127 GCC-known names, of which 83 are inventoried as unimplemented; 48 are
implemented, including four intentional Clang extensions that GCC itself does
not recognize. The regression gate cross-checks the actual registry against
its inventory, checks GCC and LCCC on 22 focused probes, and compiles two
system-header translation units with `-Wall -Wextra -Werror`. It is wired into
both the local and hosted CI mirrors. Explicit `-Wattributes` still reports
known-but-unimplemented attributes; accepting a name is not a claim that its
semantics are implemented.

## Sandbox safety incident and prevention

The previous sandbox lost its x86-64 dynamic loader after a privileged Debian
package extraction onto `/` crossed usrmerge symlinks. Never extract a package
onto `/` or a system directory, run `dpkg -i`, or install packages with sudo.
`offline_deb_extract.sh` instead refuses root and destinations outside an
isolated prefix below `/home/user` or `/tmp`, validates archive members before
using unprivileged `dpkg -x`, rewrites absolute links to sysroot-relative links,
and publishes only after staged symlink checks. The package inputs must still
be trusted and hash-verified; this is not an authentication mechanism.

`arena_session_restore.sh` formerly attempted `apt-get install` and Rustup
network installation. It now consumes the separately restored, hash-locked
agent cache and only reports missing multilib packages; `--source-only` and
`--swap-only` avoid the other restoration steps. The system loader remained
unchanged during this session.

## Observed validation so far

* Pinned cache: Rust/Cargo 1.99.1, GCC 16.2.0, Clang 23.1.4, GNU binutils
  2.47, and mold 3.0.0. Eight GiB of swap is active.
* `cargo build --profile fastbuild -j 2`, `cargo fmt --check`, and
  `cargo clippy --profile fastbuild -j 2 -- -D warnings`: PASS.
* Attribute gate: PASS (22 focused probes, two header probes, 83 pinned names).
  Diagnostic red team: PASS (999 contracts against exact GCC 16.2.0).
  Operand, severity (39 verdicts), position (12 cases), warning-compatibility,
  and frontend-recovery gates: PASS. Census ratchet: PASS.
* Offline-deb tests: PASS (six tests including archive traversal, cross-package
  symlink parents, chained escape, preexisting destination and root-boundary
  controls). CI/local gate parity, shard plan and workflow shell checks: PASS.
* A 13-package Bookworm multilib/PyYAML sysroot was extracted unprivileged.
  The cached libc packages and host `libc6-dev` both report
  `2.36-9+deb12u14`; the host's matching multiarch `bits`, `sys`, and `gnu`
  headers were copied into the private root because the multilib package is
  only a delta. GCC and LCCC each built and ran a 32-bit stdio probe using the
  private dynamic interpreter. This is not a host-wide multilib installation.

## Remaining work and limits

The full local fast matrix and hosted CI must be run on the integrated tree;
the earlier PR's red checks are not evidence for this branch. In particular,
standalone test commands that build and immediately execute `-m32` programs
need an explicit private-interpreter flag or a real host multilib installation;
merely setting `LIBRARY_PATH` cannot change a binary's ELF interpreter.
The warning registry (GP-09) and initial workload/comparator additions have
landed, and bounded GP-01–GP-24 verdicts are recorded in backlog §12.0.
Complete real-workload coverage and multi-compiler research comparisons remain
open. No generated-code performance improvement is claimed for this diagnostic
and infrastructure work.

## Workload research update

The benchmark registry now includes a zlib-ng 2.3.3 `compare256_64` kernel,
with upstream source provenance and the unaligned `memcpy` fallback. Its
self-check covers every mismatch offset from 0 through 256 across all 64
pairs of byte alignments, plus same-pointer inputs. A differential gate runs
GCC 16.2 and LCCC at `-O0`, `-O2`, `-O3` and `-Os`, both native and i686,
and requires a deliberately mutated kernel to fail. Clang 23.1.4 was also
checked locally. This is a derived kernel, not a complete compression test
or a performance claim.

The pinned zlib-ng 2.3.3 source and packaging patch were verified by SHA-256,
then the configure/Make static build was run with GCC 16.2 and LCCC at `-O2`
in zlib-compat mode. For both builds, `libz.a`, `example`, and `minigzip` were
explicitly built, and upstream `make -C test alltests` completed, including its
static/example and regression targets. Three additional inputs (empty,
deterministic random and repetitive), four compression levels, both
producers and both consumers gave 48 successful cross-compiler roundtrips;
24 streams also decoded with system gzip. This corrects an earlier attempt
where `make test` was a no-op and no `minigzip` existed. CMake/CTest and the
full upstream test configurations remain unrun because CMake was unavailable;
these results do not replace them. The local evidence is under
`~/.cache/lccc/zlib-configure-test-v2` and is not a target-hardware timing
measurement.

## Retained ELF sections: infrastructure, not frontend completion

The integrated assemblers previously discarded the `R` section flag on all
five ELF targets. They now emit `SHF_GNU_RETAIN` for uniquely named sections
and set `ELFOSABI_GNU`: GNU ld ignores that OS-specific bit when the object
incorrectly advertises System V OSABI. The x86-64 `ld -r` path also preserves
the flag and OSABI and separates retained from discardable same-named input
sections. The regression compares five integrated assemblers with GNU as
2.47, checks PROGBITS/NOBITS, executable, read-only and TLS flags, assembles
actual GCC 16.2 output, and exercises GC and relocation reachability with GNU
ld 2.47, mold 3.0.0 and the LCCC x86-64 linker. GNU and LCCC x86-64 partial
links are followed by a GNU GC link, including same-named R/non-R sections;
negative controls remove `R` and must be collected.

This does **not** implement the C `retain` attribute, which remains a silent
frontend correctness hazard. The integrated assemblers still cannot represent
same-named sections with different retain states; they now reject rather than
silently merge that case. The i686 LCCC linker currently accepts
`--gc-sections` without collecting the discarded controls; the i686 GC result
above is for GNU ld and mold, not LCCC. Separately, the local mold 3.0.0
`-r` probe discarded `R` even for GNU-as-produced input; no mold partial-link
correctness is claimed. None of these bounded checks establishes GP-03
completion or whole-program GCC compatibility.

## C digraphs: bounded GP-07 progress

The lexer now recognizes `<:`, `:>`, `<%`, `%>`, `%:` and `%:%:` as the
corresponding C punctuators. The preprocessor accepts `%:` directives and
`%:` / `%:%:` macro operators while preserving original token spelling during
stringification. Its text-level scanners respect maximal munch, prevent
accidental digraph creation at macro boundaries, and do not interpret operator
spellings inside literals. Directive line splices, multi-line comments and
source locations are included in the differential.

The checked-in gate compares GCC 16.2 and LCCC with 22 native/i686 executions
across C99, C11, C17, C23 and GNU89 (including a C23 digit-separator case),
emits objects for five integrated targets, checks seven preprocessing controls,
40 adjacent/overlapping punctuator cases, three invalid inputs and a source
position. Both local and hosted fast CI run it. These are bounded tests: UCN
identifiers remain unimplemented, trigraph and pedantic behavior are not
covered, and no whole-language parity or performance gain is claimed.
