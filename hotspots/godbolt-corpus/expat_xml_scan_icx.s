.LCPI0_1:
  .zero 4,32
main:
  pushq %rbp
  pushq %r14
  pushq %rbx
  subq $16, %rsp
  vstmxcsr 12(%rsp)
  orl $32832, 12(%rsp)
  vldmxcsr 12(%rsp)
  movl $check_name_kernel.ascii_name, %eax
  jmp .LBB0_2
.LBB0_1:
  addq %rdx, %rax
  cmpq $check_name_kernel.ascii_name+7, %rax
  jae .LBB0_24
.LBB0_2:
  movzbl (%rax), %ecx
  testb %cl, %cl
  js .LBB0_5
  movl $1, %edx
  cmpb $97, %cl
  jb .LBB0_9
  cmpb $122, %cl
  jbe .LBB0_1
  jmp .LBB0_14
.LBB0_5:
  cmpb $-62, %cl
  jb .LBB0_24
  cmpb $-32, %cl
  jb .LBB0_12
  cmpb $-16, %cl
  jae .LBB0_16
  movl $3, %edx
  xorl %esi, %esi
  jmp .LBB0_18
.LBB0_9:
  cmpb $65, %cl
  jb .LBB0_13
  cmpb $91, %cl
  jb .LBB0_1
  cmpb $95, %cl
  jne .LBB0_14
  jmp .LBB0_1
.LBB0_12:
  movl $2, %edx
  xorl %esi, %esi
  jmp .LBB0_18
.LBB0_13:
  cmpb $58, %cl
  je .LBB0_1
.LBB0_14:
  leal -58(%rcx), %esi
  cmpb $-11, %sil
  ja .LBB0_1
  addb $-47, %cl
  cmpb $-2, %cl
  jae .LBB0_1
  jmp .LBB0_24
.LBB0_16:
  cmpb $-12, %cl
  ja .LBB0_24
  movl $4, %edx
  movb $1, %sil
.LBB0_18:
  movl $check_name_kernel.ascii_name+7, %edi
  subq %rax, %rdi
  cmpq %rdx, %rdi
  jb .LBB0_24
  movzbl 1(%rax), %edi
  andb $-64, %dil
  cmpb $-128, %dil
  jne .LBB0_24
  cmpb $-32, %cl
  jb .LBB0_22
  movzbl 2(%rax), %ecx
  andb $-64, %cl
  cmpb $-128, %cl
  jne .LBB0_24
.LBB0_22:
  testb %sil, %sil
  je .LBB0_1
  movzbl 3(%rax), %ecx
  andb $-64, %cl
  cmpb $-128, %cl
  je .LBB0_1
.LBB0_24:
  movl $check_name_kernel.ascii_name, %ecx
  subq %rcx, %rax
  movl $2, %ebx
  cmpq $7, %rax
  jne .LBB0_104
  movl $check_name_kernel.utf8_name, %eax
  jmp .LBB0_27
.LBB0_26:
  addq %rdx, %rax
  cmpq $check_name_kernel.utf8_name+8, %rax
  jae .LBB0_49
.LBB0_27:
  movzbl (%rax), %ecx
  testb %cl, %cl
  js .LBB0_30
  movl $1, %edx
  cmpb $97, %cl
  jb .LBB0_34
  cmpb $122, %cl
  jbe .LBB0_26
  jmp .LBB0_39
.LBB0_30:
  cmpb $-62, %cl
  jb .LBB0_49
  cmpb $-32, %cl
  jb .LBB0_37
  cmpb $-16, %cl
  jae .LBB0_41
  movl $3, %edx
  xorl %esi, %esi
  jmp .LBB0_43
.LBB0_34:
  cmpb $65, %cl
  jb .LBB0_38
  cmpb $91, %cl
  jb .LBB0_26
  cmpb $95, %cl
  jne .LBB0_39
  jmp .LBB0_26
.LBB0_37:
  movl $2, %edx
  xorl %esi, %esi
  jmp .LBB0_43
.LBB0_38:
  cmpb $58, %cl
  je .LBB0_26
.LBB0_39:
  leal -58(%rcx), %esi
  cmpb $-11, %sil
  ja .LBB0_26
  addb $-47, %cl
  cmpb $-2, %cl
  jae .LBB0_26
  jmp .LBB0_49
