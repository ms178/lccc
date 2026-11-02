"""Whole-object identity may rename one symbol, never instructions or relocations."""
from pathlib import Path
import struct
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from benchmark_elf_identity import object_identity, rename_entry


def fixture(entry='main', code=b'\x31\xc0\xc3', relocation=2, bindtype=0x12, payload=b'x'):
    names = b'\0.text\0.strtab\0.symtab\0.shstrtab\0.rela.text\0.rodata\0'
    symbols = bytes(24) + struct.pack('<IBBHQQ', 1, bindtype, 0, 1, 0, len(code))
    sections = [('', 0, 0, b'', 0, 0, 0, 0),
                ('.text', 1, 6, code, 0, 0, 16, 0),
                ('.strtab', 3, 0, b'\0' + entry.encode() + b'\0', 0, 0, 1, 0),
                ('.symtab', 2, 0, symbols, 2, 1, 8, 24),
                ('.shstrtab', 3, 0, names, 0, 0, 1, 0),
                ('.rela.text', 4, 64, struct.pack('<QQq', 0, (1 << 32) | relocation, 0), 3, 1, 8, 24),
                ('.rodata', 1, 2, payload, 0, 0, 1, 0)]
    data = bytearray(64)
    headers = []
    for name, kind, flags, payload, link, info, align, entsize in sections:
        if align:
            data.extend(bytes((-len(data)) % align))
        offset = len(data) if kind else 0
        data.extend(payload)
        headers.append(struct.pack('<IIQQQQIIQQ', names.index(name.encode() + b'\0') if name else 0,
                                   kind, flags, 0, offset, len(payload), link, info, align, entsize))
    data.extend(bytes((-len(data)) % 8))
    shoff = len(data)
    data.extend(b''.join(headers))
    data[:16] = b'\x7fELF\x02\x01\x01' + bytes(9)
    struct.pack_into('<HHIQQQIHHHHHH', data, 16, 1, 62, 1, 0, 0, shoff, 0, 64, 0, 0, 64, len(sections), 4)
    return bytes(data)


class ObjectIdentityTests(unittest.TestCase):
    def test_exact_rename(self):
        original=fixture()
        renamed=rename_entry(original)
        self.assertEqual(object_identity(original,'main'), object_identity(renamed,'benchmark_entry'))
        self.assertEqual(original[64:67],renamed[64:67])

    def test_invalid_new_name(self):
        for name in ('', 'bad\0name'):
            with self.subTest(name=name), self.assertRaises(ValueError):
                rename_entry(fixture(), new=name)

    def test_name_only(self):
        self.assertEqual(object_identity(fixture(), 'main'),
                         object_identity(fixture('benchmark_entry'), 'benchmark_entry'))

    def test_changed_instruction(self):
        self.assertNotEqual(object_identity(fixture(), 'main'),
                            object_identity(fixture(code=b'\x31\xc0\x90'), 'main'))

    def test_changed_relocation(self):
        self.assertNotEqual(object_identity(fixture(), 'main'),
                            object_identity(fixture(relocation=4), 'main'))

    def test_invalid_entry_binding(self):
        for kind in (0x02, 0x10, 0x11, 0x22):
            with self.subTest(kind=kind), self.assertRaises(ValueError):
                object_identity(fixture(bindtype=kind), 'main')

    def test_missing_entry(self):
        with self.assertRaises(ValueError):
            object_identity(fixture(), 'missing')

    def test_truncation(self):
        for size in (0, 7, 63, 100, len(fixture()) - 1):
            with self.subTest(size=size), self.assertRaises(ValueError):
                object_identity(fixture()[:size], 'main')

    def test_wrong_architecture(self):
        data = bytearray(fixture())
        struct.pack_into('<H', data, 18, 183)
        with self.assertRaises(ValueError):
            object_identity(data, 'main')

    def test_changed_data(self):
        self.assertNotEqual(object_identity(fixture(), 'main'),
                            object_identity(fixture(payload=b'y'), 'main'))

    def test_allocated_string_table_rejected(self):
        data = bytearray(fixture())
        shoff, = struct.unpack_from('<Q', data, 40)
        struct.pack_into('<Q', data, shoff + 2 * 64 + 8, 2)
        with self.assertRaises(ValueError):
            object_identity(data, 'main')

if __name__ == '__main__':
    unittest.main()
