#define _GNU_SOURCE
#include <fenv.h>
#include <signal.h>
#include <unistd.h>
#include <float.h>
#ifndef NO_FENV_PRAGMA
#pragma STDC FENV_ACCESS ON
#endif
__attribute__((noinline)) double kernel(const double *x,int n) {
    double s=DBL_MAX;
    for(int j=0;j<n;j++) {
        int d=(j-1)*(j-1)*(j+1);
        s+=(1.0/(double)d)*x[j];
    }
    return s;
}
__attribute__((noinline)) double empty_loop(int n,double denominator) {
    double sum=4.0;
    for(int j=0;j<n;++j) sum += 1.0/denominator;
    return sum;
}
static void handler(int sig, siginfo_t *info, void *context) {
    (void)sig;(void)context;
    _exit(info->si_code==FPE_FLTOVF ? 0 : 40+info->si_code);
}
int main(void) {
    struct sigaction sa={0}; sa.sa_sigaction=handler;sa.sa_flags=SA_SIGINFO;
    sigemptyset(&sa.sa_mask); if(sigaction(SIGFPE,&sa,0))return 90;
    if(feenableexcept(FE_DIVBYZERO|FE_OVERFLOW)<0)return 91;
    volatile double zero=0.0;
    if(empty_loop(0,zero)!=4.0 || empty_loop(-3,zero)!=4.0)return 93;
#ifdef ONLY_EMPTY
    return 0;
#endif
    double x[4]={DBL_MAX,1,1,1};
    volatile double s=kernel(x,4);(void)s;return 92;
}