.LBB0_41:
  cmpb $-12, %cl
  ja .LBB0_49
  movl $4, %edx
  movb $1, %sil
.LBB0_43:
  movl $check_name_kernel.utf8_name+8, %edi
  subq %rax, %rdi
  cmpq %rdx, %rdi
  jb .LBB0_49
  movzbl 1(%rax), %edi
  andb $-64, %dil
  cmpb $-128, %dil
  jne .LBB0_49
  cmpb $-32, %cl
  jb .LBB0_47
  movzbl 2(%rax), %ecx
  andb $-64, %cl
  cmpb $-128, %cl
  jne .LBB0_49
.LBB0_47:
  testb %sil, %sil
  je .LBB0_26
  movzbl 3(%rax), %ecx
  andb $-64, %cl
  cmpb $-128, %cl
  je .LBB0_26
.LBB0_49:
  movl $check_name_kernel.utf8_name, %ecx
  subq %rcx, %rax
  cmpq $8, %rax
  jne .LBB0_104
  movq $-1048320, %rax
  vmovups make_xml_corpus.fragment+28(%rip), %ymm0
  vmovups make_xml_corpus.fragment(%rip), %ymm1
.LBB0_51:
  vmovups %ymm0, expat_xml_data+1048348(%rax)
  vmovups %ymm1, expat_xml_data+1048320(%rax)
  vmovups %ymm1, expat_xml_data+1048380(%rax)
  vmovups %ymm0, expat_xml_data+1048408(%rax)
  vmovups %ymm1, expat_xml_data+1048440(%rax)
  vmovups %ymm0, expat_xml_data+1048468(%rax)
  vmovups %ymm0, expat_xml_data+1048528(%rax)
  vmovups %ymm1, expat_xml_data+1048500(%rax)
  vmovups %ymm1, expat_xml_data+1048560(%rax)
  vmovups %ymm0, expat_xml_data+1048588(%rax)
  vmovups %ymm0, expat_xml_data+1048648(%rax)
  vmovups %ymm1, expat_xml_data+1048620(%rax)
  vmovups %ymm0, expat_xml_data+1048708(%rax)
  vmovups %ymm1, expat_xml_data+1048680(%rax)
  vmovups %ymm1, expat_xml_data+1048740(%rax)
  vmovups %ymm0, expat_xml_data+1048768(%rax)
  addq $480, %rax
  jne .LBB0_51
  vmovups %ymm0, expat_xml_data+1048348(%rip)
  vmovups %ymm1, expat_xml_data+1048320(%rip)
  vmovups %ymm1, expat_xml_data+1048380(%rip)
  vmovups %ymm0, expat_xml_data+1048408(%rip)
  vmovups %ymm1, expat_xml_data+1048440(%rip)
  vmovups %ymm0, expat_xml_data+1048468(%rip)
  vmovups %ymm1, expat_xml_data+1048500(%rip)
  vmovups %ymm0, expat_xml_data+1048528(%rip)
  vbroadcastss .LCPI0_1(%rip), %xmm0
  vmovups %xmm0, expat_xml_data+1048560(%rip)
  xorl %eax, %eax
  movabsq $1469598103934665603, %rcx
  movabsq $1099511628211, %rdx
  xorl %esi, %esi
  jmp .LBB0_54
.LBB0_53:
  movq %rax, %r8
  shlq $13, %r8
  subq %rax, %r8
  xorb $1, expat_xml_data(%r8)
  xorq %rdi, %rsi
  incq %rax
  cmpq $64, %rax
  je .LBB0_103
.LBB0_54:
  movl $expat_xml_data, %r8d
  movq %rcx, %rdi
  jmp .LBB0_59
.LBB0_102:
  movl $expat_xml_data+1048576, %r10d
.LBB0_58:
  movq %r10, %r8
  cmpq $expat_xml_data+1048576, %r10
  jae .LBB0_53
.LBB0_59:
  movzbl (%r8), %r9d
  cmpl $39, %r9d
  je .LBB0_61
  cmpl $34, %r9d
  jne .LBB0_68
.LBB0_61:
  leaq 1(%r8), %r10
  cmpq $expat_xml_data+1048576, %r10
  jae .LBB0_58
  cmpb %r9b, 1(%r8)
  je .LBB0_56
  movl $expat_xml_data+1048576, %r11d
  leaq 2(%r8), %rbx
  cmpq %r11, %rbx
  je .LBB0_102
  movl $expat_xml_data+1048574, %r11d
  subq %r8, %r11
