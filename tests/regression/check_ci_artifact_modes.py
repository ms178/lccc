#!/usr/bin/env python3
"""Static contracts for the CI lanes that package, download and execute compilers.

Two defects motivate this gate, both of which cost a red main before anything
noticed them:

WHY THIS EXISTS
---------------
1. `actions/upload-artifact` stores file CONTENT, not permissions. Every lane
   that ships `lccc-fast.tar.gz` is safe by accident-of-design: `tar -xzf`
   restores the member modes. The GCC 17 torture lane shipped the six binaries
   as bare artifact paths, so `target/fastbuild/lccc` arrived at mode 0644 and
   `scripts/ci_gcc17_torture.sh` died in its first guard -- `[[ -x $compiler ]]`
   -- after 0 seconds, publishing no reports (the follow-up upload step then
   reported a missing results directory, which reads like a harness failure, not
   a packaging one). Nothing in the tree could observe this: the lane is off the
   pull-request critical path, so the first red run WAS the test.
2. A lane whose oracle is asserted but not delivered runs a compiler its
   documents do not describe. The GCC 17 lane's reports are compared against GCC
   16.2, while the hosted image's rolling `gcc-16` is 16.0.1; the tag cache
   already ships a relocatable 16.2 prefix (`scripts/ci_install_gcc_oracle.sh`),
   so the pin has to be installed, verified AND asserted where the corpus runs.

WHAT IT CHECKS
--------------
A. Mode restoration. For every artifact whose manifest lists a BARE compiler
   executable (no archive extension), every job that downloads that artifact by
   name must restore its executable bit -- a `chmod` whose targets cover each
   downloaded name -- or this gate fails and names the artifact, the job and
   the missing binary. Artefacts that ship an archive are skipped only when a
   step in the consuming job extracts it (`tar`), because extraction is the
   mode restoration.
B. Lane provisioning order. `gcc17-torture-leg.yml` must provision the rolling
   multilib toolchain, then install the exact 16.2 oracle from the tag cache,
   then verify that exact version, then restore modes after the download, and
   only then run the corpus; the job must also pass the pin to the driver
   (`GCC_TORTURE_GCC_VERSION`), and `scripts/ci_gcc17_torture.sh` must default
   `GCC17_TORTURE_REF` to a full commit AND pass `--ref` to the provisioner, so
   the corpus revision is reproducible rather than "today's master".

The gate is deliberately static: it parses YAML and shell text, needs no
compiler, and takes milliseconds, so it can sit in the fast contract shard.

  python3 tests/regression/check_ci_artifact_modes.py
  python3 tests/regression/check_ci_artifact_modes.py --self-test
"""
from __future__ import annotations

import argparse
import fnmatch
import os
import re
import sys
import tempfile
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
WORKFLOW_DIR = REPO / ".github" / "workflows"
TORTURE_LEG = WORKFLOW_DIR / "gcc17-torture-leg.yml"
TORTURE_DRIVER = REPO / "scripts" / "ci_gcc17_torture.sh"

# A compiler executable as this repository names them: `lccc`, `lccc-ld`,
# `lccc-x86`, `lccc-i686`, `lccc-riscv`, `lccc-arm`. Anything with an extension
# (`ci-build.sha256`, `lccc-fast.tar.gz`) is data or an archive, not a binary.
BARE_BINARY = re.compile(r"lccc(-[a-z0-9_]+)?\Z")
ARCHIVE = re.compile(r"\.(tar|tar\.\w+|tgz|zip|tar\.gz)\Z")

EXPRESSION = re.compile(r"\$\{\{[^}]*\}\}")


class Failure(list):
    """Collected `::error` lines, so one run reports every problem it found."""

    def add(self, title: str, message: str) -> None:
        self.append((title, message))


