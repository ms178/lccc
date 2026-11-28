/* `target` on prototypes and `#pragma GCC target`, judged as GCC 16.2 does.
 *
 * A trailing comment holds the errors GCC gives on that line, in GCC's
 * order, separated by ` | `; a line without one is accepted. In front of a
 * message, `@name` says GCC reports it at the declared name (lccc too) and
 * `@cc1` that GCC reports it without a location at all ("cc1: error: ...",
 * which is how it refuses a pragma's string): lccc gives the pragma's line.
 *
 * What the lines show:
 *   - a prototype's target string is judged like a definition's;
 *   - a function declared through a typedef of a function type is a
 *     function (`fn_type p10;`), for the attribute and for its diagnostics;
 *   - `#pragma GCC target` puts its strings in front of the `target` of
 *     every function declared after it, so an `arch=` in a pragma and one
 *     on the function are a repetition, reported at the function;
 *   - a pragma whose string GCC refuses is diagnosed once, where it stands,
 *     and does not reach the functions after it;
 *   - `push_options` / `pop_options` / `reset_options` bound the pragmas.
 *
 * Checked by tests/regression/check_target_attribute_arguments.py. */

int p0(void) __attribute__((target("avx2")));
__attribute__((target("woof"))) int p1(void); /* expect: @name attribute 'target' argument 'woof' is unknown */
int p2(void) __attribute__((target("woof,meow"))); /* expect: @name attribute 'target' argument 'woof' is unknown | @name attribute 'target' argument 'meow' is unknown */
__attribute__((target("arch=haswell,arch=skylake"))) int *p3(void); /* expect: @name attribute value 'arch=skylake' was already specified in 'target' attribute */
__attribute__((target("avx2, sse4.2"))) extern int p4(int); /* expect: @name attribute 'target' argument ' sse4.2' is unknown */
__attribute__((target("fpmath=bogus"))) static int p5(void); /* expect: @name attribute value 'fpmath=bogus' is unknown in 'target' attribute */
__attribute__((target("no-general-regs-only"))) int p6(void); /* expect: @name pragma or attribute 'target("general-regs-only")' does not allow a negated form */
__attribute__((target("bmi2"))) int p7(void), p8(void);
__attribute__((target("arch=hiss"))) int p9(void); /* expect: bad value 'hiss' for 'target("arch=")' attribute */
typedef int fn_type(void);
__attribute__((target("sse4.2"))) fn_type p10;
__attribute__((target("SSE4.2"))) fn_type p11; /* expect: @name attribute 'target' argument 'SSE4.2' is unknown */
#pragma GCC push_options
#pragma GCC target("avx2")
int p12(void);
#pragma GCC target("bmi2", "lzcnt")
int p13(void) { return 13; }
#pragma GCC pop_options
#pragma GCC push_options
#pragma GCC target("arch=haswell")
int p14(void);
__attribute__((target("arch=skylake"))) int p15(void); /* expect: @name attribute value 'arch=skylake' was already specified in 'target' attribute */
__attribute__((target("tune=skylake"))) int p16(void) { return 16; }
#pragma GCC target("arch=skylake")
int p17(void); /* expect: @name attribute value 'arch=skylake' was already specified in 'target' attribute */
int p18(void) { return 18; } /* expect: @name attribute value 'arch=skylake' was already specified in 'target' attribute */
#pragma GCC pop_options
int p19(void) { return 19; }
#pragma GCC target("woof") /* expect: @cc1 attribute 'target' argument 'woof' is unknown */
int p20(void) { return 20; }
#pragma GCC target("avx2,AVX2,bmi 2") /* expect: @cc1 attribute 'target' argument 'AVX2' is unknown | @cc1 attribute 'target' argument 'bmi 2' is unknown */
#pragma GCC target("fpmath=bogus") /* expect: @cc1 attribute value 'fpmath=bogus' is unknown in 'target' attribute */
#pragma GCC target("no-general-regs-only") /* expect: @cc1 pragma or attribute 'target("general-regs-only")' does not allow a negated form */
#pragma GCC target("arch=k8,arch=k8") /* expect: @cc1 attribute value 'arch=k8' was already specified in 'target' attribute */
#pragma GCC target("tune=k8", "tune=k8") /* expect: @cc1 attribute value 'tune=k8' was already specified in 'target' attribute */
__attribute__((target("avx2"))) int p21(void) { return 21; }
#pragma GCC reset_options
#pragma GCC target ("sse4.1") /* a comment after the operand */
int p22(void) { return 22; }
#pragma GCC reset_options
#pragma GCC target("arch=hiss") /* expect: bad value 'hiss' for 'target("arch=")' attribute */
#pragma GCC reset_options
int p23(void) { return 23; }
