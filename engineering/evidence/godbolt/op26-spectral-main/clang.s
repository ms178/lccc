main:
  pushq %rbp
  pushq %r15
  pushq %r14
  pushq %r13
  pushq %r12
  pushq %rbx
  subq $48072, %rsp
  xorl %eax, %eax
  vbroadcastsd .LCPI0_0(%rip), %ymm0
.LBB0_1:
  vmovups %ymm0, 64(%rsp,%rax,8)
  vmovups %ymm0, 96(%rsp,%rax,8)
  vmovups %ymm0, 128(%rsp,%rax,8)
  vmovups %ymm0, 160(%rsp,%rax,8)
  vmovups %ymm0, 192(%rsp,%rax,8)
  vmovups %ymm0, 224(%rsp,%rax,8)
  vmovups %ymm0, 256(%rsp,%rax,8)
  vmovups %ymm0, 288(%rsp,%rax,8)
  vmovups %ymm0, 320(%rsp,%rax,8)
  vmovups %ymm0, 352(%rsp,%rax,8)
  vmovups %ymm0, 384(%rsp,%rax,8)
  vmovups %ymm0, 416(%rsp,%rax,8)
  vmovups %ymm0, 448(%rsp,%rax,8)
  vmovups %ymm0, 480(%rsp,%rax,8)
  vmovups %ymm0, 512(%rsp,%rax,8)
  vmovups %ymm0, 544(%rsp,%rax,8)
  vmovups %ymm0, 576(%rsp,%rax,8)
  vmovups %ymm0, 608(%rsp,%rax,8)
  vmovups %ymm0, 640(%rsp,%rax,8)
  vmovups %ymm0, 672(%rsp,%rax,8)
  addq $80, %rax
  cmpq $2000, %rax
  jne .LBB0_1
  xorl %eax, %eax
  vmovsd .LCPI0_0(%rip), %xmm0
.LBB0_3:
  movq %rax, 56(%rsp)
  xorl %eax, %eax
.LBB0_4:
  leaq 1(%rax), %rcx
  vxorpd %xmm1, %xmm1, %xmm1
  xorl %edx, %edx
.LBB0_5:
  leal (%rax,%rdx), %esi
  leal (%rax,%rdx), %edi
  incl %edi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm2
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd231sd 64(%rsp,%rdx,8), %xmm2, %xmm1
  leal (%rax,%rdx), %esi
  addl $2, %esi
  imull %esi, %edi
  shrl %edi
  addl %ecx, %edi
  vcvtsi2sd %edi, %xmm15, %xmm2
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd132sd 72(%rsp,%rdx,8), %xmm1, %xmm2
  leal (%rax,%rdx), %edi
  addl $3, %edi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm3
  vfmadd132sd 80(%rsp,%rdx,8), %xmm2, %xmm3
  leal (%rax,%rdx), %esi
  addl $4, %esi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm1
  vfmadd132sd 88(%rsp,%rdx,8), %xmm3, %xmm1
  addq $4, %rdx
  cmpq $2000, %rdx
  jne .LBB0_5
  vmovsd %xmm1, 32064(%rsp,%rax,8)
  movq %rcx, %rax
  cmpq $2000, %rcx
  jne .LBB0_4
  movl $6, %r9d
  movl $36, %edx
  movl $4, %ecx
  movl $28, %edi
  movl $2, %r8d
  xorl %esi, %esi
  movl $20, %r10d
  movl $8, %r11d
  movl $12, %eax
  movl $44, %r12d
  movl $2, %r14d
  movl $6, %r15d
  xorl %r13d, %r13d
.LBB0_8:
  movl %r11d, 4(%rsp)
  movl %ecx, 20(%rsp)
  movq %r13, 32(%rsp)
  leaq 1(%r13), %rcx
  movq %rcx, 40(%rsp)
  vxorpd %xmm1, %xmm1, %xmm1
  movl %eax, %ebx
  movl %r12d, %ebp
  movq %rsi, 48(%rsp)
  movl %esi, %ecx
  movl %r10d, 8(%rsp)
  movl %r8d, 12(%rsp)
  movl %edi, 16(%rsp)
  movl %r9d, 28(%rsp)
  movl %edx, 24(%rsp)
  xorl %r13d, %r13d
