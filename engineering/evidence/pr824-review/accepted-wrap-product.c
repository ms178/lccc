__attribute__((noinline)) int f(char *p, int start, int end) {
 int sum=0;
 for(int i=start;i<end;i++)sum+=p[i*4];
 return sum;
}
int main(void){char p[16]={1,0,0,0,2,0,0,0,3,0,0,0,4};return f(p,1073741824,1073741828)!=10;}
