#define _GNU_SOURCE
#include <stdio.h>
#include <math.h>
/* Force our builtins (not libm calls) */
double F(double x){return __builtin_floor(x);}
double C(double x){return __builtin_ceil(x);}
double T(double x){return __builtin_trunc(x);}
double R(double x){return __builtin_rint(x);}
double N(double x){return __builtin_nearbyint(x);}
double E(double x){return __builtin_roundeven(x);}
double CS(double x,double y){return __builtin_copysign(x,y);}
float Ff(float x){return __builtin_floorf(x);}
float Tf(float x){return __builtin_truncf(x);}
float Ef(float x){return __builtin_roundevenf(x);}
float CSf(float x,float y){return __builtin_copysignf(x,y);}
typedef union { double d; unsigned long long u; } B;
static int eq(double a, double b){ B x={.d=a},y={.d=b}; return x.u==y.u || (a!=a && b!=b); }
int main(void){
  double vals[] = {0.0,-0.0,0.5,-0.5,1.5,2.5,-2.5,3.5,-3.5,0.49999999999999994,
                   2.4999999999999996, 4503599627370495.5, -4503599627370496.0,
                   1e300,-1e300, 1.0/0.0, -1.0/0.0, 0.0/0.0, 123456.789,-0.25};
  int n = sizeof vals/sizeof *vals, ok=1;
  for (int i=0;i<n;i++){ double x=vals[i];
    ok &= eq(F(x),floor(x)); ok &= eq(C(x),ceil(x)); ok &= eq(T(x),trunc(x));
    ok &= eq(R(x),rint(x));  ok &= eq(N(x),nearbyint(x));
    ok &= eq(E(x),roundeven(x));
    ok &= eq(CS(x,-3.0),copysign(x,-3.0)); ok &= eq(CS(x,+3.0),copysign(x,3.0));
    float xf=(float)x;
    ok &= eq(Ff(xf),floorf(xf)); ok &= eq(Tf(xf),truncf(xf));
    ok &= eq(Ef(xf),roundevenf(xf));
    ok &= eq(CSf(xf,-2.0f),copysignf(xf,-2.0f));
  }
  printf("round-family:%s\n", ok?"ok":"MISMATCH");
  return ok?0:1;
}
