main:
..B1.1: # Preds ..B1.0
  pushq %rbp #80.1
  movq %rsp, %rbp #80.1
  andq $-128, %rsp #80.1
  pushq %r12 #80.1
  pushq %r13 #80.1
  pushq %r14 #80.1
  pushq %r15 #80.1
  pushq %rbx #80.1
  subq $216, %rsp #80.1
  movl $3, %edi #80.1
  xorl %esi, %esi #80.1
  call __intel_new_feature_proc_init #80.1
..B1.17: # Preds ..B1.1
  movdqu test_in.146.0.2(%rip), %xmm2 #71.27
  movdqu 16+test_in.146.0.2(%rip), %xmm3 #71.27
  movdqa %xmm2, %xmm6 #40.5
  movdqu 48+test_in.146.0.2(%rip), %xmm4 #71.27
  movdqa %xmm3, %xmm7 #40.5
  psrldq $4, %xmm6 #40.5
  movdqa %xmm4, %xmm8 #40.5
  psrldq $4, %xmm7 #40.5
  movdqa %xmm2, %xmm10 #40.5
  psrldq $4, %xmm8 #40.5
  movdqa %xmm3, %xmm11 #40.5
  movd %xmm6, %ebx #45.5
  movdqa %xmm4, %xmm12 #40.5
  movd %xmm7, %esi #45.5
  movdqa %xmm2, %xmm14 #40.5
  movdqu 32+test_in.146.0.2(%rip), %xmm5 #71.27
  movdqa %xmm3, %xmm15 #40.5
  psrldq $8, %xmm10 #40.5
  movdqa %xmm5, %xmm9 #40.5
  psrldq $8, %xmm11 #40.5
  addl %esi, %ebx #45.5
  movd %xmm8, %r10d #45.5
  movdqa %xmm5, %xmm13 #40.5
  psrldq $8, %xmm12 #40.5
  movdqa %xmm4, %xmm0 #40.5
  psrldq $4, %xmm9 #40.5
  movdqa %xmm5, %xmm1 #40.5
  movd %xmm10, %r9d #46.5
  xorl %ebx, %r10d #45.5
  movd %xmm11, %r8d #46.5
  movd %xmm12, %r15d #46.5
  movd %xmm9, %r12d #45.5
  shldl $16, %r10d, %r10d #45.5
  movd %xmm2, %eax #44.5
  psrldq $8, %xmm13 #40.5
  psrldq $12, %xmm14 #40.5
  addl %r8d, %r9d #46.5
  addl %r10d, %r12d #45.5
  xorl %r9d, %r15d #46.5
  xorl %r12d, %esi #45.5
  movd %xmm13, %edi #46.5
  shldl $16, %r15d, %r15d #46.5
  shldl $12, %esi, %esi #45.5
  movd %xmm14, %r13d #47.5
  psrldq $12, %xmm15 #40.5
  psrldq $12, %xmm0 #40.5
  addl %r15d, %edi #46.5
  addl %esi, %ebx #45.5
  xorl %edi, %r8d #46.5
  xorl %ebx, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  movd %xmm3, %edx #44.5
  movd %xmm15, 96(%rsp) #47.5[spill]
  movd %xmm0, 80(%rsp) #47.5[spill]
  addl %r8d, %r9d #46.5
  addl %r10d, %r12d #45.5
  xorl %r9d, %r15d #46.5
  xorl %r12d, %esi #45.5
  shldl $8, %r15d, %r15d #46.5
  shldl $7, %esi, %esi #45.5
  movd %xmm4, %ecx #44.5
  psrldq $12, %xmm1 #40.5
  movl %r12d, 88(%rsp) #45.5[spill]
  addl %r15d, %edi #46.5
  movl 96(%rsp), %r12d #47.5[spill]
  addl %r12d, %r13d #47.5
  movl %r15d, 104(%rsp) #46.5[spill]
  addl %edx, %eax #44.5
  movl 80(%rsp), %r15d #47.5[spill]
  xorl %eax, %ecx #44.5
  xorl %r13d, %r15d #47.5
  xorl %edi, %r8d #46.5
  movd %xmm1, %r14d #47.5
  shldl $16, %r15d, %r15d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %r8d, %r8d #46.5
  movd %xmm5, %r11d #44.5
  addl %r15d, %r14d #47.5
  addl %ecx, %r11d #44.5
  xorl %r14d, %r12d #47.5
  xorl %r11d, %edx #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %edx, %edx #44.5
  stmxcsr 8(%rsp) #80.1
  addl %r12d, %r13d #47.5
  addl %edx, %eax #44.5
  xorl %r13d, %r15d #47.5
  xorl %eax, %ecx #44.5
  shldl $8, %r15d, %r15d #47.5
  shldl $8, %ecx, %ecx #44.5
  addl %esi, %eax #50.5
  addl %r15d, %r14d #47.5
  xorl %eax, %r15d #50.5
  addl %r8d, %ebx #51.5
  shldl $16, %r15d, %r15d #50.5
  addl %ecx, %r11d #44.5
  xorl %ebx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r15d, %edi #50.5
  xorl %r14d, %r12d #47.5
  xorl %edi, %esi #50.5
  addl %ecx, %r14d #51.5
  shldl $12, %esi, %esi #50.5
  shldl $7, %r12d, %r12d #47.5
  xorl %r14d, %r8d #51.5
  addl %esi, %eax #50.5
  shldl $12, %r8d, %r8d #51.5
  xorl %r11d, %edx #44.5
  xorl %eax, %r15d #50.5
  shldl $7, %edx, %edx #44.5
  shldl $8, %r15d, %r15d #50.5
  addl %r12d, %r9d #52.5
  addl %r8d, %ebx #51.5
  xorl %r9d, %r10d #52.5
  xorl %ebx, %ecx #51.5
  addl %edx, %r13d #53.5
  addl %r15d, %edi #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %ecx, %ecx #51.5
  movl %r15d, 80(%rsp) #50.5[spill]
  addl %r10d, %r11d #52.5
  movl 104(%rsp), %r15d #53.5[spill]
  addl %ecx, %r14d #51.5
  xorl %r13d, %r15d #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  movl %r14d, 112(%rsp) #51.5[spill]
  xorl %r14d, %r8d #51.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  shldl $7, %r8d, %r8d #51.5
  addl %edx, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %ebx, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %edx, %edx #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %esi, %ebx #45.5
  addl %r15d, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %edx, %eax #44.5
  addl %r10d, %r14d #45.5
  xorl %eax, %ecx #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %ecx, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %edx #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %edx, %edx #44.5
  movl %r15d, 104(%rsp) #46.5[spill]
  addl %r15d, %edi #46.5
  movl 112(%rsp), %r15d #47.5[spill]
  addl %edx, %eax #44.5
  addl %r14d, %r15d #47.5
  xorl %edi, %r8d #46.5
  xorl %r15d, %r12d #47.5
  xorl %eax, %ecx #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %ebx #51.5
  xorl %r13d, %r14d #47.5
  addl %ecx, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %ebx, %ecx #51.5
  addl %r14d, %r15d #47.5
  shldl $16, %ecx, %ecx #51.5
  xorl %r15d, %r12d #47.5
  addl %esi, %eax #50.5
  addl %ecx, %r15d #51.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r8d #51.5
  xorl %r11d, %edx #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r14d, %edi #50.5
  addl %r8d, %ebx #51.5
  xorl %edi, %esi #50.5
  xorl %ebx, %ecx #51.5
  shldl $12, %esi, %esi #50.5
  shldl $8, %ecx, %ecx #51.5
  addl %r12d, %r9d #52.5
  addl %esi, %eax #50.5
  xorl %r9d, %r10d #52.5
  addl %ecx, %r15d #51.5
  addl %edx, %r13d #53.5
  xorl %eax, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %r15d, 112(%rsp) #51.5[spill]
  xorl %r15d, %r8d #51.5
  movl 104(%rsp), %r15d #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %r15d #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %edi #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %ebx, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %edx, %edx #53.5
  addl %esi, %ebx #45.5
  addl %r15d, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %ecx #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %r15d, 104(%rsp) #46.5[spill]
  addl %r15d, %edi #46.5
  movl 112(%rsp), %r15d #47.5[spill]
  addl %ecx, %r11d #44.5
  addl %r14d, %r15d #47.5
  xorl %r11d, %edx #44.5
  xorl %r15d, %r12d #47.5
  xorl %edi, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %edx, %edx #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %ecx #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %ecx, %ecx #44.5
  addl %esi, %eax #50.5
  addl %r14d, %r15d #47.5
  xorl %eax, %r14d #50.5
  addl %r8d, %ebx #51.5
  shldl $16, %r14d, %r14d #50.5
  addl %ecx, %r11d #44.5
  xorl %ebx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %edi #50.5
  xorl %r15d, %r12d #47.5
  xorl %edi, %esi #50.5
  addl %ecx, %r15d #51.5
  shldl $12, %esi, %esi #50.5
  shldl $7, %r12d, %r12d #47.5
  xorl %r15d, %r8d #51.5
  addl %esi, %eax #50.5
  shldl $12, %r8d, %r8d #51.5
  xorl %r11d, %edx #44.5
  xorl %eax, %r14d #50.5
  shldl $7, %edx, %edx #44.5
  shldl $8, %r14d, %r14d #50.5
  addl %r8d, %ebx #51.5
  addl %edx, %r13d #53.5
  xorl %ebx, %ecx #51.5
  addl %r14d, %edi #50.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r12d, %r9d #52.5
  shldl $8, %ecx, %ecx #51.5
  movl 104(%rsp), %r14d #53.5[spill]
  addl %ecx, %r15d #51.5
  xorl %r13d, %r14d #53.5
  xorl %r15d, %r8d #51.5
  shldl $16, %r14d, %r14d #53.5
  shldl $7, %r8d, %r8d #51.5
  movl %r15d, 112(%rsp) #51.5[spill]
  xorl %r9d, %r10d #52.5
  movl 88(%rsp), %r15d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %r14d, %r15d #53.5
  xorl %r15d, %edx #53.5
  shldl $12, %edx, %edx #53.5
  shldl $16, %r10d, %r10d #52.5
  shldl $7, %esi, %esi #50.5
  addl %edx, %r13d #53.5
  addl %r10d, %r11d #52.5
  xorl %r13d, %r14d #53.5
  xorl %r11d, %r12d #52.5
  shldl $8, %r14d, %r14d #53.5
  shldl $12, %r12d, %r12d #52.5
  addl %r14d, %r15d #53.5
  addl %r12d, %r9d #52.5
  xorl %r15d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $7, %edx, %edx #53.5
  shldl $8, %r10d, %r10d #52.5
  addl %esi, %ebx #45.5
  addl %edx, %eax #44.5
  addl %r10d, %r11d #52.5
  xorl %ebx, %r10d #45.5
  xorl %eax, %ecx #44.5
  xorl %r11d, %r12d #52.5
  shldl $16, %r10d, %r10d #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %r12d, %r12d #52.5
  addl %r10d, %r15d #45.5
  addl %ecx, %r11d #44.5
  xorl %r15d, %esi #45.5
  xorl %r11d, %edx #44.5
  shldl $12, %esi, %esi #45.5
  shldl $12, %edx, %edx #44.5
  addl %esi, %ebx #45.5
  addl %r8d, %r9d #46.5
  addl %edx, %eax #44.5
  xorl %ebx, %r10d #45.5
  xorl %r9d, %r14d #46.5
  xorl %eax, %ecx #44.5
  shldl $8, %r10d, %r10d #45.5
  shldl $16, %r14d, %r14d #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r10d, %r15d #45.5
  addl %r14d, %edi #46.5
  addl %r12d, %r13d #47.5
  addl %ecx, %r11d #44.5
  movl %r15d, 88(%rsp) #45.5[spill]
  xorl %r15d, %esi #45.5
  movl 80(%rsp), %r15d #47.5[spill]
  xorl %edi, %r8d #46.5
  xorl %r13d, %r15d #47.5
  xorl %r11d, %edx #44.5
  shldl $12, %r8d, %r8d #46.5
  shldl $16, %r15d, %r15d #47.5
  shldl $7, %edx, %edx #44.5
  shldl $7, %esi, %esi #45.5
  movl %edx, (%rsp) #44.5[spill]
  addl %r8d, %r9d #46.5
  movl 112(%rsp), %edx #47.5[spill]
  xorl %r9d, %r14d #46.5
  addl %r15d, %edx #47.5
  addl %esi, %eax #50.5
  xorl %edx, %r12d #47.5
  shldl $8, %r14d, %r14d #46.5
  shldl $12, %r12d, %r12d #47.5
  addl %r14d, %edi #46.5
  addl %r12d, %r13d #47.5
  xorl %edi, %r8d #46.5
  xorl %r13d, %r15d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %r15d, %r15d #47.5
  addl %r15d, %edx #47.5
  xorl %eax, %r15d #50.5
  addl %r8d, %ebx #51.5
  xorl %edx, %r12d #47.5
  shldl $16, %r15d, %r15d #50.5
  shldl $7, %r12d, %r12d #47.5
  xorl %ebx, %ecx #51.5
  addl %r15d, %edi #50.5
  shldl $16, %ecx, %ecx #51.5
  xorl %edi, %esi #50.5
  addl %ecx, %edx #51.5
  shldl $12, %esi, %esi #50.5
  xorl %edx, %r8d #51.5
  addl %esi, %eax #50.5
  shldl $12, %r8d, %r8d #51.5
  xorl %eax, %r15d #50.5
  addl %r8d, %ebx #51.5
  shldl $8, %r15d, %r15d #50.5
  xorl %ebx, %ecx #51.5
  addl %r15d, %edi #50.5
  shldl $8, %ecx, %ecx #51.5
  orl $32832, 8(%rsp) #80.1
  xorl %edi, %esi #50.5
  addl %ecx, %edx #51.5
  ldmxcsr 8(%rsp) #80.1
  movq $0, 64(%rsp) #83.16[spill]
  xorl %edx, %r8d #51.5
  movl %eax, 72(%rsp) #50.5[spill]
  movl %r15d, 80(%rsp) #50.5[spill]
  shldl $7, %esi, %esi #50.5
