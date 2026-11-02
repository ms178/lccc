int main(void){unsigned x=0;for(int i=0;i<7;i++)x^=(unsigned)(0xA5u+i);unsigned r=0;for(int i=0;i<7;i++)r^=(unsigned)(0xA5u+i);return x==r?0:1;}
