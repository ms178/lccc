#!/usr/bin/env python3
"""Exhaustive small signed-ring check of the controlling-header proof.

This tests the mathematical model, not all 32/64-bit compiler programs.
Nonheader guards are deliberately outside the admitted model.
"""
from itertools import product


def check(bits):
    low, high = -(1 << (bits - 1)), (1 << (bits - 1)) - 1
    modulus = 1 << bits

    def signed(value):
        return (value - low) % modulus + low

    cases = admitted = comparisons = 0
    for start, step, offset, bound, inclusive, header in product(
        range(low, high + 1), range(1, high + 1), range(low, high + 1),
        range(low, high + 1), (False, True), (False, True)
    ):
        cases += 1
        threshold = bound + int(inclusive) - offset
        if not header or not low <= threshold <= high or start >= threshold:
            continue
        last = start + ((threshold - start + step - 1) // step) * step
        if not (low <= last <= high and low <= start + offset <= high
                and low <= last + offset <= high):
            continue
        admitted += 1
        observed = []
        iv = start
        while True:
            assert iv not in observed, 'admitted modular loop did not terminate'
            observed.append(iv)
            value = signed(iv + offset)
            continues = value <= bound if inclusive else value < bound
            assert continues == (iv < threshold)
            if not continues:
                break
            iv = signed(iv + step)
        assert observed[-1] == last
        # Independent signed body comparisons may use this range only AFTER
        # the real guard has passed the checks above.
        for addend, other_bound in product(range(high + 1), range(low, high + 1)):
            if not (low <= other_bound - addend <= high
                    and low <= start + addend <= high and low <= last + addend <= high):
                continue
            for iv in observed:
                assert (signed(iv + addend) < other_bound) == (iv < other_bound - addend)
                comparisons += 1
    print(f'affine control model: {bits}-bit {cases} cases, {admitted} admitted, '
          f'{comparisons} comparison checks; all exact')


if __name__ == '__main__':
    for width in (2, 3, 4):
        check(width)
