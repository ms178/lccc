"""The oracle must refuse to measure a compiler channel of the wrong target.

Regression cover for a silent mislabelling: `--arch riscv64 --oracles gcc16.2`
(and the AArch64 equivalent) used to run the *x86-64* channel and store its
assembly under the requested architecture.  Per-function instruction counts
then read identically across architectures -- the giveaway -- and a survey
would have reported x86 code as RISC-V evidence.

No network: the guard runs on compiler metadata, which is what
`godbolt.compiler_metadata` returns in every report mode.
"""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "scripts"))

import codegen_oracle as oracle  # noqa: E402
import godbolt  # noqa: E402

X86_GCC = {"id": "cg162", "name": "x86-64 gcc 16.2"}
X86_CLANG = {"id": "cclang1910", "name": "x86-64 clang 19.1.0"}
AARCH64_GCC = {"id": "carm64g1610", "name": "aarch64 gcc 16.1"}
RISCV_GCC = {"id": "crv64g1610", "name": "riscv64 gcc 16.1"}


class ArchOfCompilerTest(unittest.TestCase):
    def test_identifies_the_channels_in_use(self) -> None:
        self.assertEqual(oracle._arch_of_compiler(X86_GCC), "x86")
        self.assertEqual(oracle._arch_of_compiler(AARCH64_GCC), "aarch64")
        self.assertEqual(oracle._arch_of_compiler(RISCV_GCC), "riscv64")

    def test_unknown_names_stay_unknown(self) -> None:
        self.assertIsNone(oracle._arch_of_compiler({"id": "mystery", "name": "Mystery CC"}))


class ArchGuardTest(unittest.TestCase):
    def test_x86_channel_is_refused_for_riscv64(self) -> None:
        with self.assertRaises(godbolt.GodboltError) as raised:
            oracle._check_arch("gcc16.2", X86_GCC, "-O3 -march=x86-64-v3", "riscv64")
        message = str(raised.exception)
        self.assertIn("x86", message)
        self.assertIn("rv64gcc", message, "the error names a usable channel")

    def test_x86_channel_is_refused_for_aarch64(self) -> None:
        with self.assertRaises(godbolt.GodboltError):
            oracle._check_arch("gcc16.2", X86_GCC, "-O3", "aarch64")

    def test_matching_channels_pass(self) -> None:
        oracle._check_arch("gcc16.2", X86_GCC, "-O3 -march=x86-64-v3", "x86")
        oracle._check_arch("carm64g1610", AARCH64_GCC, "-O3", "aarch64")
        oracle._check_arch("crv64g1610", RISCV_GCC, "-O3", "riscv64")

    def test_explicit_target_flag_is_a_deliberate_cross_target(self) -> None:
        # The AArch64 Clang channel is hosted as x86-64; `_record_remote` adds
        # `--target=aarch64-linux-gnu` itself.  That must keep working.
        oracle._check_arch("clang", X86_CLANG, "-O3 --target=aarch64-linux-gnu", "aarch64")
        oracle._check_arch("clang", X86_CLANG, "-O3 -target riscv64-unknown-elf", "riscv64")

    def test_march_is_not_a_target_selection(self) -> None:
        # `-march=x86-64-v3` names a *micro*architecture; it must not be taken
        # as permission to run an x86 channel under --arch aarch64.
        self.assertFalse(oracle._flags_select_arch("-O3 -march=x86-64-v3", "aarch64"))
        with self.assertRaises(godbolt.GodboltError):
            oracle._check_arch("gcc16.2", X86_GCC, "-O3 -march=x86-64-v3", "aarch64")

    def test_riscv_channels_are_recognised(self) -> None:
        # The native channel names carry "RISC-V"/"rv64gc", not "riscv64".
        self.assertEqual(
            oracle._arch_of_compiler({"id": "rv64-cgcc1610", "name": "RISC-V (64-bits) gcc 16.1.0"}),
            "riscv64",
        )
        self.assertEqual(
            oracle._arch_of_compiler({"id": "armv8-cclang2310", "name": "armv8-a clang 23.1.0"}),
            "aarch64",
        )

    def test_arch_specific_default_flags(self) -> None:
        # Handing `-march=x86-64-v3` to the AArch64/RISC-V channels made every
        # cross-architecture oracle fail to compile.
        self.assertIn("x86-64", oracle.default_flags("x86"))
        self.assertIn("armv8-a", oracle.default_flags("aarch64"))
        self.assertIn("rv64gc", oracle.default_flags("riscv64"))
        self.assertEqual(oracle.default_flags("x86", rank=True), oracle.RANK_DEFAULT_FLAGS)
        self.assertEqual(oracle.default_flags("riscv64", rank=True), oracle.default_flags("riscv64"))

    def test_per_arch_oracle_sets_name_their_architecture(self) -> None:
        # Canary for a dead/foreign channel in the default sets: each alias must
        # at least read as the architecture it is used for (`godbolt.py audit`
        # checks that it exists and is current, which needs the network).
        for alias, _ in zip(oracle.DEFAULT_AARCH64_ORACLES, oracle.DEFAULT_AARCH64_ORACLES):
            self.assertEqual(
                oracle._arch_of_compiler({"id": alias, "name": alias}), "aarch64",
                f"{alias} in DEFAULT_AARCH64_ORACLES",
            )
        for alias in oracle.DEFAULT_RISCV64_ORACLES:
            self.assertEqual(
                oracle._arch_of_compiler({"id": alias, "name": alias}), "riscv64",
                f"{alias} in DEFAULT_RISCV64_ORACLES",
            )

    def test_unidentifiable_channel_is_not_blocked(self) -> None:
        # A channel whose name carries no architecture is a blind spot, not
        # proof of a mismatch; the guard only fires on positive evidence.
        oracle._check_arch("mystery", {"id": "mystery", "name": "Mystery CC"}, "-O3", "x86")


if __name__ == "__main__":
    unittest.main()