..B1.16: # Preds ..B1.17
  movl (%rsp), %eax #53.5[spill]
  addl %eax, %r13d #53.5
  xorl %r13d, %r14d #53.5
  addl %r12d, %r9d #52.5
  shldl $16, %r14d, %r14d #53.5
  shldl $7, %r8d, %r8d #51.5
  xorl %r9d, %r10d #52.5
  addl %esi, %ebx #45.5
  movl %edx, 112(%rsp) #[spill]
  movl 88(%rsp), %edx #53.5[spill]
  shldl $16, %r10d, %r10d #52.5
  addl %r14d, %edx #53.5
  addl %r10d, %r11d #52.5
  xorl %edx, %eax #53.5
  xorl %r11d, %r12d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $12, %r12d, %r12d #52.5
  addl %eax, %r13d #53.5
  addl %r12d, %r9d #52.5
  xorl %r13d, %r14d #53.5
  xorl %r9d, %r10d #52.5
  shldl $8, %r14d, %r14d #53.5
  shldl $8, %r10d, %r10d #52.5
  addl %r8d, %r9d #46.5
  addl %r14d, %edx #53.5
  xorl %r9d, %r14d #46.5
  addl %r10d, %r11d #52.5
  shldl $16, %r14d, %r14d #46.5
  xorl %ebx, %r10d #45.5
  addl %r14d, %edi #46.5
  shldl $16, %r10d, %r10d #45.5
  xorl %edi, %r8d #46.5
  xorl %edx, %eax #53.5
  addl %r10d, %edx #45.5
  xorl %r11d, %r12d #52.5
  shldl $12, %r8d, %r8d #46.5
  shldl $7, %eax, %eax #53.5
  shldl $7, %r12d, %r12d #52.5
  xorl %edx, %esi #45.5
  addl %r8d, %r9d #46.5
  shldl $12, %esi, %esi #45.5
  movl 72(%rsp), %r15d #44.5[spill]
  xorl %r9d, %r14d #46.5
  addl %eax, %r15d #44.5
  addl %esi, %ebx #45.5
  shldl $8, %r14d, %r14d #46.5
  xorl %r15d, %ecx #44.5
  xorl %ebx, %r10d #45.5
  addl %r12d, %r13d #47.5
  addl %r14d, %edi #46.5
  shldl $16, %ecx, %ecx #44.5
  shldl $8, %r10d, %r10d #45.5
  movl %r14d, 104(%rsp) #46.5[spill]
  addl %ecx, %r11d #44.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %r10d, %edx #45.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %eax, %eax #44.5
  movl %edx, 88(%rsp) #45.5[spill]
  xorl %edx, %esi #45.5
  movl 112(%rsp), %edx #47.5[spill]
  addl %eax, %r15d #44.5
  addl %r14d, %edx #47.5
  xorl %edi, %r8d #46.5
  xorl %edx, %r12d #47.5
  xorl %r15d, %ecx #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  addl %r12d, %r13d #47.5
  addl %r8d, %ebx #51.5
  xorl %r13d, %r14d #47.5
  addl %ecx, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %ebx, %ecx #51.5
  addl %r14d, %edx #47.5
  shldl $16, %ecx, %ecx #51.5
  xorl %edx, %r12d #47.5
  addl %esi, %r15d #50.5
  addl %ecx, %edx #51.5
  xorl %r15d, %r14d #50.5
  xorl %edx, %r8d #51.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %edi #50.5
  addl %r8d, %ebx #51.5
  xorl %edi, %esi #50.5
  xorl %ebx, %ecx #51.5
  shldl $12, %esi, %esi #50.5
  shldl $8, %ecx, %ecx #51.5
  addl %r12d, %r9d #52.5
  addl %esi, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %ecx, %edx #51.5
  addl %eax, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %edx, 112(%rsp) #51.5[spill]
  xorl %edx, %r8d #51.5
  movl 104(%rsp), %edx #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %edx #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %edx, %edx #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %edi #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %edx, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %eax, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %edx #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %edx, %edx #53.5
  xorl %ebx, %r10d #45.5
  addl %edx, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %eax #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %edx #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %edx, %edx #46.5
  shldl $7, %eax, %eax #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %esi, %ebx #45.5
  addl %edx, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %eax, %r15d #44.5
  addl %r10d, %r14d #45.5
  xorl %r15d, %ecx #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %edx #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %ecx, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %eax #44.5
  shldl $8, %edx, %edx #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %eax, %eax #44.5
  movl %edx, 104(%rsp) #46.5[spill]
  addl %edx, %edi #46.5
  movl 112(%rsp), %edx #47.5[spill]
  addl %eax, %r15d #44.5
  addl %r14d, %edx #47.5
  xorl %edi, %r8d #46.5
  xorl %edx, %r12d #47.5
  xorl %r15d, %ecx #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %ebx #51.5
  xorl %r13d, %r14d #47.5
  addl %ecx, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %ebx, %ecx #51.5
  addl %r14d, %edx #47.5
  shldl $16, %ecx, %ecx #51.5
  xorl %edx, %r12d #47.5
  addl %esi, %r15d #50.5
  addl %ecx, %edx #51.5
  xorl %r15d, %r14d #50.5
  xorl %edx, %r8d #51.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %edi #50.5
  addl %r8d, %ebx #51.5
  xorl %edi, %esi #50.5
  xorl %ebx, %ecx #51.5
  shldl $12, %esi, %esi #50.5
  shldl $8, %ecx, %ecx #51.5
  addl %r12d, %r9d #52.5
  addl %esi, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %ecx, %edx #51.5
  addl %eax, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %edx, 112(%rsp) #51.5[spill]
  xorl %edx, %r8d #51.5
  movl 104(%rsp), %edx #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %edx #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %edx, %edx #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %edi #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %edx, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %eax, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %edx #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %edx, %edx #53.5
  xorl %ebx, %r10d #45.5
  addl %edx, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %eax #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %edx #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %edx, %edx #46.5
  shldl $7, %eax, %eax #53.5
  shldl $7, %r12d, %r12d #52.5
  addl %esi, %ebx #45.5
  addl %edx, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %eax, %r15d #44.5
  addl %r10d, %r14d #45.5
  xorl %r15d, %ecx #44.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %edx #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %ecx, %r11d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r11d, %eax #44.5
  shldl $8, %edx, %edx #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $12, %eax, %eax #44.5
  movl %edx, 104(%rsp) #46.5[spill]
  addl %edx, %edi #46.5
  movl 112(%rsp), %edx #47.5[spill]
  addl %eax, %r15d #44.5
  addl %r14d, %edx #47.5
  xorl %edi, %r8d #46.5
  xorl %edx, %r12d #47.5
  xorl %r15d, %ecx #44.5
  shldl $12, %r12d, %r12d #47.5
  shldl $7, %r8d, %r8d #46.5
  shldl $8, %ecx, %ecx #44.5
  addl %r12d, %r13d #47.5
  addl %r8d, %ebx #51.5
  xorl %r13d, %r14d #47.5
  addl %ecx, %r11d #44.5
  shldl $8, %r14d, %r14d #47.5
  xorl %ebx, %ecx #51.5
  addl %r14d, %edx #47.5
  shldl $16, %ecx, %ecx #51.5
  xorl %edx, %r12d #47.5
  addl %esi, %r15d #50.5
  addl %ecx, %edx #51.5
  xorl %r15d, %r14d #50.5
  xorl %edx, %r8d #51.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %edi #50.5
  addl %r8d, %ebx #51.5
  xorl %edi, %esi #50.5
  xorl %ebx, %ecx #51.5
  shldl $12, %esi, %esi #50.5
  shldl $8, %ecx, %ecx #51.5
  addl %r12d, %r9d #52.5
  addl %esi, %r15d #50.5
  xorl %r9d, %r10d #52.5
  addl %ecx, %edx #51.5
  addl %eax, %r13d #53.5
  xorl %r15d, %r14d #50.5
  shldl $16, %r10d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  movl %edx, 112(%rsp) #51.5[spill]
  xorl %edx, %r8d #51.5
  movl 104(%rsp), %edx #53.5[spill]
  addl %r10d, %r11d #52.5
  xorl %r13d, %edx #53.5
  xorl %r11d, %r12d #52.5
  shldl $16, %edx, %edx #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %edi #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %edx, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %eax #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %eax, %eax #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %eax, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %edx #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %edx, %edx #53.5
  xorl %ebx, %r10d #45.5
  addl %edx, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %eax #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %edx #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %edx, %edx #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %eax, %eax #53.5
  addl %esi, %ebx #45.5
  addl %edx, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %edx #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %eax, %r15d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r15d, %ecx #44.5
  shldl $8, %edx, %edx #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %edx, 104(%rsp) #46.5[spill]
  addl %edx, %edi #46.5
  movl 112(%rsp), %edx #47.5[spill]
  addl %ecx, %r11d #44.5
  addl %r14d, %edx #47.5
  xorl %r11d, %eax #44.5
  xorl %edx, %r12d #47.5
  xorl %edi, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %eax, %eax #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %eax, %r15d #44.5
  xorl %r13d, %r14d #47.5
  xorl %r15d, %ecx #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %ecx, %ecx #44.5
  addl %r14d, %edx #47.5
  addl %esi, %r15d #50.5
  xorl %edx, %r12d #47.5
  addl %r8d, %ebx #51.5
  shldl $7, %r12d, %r12d #47.5
  addl %r12d, %r9d #52.5
  addl %ecx, %r11d #44.5
  xorl %r15d, %r14d #50.5
  xorl %ebx, %ecx #51.5
  xorl %r9d, %r10d #52.5
  xorl %r11d, %eax #44.5
  shldl $16, %r14d, %r14d #50.5
  shldl $16, %ecx, %ecx #51.5
  shldl $16, %r10d, %r10d #52.5
  shldl $7, %eax, %eax #44.5
  addl %r14d, %edi #50.5
  addl %ecx, %edx #51.5
  addl %r10d, %r11d #52.5
  xorl %edi, %esi #50.5
  xorl %edx, %r8d #51.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $12, %r12d, %r12d #52.5
  addl %esi, %r15d #50.5
  addl %r8d, %ebx #51.5
  addl %r12d, %r9d #52.5
  xorl %r15d, %r14d #50.5
  xorl %ebx, %ecx #51.5
  xorl %r9d, %r10d #52.5
  shldl $8, %r14d, %r14d #50.5
  shldl $8, %ecx, %ecx #51.5
  shldl $8, %r10d, %r10d #52.5
  addl %r14d, %edi #50.5
  addl %ecx, %edx #51.5
  addl %r10d, %r11d #52.5
  addl %eax, %r13d #53.5
  movl %r14d, 80(%rsp) #50.5[spill]
  xorl %edi, %esi #50.5
  movl 104(%rsp), %r14d #53.5[spill]
  xorl %edx, %r8d #51.5
  xorl %r11d, %r12d #52.5
  xorl %r13d, %r14d #53.5
  movl %eax, (%rsp) #44.5[spill]
  movl %r15d, 72(%rsp) #50.5[spill]
  shldl $7, %esi, %esi #50.5
  shldl $7, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #52.5
  shldl $16, %r14d, %r14d #53.5
