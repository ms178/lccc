#!/usr/bin/env python3
"""The GCC 17 torture legs must restore the mode bits artifact transport drops.

`upload-artifact`/`download-artifact` carry files in a zip, and a zip keeps no
Unix mode: every restored file lands 0644. The lanes in `ci.yml` and `bench.yml`
never notice, because they ship ONE tarball and extract it — `tar -xzf` restores
the mode from the tar header — and `compiler-research.yml` hits the limitation
deliberately and calls `chmod +x` on every binary it downloads.

The GCC 17 leg shipped the binaries themselves and did neither, so it failed its
first `[[ -x $compiler ]]` guard in under a second on every run it ever had,
with nothing but "exit code 2" visible through the checks API. These tests pin
the repair so the next person to add a binary to that artifact cannot
reintroduce it:

  * the leg restores the mode of every binary the build job uploads, and proves
    the compiler actually runs, before the corpus step;
  * the corpus script distinguishes a missing file from an unexecutable one,
    because they have different remedies and used to print the same sentence.
"""

from __future__ import annotations

import unittest
from pathlib import Path

import yaml

REPO = Path(__file__).resolve().parents[2]
LEG = REPO / ".github" / "workflows" / "gcc17-torture-leg.yml"
BUILD = REPO / ".github" / "workflows" / "gcc17-torture.yml"
CORPUS = REPO / "scripts" / "ci_gcc17_torture.sh"

MODE_STEP = "Restore the executable bits artifact transport drops"
CORPUS_STEP = "Run full GCC 17 compile + execute corpus"


def steps(path: Path, job: str | None = None) -> list[dict]:
    workflow = yaml.safe_load(path.read_text())
    jobs = workflow["jobs"]
    if job is None:
        assert len(jobs) == 1, f"{path.name} should define exactly one job"
        return next(iter(jobs.values()))["steps"]
    return jobs[job]["steps"]


def uploaded_binaries() -> list[str]:
    """Every binary the build job puts in the artifact, by basename."""
    upload = next(
        step for step in steps(BUILD, "build")
        if str(step.get("uses", "")).startswith("actions/upload-artifact")
    )
    paths = upload["with"]["path"].split()
    return [
        Path(entry).name
        for entry in paths
        if Path(entry).name.startswith("lccc")
    ]


class ArtifactModeTest(unittest.TestCase):
    def test_build_uploads_binaries_rather_than_a_tarball(self) -> None:
        # The premise of the repair: if this ever becomes a tarball, the chmod
        # step is what has to change, and this test is where that is noticed.
        self.assertIn("lccc", uploaded_binaries())
        self.assertNotIn("lccc-fast.tar.gz", uploaded_binaries())

    def test_leg_restores_every_uploaded_binary(self) -> None:
        body = None
        for step in steps(LEG):
            if step.get("name") == MODE_STEP:
                body = step["run"]
        self.assertIsNotNone(body, f"the leg has no '{MODE_STEP}' step")
        self.assertIn("chmod +x", body)
        for name in uploaded_binaries():
            self.assertIn(name, body, f"{name} is uploaded but its mode is not restored")

    def test_leg_proves_the_compiler_runs(self) -> None:
        body = next(step["run"] for step in steps(LEG) if step.get("name") == MODE_STEP)
        # A truncated download passes an `-x` test; only executing it does not.
        self.assertIn("target/fastbuild/lccc --version", body)

    def test_modes_are_restored_before_the_corpus_runs(self) -> None:
        names = [step.get("name") for step in steps(LEG)]
        self.assertIn(MODE_STEP, names)
        self.assertIn(CORPUS_STEP, names)
        self.assertLess(names.index(MODE_STEP), names.index(CORPUS_STEP))

    def test_identity_check_still_precedes_the_mode_repair(self) -> None:
        # Order matters: the sha256 check is what proves the bytes, the chmod is
        # what proves the transport. Repairing modes first would let a corrupted
        # download reach the corpus step.
        names = [step.get("name") for step in steps(LEG)]
        self.assertLess(names.index("Verify compiler artifact identity"), names.index(MODE_STEP))

    def test_corpus_script_separates_missing_from_unexecutable(self) -> None:
        script = CORPUS.read_text()
        self.assertIn("compiler not found", script)
        self.assertIn("compiler is not executable", script)
        self.assertIn("standalone linker not found", script)
        self.assertIn("standalone linker is not executable", script)


if __name__ == "__main__":
    unittest.main()