.LBB0_9:
  movl %ecx, %r11d
  shrl %r11d
  addl %r13d, %r11d
  incl %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm2
  movl %r8d, %r11d
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd231sd 32064(%rsp,%r13,8), %xmm2, %xmm1
  shrl %r11d
  addl %r13d, %r11d
  addl $2, %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm2
  movl %r9d, %r11d
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd132sd 32072(%rsp,%r13,8), %xmm1, %xmm2
  shrl %r11d
  movl %ebx, %esi
  addl %r13d, %r11d
  addl $3, %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm1
  shrl %esi
  vdivsd %xmm1, %xmm0, %xmm3
  vfmadd132sd 32080(%rsp,%r13,8), %xmm2, %xmm3
  addl %r13d, %esi
  addl $4, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm1
  vfmadd132sd 32088(%rsp,%r13,8), %xmm3, %xmm1
  addq $4, %r13
  addl %edx, %r9d
  addl $32, %edx
  addl %edi, %r8d
  addl $32, %edi
  addl %r10d, %ecx
  addl $32, %r10d
  addl %ebp, %ebx
  addl $32, %ebp
  cmpq $2000, %r13
  jne .LBB0_9
  movq 32(%rsp), %rcx
  vmovsd %xmm1, 16064(%rsp,%rcx,8)
  movl 28(%rsp), %r9d
  addl %r15d, %r9d
  addl $2, %r15d
  movl 24(%rsp), %edx
  addl $8, %edx
  movl 20(%rsp), %ecx
  movl 12(%rsp), %r8d
  addl %ecx, %r8d
  addl $2, %ecx
  movl 16(%rsp), %edi
  addl $8, %edi
  movq 48(%rsp), %rsi
  addl %r14d, %esi
  addl $2, %r14d
  movl 8(%rsp), %r10d
  addl $8, %r10d
  movl 4(%rsp), %r11d
  addl %r11d, %eax
  addl $2, %r11d
  addl $8, %r12d
  movq 40(%rsp), %rbx
  movq %rbx, %r13
  cmpq $2000, %rbx
  jne .LBB0_8
  xorl %eax, %eax
.LBB0_12:
  leaq 1(%rax), %rcx
  vxorpd %xmm1, %xmm1, %xmm1
  xorl %edx, %edx
.LBB0_13:
  leal (%rax,%rdx), %esi
  leal (%rax,%rdx), %edi
  incl %edi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm2
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd231sd 16064(%rsp,%rdx,8), %xmm2, %xmm1
  leal (%rax,%rdx), %esi
  addl $2, %esi
  imull %esi, %edi
  shrl %edi
  addl %ecx, %edi
  vcvtsi2sd %edi, %xmm15, %xmm2
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd132sd 16072(%rsp,%rdx,8), %xmm1, %xmm2
  leal (%rax,%rdx), %edi
  addl $3, %edi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm3
  vfmadd132sd 16080(%rsp,%rdx,8), %xmm2, %xmm3
  leal (%rax,%rdx), %esi
  addl $4, %esi
  imull %edi, %esi
  shrl %esi
  addl %ecx, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm1
  vfmadd132sd 16088(%rsp,%rdx,8), %xmm3, %xmm1
  addq $4, %rdx
  cmpq $2000, %rdx
  jne .LBB0_13
  vmovsd %xmm1, 32064(%rsp,%rax,8)
  movq %rcx, %rax
  cmpq $2000, %rcx
  jne .LBB0_12
  movl $6, %r9d
  movl $36, %edx
  movl $4, %eax
  movl $28, %edi
  movl $2, %r8d
  xorl %ecx, %ecx
  movl $20, %r10d
  movl $8, %esi
  movl $12, %ebx
  movl $44, %r12d
  movl $2, %r14d
  movl $6, %r15d
  xorl %r13d, %r13d
.LBB0_16:
  movl %esi, 4(%rsp)
  movl %eax, 20(%rsp)
  movq %r13, 32(%rsp)
  leaq 1(%r13), %rax
  movq %rax, 40(%rsp)
  vxorpd %xmm1, %xmm1, %xmm1
  movl %ebx, %eax
  movl %r12d, %ebp
  movq %rcx, 48(%rsp)
  movl %r10d, 8(%rsp)
  movl %r8d, 12(%rsp)
  movl %edi, 16(%rsp)
  movl %r9d, 28(%rsp)
  movl %edx, 24(%rsp)
  xorl %r13d, %r13d
