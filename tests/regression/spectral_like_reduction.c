int main(void){double v[16];for(int i=0;i<16;i++)v[i]=1.0;double sum=0;
for(int j=0;j<16;j++)sum+=(1.0/(double)(j+1))*v[j];return(sum>3.38&&sum<3.381)?0:1;}
