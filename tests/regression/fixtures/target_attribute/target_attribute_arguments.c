/* The arguments of `target("...")`, judged as GCC 16.2 judges them.
 *
 * Every function below carries one target string. A trailing comment holds
 * the errors GCC gives for it, in GCC's order, separated by ` | `; a
 * function without one is accepted. `@name` in front of a message says GCC
 * reports it at the function's name, which lccc does too; the CPU-name
 * errors go to the start of the declaration in GCC and to the first
 * specifier in lccc, so only their line is compared.
 *
 * lccc used to take every string in silence: a misspelt extension
 * (`ssse4.1`, `bmi 2`, `AVX2`, `-mavx2`) compiled the function without it.
 * The names GCC knows and lccc does not model (`rdseed`, `clwb`, `tbm`,
 * ...) are accepted and change nothing.
 *
 * Checked by tests/regression/check_target_attribute_arguments.py. */

__attribute__((target("avx2"))) int t0(void) { return 0; }
__attribute__((target("no-avx512f,avx2"))) int t1(void) { return 1; }
__attribute__((target("rdseed"))) int t2(void) { return 2; }
__attribute__((target("clwb,no-xop,sse4a"))) int t3(void) { return 3; }
__attribute__((target("adx,fsgsbase,xsavec,xsaves,sgx"))) int t4(void) { return 4; }
__attribute__((target("cld,no-recip,80387,no-ieee-fp"))) int t5(void) { return 5; }
__attribute__((target("general-regs-only"))) int t6(void) { return 6; }
__attribute__((target("sse4.1,sse4.2,sse4,no-sse4"))) int t7(void) { return 7; }
__attribute__((target("fpmath=sse"))) int t8(void) { return 8; }
__attribute__((target("fpmath=sse+387"))) int t9(void) { return 9; }
__attribute__((target("fpmath=sse,fpmath=387"))) int t10(void) { return 10; }
__attribute__((target("prefer-vector-width=256,prefer-vector-width=none"))) int t11(void) { return 11; }
__attribute__((target("arch=haswell,tune=native"))) int t12(void) { return 12; }
__attribute__((target("tune=generic"))) int t13(void) { return 13; }
__attribute__((target("no-arch=k8"))) int t14(void) { return 14; }
__attribute__((target("avx2,sse,"))) int t15(void) { return 15; }
__attribute__((target("default"))) int t16(void) { return 16; }
__attribute__((target("3dnow,3dnowa,tbm,lwp,fma4"))) int t17(void) { return 17; }
__attribute__((target("amx-tile,amx-int8,amx-bf16,amx-fp16,amx-complex"))) int t18(void) { return 18; }
__attribute__((target("kl,widekl,pku,rdpid,waitpkg"))) int t19(void) { return 19; }
__attribute__((target("foo"))) int t20(void) { return 20; } /* expect: @name attribute 'target' argument 'foo' is unknown */
__attribute__((target("no-foo"))) int t21(void) { return 21; } /* expect: @name attribute 'target' argument 'no-foo' is unknown */
__attribute__((target("AVX2"))) int t22(void) { return 22; } /* expect: @name attribute 'target' argument 'AVX2' is unknown */
__attribute__((target("mavx2"))) int t23(void) { return 23; } /* expect: @name attribute 'target' argument 'mavx2' is unknown */
__attribute__((target("-mavx2"))) int t24(void) { return 24; } /* expect: @name attribute 'target' argument '-mavx2' is unknown */
__attribute__((target("sse4_1"))) int t25(void) { return 25; } /* expect: @name attribute 'target' argument 'sse4_1' is unknown */
__attribute__((target("avx2, sse"))) int t26(void) { return 26; } /* expect: @name attribute 'target' argument ' sse' is unknown */
__attribute__((target(" avx2"))) int t27(void) { return 27; } /* expect: @name attribute 'target' argument ' avx2' is unknown */
__attribute__((target("avx2 "))) int t28(void) { return 28; } /* expect: @name attribute 'target' argument 'avx2 ' is unknown */
__attribute__((target(",avx2"))) int t29(void) { return 29; } /* expect: @name attribute 'target' argument '' is unknown */
__attribute__((target("avx2,,sse"))) int t30(void) { return 30; } /* expect: @name attribute 'target' argument '' is unknown */
__attribute__((target("no-"))) int t31(void) { return 31; } /* expect: @name attribute 'target' argument 'no-' is unknown */
__attribute__((target("no-no-sse"))) int t32(void) { return 32; } /* expect: @name attribute 'target' argument 'no-no-sse' is unknown */
__attribute__((target("avx2,bar=3"))) int t33(void) { return 33; } /* expect: @name attribute 'target' argument 'bar=3' is unknown */
__attribute__((target("sse=1"))) int t34(void) { return 34; } /* expect: @name attribute 'target' argument 'sse=1' is unknown */
__attribute__((target("arch="))) int t35(void) { return 35; } /* expect: @name attribute 'target' argument 'arch=' is unknown */
__attribute__((target("branch-cost=2"))) int t36(void) { return 36; } /* expect: @name attribute 'target' argument 'branch-cost=2' is unknown */
__attribute__((target("fpmath=sse,387"))) int t37(void) { return 37; } /* expect: @name attribute 'target' argument '387' is unknown */
__attribute__((target("foo,arch=zzz"))) int t38(void) { return 38; } /* expect: @name attribute 'target' argument 'foo' is unknown */
__attribute__((target("arch=haswell,foo"))) int t39(void) { return 39; } /* expect: @name attribute 'target' argument 'foo' is unknown */
__attribute__((target("sse,foo,bar"))) int t40(void) { return 40; } /* expect: @name attribute 'target' argument 'foo' is unknown | @name attribute 'target' argument 'bar' is unknown */
__attribute__((target("default,avx2"))) int t41(void) { return 41; } /* expect: @name attribute 'target' argument 'default' is unknown */
__attribute__((target("no-default"))) int t42(void) { return 42; } /* expect: @name attribute 'target' argument 'no-default' is unknown */
__attribute__((target("x32"))) int t43(void) { return 43; } /* expect: @name attribute 'target' argument 'x32' is unknown */
__attribute__((target("red-zone"))) int t44(void) { return 44; } /* expect: @name attribute 'target' argument 'red-zone' is unknown */
__attribute__((target("vzeroupper"))) int t45(void) { return 45; } /* expect: @name attribute 'target' argument 'vzeroupper' is unknown */
__attribute__((target("stackrealign"))) int t46(void) { return 46; } /* expect: @name attribute 'target' argument 'stackrealign' is unknown */
__attribute__((target("ssse4.1"))) int t47(void) { return 47; } /* expect: @name attribute 'target' argument 'ssse4.1' is unknown */
__attribute__((target("avx512"))) int t48(void) { return 48; } /* expect: @name attribute 'target' argument 'avx512' is unknown */
__attribute__((target("bmi 2"))) int t49(void) { return 49; } /* expect: @name attribute 'target' argument 'bmi 2' is unknown */
__attribute__((target("sse5"))) int t50(void) { return 50; } /* expect: @name attribute 'target' argument 'sse5' is unknown */
__attribute__((target("fpmath=bogus"))) int t51(void) { return 51; } /* expect: @name attribute value 'fpmath=bogus' is unknown in 'target' attribute */
__attribute__((target("prefer-vector-width=64,foo"))) int t52(void) { return 52; } /* expect: @name attribute value 'prefer-vector-width=64' is unknown in 'target' attribute | @name attribute 'target' argument 'foo' is unknown */
__attribute__((target("no-general-regs-only,foo"))) int t53(void) { return 53; } /* expect: @name pragma or attribute 'target("general-regs-only")' does not allow a negated form */
__attribute__((target("arch=k8,arch=k8,foo"))) int t54(void) { return 54; } /* expect: @name attribute value 'arch=k8' was already specified in 'target' attribute | @name attribute 'target' argument 'foo' is unknown */
__attribute__((target("no-arch=k8,arch=k8"))) int t55(void) { return 55; } /* expect: @name attribute value 'arch=k8' was already specified in 'target' attribute */
__attribute__((target("tune=k8,tune=k8"))) int t56(void) { return 56; } /* expect: @name attribute value 'tune=k8' was already specified in 'target' attribute */
__attribute__((target("arch=zzz"))) int t57(void) { return 57; } /* expect: bad value 'zzz' for 'target("arch=")' attribute */
__attribute__((target("tune=zzz"))) int t58(void) { return 58; } /* expect: bad value 'zzz' for 'target("tune=")' attribute */
__attribute__((target("arch=zzz,tune=yyy"))) int t59(void) { return 59; } /* expect: bad value 'zzz' for 'target("arch=")' attribute | bad value 'yyy' for 'target("tune=")' attribute */
__attribute__((target("tune=yyy,arch=zzz"))) int t60(void) { return 60; } /* expect: bad value 'zzz' for 'target("arch=")' attribute | bad value 'yyy' for 'target("tune=")' attribute */
__attribute__((target("arch=generic"))) int t61(void) { return 61; } /* expect: 'generic' CPU can be used only for 'target("tune=")' attribute */
__attribute__((target("arch=native"))) int t62(void) { return 62; } /* expect: bad value 'native' for 'target("arch=")' attribute */
__attribute__((target("arch=pentium4"))) int t63(void) { return 63; } /* expect: CPU you selected does not support x86-64 instruction set */
__attribute__((target("tune=i386"))) int t64(void) { return 64; } /* expect: CPU you selected does not support x86-64 instruction set */
__attribute__((target("arch=pentium4,tune=i386"))) int t65(void) { return 65; } /* expect: CPU you selected does not support x86-64 instruction set */
__attribute__((target("tune=x86-64-v3"))) int t66(void) { return 66; } /* expect: bad value 'x86-64-v3' for 'target("tune=")' attribute */
__attribute__((target("tune=generic,arch=generic"))) int t67(void) { return 67; } /* expect: 'generic' CPU can be used only for 'target("tune=")' attribute */
