# Bulk raw evidence moved out of the source tree

The files below (raw Callgrind `.cg` outputs, `.tar.xz`/`.gz` bundles, and
JSON/text dumps over 100 KiB) were produced by this branch's measurement
sessions and committed alongside their summaries. No source file, script,
test or workflow reads them. Four of them were cited by path in this
branch's own documents (`pr829-830/summary.json` and
`pr833/{zstd-initial-results,native-fresh-S47}.json` in the two 2026-10-15
follow-ups, which now point here), and the retained receipt
`pr824-final-audit/archive-repacking.json` names the seven `pr824-final-audit`
`.tar.xz` bundles below by their original paths, which the archive preserves.
The human-readable summaries and small evidence files stay in this
directory, and `pr835/ARTIFACTS.json` authenticates this index. To keep the
patch reviewable the bulk files are archived byte-exact instead:

* archive: `artifacts/evidence-archive/engineering-evidence-bulk-s59.tar.xz`
  in the delivery workspace (paths relative to the repository root)
* source commit: `25b75bf13e54c7ba315bb48a844642282b6dff36` (the last commit carrying them)
* verify: extract, then `sha256sum -c` against the table below

| sha256 | bytes | path |
|---|---:|---|
| `0ae2ab08092db8abfa77120edd494157b13538c9c12edde99ece7b5fdba53cd1` | 2366828 | `engineering/evidence/pr824-final-audit/callgrind/compiler-cost-S28-raw.tar.xz` |
| `1e1ccd4922ed082743b5de41fe018425e99ac836ec7df736b39fecfe34b30dd4` | 682690 | `engineering/evidence/pr824-final-audit/native-final15.json` |
| `21a0e17618a3b0149c2e6842464eb20ef7d68e4b725ca675dd34793f4ca21b8c` | 461007 | `engineering/evidence/pr833/ce-boundary-refresh.json` |
| `5b86a4dd10c91f942a3a451556ed83097effd266811168c18e3153001aa9d39d` | 460758 | `engineering/evidence/pr824-final-audit/pr824-implicit-fixed-rust.txt` |
| `4082e327ae92478ce78c5f37df5bb4d86799f55a31bf0d30fb85a16cfd28b38c` | 460260 | `engineering/evidence/pr824-final-audit/pr824-fpenv-rust.txt` |
| `2897f909d4ac7e3fc3f560c7d6357e2b94005ca19386ab3e56cbe0f3871a1997` | 459092 | `engineering/evidence/pr824-final-audit/rust-gather-repair.txt` |
| `02c6523b508a569224990cf0235afe500f632b7de8ed962a943610223a02d2cf` | 449678 | `engineering/evidence/pr833/zstd-initial-results.json` |
| `e81634a4c5aa25d0878bce8f022cd589eaa77022259d412fcd2ec7c00330e5eb` | 351490 | `engineering/evidence/pr833/native-S53.json` |
| `5053ba911033b7a89a7a3a13042106c4e636f1a0d73274a20a974aea066a3257` | 294502 | `engineering/evidence/pr829-830/summary.json` |
| `8ad49f93afba2ce2b20106877ecf3489da49aab8dfceb1a8b74f647507609f04` | 292295 | `engineering/evidence/pr833/native-fresh-S47.json` |
| `a1fecc401dc9552c10626a9b289470c32dda0799fed0631f5a718400512aa1d4` | 283766 | `engineering/evidence/2026-10-14D-shared-binding/relocation-contract.json` |
| `d69c5f09834955baf7dad46974d7d455a9b9c72acf684e7f322d1bbafcf3e395` | 283766 | `engineering/evidence/2026-10-14D-shared-binding/relocation-contract-final.json` |
| `f6feb49424aebabfb774d96fffd78979d76a3fbd3ae695818b81bb8e517ee769` | 279959 | `engineering/evidence/pr831-832/compiler-main-memcheck.txt` |
| `17078bb080f648716324c1aafbbff2855969c6b44ab5b0681fe7961ce9d41934` | 272236 | `engineering/evidence/pr831-832/compiler-memcheck.txt` |
| `6bf6c00e5927cb790d29b518999f28a76676763ce8d15dc929098b094839fa63` | 240856 | `engineering/evidence/pr831-832/codegen-combined.json.gz` |
| `15707043d1479dd6147e4069fcac819486baa2bdb868c5c2443b612a82cd9a4d` | 240552 | `engineering/evidence/pr833/oracle-hotspots.json` |
| `0d1917d06c2e6262e14231026a7d79eb9c17a258387a5460af5d85f0ee579fbc` | 221891 | `engineering/evidence/pr829-830/native45.json.gz` |
| `f7973430649a0122d7432c625cd19d6988eb2f3f8bb6f9a1f62a54940450ad4a` | 193808 | `engineering/evidence/pr831-832/native-final.json.gz` |
| `f60c4f9a7fb4154f6b034b559d52e07d0df55f1e22bea6c5130d7da05da98427` | 191638 | `engineering/evidence/2026-10-14D-shared-binding/oracle.json` |
| `9b0947ef3dfc19475f5f06b30d426b00b204e1aae7def8b64e9c31c2fa97d8ae` | 188053 | `engineering/evidence/pr831-832/native-original.json.gz` |
| `6fe844371f68f52caaecc968d9931d7935e5c39d0789b21ad972146efd5a4688` | 172796 | `engineering/evidence/pr824-final-audit/callgrind/main-gcc16-raw.tar.xz` |
| `5638e4ef644706ea4c5a5f6fbc5f82e21fdf2f28c35af7988f20393beb6438f4` | 165496 | `engineering/evidence/pr831-832/summary.json` |
| `c8d7053648dec639c737184b0fecc46f05c1b765e00b87e4f658c688679b32a8` | 153192 | `engineering/evidence/pr833/zstd-structural-trial.json` |
| `8fb0df014daa84afe29854f0dfba5292e8e971b9046ac8fe5ce8a4920e01a505` | 143616 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/sqlite_varint/bb-sqlite_varint.cg` |
| `46c52a62886b3fb989e7532c983b9e5ae43ecd0fb02e8efc6f25820d019a6fc8` | 143574 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/sqlite_varint/bb-sqlite_varint.cg` |
| `d785928e423288e4d9c885985a67e596bc7281300f0db896cdffc511803db490` | 143414 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/sqlite_varint/bb-sqlite_varint.cg` |
| `fcf2f2d7f14a616e88f8bec166d67c92bc7001201d2b262ac3a9b082b25f67b2` | 143146 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/linux_find_bit/bb-linux_find_bit.cg` |
| `180b009425cacdf4d2855e1f44daf342e4b94d4274e4d402b1ebb19a3ace220a` | 143113 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/sqlite_varint/bb-sqlite_varint.cg` |
| `4ff23279f3837f5212026326ec39f31f6ba112c4986f58143cd5ea81a94f9718` | 143113 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/sqlite_varint/aa-sqlite_varint.cg` |
| `ed96b9c3049e77ddcb8425a8de4adf6445e5dacaa370753c98b11e3530a01f01` | 143109 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/sqlite_varint/aa-sqlite_varint.cg` |
| `0036a37beb998ab5c9d7829ef457e80c54bb89840090574101dcdd02f3c68c66` | 143106 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/sqlite_varint/aa-sqlite_varint.cg` |
| `239c52d19fd18aa7a50ec544b244606957f34dbb26ad7f1e220bb9077432be1b` | 143096 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/glibc_memcmp/bb-glibc_memcmp.cg` |
| `630e6e1e0293c2db68958364c10bbf03c19c1ca0c1902fdcc33fcae50b2a8b95` | 143096 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/glibc_memcmp/aa-glibc_memcmp.cg` |
| `b7d2626d5403a328d8b863206a980339b9ab485ba2fb2b5f12dba1f40b01f23f` | 143086 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/glibc_memcmp/aa-glibc_memcmp.cg` |
| `a7d4960e2b37dcad97ee4b04e2f9c14b2ed53960947ceba011126e045c43f0b1` | 143076 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/glibc_memcmp/bb-glibc_memcmp.cg` |
| `2212423c115a0af07d69eb00b4fd5cff465d7265b717c7f65ac4c7bbb67cafe4` | 143036 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/sqlite_varint/aa-sqlite_varint.cg` |
| `22bb2fa4d46b092ebb02aeabc483f9c4831a91f087bd203ff077d87ee74324e0` | 143019 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/glibc_memcmp/bb-glibc_memcmp.cg` |
| `2213092755ea344b612621e81d509a731de40d0257c0d3e37f560fc63f5cc37e` | 143017 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/glibc_memcmp/bb-glibc_memcmp.cg` |
| `8233b613eb28089bbced9c9d0f369b18c463c8cf3f39f57c6f9b7cf68de56748` | 143017 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/glibc_memcmp/aa-glibc_memcmp.cg` |
| `60a1d0a518f2c6a2864eee43add0731078dbbde3890edcc98c576f320dcbfdec` | 143007 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/glibc_memcmp/aa-glibc_memcmp.cg` |
| `e1ea58c3ea8581f57d9b3f5283d172bb1c1929c391b1e8b7fc50d7ac5ae5f1f2` | 143007 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/glibc_memcmp/aa-glibc_memcmp.cg` |
| `23a4c44d0cb3c406fc4aea3e90ad754f79cfff749a99af1058a0763b100abe6b` | 142995 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/glibc_memcmp/bb-glibc_memcmp.cg` |
| `8964ce2865317bca07ab75a3ad446b0843ee56f4c2e07246997cfbe2de24f539` | 142982 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/linux_find_bit/bb-linux_find_bit.cg` |
| `8d960c57826b958cbfffda6290a9980040d6f39b39faf0f2cf749e93aba6a4db` | 142928 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/sqlite_varint/bb-sqlite_varint.cg` |
| `63e9fab54ff7f9f494e29a0f916d062ef2c8cf7731ecbb9652a9838e81e0bf75` | 142889 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/expat_xml_scan/bb-expat_xml_scan.cg` |
| `3b9760f03a59bb4acee4afc46026cc8f04650cc4dace975f36bca44386e9029d` | 142879 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/sqlite_varint/aa-sqlite_varint.cg` |
| `93f6a20a87de588796d315c5f87ad41951bb58634b478046ee556e7c5bf1d316` | 142855 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/linux_find_bit/aa-linux_find_bit.cg` |
| `36dc9b1b269baa48014d50ab17a934b367fa5371a6f6714d95adeefaeed8dd55` | 142844 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/linux_find_bit/aa-linux_find_bit.cg` |
| `812120711191528225e4cb5f029962a96ae9d89d38a187e78d544bceb76775f4` | 142842 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/linux_find_bit/bb-linux_find_bit.cg` |
| `4455ac9b1f039a744bb34d33596c124a2997459d4a140cd39f739d85e0999f21` | 142842 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/linux_find_bit/aa-linux_find_bit.cg` |
| `898507650157f50e331b833e22760699e50382ddab09e8edb799ff4c6bde9c45` | 142806 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `bbef1b771befeed7d83df3078d4c3f658d4d583957f0f24373999196327b814a` | 142792 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/gzip_crc32/bb-gzip_crc32.cg` |
| `cec4ab2f900578b9d043837374f9c3394b6b9f9892213c579c76be5a3fce9e3e` | 142782 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/sqlite_varint/bb-sqlite_varint.cg` |
| `63638b7f9c8171dd3420fa8dda17e918edeab6429c9495b299ba34712f0e104f` | 142782 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/sqlite_varint/aa-sqlite_varint.cg` |
| `83657eacb941bf4fef7581c23710bbc455f4ff6133a0aa7790ccfe780da4afbb` | 142773 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/glibc_memcmp/aa-glibc_memcmp.cg` |
| `28e06e0a34080cd2b5903ec39a0889f9775da34e2a1680dd343c3b5357996c33` | 142767 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/linux_find_bit/aa-linux_find_bit.cg` |
| `23dc22d239e961f05bcaf382099fe7c9ec1cbb993fa8b0eccca7870273594e01` | 142767 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/expat_xml_scan/bb-expat_xml_scan.cg` |
| `5226b0f0a65d91b19c5d134bdac2ee93ef35c390b4e837914990bf8c70aa1cb2` | 142760 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/linux_find_bit/aa-linux_find_bit.cg` |
| `323e16401123850b0ba8ee743bf2f7e79b5bc2b9ce9566c2dd1e1d0d8d27528f` | 142749 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `fc504bf776935ad38628638f35534fd69aa8fdc18f4d383f151bdec3e5a62192` | 142744 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/gzip_crc32/bb-gzip_crc32.cg` |
| `a9bae7c30777ffb18f284b2fa1006e9407a091fe01ee2112f728d4fcd3c0e885` | 142744 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/gzip_crc32/aa-gzip_crc32.cg` |
| `fc0482132d2d1b3a647f3372a31794fc447bc3f1f23b9cc5ef88cf2d340df43e` | 142718 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/gzip_crc32/bb-gzip_crc32.cg` |
| `76a48a881ff36dfc8874ef6ec842c2fbc5a79f52a585aab8fed64160ca8b8661` | 142718 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/gzip_crc32/aa-gzip_crc32.cg` |
| `5d443602cbaa8cb8fab9e9b7e89826b859220a3062343e03f22c99f6f7429e31` | 142663 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/gzip_crc32/aa-gzip_crc32.cg` |
| `b4a71aafe46dc6ae452e2ab6a324a0dcbbe3a6615212e3baf849535f9701fb54` | 142587 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/linux_find_bit/bb-linux_find_bit.cg` |
| `d8cd5f66554809570525a06158dff8bdd78977248a25fdea3542dabe798097d4` | 142547 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/expat_xml_scan/aa-expat_xml_scan.cg` |
| `7b5af4f44a613c496d02cd02228a3661f3b92c6c8bf8eaa62ff078992c1ae6f7` | 142541 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/linux_find_bit/bb-linux_find_bit.cg` |
| `74a461f31bd7a59f90af8fb27203968deee4fb3cefe7bfb759805a87df76c084` | 142534 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/expat_xml_scan/bb-expat_xml_scan.cg` |
| `8c9fb246106cb14ba34d1bed41c3ac26e308e6330f6f9094af30e036d7133f43` | 142534 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/expat_xml_scan/aa-expat_xml_scan.cg` |
| `0bbbf8d31183a3d7cab2428dc6f8b3042a6ddee002d6331512b0b867b0ba39ec` | 142533 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/glibc_memcmp/bb-glibc_memcmp.cg` |
| `43fbbfb51568754effc8b48060f44c098d2e83dfb5342ff4bb38904c4eebf0db` | 142524 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/expat_xml_scan/aa-expat_xml_scan.cg` |
| `27e0e006baa37a5cb8aee405a5c5667bf4d19596e9c12a91b5b8fd1866b97da7` | 142509 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `50ebff6802e32e1532d7f789cb23cf33a16c2d483a30fb8db248e4c31cb24113` | 142505 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `7978156b170ae04954e85d2e29c2032f371908140e2c1f0ddffc5c7934721d6d` | 142505 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/baseline/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `dd8173e8fc819903aa75ab5338a39615e2e1d6d5c055bee8213568a067de688f` | 142495 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `67719a5ffe8a8ad22e8d82f71ef76c165a6161f5e6e8f6ff568f78977e6c700f` | 142463 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/gzip_crc32/bb-gzip_crc32.cg` |
| `f30f141a8d9493372c581be2cd0736b22fde13346a6acd12da6a815107f4c941` | 142457 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/expat_xml_scan/aa-expat_xml_scan.cg` |
| `a3a3aa774a9fc206d16d9eb8e914366e9d62da28e19e4770009f3b0e7eda7b19` | 142456 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/gzip_crc32/bb-gzip_crc32.cg` |
| `bced9410658fa956f5bf267462385f542ea377abcdd4c6cda530761a946be130` | 142450 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/expat_xml_scan/aa-expat_xml_scan.cg` |
| `58f5cabada35f748cb4d01c28417cec3b00f29f0eab7b8c697aeb9497abe82ac` | 142443 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/gzip_crc32/aa-gzip_crc32.cg` |
| `57fcba4fc09e1793d8a3339a726370cc61d80d7455533b692edcde9c1eda1ae3` | 142436 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/expat_xml_scan/bb-expat_xml_scan.cg` |
| `5f33e5943af1fee16ff41db9abd1d577a2330d61ae347869a87e486ec7aee7df` | 142433 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `3325c08d0a0eed73a7876618ab816f33140ea99ee91121581a9ceea79cff0d73` | 142429 | `engineering/evidence/2026-10-14D-shared-binding/callgrind/gcc/gzip_crc32/aa-gzip_crc32.cg` |
| `dbd0c925dd275c906f1e5e00e2c7262aa6c9dc89ffae455be01633407c1f93eb` | 142429 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/gcc/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `e239ee00d1a5139cea51f05b327cbe3035cba4611fd7c12803580cd67a3f35ad` | 142423 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/linux_find_bit/bb-linux_find_bit.cg` |
| `c42165be5dfd510398e068b54ab1c6274013ffa2f221045a73cdc2db45b6e334` | 142423 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/linux_find_bit/aa-linux_find_bit.cg` |
| `ff60558ad323f3dcf6c88b78bfd6523f20d2b3ca65a2faad800d7b6b19d7fded` | 142422 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/clang23/gzip_crc32/aa-gzip_crc32.cg` |
| `20cb156a65f0dd8ecde9e7cfee288206efe611860556a016437e108bdf390e28` | 142399 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/expat_xml_scan/bb-expat_xml_scan.cg` |
| `d0ac79fdcd479a5389e270c76ae2430c20055851bb5e3e96654a9a503147dc49` | 142315 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `bb013eff75d253f31ea68358a20d0835ff0df4db90ab2f6ad9ece980dd315083` | 142315 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `31b7d9b189a986137aa15e0b4f4d21207e280e41c3a985771cb9bf86e9a33007` | 142228 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/zlib_ng_adler32/aa-zlib_ng_adler32.cg` |
| `f3175e1751c57ddfd70480b8c124396e18f667b188399980e2ffca1df53faaa6` | 142134 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/zlib_ng_adler32/bb-zlib_ng_adler32.cg` |
| `78483be5c9a01bb5609db382b99a1d1e928959b0f951a16fdb0c8c3f2d01e7c2` | 142119 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/expat_xml_scan/bb-expat_xml_scan.cg` |
| `914d29c6318e1bce8017d3d5c75bc75cba287003feced759f5d86c1f328255c5` | 142119 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-final/baseline/expat_xml_scan/aa-expat_xml_scan.cg` |
| `f223ad32a3a8a13d3202e8d97cc7110a5870cefd30766e2e13b98ec50d0c4f81` | 141983 | `engineering/evidence/2026-10-14D-shared-binding/callgrind-pinned/gcc16/gzip_crc32/bb-gzip_crc32.cg` |
| `a38ccdac579702624e3e9d960b04be605821526ab6305b4963cbc71e13f83104` | 124156 | `engineering/evidence/pr824-final-audit/callgrind/pr-main-raw.tar.xz` |
| `04df02fcc1d16d4678098746c3a54f68e10b6db7dd9f41f5094b472af94904b8` | 122600 | `engineering/evidence/pr824-final-audit/callgrind/wrap-range-raw.tar.xz` |
| `a6b359231d97f8a1cd641cf9a15cc01d38d2a4802cfd8d5ce043351336a62ffd` | 111972 | `engineering/evidence/pr824-final-audit/callgrind/loop-bound-raw.tar.xz` |
| `efa53a197beeae67a4d5e189938ccbe989a5b391bfd8772675865137ed4f786d` | 111192 | `engineering/evidence/pr824-final-audit/callgrind/early-byte-raw.tar.xz` |
| `21b7d7495e32946e8e43120eca161a53163e973117e754b4bddd0d82a9e53b8a` | 105392 | `engineering/evidence/2026-10-14D-shared-binding/shared-binding-base-final.json` |
| `2769042348b36e0f6ab6a53ddd7fa9e4985c04241dda6b0dc43ecaf6896d36a5` | 48012 | `engineering/evidence/pr824-final-audit/packed-callgrind-raw.tar.xz` |
| `66cbd0c513159edeb7b6edb43663a7512cfb184773b52299c9595a3d5b524dc6` | 35664 | `engineering/evidence/pr824-final-audit/readonly-placement-raw.tar.xz` |
| `4fdd81711c16ceb528270e77470d84fd7910cccc3b9f05d87d8b147e8a2bf77f` | 33952 | `engineering/evidence/pr824-final-audit/readonly-initial-raw.tar.xz` |
| `978fb1396460a303b6296425a064b1063d4158eb8488f2446a3017a8915fc774` | 33650 | `engineering/evidence/pr831-832/callgrind-main.json.gz` |
| `427efe14220d70fac2229b556d142085c811ead2b5db4836d5c1aa5925ffba18` | 13124 | `engineering/evidence/pr831-832/native-dynamic.json.gz` |
| `487acceccc78bb8f7a18056b8b31ede12d0fe9a897af16a0f87da7788e2346d8` | 10859 | `engineering/evidence/pr831-832/callgrind-clang-23.json.gz` |
| `6b031fc75cc56b07f7ee25e86822cbdf15d401f798466c7b53740065d82e110d` | 10710 | `engineering/evidence/pr831-832/callgrind-gcc-16.json.gz` |
| `1f0110dfe6c4a9d022a87f18b3a2d4d0c5a25316eaa0be1013c74c3b1d3f8894` | 7611 | `engineering/evidence/pr829-830/native45-strict-revalidation.json.gz` |
| `c76cf32a5d8c6ed68a0085e0b8ff702ddab5960a642867ca57f4572b28d00062` | 7578 | `engineering/evidence/pr829-830/native-tls-v4.json.gz` |
| `16814076510dca221455df5289efdac8303a122ad1741597774d02bb75287389` | 3829 | `engineering/evidence/pr829-830/linker-v4.json.gz` |
| `0596c524c203af33337b790e6a88d058e40de136554b0a27e8efb896654108f1` | 3822 | `engineering/evidence/pr831-832/callgrind-main-S41-index.json.gz` |
| `953777ffa8351620433c9e80b2f49e29ac45b3749efb9cc98082c99c9ece55f7` | 3610 | `engineering/evidence/pr831-832/native-replay.json.gz` |
| `f1d0cfbd5df4d3b54976d93bb98c15bdf38a9de688b4a2e7f04bb6deda484baf` | 3562 | `engineering/evidence/pr829-830/shared-pr830.json.gz` |
| `05da408edb4ffc153009866a2c9dce25216400840c8399e64151b88b62b21765` | 3202 | `engineering/evidence/pr829-830/shared-loader-pr830.json.gz` |
| `c6ca9c1d7853c06ae94dccb083cc74381e1c5c5d89766b4768be3a998bbf6af0` | 3107 | `engineering/evidence/pr829-830/shared-loader-synthesis.json.gz` |
| `b895161ab15e292ebda2a24f89d9cce5dbeca58ce7623ce60cd8380da4e3f74e` | 2160 | `engineering/evidence/pr831-832/profile.json.gz` |
| `88dbebbee06b25033190c626c71c23b5c81f1e2a137e28589146602a6a7396c1` | 2050 | `engineering/evidence/pr829-830/shared-synthesis.json.gz` |
| `cc6d2af60a95272d9e59227bdd2eb9a00f14d14b5feefd123ee0ab98f53bf45c` | 2022 | `engineering/evidence/pr829-830/intel-executed-identities.json.gz` |
| `7ca960ad8c2d2839292da48348efc5331ea540e0bc3087859940fbe8f11a4cae` | 983 | `engineering/evidence/pr829-830/tls-native-paired.json.gz` |
| `b7d258d406e1ee27b3320a309c9fa7f37c7bc4c5562b1e5cda8452a5ef572c57` | 957 | `engineering/evidence/pr829-830/struct-native-paired.json.gz` |
| `7aff7e490d9c042491fe38e50bf65ba5fe94386c5a8582008f898876f422f7b5` | 471 | `engineering/evidence/pr829-830/compiler-identities.json.gz` |
| `993e2352a5a5a42598cc12950a4256116153a1966679c857e6f87c6fc2763f08` | 370 | `engineering/evidence/pr829-830/fp-recurrence-controls.json.gz` |
| `028313f96680280a651c8418df35652ac92acd4349d6cb3851da5698dba37cdb` | 194 | `engineering/evidence/pr829-830/fp-recurrence-ablations-2.json.gz` |
