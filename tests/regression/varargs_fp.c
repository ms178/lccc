#include <stdio.h>
#include <stdarg.h>
double vs(int n, ...){ va_list ap; va_start(ap,n); double s=0; for(int i=0;i<n;i++)s+=va_arg(ap,double); va_end(ap); return s; }
long vi(int n, ...){ va_list ap; va_start(ap,n); long s=0; for(int i=0;i<n;i++)s+=va_arg(ap,int); va_end(ap); return s; }
int main(void){ printf("%.2f %ld\n", vs(3,1.5,2.5,3.0), vi(3,10,20,30));
  return (vs(3,1.5,2.5,3.0)==7.0 && vi(3,10,20,30)==60)?0:1; }