..B1.15: # Preds ..B1.16
  movl 88(%rsp), %r15d #53.5[spill]
  addl %r8d, %r9d #46.5
  addl %r14d, %r15d #53.5
  addl %esi, %ebx #45.5
  movl %edx, 112(%rsp) #[spill]
  xorl %ebx, %r10d #45.5
  movl %eax, %edx #53.5
  xorl %r15d, %edx #53.5
  shldl $12, %edx, %edx #53.5
  shldl $16, %r10d, %r10d #45.5
  addl %edx, %r13d #53.5
  xorl %r13d, %r14d #53.5
  addl %r12d, %r13d #47.5
  shldl $8, %r14d, %r14d #53.5
  addl %r14d, %r15d #53.5
  xorl %r9d, %r14d #46.5
  shldl $16, %r14d, %r14d #46.5
  addl %r14d, %edi #46.5
  xorl %r15d, %edx #53.5
  xorl %edi, %r8d #46.5
  addl %r10d, %r15d #45.5
  shldl $12, %r8d, %r8d #46.5
  shldl $7, %edx, %edx #53.5
  xorl %r15d, %esi #45.5
  addl %r8d, %r9d #46.5
  shldl $12, %esi, %esi #45.5
  movl 72(%rsp), %eax #44.5[spill]
  xorl %r9d, %r14d #46.5
  addl %edx, %eax #44.5
  addl %esi, %ebx #45.5
  shldl $8, %r14d, %r14d #46.5
  xorl %eax, %ecx #44.5
  xorl %ebx, %r10d #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $8, %r10d, %r10d #45.5
  movl %r14d, 104(%rsp) #46.5[spill]
  addl %r14d, %edi #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %ecx, %r11d #44.5
  xorl %r13d, %r14d #47.5
  addl %r10d, %r15d #45.5
  shldl $16, %r14d, %r14d #47.5
  xorl %r11d, %edx #44.5
  xorl %r15d, %esi #45.5
  movl %r15d, 88(%rsp) #45.5[spill]
  xorl %edi, %r8d #46.5
  movl 112(%rsp), %r15d #47.5[spill]
  addl %r14d, %r15d #47.5
  shldl $12, %edx, %edx #44.5
  shldl $7, %r8d, %r8d #46.5
  shldl $7, %esi, %esi #45.5
  xorl %r15d, %r12d #47.5
  addl %edx, %eax #44.5
  shldl $12, %r12d, %r12d #47.5
  xorl %eax, %ecx #44.5
  addl %r12d, %r13d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r13d, %r14d #47.5
  addl %r8d, %ebx #51.5
  shldl $8, %r14d, %r14d #47.5
  addl %ecx, %r11d #44.5
  xorl %ebx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r14d, %r15d #47.5
  addl %esi, %eax #50.5
  xorl %r15d, %r12d #47.5
  addl %ecx, %r15d #51.5
  xorl %eax, %r14d #50.5
  xorl %r15d, %r8d #51.5
  shldl $16, %r14d, %r14d #50.5
  shldl $12, %r8d, %r8d #51.5
  shldl $7, %r12d, %r12d #47.5
  addl %r14d, %edi #50.5
  addl %r8d, %ebx #51.5
  xorl %edi, %esi #50.5
  xorl %ebx, %ecx #51.5
  xorl %r11d, %edx #44.5
  addl %r12d, %r9d #52.5
  shldl $12, %esi, %esi #50.5
  shldl $8, %ecx, %ecx #51.5
  shldl $7, %edx, %edx #44.5
  xorl %r9d, %r10d #52.5
  addl %esi, %eax #50.5
  addl %ecx, %r15d #51.5
  addl %edx, %r13d #53.5
  shldl $16, %r10d, %r10d #52.5
  movl %r15d, 112(%rsp) #51.5[spill]
  xorl %eax, %r14d #50.5
  xorl %r15d, %r8d #51.5
  addl %r10d, %r11d #52.5
  movl 104(%rsp), %r15d #53.5[spill]
  xorl %r11d, %r12d #52.5
  xorl %r13d, %r15d #53.5
  shldl $8, %r14d, %r14d #50.5
  shldl $16, %r15d, %r15d #53.5
  shldl $12, %r12d, %r12d #52.5
  shldl $7, %r8d, %r8d #51.5
  movl %r14d, 80(%rsp) #50.5[spill]
  addl %r14d, %edi #50.5
  movl 88(%rsp), %r14d #53.5[spill]
  xorl %edi, %esi #50.5
  addl %r15d, %r14d #53.5
  addl %r12d, %r9d #52.5
  xorl %r14d, %edx #53.5
  xorl %r9d, %r10d #52.5
  shldl $12, %edx, %edx #53.5
  shldl $7, %esi, %esi #50.5
  shldl $8, %r10d, %r10d #52.5
  addl %edx, %r13d #53.5
  addl %esi, %ebx #45.5
  xorl %r13d, %r15d #53.5
  addl %r10d, %r11d #52.5
  shldl $8, %r15d, %r15d #53.5
  xorl %ebx, %r10d #45.5
  addl %r15d, %r14d #53.5
  shldl $16, %r10d, %r10d #45.5
  xorl %r14d, %edx #53.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  xorl %r14d, %esi #45.5
  xorl %r9d, %r15d #46.5
  xorl %r11d, %r12d #52.5
  shldl $12, %esi, %esi #45.5
  shldl $16, %r15d, %r15d #46.5
  shldl $7, %r12d, %r12d #52.5
  shldl $7, %edx, %edx #53.5
  addl %esi, %ebx #45.5
  addl %r15d, %edi #46.5
  xorl %ebx, %r10d #45.5
  xorl %edi, %r8d #46.5
  shldl $8, %r10d, %r10d #45.5
  shldl $12, %r8d, %r8d #46.5
  addl %r10d, %r14d #45.5
  addl %r8d, %r9d #46.5
  addl %r12d, %r13d #47.5
  xorl %r14d, %esi #45.5
  movl %r14d, 88(%rsp) #45.5[spill]
  xorl %r9d, %r15d #46.5
  movl 80(%rsp), %r14d #47.5[spill]
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %ecx #44.5
  shldl $8, %r15d, %r15d #46.5
  shldl $16, %r14d, %r14d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %esi, %esi #45.5
  movl %r15d, 104(%rsp) #46.5[spill]
  addl %r15d, %edi #46.5
  movl 112(%rsp), %r15d #47.5[spill]
  addl %ecx, %r11d #44.5
  addl %r14d, %r15d #47.5
  xorl %r11d, %edx #44.5
  xorl %r15d, %r12d #47.5
  xorl %edi, %r8d #46.5
  shldl $12, %r12d, %r12d #47.5
  shldl $12, %edx, %edx #44.5
  shldl $7, %r8d, %r8d #46.5
  addl %r12d, %r13d #47.5
  addl %edx, %eax #44.5
  xorl %r13d, %r14d #47.5
  xorl %eax, %ecx #44.5
  shldl $8, %r14d, %r14d #47.5
  shldl $8, %ecx, %ecx #44.5
  addl %r14d, %r15d #47.5
  addl %ecx, %r11d #44.5
  xorl %r15d, %r12d #47.5
  xorl %r11d, %edx #44.5
  shldl $7, %r12d, %r12d #47.5
  shldl $7, %edx, %edx #44.5
  addl %r12d, %r9d #52.5
  addl %esi, %eax #50.5
  xorl %r9d, %r10d #52.5
  addl %r8d, %ebx #51.5
  shldl $16, %r10d, %r10d #52.5
  addl %r10d, %r11d #52.5
  addl %edx, %r13d #53.5
  xorl %r11d, %r12d #52.5
  xorl %eax, %r14d #50.5
  shldl $12, %r12d, %r12d #52.5
  shldl $16, %r14d, %r14d #50.5
  addl %r12d, %r9d #52.5
  xorl %ebx, %ecx #51.5
  xorl %r9d, %r10d #52.5
  addl %r14d, %edi #50.5
  shldl $8, %r10d, %r10d #52.5
  shldl $16, %ecx, %ecx #51.5
  addl %r10d, %r11d #52.5
  addl %ecx, %r15d #51.5
  movl %r11d, 120(%rsp) #52.5[spill]
  xorl %r11d, %r12d #52.5
  movl 104(%rsp), %r11d #53.5[spill]
  xorl %edi, %esi #50.5
  xorl %r13d, %r11d #53.5
  xorl %r15d, %r8d #51.5
  shldl $16, %r11d, %r11d #53.5
  shldl $12, %esi, %esi #50.5
  shldl $12, %r8d, %r8d #51.5
  movl %r12d, 96(%rsp) #52.5[spill]
  addl %esi, %eax #50.5
  movl 88(%rsp), %r12d #53.5[spill]
  addl %r8d, %ebx #51.5
  addl %r11d, %r12d #53.5
  xorl %eax, %r14d #50.5
  xorl %r12d, %edx #53.5
  xorl %ebx, %ecx #51.5
  shldl $12, %edx, %edx #53.5
  shldl $8, %r14d, %r14d #50.5
  shldl $8, %ecx, %ecx #51.5
  addl %edx, %r13d #53.5
  addl %r14d, %edi #50.5
  xorl %r13d, %r11d #53.5
  addl %ecx, %r15d #51.5
  shldl $8, %r11d, %r11d #53.5
  addl %r11d, %r12d #53.5
  xorl %edi, %esi #50.5
  xorl %r15d, %r8d #51.5
  xorl %r12d, %edx #53.5
  movd %ebx, %xmm0 #51.5
  movl 96(%rsp), %ebx #52.5[spill]
  movd %eax, %xmm15 #50.5
  shldl $7, %esi, %esi #50.5
  shldl $7, %r8d, %r8d #51.5
  shldl $7, %ebx, %ebx #52.5
  shldl $7, %edx, %edx #53.5
  movd %r9d, %xmm2 #52.5
  movd %r13d, %xmm1 #53.5
  movd %ecx, %xmm14 #51.5
  movl 120(%rsp), %ecx #52.5[spill]
  movd %r14d, %xmm12 #50.5
  unpcklps %xmm0, %xmm15 #57.14
  movd %edi, %xmm9 #50.5
  unpcklps %xmm1, %xmm2 #57.14
  movd %esi, %xmm3 #50.5
  movlhps %xmm2, %xmm15 #57.14
  movd %r15d, %xmm8 #51.5
  movd %r8d, %xmm5 #51.5
  movd %r10d, %xmm11 #52.5
  movd %ecx, %xmm10 #52.5
  movd %ebx, %xmm4 #52.5
  movd %r11d, %xmm13 #53.5
  movd %r12d, %xmm7 #53.5
  movd %edx, %xmm6 #53.5
  paddd test_in.146.0.2(%rip), %xmm15 #57.21
  unpcklps %xmm3, %xmm6 #57.14
  unpcklps %xmm4, %xmm5 #57.14
  unpcklps %xmm7, %xmm10 #57.14
  unpcklps %xmm8, %xmm9 #57.14
  unpcklps %xmm11, %xmm14 #57.14
  unpcklps %xmm12, %xmm13 #57.14
  movd %xmm15, %esi #72.7
  movlhps %xmm5, %xmm6 #57.14
  movlhps %xmm9, %xmm10 #57.14
  movlhps %xmm13, %xmm14 #57.14
  paddd 16+test_in.146.0.2(%rip), %xmm6 #57.21
  paddd 32+test_in.146.0.2(%rip), %xmm10 #57.21
  paddd 48+test_in.146.0.2(%rip), %xmm14 #57.21
  movdqu %xmm15, (%rsp) #71.17
  movdqu %xmm6, 16(%rsp) #71.17
  movdqu %xmm10, 32(%rsp) #71.17
  movdqu %xmm14, 48(%rsp) #71.17
  cmpl $-454561520, %esi #72.22
  jne ..B1.12 # Prob 28% #72.22
