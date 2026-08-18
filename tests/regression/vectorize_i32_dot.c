int dot_i32(const int *a,const int *b,int n){int s=0;for(int i=0;i<n;i++)s+=a[i]*b[i];return s;}int main(void){int a[8]={1,2,3,4,5,6,7,8};int b[8]={2,2,2,2,2,2,2,2};return dot_i32(a,b,8)==72?0:1;}
