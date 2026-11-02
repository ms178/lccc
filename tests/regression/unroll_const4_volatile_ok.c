int main(void){volatile int sink=0;int s=0;for(int i=0;i<4;i++){s+=i;sink=s;}return(s==6&&sink==6)?0:1;}
