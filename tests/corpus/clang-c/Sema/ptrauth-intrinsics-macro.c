// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fp
//options:  --c
# 1 "Sema/ptrauth-intrinsics-macro.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/ptrauth-intrinsics-macro.c" 2





# 1 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/ptrauth.h" 1
# 13 "/mds/clang/clang-22.1.0/build/lib/clang/22/include/ptrauth.h"
typedef enum {
  ptrauth_key_asia = 0,
  ptrauth_key_asib = 1,
  ptrauth_key_asda = 2,
  ptrauth_key_asdb = 3,


  ptrauth_key_process_independent_code = ptrauth_key_asia,


  ptrauth_key_process_dependent_code = ptrauth_key_asib,


  ptrauth_key_process_independent_data = ptrauth_key_asda,


  ptrauth_key_process_dependent_data = ptrauth_key_asdb,





  ptrauth_key_return_address = ptrauth_key_process_dependent_code,



  ptrauth_key_function_pointer = ptrauth_key_process_independent_code,



  ptrauth_key_cxx_vtable_pointer = ptrauth_key_process_independent_data,


  ptrauth_key_method_list_pointer = ptrauth_key_asda,


  ptrauth_key_objc_isa_pointer = ptrauth_key_process_independent_data,
  ptrauth_key_objc_super_pointer = ptrauth_key_process_independent_data,


  ptrauth_key_objc_sel_pointer = ptrauth_key_process_dependent_data,


  ptrauth_key_objc_class_ro_pointer = ptrauth_key_process_independent_data,


  ptrauth_key_init_fini_pointer = ptrauth_key_process_independent_code,



} ptrauth_key;


typedef long unsigned int ptrauth_extra_data_t;


typedef long unsigned int ptrauth_generic_signature_t;
# 7 "Sema/ptrauth-intrinsics-macro.c" 2




extern int dv;

void test(int *dp, int value) {
  dp = ({ (void)2; dp; });
  ptrauth_extra_data_t t0 = ({ (void)dp; (void)value; ((ptrauth_extra_data_t)0); });
  (void)t0;
  dp = ({ (void)2; (void)0; dp; });
  dp = ({ (void)2; (void)dp; (void)2; (void)dp; dp; });
  dp = ({ (void)2; (void)0; dp; });
  int pu0 = 0, pu1 = 0, pu2 = 0, pu3 = 0, pu4 = 0, pu5 = 0, pu6 = 0, pu7 = 0;
  ({ (void)&pu0; (void)value; ((ptrauth_extra_data_t)0); });
  ({ (void)2; (void)dp; (void)2; (void)dp; &pu1; });
  ({ (void)2; (void)&pu2; (void)2; (void)dp; dp; });
  ({ (void)2; (void)dp; (void)2; (void)&pu3; dp; });
  ({ (void)pu4; (void)dp; ((ptrauth_generic_signature_t)0); });
  ({ (void)dp; (void)pu5; ((ptrauth_generic_signature_t)0); });
  ({ (void)2; (void)value; &pu6; });
  ({ (void)2; (void)pu7; dp; });



  int t2 = ({ (void)dp; (void)0; ((ptrauth_generic_signature_t)0); });
  (void)t2;
}

void test_string_discriminator(int *dp) {
  ptrauth_extra_data_t t0 = ({ (void)"string"; ((ptrauth_extra_data_t)0); });
  (void)t0;
}

void test_type_discriminator(int *dp) {
  ptrauth_extra_data_t t0 = ((ptrauth_extra_data_t)0);
  (void)t0;
}

void test_sign_constant(int *dp) {
  dp = ({ (void)2; (void)0; &dv; });
}
