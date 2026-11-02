# ORACLE-CACHE-1 — one persistent Compiler Explorer cache for the whole tree

**Status:** implemented, self-tested, gated (`godbolt-cache-selftest`).
**Found by:** red-team review of upstream PR #677 while rebasing onto it.
**Files:** `scripts/godbolt_cache.py`, `scripts/test_godbolt_cache.py` (new);
`scripts/godbolt.py`, `scripts/codegen_oracle.py`, `scripts/encdiff.py`,
`tools/oracle/godbolt_oracle.py`, `scripts/ci_local.sh`,
`.github/workflows/ci.yml` (modified).

---

## 1. The defect

Four tools in this tree ask Compiler Explorer (CE) for the same
`(compiler, flags, source)` tuples:

| tool | what it measures | cache before |
|---|---|---|
| `scripts/codegen_oracle.py` | static instruction survey (`--rank`) | on-disk, `<repo>/.godbolt-cache/` |
| `scripts/encdiff.py` | encoding diff (binary opcodes) | on-disk, **`.godbolt-cache` relative to cwd** |
| `scripts/oracle_asm.py` | offline per-function asm dump | reuses `codegen_oracle`'s |
| `tools/oracle/godbolt_oracle.py` | execution oracle (semantics + size) | **process-wide `dict`** |

Three different designs, three different locations. Two consequences, both of
which had already cost time before this was written:

**(a) The most rate-limit-expensive job had no persistence.** The execution
oracle is the heaviest CE consumer in the tree by construction: the API
returns assembly only from a *non*-executing request, so `remote()` issues
**two** requests per compiler per program — an asm pass and an execute pass.
With a default oracle set of three vendors and eight programs that is ~48
requests per sweep, all re-issued on every invocation, because the memoising
`dict` died with the process. CE rate-limits hard (HTTP 429). The practical
results were:

* a sweep could not be **resumed** after tripping the limiter — the partial
  work was discarded with the process;
* a published table could not be **reproduced** offline, because the evidence
  evaporated the moment the producing run exited;
* the tool could not run in **CI** at all, since every CI run would have
  re-bought the same 48 requests.

This is the same failure `scripts/oracle_asm.py` was written to work around in
the first place — its docstring records that a redundant second request per
compiler "collides with CE's rate limiter (HTTP 429) and the deep-dive
silently degrades to *all oracles ERROR*". PR #677 fixed the *question* the
oracle asks (semantics before size) but left the transport unpersisted.

**(b) `encdiff.py`'s cache location depended on the working directory.** It
defaulted to the **relative** path `.godbolt-cache`, while `godbolt.py`
resolved an absolute `<repo>/.godbolt-cache`. Run `encdiff.py` from anywhere
but the repository root and it silently built a second, private cache in that
directory: every request re-issued, and a stray `.godbolt-cache/` left behind
wherever the tool happened to be invoked. A cache whose hit rate is a function
of your shell's `pwd` is not a cache.

## 2. Why not just "add a cache to the new tool"

Two of the three designs were already individually reasonable. The defect was
that there were three of them. Adding a fourth would have fixed the symptom
(the new tool re-requests) while leaving the actual problem — *N tools, N
caches, N× the traffic and N independent corruption policies* — in place, and
worse, would have left two caches holding **the same tuples under different
keys**, so a `--rank` sweep would warm a cache the execution oracle could
never read.

The fix has to be **one cache, one policy, four callers**, which is what
`scripts/godbolt_cache.py` is.

## 3. The contract

**Namespaced.** Records are keyed by `(namespace, *parts)`. A namespace is a
*format contract*, not a label:

| namespace | contents |
|---|---|
| `att-v2` | AT&T-syntax assembly, text |
| `oracle-v1` | execution-oracle records, JSON |
| `encdiff-v1` | raw CE `/compile` replies including binary opcodes, JSON |

