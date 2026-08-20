main:
  movl $305419896, %ecx
  xorl %edx, %edx
  leaq gzip_crc_data(%rip), %rax
.LBB0_1:
  imull $1664525, %ecx, %esi
  addl $1013904223, %esi
  shrl $24, %esi
  movb %sil, (%rdx,%rax)
  imull $389569705, %ecx, %esi
  addl $1196435762, %esi
  shrl $24, %esi
  movb %sil, 1(%rdx,%rax)
  imull $-1354167659, %ecx, %esi
  addl $-775096599, %esi
  shrl $24, %esi
  movb %sil, 2(%rdx,%rax)
  imull $158984081, %ecx, %esi
  addl $-1426500812, %esi
  shrl $24, %esi
  movb %sil, 3(%rdx,%rax)
  imull $-1432516515, %ecx, %esi
  addl $1649599747, %esi
  shrl $24, %esi
  movb %sil, 4(%rdx,%rax)
  imull $-1083573575, %ecx, %esi
  addl $-1624324474, %esi
  shrl $24, %esi
  movb %sil, 5(%rdx,%rax)
  imull $1851289957, %ecx, %esi
  addl $1476291629, %esi
  shrl $24, %esi
  movb %sil, 6(%rdx,%rax)
  imull $-360120287, %ecx, %ecx
  addl $-1546035288, %ecx
  movl %ecx, %esi
  shrl $24, %esi
  movb %sil, 7(%rdx,%rax)
  addq $8, %rdx
  cmpq $1048576, %rdx
  jne .LBB0_1
  movl $-1, %ecx
  xorl %edx, %edx
  leaq gzip_crc32_table(%rip), %rdi
.LBB0_3:
  xorl %esi, %esi