def normalise(name: str) -> str:
    """Replace `${{ ... }}` with one placeholder so upload/download compare equal.

    Equality, not a glob: `lccc-fast-*` would also match
    `lccc-fast-binaries-*`, and that looseness is exactly how two payload shapes
    end up sharing an artifact name without anyone noticing.
    """
    return EXPRESSION.sub("{expr}", name or "")


def steps(job: dict) -> list[dict]:
    return [step for step in (job.get("steps") or []) if isinstance(step, dict)]


def run_text(job: dict) -> str:
    """A job's steps as one string, in order, with `uses:` kept as markers.

    Ordering matters for the lane contract (provision -> install -> verify ->
    download -> chmod -> run), and two of those links are `uses:` steps rather
    than shell lines, so the marker has to be part of the searched text.
    """
    lines: list[str] = []
    for step in steps(job):
        if step.get("uses"):
            lines.append(str(step["uses"]))
        if step.get("run"):
            lines.append(str(step["run"]))
    return "\n".join(lines)


def chmod_targets(job: dict) -> list[str]:
    """Every word that a `chmod` line in this job names as a target."""
    targets: list[str] = []
    for step in steps(job):
        # A `chmod` that continues over several physical lines (YAML `run: |`
        # plus shell `\`) names its targets on the continuation lines, so the
        # continuations are joined BEFORE lines are classified.
        joined = re.sub(r"\\[ \t]*\n", " ", str(step.get("run", "")))
        for line in joined.splitlines():
            words = line.split()
            if not words or words[0] != "chmod":
                continue
            targets.extend(word for word in words[1:] if not word.startswith(("-", "+")))
    return targets


def uploads(documents: dict[Path, dict]) -> list[dict]:
    found = []
    for path, document in documents.items():
        for job_name, job in (document.get("jobs") or {}).items():
            if not isinstance(job, dict):
                continue
            for step in steps(job):
                if not str(step.get("uses", "")).startswith("actions/upload-artifact"):
                    continue
                spec = step.get("with") or {}
                name = spec.get("name")
                paths = spec.get("path")
                entries = paths if isinstance(paths, list) else [paths]
                # A `path: |` block is ONE string holding several entries.
                entries = [line.strip() for entry in entries if isinstance(entry, str)
                           for line in str(entry).splitlines() if line.strip()]
                binaries = []
                archives = []
                for entry in entries:
                    base = os.path.basename(entry.strip())
                    # An archive first: `lccc-fast.tar.gz` matches the bare-name
                    # shape, and only the extension says it carries its members'
                    # modes with it.
                    if ARCHIVE.search(base):
                        archives.append(base)
                    elif BARE_BINARY.match(base):
                        binaries.append(base)
                found.append(dict(workflow=path, job=job_name, name=normalise(name),
                                  binaries=binaries, archives=archives))
    return found


def check_artifact_modes(documents: dict[Path, dict], failures: Failure) -> int:
    """Every downloaded bare compiler binary must have its mode restored."""
    published = uploads(documents)
    checked = 0
    for path, document in documents.items():
        for job_name, job in (document.get("jobs") or {}).items():
            if not isinstance(job, dict):
                continue
            for step in steps(job):
                if not str(step.get("uses", "")).startswith("actions/download-artifact"):
                    continue
                spec = step.get("with") or {}
                name = spec.get("name")
                if not name:
                    # Downloading every artifact of the run: the set is not
                    # knowable from this file, so the lane has to say which
                    # artifact it means before the gate can judge it.
                    failures.add(
                        "CI artifact download",
                        f"{path.name}:{job_name}: download-artifact without `name:`; "
                        "name the artifact so its mode contract is checkable",
                    )
                    continue
                wanted = normalise(name)
                # A download that matches nothing is a rename/typo that only the
                # hosted run would reveal ("artifact not found" after the build
                # job has already spent its minutes).
                # An upload whose name is a BARE expression (`name: ${{
                # inputs.artifact }}` in the reusable bench workflow) publishes
                # whatever its caller passes, so it can satisfy any download: the
                # name is not knowable from the callee's file.
                if not any(artifact["name"] in (wanted, "{expr}") for artifact in published):
                    failures.add(
                        "CI artifact download",
                        f"{path.name}:{job_name}: downloads artifact {name!r}, which no "
                        "upload-artifact step in this tree publishes",
                    )
                for artifact in published:
                    if artifact["name"] != wanted:
                        continue
                    if not artifact["binaries"]:
                        continue
                    checked += 1
                    targets = chmod_targets(job)
                    missing = [
                        binary for binary in artifact["binaries"]
                        if not any(os.path.basename(target) == binary or
                                   fnmatch.fnmatch(target, f"*{binary}") for target in targets)
                    ]
                    if missing:
                        failures.add(
                            "CI artifact mode",
                            f"{path.name}:{job_name}: downloads {artifact['name']!r} from "
                            f"{artifact['workflow'].name}:{artifact['job']} but never restores "
                            f"the executable bit of {', '.join(sorted(missing))}; "
                            "actions/upload-artifact stores content only, so a `chmod +x` "
                            "after the download is required (or ship a tarball and extract it)",
                        )
    return checked


