struct wide { long values[5]; };
struct flex { int count; int values[]; };
struct wide global_wide;
typedef int v4 __attribute__((vector_size(16)));
typedef unsigned char v2 __attribute__((vector_size(2)));
int global_vector __attribute__((used, vector_size(16), section(".global_vector"))) = { 1, 2, 3, 4 };

__attribute__((noinline, used)) int locals(void) {
    static signed char c __attribute__((used)) = -3;
    static unsigned char c_zero __attribute__((used));
    static short h __attribute__((used)) = 12;
    static int i __attribute__((used)) = 25;
    static int i_zero __attribute__((used));
    static float f __attribute__((used)) = 1.5f;
    static double d __attribute__((used)) = 2.5;
    static __thread int tls __attribute__((used, retain)) = 9;
    static int array[] __attribute__((used)) = { 1, 2, 3 };
    static struct wide aggregate __attribute__((used)) = {{ 1, 2, 3, 4, 5 }};
    static struct flex flexible __attribute__((used)) = { 3, { 6, 7, 8 } };
    static v4 vector __attribute__((used)) = { 1, 2, 3, 4 };
    static v2 tiny __attribute__((used, section(".tiny_vector"))) = { 1, 2 };
    static int decl_vec __attribute__((used, vector_size(16), section(".decl_vector"))) = { 1, 2, 3, 4 };
    static const struct wide *p __attribute__((used, section(".ptr_table"))) = { &global_wide };
    static const struct wide *q __attribute__((used, section(".ptr_table"))) = { &global_wide };
    return c + h + i + (int)f + (int)d + tls + array[2] + (int)aggregate.values[4]
           + flexible.values[2] + vector[3] + tiny[1] + decl_vec[3] + global_vector[3]
           + (p != q) + c_zero + i_zero;
}
int main(void) { return locals() != 76; }
