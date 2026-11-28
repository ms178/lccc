# Host identity, 2026-11-26: where a campaign ran is reported, not refused

## What was red

`GCC 17 C torture (x86)` runs on pushes to `main`, weekly and on dispatch, not
on pull requests. Thirteen of its last fourteen runs failed, every failed leg
in the step `Enforce whole-corpus per-case ratchet`, and every one with the
same message and no other:

```
{"problems":["identity mismatch: host, host_image"],"omitted":0}
```

(or `host_image` alone). No case had changed in fifteen of the 23 failed legs.
In the other eight three had, and the ratchet did not say so.

## Why

`scripts/torture_ci_ratchet.py` compared the whole `comparison` object of the
report with the baseline's and returned on the first difference, before it
looked at a row. Two of its keys describe the machine the provider handed the
job:

* `host`: `platform.platform()`, which carries the kernel build:
  `Linux-7.0.0-1014-aws-x86_64-with-glibc2.39` or
  `Linux-7.0.0-1017-azure-x86_64-with-glibc2.39`;
* `host_image`: `ImageOS` and `ImageVersion`, the second the build of the
  image: `20261111.370.1`, `20261116.133.1`, `20261116.376.1`.

One `runs-on: ubuntu-24.04` is served from two fleets at once, each with its
own kernel and its own build of the image of the week, and the two legs of one
run draw independently. The baseline of 2026-11-24 was captured from run
45388535959, whose i686 leg had drawn the one fleet and whose x86_64 leg the
other, and pinned each leg to the fleet it happened to get. Since then the
workflow is green only when the i686 job draws `aws` and the x86_64 job draws
`azure`. Before that, the baseline of 2026-11-18 (`3842b3da3`) was written
for the image `20261111.370.1` and nothing else -- no row and no other key
differed from its predecessor -- and the build that replaced that image was
on the runners the day after. The one of 2026-11-24 (`e92f4711f`) had the
image to follow again, besides its three rows.

The rows do not depend on the host. Each leg publishes its identity and every
non-pass row as check annotations; the table is those annotations of the
fourteen runs (`gh api repos/ms178/lccc/check-runs/JOB/annotations`), against
the baseline that was checked in at the run's head:

| run | head | leg | ran on | baseline of | ratchet said | rows against that baseline |
|---|---|---|---|---|---|---|
| 45646676499 | `e9aea413a` | i686 | azure `20261116.376.1` | aws `20261116.133.1` | identity mismatch: host, host_image | the same |
| 45646676499 | `e9aea413a` | x86_64 | azure `20261116.376.1` | azure `20261116.376.1` | matches | the same |
| 45506479582 | `8d390163c` | i686 | aws `20261116.133.1` | aws `20261116.133.1` | matches | the same |
| 45506479582 | `8d390163c` | x86_64 | aws `20261116.133.1` | azure `20261116.376.1` | identity mismatch: host, host_image | the same |
| 45388535959 | `307cd03b4` | i686 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | compile: 3 differ |
| 45388535959 | `307cd03b4` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45365244264 | `ec511c301` | i686 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45365244264 | `ec511c301` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45174788820 | `41d43f3a5` | i686 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45174788820 | `41d43f3a5` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45094560271 | `41d43f3a5` | i686 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45094560271 | `41d43f3a5` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | compile: 3 differ |
| 45033411926 | `dbdadc9cd` | i686 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | the same |
| 45033411926 | `dbdadc9cd` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | the same |
| 44937946674 | `a4145e2db` | i686 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | the same |
| 44937946674 | `a4145e2db` | x86_64 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44928832187 | `e0ccc52fa` | i686 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44928832187 | `e0ccc52fa` | x86_64 | azure `20261116.376.1` | azure `20261111.370.1` | identity mismatch: host_image | the same |
| 44833869768 | `4d2847364` | i686 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44833869768 | `4d2847364` | x86_64 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44800838001 | `29812a70b` | i686 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44800838001 | `29812a70b` | x86_64 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44654902829 | `d82a08587` | i686 | azure `20261111.370.1` | azure `20261111.370.1` | matches | the same |
| 44654902829 | `d82a08587` | x86_64 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44633976155 | `67e97bd10` | i686 | azure `20261111.370.1` | azure `20261111.370.1` | matches | the same |
| 44633976155 | `67e97bd10` | x86_64 | azure `20261111.370.1` | azure `20261111.370.1` | matches | the same |
| 44536528252 | `021ba1ad3` | i686 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |
| 44536528252 | `021ba1ad3` | x86_64 | aws `20261116.133.1` | azure `20261111.370.1` | identity mismatch: host, host_image | the same |

28 legs, 23 failed. In 15 of them the rows were the baseline's, on either
fleet and on three builds of the image. In the 8 from `41d43f3a5` to
`307cd03b4` three compile rows differed, `limits-caselabels.c`,
`limits-exprparen.c` and `limits-structnest.c`, `compile-fail -> pass`: the
fixes reviewed in `REBASELINE-2026-11-24-limits-torture.md`. A fix needs a
review as a failure does, and those four runs were rightly red, but for a
reason the ratchet never printed: the reviewer found the three cases by
diffing captured inventories by hand.

## What changed

The identity has two parts now (`what_ran`, `host_drift` in
`scripts/torture_ci_ratchet.py`).

* **What ran** refuses the comparison as before: target, mode, corpus
  revision and stamp, case set, selection, dependency digest, flags, runner
  and harness digests, oracle driver and its search environment, link mode,
  limits, feature environment, and `host_image.os`, the release that
  `runs-on` names. A key added to the provenance later refuses too, until it
  is put with the host on purpose (a test holds that).
* **Where it ran**, `host` and `host_image.version`, is recorded in the
  baseline (a baseline without them is refused), printed when it differs --
  a `GCC torture ratchet host` notice on the check, in a passing leg and in a
  failing one --, and the rows are compared all the same.

Nothing is given up by that. A host that changes what a case does changes a
row: status, phase, exit code or the digest of the diagnostic, and a missing
or an added case. That comparison is the gate, it fails closed, and it now
runs where it used to be skipped. What the host keys added was a refusal
when no row had changed.

Replayed on the identities and rows of the 28 legs above: the 23 red legs
become 8 red, each on its three changed rows and saying so, and 15 green with
the notice. No leg that passed fails.

The baselines are as they were. They keep the fleet each was captured on, as
provenance.

## Alternatives weighed

* **Another rebaseline.** The fourth in sixteen days for the same reason,
  green until the next image or the next draw of the other fleet.
* **A list of accepted hosts in the baseline.** Holds for the two fleets and
  is red again with the image of next week; each new build would be accepted
  after looking at rows that the ratchet can look at itself.
* **Comparing the date of the image and not its build** (`20261116`).
  Removes the fleet, keeps the week.
* **A container pinned by digest for the campaign.** That would pin headers,
  CRT and binutils, which no key here does (see the README: not hermetic),
  and is a different piece of work: the legs provision their compilers from
  the APT archive cache of the runner. The kernel would still be the
  provider's.

## What this does not claim

The campaign is not hermetic, as before. Nothing here says the next image
cannot change a row; it says that when it does, the row is what fails.
