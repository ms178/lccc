// regr_v8_switch_table.c
//
// v8 F7: profile-driven switch lowering. With -fprofile-use:
//   * a HOT dense switch must be lowered to a jump table with the dominant
//     case HOISTED out of it (single compare + je before the table);
//   * a COLD dense switch must NOT get a jump table (compare chain instead).
// The self-checking reference verifies correctness under both lowering
// forms. The round-trip script greps the emitted assembly for the table
// (.long .LBB relocations) and for the hoisted compare.
#include <stdio.h>

__attribute__((noinline)) static int hot_dispatch(int x) {
    /* dense, called millions of times; case 5 dominates the training run */
    switch (x & 15) {
    case 0:  return 1000;
    case 1:  return 1001;
    case 2:  return 1002;
    case 3:  return 1003;
    case 4:  return 1004;
    case 5:  return 2005;   /* dominant */
    case 6:  return 1006;
    case 7:  return 1007;
    case 8:  return 1008;
    case 9:  return 1009;
    case 10: return 1010;
    case 11: return 1011;
    case 12: return 1012;
    case 13: return 1013;
    case 14: return 1014;
    default: return 1015;
    }
}

__attribute__((noinline)) static int cold_dispatch(int x) {
    /* dense but executed once at startup — must stay a chain */
    switch (x & 7) {
    case 0: return 50;
    case 1: return 51;
    case 2: return 52;
    case 3: return 53;
    case 4: return 54;
    case 5: return 55;
    case 6: return 56;
    default: return 57;
    }
}

int main(int argc, char** argv) {
    (void)argv;
    long s = 0;
    for (int i = 0; i < 4000000; i++) {
        /* 70% case 5, rest uniform -> case 5 dominates (hoist trigger) */
        s += hot_dispatch(((i % 10) < 7) ? 5 : (i & 15));
    }
    s += cold_dispatch(argc & 7);   /* runtime value: not foldable; runs once */
    long e = 0;
    for (int i = 0; i < 4000000; i++) {
        int x = ((i % 10) < 7) ? 5 : (i & 15);
        switch (x & 15) {
        case 0: e += 1000; break; case 1: e += 1001; break;
        case 2: e += 1002; break; case 3: e += 1003; break;
        case 4: e += 1004; break; case 5: e += 2005; break;
        case 6: e += 1006; break; case 7: e += 1007; break;
        case 8: e += 1008; break; case 9: e += 1009; break;
        case 10: e += 1010; break; case 11: e += 1011; break;
        case 12: e += 1012; break; case 13: e += 1013; break;
        case 14: e += 1014; break; default: e += 1015; break;
        }
    }
    e += 50 + (argc & 7);   /* cold_dispatch(argc & 7) */
    if (s != e) { printf("MISMATCH s=%ld e=%ld\n", s, e); return 1; }
    printf("ok %ld\n", s);
    return 0;
}
