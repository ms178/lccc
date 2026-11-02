  .file "example.c"
  .text
  .globl pc32
  .p2align 2
  .type pc32,@function
pc32:
.Lfunc_begin0:
  .file 0 "/app""/app/example.c" md5 0xd743dd91ee816e4315bac2e7e9951039
  .cfi_startproc
  .file 1 "example.c" md5 0xd743dd91ee816e4315bac2e7e9951039
  .loc 1 1 36 prologue_end
  lsr w9, w0, #1
  mov w8, #16843009
  and w9, w9, #0x55555555
  sub w9, w0, w9
  lsr w10, w9, #2
  and w9, w9, #0x33333333
  and w10, w10, #0x33333333
  add w9, w9, w10
  add w9, w9, w9, lsr #4
  and w9, w9, #0xf0f0f0f
  mul w8, w9, w8
  lsr w0, w8, #24
.Ltmp0:
  .loc 1 1 29 is_stmt 0
  ret
.Lfunc_end0:
  .size pc32, .Lfunc_end0-pc32
  .cfi_endproc

  .globl pc64
  .p2align 2
  .type pc64,@function
pc64:
.Lfunc_begin1:
  .cfi_startproc
  .loc 1 2 46 prologue_end is_stmt 1
  lsr x9, x0, #1
  mov x8, #72340172838076673
  and x9, x9, #0x5555555555555555
  sub x9, x0, x9
  lsr x10, x9, #2
  and x9, x9, #0x3333333333333333
  and x10, x10, #0x3333333333333333
  add x9, x9, x10
  add x9, x9, x9, lsr #4
  and x9, x9, #0xf0f0f0f0f0f0f0f
  mul x8, x9, x8
  lsr x0, x8, #56
.Ltmp2:
  .loc 1 2 39 is_stmt 0
  ret
.Lfunc_end1:
  .size pc64, .Lfunc_end1-pc64
  .cfi_endproc

  .globl sw32
  .p2align 2
  .type sw32,@function
sw32:
.Lfunc_begin2:
  .cfi_startproc
  .loc 1 7 27 prologue_end is_stmt 1
  lsr w9, w0, #1
  mov w8, #16843009
  and w9, w9, #0x55555555
  sub w9, w0, w9
  lsr w10, w9, #2
  and w9, w9, #0x33333333
  and w10, w10, #0x33333333
  add w9, w9, w10
  add w9, w9, w9, lsr #4
  and w9, w9, #0xf0f0f0f
  mul w8, w9, w8
  lsr w0, w8, #24
.Ltmp4:
  .loc 1 7 2 is_stmt 0
  ret
.Lfunc_end2:
  .size sw32, .Lfunc_end2-sw32
  .cfi_endproc

  .globl sw64
  .p2align 2
  .type sw64,@function
sw64:
.Lfunc_begin3:
  .cfi_startproc
  .loc 1 13 37 prologue_end is_stmt 1
  lsr x9, x0, #1
  mov x8, #72340172838076673
  and x9, x9, #0x5555555555555555
  sub x9, x0, x9
  lsr x10, x9, #2
  and x9, x9, #0x3333333333333333
  and x10, x10, #0x3333333333333333
  add x9, x9, x10
  add x9, x9, x9, lsr #4
  and x9, x9, #0xf0f0f0f0f0f0f0f
  mul x8, x9, x8
  lsr x0, x8, #56
.Ltmp6:
  .loc 1 13 2 is_stmt 0
  ret
.Lfunc_end3:
  .size sw64, .Lfunc_end3-sw64
  .cfi_endproc

  .section .debug_loclists,"",@progbits
  .word .Ldebug_list_header_end0-.Ldebug_list_header_start0
.Ldebug_list_header_start0:
  .hword 5
  .byte 8
  .byte 0
  .word 4
.Lloclists_table_base0:
  .word .Ldebug_loc0-.Lloclists_table_base0
  .word .Ldebug_loc1-.Lloclists_table_base0
  .word .Ldebug_loc2-.Lloclists_table_base0
  .word .Ldebug_loc3-.Lloclists_table_base0