`att-v2` is named that way because a pre-v2 cache stored **Intel** syntax,
which made AT&T load/store metrics silently read as zero — in a table that
then looked like a triumph. Bumping the namespace is the sanctioned way for a
format change to invalidate old records instead of poisoning new
measurements, which is why it is in the key and not in a comment.

**Collision-proof.** Parts are joined with NUL, not a printable separator, so
no choice of source text can forge a field boundary. `("a,b", "c")` and
`("a", "b,c")` must not collide; with a comma separator they would. In a
compiler oracle a forged key means **one program's result is served for
another program's** — a confident wrong answer, which is the worst kind.

**Atomic.** Writers write a sibling temp file named per `(pid, thread)` and
`os.replace` it into position. `codegen_oracle --rank` fans out over threads,
so the temp name has to be per-thread; and a killed or Ctrl-C'd sweep must
never leave a truncated record that later reads as a hit.

**Corruption is a miss, never an exception.** A truncated record from a killed
sweep is the common case. A cache that can abort a sweep is strictly worse
than no cache.

**Empty is a hit.** An empty assembly body is real data — a compiler that
inlined a function away entirely. `load_lines` returns `[]`, never `None`;
collapsing it to a miss would make `--rank` drop the column and hide exactly
the case where lccc deleted the whole body.

## 4. Not every failure is cacheable

This is the part that is easy to get wrong and expensive to get wrong.
`remote()` previously cached **every** result, including failures. Cached
persistently, that means a single transient 429 becomes a **permanent
verdict**: the oracle is recorded as failed, every future sweep reads that
record and skips it, and it never comes back without deleting the cache by
hand. One bad network minute would silently and permanently shrink the oracle
set — and a shrinking oracle set is invisible, because the sweep still
reports PASS on the oracles that did run.

So failures are classified:

| failure | cached? | why |
|---|---|---|
| transport: 429, 5xx, `URLError`, timeout, bad JSON | **no** | transient; must be retried next run |
| the oracle rejects the program (`code != 0`, no asm) | yes | deterministic; will be identical |
| CE compiles but declines to execute | yes | a property of the program, not of the network |

`--no-cache` forces every request to the network; `--cache-stats` reports
per-namespace record counts.

## 5. Verification

`scripts/test_godbolt_cache.py` — 13 checks, no network, gated in both
`ci_local.sh` and GitHub CI as `godbolt-cache-selftest`:

* JSON and line round-trips are exact;
* a miss is `None` on every entry point;
* **namespace isolation**: the same tuple stored in two namespaces does not
  see each other (the collision that would serve an asm body where an
  execution record was expected);
* **separators are not forgeable** (`("a,b","c") != ("a","b,c")`, and not
  associative either);
* **a truncated or garbage record reads as a miss, not a crash**;
* **no temp files are left behind** after a successful store;
* **an empty body is a hit with an empty body**, not a miss;
* non-ASCII source (a wide string literal in a test program) round-trips;
* **64 concurrent writers** all read back intact — no torn records, which is
  the live `--rank` case.

Upstream's own `tools/oracle/godbolt_oracle_selftest.py` (11 tests) still
passes unmodified, which is the point: the change is transport-level and does
not touch the stream-framing logic it pins.

## 6. What this buys

* A sweep is **resumable** after a 429 instead of discard-and-restart.
* A published table is **reproducible** offline, because the evidence
  outlives the process that gathered it.
* `--rank` and the execution oracle now warm **the same** cache, so running
  both costs one set of requests, not two.
* `encdiff.py`'s hit rate no longer depends on the working directory.
* The execution oracle becomes **CI-viable**, which it was not.

## 7. Deliberately not done

* **No TTL / eviction.** Records are immutable for a given tuple, so the only
  growth is new tuples, and a stale-record bug is worse than a large cache
  directory. If it ever matters, eviction belongs in `godbolt_cache.py` as one
  policy, not per caller.
* **No cross-machine cache sharing.** CE results are a function of the
  compiler id plus the source, so sharing would be safe, but it needs a
  provenance story (who compiled this, with which resolved compiler version)
  that the manifest already carries and this cache does not yet record.
