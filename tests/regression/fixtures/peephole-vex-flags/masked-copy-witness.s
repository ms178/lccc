# A masked result is not a copy of the source, even with zero-masking.
# k1=0 makes the merge preserve the old destination and the zero form clear it.
.text
.macro merge name, result
.globl \name
.type \name,@function
\name:
    kxorw %k1,%k1,%k1
    vpxor %xmm1,%xmm1,%xmm1
    vpcmpeqd %xmm2,%xmm2,%xmm2
    vmovdqa64 %xmm1,%xmm2{%k1}
    vmovq \result,%rax
    vzeroupper
    ret
.size \name,.-\name
.endm
merge masked_merge_original, %xmm2
merge masked_merge_illegal, %xmm1
.macro zero name, result
.globl \name
.type \name,@function
\name:
    kxorw %k1,%k1,%k1
    vpcmpeqd %xmm1,%xmm1,%xmm1
    vpxor %xmm2,%xmm2,%xmm2
    vmovdqa64 %xmm1,%xmm2{%k1}{z}
    vmovq \result,%rax
    vzeroupper
    ret
.size \name,.-\name
.endm
zero masked_zero_original, %xmm2
zero masked_zero_illegal, %xmm1
.section .note.GNU-stack,"",@progbits