.LBB0_4:
  movzbl (%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 1(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 2(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  movzbl 3(%rsi,%rax), %r8d
  xorb %cl, %r8b
  movzbl %r8b, %r8d
  shrl $8, %ecx
  xorl (%rdi,%r8,4), %ecx
  addq $4, %rsi
  cmpq $1048576, %rsi
  jne .LBB0_4
  movl %ecx, %esi
  notl %esi
  movq %rdx, %r8
  shlq $13, %r8
  subq %rdx, %r8
  xorb %sil, (%r8,%rax)
  incq %rdx
  cmpq $64, %rdx
  jne .LBB0_3
  pushq %rax
  leaq .L.str(%rip), %rdi
  xorl %eax, %eax
  callq printf@PLT
  xorl %eax, %eax
  popq %rcx
  retq

.L.str:
  .asciz "%08x\n"

gzip_crc32_table:
  .long 0
  .long 1996959894
  .long 3993919788
  .long 2567524794
  .long 124634137
  .long 1886057615
  .long 3915621685
  .long 2657392035
  .long 249268274
  .long 2044508324
  .long 3772115230
  .long 2547177864
  .long 162941995
  .long 2125561021
  .long 3887607047
  .long 2428444049
  .long 498536548
  .long 1789927666
  .long 4089016648
  .long 2227061214
  .long 450548861
  .long 1843258603
  .long 4107580753
  .long 2211677639
  .long 325883990
  .long 1684777152
  .long 4251122042
  .long 2321926636
  .long 335633487
  .long 1661365465
  .long 4195302755
  .long 2366115317
  .long 997073096
  .long 1281953886
  .long 3579855332
  .long 2724688242
  .long 1006888145
  .long 1258607687
  .long 3524101629
  .long 2768942443
  .long 901097722
  .long 1119000684
  .long 3686517206
  .long 2898065728
  .long 853044451
  .long 1172266101
  .long 3705015759
  .long 2882616665
  .long 651767980
  .long 1373503546
  .long 3369554304
  .long 3218104598
  .long 565507253
  .long 1454621731
  .long 3485111705
  .long 3099436303
  .long 671266974
  .long 1594198024
  .long 3322730930
  .long 2970347812
  .long 795835527
  .long 1483230225
  .long 3244367275
  .long 3060149565
  .long 1994146192
  .long 31158534
  .long 2563907772
  .long 4023717930
  .long 1907459465
  .long 112637215
  .long 2680153253
  .long 3904427059
  .long 2013776290
  .long 251722036
  .long 2517215374
  .long 3775830040
  .long 2137656763
  .long 141376813
  .long 2439277719
  .long 3865271297
  .long 1802195444
  .long 476864866
  .long 2238001368
  .long 4066508878
  .long 1812370925
  .long 453092731
  .long 2181625025
  .long 4111451223
  .long 1706088902
  .long 314042704
  .long 2344532202
  .long 4240017532
  .long 1658658271
  .long 366619977
  .long 2362670323
  .long 4224994405
  .long 1303535960
  .long 984961486
  .long 2747007092
  .long 3569037538
  .long 1256170817
  .long 1037604311
  .long 2765210733
  .long 3554079995
  .long 1131014506
  .long 879679996
  .long 2909243462
  .long 3663771856
  .long 1141124467
  .long 855842277
  .long 2852801631
  .long 3708648649
  .long 1342533948
  .long 654459306
  .long 3188396048
  .long 3373015174
  .long 1466479909
  .long 544179635
  .long 3110523913
  .long 3462522015
  .long 1591671054
  .long 702138776
  .long 2966460450
  .long 3352799412
  .long 1504918807
  .long 783551873
  .long 3082640443
  .long 3233442989
  .long 3988292384
  .long 2596254646
  .long 62317068
  .long 1957810842
  .long 3939845945
  .long 2647816111
  .long 81470997
  .long 1943803523
  .long 3814918930
  .long 2489596804
  .long 225274430
  .long 2053790376
  .long 3826175755
  .long 2466906013
  .long 167816743
  .long 2097651377
  .long 4027552580
  .long 2265490386
  .long 503444072
  .long 1762050814
  .long 4150417245
  .long 2154129355
  .long 426522225
  .long 1852507879
  .long 4275313526
  .long 2312317920
  .long 282753626
  .long 1742555852
  .long 4189708143
  .long 2394877945
  .long 397917763
  .long 1622183637
  .long 3604390888
  .long 2714866558
  .long 953729732
  .long 1340076626
  .long 3518719985
  .long 2797360999
  .long 1068828381
  .long 1219638859
  .long 3624741850
  .long 2936675148
  .long 906185462
  .long 1090812512
  .long 3747672003
  .long 2825379669
  .long 829329135
  .long 1181335161
  .long 3412177804
  .long 3160834842
  .long 628085408
  .long 1382605366
  .long 3423369109
  .long 3138078467
  .long 570562233
  .long 1426400815
  .long 3317316542
  .long 2998733608
  .long 733239954
  .long 1555261956
  .long 3268935591
  .long 3050360625
  .long 752459403
  .long 1541320221
  .long 2607071920
  .long 3965973030
  .long 1969922972
  .long 40735498
  .long 2617837225
  .long 3943577151
  .long 1913087877
  .long 83908371
  .long 2512341634
  .long 3803740692
  .long 2075208622
  .long 213261112
  .long 2463272603
  .long 3855990285
  .long 2094854071
  .long 198958881
  .long 2262029012
  .long 4057260610
  .long 1759359992
  .long 534414190
  .long 2176718541
  .long 4139329115
  .long 1873836001
  .long 414664567
  .long 2282248934
  .long 4279200368
  .long 1711684554
  .long 285281116
  .long 2405801727
  .long 4167216745
  .long 1634467795
  .long 376229701
  .long 2685067896
  .long 3608007406
  .long 1308918612
  .long 956543938
  .long 2808555105
  .long 3495958263
  .long 1231636301
  .long 1047427035
  .long 2932959818
  .long 3654703836
  .long 1088359270
  .long 936918000
  .long 2847714899
  .long 3736837829
  .long 1202900863
  .long 817233897
  .long 3183342108
  .long 3401237130
  .long 1404277552
  .long 615818150
  .long 3134207493
  .long 3453421203
  .long 1423857449
  .long 601450431
  .long 3009837614
  .long 3294710456
  .long 1567103746
  .long 711928724
  .long 3020668471
  .long 3272380065
  .long 1510334235
  .long 755167117

