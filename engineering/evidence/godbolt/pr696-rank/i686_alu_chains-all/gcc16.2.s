k_urem7:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L2
  xorl %esi, %esi
.L3:
  movl %eax, %ecx
  movl %eax, %edx
  imulq $613566757, %rcx, %rcx
  shrq $32, %rcx
  subl %ecx, %edx
  shrl %edx
  addl %ecx, %edx
  shrl $2, %edx
  leal 0(,%rdx,8), %ecx
  subl %edx, %ecx
  subl %ecx, %eax
  addl %esi, %eax
  addl $1, %esi
  cmpl %esi, %edi
  jne .L3
.L2:
  ret
k_urem10:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L10
  xorl %ecx, %ecx
  movl $3435973837, %r8d
.L11:
  movl %eax, %edx
  imulq %r8, %rdx
  shrq $35, %rdx
  leal (%rdx,%rdx,4), %esi
  movl %eax, %edx
  xorl %ecx, %eax
  addl $1, %ecx
  addl %esi, %esi
  subl %esi, %edx
  addl %edx, %eax
  cmpl %ecx, %edi
  jne .L11
.L10:
  ret
k_udiv7:
  testl %esi, %esi
  je .L19
  leal (%rsi,%rsi,2), %esi
  movl %edi, %eax
  xorl %ecx, %ecx
.L18:
  movl %eax, %edx
  imulq $613566757, %rdx, %rdx
  shrq $32, %rdx
  subl %edx, %eax
  shrl %eax
  addl %edx, %eax
  shrl $2, %eax
  addl %ecx, %eax
  addl $3, %ecx
  cmpl %ecx, %esi
  jne .L18
  ret
.L19:
  movl %edi, %eax
  ret
k_udr10:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L22
  xorl %ecx, %ecx
  movl $3435973837, %r8d
.L23:
  movl %eax, %edx
  imulq %r8, %rdx
  shrq $35, %rdx
  leal (%rdx,%rdx,4), %esi
  addl %ecx, %edx
  addl $1, %ecx
  addl %esi, %esi
  subl %esi, %eax
  leal (%rax,%rax,2), %eax
  addl %edx, %eax
  cmpl %ecx, %edi
  jne .L23
.L22:
  ret
k_sdr7:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L29
  xorl %ecx, %ecx
.L30:
  movslq %eax, %rdx
  movl %eax, %esi
  imulq $-1840700269, %rdx, %rdx
  sarl $31, %esi
  shrq $32, %rdx
  addl %eax, %edx
  sarl $2, %edx
  subl %esi, %edx
  leal 0(,%rdx,8), %esi
  subl %edx, %esi
  subl %esi, %eax
  leal (%rax,%rax,4), %eax
  addl %edx, %eax
  subl %ecx, %eax
  addl $1, %ecx
  cmpl %ecx, %edi
  jne .L30
.L29:
  ret
k_srem7:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L36
  xorl %ecx, %ecx
.L37:
  movslq %eax, %rdx
  movl %eax, %esi
  imulq $-1840700269, %rdx, %rdx
  sarl $31, %esi
  shrq $32, %rdx
  addl %eax, %edx
  sarl $2, %edx
  subl %esi, %edx
  leal 0(,%rdx,8), %esi
  subl %edx, %esi
  movzwl %cx, %edx
  addl $1, %ecx
  subl %esi, %eax
  leal -30000(%rax,%rdx), %eax
  cmpl %ecx, %edi
  jne .L37
.L36:
  ret
k_srem16:
  movl %edi, %eax
  testl %esi, %esi
  je .L43
  xorl %ecx, %ecx
.L44:
  cltd
  shrl $28, %edx
  addl %edx, %eax
  andl $15, %eax
  subl %edx, %eax
  leal (%rax,%rax,2), %edx
  movzbl %cl, %eax
  addl $1, %ecx
  leal -100(%rdx,%rax), %eax
  cmpl %ecx, %esi
  jne .L44
.L43:
  ret
k_sdr16:
  movl %edi, %eax
  movl %esi, %edi
  testl %esi, %esi
  je .L50
  xorl %esi, %esi
