# GCC 16 provisioning: the 2026-10-21 hosted-runner outage and its fix

## Symptom

`main` at `91bbcda` failed `CI` in two lanes, and the `Test Suite` aggregator
that requires every lane turned red as a consequence:

| Lane | Job | Failing step |
| --- | --- | --- |
| Assembler and linker contracts | 119258356623 | `Install GCC 16 and i686 C runtime/development files` |
| SSA regression corpus | 119277283975 | same step |

Every other lane passed the same step in the same run, 100 s to 105 s, which
looked like flakiness. The two failures took 198 s, which is the signature of
the retry backoff, not of a slow mirror.

## Evidence (from the runner log, not a hypothesis)

```
Get:7 https://ppa.launchpadcontent.net/ubuntu-toolchain-r/test/ubuntu noble InRelease [24.4 kB]
Ign:7  ... noble InRelease
Err:7  https://ppa.launchpadcontent.net/ubuntu-toolchain-r/test/ubuntu/distribs/noble/InRelease
   503  Service Unavailable [IP: 185.125.189.188 443]
W: Some index files failed to download. They have been ignored, or old ones used instead.
E: Unable to locate package gcc-16
E: Unable to locate package gcc-16-multilib
E: Unable to locate package g++-16-multilib
E: Unable to locate package lib32gcc-16-dev
```

The PPA publication itself is healthy: Launchpad's API reports
`gcc-16-multilib 16-20260315-1ubuntu1~24~ppa1` and
`lib32gcc-16-dev 16-20260315-1ubuntu1~24~ppa1` published for `noble amd64`
since 2026-03-16. Nothing was wrong with the packages; the *index fetch*
transiently failed.

## Root cause

`scripts/ci_install_gcc16.sh` inferred index health from `apt-get update`'s
exit status. apt exits **0** when an individual repository index fails to
download: it prints `W: Some index files failed to download` and continues with
the previous lists. So the script believed the index was current, and then
replayed `apt-get install` three times against an index that did not contain
`gcc-16` at all. Those three attempts were guaranteed to fail:

* they never re-fetched the index, so the 503 could not clear;
* their 25 s + 50 s backoff turned a two-second CDN hiccup into a 198 s red
  lane;
* the third attempt ended in a diagnostic that dumped `apt-cache policy` for
  every *regex* match of every requested name — 56 KB dominated by
  `clang-16`, `libflang-16-dev`, `python3-clang-16` — burying the only line
  that mattered. Triage required reconstructing the log through a temporary
  probe workflow because the Actions log endpoint was unreachable from the
  triage client.

Two independent defects produced the confusion:

1. **Unsound success predicate.** `apt-get update` exit status ≠ index usable.
2. **Unsound presence probe.** `apt-cache policy gcc-16` falls back to regex
   matching when the exact name is unknown, so it reported `clang-16`'s
   candidate (`1:16.0.6-23ubuntu4`) as "gcc-16 is available". Depending on
   which unrelated packages a runner image ships, the PPA fallback could be
   skipped entirely.

## Fix

`scripts/ci_install_gcc16.sh` now keys every decision on the real contract:
*every requested package is resolvable under its exact name*.

* `package_known` uses `apt-cache pkgnames <name>` plus `grep -Fx`, which
  matches real package names only. `missing_packages` is the single source of
  truth for both the boolean test and the failure report, so they cannot
  disagree.
* `ensure_packages_resolvable` refreshes the index and re-checks
  resolvability, up to a bounded number of rounds. A 503 that clears on the
  next fetch is recovered; a persistent one is reported, not replayed.
* `install_packages` refreshes the index between failed attempts, because the
  two apt failure families that survive `Acquire::Retries` — "unable to
  locate" from a partial index and 404s from a rotated mirror URL — are both
  index problems, and a plain replay is not.
* `apt-get update` runs with `Acquire::Retries=3` (apt-level retry for
  transient HTTP failures) and `Acquire::Languages=none`. The translation
  indexes are ~2 MB per run that nothing in this repository reads, and the
  PPA's `i18n/Translation-en` index is a recurring 503 source — it is the URL
  in the failure above.
* The distribution is still tried before the PPA, with exactly one refresh
  before the decision, so an Ubuntu lane does not spend a minute of backoff
  waiting for a package its distribution will never carry. A non-Ubuntu host
  still refuses an Ubuntu PPA.
* The failure report is bounded and high-signal: exact missing package names,
  the failing index fetches (`Err:`/`E:`/`W:`, at most 20 lines) and the
  resolver's own verdict (`--dry-run`, at most 20 lines).
