long long sum_i64(const long long *a, int n){long long s=0;for(int i=0;i<n;i++)s+=a[i];return s;}int main(void){long long a[16];for(int i=0;i<16;i++)a[i]=i+1;return sum_i64(a,16)==136?0:1;}