.L51:
  movl %eax, %ecx
  sarl $31, %ecx
  shrl $28, %ecx
  leal (%rax,%rcx), %edx
  andl $15, %edx
  subl %ecx, %edx
  leal 15(%rax), %ecx
  sall $3, %edx
  testl %eax, %eax
  cmovs %ecx, %eax
  sarl $4, %eax
  xorl %esi, %eax
  addl $1, %esi
  xorl %edx, %eax
  cmpl %esi, %edi
  jne .L51
.L50:
  ret
k_mul7:
  movl %edi, %eax
  testl %esi, %esi
  je .L57
  xorl %edx, %edx
.L58:
  leal 0(,%rax,8), %ecx
  subl %eax, %ecx
  movl %ecx, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L58
.L57:
  ret
k_mul11:
  movl %edi, %eax
  testl %esi, %esi
  je .L64
  xorl %edx, %edx
.L65:
  leal (%rax,%rax,4), %ecx
  leal (%rax,%rcx,2), %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L65
.L64:
  ret
k_mul17:
  movl %edi, %eax
  testl %esi, %esi
  je .L71
  xorl %edx, %edx
.L72:
  movl %eax, %ecx
  sall $4, %ecx
  addl %ecx, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L72
.L71:
  ret
k_mul24:
  movl %edi, %eax
  testl %esi, %esi
  je .L78
  xorl %edx, %edx
.L79:
  leal (%rax,%rax,2), %eax
  sall $3, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L79
.L78:
  ret
k_mul45:
  movl %edi, %eax
  testl %esi, %esi
  je .L85
  xorl %edx, %edx
.L86:
  imull $45, %eax, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L86
.L85:
  ret
k_mulm3:
  movl %edi, %eax
  testl %esi, %esi
  je .L92
  xorl %edx, %edx
.L93:
  leal 0(,%rax,4), %ecx
  subl %ecx, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L93
.L92:
  ret
k_mul1000:
  movl %edi, %eax
  testl %esi, %esi
  je .L99
  xorl %edx, %edx
.L100:
  imull $1000, %eax, %eax
  xorl %edx, %eax
  addl $1, %edx
  cmpl %edx, %esi
  jne .L100
.L99:
  ret
k_fnv:
  movl %edi, %eax
  testl %esi, %esi
  je .L106
  xorl %edx, %edx
.L107:
  movzbl %dl, %ecx
  addl $1, %edx
  xorl %ecx, %eax
  imull $16777619, %eax, %eax
  cmpl %edx, %esi
  jne .L107
.L106:
  ret
k_digits:
  testl %esi, %esi
  je .L117
  leal (%rsi,%rdi), %r10d
  movl $3435973837, %r9d
  xorl %esi, %esi
.L116:
  testl %edi, %edi
  je .L114
  movl %edi, %edx
.L115:
  movl %edx, %eax
  movl %edx, %r8d
  imulq %r9, %rax
  shrq $35, %rax
  leal (%rax,%rax,4), %ecx
  addl %ecx, %ecx
  subl %ecx, %r8d
  addl %r8d, %esi
  cmpl $9, %edx
  movl %eax, %edx
  ja .L115
.L114:
  addl $1, %edi
  cmpl %edi, %r10d
  jne .L116
  movl %esi, %eax
  ret
.L117:
  xorl %esi, %esi
  movl %esi, %eax
  ret
.LC0:
.LC1:
.LC2:
.LC3:
.LC4:
.LC5:
.LC6:
.LC7:
.LC8:
.LC9:
.LC10:
.LC11:
.LC12:
.LC13:
.LC14:
.LC15:
.LC16:
.LC18:
.LC19:
.LC20:
main:
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  movl $20000000, %r12d
  pushq %rbp
  pushq %rbx
  subq $456, %rsp
  cmpl $1, %edi
  jle .L124
  movq 8(%rsi), %rdi
  xorl %edx, %edx
  xorl %esi, %esi
  call __isoc23_strtoul
  movl %eax, %r12d
