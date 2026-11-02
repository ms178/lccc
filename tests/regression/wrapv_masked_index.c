/* Nonnegative masks bound signed results too; negative masks and additions
   which can overflow must not acquire a fictitious nonwrapping range. */
static volatile int data[16]={0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15};
__attribute__((noinline)) int bounded(void) {
    int sum=0;
    for(int i=2;i<=64;i++) sum+=data[(i&7)+1];
    return sum;
}
__attribute__((noinline)) long long boundary(void) {
    long long sum=0;
    for(int i=2147483643;i<=2147483646;i++) {
        int j=(i&2147483647)+4;
        sum+=(long long)j+data[(i&7)+1];
    }
    return sum;
}
int main(void) { return bounded()!=286 || boundary()!=-4294967272LL; }
