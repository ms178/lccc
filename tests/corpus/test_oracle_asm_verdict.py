"""Offline fail-closed contracts for the Compiler Explorer assembly dump."""
import io
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import codegen_oracle as CG


class AssemblyDumpVerdict(unittest.TestCase):
    def run_dump(self, local_error=False, remote_error=False):
        from unittest.mock import patch
        import oracle_asm
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "probe.c"
            source.write_text("int main(void) { return 0; }\n")
            def remote(name, *_):
                if remote_error and name == "clang23.1":
                    raise RuntimeError("injected remote failure")
                return ["main:", "    ret"]
            with patch.object(CG, "_local_compile", side_effect=RuntimeError("injected local failure") if local_error else None, return_value=["main:", "    ret"]), patch.object(CG, "_compile_remote", side_effect=remote), redirect_stdout(io.StringIO()):
                return oracle_asm.main([str(source), "--oracles", "gcc16.2,clang23.1,icx"])

    def test_local_failure_fails_dump(self):
        self.assertNotEqual(self.run_dump(local_error=True), 0)

    def test_requested_vendor_failure_fails_dump(self):
        self.assertNotEqual(self.run_dump(remote_error=True), 0)

    def test_complete_dump_succeeds(self):
        self.assertEqual(self.run_dump(), 0)


if __name__ == "__main__":
    unittest.main()