.L124:
  movq $.LC0, 32(%rsp)
  leaq 32(%rsp), %r15
  xorl %ebx, %ebx
  movq $k_urem7, 40(%rsp)
  movl $123456789, 48(%rsp)
  movq $.LC1, 56(%rsp)
  movq $k_urem10, 64(%rsp)
  movl $987654321, 72(%rsp)
  movq $.LC2, 80(%rsp)
  movq $k_udiv7, 88(%rsp)
  movl $-559038737, 96(%rsp)
  movq $.LC3, 104(%rsp)
  movq $k_udr10, 112(%rsp)
  movl $305419896, 120(%rsp)
  movq $.LC4, 128(%rsp)
  movq $k_sdr7, 136(%rsp)
  movl $2147422772, 144(%rsp)
  movq $.LC5, 152(%rsp)
  movq $k_srem7, 160(%rsp)
  movl $-2147478988, 168(%rsp)
  movq $.LC6, 176(%rsp)
  movq $k_srem16, 184(%rsp)
  movl $-16, 192(%rsp)
  movq $.LC7, 200(%rsp)
  movq $k_sdr16, 208(%rsp)
  movl $267242409, 216(%rsp)
  movq $.LC8, 224(%rsp)
  movq $k_mul7, 232(%rsp)
  movl $1, 240(%rsp)
  movq $.LC9, 248(%rsp)
  movq $k_mul11, 256(%rsp)
  movl $3, 264(%rsp)
  movq $.LC10, 272(%rsp)
  movq $k_mul17, 280(%rsp)
  movl $5, 288(%rsp)
  movq $.LC11, 296(%rsp)
  movq $k_mul24, 304(%rsp)
  movl $7, 312(%rsp)
  movq $.LC12, 320(%rsp)
  movq $k_mul45, 328(%rsp)
  movl $9, 336(%rsp)
  movq $.LC13, 344(%rsp)
  movq $k_mulm3, 352(%rsp)
  movl $11, 360(%rsp)
  movq $.LC14, 368(%rsp)
  movq $k_mul1000, 376(%rsp)
  movl $13, 384(%rsp)
  movq $.LC15, 392(%rsp)
  movq $k_fnv, 400(%rsp)
  movl $-2128831035, 408(%rsp)
  movq $.LC16, 416(%rsp)
  movq $k_digits, 424(%rsp)
  movl $1234567, 432(%rsp)
.L128:
  movq 8(%r15), %rbp
  movl %r12d, %r14d
  leaq 16(%rsp), %rsi
  movl $1, %edi
  shrl $4, %r14d
  cmpq $k_digits, %rbp
  cmovne %r12, %r14
  addq $24, %r15
  call clock_gettime
  vxorpd %xmm2, %xmm2, %xmm2
  vcvtsi2sdq 24(%rsp), %xmm2, %xmm0
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm1
  vfmadd132sd .LC17(%rip), %xmm0, %xmm1
  movl -8(%r15), %edi
  movl %r14d, %esi
  vmovsd %xmm1, 8(%rsp)
  call *%rbp
  leaq 16(%rsp), %rsi
  movl $1, %edi
  movl %eax, %ebp
  call clock_gettime
  vxorpd %xmm2, %xmm2, %xmm2
  movl %ebx, %eax
  movq -24(%r15), %r13
  vcvtsi2sdq 24(%rsp), %xmm2, %xmm1
  sall $5, %eax
  vcvtsi2sdq 16(%rsp), %xmm2, %xmm0
  vfmadd132sd .LC17(%rip), %xmm1, %xmm0
  vsubsd 8(%rsp), %xmm0, %xmm0
  vcvtsi2sdq %r14, %xmm2, %xmm1
  subl %ebx, %eax
  movq %r13, %rdx
  movq stderr(%rip), %rdi
  movl $.LC18, %esi
  leal (%rax,%rbp), %ebx
  movl $1, %eax
  vdivsd %xmm1, %xmm0, %xmm0
  call fprintf
  movl %ebp, %edx
  movq %r13, %rsi
  movl $.LC19, %edi
  xorl %eax, %eax
  call printf
  leaq 440(%rsp), %rax
  cmpq %rax, %r15
  jne .L128
  movl %ebx, %esi
  movl $.LC20, %edi
  xorl %eax, %eax
  call printf
  addq $456, %rsp
  xorl %eax, %eax
  popq %rbx
  popq %rbp
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  ret
.LC17:
