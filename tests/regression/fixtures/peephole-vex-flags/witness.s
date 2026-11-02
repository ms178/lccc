.text
.globl original_vucomisd
.type original_vucomisd, @function
original_vucomisd:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vucomisd %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size original_vucomisd, .-original_vucomisd
.globl removed_vucomisd
.type removed_vucomisd, @function
removed_vucomisd:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vucomisd %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size removed_vucomisd, .-removed_vucomisd
.globl original_vucomiss
.type original_vucomiss, @function
original_vucomiss:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vucomiss %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size original_vucomiss, .-original_vucomiss
.globl removed_vucomiss
.type removed_vucomiss, @function
removed_vucomiss:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vucomiss %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size removed_vucomiss, .-removed_vucomiss
.globl original_vcomisd
.type original_vcomisd, @function
original_vcomisd:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vcomisd %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size original_vcomisd, .-original_vcomisd
.globl removed_vcomisd
.type removed_vcomisd, @function
removed_vcomisd:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vcomisd %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size removed_vcomisd, .-removed_vcomisd
.globl original_vcomiss
.type original_vcomiss, @function
original_vcomiss:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vcomiss %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size original_vcomiss, .-original_vcomiss
.globl removed_vcomiss
.type removed_vcomiss, @function
removed_vcomiss:
    vxorpd %ymm6, %ymm6, %ymm6
    vmovddup .Lvalue(%rip), %xmm5
    vmovapd %ymm5, %ymm7
    vcomiss %xmm5, %xmm7
    setne %al
    movzbl %al, %eax
    ret
.size removed_vcomiss, .-removed_vcomiss
.globl original_vptest
.type original_vptest, @function
original_vptest:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vptest %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size original_vptest, .-original_vptest
.globl removed_vptest
.type removed_vptest, @function
removed_vptest:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vptest %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size removed_vptest, .-removed_vptest
.globl original_vtestpd
.type original_vtestpd, @function
original_vtestpd:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vtestpd %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size original_vtestpd, .-original_vtestpd
.globl removed_vtestpd
.type removed_vtestpd, @function
removed_vtestpd:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vtestpd %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size removed_vtestpd, .-removed_vtestpd
.globl original_vtestps
.type original_vtestps, @function
original_vtestps:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    vtestps %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size original_vtestps, .-original_vtestps
.globl removed_vtestps
.type removed_vtestps, @function
removed_vtestps:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vtestps %ymm5, %ymm7
    sete %al
    movzbl %al, %eax
    ret
.size removed_vtestps, .-removed_vtestps
.globl original_vpcmpistri
.type original_vpcmpistri, @function
original_vpcmpistri:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    movl $16, %eax
    movl $16, %edx
    vpcmpistri $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size original_vpcmpistri, .-original_vpcmpistri
.globl removed_vpcmpistri
.type removed_vpcmpistri, @function
removed_vpcmpistri:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    movl $16, %eax
    movl $16, %edx
    vpcmpistri $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size removed_vpcmpistri, .-removed_vpcmpistri
.globl original_vpcmpistrm
.type original_vpcmpistrm, @function
original_vpcmpistrm:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    movl $16, %eax
    movl $16, %edx
    vpcmpistrm $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size original_vpcmpistrm, .-original_vpcmpistrm
.globl removed_vpcmpistrm
.type removed_vpcmpistrm, @function
removed_vpcmpistrm:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    movl $16, %eax
    movl $16, %edx
    vpcmpistrm $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size removed_vpcmpistrm, .-removed_vpcmpistrm
.globl original_vpcmpestri
.type original_vpcmpestri, @function
original_vpcmpestri:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    movl $16, %eax
    movl $16, %edx
    vpcmpestri $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size original_vpcmpestri, .-original_vpcmpestri
.globl removed_vpcmpestri
.type removed_vpcmpestri, @function
removed_vpcmpestri:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    movl $16, %eax
    movl $16, %edx
    vpcmpestri $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size removed_vpcmpestri, .-removed_vpcmpestri
.globl original_vpcmpestrm
.type original_vpcmpestrm, @function
original_vpcmpestrm:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    vmovapd %xmm6, %xmm7
    movl $16, %eax
    movl $16, %edx
    vpcmpestrm $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size original_vpcmpestrm, .-original_vpcmpestrm
.globl removed_vpcmpestrm
.type removed_vpcmpestrm, @function
removed_vpcmpestrm:
    vxorpd %ymm6, %ymm6, %ymm6
    vpcmpeqd %ymm5, %ymm5, %ymm5
    vmovapd %ymm5, %ymm7
    movl $16, %eax
    movl $16, %edx
    vpcmpestrm $0, %xmm5, %xmm7
    setnc %al
    movzbl %al, %eax
    ret
.size removed_vpcmpestrm, .-removed_vpcmpestrm
.section .rodata
.p2align 3
.Lvalue:
.quad 0x3ff000003f800000
.section .note.GNU-stack,"",@progbits
