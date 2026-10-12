import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("switch_shape", Path(__file__).resolve().parents[2] / "scripts/check_switch_bound_shape.py")
shape = importlib.util.module_from_spec(spec)
spec.loader.exec_module(shape)


def asm(guard=""):
    return "dispatch16:\n.cfi_startproc\n# cmpq $16, %rax\n" + guard + "  jmp *%rax\n.cfi_endproc\n"


class SwitchShape(unittest.TestCase):
    def test_width_and_immediate_spelling_do_not_define_the_contract(self):
        for op, imm in (("cmpl", "0x10"), ("cmpq", "16")):
            shape.check(asm(), asm(f"{op} ${imm}, %eax\n jae .Ldefault\n"))

    def test_missing_dispatch_or_ineffective_controls_fail(self):
        for a, b in ((asm(), asm()), (asm("cmpq $16,%rax\njae .Ld\n"), asm()), ("", asm())):
            with self.subTest(a=a, b=b), self.assertRaises(ValueError):
                shape.check(a, b)
