
## x86-64 (-O2)

| kernel | gcc | clang | base | lccc |
|---|---|---|---|---|
| `t_ne` | 4 | 4 | 4 | 4 |
| `t_eq` | 4 | 4 | 7 | 4 |
| `t_eq64` | 4 | 4 | 7 | 4 |
| `t_shr` | 5 | 4 | 5 | 4 |
| `t_shr_not` | 4 | 4 | 7 | 4 |
| `t_not_ne` | 6 | 6 | 7 | 5 |
| `t_const_mask` | 5 | 5 | 5 | 5 |
| `t_msb` | 11 | 9 | 11 | 9 |
| `b_ne` | 5 | 5 | 8 | 5 |
| `b_eq` | 5 | 5 | 8 | 5 |
| `b_shr` | 5 | 5 | 5 | 5 |
| `b_not` | 7 | 7 | 10 | 6 |
| `b_const_mask` | 5 | 4 | 8 | 4 |
| `b_idx_mask` | 6 | 7 | 8 | 6 |
| `ff_test_clear` | 12 | 6 | 8 | 6 |
| `ff_test_clear64` | 12 | 6 | 8 | 6 |
| `ff_test_set` | 8 | 6 | 10 | 6 |
| `ff_test_toggle` | 8 | 6 | 10 | 6 |
| `ff_branch_clear` | 12 | 12 | 12 | 7 |
| `ff_visit` | 15 | 14 | 17 | 14 |
| `shared_four` | 16 | 7 | 11 | 7 |
| `sqlite_maskbit` | 7 | 7 | 11 | 8 |
| `sqlite_dbmask` | 7 | 7 | 9 | 8 |
| `sqlite_uses_btree` | 11 | 13 | 12 | 12 |
| `sqlite_bitvec_test` | 8 | 8 | 8 | 8 |
| `sqlite_bitvec_set` | 8 | 7 | 7 | 7 |
| `sqlite_bitvec_clear` | 7 | 8 | 11 | 7 |
| `sctp_present` | 8 | 8 | 8 | 8 |
| `sctp_set` | 8 | 7 | 7 | 7 |
| `sctp_unset` | 7 | 8 | 9 | 7 |
| `srtp_get` | 7 | 7 | 8 | 7 |
| `srtp_set` | 7 | 6 | 7 | 6 |
| `srtp_clear` | 7 | 6 | 7 | 6 |
| `srtp_rdb_check` | 20 | 17 | 26 | 22 |
| `icu_lead3` | 8 | 9 | 9 | 9 |
| `icu_lead4` | 9 | 8 | 9 | 9 |
| `brotli_mark` | 7 | 6 | 7 | 6 |
| `brotli_trivial` | 8 | 7 | 8 | 7 |
| `ft_is32_clear` | 24 | 22 | 25 | 22 |
| `ft_is32_range_set` | 22 | 18 | 19 | 18 |
| `ft_mask_any3` | 32 | 28 | 35 | 29 |
| `ft_mask_set` | 8 | 8 | 10 | 8 |
| `nss_set_bit` | 16 | 13 | 12 | 11 |
| `nss_get_bit` | 7 | 7 | 7 | 7 |
| `libxml_occurs` | 13 | 13 | 17 | 17 |
| `lzma_count` | 30 | 43 | 21 | 17 |
| `lzma_defined_sum` | 25 | 47 | 22 | 19 |
| **total** | 480 | 468 | 507 | 414 |

## i686 (-O2)

