"""A green compiler PR must not silently omit kernel validation."""
import fnmatch
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
