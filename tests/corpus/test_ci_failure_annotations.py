"""Offline contract tests for user-visible GitHub Actions failure annotations.

Provisioning failures are diagnosed through check-run annotations because the
Actions log download endpoint is not reachable from every API client (it was
unreachable from the environment that triaged the 2026-10-21 regression). The
provisioning helper therefore publishes its own bounded `::error` annotation
and every CI lane invokes it directly; this suite pins both halves of that
contract without touching the network, apt, or root.
"""

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
PROVISION = ROOT / "scripts/ci_install_gcc16.sh"

STUB = r'''
import os, pathlib, sys

name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['STUB_LOG'], 'a') as handle:
    handle.write(name + ' ' + ' '.join(args) + '\n')
if name == 'sudo':
    if args and args[0] == '--':
        args.pop(0)
    os.execvp(args[0], args)
elif name == 'uname':
    print('Linux')
elif name == 'dpkg-query':
    print('not-installed')
elif name == 'apt-cache':
    pass  # nothing is resolvable: the index is unusable
elif name == 'apt-get':
    if args[:1] == ['update']:
        print('E: Failed to fetch http://mirror.example/InRelease  100% unusable')
        sys.exit(100)
    print('E: Unable to locate package %s' % ' '.join(
        a for a in args if a.startswith(('gcc', 'g++', 'lib'))))
    sys.exit(100)
elif name == 'add-apt-repository':
    print('Adding repository.')
elif name in ('gcc-16', 'g++-16'):
    if args == ['-dumpfullversion']:
        print('13.2.0')
    else:
        print(name + ' (stub) 13.2.0')
'''


def _linker_runner_module():
    spec = importlib.util.spec_from_file_location("lccc_linker_test_runner", LINKER_RUNNER)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"could not load {LINKER_RUNNER}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FailureAnnotationContracts(unittest.TestCase):
    def run_provisioning(self, actions=True):
        directory = Path(tempfile.mkdtemp(prefix="lccc-ci-annotation-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(directory)], check=False))
        stub = directory / "stub"
        stub.write_text(f"#!{__import__('sys').executable}\n" + STUB)
        stub.chmod(0o755)
        for command in ("sudo", "uname", "dpkg-query", "dpkg", "apt-get",
                        "apt-cache", "add-apt-repository", "gcc-16", "g++-16"):
            (directory / command).symlink_to(stub)
        release = directory / "os-release"
        release.write_text("ID=ubuntu\n")
        env = {**os.environ,
               "PATH": f"{directory}:{os.defpath}",
               "STUB_LOG": str(directory / "log"),
               "LCCC_CI_OS_RELEASE": str(release),
               "LCCC_CI_APT_RETRY_DELAY": "0"}
        env.pop("GITHUB_ACTIONS", None)
        if actions:
            env["GITHUB_ACTIONS"] = "true"
        return subprocess.run(["bash", str(PROVISION), "--multilib"],
                              env=env, text=True, capture_output=True,
                              check=False, timeout=60)

    def test_annotation_data_escapes_github_command_metacharacters(self):
        module = _linker_runner_module()
        self.assertEqual(
            module._escape_github_annotation_data("a%\rb\nc"),
            "a%25%0Db%0Ac",
        )

    def test_provisioning_failure_is_annotated_and_status_is_preserved(self):
        result = self.run_provisioning()
        self.assertNotEqual(result.returncode, 0)
        annotations = [line for line in result.stdout.splitlines()
                       if line.startswith("::error ")]
        self.assertEqual(len(annotations), 1, result.stdout + result.stderr)
        annotation = annotations[0]
        self.assertIn("title=GCC 16 provisioning failed (mode --multilib)", annotation)
        self.assertIn("gcc-16", annotation)
        self.assertIn("lib32gcc-16-dev", annotation)
        # GitHub command data must be single-line and percent-escaped.
        self.assertIn("%25", annotation)
        self.assertIn("%0A", annotation)
        self.assertNotIn("\r", annotation)
        # A downgrade that never happened must never be announced.
        self.assertNotIn("degraded", result.stdout)

    def test_no_annotation_outside_github_actions(self):
        result = self.run_provisioning(actions=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("::error", result.stdout)
        self.assertIn("not resolvable from the configured repositories", result.stderr)

    def test_every_lane_invokes_the_helper_directly(self):
        # A lane that pipes the helper through its own capture wrapper would
        # swallow the annotation; the helper is the single source of truth.
        # The helper is ci_provision_toolchain.sh, which owns the whole oracle
        # set (GCC 16, i686 multilib, qemu, mold, lld, python3-yaml) and calls
        # ci_install_gcc16.sh internally on a bundle miss -- so no lane names
        # the installer directly any more, and a lane that did would be
        # provisioning around the shared cache entry.
        workflow = yaml.load(WORKFLOW.read_text(), Loader=yaml.BaseLoader)
        lanes = []
        for job, definition in workflow["jobs"].items():
            for step in definition.get("steps", []):
                script = step.get("run") or ""
                if "ci_provision_toolchain.sh" in script:
                    lanes.append(job)
                    self.assertIn("bash scripts/ci_provision_toolchain.sh --multilib",
                                  script,
                                  f"{job}: unexpected provisioning invocation")
                    self.assertNotIn("ci_install_gcc16.sh", script,
                                     f"{job}: bypasses the shared toolchain bundle")
                    self.assertNotIn("install_log", script,
                                     f"{job}: capture wrapper duplicated")
                    self.assertNotIn("::error", script,
                                     f"{job}: annotation duplicated")
        self.assertEqual(sorted(lanes), sorted([
            "build", "debug", "corpus", "frontend", "codegen", "backend",
            "backend-contracts-a", "backend-contracts-b", "unit"]))


if __name__ == "__main__":
    unittest.main()
