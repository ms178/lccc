# GNU gzip 1.14 end-to-end workload

**Evidence:** VM wall-clock screening only; no PMU or bare-metal claim.

Archive SHA-256: `01a7b881bd220bfdf615f97b8718f80bdfd3f6add385b993dcf6efd14e8c0ac6`; upstream test suites: 30/30 for each build.
Pinned CPU: `0`; rounds: 11; warmups: 2.

| case | compiler | median ms | ratio to gcc | best ms | worst ms |
|---|---|---:|---:|---:|---:|
| compress-source-l1 | lccc | 76.543 | 1.25288 | 73.068 | 89.780 |
| compress-source-l1 | gcc | 61.094 | 1.00000 | 58.900 | 77.557 |
| compress-source-l6 | lccc | 214.138 | 1.15562 | 176.074 | 222.669 |
| compress-source-l6 | gcc | 185.301 | 1.00000 | 157.533 | 214.746 |
| compress-source-l9 | lccc | 525.103 | 1.14874 | 421.402 | 577.719 |
| compress-source-l9 | gcc | 457.111 | 1.00000 | 402.973 | 523.391 |
| compress-mixed-l6 | lccc | 148.655 | 1.26995 | 120.626 | 157.800 |
| compress-mixed-l6 | gcc | 117.056 | 1.00000 | 93.333 | 125.495 |
| decompress-source-l6 | lccc | 22.861 | 1.07232 | 21.456 | 28.465 |
| decompress-source-l6 | gcc | 21.319 | 1.00000 | 19.110 | 27.005 |

| compiler | arithmetic mean ratio | geometric mean ratio |
|---|---:|---:|
| lccc | 1.179904 | 1.177639 |
| gcc | 1.000000 | 1.000000 |


The current archpkgbuilds recipe checksum differs from the repeatedly fetched
archive digest above; the archive signature was not verified in this run.
