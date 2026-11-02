// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/inline.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/inline.c" 2
# 49 "Sema/inline.c"
# 1 "Sema/inline.c" 1








static int staticVar;
static int staticFunction(void);
static struct { int x; } staticStruct;

inline int useStatic (void) {
  staticFunction();
  (void)staticStruct.x;
  return staticVar;
}

extern inline int useStaticFromExtern (void) {
  staticFunction();
  return staticVar;
}

static inline int useStaticFromStatic (void) {
  staticFunction();
  return staticVar;
}

extern inline int useStaticInlineFromExtern (void) {




  return useStaticFromStatic();
}

static int constFunction(void) __attribute__((const));

inline int useConst (void) {
  return constFunction();
}
# 50 "Sema/inline.c" 2


inline int a;
typedef inline int b;
int d(inline int a);




inline int useStaticMainFile (void) {
  staticFunction();
  return staticVar;
}



#pragma clang diagnostic push
#pragma clang diagnostic warning "-Wstatic-in-inline"

inline int useStaticAgain (void) {
  staticFunction();
  return staticVar;
}

#pragma clang diagnostic pop

inline void defineStaticVar(void) {
  static const int x = 0;
  static int y = 0;
}

extern inline void defineStaticVarInExtern(void) {
  static const int x = 0;
  static int y = 0;
}


# 1 "XXX.h" 1
inline int useStaticMainFileInLineMarker(void) {
  staticFunction();
  return staticVar;
}
# 100 "inline.c" 2

inline int useStaticMainFileAfterLineMarker(void) {
  staticFunction();
  return staticVar;
}