.LBB0_17:
  movl %ecx, %r11d
  shrl %r11d
  addl %r13d, %r11d
  incl %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm2
  movl %r8d, %r11d
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd231sd 32064(%rsp,%r13,8), %xmm2, %xmm1
  shrl %r11d
  addl %r13d, %r11d
  addl $2, %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm2
  movl %r9d, %r11d
  vdivsd %xmm2, %xmm0, %xmm2
  vfmadd132sd 32072(%rsp,%r13,8), %xmm1, %xmm2
  shrl %r11d
  movl %eax, %esi
  addl %r13d, %r11d
  addl $3, %r11d
  vcvtsi2sd %r11d, %xmm15, %xmm1
  shrl %esi
  vdivsd %xmm1, %xmm0, %xmm3
  vfmadd132sd 32080(%rsp,%r13,8), %xmm2, %xmm3
  addl %r13d, %esi
  addl $4, %esi
  vcvtsi2sd %esi, %xmm15, %xmm1
  vdivsd %xmm1, %xmm0, %xmm1
  vfmadd132sd 32088(%rsp,%r13,8), %xmm3, %xmm1
  addq $4, %r13
  addl %edx, %r9d
  addl $32, %edx
  addl %edi, %r8d
  addl $32, %edi
  addl %r10d, %ecx
  addl $32, %r10d
  addl %ebp, %eax
  addl $32, %ebp
  cmpq $2000, %r13
  jne .LBB0_17
  movq 32(%rsp), %rax
  vmovsd %xmm1, 64(%rsp,%rax,8)
  movl 28(%rsp), %r9d
  addl %r15d, %r9d
  addl $2, %r15d
  movl 24(%rsp), %edx
  addl $8, %edx
  movl 20(%rsp), %eax
  movl 12(%rsp), %r8d
  addl %eax, %r8d
  addl $2, %eax
  movl 16(%rsp), %edi
  addl $8, %edi
  movq 48(%rsp), %rcx
  addl %r14d, %ecx
  addl $2, %r14d
  movl 8(%rsp), %r10d
  addl $8, %r10d
  movl 4(%rsp), %esi
  addl %esi, %ebx
  addl $2, %esi
  addl $8, %r12d
  movq 40(%rsp), %r11
  movq %r11, %r13
  cmpq $2000, %r11
  jne .LBB0_16
  movq 56(%rsp), %rax
  incl %eax
  cmpl $10, %eax
  jne .LBB0_3
  vxorpd %xmm0, %xmm0, %xmm0
  movl $7, %eax
  vxorpd %xmm1, %xmm1, %xmm1
.LBB0_21:
  vmovsd 16008(%rsp,%rax,8), %xmm2
  vmovsd 16016(%rsp,%rax,8), %xmm3
  vfmadd231sd 8(%rsp,%rax,8), %xmm2, %xmm1
  vfmadd231sd 16(%rsp,%rax,8), %xmm3, %xmm1
  vfmadd231sd %xmm2, %xmm2, %xmm0
  vmovsd 16024(%rsp,%rax,8), %xmm2
  vfmadd231sd 24(%rsp,%rax,8), %xmm2, %xmm1
  vfmadd231sd %xmm3, %xmm3, %xmm0
  vmovsd 16032(%rsp,%rax,8), %xmm3
  vfmadd231sd 32(%rsp,%rax,8), %xmm3, %xmm1
  vfmadd231sd %xmm2, %xmm2, %xmm0
  vmovsd 16040(%rsp,%rax,8), %xmm2
  vfmadd231sd 40(%rsp,%rax,8), %xmm2, %xmm1
  vfmadd231sd %xmm3, %xmm3, %xmm0
  vmovsd 16048(%rsp,%rax,8), %xmm3
  vfmadd231sd 48(%rsp,%rax,8), %xmm3, %xmm1
  vfmadd231sd %xmm2, %xmm2, %xmm0
  vmovsd 16056(%rsp,%rax,8), %xmm2
  vfmadd231sd 56(%rsp,%rax,8), %xmm2, %xmm1
  vfmadd231sd %xmm3, %xmm3, %xmm0
  vmovsd 16064(%rsp,%rax,8), %xmm3
  vfmadd231sd 64(%rsp,%rax,8), %xmm3, %xmm1
  vfmadd231sd %xmm2, %xmm2, %xmm0
  vfmadd231sd %xmm3, %xmm3, %xmm0
  addq $8, %rax
  cmpq $2007, %rax
  jne .LBB0_21
  vdivsd %xmm0, %xmm1, %xmm0
  vxorpd %xmm1, %xmm1, %xmm1
  vucomisd %xmm1, %xmm0
  jb .LBB0_24
  vsqrtsd %xmm0, %xmm0, %xmm0
  jmp .LBB0_25
.LBB0_24:
  vzeroupper
  callq sqrt@PLT
.LBB0_25:
  leaq .L.str(%rip), %rdi
  movb $1, %al
  vzeroupper
  callq printf@PLT
  xorl %eax, %eax
  addq $48072, %rsp
  popq %rbx
  popq %r12
  popq %r13
  popq %r14
  popq %r15
  popq %rbp
  retq

.L.str:
  .asciz "%.9f\n"
