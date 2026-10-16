#!/usr/bin/env python3
"""Compiler-free red-team checks for the shared-binding verdict oracle."""
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import check_riscv_shared_binding as gate
from check_riscv_relocation_contract import fixture, sections


class SharedBindingTests(unittest.TestCase):
    def image(self, *, info=(1 << 32) | 2, addend=-7, visibility=0, absolute=False):
        header = lambda address: (0, 1, 3, address, 0, 16, 0, 0, 8, 0)
        return {
            ".text": (header(0x1000), bytes(16)),
            ".data": (header(0x2000), struct.pack("<QQ", 0, 0)),
            ".dynsym": (header(0), bytes(24) + struct.pack("<IBBHQQ", 1, 0x10,
                visibility, 0xfff1 if absolute else 1, 0x1000, 0)),
            ".dynstr": (header(0), b"\0target\0"),
            ".rela.dyn": (header(0), struct.pack("<QQq", 0x2000, info, addend)),
        }

    def verify(self, image, **overrides):
        options = dict(binding=1, visibility=0, defined=True, absolute=False,
                       target_value=0, kind=2, addend=-7)
        options.update(overrides)
        with patch.object(gate, "sections", return_value=image):
            gate.verify(Path("unused"), options)

    def test_protected_symbolic_relocation_is_bound_locally(self):
        self.verify(self.image(visibility=3), visibility=3)

    def test_protected_absolute_symbolic_relocation_has_no_load_bias(self):
        self.verify(self.image(visibility=3, absolute=True), visibility=3,
                    absolute=True, target_value=0x1000)

    def test_absolute_relative_relocation_is_not_equivalent(self):
        with self.assertRaises(AssertionError):
            self.verify(self.image(info=3, visibility=3, absolute=True),
                        visibility=3, absolute=True, target_value=0x1000)

    def test_protected_symbolic_wrong_addend_is_rejected(self):
        with self.assertRaises(AssertionError):
            self.verify(self.image(visibility=3, addend=11), visibility=3)

    def test_symbolic_relocation_is_accepted(self):
        self.verify(self.image())

    def test_relative_is_not_a_symbolic_relocation(self):
        with self.assertRaises(AssertionError):
            self.verify(self.image(info=3))

    def test_wrong_tls_type_is_not_an_address_relocation(self):
        with self.assertRaises(AssertionError):
            self.verify(self.image(info=(1 << 32) | 6))

    def test_addend_and_symbol_index_are_checked(self):
        for image in (self.image(addend=0), self.image(info=(2 << 32) | 2)):
            with self.subTest(image=image), self.assertRaises(AssertionError):
                self.verify(image)

    def test_hidden_export_and_protected_metadata_are_checked(self):
        for visibility in (1, 2, 3):
            with self.subTest(visibility=visibility), self.assertRaises(AssertionError):
                self.verify(self.image(), visibility=visibility)

    def test_absolute_binding_is_not_base_adjusted(self):
        image = self.image(info=3, addend=249)
        image[".dynsym"] = (image[".dynsym"][0], bytes(24))
        with self.assertRaises(AssertionError):
            self.verify(image, binding=0, absolute=True, target_value=256)
        image[".rela.dyn"] = (image[".rela.dyn"][0], b"")
        image[".data"] = (image[".data"][0], struct.pack("<QQ", 249, 0))
        self.verify(image, binding=0, absolute=True, target_value=256)

    def test_rv64_shared_32_requires_absolute(self):
        rows = [(o, accept) for _, o, accept in gate.cases() if o["kind"] == 1]
        self.assertTrue(rows)
        self.assertTrue(any(accept for _, accept in rows))
        self.assertTrue(any(not accept for _, accept in rows))
        for options, accept in rows:
            self.assertEqual(accept, options["defined"] and options["absolute"])

    def test_absolute_32_checks_four_bytes_and_neighbor(self):
        image = self.image(absolute=True)
        image[".dynsym"] = (image[".dynsym"][0], bytes(24))
        image[".rela.dyn"] = (image[".rela.dyn"][0], b"")
        correct = struct.pack("<I", 249) + b"\xa5" * 12
        image[".data"] = (image[".data"][0], correct)
        self.verify(image, binding=0, absolute=True, target_value=256, kind=1)
        for corrupt in (struct.pack("<I", 248) + correct[4:], correct[:4] + bytes(4) + correct[8:]):
            image[".data"] = (image[".data"][0], corrupt)
            with self.assertRaises(AssertionError):
                self.verify(image, binding=0, absolute=True, target_value=256, kind=1)

    def test_reserved_got_header_is_not_the_requested_slot(self):
        image = self.image(addend=0)
        header = list(image[".data"][0]); header[3] = 0x3000
        image[".got"] = (tuple(header), struct.pack("<QQ", 0x4000, 0))
        image[".rela.dyn"] = (header, struct.pack("<QQq", 0x3008, (1 << 32) | 2, 0))
        image[".data"] = (image[".data"][0], struct.pack("<I", 1 << 12) + bytes(12))
        self.verify(image, kind=20, addend=0)
        image[".data"] = (image[".data"][0], bytes(16))
        with self.assertRaises(AssertionError):
            self.verify(image, kind=20, addend=0)

    def test_rebased_fixture_preserves_both_parameter_families(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "case.o"
            for kwargs, section, value in (({}, 1, 0),
                    (dict(absolute=True, target_value=256), 0xfff1, 256),
                    (dict(displacement=8, offset=4), 2, 12)):
                path.write_bytes(fixture(**kwargs))
                sym = struct.unpack_from("<IBBHQQ", sections(path)[".symtab"][1], 24)
                self.assertEqual((sym[3], sym[4]), (section, value))

    def test_missing_tool_is_structured_failure(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            report = root / "report.json"
            p = subprocess.run([sys.executable, gate.__file__, "--lccc", str(root / "absent"),
                                "--json", str(report)], capture_output=True, text=True, timeout=20)
            self.assertNotEqual(p.returncode, 0)
            result = json.loads(report.read_text())
            self.assertEqual((result["passed"], result["failed"]), (0, 1))
            self.assertEqual(result["records"][0]["case"], "infrastructure")

    def test_optimized_python_cannot_disable_the_verdict(self):
        p = subprocess.run([sys.executable, "-O", gate.__file__, "--lccc", "absent"],
                           capture_output=True, text=True, timeout=20)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("assertions are required", p.stderr)


if __name__ == "__main__":
    unittest.main()
