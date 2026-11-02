/* zlib-ng zng_deflateSetParams: `int32_t err = p->size < min`
 * must materialize the compare, not reuse the size operand.
 *
 * LCCC classified the Cmp dest as an immediately-consumed Cast source
 * and denied it a home. The Cmp then skipped setcc and the Cast read
 * the still-live `size` register (4), so `if (err)` fired for a valid
 * 4-byte buffer (Z_BUF_ERROR, example test).
 */
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>

typedef struct {
    int param;
    void *buf;
    size_t size;
    int32_t status;
} PV;

static int32_t pre(PV **out, size_t min_size, PV *param) {
    int32_t buf_error = param->size < min_size;
    if (*out != NULL) {
        (*out)->status = -5;
        buf_error = 1;
    }
    *out = param;
    return buf_error;
}

__attribute__((noinline))
int setparams(PV *params, size_t count) {
    PV *new_level = NULL;
    PV *new_strategy = NULL;
    int buf_error = 0;
    for (size_t i = 0; i < count; i++)
        params[i].status = 0;
    for (size_t i = 0; i < count; i++) {
        int param_buf_error;
        switch (params[i].param) {
        case 0:
            param_buf_error = pre(&new_level, sizeof(int), &params[i]);
            break;
        case 1:
            param_buf_error = pre(&new_strategy, sizeof(int), &params[i]);
            break;
        default:
            param_buf_error = 0;
            break;
        }
        if (param_buf_error) {
            params[i].status = -5;
            buf_error = 1;
        }
    }
    return buf_error ? -5 : 0;
}

int main(void) {
    int a = 0, b = 0;
    PV p[2] = {
        {0, &a, sizeof(int), 0},
        {1, &b, sizeof(int), 0},
    };
    int r = setparams(p, 2);
    if (r != 0 || p[0].status != 0 || p[1].status != 0) {
        printf("FAIL ret=%d st=%d,%d\n", r, p[0].status, p[1].status);
        return 1;
    }
    return 0;
}
