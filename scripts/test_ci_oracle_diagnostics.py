#!/usr/bin/env python3
"""Every lane provisions its oracles through ONE step, and failures survive it.

The old shape of this contract was a six-line `run:` body repeated in six jobs
(`ci_install_gcc16.sh --multilib` followed by an apt transaction), and this test
executed that body against fake tools to prove that a failing GCC install never
reached the qemu install and that the installer's status became the step's.

Both properties still matter, but they now live in two layers, so they are
checked in the layer that owns them:

* the WORKFLOW guarantees that every lane runs the same one-line provisioning
  command -- a lane that quietly installs a different package set is the
  regression this test exists to prevent;
* `scripts/ci_provision_toolchain.sh` guarantees ordering and status
  propagation, and `tests/regression/check_ci_toolchain_bundle.py` exercises it
  with fake tools (see `test_an_installer_failure_stops_before_any_extra`).

The annotation-encoding helper is still tested here directly: it is the reason
a failed install is readable in Checks instead of a bare "exit code 1".
"""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml

import ci_annotate_log

ROOT = Path(__file__).resolve().parents[1]
PROVISION_STEP = "Provision oracle toolchain (cached bundle, else apt)"
PROVISION_COMMAND = "bash scripts/ci_provision_toolchain.sh --multilib"