| kernel | gcc | clang | base | lccc |
|---|---|---|---|---|
| `t_ne` | 7 | 6 | 13 | 12 |
| `t_eq` | 7 | 6 | 16 | 14 |
| `t_eq64` | 11 | 11 | 33 | 33 |
| `t_shr` | 5 | 6 | 13 | 12 |
| `t_shr_not` | 6 | 6 | 15 | 14 |
| `t_not_ne` | 8 | 8 | 20 | 15 |
| `t_const_mask` | 5 | 6 | 10 | 10 |
| `t_msb` | 12 | 10 | 19 | 18 |
| `b_ne` | 8 | 9 | 20 | 17 |
| `b_eq` | 8 | 11 | 20 | 17 |
| `b_shr` | 7 | 9 | 17 | 17 |
| `b_not` | 9 | 14 | 25 | 19 |
| `b_const_mask` | 7 | 5 | 18 | 14 |
| `b_idx_mask` | 8 | 12 | 23 | 19 |
| `ff_test_clear` | 16 | 10 | 23 | 22 |
| `ff_test_clear64` | 28 | 34 | 50 | 50 |
| `ff_test_set` | 11 | 10 | 23 | 22 |
| `ff_test_toggle` | 11 | 10 | 23 | 22 |
| `ff_branch_clear` | 16 | 17 | 30 | 24 |
| `ff_visit` | 18 | 20 | 40 | 33 |
| `shared_four` | 16 | 16 | 37 | 36 |
| `sqlite_maskbit` | 15 | 12 | 41 | 41 |
| `sqlite_dbmask` | 9 | 9 | 24 | 23 |
| `sqlite_uses_btree` | 13 | 14 | 26 | 26 |
| `sqlite_bitvec_test` | 9 | 10 | 16 | 16 |
| `sqlite_bitvec_set` | 9 | 11 | 22 | 22 |
| `sqlite_bitvec_clear` | 9 | 12 | 28 | 22 |
| `sctp_present` | 9 | 10 | 17 | 16 |
| `sctp_set` | 9 | 11 | 22 | 22 |
| `sctp_unset` | 9 | 12 | 23 | 22 |
| `srtp_get` | 9 | 9 | 25 | 21 |
| `srtp_set` | 10 | 7 | 25 | 23 |
| `srtp_clear` | 10 | 7 | 25 | 23 |
| `srtp_rdb_check` | 23 | 22 | 41 | 35 |
| `icu_lead3` | 8 | 10 | 22 | 20 |
| `icu_lead4` | 9 | 9 | 16 | 16 |
| `brotli_mark` | 9 | 7 | 25 | 23 |
| `brotli_trivial` | 9 | 9 | 25 | 21 |
| `ft_is32_clear` | 31 | 28 | 40 | 39 |
| `ft_is32_range_set` | 32 | 27 | 38 | 36 |
| `ft_mask_any3` | 38 | 31 | 49 | 49 |
| `ft_mask_set` | 9 | 11 | 20 | 20 |
| `nss_set_bit` | 18 | 12 | 38 | 37 |
| `nss_get_bit` | 9 | 9 | 26 | 21 |
| `libxml_occurs` | 15 | 16 | 28 | 24 |
| `lzma_count` | 51 | 53 | 44 | 40 |
| `lzma_defined_sum` | 37 | 60 | 38 | 35 |
| **total** | 642 | 664 | 1232 | 1133 |

## aarch64 (-O2)

| kernel | clang | base | lccc |
|---|---|---|---|
| `t_ne` | 3 | 12 | 11 |
| `t_eq` | 4 | 13 | 12 |
| `t_eq64` | 4 | 13 | 11 |
| `t_shr` | 3 | 12 | 11 |
| `t_shr_not` | 4 | 15 | 12 |
| `t_not_ne` | 5 | 18 | 13 |
| `t_const_mask` | 5 | 13 | 13 |
| `t_msb` | 7 | 17 | 17 |
| `b_ne` | 5 | 15 | 14 |
| `b_eq` | 5 | 15 | 13 |
| `b_shr` | 5 | 17 | 14 |
| `b_not` | 6 | 23 | 15 |
| `b_const_mask` | 6 | 17 | 17 |
| `b_idx_mask` | 6 | 19 | 16 |
| `ff_test_clear` | 9 | 23 | 23 |
| `ff_test_clear64` | 9 | 21 | 21 |
| `ff_test_set` | 9 | 22 | 19 |
| `ff_test_toggle` | 9 | 22 | 19 |
| `ff_branch_clear` | 11 | 23 | 22 |
| `ff_visit` | 12 | 30 | 27 |
| `shared_four` | 11 | 30 | 30 |
| `sqlite_maskbit` | 5 | 21 | 18 |
| `sqlite_dbmask` | 5 | 21 | 18 |
| `sqlite_uses_btree` | 12 | 31 | 30 |
| `sqlite_bitvec_test` | 7 | 16 | 15 |
| `sqlite_bitvec_set` | 9 | 18 | 18 |
| `sqlite_bitvec_clear` | 9 | 24 | 18 |
| `sctp_present` | 7 | 16 | 15 |
| `sctp_set` | 9 | 18 | 18 |
| `sctp_unset` | 9 | 20 | 18 |
| `srtp_get` | 5 | 15 | 13 |
| `srtp_set` | 7 | 17 | 16 |
| `srtp_clear` | 7 | 17 | 16 |
| `srtp_rdb_check` | 18 | 46 | 40 |
| `icu_lead3` | 8 | 19 | 18 |
| `icu_lead4` | 8 | 20 | 19 |
| `brotli_mark` | 8 | 21 | 17 |
| `brotli_trivial` | 6 | 18 | 13 |
| `ft_is32_clear` | 15 | 39 | 39 |
| `ft_is32_range_set` | 13 | 38 | 38 |
| `ft_mask_any3` | 23 | 44 | 44 |
| `ft_mask_set` | 9 | 20 | 20 |
| `nss_set_bit` | 12 | 30 | 30 |
| `nss_get_bit` | 5 | 15 | 15 |
| `libxml_occurs` | 13 | 37 | 37 |
| `lzma_count` | 17 | 40 | 36 |
| `lzma_defined_sum` | 20 | 37 | 37 |
| **total** | 404 | 1048 | 966 |

