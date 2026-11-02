inc_if_sge_i32:
  cmp w1, w2
  cinc w0, w0, ge
  ret