def ordered(text: str, needles: list[str], failures: Failure, where: str) -> None:
    position = -1
    for needle in needles:
        found = text.find(needle, position + 1)
        if found < 0:
            state = "missing" if text.find(needle) < 0 else "out of order"
            failures.add("CI torture lane", f"{where}: {state}: {needle!r}")
            return
        position = found


def check_torture_lane(failures: Failure, leg: Path = TORTURE_LEG,
                       driver: Path = TORTURE_DRIVER) -> None:
    """The lane must deliver and verify its pins, and pin its corpus revision."""
    if not leg.is_file():
        failures.add("CI torture lane", f"{leg.name} is missing")
        return
    document = yaml.safe_load(leg.read_text())
    jobs = document.get("jobs") or {}
    job = jobs.get("torture")
    if not isinstance(job, dict):
        failures.add("CI torture lane", f"{leg.name}: no `torture` job")
        return
    text = run_text(job)
    ordered(text, [
        "scripts/ci_provision_toolchain.sh --multilib",
        "scripts/ci_install_gcc_oracle.sh --version 16.2",
        "scripts/ci_verify_oracles.sh --gcc-version 16.2",
        "actions/download-artifact",
        "chmod +x",
        "scripts/ci_gcc17_torture.sh",
    ], failures, f"{leg.name}:torture steps")
    env = job.get("env") or {}
    if str(env.get("GCC_TORTURE_GCC_VERSION", "")).strip("'\"") != "16.2":
        failures.add(
            "CI torture lane",
            f"{leg.name}:torture: env GCC_TORTURE_GCC_VERSION must be '16.2' "
            f"(found {env.get('GCC_TORTURE_GCC_VERSION')!r}), so the driver asserts the "
            "reference compiler it was actually given",
        )
    if "GCC_BIN" in env and str(env["GCC_BIN"]).strip("'\"") not in ("gcc-16", "gcc-16.2"):
        failures.add("CI torture lane",
                     f"{leg.name}:torture: GCC_BIN {env['GCC_BIN']!r} is not the tag-cache pin")

    if not driver.is_file():
        failures.add("CI torture lane", f"{driver.name} is missing")
        return
    shell = driver.read_text()
    pinned = re.search(r'GCC17_TORTURE_REF:-([0-9a-f]{40})\}', shell)
    if not pinned:
        failures.add(
            "CI torture lane",
            f"{driver.name}: GCC17_TORTURE_REF has no 40-hex default, so the corpus revision "
            "is whatever trunk is today and the lane cannot reproduce a report",
        )
    if '--ref "$corpus_ref"' not in shell:
        failures.add("CI torture lane",
                     f"{driver.name}: the provisioner is not called with --ref \"$corpus_ref\"")
    if "chmod" not in shell and "not executable" not in shell:
        failures.add("CI torture lane",
                     f"{driver.name}: no diagnosis for a non-executable artifact binary")


