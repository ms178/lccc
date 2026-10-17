#!/usr/bin/env python3
"""Compiler-free mutation controls for producer status and kernel identity."""
from collections import Counter
import os
import sys
import unittest

import check_generator_map as gate


class TraceContract(unittest.TestCase):
    def producer(self, exit_code, kernels):
        script = '\n'.join([*(f'print("[VEC-GEN] fn={name} body=B2")' for name in kernels),
                            f'raise SystemExit({exit_code})'])
        return gate.capture_trace([sys.executable, '-c', script], dict(os.environ))

    def test_success_with_zero_is_valid_only_for_disabled_arm(self):
        counts = self.producer(0, [])
        gate.validate_trace(counts, False)
        with self.assertRaises(RuntimeError):
            gate.validate_trace(counts, True)

    def test_success_with_six_named_kernels(self):
        counts = self.producer(0, sorted(gate.POSITIVES))
        gate.validate_trace(counts, True)
        with self.assertRaises(RuntimeError):
            gate.validate_trace(counts, False)

    def test_failure_without_trace_is_never_a_success(self):
        with self.assertRaisesRegex(RuntimeError, 'exited 23'):
            self.producer(23, [])

    def test_failure_with_complete_trace_is_never_a_success(self):
        with self.assertRaisesRegex(RuntimeError, 'exited 23'):
            self.producer(23, sorted(gate.POSITIVES))

    def test_duplicate_positive_cannot_mask_missing_kernel(self):
        counts = Counter({name: 1 for name in gate.POSITIVES if name != 'gen_a'})
        counts['gen_b'] += 1
        with self.assertRaises(RuntimeError):
            gate.validate_trace(counts, True)

    def test_duplicate_positive_is_rejected_when_all_kernels_are_present(self):
        counts = Counter({name: 1 for name in gate.POSITIVES})
        counts['gen_b'] += 1
        with self.assertRaisesRegex(RuntimeError, 'gen_b.*2'):
            gate.validate_trace(counts, True)

    def test_unexpected_or_forbidden_kernel_is_rejected(self):
        for name in ('gen_g', 'gen_volatile', 'gen_constant', 'unknown'):
            counts = Counter({name: 1, **{k: 1 for k in gate.POSITIVES}})
            with self.subTest(name=name), self.assertRaises(RuntimeError):
                gate.validate_trace(counts, True)


if __name__ == '__main__':
    unittest.main()
