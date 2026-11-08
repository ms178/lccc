"""A green compiler PR must not silently omit kernel validation."""
import fnmatch
import re
from pathlib import Path
import unittest

import yaml

ROOT = Path(__file__).resolve().parents[2]


class KernelWorkflowPaths(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        workflow = yaml.safe_load((ROOT / ".github/workflows/kernel-61858.yml").read_text())
        # PyYAML's YAML1.1 resolver reads the unquoted Actions `on` key as True.
        events = workflow.get("on", workflow.get(True))
        cls.patterns = events["pull_request"]["paths"]

    def selected(self, path):
        # The pinned patterns are positive file/prefix globs, not exclusions.
        # For these patterns fnmatch's slash matching agrees with Actions **.
        return any(fnmatch.fnmatchcase(path, pattern) for pattern in self.patterns)

    def test_compiler_and_linker_changes_select_kernel_validation(self):
        for path in ["src/frontend/sema/analysis.rs", "src/backend/x86/codegen/intrinsics.rs",
                     "src/backend/arm/codegen/calls.rs", "src/backend/riscv/codegen/calls.rs",
                     "src/backend/i686/codegen/prologue.rs", "src/bin/lccc_ld.rs",
                     "src/passes/vectorize.rs", "src/common/types.rs", "src/driver/cli.rs",
                     "src/ir/lowering/expr_calls.rs", "src/linker_entry.rs"]:
            with self.subTest(path=path):
                self.assertTrue(self.selected(path), path)

    def test_compiler_build_inputs_select_kernel_validation(self):
        for path in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml",
                     "scripts/build_lccc_fast.sh", "scripts/rust_toolchain.sh",
                     "scripts/ensure_swap.sh", "scripts/ci_provision_toolchain.sh",
                     "engineering/toolchains/agent-cache-lock.json"]:
            with self.subTest(path=path):
                self.assertTrue(self.selected(path), path)

    def test_boot_harness_dependencies_select_kernel_validation(self):
        for path in ["scripts/boot_flags.sh", "scripts/boot_offsets.sh",
                     "scripts/kernel_version.sh", "scripts/kernel_tool_identity.sh",
                     "scripts/kconfig_query.sh", "scripts/build_kernel_full.sh",
                     "scripts/build_kernel_boot.sh", "scripts/prepare_kernel_tree.sh",
                     "scripts/qemu_defconfig_smoke.sh", "scripts/qemu_firmware.sh"]:
            with self.subTest(path=path):
                self.assertTrue(self.selected(path), path)

    def test_docs_only_do_not_force_a_full_kernel_build(self):
        for path in ["README.md", "docs/KERNEL-PATTERN-REVIEW.md",
                     "engineering/KERNEL-DIAGNOSTICS-SUCCESSOR-2026-11-03.md"]:
            with self.subTest(path=path):
                self.assertFalse(self.selected(path), path)

    def test_filter_contract_itself_is_covered(self):
        self.assertTrue(self.selected("tests/corpus/test_kernel_workflow_paths.py"))

    def test_no_negative_glob_can_silently_exclude_compiler_paths(self):
        self.assertTrue(all(not pattern.startswith("!") for pattern in self.patterns))


class KernelJobIsOnDemandOrWeekly(unittest.TestCase):
    """The 240-minute kernel gate must not be a default per-PR gate.

    A normal compiler PR still triggers this workflow (so the skipped status
    is visible), but `jobs.kernel` must be gated to a weekly schedule, a
    manual dispatch, or the explicit `kernel-full` label. Keeping the path
    filter intact is what makes the omission visible rather than silent.
    """
    @classmethod
    def setUpClass(cls):
        import yaml
        workflow = yaml.safe_load(
            (ROOT / ".github/workflows/kernel-61858.yml").read_text())
        cls.events = workflow.get("on", workflow.get(True))
        cls.job_if = workflow["jobs"]["kernel"]["if"]

    def test_weekly_schedule_exists(self):
        crons = self.events["schedule"]
        self.assertTrue(crons, "a weekly cron must be declared")
        self.assertTrue(all("cron" in item for item in crons))
        self.assertEqual(crons[0]["cron"], "17 3 * * 1")

    def test_manual_dispatch_can_run_kernel(self):
        inputs = self.events["workflow_dispatch"]["inputs"]
        self.assertIn("run_kernel", inputs)
        self.assertIn(inputs["run_kernel"]["default"], (True, "true"))
        self.assertIn("inputs.run_kernel", self.job_if)

    def test_kernel_job_requires_an_explicit_opt_in(self):
        self.assertIn("github.event_name == 'schedule'", self.job_if)
        self.assertIn("'kernel-full'", self.job_if)
        self.assertIn("contains(github.event.pull_request.labels.*.name", self.job_if)
        # A bare PR event must never be enough to start the 240-minute job.
        self.assertIn("github.event_name == 'pull_request' &&", self.job_if)

    def test_label_event_re_evaluates_the_workflow(self):
        self.assertIn("labeled", self.events["pull_request"]["types"])

    def test_path_filter_still_selects_compiler_changes(self):
        # Regression guard for the visible-skip property: a compiler-only PR
        # must still trigger the workflow, otherwise the skip would be silent.
        self.assertTrue(self.events["pull_request"]["paths"])


class KernelVersionPinsAgree(unittest.TestCase):
    """The three places the pinned kernel release is spelled must agree.

    `scripts/kernel_version.sh` exports the shell fallback, `scripts/
    lccc_paths.py` the Python one, and the hermetic kernel workflow pins the
    release it actually builds. The two fallbacks name the SAME tree — the
    Python one documents itself as mirroring `LCCC_KERNEL_FALLBACK_KVER`, and
    every boot/bisect harness resolves a tree through one or the other — so a
    divergence silently points half the tooling at a directory that either
    does not exist or, worse, holds a DIFFERENT release's sources.
    """

    @classmethod
    def setUpClass(cls):
        cls.sh = (ROOT / "scripts/kernel_version.sh").read_text()
        cls.py = (ROOT / "scripts/lccc_paths.py").read_text()
        cls.workflow = (
            ROOT / ".github/workflows/kernel-61858.yml"
        ).read_text()

    def _sh_fallback(self):
        match = re.search(r"^LCCC_KERNEL_FALLBACK_KVER=([0-9.]+)$", self.sh, re.M)
        self.assertIsNotNone(match, "the shell fallback pin must exist")
        return match.group(1)

    def _py_fallback(self):
        match = re.search(r'^\s+return "([0-9.]+)"', self.py, re.M)
        self.assertIsNotNone(match, "the Python fallback pin must exist")
        return match.group(1)

    def test_shell_and_python_fallbacks_are_identical(self):
        self.assertEqual(
            self._sh_fallback(),
            self._py_fallback(),
            "the two fallback pins must name the same kernel tree",
        )

    def test_python_documents_the_shell_fallback_as_its_source(self):
        # A future edit must not be able to change one pin and leave the
        # other's documentation claiming it mirrors it.
        self.assertIn("LCCC_KERNEL_FALLBACK_KVER", self.py)

    def test_fallback_matches_the_release_the_workflow_pins(self):
        match = re.search(r"^\s+KVER:\s*([0-9.]+)$", self.workflow, re.M)
        self.assertIsNotNone(match, "the workflow must pin KVER explicitly")
        self.assertEqual(
            self._sh_fallback(),
            match.group(1),
            "the fallback only names a directory; it must not drift from the "
            "release the kernel lane actually builds",
        )
