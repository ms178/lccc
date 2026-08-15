// Vectorizer profitability gate: constant trip counts below 2x the vector
// width must stay scalar (one vector iteration + setup + horizontal combine
// + remainder is a net loss), while larger and dynamic trip counts still
// vectorize. All sums verified against GCC output byte-for-byte.
#include <stdio.h>

double sum4(double *a){ double s=0; for(int i=0;i<4;i++) s+=a[i]; return s; }
double sum7(double *a){ double s=0; for(int i=0;i<7;i++) s+=a[i]; return s; }
double sum8(double *a){ double s=0; for(int i=0;i<8;i++) s+=a[i]; return s; }
double sum9(double *a){ double s=0; for(int i=0;i<9;i++) s+=a[i]; return s; }
double sumN(double *a,int n){ double s=0; for(int i=0;i<n;i++) s+=a[i]; return s; }

int main(void){
    double a[32];
    for(int i=0;i<32;i++) a[i]=i*0.25+0.5;
    printf("%.2f %.2f %.2f %.2f\n", sum4(a), sum7(a), sum8(a), sum9(a));
    /* dynamic trip counts, incl. below-width and remainder-heavy */
    for (int n = 0; n <= 13; n++) printf("%.2f ", sumN(a, n));
    printf("\n");
    return 0;
}