class OracleDiagnostics(unittest.TestCase):
    def jobs(self):
        return yaml.safe_load((ROOT / ".github/workflows/ci.yml").read_text())["jobs"]

    def provisioning_bodies(self):
        return [step["run"] for job in self.jobs().values()
                for step in job.get("steps", [])
                if step.get("name") == PROVISION_STEP]

    def lanes_needing_oracles(self):
        """Jobs whose steps call the `gcc-16` oracle, mapped to their steps.

        `gcc-16` is the oracle binary itself; merely running the built
        `target/fastbuild/lccc` (the ICX lane does -- it is the compiler under
        test there) does not need the oracle packages.
        """
        lanes = {}
        for name, job in self.jobs().items():
            bodies = [step.get("run") or "" for step in job.get("steps", [])]
            if any("gcc-16" in body for body in bodies):
                lanes[name] = bodies
        return lanes

    def test_every_lane_provisions_through_the_same_step(self):
        bodies = self.provisioning_bodies()
        self.assertEqual(set(bodies), {PROVISION_COMMAND})
        # The COUNT is a consequence of the lane map, not an invariant: every
        # lane that uses the oracle must carry the step, and the step must not
        # exist where no oracle is used. Pinning a number turned every
        # re-balance (which splits a lane in two) into a test edit, and would
        # have hidden a lane that lost its provisioning while another gained a
        # duplicate.
        with_step = {name for name, job in self.jobs().items()
                     if any(step.get("name") == PROVISION_STEP
                            for step in job.get("steps", []))}
        self.assertTrue(with_step, "no lane provisions the oracle toolchain")
        self.assertEqual(with_step, set(self.lanes_needing_oracles()),
                         "provisioning step presence must follow oracle use")
        # No lane may keep a private apt transaction: that is the duplicate this
        # consolidation removed, and it would silently come back as a step.
        for name, bodies in self.lanes_needing_oracles().items():
            for body in bodies:
                self.assertNotIn("ci_install_gcc16.sh", body, name)
                self.assertNotIn("apt-get install -y --no-install-recommends mold", body, name)

    def test_the_provisioner_owns_the_installer_and_its_failure_status(self):
        """The step is a passthrough: `bash script` returns the script's status.

        Executed for real with a fake repo, so `set -e` inside the step body
        cannot silently turn a failed install into a green step.
        """
        script = ROOT / "scripts/ci_provision_toolchain.sh"
        with tempfile.TemporaryDirectory() as name:
            repo = Path(name)
            (repo / "scripts").mkdir()
            (repo / "bin").mkdir()
            (repo / "scripts/ci_provision_toolchain.sh").write_bytes(script.read_bytes())
            (repo / "scripts/ci_install_gcc16.sh").write_text("exit 9\n")
            (repo / "scripts/ci_toolchain_bundle.sh").write_text("exit 0\n")
            sudo = repo / "bin/sudo"
            sudo.write_text("#!/bin/sh\ntouch used-sudo\nexit 0\n")
            sudo.chmod(0o755)
            bundle = repo / "bundle"
            bundle.mkdir()
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", PROVISION_COMMAND.replace(
                    "bash scripts/ci_provision_toolchain.sh",
                    f"bash {repo}/scripts/ci_provision_toolchain.sh") + f" --bundle-dir {bundle}"],
                cwd=repo, capture_output=True, text=True, timeout=60,
                env=os.environ | {"PATH": f"{repo / 'bin'}:{os.environ['PATH']}"})
            self.assertEqual(result.returncode, 9, result.stdout + result.stderr)
            self.assertFalse((repo / "used-sudo").exists(),
                             "a failed GCC install must not reach the extras transaction")

    def test_fetch_failure_survives_long_resolver_diagnostics(self):
        lines = ['Err:4 https://ppa.example.invalid noble InRelease',
                 '  503 Service Unavailable',
                 'W: Failed to fetch repository index']
        lines += ['package policy detail'] * 90
        lines += ['E: Unable to locate package gcc-16']
        chosen = ci_annotate_log.select(lines, context=3, tail=40)
        self.assertIn(lines[0], chosen)
        self.assertIn(lines[1], chosen)
        self.assertIn(lines[-1], chosen)

    def test_warning_policy_text_is_not_a_fatal_verdict(self):
        lines = ["warning: unsupported option; make this an error"] * 1000
        lines += ["unit.c:7: error: actual root cause"]
        chosen = ci_annotate_log.select(lines, context=0, tail=40)
        self.assertEqual(chosen, [lines[-1]])

    def test_annotation_budget_preserves_first_cause_and_last_summary(self):
        with tempfile.TemporaryDirectory() as work:
            log = Path(work) / "kernel.log"
            text = "unit.c:1: error: ROOT_CAUSE_SENTINEL\n"
            text += "warning: unsupported option; make this an error\n" * 1000
            text += "other.c:2: error: " + "x" * 300 + "\n"
            text += ("other.c:3: error: " + "x" * 300 + "\n") * 100
            text += "make: Error 2 FINAL_SUMMARY_SENTINEL\n"
            log.write_text(text)
            proc = subprocess.run(
                ["python3", str(ROOT / "scripts/ci_annotate_log.py"),
                 "--max", "2", "--context", "0", str(log)],
                capture_output=True, text=True, check=True)
            self.assertEqual(proc.stdout.count("::error title="), 2)
            self.assertIn("ROOT_CAUSE_SENTINEL", proc.stdout)
            self.assertIn("FINAL_SUMMARY_SENTINEL", proc.stdout)

    def test_success_never_emits_failure_annotation(self):
        """A clean install prints no `::error`, so Checks stays quiet."""
        with tempfile.TemporaryDirectory() as name:
            repo = Path(name)
            (repo / "scripts").mkdir()
            (repo / "scripts/ci_install_gcc16.sh").write_text("echo 'GCC provisioning clean'\n")
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c",
                 f"bash {repo}/scripts/ci_install_gcc16.sh --multilib"],
                cwd=repo, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 0)
        self.assertNotIn("::error", result.stdout)

    def test_installer_annotations_survive_the_step_unchanged(self):
        """The `%25`-encoded annotation the installer writes reaches the log."""
        with tempfile.TemporaryDirectory() as name:
            repo = Path(name)
            (repo / "scripts").mkdir()
            (repo / "scripts/ci_install_gcc16.sh").write_text(
                'echo "::error title=GCC 16 provisioning failed (mode --multilib)::'
                'resolver %25 failure"; exit 7\n')
            result = subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c",
                 f"bash {repo}/scripts/ci_install_gcc16.sh --multilib"],
                cwd=repo, capture_output=True, text=True, timeout=60)
        self.assertEqual(result.returncode, 7)
        self.assertIn("::error title=GCC 16 provisioning failed (mode --multilib)::", result.stdout)
        self.assertIn("%25", result.stdout)


if __name__ == "__main__":
    unittest.main()
