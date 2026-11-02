__attribute__((noinline)) unsigned probe(void) {
 unsigned count=0, sum=0;
 for(unsigned i=0;;i+=1073741824U) {
  if (count==0 || count>=3) {
   unsigned keep=i+1073741824U<1073741825U;
   sum+=keep*17;
   if(keep) { } else break;
  }
  ++count;
 }
 return (sum<<8)|count;
}
int main(void){return probe()!=13061;}