..B1.2: # Preds ..B1.15
  cmpl $358169553, 4(%rsp) #72.51
  jne ..B1.12 # Prob 28% #72.51
..B1.3: # Preds ..B1.2
  cmpl $-394014517, 56(%rsp) #73.23
  jne ..B1.12 # Prob 28% #73.23
..B1.4: # Preds ..B1.3
  cmpl $1312575650, 60(%rsp) #73.53
  jne ..B1.12 # Prob 50% #73.53
..B1.5: # Preds ..B1.4
  movdqu .L_2il0floatpacket.1(%rip), %xmm5 #94.13
  xorl %r13d, %r13d #100.8
  movdqu .L_2il0floatpacket.0(%rip), %xmm1 #94.5
  movdqa %xmm5, %xmm6 #94.32
  movdqu .L_2il0floatpacket.4(%rip), %xmm2 #94.5
  movdqa %xmm1, %xmm0 #94.32
  movdqu .L_2il0floatpacket.2(%rip), %xmm4 #94.13
  psrlq $32, %xmm0 #94.32
  pmuludq %xmm1, %xmm6 #94.32
  paddd %xmm2, %xmm1 #94.5
  pmuludq %xmm1, %xmm5 #94.32
  psrlq $32, %xmm4 #94.32
  psrlq $32, %xmm1 #94.32
  pmuludq %xmm4, %xmm0 #94.32
  pmuludq %xmm1, %xmm4 #94.32
  movdqu .L_2il0floatpacket.3(%rip), %xmm3 #94.32
  psllq $32, %xmm0 #94.32
  pand %xmm3, %xmm6 #94.32
  pand %xmm3, %xmm5 #94.32
  psllq $32, %xmm4 #94.32
  por %xmm0, %xmm6 #94.32
  por %xmm4, %xmm5 #94.32
  movl $1634760805, 80(%rsp) #92.3
  movl $857760878, 84(%rsp) #92.23
  movl $2036477234, 88(%rsp) #92.43
  movl $1797285236, 92(%rsp) #92.63
  movdqu %xmm6, 96(%rsp) #94.5
  movdqu %xmm5, 112(%rsp) #94.5
  movl $-559038737, 132(%rsp) #96.3
  movl $19088743, 136(%rsp) #97.3
  movl $-1985229329, 140(%rsp) #98.3
  movd %xmm6, %ebx #105.7
  movq 64(%rsp), %r12 #105.7[spill]
..B1.6: # Preds ..B1.9 ..B1.5
  xorl %r14d, %r14d #101.10
..B1.7: # Preds ..B1.8 ..B1.6
  movl %r13d, %eax #102.20
  lea (%rsp), %rdi #103.7
  xorl %r14d, %eax #102.20
  lea 80(%rsp), %rsi #103.7
  movl %eax, 48(%rsi) #102.7
  call chacha20_core #103.7
..B1.8: # Preds ..B1.7
  movl (%rsp), %esi #104.25
  movl %esi, %ecx #104.25
  incl %r14d #101.34
  shlq $32, %rcx #104.35
  movl 60(%rsp), %eax #104.41
  xorq %rax, %rcx #104.41
  movl 28(%rsp), %edx #104.57
  shlq $16, %rdx #104.67
  movzbl %sil, %esi #105.25
  xorq %rdx, %rcx #104.67
  addq %rcx, %r12 #104.7
  xorl %esi, %ebx #105.7
  movl %ebx, 96(%rsp) #105.7
  cmpl $131072, %r14d #101.21
  jb ..B1.7 # Prob 99% #101.21
..B1.9: # Preds ..B1.8
  incl %r13d #100.33
  cmpl $16, %r13d #100.25
  jb ..B1.6 # Prob 93% #100.25
..B1.10: # Preds ..B1.9
  movq %r12, 64(%rsp) #[spill]
  movl $.L_2__STRING.0, %edi #109.3
  xorl %eax, %eax #109.3
  movq %r12, %rsi #109.3
  call printf #109.3
..B1.11: # Preds ..B1.10
  xorl %eax, %eax #110.10
  addq $216, %rsp #110.10
  popq %rbx #110.10
  popq %r15 #110.10
  popq %r14 #110.10
  popq %r13 #110.10
  popq %r12 #110.10
  movq %rbp, %rsp #110.10
  popq %rbp #110.10
  ret #110.10
..B1.12: # Preds ..B1.15 ..B1.3 ..B1.2 ..B1.4
  movl $2, %eax #89.12
  addq $216, %rsp #89.12
  popq %rbx #89.12
  popq %r15 #89.12
  popq %r14 #89.12
  popq %r13 #89.12
  popq %r12 #89.12
  movq %rbp, %rsp #89.12
  popq %rbp #89.12
  ret #89.12
