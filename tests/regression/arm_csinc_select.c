/* Freestanding typedefs keep the AArch64 structural regression independent of
 * host/cross libc header layout. The test is also runnable on native x86 via
 * the normal regression harness, so keep the printf declaration. */
typedef unsigned int uint32_t;
typedef int int32_t;
typedef unsigned long uint64_t;
#define UINT32_MAX (~0u)
#define UINT64_MAX (~0ul)
extern int printf(const char *, ...);

static volatile uint32_t observed;

__attribute__((noinline)) uint32_t inc_if_true_u32(uint32_t base, uint32_t condition) {
    return condition ? base + 1u : base;
}

__attribute__((noinline)) uint32_t inc_if_false_u32(uint32_t base, uint32_t condition) {
    return condition ? base : base + 1u;
}

__attribute__((noinline)) uint64_t inc_if_true_u64(uint64_t base, uint64_t condition) {
    return condition ? base + 1u : base;
}

#define DEFINE_COMPARE_INCREMENT(name, compare_type, operator) \
    __attribute__((noinline)) uint32_t name(                    \
        uint32_t base, compare_type lhs, compare_type rhs       \
    ) {                                                          \
        return lhs operator rhs ? base + 1u : base;              \
    }

DEFINE_COMPARE_INCREMENT(inc_if_eq_u32, uint32_t, ==)
DEFINE_COMPARE_INCREMENT(inc_if_ne_u32, uint32_t, !=)
DEFINE_COMPARE_INCREMENT(inc_if_slt_i32, int32_t, <)
DEFINE_COMPARE_INCREMENT(inc_if_sle_i32, int32_t, <=)
DEFINE_COMPARE_INCREMENT(inc_if_sgt_i32, int32_t, >)
DEFINE_COMPARE_INCREMENT(inc_if_sge_i32, int32_t, >=)
DEFINE_COMPARE_INCREMENT(inc_if_ult_u32, uint32_t, <)
DEFINE_COMPARE_INCREMENT(inc_if_ule_u32, uint32_t, <=)
DEFINE_COMPARE_INCREMENT(inc_if_ugt_u32, uint32_t, >)
DEFINE_COMPARE_INCREMENT(inc_if_uge_u32, uint32_t, >=)

#undef DEFINE_COMPARE_INCREMENT

__attribute__((noinline)) uint32_t inc_loaded_condition(
    uint32_t base,
    const unsigned char *condition
) {
    return *condition ? base + 1u : base;
}

__attribute__((noinline)) uint32_t do_not_fold_delta_two(uint32_t base, uint32_t condition) {
    return condition ? base + 2u : base;
}

__attribute__((noinline)) uint32_t do_not_fold_extra_use(uint32_t base, uint32_t condition) {
    uint32_t increment = base + 1u;
    observed = increment;
    return condition ? increment : base;
}

int main(void) {
    static const unsigned char conditions[] = {0, 1, 2, 255};
    uint64_t hash = 0xcbf29ce484222325ULL;
    for (uint32_t base = 0; base < 9; ++base) {
        for (uint32_t condition = 0; condition < 4; ++condition) {
#define MIX(value) do { hash = (hash ^ (uint64_t)(value)) * 0x100000001b3ULL; } while (0)
            MIX(inc_if_true_u32(base, condition));
            MIX(inc_if_false_u32(base, condition));
            MIX(inc_if_true_u64(UINT64_MAX - base, condition));
            MIX(inc_if_eq_u32(base, condition, 2));
            MIX(inc_if_ne_u32(base, condition, 2));
            MIX(inc_if_slt_i32(base, (int32_t)condition - 2, -1));
            MIX(inc_if_sle_i32(base, (int32_t)condition - 2, -1));
            MIX(inc_if_sgt_i32(base, (int32_t)condition - 2, -1));
            MIX(inc_if_sge_i32(base, (int32_t)condition - 2, -1));
            MIX(inc_if_ult_u32(base, condition, 2));
            MIX(inc_if_ule_u32(base, condition, 2));
            MIX(inc_if_ugt_u32(base, condition, 2));
            MIX(inc_if_uge_u32(base, condition, 2));
            MIX(inc_loaded_condition(base, &conditions[condition]));
            MIX(do_not_fold_delta_two(base, condition));
            MIX(do_not_fold_extra_use(base, condition));
#undef MIX
        }
    }
    /* Exercise defined unsigned wraparound in both 32- and 64-bit forms. */
    hash ^= inc_if_true_u32(UINT32_MAX, 1);
    hash ^= inc_if_true_u64(UINT64_MAX, 1);
    printf("%016llx %u\n", (unsigned long long)hash, observed);
    return 0;
}