.Ldebug_loc0:
  .byte 4
  .uleb128 .Lfunc_begin0-.Lfunc_begin0
  .uleb128 .Ltmp0-.Lfunc_begin0
  .byte 1
  .byte 80
  .byte 4
  .uleb128 .Ltmp0-.Lfunc_begin0
  .uleb128 .Lfunc_end0-.Lfunc_begin0
  .byte 4
  .byte 163
  .byte 1
  .byte 80
  .byte 159
  .byte 0
.Ldebug_loc1:
  .byte 4
  .uleb128 .Lfunc_begin1-.Lfunc_begin0
  .uleb128 .Ltmp2-.Lfunc_begin0
  .byte 1
  .byte 80
  .byte 4
  .uleb128 .Ltmp2-.Lfunc_begin0
  .uleb128 .Lfunc_end1-.Lfunc_begin0
  .byte 4
  .byte 163
  .byte 1
  .byte 80
  .byte 159
  .byte 0
.Ldebug_loc2:
  .byte 4
  .uleb128 .Lfunc_begin2-.Lfunc_begin0
  .uleb128 .Ltmp4-.Lfunc_begin0
  .byte 100
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 5
  .byte 26
  .byte 28
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 3
  .byte 26
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 5
  .byte 26
  .byte 28
  .byte 50
  .byte 37
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 3
  .byte 26
  .byte 34
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 5
  .byte 26
  .byte 28
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 3
  .byte 26
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 5
  .byte 26
  .byte 28
  .byte 50
  .byte 37
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 3
  .byte 26
  .byte 34
  .byte 52
  .byte 37
  .byte 34
  .byte 16
  .byte 143
  .byte 158
  .byte 188
  .byte 120
  .byte 26
  .byte 159
  .byte 0
.Ldebug_loc3:
  .byte 4
  .uleb128 .Lfunc_begin3-.Lfunc_begin0
  .uleb128 .Ltmp6-.Lfunc_begin0
  .ascii "\211\001"
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 85
  .byte 26
  .byte 28
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 51
  .byte 26
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 85
  .byte 26
  .byte 28
  .byte 50
  .byte 37
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 51
  .byte 26
  .byte 34
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 85
  .byte 26
  .byte 28
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 51
  .byte 26
  .byte 112
  .byte 0
  .byte 112
  .byte 0
  .byte 49
  .byte 37
  .byte 16
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 213
  .byte 170
  .byte 85
  .byte 26
  .byte 28
  .byte 50
  .byte 37
  .byte 16
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 179
  .byte 230
  .byte 204
  .byte 153
  .byte 51
  .byte 26
  .byte 34
  .byte 52
  .byte 37
  .byte 34
  .byte 16
  .byte 143
  .byte 158
  .byte 188
  .byte 248
  .byte 240
  .byte 225
  .byte 195
  .byte 135
  .byte 15
  .byte 26
  .byte 159
  .byte 0
.Ldebug_list_header_end0:
  .section .debug_abbrev,"",@progbits
  .byte 1
  .byte 17
  .byte 1
  .byte 37
  .byte 37
  .byte 19
  .byte 5
  .byte 3
  .byte 37
  .byte 114
  .byte 23
  .byte 16
  .byte 23
  .byte 27
  .byte 37
  .byte 17
  .byte 27
  .byte 18
  .byte 6
  .byte 115
  .byte 23
  .ascii "\214\001"
  .byte 23
  .byte 0
  .byte 0
  .byte 2
  .byte 46
  .byte 1
  .byte 17
  .byte 27
  .byte 18
  .byte 6
  .byte 64
  .byte 24
  .byte 122
  .byte 25
  .byte 3
  .byte 37
  .byte 58
  .byte 11
  .byte 59
  .byte 11
  .byte 39
  .byte 25
  .byte 73
  .byte 19
  .byte 63
  .byte 25
  .byte 0
  .byte 0
  .byte 3
  .byte 5
  .byte 0
  .byte 2
  .byte 34
  .byte 3
  .byte 37
  .byte 58
  .byte 11
  .byte 59
  .byte 11
  .byte 73
  .byte 19
  .byte 0
  .byte 0
  .byte 4
  .byte 36
  .byte 0
  .byte 3
  .byte 37
  .byte 62
  .byte 11
  .byte 11
  .byte 11
  .byte 0
  .byte 0
  .byte 0
  .section .debug_info,"",@progbits
  .word .Ldebug_info_end0-.Ldebug_info_start0
