// Narrow-integer codegen shapes, pinned in the emitted TEXT as well as in
// behaviour.
//
// The backend's narrow-integer invariant is "an i8/i16 home holds the value
// extended to 32 bits", and every consumer that reads the register at 64-bit
// width must re-extend from the narrow width (`operand_to_rax` /
// `operand_to_rcx`).  That rule is invisible in a differential run when the
// producer happens to leave the upper half zero, so `check_narrow_int_codegen.py`
// asserts the shapes below directly:
//
//   * `div_i16` / `rem_i16` / `div_i8` / `div_u16` — a narrow divide is a
//     32-bit divide (`cltd; idivl` / `xorl %edx,%edx; divl`), never `idivq`.
//     A narrow dividend is bounded by its own width, so `idivl` is exact and
//     `INT32_MIN / -1` is unreachable.
//   * `index_i16` / `index_i8` / `index_u16` — a narrow value used as an index
//     is SIGN-extended (signed) or zero-extended (unsigned) into the address
//     computation; the values are chosen negative so a zero-extending copy of
//     the signed lanes would read the wrong element.
//   * `staged_twice_i16` — the same narrow value staged into the accumulator
//     twice must not be re-canonicalized twice (the accumulator cache marks
//     the 64-bit-canonical form once).
//
// The functions are also called from `main` with negative operands, so the
// behavioural half of the corpus run fails if any of the above regresses.
//
// TARGET SCOPE: x86-64 only in intent, but the fixture itself is portable and
// needs no `-m32` sidecar -- i686's narrow divide is ALREADY the 32-bit
// `idivl`, and its narrow staging copies are 32-bit, so nothing here is a
// distinct path there. A host without 32-bit multilib reports the build as
// `host-skip` through `run_regression.py`, which is the honest verdict.
#include <stdint.h>
#include <stdio.h>

static long g_arr[64];

__attribute__((noinline)) long div_i16(int16_t a, int16_t b) { return a / b; }
__attribute__((noinline)) long rem_i16(int16_t a, int16_t b) { return a % b; }
__attribute__((noinline)) long div_i8(int8_t a, int8_t b) { return a / b; }
__attribute__((noinline)) unsigned long div_u16(uint16_t a, uint16_t b) {
    return a / b;
}
__attribute__((noinline)) unsigned long rem_u16(uint16_t a, uint16_t b) {
    return a % b;
}

// The INDEX is the narrow value; the bias keeps every access inside `g_arr`
// so the only thing under test is how the index is widened.
__attribute__((noinline)) long index_i16(int16_t i) { return g_arr[i + 32]; }
__attribute__((noinline)) long index_i8(int8_t i) { return g_arr[i + 32]; }
__attribute__((noinline)) long index_u16(uint16_t i) { return g_arr[i]; }

// One narrow value, two accumulator stagings: the divide stages the dividend
// and the following store stages the same value again.
__attribute__((noinline)) long staged_twice_i16(int16_t a, int16_t b) {
    long q = a / b;
    return q + (a % b);
}

int main(void) {
    long checks = 0;
    long long sum = 0;
    for (int i = 0; i < 64; i++) {
        g_arr[i] = (long)(i * 7 - 100);
    }
    static const int16_t lanes[] = {-32768, -300, -17, -2, -1, 1, 2, 3, 17, 300,
                                    32767};
    const int n = (int)(sizeof lanes / sizeof lanes[0]);
    for (int i = 0; i < n; i++) {
        for (int j = 0; j < n; j++) {
            int16_t a = lanes[i];
            int16_t b = lanes[j];
            if (!b) {
                continue;
            }
            // INT_MIN / -1 has no representable quotient: UB, and a narrow
            // `idiv` raises #DE. Skip it as an INPUT (never as a check).
            if (a == INT16_MIN && b == -1) {
                continue;
            }
            sum += div_i16(a, b) + rem_i16(a, b) + (long long)((a / 2) / b);
            // `(int8_t)b` truncates to zero for some lanes (-32768 does), and
            // a division by the truncated zero raises #DE: guard the CONVERTED
            // divisor, not the original.
            if ((int8_t)b != 0) {
                sum += div_i8((int8_t)(a / 128), (int8_t)b);
            }
            if (b > 0) {
                sum += (long long)div_u16((uint16_t)a, (uint16_t)b);
                sum += (long long)rem_u16((uint16_t)a, (uint16_t)b);
            }
            checks += 3;
        }
    }
    for (int16_t i = -32; i < 32; i++) {
        sum += index_i16(i) + index_i8((int8_t)i) + index_u16((uint16_t)(i + 32));
        checks++;
    }
    for (int16_t a = -9; a <= 9; a++) {
        for (int16_t b = -3; b <= 3; b++) {
            if (!b || (a == INT16_MIN && b == -1)) {
                continue;
            }
            sum += staged_twice_i16(a, b);
            checks++;
        }
    }
    printf("narrow_checks=%ld sum=%lld\n", checks, sum);
    return 0;
}
