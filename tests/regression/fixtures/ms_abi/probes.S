.text
.globl shadow_probe
.type shadow_probe,@function
shadow_probe:
 mov %rsp,%rax
 and $15,%eax
 cmp $8,%eax
 jne 1f
 mov %rcx,8(%rsp)
 mov %rdx,16(%rsp)
 mov %r8,24(%rsp)
 mov %r9,32(%rsp)
 mov 8(%rsp),%rax
 add 16(%rsp),%rax
 add 24(%rsp),%rax
 add 32(%rsp),%rax
 add 40(%rsp),%rax
 ret
1: mov $-1,%rax
 ret
.size shadow_probe,.-shadow_probe
.globl preservation_probe
.type preservation_probe,@function
preservation_probe:
 sub $40,%rsp
 mov $1234567,%rdi
 mov $7654321,%rsi
 pcmpeqd %xmm6,%xmm6
 pcmpeqd %xmm7,%xmm7
 pcmpeqd %xmm8,%xmm8
 pcmpeqd %xmm9,%xmm9
 pcmpeqd %xmm10,%xmm10
 pcmpeqd %xmm11,%xmm11
 pcmpeqd %xmm12,%xmm12
 pcmpeqd %xmm13,%xmm13
 pcmpeqd %xmm14,%xmm14
 pcmpeqd %xmm15,%xmm15
 call lccc_ms_clobber
 cmp $1234567,%rdi
 jne 1f
 cmp $7654321,%rsi
 jne 1f
 pmovmskb %xmm6,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm7,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm8,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm9,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm10,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm11,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm12,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm13,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm14,%eax
 cmp $65535,%eax
 jne 1f
 pmovmskb %xmm15,%eax
 cmp $65535,%eax
 jne 1f
 xor %eax,%eax
 jmp 2f
1: mov $1,%eax
2: add $40,%rsp
 ret
.size preservation_probe,.-preservation_probe
.section .note.GNU-stack,"",@progbits
