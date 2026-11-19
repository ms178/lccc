"""Pin the accounting for scheduled pairs when a deduplicated pair intervenes."""

import unittest

from tests.regression.check_peephole_pair_parallel import check_accounting


class PairCounterTests(unittest.TestCase):
    def test_deduped_is_not_a_scheduled_pair(self):
        failures = []
        check_accounting("x86-64", "probe.c", "-O1", ["parallel", "serial"], (1, 1, 1), failures)
        self.assertEqual(failures, [])

    def test_serial_parallel_swap_is_rejected_even_if_total_matches(self):
        failures = []
        check_accounting("x86-64", "probe.c", "-O1", ["parallel"], (0, 1, 0), failures)
        self.assertEqual(len(failures), 1)
