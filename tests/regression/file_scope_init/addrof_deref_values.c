/* C11 6.5.3.2p3: &*p is p, even when p is null. Exercise static
 * scalar, array and aggregate initializers, values and relocations. */
#include <stdint.h>
#include <stdio.h>

static int data[] = { 3, 7, 11, 19 };
static int f(void) { return 23; }

static int *null_ptr = &*(int *)0;
static int *raw_ptr = &*(int *)(uintptr_t)0x1234;
static int *global_ptr = &*(&data[0]);
static int *offset_ptr = &*(&data[2]);
static int (*array_ptr)[4] = &*(&data);
static int (*function_ptr)(void) = &*f;
static const char *string_ptr = &*("abc" + 1);
static int *conditional_ptr = &*(1 ? &data[3] : (int *)0);
static int *nested_ptr = &*(&*(&data[1]));
static int *ptrs[] = { &*(int *)0, &*(&data[2]), &*(data + 3) };
struct record { int *null; int *at; int (*func)(void); uintptr_t raw; };
static struct record rec = { &*(int *)0, &*(&data[1]), &*f,
                             (uintptr_t)&*(int *)(uintptr_t)0x1234 };
static int *compound_ptr = &*(&(int){ 31 });
struct inner { int *p; };
struct outer { struct inner inner; int *at; };
static struct outer nested = { { &*(&(int){ 37 }) }, &data[0] };
struct raw_record { uintptr_t p; };
static struct raw_record raw_record = { (uintptr_t)&*(&(int){ 43 }) };
static struct record records[] = {
    { &*(int *)0, &*(&(int){ 47 }), &*f, (uintptr_t)&*(int *)0 }
};
/* A same-typed array compound literal initializes the member's value, not
 * a relocation, even when another member forces a relocation-aware image. */
struct mixed { int a[2][3]; int *p; };
static struct mixed mixed = {
    .a = (int[2][3]){{1, 2, 3}, {4, 5, 6}}, .p = &*(&(int){53})
};
struct pointer_box { int *a[3]; int *p; };
static struct pointer_box box = {
    .a = (int *[3]){ &*(&(int){59}), 0, &*(data + 1) },
    .p = &*(&data[0])
};

static int local(void) {
    static int item = 41;
    static int *ptr = &*(&item);
    static int *np = &*(int *)0;
    return *ptr == 41 && np == 0;
}

int main(void) {
    int ok = null_ptr == 0 && (uintptr_t)raw_ptr == 0x1234 &&
        global_ptr == &data[0] && offset_ptr == &data[2] &&
        array_ptr == &data && function_ptr() == 23 &&
        string_ptr[0] == 'b' && conditional_ptr == &data[3] &&
        nested_ptr == &data[1] && ptrs[0] == 0 &&
        ptrs[1] == &data[2] && ptrs[2] == &data[3] &&
        rec.null == 0 && rec.at == &data[1] && rec.func() == 23 &&
        rec.raw == 0x1234 && *compound_ptr == 31 &&
        *nested.inner.p == 37 && nested.at == &data[0] &&
        *(int *)raw_record.p == 43 && records[0].null == 0 &&
        *records[0].at == 47 && records[0].func() == 23 &&
        records[0].raw == 0 && mixed.a[0][0] == 1 && mixed.a[1][2] == 6 &&
        *mixed.p == 53 && *box.a[0] == 59 && box.a[1] == 0 &&
        box.a[2] == &data[1] && box.p == &data[0] && local();
    printf("%d\n", ok);
    return !ok;
}
