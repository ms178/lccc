/* glibc_gi_redirect.c — __asm__("__GI_x") hidden-symbol redirects in all
 * data-reference paths: plain loads, address-of, struct member access,
 * array subscript (glibc _dl_argv[0] / _null_auth.oa_flavor / &_null_auth).
 * Missing redirects produced undefined plain references at ld.so link. */
#include <stdio.h>

extern int plain_var __asm__("__GI_plain_var");
int plain_var = 5;

extern int arr[3] __asm__("__GI_arr");
int arr[3] = {10, 20, 30};

struct s { int a; int b; };
extern struct s st __asm__("__GI_st");
struct s st = {7, 8};

int main(void) {
    if (plain_var != 5) { printf("FAIL gi plain\n"); return 1; }
    if (arr[1] != 20) { printf("FAIL gi array\n"); return 1; }
    if (st.b != 8) { printf("FAIL gi struct\n"); return 1; }
    int *p = &plain_var;
    if (*p != 5) { printf("FAIL gi addr\n"); return 1; }
    printf("PASS gi_redirect\n");
    return 0;
}
