# Label identity and i686 trampoline threading

## Invariants

Control-flow equivalence is not address equivalence. Removing the sole
instruction between observable labels can change `&&a != &&b`; retargeting a
short-only branch can exceed its architectural displacement range. Neither is
proved safe by a nearby textual label or by a successful assembly on one input.

The implementation distinguishes five uses: definitions, observable addresses,
short-only branches, relaxable branches, and proven dispatch entries. Unknown
uses retain identity. Inline assembly is opaque and never rewritten. References
are collected across the module, including data emitted before later functions.

## Architecture and correctness argument

### One classification contract

`visit_label_uses` supplies both the lazy identity-pin set used by jump-to-next
folds and the indexed reference census used by threading. They cannot silently
disagree about whether a `.long` is an address or dispatch. Dispatch requires a
contiguous run under a base proven by an actual indirect `jmp` or PIC `leal`;
quoted text and data mentioning `@GOTOFF` do not prove a base. Both operands of
unrecognized label differences are identity uses. Comments are not references.

Each candidate must have a unique definition, a unique target in its function
window, exactly one unconditional jump in its body, and no incoming physical
fallthrough. Those preconditions are checked before any mutation.

### Analyze, resolve, then rewrite

The threader constructs a functional graph of candidates in assembly order.
An iterative path-compression walk resolves every acyclic candidate to a final
non-candidate destination. Cycles and paths into cycles are retained. There is
no recursive stack growth and no dependence on randomized map iteration.

Only after graph resolution does rewriting begin:

- Relaxable branches may bypass even identity-pinned trampolines: their original
  label and separating jump remain when needed.
- Dispatch entries may bypass a candidate only when its address is not pinned.
- Short-only references are never changed; they keep the relay at its original
  location relative to surrounding code. This pass does not invent long forms.
- A candidate's label and jump are removed only if neither address nor short-only
  references require them. All rewritten destinations are surviving definitions.
- Inline assembly and trailing comments remain byte-for-byte unchanged.

Analysis is immutable and each indexed reference is rewritten at most once.
There is no mutation-sensitive census cache or reference-list migration. For M
module bytes, C candidates and R symbol references, expected work is O(M+C+R),
with O(C+R) analysis storage (plus definition/name storage). Hash-table lookup
cost is expected, not a worst-case adversarial complexity guarantee. This
replaces multiple module scans per candidate; it does not claim faster generated
programs or eliminate unrelated costs in the rest of the optimizer.

### Block-scoped emitter evidence

The shared codegen driver records an output offset immediately after emitting
an observable block's label. The query sees only that block's emitted suffix;
it cannot mistake a previous block's instruction for evidence of separation.
The mark is private to `CodegenState`, reset per block/function, and a buffer
shorter than the recorded mark fails closed.

A real compiler-authored instruction establishes positive-size evidence.
Metadata and padding do not: `.p2align`, `.align`, `.balign`, `.space`, `.skip`
and `.fill` can emit zero bytes. Unknown directives, section changes, compound
statements and inline assembly invalidate the proof. Unobservable blocks take
an O(1) fast path. Observable blocks scan only their own emitted output.

This deliberately does not add an instruction counter to every emission API:
the output buffer also supports direct writes, deferred emission and truncation.
A parallel counter would require transactional synchronization with all of those
operations. The explicit block boundary plus a conservative output proof covers
them without counting IR instructions that may disappear during lowering. The
predicate still interprets compiler-authored assembly syntax; it is not an
assembler or a claim of fully typed machine-instruction emission.

The i686 jump-to-next guard independently preserves a preceding pinned label
across zero-byte directives; the emitter's proof cannot authorize a later pass
to erase the last separating instruction. Conditional jumps use the same guard.

## Tests

- Actual `.quad` dispatch rewriting with real local symbols and i686 registers.
- Both identity pins for every supported small-width label-difference directive.
- Address-pinned dispatch versus relaxable branches; all short-only mnemonics.
- Fake jump-table evidence in strings, data and inline asm; broken dispatch runs.
- Duplicate definitions, unchanged inline asm/comments, zero-size directives,
  positive/negative conditional-jump folds, misleading target comments, compound
  statement rejection, cycles and paths into cycles.
- A 4,096-link chain resolved in one invocation, deterministic and idempotent.
- All 625 four-node functional graphs, every start node, compared with an
  independent cycle-detecting oracle.
- Emitter metadata/padding, section changes, block/function resets, and rewind.
- The assembly-data checker has seven Python self-tests covering every operand,
  comments, quotes, token boundaries, and multiple statements/definitions.

The existing i686 runtime regression invokes the lexical checker and its
self-tests before running the compiler fixtures. Normal Cargo tests discover the
Rust tests; the previously unregistered inline-assembly test is enabled.

## Validation record

Local provisioning activated a 2 GiB swap file. The official latest stable
release is Rust 1.99.0, but the sandbox cannot reach `static.rust-lang.org`.
The repository and hosted CI continue to select moving `stable` with rustfmt
and Clippy; no MSRV or CI gate has been lowered to fit this sandbox.

The supplementary Rust suite passed 38 tests (the manual timing test is ignored
unless explicitly selected); Python checker self-tests passed all seven cases.
Shell syntax, whitespace, conflict markers and CI/local gate parity also passed.

An older Rust 1.88.0 tool bundle, obtained through the allowed npm registry with
registry integrity verification, supports supplementary isolated tests of the
actual label-pass source and its production classifier/LineStore, plus the pure
emitter predicate. This is **not** a full-crate build, MSRV validation, or a
substitute for hosted stable-Rust CI. State/driver integration is covered by the
normal crate tests on CI, not by the isolated local harness.

Reproducible manual microbenchmark (not part of timed CI acceptance):

```sh
cargo test --profile fastbuild --lib threading_scaling_benchmark -- \
  --ignored --nocapture --test-threads=1
```

It measures only the threading pass on independent relays and chains at three
sizes, reports the median of five runs, and checks each output. Parsing and
fixture construction are outside the timed region. No corpus-wide runtime
speedup, vendor comparison, or hardware-independent timing guarantee is claimed.

### Local A/B measurements

Same x86-64 sandbox (two logical CPUs, reported Xeon 2.60 GHz), same Rust 1.88.0
`rustc --test -C opt-level=1`, same classifier/LineStore and benchmark fixture.
The before/after harnesses extracted the respective production pass bodies;
neither timed fixture parsing. Each cell is a five-run median in microseconds.

| Shape | Candidates | Before | After |
|---|---:|---:|---:|
| Independent relays | 128 | 7,043 | 158 |
| Independent relays | 512 | 67,517 | 816 |
| Independent relays | 2,048 | 1,070,646 | 3,021 |
| Chain | 128 | 4,349 | 182 |
| Chain | 512 | 66,816 | 561 |
| Chain | 2,048 | 1,304,722 | 2,271 |

Both versions' benchmark outputs passed the same structural assertions. These
synthetic pass-level measurements support the complexity improvement; they are
not whole-compiler throughput or generated-code runtime measurements. Shared
host noise, allocator/hash costs and the older local toolchain limit generality.