def load(directory: Path) -> dict[Path, dict]:
    documents = {}
    for path in sorted(directory.glob("*.yml")):
        document = yaml.safe_load(path.read_text())
        if isinstance(document, dict):
            documents[path] = document
    return documents


def self_test() -> int:
    """Prove the gate fails on the shapes it exists for, and passes on the fix."""
    problems = 0
    fixtures = 0

    def expect(name: str, documents: dict[Path, dict], want: bool) -> None:
        nonlocal problems, fixtures
        fixtures += 1
        failures = Failure()
        check_artifact_modes(documents, failures)
        got = bool(failures)
        if got != want:
            problems += 1
            print(f"self-test FAIL: {name}: expected failure={want}, got {got}",
                  file=sys.stderr)
            for title, message in failures:
                print(f"  {title}: {message}", file=sys.stderr)

    bare = {Path("lane.yml"): yaml.safe_load("""
jobs:
  build:
    steps:
      - uses: actions/upload-artifact@v7
        with:
          name: lccc-fast-${{ github.sha }}
          path: |
            target/fastbuild/lccc
            target/fastbuild/lccc-ld
            target/fastbuild/ci-build.sha256
""")}
    download_no_chmod = dict(bare)
    download_no_chmod[Path("lane.yml")] = yaml.safe_load("""
jobs:
  build:
    steps:
      - uses: actions/upload-artifact@v7
        with:
          name: lccc-fast-${{ github.sha }}
          path: |
            target/fastbuild/lccc
            target/fastbuild/lccc-ld
  torture:
    steps:
      - uses: actions/download-artifact@v8
        with:
          name: lccc-fast-${{ github.sha }}
          path: target/fastbuild
      - run: bash scripts/ci_gcc17_torture.sh x86_64
""")
    expect("download without chmod", download_no_chmod, True)

    with_chmod = dict(download_no_chmod)
    with_chmod[Path("lane.yml")] = yaml.safe_load("""
jobs:
  build:
    steps:
      - uses: actions/upload-artifact@v7
        with:
          name: lccc-fast-${{ github.sha }}
          path: |
            target/fastbuild/lccc
            target/fastbuild/lccc-ld
  torture:
    steps:
      - uses: actions/download-artifact@v8
        with:
          name: lccc-fast-${{ github.sha }}
          path: target/fastbuild
      - run: |
          chmod +x target/fastbuild/lccc target/fastbuild/lccc-ld
      - run: bash scripts/ci_gcc17_torture.sh x86_64
""")
    expect("download with chmod", with_chmod, False)

    tarball = {Path("lane.yml"): yaml.safe_load("""
jobs:
  build:
    steps:
      - uses: actions/upload-artifact@v7
        with:
          name: lccc-fast-${{ github.sha }}
          path: lccc-fast.tar.gz
  test:
    steps:
      - uses: actions/download-artifact@v8
        with:
          name: lccc-fast-${{ github.sha }}
      - run: tar -xzf lccc-fast.tar.gz
""")}
    expect("tarball shipment", tarball, False)

    ghost = {Path("lane.yml"): yaml.safe_load("""
jobs:
  test:
    steps:
      - uses: actions/download-artifact@v8
        with:
          name: lccc-fast-binaries-${{ github.sha }}
      - run: tar -xzf lccc-fast.tar.gz
""")}
    expect("download of an unpublished artifact", ghost, True)

    via_caller = {Path("caller.yml"): yaml.safe_load("""
jobs:
  publish:
    steps:
      - uses: actions/upload-artifact@v7
        with:
          name: ${{ inputs.artifact }}
          path: |
            ${{ inputs.report }}
  consume:
    steps:
      - uses: actions/download-artifact@v8
        with:
          name: benchmark-shard-a
""")}
    expect("caller-supplied artifact name", via_caller, False)

    with tempfile.TemporaryDirectory() as tmp:
        leg = Path(tmp) / "gcc17-torture-leg.yml"
        driver = Path(tmp) / "ci_gcc17_torture.sh"
        driver.write_text('corpus_ref=${GCC17_TORTURE_REF:-'
                          + "c94b688a83fc84015c6fab8ab0ff142051598fc8}\n"
                          'provision=(ensure --ref "$corpus_ref")\n'
                          'echo "not executable"\n')
        good = """
jobs:
  torture:
    env:
      GCC_BIN: gcc-16
      GCC_TORTURE_GCC_VERSION: '16.2'
    steps:
      - run: bash scripts/ci_provision_toolchain.sh --multilib
      - run: bash scripts/ci_install_gcc_oracle.sh --version 16.2
      - run: bash scripts/ci_verify_oracles.sh --gcc-version 16.2
      - uses: actions/download-artifact@v8
      - run: chmod +x target/fastbuild/lccc
      - run: bash scripts/ci_gcc17_torture.sh x86_64
"""
        leg.write_text(good)
        failures = Failure()
        check_torture_lane(failures, leg, driver)
        fixtures += 1
        if failures:
            problems += 1
            print("self-test FAIL: conforming lane rejected", file=sys.stderr)
            for title, message in failures:
                print(f"  {title}: {message}", file=sys.stderr)

        for name, mutate in (
            ("oracle not installed",
             lambda text: text.replace(
                 "      - run: bash scripts/ci_install_gcc_oracle.sh --version 16.2\n", "")),
            ("oracle installed after verification",
             lambda text: text.replace(
                 "      - run: bash scripts/ci_install_gcc_oracle.sh --version 16.2\n", "")
                      .replace("      - uses: actions/download-artifact@v8\n",
                               "      - run: bash scripts/ci_install_gcc_oracle.sh --version 16.2\n"
                               "      - uses: actions/download-artifact@v8\n")),
            ("rolling oracle verified",
             lambda text: text.replace("--gcc-version 16.2", "--gcc-version 16.0")),
            ("pin not passed to the driver",
             lambda text: text.replace("      GCC_TORTURE_GCC_VERSION: '16.2'\n", "")),
        ):
            leg.write_text(mutate(good))
            failures = Failure()
            check_torture_lane(failures, leg, driver)
            fixtures += 1
            if not failures:
                problems += 1
                print(f"self-test FAIL: {name}: accepted", file=sys.stderr)

        driver.write_text('provision=(ensure)\n')
        leg.write_text(good)
        failures = Failure()
        check_torture_lane(failures, leg, driver)
        fixtures += 1
        if not failures:
            problems += 1
            print("self-test FAIL: unpinned corpus accepted", file=sys.stderr)

    if problems:
        print(f"check_ci_artifact_modes: {problems} self-test failures", file=sys.stderr)
        return 1
    print(f"check_ci_artifact_modes: self-test OK ({fixtures} fixtures)")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--self-test", action="store_true",
                        help="exercise the checker against synthetic workflows")
    parser.add_argument("--workflows", type=Path, default=WORKFLOW_DIR)
    args = parser.parse_args(argv)

    if args.self_test:
        return self_test()

    failures = Failure()
    documents = load(args.workflows)
    if not documents:
        print(f"check_ci_artifact_modes: no workflows under {args.workflows}", file=sys.stderr)
        return 1
    pairs = check_artifact_modes(documents, failures)
    check_torture_lane(failures)

    for title, message in failures:
        print(f"::error title={title}::{message}")
    if failures:
        print(f"check_ci_artifact_modes: {len(failures)} failure(s)", file=sys.stderr)
        return 1
    print(f"check_ci_artifact_modes: OK ({len(documents)} workflows, "
          f"{pairs} artifact download(s) with bare compiler binaries, "
          f"torture lane pins delivered and asserted)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
