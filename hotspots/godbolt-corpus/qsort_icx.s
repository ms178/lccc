.LCPI0_0:
  .long 2147483647
main:
  pushq %rax
  vstmxcsr 4(%rsp)
  orl $32832, 4(%rsp)
  vldmxcsr 4(%rsp)
  movl $42, %ecx
  movq $-4000000, %rax
  vpbroadcastd .LCPI0_0(%rip), %ymm0
.LBB0_1:
  imull $1664525, %ecx, %ecx
  addl $1013904223, %ecx
  imull $1664525, %ecx, %edx
  addl $1013904223, %edx
  imull $1664525, %edx, %esi
  addl $1013904223, %esi
  imull $1664525, %esi, %edi
  addl $1013904223, %edi
  imull $1664525, %edi, %r8d
  addl $1013904223, %r8d
  imull $1664525, %r8d, %r9d
  addl $1013904223, %r9d
  imull $1664525, %r9d, %r10d
  addl $1013904223, %r10d
  vmovd %ecx, %xmm1
  vpinsrd $1, %edx, %xmm1, %xmm1
  vpinsrd $2, %esi, %xmm1, %xmm1
  imull $1664525, %r10d, %ecx
  vpinsrd $3, %edi, %xmm1, %xmm1
  vmovd %r8d, %xmm2
  vpinsrd $1, %r9d, %xmm2, %xmm2
  vpinsrd $2, %r10d, %xmm2, %xmm2
  addl $1013904223, %ecx
  vpinsrd $3, %ecx, %xmm2, %xmm2
  vinserti128 $1, %xmm2, %ymm1, %ymm1
  vpand %ymm0, %ymm1, %ymm1
  vmovdqu %ymm1, arr+4000000(%rax)
  addq $32, %rax
  jne .LBB0_1
  movl $arr, %edi
  movl $1000000, %esi
  movl $4, %edx
  movl $cmp, %ecx
  vzeroupper
  callq qsort
  movl arr+2000000(%rip), %esi
  movl $.L.str, %edi
  xorl %eax, %eax
  callq printf
  xorl %eax, %eax
  popq %rcx
  retq

cmp:
  movl (%rdi), %eax
  subl (%rsi), %eax
  retq

.L.str:
  .asciz "qsort: arr[500000] = %d\n"

