count_primes:
  movl $10000001, %edx
  subq $8, %rsp
  movl $1, %esi
  movl $sieve, %edi
  call memset
  xorl %eax, %eax
  movl $2, %edx
  movw %ax, sieve(%rip)
  movl $4, %eax
  jmp .L4
.L2:
  leal 1(%rcx), %eax
  addq $1, %rdx
  imull %eax, %eax
  cmpl $10000000, %eax
  jg .L12
.L4:
  movl %edx, %ecx
  cmpb $0, sieve(%rdx)
  je .L2
.L3:
  movb $0, sieve(%rax)
  addq %rdx, %rax
  cmpl $10000000, %eax
  jle .L3
  jmp .L2
.L12:
  movl $sieve+2, %eax
  movl $sieve+10000001, %ecx
  xorl %edx, %edx
.L6:
  cmpb $1, (%rax)
  sbbl $-1, %edx
  addq $1, %rax
  cmpq %rax, %rcx
  jne .L6
  movl %edx, %eax
  addq $8, %rsp
  ret
.LC0:
  .string "primes up to %d: %d\n"
main:
  subq $24, %rsp
  call count_primes
  movl $10000000, %esi
  movl $.LC0, %edi
  movl %eax, 12(%rsp)
  movl 12(%rsp), %edx
  xorl %eax, %eax
  call printf
  xorl %eax, %eax
  addq $24, %rsp
  ret