test_in.146.0.2:
chacha20_core:
..B2.1: # Preds ..B2.0
  pushq %r12 #35.1
  pushq %r13 #35.1
  pushq %r14 #35.1
  pushq %r15 #35.1
  pushq %rbx #35.1
  pushq %rbp #35.1
  movdqu (%rsi), %xmm11 #40.12
  movdqu 16(%rsi), %xmm12 #40.12
  movdqu 48(%rsi), %xmm1 #40.12
  movdqu 32(%rsi), %xmm13 #40.12
  movd %xmm11, %r14d #44.5
  movdqa %xmm11, %xmm2 #40.5
  movdqa %xmm12, %xmm3 #40.5
  psrldq $4, %xmm2 #40.5
  movdqa %xmm1, %xmm4 #40.5
  psrldq $4, %xmm3 #40.5
  movdqa %xmm13, %xmm5 #40.5
  psrldq $4, %xmm4 #40.5
  movdqa %xmm11, %xmm6 #40.5
  movd %xmm2, %edx #45.5
  movdqa %xmm12, %xmm7 #40.5
  movd %xmm3, %r13d #45.5
  movdqa %xmm1, %xmm8 #40.5
  psrldq $4, %xmm5 #40.5
  movdqa %xmm13, %xmm9 #40.5
  psrldq $8, %xmm6 #40.5
  movdqa %xmm11, %xmm10 #40.5
  movq %rdi, -40(%rsp) #35.1[spill]
  addl %r13d, %edx #45.5
  movd %xmm4, %edi #45.5
  movdqa %xmm12, %xmm14 #40.5
  movd %xmm5, %ebx #45.5
  movdqa %xmm1, %xmm15 #40.5
  psrldq $8, %xmm7 #40.5
  movdqa %xmm13, %xmm0 #40.5
  psrldq $8, %xmm8 #40.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  movd %xmm6, %esi #46.5
  movd %xmm7, %ebp #46.5
  movd %xmm8, %r11d #46.5
  psrldq $8, %xmm9 #40.5
  psrldq $12, %xmm10 #40.5
  addl %edi, %ebx #45.5
  addl %ebp, %esi #46.5
  xorl %ebx, %r13d #45.5
  xorl %esi, %r11d #46.5
  shldl $12, %r13d, %r13d #45.5
  shldl $16, %r11d, %r11d #46.5
  movd %xmm9, %eax #46.5
  movd %xmm12, %r12d #44.5
  psrldq $12, %xmm14 #40.5
  psrldq $12, %xmm15 #40.5
  addl %r13d, %edx #45.5
  addl %r11d, %eax #46.5
  movd %xmm1, %ecx #44.5
  xorl %edx, %edi #45.5
  movd %xmm10, %r10d #47.5
  movd %xmm14, %r9d #47.5
  shldl $8, %edi, %edi #45.5
  movd %xmm13, %r8d #44.5
  movd %xmm15, -104(%rsp) #47.5[spill]
  psrldq $12, %xmm0 #40.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  shldl $12, %ebp, %ebp #46.5
  movd %xmm0, %r15d #47.5
  xorl %r14d, %ecx #44.5
  addl %edi, %ebx #45.5
  addl %r9d, %r10d #47.5
  xorl %ebx, %r13d #45.5
  shldl $16, %ecx, %ecx #44.5
  shldl $7, %r13d, %r13d #45.5
  movl %ebx, -24(%rsp) #45.5[spill]
  addl %ebp, %esi #46.5
  movl -104(%rsp), %ebx #47.5[spill]
  addl %ecx, %r8d #44.5
  xorl %r10d, %ebx #47.5
  xorl %esi, %r11d #46.5
  shldl $16, %ebx, %ebx #47.5
  shldl $8, %r11d, %r11d #46.5
  xorl %r8d, %r12d #44.5
  addl %ebx, %r15d #47.5
  shldl $12, %r12d, %r12d #44.5
  addl %r11d, %eax #46.5
  xorl %r15d, %r9d #47.5
  shldl $12, %r9d, %r9d #47.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  shldl $7, %ebp, %ebp #46.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  addl %r11d, %r15d #53.5
  xorl %esi, %r11d #46.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r9d #52.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  shldl $7, %r9d, %r9d #52.5
  xorl %r15d, %r13d #45.5
  addl %r11d, %eax #46.5
  shldl $12, %r13d, %r13d #45.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  addl %r13d, %edx #45.5
  xorl %r14d, %ecx #44.5
  shldl $12, %ebp, %ebp #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %edi #45.5
  addl %r9d, %r10d #47.5
  shldl $8, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  xorl %r10d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r12d #44.5
  shldl $8, %r11d, %r11d #46.5
  shldl $12, %r12d, %r12d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %r15d, %r9d #47.5
  xorl %eax, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  addl %r11d, %r15d #53.5
  xorl %esi, %r11d #46.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r9d #52.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  shldl $7, %r9d, %r9d #52.5
  xorl %r15d, %r13d #45.5
  addl %r11d, %eax #46.5
  shldl $12, %r13d, %r13d #45.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  addl %r13d, %edx #45.5
  xorl %r14d, %ecx #44.5
  shldl $12, %ebp, %ebp #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %edi #45.5
  addl %r9d, %r10d #47.5
  shldl $8, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  xorl %r10d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r12d #44.5
  shldl $8, %r11d, %r11d #46.5
  shldl $12, %r12d, %r12d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %r15d, %r9d #47.5
  xorl %eax, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %r11d, %r15d #53.5
  xorl %r8d, %r9d #52.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r15d, %r13d #45.5
  addl %ebp, %esi #46.5
  shldl $12, %r13d, %r13d #45.5
  shldl $7, %r9d, %r9d #52.5
  shldl $7, %r12d, %r12d #53.5
  addl %r13d, %edx #45.5
  xorl %esi, %r11d #46.5
  xorl %edx, %edi #45.5
  addl %r9d, %r10d #47.5
  shldl $8, %edi, %edi #45.5
  shldl $16, %r11d, %r11d #46.5
  xorl %r10d, %ebx #47.5
  addl %r12d, %r14d #44.5
  shldl $16, %ebx, %ebx #47.5
  addl %edi, %r15d #45.5
  xorl %r14d, %ecx #44.5
  addl %r11d, %eax #46.5
  xorl %r15d, %r13d #45.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %eax, %ebp #46.5
  movl -16(%rsp), %r15d #47.5[spill]
  shldl $16, %ecx, %ecx #44.5
  shldl $12, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  addl %ebx, %r15d #47.5
  addl %ecx, %r8d #44.5
  xorl %r15d, %r9d #47.5
  xorl %r8d, %r12d #44.5
  shldl $12, %r9d, %r9d #47.5
  shldl $12, %r12d, %r12d #44.5
  addl %ebp, %esi #46.5
  addl %r9d, %r10d #47.5
  xorl %esi, %r11d #46.5
  xorl %r10d, %ebx #47.5
  shldl $8, %r11d, %r11d #46.5
  shldl $8, %ebx, %ebx #47.5
  addl %r12d, %r14d #44.5
  addl %r11d, %eax #46.5
  xorl %r14d, %ecx #44.5
  addl %r13d, %r14d #50.5
  xorl %eax, %ebp #46.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $16, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #44.5
  shldl $7, %r9d, %r9d #47.5
  addl %ebx, %eax #50.5
  addl %ebp, %edx #51.5
  addl %ecx, %r8d #44.5
  xorl %eax, %r13d #50.5
  xorl %edx, %ecx #51.5
  xorl %r8d, %r12d #44.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %ecx, %ecx #51.5
  shldl $7, %r12d, %r12d #44.5
  addl %r13d, %r14d #50.5
  addl %ecx, %r15d #51.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %ebp #51.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %ebp, %ebp #51.5
  addl %ebx, %eax #50.5
  addl %ebp, %edx #51.5
  xorl %eax, %r13d #50.5
  xorl %edx, %ecx #51.5
  movl %r14d, -32(%rsp) #50.5[spill]
  shldl $7, %r13d, %r13d #50.5
  shldl $8, %ecx, %ecx #51.5