.LBB0_65:
  cmpb %r9b, 1(%r10)
  je .LBB0_55
  incq %r10
  decq %r11
  jne .LBB0_65
  jmp .LBB0_102
.LBB0_68:
  cmpb $97, %r9b
  jb .LBB0_70
  cmpb $123, %r9b
  jae .LBB0_73
  jmp .LBB0_76
.LBB0_70:
  cmpb $65, %r9b
  jb .LBB0_73
  cmpb $91, %r9b
  jb .LBB0_76
  cmpb $95, %r9b
  je .LBB0_76
.LBB0_73:
  cmpq $58, %r9
  je .LBB0_76
  cmpb $-62, %r9b
  jae .LBB0_76
  incq %r8
  jmp .LBB0_57
.LBB0_76:
  movq %r8, %r10
  jmp .LBB0_78
.LBB0_77:
  addq %rbx, %r10
  cmpq $expat_xml_data+1048576, %r10
  jae .LBB0_100
.LBB0_78:
  movzbl (%r10), %r11d
  testb %r11b, %r11b
  js .LBB0_81
  movl $1, %ebx
  cmpb $97, %r11b
  jb .LBB0_85
  cmpb $122, %r11b
  jbe .LBB0_77
  jmp .LBB0_90
.LBB0_81:
  cmpb $-62, %r11b
  jb .LBB0_100
  cmpb $-32, %r11b
  jb .LBB0_88
  cmpb $-16, %r11b
  jae .LBB0_92
  movl $3, %ebx
  xorl %ebp, %ebp
  jmp .LBB0_94
.LBB0_85:
  cmpb $65, %r11b
  jb .LBB0_89
  cmpb $91, %r11b
  jb .LBB0_77
  cmpb $95, %r11b
  jne .LBB0_90
  jmp .LBB0_77
.LBB0_88:
  movl $2, %ebx
  xorl %ebp, %ebp
  jmp .LBB0_94
.LBB0_89:
  cmpb $58, %r11b
  je .LBB0_77
.LBB0_90:
  leal -58(%r11), %ebp
  cmpb $-11, %bpl
  ja .LBB0_77
  addb $-47, %r11b
  cmpb $-2, %r11b
  jae .LBB0_77
  jmp .LBB0_100
.LBB0_92:
  cmpb $-12, %r11b
  ja .LBB0_100
  movl $4, %ebx
  movb $1, %bpl
.LBB0_94:
  movl $expat_xml_data+1048576, %r14d
  subq %r10, %r14
  cmpq %rbx, %r14
  jb .LBB0_100
  movzbl 1(%r10), %r14d
  andb $-64, %r14b
  cmpb $-128, %r14b
  jne .LBB0_100
  cmpb $-32, %r11b
  jb .LBB0_98
  movzbl 2(%r10), %r11d
  andb $-64, %r11b
  cmpb $-128, %r11b
  jne .LBB0_100
.LBB0_98:
  testb %bpl, %bpl
  je .LBB0_77
  movzbl 3(%r10), %r11d
  andb $-64, %r11b
  cmpb $-128, %r11b
  je .LBB0_77
.LBB0_100:
  movq %r10, %r11
  subq %r8, %r11
  je .LBB0_53
  addq %r9, %r11
  xorq %rdi, %r11
  imulq %rdx, %r11
  movq %r11, %rdi
  jmp .LBB0_58
.LBB0_55:
  movq %r10, %r8
.LBB0_56:
  addq $2, %r8
.LBB0_57:
  movq %r8, %r10
  jmp .LBB0_58
.LBB0_103:
  xorl %ebx, %ebx
  movl $.L.str, %edi
  xorl %eax, %eax
  vzeroupper
  callq printf
.LBB0_104:
  movl %ebx, %eax
  addq $16, %rsp
  popq %rbx
  popq %r14
  popq %rbp
  retq

.L.str:
  .asciz "%lu\n"

check_name_kernel.ascii_name:
  .asciz "alpha-9"

check_name_kernel.utf8_name:
  .asciz "caf\303\251Tag"

make_xml_corpus.fragment:
  .asciz "<caf\303\251 data-id=\"17\" role=\"entry_42\">text &amp; more</caf\303\251>\n"

