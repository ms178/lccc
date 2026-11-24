# Rebaseline 2026-11-24: three root-fixed torture cases; runner-image identity drift

## What changed and why this file exists

The per-case ratchet correctly refused [run 45388535959](https://github.com/ms178/lccc/actions/runs/45388535959)
(head `307cd03b495d13c8224ec59b6db433d0bd094ebf`, current `main`) on both x86
targets. Two distinct, disjoint causes:

1. **Runner-image identity drift.** `ubuntu-24.04` moved for the hosted legs:
   `host_image` `20261111.370.1` -> `20261116.376.1` (x86_64; host kernel string
   unchanged: `Linux-7.0.0-1017-azure-x86_64-with-glibc2.39`) and
   `20261111.370.1` -> `20261116.133.1` (i686; host kernel changed:
   `Linux-7.0.0-1017-azure-x86_64-with-glibc2.39` -> `Linux-7.0.0-1014-aws-x86_64-with-glibc2.39`;
   the two arch legs drew different runner SKUs on the same image revision).
   Everything else in the comparison is byte-identical to the committed
   baseline: corpus stamp `c94b688a83fc…`, case-set digests, dependency/stamp
   digests, runner/harness/oracle digests (`a5e35a5ac7ca`, `8f48f763e9f4`,
   `67ed99639f74`, `96367560ae96`), selection metadata.
2. **Three cases flipped `compile-fail` -> `pass`**, on both arches and both
   within the `-O2` compile leg:

   ```
   limits-caselabels.c  compile-fail -> pass
   limits-exprparen.c   compile-fail -> pass
   limits-structnest.c  compile-fail -> pass
   ```

   These are exactly the three torture compile-fails accountable to the
   2026-11-21 flat-label-run / paren-chain-run / record-trampoline work whose
   fixes were merged to `main` before this run; the 2026-11-19 session
   (FOLLOWUP-2026-11-19 / the 2026-11-21 follow-up note, item 5) explicitly
   predicted `1995p/5f -> 1998p/2f` (x86_64) and `1988p/5f -> 1991p/2f` (i686)
   "expect after the wave". The remaining non-pass outcomes are the two
   documented deliberate refusals (`pr115143-2.c`, `pr115143-3.c`, GCC GIMPLE
   IR syntax LCCC does not implement; accepted contract per
   `check_parser_depth_limits.sh`) and are unchanged as rows.

The ratchet's contract is that a fix, like a new failure, requires explicit
baseline review rather than being silently accepted. This note is that review.

## Review procedure actually executed

1. Retrieved the run and job metadata with the public GitHub API (run
   `45388535959`, jobs `138427340681` x86_64 / `138427340898` i686; head
   `307cd03b4`), never with a cached copy. Job names, heads, statuses and step
   conclusions were verified by the capture tool before use.
2. Retrieved `check-runs/<job>/annotations` (both jobs, complete, 12/13 rows)
   directly from the API. No job log access was possible (public-API 403); all
   evidence used here is the published annotation set plus the committed
   baseline itself.
3. Ran the sanctioned capture tool on that input:

   ```
   python3 scripts/torture_ci_capture.py --run run.json --jobs jobs.json \
       --annotations-dir <dir> --out-dir <empty> --allow-ratchet-failure
   ```

   `--allow-ratchet-failure` was required because the ratchet refused on the
   image-identity mismatch; that is the flag's documented purpose.
4. Diffed the proposed baselines against the committed ones, mechanically:
   the ONLY `comparison` differences are `host_image` (and `host` on i686);
   the ONLY outcome-row differences are the three `limits-*.c` removals in the
   two compile inventories (status `compile-fail` -> absent, i.e. pass); counts
   move exactly as predicted (1995->1998, 1988->1991 pass; 5->2 compile-fail);
   execute inventories are identical except `host_image`/(`host`); no other row
   moved, no diagnostic digest changed, no skip appeared or vanished.
5. The three flipped cases are compiler-fix outcomes, not corpus edits: the
   corpus pin is unchanged (`c94b688a83fc…`), the runner and harness digests are
   unchanged, and the fixes carry their own regression fixtures and oracles in
   the merged tree (`flat_case_label_runs.c`, `paren_chain_deep.c`,
   `struct_nest_deep.c`).

## Statements

* This rebaseline accepts ONLY the three reviewed status transitions above and
  the runner-image identity refresh. Any other changed outcome would fail the
  next ratchet comparison and must be treated as a regression or a corpus
  defect until proven otherwise.
* The known refusals (`pr115143-2.c`, `pr115143-3.c`) remain in the baseline as
  `compile-fail` rows; they are not masked, skipped or re-labelled.
* No outcome row was edited by hand; all four JSONs were produced by
  `scripts/torture_ci_capture.py` and are internally checked (schema, chunk
  completeness, counts, uniqueness, no reference failures) by that tool before
  acceptance.
* This is still not a hermetic oracle: the caveats in README.md apply
  unchanged. The image refresh is comparable evidence, not proof of host
  equivalence.