## riscv64 (-O2)

| kernel | clang | base | lccc |
|---|---|---|---|
| `t_ne` | 3 | 11 | 11 |
| `t_eq` | 4 | 19 | 16 |
| `t_eq64` | 4 | 17 | 14 |
| `t_shr` | 3 | 15 | 13 |
| `t_shr_not` | 4 | 19 | 16 |
| `t_not_ne` | 5 | 24 | 19 |
| `t_const_mask` | 5 | 9 | 9 |
| `t_msb` | 8 | 37 | 37 |
| `b_ne` | 6 | 37 | 33 |
| `b_eq` | 6 | 36 | 32 |
| `b_shr` | 6 | 38 | 33 |
| `b_not` | 7 | 52 | 38 |
| `b_const_mask` | 7 | 32 | 29 |
| `b_idx_mask` | 7 | 50 | 39 |
| `ff_test_clear` | 10 | 32 | 32 |
| `ff_test_clear64` | 10 | 30 | 30 |
| `ff_test_set` | 9 | 25 | 22 |
| `ff_test_toggle` | 9 | 25 | 22 |
| `ff_branch_clear` | 12 | 53 | 46 |
| `ff_visit` | 13 | 74 | 61 |
| `shared_four` | 12 | 48 | 48 |
| `sqlite_maskbit` | 7 | 23 | 17 |
| `sqlite_dbmask` | 7 | 25 | 24 |
| `sqlite_uses_btree` | 14 | 34 | 34 |
| `sqlite_bitvec_test` | 7 | 29 | 29 |
| `sqlite_bitvec_set` | 9 | 32 | 32 |
| `sqlite_bitvec_clear` | 10 | 38 | 32 |
| `sctp_present` | 7 | 30 | 29 |
| `sctp_set` | 9 | 32 | 32 |
| `sctp_unset` | 10 | 37 | 32 |
| `srtp_get` | 7 | 35 | 28 |
| `srtp_set` | 9 | 34 | 29 |
| `srtp_clear` | 10 | 35 | 30 |
| `srtp_rdb_check` | 21 | 80 | 70 |
| `icu_lead3` | 9 | 30 | 30 |
| `icu_lead4` | 9 | 33 | 33 |
| `brotli_mark` | 9 | 32 | 25 |
| `brotli_trivial` | 8 | 33 | 24 |
| `ft_is32_clear` | 19 | 87 | 87 |
| `ft_is32_range_set` | 16 | 88 | 88 |
| `ft_mask_any3` | 25 | 104 | 104 |
| `ft_mask_set` | 9 | 36 | 36 |
| `nss_set_bit` | 15 | 66 | 66 |
| `nss_get_bit` | 7 | 38 | 38 |
| `libxml_occurs` | 12 | 51 | 51 |
| `lzma_count` | 18 | 77 | 75 |
| `lzma_defined_sum` | 24 | 76 | 76 |
| **total** | 457 | 1898 | 1751 |