..B2.5: # Preds ..B2.1
  addl %r9d, %esi #52.5
  addl %r12d, %r10d #53.5
  xorl %esi, %edi #52.5
  xorl %r10d, %r11d #53.5
  shldl $16, %edi, %edi #52.5
  shldl $16, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  xorl %r15d, %ebp #51.5
  movl %r15d, -16(%rsp) #51.5[spill]
  addl %r13d, %edx #45.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %r11d, %r15d #53.5
  shldl $12, %r9d, %r9d #52.5
  shldl $7, %ebp, %ebp #51.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %edi, %r8d #52.5
  shldl $8, %r11d, %r11d #53.5
  xorl %edx, %edi #45.5
  addl %ebp, %esi #46.5
  shldl $16, %edi, %edi #45.5
  addl %r11d, %r15d #53.5
  xorl %esi, %r11d #46.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  xorl %r15d, %r13d #45.5
  xorl %r8d, %r9d #52.5
  shldl $12, %r13d, %r13d #45.5
  shldl $7, %r9d, %r9d #52.5
  addl %r11d, %eax #46.5
  addl %r13d, %edx #45.5
  xorl %eax, %ebp #46.5
  addl %r12d, %r14d #44.5
  xorl %edx, %edi #45.5
  shldl $12, %ebp, %ebp #46.5
  shldl $8, %edi, %edi #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $16, %ecx, %ecx #44.5
  addl %ebp, %esi #46.5
  xorl %r10d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r12d #44.5
  shldl $8, %r11d, %r11d #46.5
  shldl $12, %r12d, %r12d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %r15d, %r9d #47.5
  xorl %eax, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  addl %r11d, %r15d #53.5
  xorl %esi, %r11d #46.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r9d #52.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  shldl $7, %r9d, %r9d #52.5
  xorl %r15d, %r13d #45.5
  addl %r11d, %eax #46.5
  shldl $12, %r13d, %r13d #45.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  addl %r13d, %edx #45.5
  xorl %r14d, %ecx #44.5
  shldl $12, %ebp, %ebp #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %edi #45.5
  addl %r9d, %r10d #47.5
  shldl $8, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  xorl %r10d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r12d #44.5
  shldl $8, %r11d, %r11d #46.5
  shldl $12, %r12d, %r12d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %r15d, %r9d #47.5
  xorl %eax, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  addl %r11d, %r15d #53.5
  xorl %esi, %r11d #46.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r9d #52.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  shldl $7, %r9d, %r9d #52.5
  xorl %r15d, %r13d #45.5
  addl %r11d, %eax #46.5
  shldl $12, %r13d, %r13d #45.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  addl %r13d, %edx #45.5
  xorl %r14d, %ecx #44.5
  shldl $12, %ebp, %ebp #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %edx, %edi #45.5
  addl %r9d, %r10d #47.5
  shldl $8, %edi, %edi #45.5
  addl %ebp, %esi #46.5
  xorl %r10d, %ebx #47.5
  shldl $16, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  addl %edi, %r15d #45.5
  xorl %r8d, %r12d #44.5
  shldl $8, %r11d, %r11d #46.5
  shldl $12, %r12d, %r12d #44.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %r15d, %r9d #47.5
  xorl %eax, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  addl %ebx, %r15d #47.5
  xorl %r14d, %ebx #50.5
  xorl %r15d, %r9d #47.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r15d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r14d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r14d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r15d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r15d, -16(%rsp) #51.5[spill]
  xorl %r15d, %ebp #51.5
  movl -24(%rsp), %r15d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r15d #53.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %r11d, %r15d #53.5
  xorl %r8d, %r9d #52.5
  xorl %r15d, %r12d #53.5
  addl %edi, %r15d #45.5
  xorl %r15d, %r13d #45.5
  addl %ebp, %esi #46.5
  shldl $12, %r13d, %r13d #45.5
  shldl $7, %r9d, %r9d #52.5
  shldl $7, %r12d, %r12d #53.5
  addl %r13d, %edx #45.5
  addl %r9d, %r10d #47.5
  xorl %edx, %edi #45.5
  xorl %esi, %r11d #46.5
  shldl $8, %edi, %edi #45.5
  shldl $16, %r11d, %r11d #46.5
  xorl %r10d, %ebx #47.5
  addl %edi, %r15d #45.5
  shldl $16, %ebx, %ebx #47.5
  movl %r15d, -24(%rsp) #45.5[spill]
  xorl %r15d, %r13d #45.5
  movl -16(%rsp), %r15d #47.5[spill]
  addl %r11d, %eax #46.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  xorl %r15d, %r9d #47.5
  shldl $12, %ebp, %ebp #46.5
  shldl $12, %r9d, %r9d #47.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %ecx #44.5
  addl %ebp, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  addl %r9d, %r10d #47.5
  addl %ecx, %r8d #44.5
  xorl %esi, %r11d #46.5
  xorl %r10d, %ebx #47.5
  shldl $8, %r11d, %r11d #46.5
  shldl $8, %ebx, %ebx #47.5
  xorl %r8d, %r12d #44.5
  addl %r11d, %eax #46.5
  shldl $12, %r12d, %r12d #44.5
  addl %ebx, %r15d #47.5
  addl %r12d, %r14d #44.5
  xorl %eax, %ebp #46.5
  xorl %r15d, %r9d #47.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r9d, %r9d #47.5
  xorl %r14d, %ecx #44.5
  addl %r13d, %r14d #50.5
  shldl $8, %ecx, %ecx #44.5
  addl %ebp, %edx #51.5
  addl %r9d, %esi #52.5
  addl %ecx, %r8d #44.5
  xorl %r14d, %ebx #50.5
  xorl %edx, %ecx #51.5
  xorl %esi, %edi #52.5
  shldl $16, %ebx, %ebx #50.5
  shldl $16, %ecx, %ecx #51.5
  shldl $16, %edi, %edi #52.5
  xorl %r8d, %r12d #44.5
  addl %ebx, %eax #50.5
  addl %ecx, %r15d #51.5
  addl %edi, %r8d #52.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %ebp #51.5
  xorl %r8d, %r9d #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $12, %ebp, %ebp #51.5
  shldl $12, %r9d, %r9d #52.5
  shldl $7, %r12d, %r12d #44.5
  addl %r13d, %r14d #50.5
  addl %ebp, %edx #51.5
  addl %r9d, %esi #52.5
  xorl %r14d, %ebx #50.5
  xorl %edx, %ecx #51.5
  xorl %esi, %edi #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #51.5
  shldl $8, %edi, %edi #52.5
  addl %ebx, %eax #50.5
  addl %ecx, %r15d #51.5
  addl %edi, %r8d #52.5
  xorl %eax, %r13d #50.5
  xorl %r15d, %ebp #51.5
  xorl %r8d, %r9d #52.5
  movl %r14d, -32(%rsp) #50.5[spill]
  addl %r12d, %r10d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  shldl $7, %r9d, %r9d #52.5