* `--multilib` now proves `gcc-16 -m32` can link before returning success.
  Every `--multilib` lane runs the i686 differential suites through `gcc -m32`
  (the workflow already asserts `gcc -m32` after cross provisioning); a broken
  32-bit runtime used to surface forty minutes later as dozens of unrelated
  test failures.
* The helper publishes one bounded `::error` annotation itself (only when
  `GITHUB_ACTIONS=true`) and every lane invokes it directly, replacing the
  single bespoke capture wrapper that only the `frontend` job had. All six
  lanes are now diagnosable from the Checks API.

## Second outage, same day: the CDN edge itself

While the fix above was being verified, `ppa.launchpadcontent.net` stopped
serving the PPA altogether, and then started flapping: the `build` lane
provisioned successfully at 15:42, the next lane's run at 15:48 got

```
Err:7 https://ppa.launchpadcontent.net/ubuntu-toolchain-r/test/ubuntu noble InRelease
W: Failed to fetch .../dists/noble/InRelease  503  Service Unavailable [IP: 185.125.189.188 443]
```

No retry policy recovers from a host that is down, and noble has no other
source of GCC 16. Two mitigations were considered.

### Adopted: cached index + bounded patience

* apt's index lives in the cached archive directory
  (`-o Dir::State::lists=$LCCC_CI_APT_ARCHIVE_DIR/lists`), so the existing
  weekly `actions/cache` entry persists it next to the debs. A warm runner
  resolves `gcc-16` from the cached index and installs from the cached debs
  **without contacting the CDN at all** — that is both the outage mitigation
  and a latency win on every lane.
* The refresh budget after the PPA is configured is four rounds with linear
  backoff (25/50/75/100 s between rounds, three apt attempts per round with
  `Acquire::Retries=3` each). That covers the observed flap window without
  turning a two-second hiccup into a 198 s failure and without retrying for
  an unbounded time.
* The rolling-major `16.x` policy tolerates a week-old index; when
  `LCCC_CI_GCC_VERSION` pins an exact version the post-install check still
  rejects a stale toolchain instead of silently accepting it.

### Rejected: retargeting to the legacy `ppa.launchpad.net` host

It looks equivalent — the legacy name serves the same archive over plain
HTTP — and it was implemented first. CI proved it cannot work:

```
W: Failed to fetch https://ppa.launchpad.net/ubuntu-toolchain-r/test/ubuntu/dists/noble/InRelease
   Certificate verification failed: The certificate is NOT trusted.
   The name in the certificate does not match the expected.
```

The alias presents a certificate that does not cover the legacy name, so apt
rejects the index. Disabling peer verification to work around it is not
acceptable, and the retarget was removed. The rejection is recorded here so
nobody re-invents it.

### What CI actually observed

On the first run of this change the primary CDN edge was healthy again:
`Install GCC 16 and i686 C runtime/development files` completed successfully
in the `build` lane with **no** degraded-provisioning annotation, i.e. the
fallback retarget was available but not taken. The outage that motivated it
lasted hours; the fallback exists for the next one and is covered by the
model tests rather than by a live outage.

## Regression coverage

`tests/regression/check_ci_gcc16_provisioning.py` models apt instead of
mocking the script's assumptions:

* `apt-get update` can exit 0 while the PPA index is unpublished
  (`PPA_INDEX_FAILURES`), which reproduces the outage without a network;
* `apt-cache policy` reproduces the regex fallback, including the `clang-16`
  decoy;
* `apt-get install` refuses any package that is not in the modelled index, so
  a script that installs from an unusable index fails the suite.

The stub also models the two PPA hosts: `PRIMARY_HOST_DOWN` refuses every
index while the source points at the current CDN edge (republishing once the
source is retargeted), and `LEGACY_HOST_DOWN` refuses the fallback, so the
"both hosts down" failure path is exercised too.

The previous revision of the script fails 9 of the 19 original tests (mutation
check); the new one passes all 22. `tests/corpus/test_ci_failure_annotations.py` pins
the annotation contract: single line, percent-escaped, status preserved, no
annotation outside Actions, and no lane reintroducing a capture wrapper.

## Invariants for future changes

* Never treat `apt-get update` exit status as proof that a package is
  installable.
* Never probe package presence with `apt-cache policy` (regex fallback) or
  `apt-cache show` (same matcher); use `apt-cache pkgnames` + `grep -Fx`.
* Never retry an operation whose inputs are known to be wrong; refresh the
  index instead.
* Keep failure output bounded and exact — a 56 KB dump is not a diagnosis.
