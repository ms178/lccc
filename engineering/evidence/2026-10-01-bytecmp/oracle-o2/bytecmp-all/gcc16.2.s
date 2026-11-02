bc_scan:
  cmpq %rsi, %rdi
  jb .L2
  jmp .L7
.L4:
  addq $1, %rdi
  addq $1, %rdx
  cmpq %rdi, %rsi
  je .L5
.L2:
  movzbl (%rdx), %eax
  cmpb %al, (%rdi)
  je .L4
.L7:
  movq %rdi, %rax
  ret
.L5:
  movq %rsi, %rax
  ret
bc_scan_signed:
  cmpq %rsi, %rdi
  jb .L10
  jmp .L15
.L12:
  addq $1, %rdi
  addq $1, %rdx
  cmpq %rdi, %rsi
  je .L13
.L10:
  movzbl (%rdx), %eax
  cmpb %al, (%rdi)
  je .L12
.L15:
  movq %rdi, %rax
  ret
.L13:
  movq %rsi, %rax
  ret
match_extend:
  leaq 8(%rdi), %rax
  movq %rdx, %rcx
  cmpq %rax, %rdx
  jnb .L17
  jmp .L26
.L19:
  leaq 8(%rax), %rdx
  addq $8, %rsi
  cmpq %rdx, %rcx
  jb .L18
  movq %rdx, %rax
.L17:
  movq (%rsi), %rdx
  cmpq %rdx, -8(%rax)
  je .L19
  subq $8, %rax
.L18:
  cmpq %rcx, %rax
  jb .L20
  jmp .L21
.L22:
  addq $1, %rax
  addq $1, %rsi
  cmpq %rax, %rcx
  je .L21
.L20:
  movzbl (%rsi), %edx
  cmpb %dl, (%rax)
  je .L22
.L21:
  subq %rdi, %rax
  ret
.L26:
  movq %rdi, %rax
  jmp .L18