..B2.4: # Preds ..B2.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $16, %r11d, %r11d #53.5
  movl -24(%rsp), %r14d #53.5[spill]
  xorl %edx, %edi #45.5
  addl %r11d, %r14d #53.5
  addl %ebp, %esi #46.5
  xorl %r14d, %r12d #53.5
  shldl $12, %r12d, %r12d #53.5
  shldl $16, %edi, %edi #45.5
  addl %r12d, %r10d #53.5
  xorl %r10d, %r11d #53.5
  addl %r9d, %r10d #47.5
  shldl $8, %r11d, %r11d #53.5
  addl %r11d, %r14d #53.5
  xorl %esi, %r11d #46.5
  xorl %r14d, %r12d #53.5
  addl %edi, %r14d #45.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r12d, %r12d #53.5
  xorl %r14d, %r13d #45.5
  addl %r11d, %eax #46.5
  shldl $12, %r13d, %r13d #45.5
  movl %r15d, -16(%rsp) #[spill]
  xorl %eax, %ebp #46.5
  movl -32(%rsp), %r15d #44.5[spill]
  addl %r13d, %edx #45.5
  addl %r12d, %r15d #44.5
  xorl %edx, %edi #45.5
  shldl $12, %ebp, %ebp #46.5
  shldl $8, %edi, %edi #45.5
  xorl %r15d, %ecx #44.5
  addl %ebp, %esi #46.5
  shldl $16, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ecx, %r8d #44.5
  shldl $16, %ebx, %ebx #47.5
  xorl %esi, %r11d #46.5
  addl %edi, %r14d #45.5
  shldl $8, %r11d, %r11d #46.5
  xorl %r8d, %r12d #44.5
  xorl %r14d, %r13d #45.5
  movl %r14d, -24(%rsp) #45.5[spill]
  addl %r11d, %eax #46.5
  movl -16(%rsp), %r14d #47.5[spill]
  xorl %eax, %ebp #46.5
  addl %ebx, %r14d #47.5
  shldl $12, %r12d, %r12d #44.5
  shldl $7, %ebp, %ebp #46.5
  shldl $7, %r13d, %r13d #45.5
  xorl %r14d, %r9d #47.5
  addl %r12d, %r15d #44.5
  shldl $12, %r9d, %r9d #47.5
  xorl %r15d, %ecx #44.5
  addl %r9d, %r10d #47.5
  shldl $8, %ecx, %ecx #44.5
  xorl %r10d, %ebx #47.5
  addl %ebp, %edx #51.5
  shldl $8, %ebx, %ebx #47.5
  addl %ecx, %r8d #44.5
  xorl %edx, %ecx #51.5
  shldl $16, %ecx, %ecx #51.5
  addl %r13d, %r15d #50.5
  addl %ebx, %r14d #47.5
  xorl %r15d, %ebx #50.5
  xorl %r14d, %r9d #47.5
  addl %ecx, %r14d #51.5
  xorl %r8d, %r12d #44.5
  shldl $16, %ebx, %ebx #50.5
  shldl $7, %r9d, %r9d #47.5
  shldl $7, %r12d, %r12d #44.5
  xorl %r14d, %ebp #51.5
  addl %ebx, %eax #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %eax, %r13d #50.5
  addl %r9d, %esi #52.5
  addl %ebp, %edx #51.5
  xorl %esi, %edi #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $16, %edi, %edi #52.5
  xorl %edx, %ecx #51.5
  addl %r12d, %r10d #53.5
  shldl $8, %ecx, %ecx #51.5
  addl %r13d, %r15d #50.5
  xorl %r10d, %r11d #53.5
  shldl $16, %r11d, %r11d #53.5
  xorl %r15d, %ebx #50.5
  addl %edi, %r8d #52.5
  addl %ecx, %r14d #51.5
  xorl %r8d, %r9d #52.5
  shldl $8, %ebx, %ebx #50.5
  shldl $12, %r9d, %r9d #52.5
  movl %r14d, -16(%rsp) #51.5[spill]
  xorl %r14d, %ebp #51.5
  movl -24(%rsp), %r14d #53.5[spill]
  addl %ebx, %eax #50.5
  addl %r11d, %r14d #53.5
  xorl %eax, %r13d #50.5
  xorl %r14d, %r12d #53.5
  addl %r9d, %esi #52.5
  shldl $12, %r12d, %r12d #53.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  xorl %esi, %edi #52.5
  addl %r12d, %r10d #53.5
  shldl $8, %edi, %edi #52.5
  xorl %r10d, %r11d #53.5
  addl %r13d, %edx #45.5
  shldl $8, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %edx, %edi #45.5
  shldl $16, %edi, %edi #45.5
  addl %r11d, %r14d #53.5
  xorl %r8d, %r9d #52.5
  xorl %r14d, %r12d #53.5
  addl %edi, %r14d #45.5
  xorl %r14d, %r13d #45.5
  addl %ebp, %esi #46.5
  shldl $12, %r13d, %r13d #45.5
  shldl $7, %r9d, %r9d #52.5
  shldl $7, %r12d, %r12d #53.5
  addl %r13d, %edx #45.5
  addl %r9d, %r10d #47.5
  xorl %edx, %edi #45.5
  xorl %r10d, %ebx #47.5
  shldl $8, %edi, %edi #45.5
  shldl $16, %ebx, %ebx #47.5
  addl %edi, %r14d #45.5
  addl %r12d, %r15d #44.5
  movl %r14d, -24(%rsp) #45.5[spill]
  xorl %r14d, %r13d #45.5
  movl -16(%rsp), %r14d #47.5[spill]
  xorl %r15d, %ecx #44.5
  addl %ebx, %r14d #47.5
  xorl %esi, %r11d #46.5
  xorl %r14d, %r9d #47.5
  shldl $12, %r9d, %r9d #47.5
  shldl $16, %ecx, %ecx #44.5
  shldl $16, %r11d, %r11d #46.5
  shldl $7, %r13d, %r13d #45.5
  addl %r9d, %r10d #47.5
  addl %ecx, %r8d #44.5
  xorl %r10d, %ebx #47.5
  addl %r11d, %eax #46.5
  shldl $8, %ebx, %ebx #47.5
  xorl %r8d, %r12d #44.5
  xorl %eax, %ebp #46.5
  addl %ebx, %r14d #47.5
  shldl $12, %r12d, %r12d #44.5
  shldl $12, %ebp, %ebp #46.5
  xorl %r14d, %r9d #47.5
  addl %r12d, %r15d #44.5
  shldl $7, %r9d, %r9d #47.5
  addl %ebp, %esi #46.5
  xorl %r15d, %ecx #44.5
  xorl %esi, %r11d #46.5
  addl %r9d, %esi #52.5
  shldl $8, %ecx, %ecx #44.5
  shldl $8, %r11d, %r11d #46.5
  xorl %esi, %edi #52.5
  addl %ecx, %r8d #44.5
  shldl $16, %edi, %edi #52.5
  xorl %r8d, %r12d #44.5
  addl %edi, %r8d #52.5
  xorl %r8d, %r9d #52.5
  addl %r11d, %eax #46.5
  shldl $12, %r9d, %r9d #52.5
  shldl $7, %r12d, %r12d #44.5
  xorl %eax, %ebp #46.5
  addl %r13d, %r15d #50.5
  shldl $7, %ebp, %ebp #46.5
  addl %r9d, %esi #52.5
  xorl %r15d, %ebx #50.5
  xorl %esi, %edi #52.5
  addl %ebp, %edx #51.5
  addl %r12d, %r10d #53.5
  xorl %edx, %ecx #51.5
  shldl $16, %ebx, %ebx #50.5
  shldl $8, %edi, %edi #52.5
  shldl $16, %ecx, %ecx #51.5
  xorl %r10d, %r11d #53.5
  addl %ebx, %eax #50.5
  shldl $16, %r11d, %r11d #53.5
  addl %edi, %r8d #52.5
  xorl %eax, %r13d #50.5
  xorl %r8d, %r9d #52.5
  addl %ecx, %r14d #51.5
  movl %r9d, -8(%rsp) #52.5[spill]
  xorl %r14d, %ebp #51.5
  movl -24(%rsp), %r9d #53.5[spill]
  movd %esi, %xmm5 #52.5
  addl %r11d, %r9d #53.5
  movd %edi, %xmm15 #52.5
  shldl $12, %r13d, %r13d #50.5
  shldl $12, %ebp, %ebp #51.5
  xorl %r9d, %r12d #53.5
  addl %r13d, %r15d #50.5
  shldl $12, %r12d, %r12d #53.5
  xorl %r15d, %ebx #50.5
  addl %ebp, %edx #51.5
  addl %r12d, %r10d #53.5
  xorl %edx, %ecx #51.5
  shldl $8, %ebx, %ebx #50.5
  shldl $8, %ecx, %ecx #51.5
  xorl %r10d, %r11d #53.5
  addl %ebx, %eax #50.5
  shldl $8, %r11d, %r11d #53.5
  addl %ecx, %r14d #51.5
  addl %r11d, %r9d #53.5
  xorl %eax, %r13d #50.5
  xorl %r14d, %ebp #51.5
  xorl %r9d, %r12d #53.5
  movd %eax, %xmm8 #50.5
  movd %r15d, %xmm10 #50.5
  movl -8(%rsp), %eax #52.5[spill]
  movd %edx, %xmm3 #51.5
  shldl $7, %r13d, %r13d #50.5
  shldl $7, %ebp, %ebp #51.5
  shldl $7, %eax, %eax #52.5
  shldl $7, %r12d, %r12d #53.5
  movd %r10d, %xmm0 #53.5
  movd %ebx, %xmm14 #50.5
  unpcklps %xmm3, %xmm10 #57.14
  movd %r13d, %xmm4 #50.5
  unpcklps %xmm0, %xmm5 #57.14
  movd %ecx, %xmm9 #51.5
  movdqu %xmm14, -88(%rsp) #50.5[spill]
  movd %r11d, %xmm14 #53.5
  movups %xmm14, -56(%rsp) #53.5[spill]
  movd %r14d, %xmm7 #51.5
  movlhps %xmm5, %xmm10 #57.14
  movd %ebp, %xmm6 #51.5
  movdqu %xmm1, -104(%rsp) #[spill]
  movd %r8d, %xmm2 #52.5
  movdqu %xmm15, -72(%rsp) #52.5[spill]
  movd %eax, %xmm1 #52.5
  movd %r9d, %xmm14 #53.5
  movd %r12d, %xmm15 #53.5
  paddd %xmm11, %xmm10 #57.21
  movups -56(%rsp), %xmm11 #57.14[spill]
  unpcklps -72(%rsp), %xmm9 #57.14[spill]
  unpcklps -88(%rsp), %xmm11 #57.14[spill]
  unpcklps %xmm4, %xmm15 #57.14
  unpcklps %xmm1, %xmm6 #57.14
  unpcklps %xmm14, %xmm2 #57.14
  unpcklps %xmm7, %xmm8 #57.14
  movq -40(%rsp), %rdx #57.5[spill]
  movlhps %xmm6, %xmm15 #57.14
  movlhps %xmm8, %xmm2 #57.14
  paddd %xmm12, %xmm15 #57.21
  movlhps %xmm11, %xmm9 #57.14
  paddd %xmm13, %xmm2 #57.21
  paddd -104(%rsp), %xmm9 #57.21[spill]
  movdqu %xmm10, (%rdx) #57.5
  movdqu %xmm15, 16(%rdx) #57.5
  movdqu %xmm2, 32(%rdx) #57.5
  movdqu %xmm9, 48(%rdx) #57.5
  popq %rbp #58.1
  popq %rbx #58.1
  popq %r15 #58.1
  popq %r14 #58.1
  popq %r13 #58.1
  popq %r12 #58.1
  ret #58.1
.L_2il0floatpacket.0:
.L_2il0floatpacket.1:
.L_2il0floatpacket.2:
.L_2il0floatpacket.3:
.L_2il0floatpacket.4:
.L_2__STRING.0:
