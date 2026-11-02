// SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
// Origin: edgcpp/compiler tests/tests/imported/clang/c (Apache-2.0 WITH
// LLVM-exception).  Adapted into LCCC by scripts/edg_corpus_mine.py.
//type: fn
//options:  --c
# 1 "Sema/diagnose_if.c"
# 1 "<built-in>" 1
# 1 "<built-in>" 3
# 412 "<built-in>" 3
# 1 "<command line>" 1
# 1 "<built-in>" 2
# 1 "Sema/diagnose_if.c" 2





void failure1(void) __attribute__((diagnose_if()));
void failure2(void) __attribute__((diagnose_if(0)));
void failure3(void) __attribute__((diagnose_if(0, "")));
void failure4(void) __attribute__((diagnose_if(0, "", "error", 1)));
void failure5(void) __attribute__((diagnose_if(0, 0, "error")));
void failure6(void) __attribute__((diagnose_if(0, "", "invalid")));
void failure7(void) __attribute__((diagnose_if(0, "", "ERROR")));
void failure8(int a) __attribute__((diagnose_if(a, "", "")));
void failure9(void) __attribute__((diagnose_if(a, "", "")));

int globalVar;
void never_constant(void) __attribute__((diagnose_if(globalVar, "", "error")));
void never_constant(void) __attribute__((diagnose_if(globalVar, "", "warning")));

int alwaysok(int q) __attribute__((diagnose_if(0, "", "error")));
int neverok(int q) __attribute__((diagnose_if(1, "oh no", "error")));
int alwayswarn(int q) __attribute__((diagnose_if(1, "oh no", "warning")));
int neverwarn(int q) __attribute__((diagnose_if(0, "", "warning")));

void runConstant(void) {
  int m;
  alwaysok(0);
  alwaysok(1);
  alwaysok(m);

  {
    int (*pok)(int) = alwaysok;
    pok = &alwaysok;
  }

  neverok(0);
  neverok(1);
  neverok(m);
  {
    int (*pok)(int) = neverok;
    pok = &neverok;
  }

  alwayswarn(0);
  alwayswarn(1);
  alwayswarn(m);
  {
    int (*pok)(int) = alwayswarn;
    pok = &alwayswarn;
  }

  neverwarn(0);
  neverwarn(1);
  neverwarn(m);
  {
    int (*pok)(int) = neverwarn;
    pok = &neverwarn;
  }
}

int abs(int q) __attribute__((diagnose_if(q >= 0, "redundant abs call", "error")));
void runVariable(void) {
  int m;
  abs(-1);
  abs(1);
  abs(m);

  int (*pabs)(int) = abs;
  pabs = &abs;
}



int ovl1(const char *n) __attribute__((overloadable)) __attribute__((diagnose_if(n, "oh no", "error")));
int ovl1(void *m) __attribute__((overloadable));

int ovl2(const char *n) __attribute__((overloadable)) __attribute__((diagnose_if(n, "oh no", "error")));
int ovl2(char *m) __attribute__((overloadable));
void overloadsYay(void) {
  ovl1((void *)0);
  ovl1("");

  ovl2((void *)0);
}

void errorWarnDiagnose1(void) __attribute__((diagnose_if(1, "oh no", "error")))
  __attribute__((diagnose_if(1, "nop", "warning")));
void errorWarnDiagnose2(void) __attribute__((diagnose_if(1, "oh no", "error")))
  __attribute__((diagnose_if(1, "nop", "error")));
void errorWarnDiagnose3(void) __attribute__((diagnose_if(1, "nop", "warning")))
  __attribute__((diagnose_if(1, "oh no", "error")));

void errorWarnDiagnoseArg1(int a) __attribute__((diagnose_if(a == 1, "oh no", "error")))
  __attribute__((diagnose_if(a == 1, "nop", "warning")));
void errorWarnDiagnoseArg2(int a) __attribute__((diagnose_if(a == 1, "oh no", "error")))
  __attribute__((diagnose_if(a == 1, "nop", "error")));
void errorWarnDiagnoseArg3(int a) __attribute__((diagnose_if(a == 1, "nop", "warning")))
  __attribute__((diagnose_if(a == 1, "oh no", "error")));

void runErrorWarnDiagnose(void) {
  errorWarnDiagnose1();
  errorWarnDiagnose2();
  errorWarnDiagnose3();

  errorWarnDiagnoseArg1(1);
  errorWarnDiagnoseArg2(1);
  errorWarnDiagnoseArg3(1);
}

void warnWarnDiagnose(void) __attribute__((diagnose_if(1, "oh no!", "warning"))) __attribute__((diagnose_if(1, "foo", "warning")));
void runWarnWarnDiagnose(void) {
  warnWarnDiagnose();
}

void declsStackErr1(int a) __attribute__((diagnose_if(a & 1, "decl1", "error")));
void declsStackErr1(int a) __attribute__((diagnose_if(a & 2, "decl2", "error")));
void declsStackErr2(void);
void declsStackErr2(void) __attribute__((diagnose_if(1, "complaint", "error")));
void declsStackErr3(void) __attribute__((diagnose_if(1, "complaint", "error")));
void declsStackErr3(void);
void runDeclsStackErr(void) {
  declsStackErr1(0);
  declsStackErr1(1);
  declsStackErr1(2);
  declsStackErr1(3);
  declsStackErr2();
  declsStackErr3();
}

void declsStackWarn1(int a) __attribute__((diagnose_if(a & 1, "decl1", "warning")));
void declsStackWarn1(int a) __attribute__((diagnose_if(a & 2, "decl2", "warning")));
void declsStackWarn2(void);
void declsStackWarn2(void) __attribute__((diagnose_if(1, "complaint", "warning")));
void declsStackWarn3(void) __attribute__((diagnose_if(1, "complaint", "warning")));
void declsStackWarn3(void);
void runDeclsStackWarn(void) {
  declsStackWarn1(0);
  declsStackWarn1(1);
  declsStackWarn1(2);
  declsStackWarn1(3);
  declsStackWarn2();
  declsStackWarn3();
}

void noMsg(int n) __attribute__((diagnose_if(n, "", "warning")));
void runNoMsg(void) {
  noMsg(1);
}

void alwaysWarnWithArg(int a) __attribute__((diagnose_if(1 || a, "alwaysWarn", "warning")));
void runAlwaysWarnWithArg(int a) {
  alwaysWarnWithArg(a);
}


# 1 "Sema/Inputs/diagnose-if-warn-system-header.h" 1
# 2 "Sema/Inputs/diagnose-if-warn-system-header.h" 3

inline int system_header_func(int x)
  __attribute__((diagnose_if(x == x, "system header warning", "warning")))
{
  return 0;
}

void test_system_header(void) {
  system_header_func(0);
}
# 157 "Sema/diagnose_if.c" 2



void underbarName(int a) __attribute__((__diagnose_if__(a, "", "warning")));


void constCharStar(const char *str) __attribute__((__diagnose_if__(!str[0], "empty string not allowed", "error")));
void charStar(char *str) __attribute__((__diagnose_if__(!str[0], "empty string not allowed", "error")));
void runConstCharStar(void) {
  constCharStar("foo");
  charStar("bar");
  constCharStar("");
  charStar("");
}
