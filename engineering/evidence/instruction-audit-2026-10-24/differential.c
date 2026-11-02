#define CAT_(a,b) a##b
#define CAT(a,b) CAT_(a,b)
#ifndef PREFIX
#define PREFIX lccc_
#endif
typedef unsigned d4 __attribute__((vector_size(16)));
d4 CAT(PREFIX,d4sl)(d4 a,unsigned n){return a << n;}
d4 CAT(PREFIX,d4sr)(d4 a,unsigned n){return a >> n;}
typedef int signed_d4 __attribute__((vector_size(16)));
d4 CAT(PREFIX,d4sa)(d4 a,unsigned n){return (d4)(((signed_d4)a) >> n);}
typedef unsigned d8 __attribute__((vector_size(32)));
d8 CAT(PREFIX,d8sl)(d8 a,unsigned n){return a << n;}
d8 CAT(PREFIX,d8sr)(d8 a,unsigned n){return a >> n;}
typedef int signed_d8 __attribute__((vector_size(32)));
d8 CAT(PREFIX,d8sa)(d8 a,unsigned n){return (d8)(((signed_d8)a) >> n);}
typedef unsigned long long q2 __attribute__((vector_size(16)));
q2 CAT(PREFIX,q2sl)(q2 a,unsigned n){return a << n;}
q2 CAT(PREFIX,q2sr)(q2 a,unsigned n){return a >> n;}
typedef unsigned long long q4 __attribute__((vector_size(32)));
q4 CAT(PREFIX,q4sl)(q4 a,unsigned n){return a << n;}
q4 CAT(PREFIX,q4sr)(q4 a,unsigned n){return a >> n;}