.Ldebug_info_start0:
  .hword 5
  .byte 1
  .byte 8
  .word .debug_abbrev
  .byte 1
  .byte 0
  .hword 29
  .byte 1
  .word .Lstr_offsets_base0
  .word .Lline_table_start0
  .byte 2
  .byte 0
  .word .Lfunc_end3-.Lfunc_begin0
  .word .Laddr_table_base0
  .word .Lloclists_table_base0
  .byte 2
  .byte 0
  .word .Lfunc_end0-.Lfunc_begin0
  .byte 1
  .byte 111

  .byte 3
  .byte 1
  .byte 1

  .word 139

  .byte 3
  .byte 0
  .byte 8
  .byte 1
  .byte 1
  .word 139
  .byte 0
  .byte 2
  .byte 1
  .word .Lfunc_end1-.Lfunc_begin1
  .byte 1
  .byte 111

  .byte 5
  .byte 1
  .byte 2

  .word 139

  .byte 3
  .byte 1
  .byte 8
  .byte 1
  .byte 2
  .word 143
  .byte 0
  .byte 2
  .byte 2
  .word .Lfunc_end2-.Lfunc_begin2
  .byte 1
  .byte 111

  .byte 6
  .byte 1
  .byte 3

  .word 139

  .byte 3
  .byte 2
  .byte 8
  .byte 1
  .byte 3
  .word 139
  .byte 0
  .byte 2
  .byte 3
  .word .Lfunc_end3-.Lfunc_begin3
  .byte 1
  .byte 111

  .byte 7
  .byte 1
  .byte 9

  .word 139

  .byte 3
  .byte 3
  .byte 8
  .byte 1
  .byte 9
  .word 143
  .byte 0
  .byte 4
  .byte 4
  .byte 7
  .byte 4
  .byte 4
  .byte 9
  .byte 7
  .byte 8
  .byte 0
.Ldebug_info_end0:
  .section .debug_str_offsets,"",@progbits
  .word 44
  .hword 5
  .hword 0
.Lstr_offsets_base0:
  .section .debug_str,"MS",@progbits,1
.Linfo_string0:
  .asciz "clang version 23.1.0 (https://github.com/llvm/llvm-project.git ea7d852a70e8bdfaf601d6626a760f9771b2c4b4)"
.Linfo_string1:
  .asciz "/app/example.c"
.Linfo_string2:
  .asciz "/app"
.Linfo_string3:
  .asciz "pc32"
.Linfo_string4:
  .asciz "unsigned int"
.Linfo_string5:
  .asciz "pc64"
.Linfo_string6:
  .asciz "sw32"
.Linfo_string7:
  .asciz "sw64"
.Linfo_string8:
  .asciz "x"
.Linfo_string9:
  .asciz "unsigned long long"
  .section .debug_str_offsets,"",@progbits
  .word .Linfo_string0
  .word .Linfo_string1
  .word .Linfo_string2
  .word .Linfo_string3
  .word .Linfo_string4
  .word .Linfo_string5
  .word .Linfo_string6
  .word .Linfo_string7
  .word .Linfo_string8
  .word .Linfo_string9
  .section .debug_addr,"",@progbits
  .word .Ldebug_addr_end0-.Ldebug_addr_start0
.Ldebug_addr_start0:
  .hword 5
  .byte 8
  .byte 0
.Laddr_table_base0:
  .xword .Lfunc_begin0
  .xword .Lfunc_begin1
  .xword .Lfunc_begin2
  .xword .Lfunc_begin3
.Ldebug_addr_end0:
  .ident "clang version 23.1.0 (https://github.com/llvm/llvm-project.git ea7d852a70e8bdfaf601d6626a760f9771b2c4b4)"
  .section ".note.GNU-stack","",@progbits
  .addrsig
  .section .debug_line,"",@progbits
.Lline_table_start0:
