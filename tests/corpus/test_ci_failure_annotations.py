"""Offline contract tests for user-visible GitHub Actions failure annotations."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/ci.yml"
LINKER_RUNNER = ROOT / "tests/linker/run_linker_tests.py"


def _linker_runner_module():
    spec = importlib.util.spec_from_file_location("lccc_linker_test_runner", LINKER_RUNNER)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {LINKER_RUNNER}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def _gcc16_provision_script():
    workflow = yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)
    step = next(
        step
        for step in workflow["jobs"]["frontend"]["steps"]
        if step.get("name") == "Install GCC 16 and i686 C runtime/development files"
    )
    return step["run"]


class FailureAnnotationContracts(unittest.TestCase):
    def _run_mocked_provisioning(self, script, install_command, qemu_command):
        original_install = 'bash scripts/ci_install_gcc16.sh --multilib >"$install_log" 2>&1'
        original_qemu = (
            "sudo apt-get install -y --no-install-recommends qemu-user "
            '>>"$install_log" 2>&1'
        )
        self.assertEqual(script.count(original_install), 1)
        self.assertEqual(script.count(original_qemu), 1)
        script = script.replace(original_install, install_command, 1)
        script = script.replace(original_qemu, qemu_command, 1)

        with tempfile.TemporaryDirectory(prefix="lccc-ci-annotation-") as runner_temp:
            env = os.environ.copy()
            env["RUNNER_TEMP"] = runner_temp
            return subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", script],
                cwd=ROOT,
                env=env,
                text=True,
                capture_output=True,
                check=False,
                timeout=15,
            )

    def test_annotation_data_escapes_github_command_metacharacters(self):
        module = _linker_runner_module()
        self.assertEqual(
            module._escape_github_annotation_data("a%\rb\nc"),
            "a%25%0Db%0Ac",
        )

    def test_gcc16_failure_is_annotated_and_original_status_is_preserved(self):
        script = _gcc16_provision_script()
        install = r'''bash -c 'printf "synthetic 100%% failure\r\nsecond line\n"; exit 23' >"$install_log" 2>&1'''
        qemu = r'''bash -c 'printf "synthetic qemu provisioning success\n"' >>"$install_log" 2>&1'''
        result = self._run_mocked_provisioning(script, install, qemu)

        self.assertEqual(result.returncode, 23, result.stdout + result.stderr)
        self.assertIn("GCC 16 / i686 provisioning failed", result.stdout)
        self.assertIn("synthetic 100%25 failure%0Asecond line", result.stdout)
        self.assertNotIn("synthetic qemu provisioning success", result.stdout)

    def test_successful_gcc16_provisioning_emits_no_error_annotation(self):
        script = _gcc16_provision_script()
        install = r'''bash -c 'printf "synthetic GCC provisioning success\n"' >"$install_log" 2>&1'''
        qemu = r'''bash -c 'printf "synthetic qemu provisioning success\n"' >>"$install_log" 2>&1'''
        result = self._run_mocked_provisioning(script, install, qemu)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("synthetic GCC provisioning success", result.stdout)
        self.assertIn("synthetic qemu provisioning success", result.stdout)
        self.assertNotIn("::error ", result.stdout)


if __name__ == "__main__":
    unittest.main()
