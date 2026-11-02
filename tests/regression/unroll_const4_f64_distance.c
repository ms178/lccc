typedef struct{double x,y,z;}P;
static double dist2(P a,P b){double dx=a.x-b.x,dy=a.y-b.y,dz=a.z-b.z;return dx*dx+dy*dy+dz*dz;}
int main(void){P g[4];for(int i=0;i<4;i++){g[i].x=i;g[i].y=i*2.0;g[i].z=i*3.0;}
double e=0;for(int i=0;i<4;i++)for(int j=i+1;j<4;j++)e+=dist2(g[i],g[j]);
return(e>279.9&&e<280.1)?0:1;}
