
## lccc vs oracles — tune_probe.c (-O2 -march=x86-64-v3 -mtune=raptorlake)

| function | lccc | gcc16.2 | clang | icx |
|---|---|---|---|---|
| `popcnt_sum` | popcnt branch n=12 | popcnt branch n=12 | ymm_copy vzeroupper popcnt branch n=107 | ymm_copy vzeroupper popcnt branch n=42 |
| `ctz_of` | tzcnt_lzcnt n=3 | tzcnt_lzcnt n=2 | tzcnt_lzcnt n=2 | tzcnt_lzcnt n=2 |
| `clz_of` | tzcnt_lzcnt n=3 | tzcnt_lzcnt n=2 | tzcnt_lzcnt n=2 | tzcnt_lzcnt n=2 |
| `shl_var` | n=3 | shlx n=2 | n=2 | n=2 |
| `shr_var` | n=3 | shlx n=2 | n=2 | n=2 |
| `sar_var` | n=3 | shlx n=2 | n=2 | n=2 |
| `rot_hash` | branch n=12 | shlx branch n=12 | branch n=43 | branch n=38 |
| `copy_200` | ymm_copy vzeroupper n=16 | ymm_copy vzeroupper n=16 | vzeroupper n=16 | vzeroupper n=16 |
| `copy_2112` | rep_movsb n=3 | call_memcpy n=5 | n=2 | n=2 |
| `copy_4096` | rep_movsb n=3 | call_memcpy n=5 | n=2 | n=2 |
| `copy_8192` | rep_movsb n=3 | call_memcpy n=5 | n=2 | n=2 |
