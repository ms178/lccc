# Literal payload preservation — frontend follow-through

## Reproduced failures

Reviewing S86's synthetic-boundary repair exposed an additional defect in the
serialization layer: global marker replacement modified literal payloads.
The older blue-paint/paste bytes 0x01–0x03 were also globally erased. Raw 0x04
inside a quoted literal became a space. Such literal contents are accepted by
GCC; they must not be confused with metadata outside preprocessing tokens.

The same review found bytewise stringification corrupting UTF-8: `éλ😀` became
`Ã©Î»ð…`. Two new isolated production-module tests fail before the repair and
pass after it; the existing two boundary tests stay green. The actual S86
compiler builds the new 28-assertion C regression with no diagnostic but its
executable returns1; GCC16 builds it without diagnostics and returns0.

## Design choice

- One quote/escape-aware marker normalizer serves final serialization and
  blue-paint removal for token pasting. It copies literal slices unchanged.
- Token-paste protection scanning skips whole literals, so a literal 0x03
  cannot masquerade as the protection terminator.
- Stringification traverses literal characters, escaping quotes/backslashes
  while preserving UTF-8 and interior whitespace. Outside literals it still
  discards synthetic boundaries and collapses source whitespace.
- Marker-free inputs retain a borrowed fast path; allocation occurs only when
  an actual metadata edit is needed. No measured compile-time win is claimed.
- Rejected: choosing another supposedly unused sentinel (the same collision
  can recur), globally escaping user text (changes source spelling), or deleting
  token separation (reintroduces S85's 25 preprocessing mismatches).

The fix is architecture-independent frontend logic. The raw control bytes in
`tests/regression/macro_literal_payload.c` are deliberate: replacing them with
C escape spellings would stop exercising the bug. Its expected strings use
escape spellings independently of its raw inputs. It also tests placemarker
pasting, two-stage stringification, character literals, escapes and UTF-8.

Frozen production modules, utilities, red/green logs and hashes are under
`evidence/redteam89-literal-payloads/`. Run its `replay.py --out /tmp/literal-replay`.
This isolated harness uses standard hash maps and a flat line resolver; it is
not a substitute for the full compiler test and FAST gates. Current-tree
validation is recorded in the final delivery receipt, not inferred from S86.

## Independent compiler executions

Fresh Godbolt executions of the exact 28-assertion C fixture passed with
`cg162`, `cclang2310`, `cicc2021100` and `cicxlatest`: compile0, didExecute=true,
execution0, timedOut=false. Raw responses and a normalized index are retained
under `evidence/redteam89-literal-payloads/ce/`. The executor endpoint returned
its execution fields at the top level, not nested in `execResult`; the saved
index follows the actual response shape. These are remote correctness controls,
not local generated-code benchmarks or proof of a newer ICX vendor release.

The audit census initially caught one newly introduced hot-path `unwrap`.
It was removed rather than increasing the baseline; the current census is
883 total panic-class sites and 109 hot-path sites, within the existing limits.
The standalone red/green replay was rerun after that edit: two historical
failures before, all four tests passing after. No gate waiver was introduced.

## Full compiler validation and measured-code identity

The repaired compiler (SHA256
`0f555f740170041c63af9c06d0080e56a37a146b37845a6ad1d83728c9cd2ba7`)
passes **4,808 library tests**, zero failures, eight ignored. The 28-assertion
production C fixture passes eight native Memcheck executions (candidate/GCC,
O0/O1/O2/O3), plus six cross-target executions (candidate/Clang, AArch64 and
RISC-V under QEMU, i686 natively). Compilation/execution diagnostics are empty
for those local controls. The four CE executions are separate; ICC emits its
vendor deprecation remark, retained in the raw response.

Recompiled all45 native benchmark ELFs: **45/45 byte-identical to S86**.
Recompiled all49 compiler-produced objects (C and preprocessed assembly) used by the measured patched-zstd library/CLI:
**49/49 byte-identical**. Relinked the static library, CLI and shared GCC API
driver: **all three byte-identical** to the measured S86 counterparts.
Actual commands, old/new hashes and diagnostic fields are retained in
`code-identity.json.gz`. Thus the frontend repair fixes the new witnesses
without changing the measured benchmark artifacts. It is **not a generated-code
performance improvement**; the previously measured costs remain unresolved.

Original `redteam86-frontend/probes.c` bytes are preserved as `probes.c.gz`
(verified round-trip). The readable `.c` mirror removes trailing whitespace
only; the archived original is unchanged.
The portable replay generates a separate copy in its requested output folder.
