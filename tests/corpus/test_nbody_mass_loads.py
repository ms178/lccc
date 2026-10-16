"""Mutation controls for the actual nbody semantic mass-read census."""
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import nbody_mass_loads as census

ASM = '''.size bodies, 280
main:
 movq %rsp, %rbp
 leaq bodies(%rip), %r11
 leaq 56(%r11), %rsi
 leaq 48(%r11), %r13
.loop:
 movsd 48(%rsi), %xmm1
 movsd (%r13), %xmm2
 movupd %xmm5, 24(%r11)
 movupd %xmm6, 24(%rsi)
 leaq 56(%rsi), %rsi
 cmpq %rax, %rsi
 jne .loop
 ret
.size main, .-main
'''


class MassReadCensus(unittest.TestCase):
    def rejected(self, text):
        with self.assertRaises(ValueError):
            census.census(text)

    def test_duplicate_loads_observe_high_lanes_not_the_holes(self):
        for reg, width in [('xmm7', 8), ('ymm7', 32), ('zmm7', 64)]:
            self.assertEqual(census.width('vmovddup', ['0(%rsi)', '%' + reg]), width)
        self.assertEqual(list(census.read_offsets('vmovddup', ['0(%rsi)', '%ymm7'])),
                         list(range(8)) + list(range(16, 24)))
        self.rejected(ASM.replace('movupd %xmm5', 'vmovddup 32(%rsi), %ymm7\n movupd %xmm5'))

    def test_unary_imul_retires_both_implicit_address_results(self):
        for accumulator in ['rax', 'rdx']:
            extra = f'movq %r11, %{accumulator}\n addq $8, %{accumulator}\n imulq %r15\n movsd (%{accumulator}), %xmm7\n '
            self.rejected(ASM.replace('movupd %xmm5', extra + 'movupd %xmm5'))

    def test_displacement_and_hoisted_pointer_are_the_same_field(self):
        row = census.census(ASM)
        self.assertEqual(row['mass_reads'], 2)
        copied = ASM.replace('.loop:', 'movq %r13, %r14\n.loop:').replace('(%r13)', '(%r14)')
        self.assertEqual(census.census(copied)['mass_reads'], 2)

    def test_folded_operand_still_counts(self):
        text = ASM.replace('movsd 48(%rsi), %xmm1', 'vmulsd 48(%rsi), %xmm1, %xmm1')
        self.assertEqual(census.census(text)['mass_reads'], 2)

    def test_hoisted_and_folded_duplicates_fail(self):
        for line in ('movsd (%r13), %xmm7', 'vmulsd (%r13), %xmm7, %xmm7',
                     'movq (%r13), %rax'):
            with self.subTest(line=line):
                self.rejected(ASM.replace('movupd %xmm5', line + '\n movupd %xmm5'))

    def test_stores_are_not_reads_and_comments_are_not_instructions(self):
        text = ASM.replace('movupd %xmm5', 'movsd %xmm7, 48(%rsi)\n # movsd (%r13), %xmm7\n movupd %xmm5')
        self.assertEqual(census.census(text)['mass_reads'], 2)

    def test_partial_register_clobber_cannot_preserve_a_pointer(self):
        self.rejected(ASM.replace('movsd (%r13)', 'movl $0, %r13d\n movsd (%r13)'))
        self.rejected(ASM.replace('movsd (%r13)', 'movb $0, %r13b\n movsd (%r13)'))

    def test_conflicting_predecessors_do_not_guess_a_field(self):
        text = ASM.replace('.loop:', '''testq %rax, %rax
 je .other
 jmp .join
.other:
 leaq 40(%r11), %r13
.join:
.loop:''')
        self.rejected(text)

    def test_fixed_frame_pointer_spill_can_be_proved(self):
        text = ASM.replace('.loop:', 'movq %r13, -8(%rbp)\n.loop:')
        text = text.replace('movsd (%r13)', 'movq -8(%rbp), %rax\n movsd (%rax)')
        self.assertEqual(census.census(text)['mass_reads'], 2)
        for store in ('movdqu %xmm7, -16(%rbp)', 'movl $0, -4(%rbp)', 'movq %rcx, -8(%rsp)'):
            with self.subTest(store=store):
                self.rejected(text.replace('movq -8(%rbp), %rax', store + '\n movq -8(%rbp), %rax'))

    def test_frame_register_names_do_not_excuse_body_reads(self):
        duplicate = ASM.replace('movupd %xmm5', 'movq %r13, %rbp\n movsd (%rbp), %xmm7\n movupd %xmm5')
        self.rejected(duplicate)

    def test_loop_local_calls_and_unknown_effects_are_not_hidden_by_reseeding(self):
        # Re-establish all Body facts in the header, so the next iteration
        # cannot accidentally reject a hidden effect merely through clobbers.
        text = ASM.replace('.loop:', '.loop:\n leaq bodies(%rip), %r11\n leaq 56(%r11), %rsi\n leaq 48(%r11), %r13')
        for line in ('call hidden_mass_read', 'call *%rax', 'vmystery %xmm1, %xmm2',
                     'vmystery (%r11), %xmm2', 'movsb', 'popq %rax'):
            with self.subTest(line=line):
                self.rejected(text.replace('leaq 56(%rsi)', line + '\n leaq 56(%rsi)'))

    def test_unknown_instruction_and_unknown_memory_cannot_certify_non_mass(self):
        self.rejected(ASM.replace('movsd (%r13)', 'mull %r10d\n movsd (%r13)'))
        self.rejected(ASM.replace('movsd (%r13)', 'movsd (%r14)'))
        self.rejected(ASM.replace('movsd (%r13)', 'movsd (%r13,%rcx,8)'))

    def test_empty_or_unrecognized_workload_fails(self):
        for text in ('', ASM.replace('main:', 'unrelated:'),
                     ASM.replace('jne .loop', 'jne .elsewhere'),
                     ASM.replace('movsd (%r13)', 'movsd (%r11)').replace('48(%rsi)', '40(%rsi)'),
                     ASM.replace('.size bodies, 280', '.size bodies, 320')):
            with self.subTest(text=text):
                self.rejected(text)

    def test_readonly_labels_require_definitions_and_aliases_are_unmodelled(self):
        literal = '.section .rodata\n.LCFP0:\n .quad 0\n.text\n' + ASM
        literal = literal.replace('movupd %xmm5', 'vmulsd .LCFP0(%rip), %xmm7, %xmm7\n movupd %xmm5')
        self.assertEqual(census.census(literal)['mass_reads'], 2)
        self.rejected(literal.replace('.LCFP0:', '.other:'))
        self.rejected('.set .LCFP0, bodies+48\n' + literal)

    def test_label_numbers_and_formatting_do_not_define_the_metric(self):
        self.assertEqual(census.census(ASM.replace('.loop', '.Lrenamed901'))['mass_reads'], 2)


if __name__ == '__main__':
    unittest.main()
