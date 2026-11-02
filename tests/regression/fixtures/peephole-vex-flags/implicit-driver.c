#include <stdio.h>

extern int original_sha(void), removed_sha(void);
extern int original_fxsave(void), removed_fxsave(void);
extern int original_xsave_upper(void), removed_xsave_upper(void);
extern int original_fxrstor_slot(void), removed_fxrstor_slot(void);

extern int original_index_spill(void), rewritten_index_spill(void);
extern int original_mask_copy(void), rewritten_mask_copy(void);
extern int original_mask_width(void), rewritten_mask_width(void);
extern int original_saved_bracket(void), rewritten_saved_bracket(void);
extern int original_blend_result(void), rewritten_blend_result(void);

static int check(const char *name, int (*original)(void), int (*removed)(void)) {
    int a = original(), b = removed();
    printf("%s: original=%d removed=%d\n", name, a, b);
    return a != 1 || b != 0;
}

int main(void) {
    int bad = 0;
    if (__builtin_cpu_supports("sha"))
        bad |= check("sha256rnds2 implicit XMM0", original_sha, removed_sha);
    else
        puts("SKIP: SHA native witness (CPU lacks SHA; Rust effect test remains mandatory)");
    bad |= check("fxsave implicit XMM7", original_fxsave, removed_fxsave);
    bad |= check("xsave upper YMM7", original_xsave_upper, removed_xsave_upper);
    bad |= check("fxrstor non-scalar frame read", original_fxrstor_slot, removed_fxrstor_slot);
    bad |= check("string-mask copy relation", original_mask_copy, rewritten_mask_copy);
    bad |= check("string-mask prior upper definition", original_mask_width, rewritten_mask_width);
    bad |= check("saved bracket state", original_saved_bracket, rewritten_saved_bracket);
    bad |= check("implicit wide blend mask", original_blend_result, rewritten_blend_result);
    bad |= check("string-index spill home", original_index_spill, rewritten_index_spill);
    return bad;
}
