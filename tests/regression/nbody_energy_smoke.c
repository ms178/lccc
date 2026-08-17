/* Tiny nbody energy smoke: 2 bodies, one step, energy finite. */
#include <math.h>
typedef struct { double x,y,z,vx,vy,vz,mass; } B;
int main(void) {
    B a = {0,0,0,0,0,0,1.0};
    B b = {1,0,0,0,0,0,1.0};
    double dx=a.x-b.x, dy=a.y-b.y, dz=a.z-b.z;
    double d2=dx*dx+dy*dy+dz*dz;
    double e = 0.5*a.mass*(a.vx*a.vx) + 0.5*b.mass*(b.vx*b.vx)
             - a.mass*b.mass/sqrt(d2);
    return (e > -1.1 && e < -0.9) ? 0 : 1;
}
