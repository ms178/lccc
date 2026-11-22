//! The instructions no arm of the encoders knows, as GNU as 2.47 encodes
//! them: one row for each form of each mnemonic of the list below.
//!
//! Written by `scripts/x86_learned_rows.py --write`, which asks GNU as;
//! `--check` holds the file to what it answers and `--verify` holds what
//! `learned.rs` makes of the rows to it. Not edited by hand.
//!
//! The list:
//! aadd aand adcx adcxl adcxq adox adoxl adoxq aesdec128kl aesdec256kl
//! aesdecwide128kl aesdecwide256kl aesenc128kl aesenc256kl aesencwide128kl
//! aesencwide256kl aor axor blcfill blcfilll blcfillq blci blcic blcicl
//! blcicq blcil blciq blcmsk blcmskl blcmskq blcs blcsl blcsq blsfill
//! blsfilll blsfillq blsic blsicl blsicq cfcmovnp cfcmovnpl cfcmovnpq
//! cfcmovnpw cfcmovp cfcmovpl cfcmovpq cfcmovpw clflushopt clrssbsy clwb
//! clzero cmpaexadd cmpaxadd cmpbexadd cmpbxadd cmpcxadd cmpexadd cmpgexadd
//! cmpgxadd cmplexadd cmplxadd cmpnaexadd cmpnaxadd cmpnbexadd cmpnbxadd
//! cmpncxadd cmpnexadd cmpngexadd cmpngxadd cmpnlexadd cmpnlxadd cmpnoxadd
//! cmpnpxadd cmpnsxadd cmpnzxadd cmpoxadd cmppexadd cmppoxadd cmppxadd cmps
//! cmpsxadd cmpxchg cmpxchgb cmpxchgl cmpxchgw cmpzxadd cvtpd2pi cvtpi2pd
//! cvtpi2ps cvtps2pi cvttpd2pi cvttps2pi div divb divl divw encodekey128
//! encodekey256 endbr64 enqcmd enqcmds fldcww fnstcww fnstsww idiv idivb
//! idivl idivw imulb incsspd ins invd invept invlpga invlpgb invpcid invvpid
//! kmovb kmovd kmovq kmovw ldtilecfg llwpcb loadiwkey lods lzcnt lzcntl
//! lzcntw maskmovdqu maskmovq movbe movbel movbew movdir64b movdiri movdiril
//! movdiriq movdq2q movmskpdl movmskpdq movmskpsl movmskpsq movq2dq movrs
//! movrsb movrsl movrsq movrsw movs movsx movsxb movsxd movsxl movsxw movzx
//! movzxb movzxw nopl nopw outs pavgusb pcmpestril pcmpestriq pcmpestrml
//! pcmpestrmq pextrw pextrwl pextrwq pf2id pf2iw pfacc pfadd pfcmpeq pfcmpge
//! pfcmpgt pfmax pfmin pfmul pfnacc pfpnacc pfrcp pfrcpit1 pfrcpit2 pfrsqit1
//! pfrsqrt pfsub pfsubr pi2fd pi2fw pinsrw pinsrwl pinsrwq pmovmskb pmovmskbl
//! pmovmskbq pmulhrw pop2pq pop2q prefetch prefetchit0 prefetchit1
//! prefetchrst2 prefetchwt1 pswapd push2pq push2q pvalidate rdsspd rstorssp
//! salc salcb saveprevssp scas scmp scmpb scmpl scmpq scmpw setab setaeb
//! setael setaeq setal setaq setbb setbeb setbel setbeq setbl setbq setcb
//! setcl setcq seteb setel seteq setgb setgeb setgel setgeq setgl setgq setlb
//! setleb setlel setleq setll setlq setna setnab setnae setnaeb setnael
//! setnaeq setnal setnaq setnb setnbb setnbe setnbeb setnbel setnbeq setnbl
//! setnbq setncb setncl setncq setneb setnel setneq setng setngb setnge
//! setngeb setngel setngeq setngl setngq setnl setnlb setnle setnleb setnlel
//! setnleq setnll setnlq setnob setnol setnoq setnpb setnpl setnpq setnsb
//! setnsl setnsq setnzb setnzl setnzq setob setol setoq setpb setpe setpeb
//! setpel setpeq setpl setpo setpob setpol setpoq setpq setsb setsl setsq
//! setssbsy setzb setzl setzq slod slodb slodl slodq slodw slwpcb smov smovb
//! smovl smovq smovw ssca sscab sscal sscaq sscaw ssto sstob sstol sstoq
//! sstow stos sttilecfg sysexit sysexitl sysret sysretl t1mskc t1mskcl
//! t1mskcq tcmmimfp16ps tcmmrlfp16ps tcvtrowd2ps tcvtrowps2bf16h
//! tcvtrowps2bf16l tcvtrowps2phh tcvtrowps2phl tdpbf16ps tdpbf8ps tdpbhf8ps
//! tdpbssd tdpbsud tdpbusd tdpbuud tdpfp16ps tdphbf8ps tdphf8ps tileloadd
//! tileloaddrs tileloaddrst1 tileloaddt1 tilemovrow tilerelease tilestored
//! tilezero tzcnt tzcntl tzcntw tzmsk tzmskl tzmskq urdmsr uwrmsr v4fmaddps
//! v4fmaddss v4fnmaddps v4fnmaddss vaddbf16 vbcstnebf162ps vbcstnesh2ps
//! vbitrevb vbmacor16x16x16 vbmacxor16x16x16 vcmpnge_uspd vcmpnge_usph
//! vcmpnge_usps vcmpnge_ussd vcmpnge_ussh vcmpnge_usss vcmpngt_uspd
//! vcmpngt_usph vcmpngt_usps vcmpngt_ussd vcmpngt_ussh vcmpngt_usss
//! vcomisbf16 vcomxsd vcomxsh vcomxss vcvt2ph2bf8 vcvt2ph2bf8s vcvt2ph2hf8
//! vcvt2ph2hf8s vcvt2ps2phx vcvtbf162ibs vcvtbf162iubs vcvtbiasph2bf8
//! vcvtbiasph2bf8s vcvtbiasph2hf8 vcvtbiasph2hf8s vcvthf82ph vcvtneebf162ps
//! vcvtneeph2ps vcvtneobf162ps vcvtneoph2ps vcvtph2bf8 vcvtph2bf8s
//! vcvtph2bf8sx vcvtph2bf8sy vcvtph2bf8x vcvtph2bf8y vcvtph2hf8 vcvtph2hf8s
//! vcvtph2hf8sx vcvtph2hf8sy vcvtph2hf8x vcvtph2hf8y vcvtph2ibs vcvtph2iubs
//! vcvtps2ibs vcvtps2iubs vcvtsd2sh vcvtsd2sil vcvtsd2siq vcvtsd2usi
//! vcvtsh2sd vcvtsh2si vcvtsh2ss vcvtsh2usi vcvtsi2sd vcvtsi2sh vcvtsi2shl
//! vcvtsi2shq vcvtsi2ss vcvtss2sh vcvtss2sil vcvtss2siq vcvtss2usi
//! vcvttbf162ibs vcvttbf162iubs vcvttpd2dqs vcvttpd2dqsx vcvttpd2dqsy
//! vcvttpd2qqs vcvttpd2udqs vcvttpd2udqsx vcvttpd2udqsy vcvttpd2uqqs
//! vcvttph2ibs vcvttph2iubs vcvttps2dq vcvttps2dqs vcvttps2ibs vcvttps2iubs
//! vcvttps2qqs vcvttps2udqs vcvttps2uqqs vcvttsd2sil vcvttsd2siq vcvttsd2sis
//! vcvttsd2usi vcvttsd2usis vcvttsh2si vcvttsh2usi vcvttss2sil vcvttss2siq
//! vcvttss2sis vcvttss2usi vcvttss2usis vcvtusi2sh vcvtusi2shl vcvtusi2shq
//! vdivbf16 vdpphps vfmadd132bf16 vfmadd213bf16 vfmadd231bf16 vfmaddpd
//! vfmaddps vfmaddsd vfmaddss vfmaddsubpd vfmaddsubps vfmsub132bf16
//! vfmsub213bf16 vfmsub231bf16 vfmsubaddpd vfmsubaddps vfmsubpd vfmsubps
//! vfmsubsd vfmsubss vfnmadd132bf16 vfnmadd213bf16 vfnmadd231bf16 vfnmaddpd
//! vfnmaddps vfnmaddsd vfnmaddss vfnmsub132bf16 vfnmsub213bf16 vfnmsub231bf16
//! vfnmsubpd vfnmsubps vfnmsubsd vfnmsubss vgetexpbf16 vgetmantbf16
//! vgetmantsh vmaskmovdqu vmaxbf16 vminbf16 vminmaxsd vminmaxsh vminmaxss
//! vmload vmovhlps vmovhpd vmovhps vmovlhps vmovlpd vmovlps vmovmskpdl
//! vmovmskpdq vmovmskpsl vmovmskpsq vmovntdq vmovntdqa vmovntpd vmovntps
//! vmovq vmovrsb vmovrsd vmovrsq vmovrsw vmovw vmread vmreadl vmreadq vmrun
//! vmsave vmulbf16 vmwrite vmwritel vmwriteq vp2intersectd vp2intersectq
//! vp4dpwssd vp4dpwssds vpbroadcastmb2q vpbroadcastmw2d vpcmpequb vpcmpequd
//! vpcmpequq vpcmpequw vpcmpestri vpcmpestril vpcmpestriq vpcmpestrm
//! vpcmpestrml vpcmpestrmq vpcmpistri vpcmpistrm vpcmpleb vpcmpled vpcmpleq
//! vpcmpleub vpcmpleud vpcmpleuq vpcmpleuw vpcmplew vpcmpltb vpcmpltd
//! vpcmpltq vpcmpltub vpcmpltud vpcmpltuq vpcmpltuw vpcmpltw vpcmpneqb
//! vpcmpneqd vpcmpneqq vpcmpnequb vpcmpnequd vpcmpnequq vpcmpnequw vpcmpneqw
//! vpcmpnleb vpcmpnled vpcmpnleq vpcmpnleub vpcmpnleud vpcmpnleuq vpcmpnleuw
//! vpcmpnlew vpcmpnltb vpcmpnltd vpcmpnltq vpcmpnltub vpcmpnltud vpcmpnltuq
//! vpcmpnltuw vpcmpnltw vpcomb vpcomd vpcomeqb vpcomeqd vpcomeqq vpcomequb
//! vpcomequd vpcomequq vpcomequw vpcomeqw vpcomleb vpcomled vpcomleq
//! vpcomleub vpcomleud vpcomleuq vpcomleuw vpcomlew vpcomltb vpcomltd
//! vpcomltq vpcomltub vpcomltud vpcomltuq vpcomltuw vpcomltw vpcomneqb
//! vpcomneqd vpcomneqq vpcomnequb vpcomnequd vpcomnequq vpcomnequw vpcomneqw
//! vpcomq vpcomub vpcomud vpcomuq vpcomuw vpcomw vpextrb vpextrd vpextrw
//! vpextrwl vpextrwq vpinsrb vpinsrd vpinsrw vpinsrwl vpinsrwq vpmadcsswd
//! vpmadcswd vpmovb2m vpmovd2m vpmovm2b vpmovm2d vpmovm2q vpmovm2w vpmovmskbl
//! vpmovmskbq vpmovq2m vpmovw2m vprotb vprotd vprotq vprotw vpshab vpshad
//! vpshaq vpshaw vpshlb vpshld vpshlq vpshlw vrcp14sd vrcp14ss vrcp28sd
//! vrcp28ss vrcpbf16 vrcpss vreducebf16 vreducesh vrndscalebf16 vrndscalesh
//! vrsqrt14sd vrsqrt14ss vrsqrt28sd vrsqrt28ss vrsqrtbf16 vrsqrtss
//! vscalefbf16 vsha512msg1 vsha512msg2 vsha512rnds2 vsm3msg1 vsm3msg2
//! vsm3rnds2 vsqrtbf16 vsubbf16 vucomxsd vucomxsh vucomxss wrssd wrussd xadd
//! xaddb xaddl xaddw xrstors xsavec xsaves xsetbv

use super::learned::{
    ADDRESSED, DATA16, EGPR, M16, M32, M64, MASK, MODRM, ND, Number, ROUND, Row, SAE, SIB, SIZED,
    W, WHEN_EVEX, WHEN_VEX, ZERO, number, row,
};

/// The mnemonics that have rows, in order.
#[rustfmt::skip]
pub(super) static NAMES: [&str; 734] = [
    "aadd", "aand", "adcx", "adcxl", "adcxq", "adox", "adoxl", "adoxq", "aesdec128kl",
    "aesdec256kl", "aesdecwide128kl", "aesdecwide256kl", "aesenc128kl", "aesenc256kl",
    "aesencwide128kl", "aesencwide256kl", "aor", "axor", "blcfill", "blcfilll", "blcfillq", "blci",
    "blcic", "blcicl", "blcicq", "blcil", "blciq", "blcmsk", "blcmskl", "blcmskq", "blcs", "blcsl",
    "blcsq", "blsfill", "blsfilll", "blsfillq", "blsic", "blsicl", "blsicq", "cfcmovnp",
    "cfcmovnpl", "cfcmovnpq", "cfcmovnpw", "cfcmovp", "cfcmovpl", "cfcmovpq", "cfcmovpw",
    "clflushopt", "clrssbsy", "clwb", "clzero", "cmpaexadd", "cmpaxadd", "cmpbexadd", "cmpbxadd",
    "cmpcxadd", "cmpexadd", "cmpgexadd", "cmpgxadd", "cmplexadd", "cmplxadd", "cmpnaexadd",
    "cmpnaxadd", "cmpnbexadd", "cmpnbxadd", "cmpncxadd", "cmpnexadd", "cmpngexadd", "cmpngxadd",
    "cmpnlexadd", "cmpnlxadd", "cmpnoxadd", "cmpnpxadd", "cmpnsxadd", "cmpnzxadd", "cmpoxadd",
    "cmppexadd", "cmppoxadd", "cmppxadd", "cmps", "cmpsxadd", "cmpxchg", "cmpxchgb", "cmpxchgl",
    "cmpxchgw", "cmpzxadd", "cvtpd2pi", "cvtpi2pd", "cvtpi2ps", "cvtps2pi", "cvttpd2pi",
    "cvttps2pi", "div", "divb", "divl", "divw", "encodekey128", "encodekey256", "endbr64", "enqcmd",
    "enqcmds", "fldcww", "fnstcww", "fnstsww", "idiv", "idivb", "idivl", "idivw", "imulb",
    "incsspd", "ins", "invd", "invept", "invlpga", "invlpgb", "invpcid", "invvpid", "kmovb",
    "kmovd", "kmovq", "kmovw", "ldtilecfg", "llwpcb", "loadiwkey", "lods", "lzcnt", "lzcntl",
    "lzcntw", "maskmovdqu", "maskmovq", "movbe", "movbel", "movbew", "movdir64b", "movdiri",
    "movdiril", "movdiriq", "movdq2q", "movmskpdl", "movmskpdq", "movmskpsl", "movmskpsq",
    "movq2dq", "movrs", "movrsb", "movrsl", "movrsq", "movrsw", "movs", "movsx", "movsxb", "movsxd",
    "movsxl", "movsxw", "movzx", "movzxb", "movzxw", "nopl", "nopw", "outs", "pavgusb",
    "pcmpestril", "pcmpestriq", "pcmpestrml", "pcmpestrmq", "pextrw", "pextrwl", "pextrwq", "pf2id",
    "pf2iw", "pfacc", "pfadd", "pfcmpeq", "pfcmpge", "pfcmpgt", "pfmax", "pfmin", "pfmul", "pfnacc",
    "pfpnacc", "pfrcp", "pfrcpit1", "pfrcpit2", "pfrsqit1", "pfrsqrt", "pfsub", "pfsubr", "pi2fd",
    "pi2fw", "pinsrw", "pinsrwl", "pinsrwq", "pmovmskb", "pmovmskbl", "pmovmskbq", "pmulhrw",
    "pop2pq", "pop2q", "prefetch", "prefetchit0", "prefetchit1", "prefetchrst2", "prefetchwt1",
    "pswapd", "push2pq", "push2q", "pvalidate", "rdsspd", "rstorssp", "salc", "salcb",
    "saveprevssp", "scas", "scmp", "scmpb", "scmpl", "scmpq", "scmpw", "setab", "setaeb", "setael",
    "setaeq", "setal", "setaq", "setbb", "setbeb", "setbel", "setbeq", "setbl", "setbq", "setcb",
    "setcl", "setcq", "seteb", "setel", "seteq", "setgb", "setgeb", "setgel", "setgeq", "setgl",
    "setgq", "setlb", "setleb", "setlel", "setleq", "setll", "setlq", "setna", "setnab", "setnae",
    "setnaeb", "setnael", "setnaeq", "setnal", "setnaq", "setnb", "setnbb", "setnbe", "setnbeb",
    "setnbel", "setnbeq", "setnbl", "setnbq", "setncb", "setncl", "setncq", "setneb", "setnel",
    "setneq", "setng", "setngb", "setnge", "setngeb", "setngel", "setngeq", "setngl", "setngq",
    "setnl", "setnlb", "setnle", "setnleb", "setnlel", "setnleq", "setnll", "setnlq", "setnob",
    "setnol", "setnoq", "setnpb", "setnpl", "setnpq", "setnsb", "setnsl", "setnsq", "setnzb",
    "setnzl", "setnzq", "setob", "setol", "setoq", "setpb", "setpe", "setpeb", "setpel", "setpeq",
    "setpl", "setpo", "setpob", "setpol", "setpoq", "setpq", "setsb", "setsl", "setsq", "setssbsy",
    "setzb", "setzl", "setzq", "slod", "slodb", "slodl", "slodq", "slodw", "slwpcb", "smov",
    "smovb", "smovl", "smovq", "smovw", "ssca", "sscab", "sscal", "sscaq", "sscaw", "ssto", "sstob",
    "sstol", "sstoq", "sstow", "stos", "sttilecfg", "sysexit", "sysexitl", "sysret", "sysretl",
    "t1mskc", "t1mskcl", "t1mskcq", "tcmmimfp16ps", "tcmmrlfp16ps", "tcvtrowd2ps",
    "tcvtrowps2bf16h", "tcvtrowps2bf16l", "tcvtrowps2phh", "tcvtrowps2phl", "tdpbf16ps", "tdpbf8ps",
    "tdpbhf8ps", "tdpbssd", "tdpbsud", "tdpbusd", "tdpbuud", "tdpfp16ps", "tdphbf8ps", "tdphf8ps",
    "tileloadd", "tileloaddrs", "tileloaddrst1", "tileloaddt1", "tilemovrow", "tilerelease",
    "tilestored", "tilezero", "tzcnt", "tzcntl", "tzcntw", "tzmsk", "tzmskl", "tzmskq", "urdmsr",
    "uwrmsr", "v4fmaddps", "v4fmaddss", "v4fnmaddps", "v4fnmaddss", "vaddbf16", "vbcstnebf162ps",
    "vbcstnesh2ps", "vbitrevb", "vbmacor16x16x16", "vbmacxor16x16x16", "vcmpnge_uspd",
    "vcmpnge_usph", "vcmpnge_usps", "vcmpnge_ussd", "vcmpnge_ussh", "vcmpnge_usss", "vcmpngt_uspd",
    "vcmpngt_usph", "vcmpngt_usps", "vcmpngt_ussd", "vcmpngt_ussh", "vcmpngt_usss", "vcomisbf16",
    "vcomxsd", "vcomxsh", "vcomxss", "vcvt2ph2bf8", "vcvt2ph2bf8s", "vcvt2ph2hf8", "vcvt2ph2hf8s",
    "vcvt2ps2phx", "vcvtbf162ibs", "vcvtbf162iubs", "vcvtbiasph2bf8", "vcvtbiasph2bf8s",
    "vcvtbiasph2hf8", "vcvtbiasph2hf8s", "vcvthf82ph", "vcvtneebf162ps", "vcvtneeph2ps",
    "vcvtneobf162ps", "vcvtneoph2ps", "vcvtph2bf8", "vcvtph2bf8s", "vcvtph2bf8sx", "vcvtph2bf8sy",
    "vcvtph2bf8x", "vcvtph2bf8y", "vcvtph2hf8", "vcvtph2hf8s", "vcvtph2hf8sx", "vcvtph2hf8sy",
    "vcvtph2hf8x", "vcvtph2hf8y", "vcvtph2ibs", "vcvtph2iubs", "vcvtps2ibs", "vcvtps2iubs",
    "vcvtsd2sh", "vcvtsd2sil", "vcvtsd2siq", "vcvtsd2usi", "vcvtsh2sd", "vcvtsh2si", "vcvtsh2ss",
    "vcvtsh2usi", "vcvtsi2sd", "vcvtsi2sh", "vcvtsi2shl", "vcvtsi2shq", "vcvtsi2ss", "vcvtss2sh",
    "vcvtss2sil", "vcvtss2siq", "vcvtss2usi", "vcvttbf162ibs", "vcvttbf162iubs", "vcvttpd2dqs",
    "vcvttpd2dqsx", "vcvttpd2dqsy", "vcvttpd2qqs", "vcvttpd2udqs", "vcvttpd2udqsx", "vcvttpd2udqsy",
    "vcvttpd2uqqs", "vcvttph2ibs", "vcvttph2iubs", "vcvttps2dq", "vcvttps2dqs", "vcvttps2ibs",
    "vcvttps2iubs", "vcvttps2qqs", "vcvttps2udqs", "vcvttps2uqqs", "vcvttsd2sil", "vcvttsd2siq",
    "vcvttsd2sis", "vcvttsd2usi", "vcvttsd2usis", "vcvttsh2si", "vcvttsh2usi", "vcvttss2sil",
    "vcvttss2siq", "vcvttss2sis", "vcvttss2usi", "vcvttss2usis", "vcvtusi2sh", "vcvtusi2shl",
    "vcvtusi2shq", "vdivbf16", "vdpphps", "vfmadd132bf16", "vfmadd213bf16", "vfmadd231bf16",
    "vfmaddpd", "vfmaddps", "vfmaddsd", "vfmaddss", "vfmaddsubpd", "vfmaddsubps", "vfmsub132bf16",
    "vfmsub213bf16", "vfmsub231bf16", "vfmsubaddpd", "vfmsubaddps", "vfmsubpd", "vfmsubps",
    "vfmsubsd", "vfmsubss", "vfnmadd132bf16", "vfnmadd213bf16", "vfnmadd231bf16", "vfnmaddpd",
    "vfnmaddps", "vfnmaddsd", "vfnmaddss", "vfnmsub132bf16", "vfnmsub213bf16", "vfnmsub231bf16",
    "vfnmsubpd", "vfnmsubps", "vfnmsubsd", "vfnmsubss", "vgetexpbf16", "vgetmantbf16", "vgetmantsh",
    "vmaskmovdqu", "vmaxbf16", "vminbf16", "vminmaxsd", "vminmaxsh", "vminmaxss", "vmload",
    "vmovhlps", "vmovhpd", "vmovhps", "vmovlhps", "vmovlpd", "vmovlps", "vmovmskpdl", "vmovmskpdq",
    "vmovmskpsl", "vmovmskpsq", "vmovntdq", "vmovntdqa", "vmovntpd", "vmovntps", "vmovq", "vmovrsb",
    "vmovrsd", "vmovrsq", "vmovrsw", "vmovw", "vmread", "vmreadl", "vmreadq", "vmrun", "vmsave",
    "vmulbf16", "vmwrite", "vmwritel", "vmwriteq", "vp2intersectd", "vp2intersectq", "vp4dpwssd",
    "vp4dpwssds", "vpbroadcastmb2q", "vpbroadcastmw2d", "vpcmpequb", "vpcmpequd", "vpcmpequq",
    "vpcmpequw", "vpcmpestri", "vpcmpestril", "vpcmpestriq", "vpcmpestrm", "vpcmpestrml",
    "vpcmpestrmq", "vpcmpistri", "vpcmpistrm", "vpcmpleb", "vpcmpled", "vpcmpleq", "vpcmpleub",
    "vpcmpleud", "vpcmpleuq", "vpcmpleuw", "vpcmplew", "vpcmpltb", "vpcmpltd", "vpcmpltq",
    "vpcmpltub", "vpcmpltud", "vpcmpltuq", "vpcmpltuw", "vpcmpltw", "vpcmpneqb", "vpcmpneqd",
    "vpcmpneqq", "vpcmpnequb", "vpcmpnequd", "vpcmpnequq", "vpcmpnequw", "vpcmpneqw", "vpcmpnleb",
    "vpcmpnled", "vpcmpnleq", "vpcmpnleub", "vpcmpnleud", "vpcmpnleuq", "vpcmpnleuw", "vpcmpnlew",
    "vpcmpnltb", "vpcmpnltd", "vpcmpnltq", "vpcmpnltub", "vpcmpnltud", "vpcmpnltuq", "vpcmpnltuw",
    "vpcmpnltw", "vpcomb", "vpcomd", "vpcomeqb", "vpcomeqd", "vpcomeqq", "vpcomequb", "vpcomequd",
    "vpcomequq", "vpcomequw", "vpcomeqw", "vpcomleb", "vpcomled", "vpcomleq", "vpcomleub",
    "vpcomleud", "vpcomleuq", "vpcomleuw", "vpcomlew", "vpcomltb", "vpcomltd", "vpcomltq",
    "vpcomltub", "vpcomltud", "vpcomltuq", "vpcomltuw", "vpcomltw", "vpcomneqb", "vpcomneqd",
    "vpcomneqq", "vpcomnequb", "vpcomnequd", "vpcomnequq", "vpcomnequw", "vpcomneqw", "vpcomq",
    "vpcomub", "vpcomud", "vpcomuq", "vpcomuw", "vpcomw", "vpextrb", "vpextrd", "vpextrw",
    "vpextrwl", "vpextrwq", "vpinsrb", "vpinsrd", "vpinsrw", "vpinsrwl", "vpinsrwq", "vpmadcsswd",
    "vpmadcswd", "vpmovb2m", "vpmovd2m", "vpmovm2b", "vpmovm2d", "vpmovm2q", "vpmovm2w",
    "vpmovmskbl", "vpmovmskbq", "vpmovq2m", "vpmovw2m", "vprotb", "vprotd", "vprotq", "vprotw",
    "vpshab", "vpshad", "vpshaq", "vpshaw", "vpshlb", "vpshld", "vpshlq", "vpshlw", "vrcp14sd",
    "vrcp14ss", "vrcp28sd", "vrcp28ss", "vrcpbf16", "vrcpss", "vreducebf16", "vreducesh",
    "vrndscalebf16", "vrndscalesh", "vrsqrt14sd", "vrsqrt14ss", "vrsqrt28sd", "vrsqrt28ss",
    "vrsqrtbf16", "vrsqrtss", "vscalefbf16", "vsha512msg1", "vsha512msg2", "vsha512rnds2",
    "vsm3msg1", "vsm3msg2", "vsm3rnds2", "vsqrtbf16", "vsubbf16", "vucomxsd", "vucomxsh",
    "vucomxss", "wrssd", "wrussd", "xadd", "xaddb", "xaddl", "xaddw", "xrstors", "xsavec", "xsaves",
    "xsetbv",
];

/// Where the rows of each mnemonic of `NAMES` begin in `ROWS`, and the end.
#[rustfmt::skip]
pub(super) static STARTS: [u16; 735] = [
    0, 4, 8, 20, 26, 32, 44, 50, 56, 57, 58, 59, 60, 61, 62, 63, 64, 68, 72, 76, 78, 80, 84, 88, 90,
    92, 94, 96, 100, 102, 104, 108, 110, 112, 116, 118, 120, 124, 126, 128, 134, 136, 138, 140, 146,
    148, 150, 152, 153, 154, 155, 158, 162, 166, 170, 174, 178, 182, 186, 190, 194, 198, 202, 206,
    210, 214, 218, 222, 226, 230, 234, 238, 242, 246, 250, 254, 258, 262, 266, 270, 271, 275, 283,
    285, 287, 289, 293, 295, 297, 299, 301, 303, 305, 331, 339, 347, 355, 356, 357, 358, 361, 364,
    365, 366, 367, 393, 401, 409, 417, 421, 422, 423, 424, 427, 430, 433, 436, 439, 449, 459, 469,
    479, 481, 483, 484, 485, 497, 501, 505, 506, 507, 522, 527, 532, 535, 539, 541, 543, 544, 545,
    546, 547, 548, 549, 557, 559, 561, 563, 565, 566, 577, 583, 589, 593, 599, 608, 614, 620, 622,
    624, 625, 627, 629, 631, 633, 635, 640, 642, 644, 646, 648, 650, 652, 654, 656, 658, 660, 662,
    664, 666, 668, 670, 672, 674, 676, 678, 680, 682, 684, 686, 692, 694, 696, 700, 702, 704, 706,
    707, 708, 709, 710, 711, 712, 713, 715, 716, 717, 722, 723, 724, 725, 726, 727, 728, 729, 730,
    731, 732, 733, 737, 741, 742, 743, 744, 745, 749, 753, 754, 755, 756, 757, 761, 762, 763, 767,
    768, 769, 773, 777, 778, 779, 780, 781, 785, 789, 790, 791, 792, 793, 799, 803, 809, 813, 814,
    815, 816, 817, 823, 827, 833, 837, 838, 839, 840, 841, 845, 846, 847, 851, 852, 853, 859, 863,
    869, 873, 874, 875, 876, 877, 883, 887, 893, 897, 898, 899, 900, 901, 905, 906, 907, 911, 912,
    913, 917, 918, 919, 923, 924, 925, 929, 930, 931, 935, 941, 945, 946, 947, 948, 954, 958, 959,
    960, 961, 965, 966, 967, 968, 972, 973, 974, 975, 976, 977, 978, 979, 981, 982, 983, 984, 985,
    986, 987, 988, 989, 990, 991, 992, 993, 994, 995, 996, 997, 999, 1000, 1001, 1002, 1003, 1007,
    1009, 1011, 1012, 1013, 1015, 1017, 1019, 1021, 1023, 1024, 1025, 1026, 1027, 1028, 1029, 1030,
    1031, 1032, 1033, 1035, 1037, 1039, 1041, 1043, 1044, 1046, 1047, 1059, 1063, 1067, 1071, 1073,
    1075, 1077, 1079, 1080, 1081, 1082, 1083, 1089, 1091, 1093, 1099, 1103, 1107, 1117, 1123, 1133,
    1137, 1139, 1143, 1153, 1159, 1169, 1173, 1175, 1179, 1181, 1183, 1185, 1187, 1193, 1199, 1205,
    1211, 1217, 1223, 1229, 1235, 1241, 1247, 1253, 1259, 1261, 1263, 1265, 1267, 1271, 1275, 1277,
    1279, 1281, 1283, 1287, 1291, 1293, 1295, 1297, 1299, 1305, 1311, 1317, 1323, 1325, 1329, 1333,
    1337, 1339, 1343, 1345, 1349, 1355, 1358, 1360, 1362, 1368, 1370, 1374, 1378, 1382, 1388, 1394,
    1398, 1400, 1402, 1408, 1412, 1414, 1416, 1422, 1428, 1434, 1446, 1452, 1458, 1464, 1470, 1476,
    1482, 1486, 1490, 1494, 1498, 1502, 1506, 1510, 1514, 1518, 1522, 1526, 1530, 1533, 1535, 1537,
    1543, 1549, 1555, 1561, 1567, 1573, 1579, 1582, 1585, 1591, 1597, 1603, 1609, 1615, 1621, 1627,
    1633, 1639, 1642, 1645, 1651, 1657, 1663, 1669, 1675, 1678, 1681, 1687, 1693, 1699, 1705, 1711,
    1714, 1717, 1723, 1729, 1731, 1732, 1738, 1744, 1746, 1748, 1750, 1753, 1755, 1759, 1763, 1765,
    1769, 1773, 1775, 1777, 1779, 1781, 1786, 1791, 1796, 1801, 1813, 1816, 1819, 1822, 1825, 1830,
    1834, 1836, 1838, 1841, 1844, 1850, 1854, 1856, 1858, 1864, 1870, 1871, 1872, 1875, 1878, 1884,
    1890, 1896, 1902, 1904, 1906, 1908, 1910, 1912, 1914, 1916, 1918, 1924, 1930, 1936, 1942, 1948,
    1954, 1960, 1966, 1972, 1978, 1984, 1990, 1996, 2002, 2008, 2014, 2020, 2026, 2032, 2038, 2044,
    2050, 2056, 2062, 2068, 2074, 2080, 2086, 2092, 2098, 2104, 2110, 2116, 2122, 2128, 2134, 2140,
    2146, 2152, 2158, 2160, 2162, 2164, 2166, 2168, 2170, 2172, 2174, 2176, 2178, 2180, 2182, 2184,
    2186, 2188, 2190, 2192, 2194, 2196, 2198, 2200, 2202, 2204, 2206, 2208, 2210, 2212, 2214, 2216,
    2218, 2220, 2222, 2224, 2226, 2228, 2230, 2232, 2234, 2236, 2238, 2244, 2248, 2254, 2255, 2256,
    2262, 2266, 2272, 2273, 2274, 2276, 2278, 2281, 2284, 2287, 2290, 2293, 2296, 2298, 2300, 2303,
    2306, 2311, 2316, 2321, 2326, 2329, 2332, 2335, 2338, 2341, 2344, 2347, 2350, 2352, 2354, 2356,
    2358, 2364, 2366, 2372, 2374, 2380, 2382, 2384, 2386, 2388, 2390, 2396, 2398, 2404, 2405, 2406,
    2407, 2409, 2411, 2413, 2419, 2425, 2427, 2429, 2431, 2433, 2435, 2443, 2445, 2447, 2449, 2450,
    2451, 2452, 2453,
];

/// The rows, by mnemonic; of one mnemonic and one form, the encoding GNU as
/// chooses first.
#[rustfmt::skip]
pub(super) static ROWS: [Row; 2453] = [
    // aadd
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // aand
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 1, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 1, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 1, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 1, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // adcx
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | ND, 0, [3, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | ND, 0, [11, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32 r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | ND, 0, [11, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64 r64
    // adcxl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | ND, 0, [3, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | ND, 0, [11, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32 r32
    // adcxq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | ND, 0, [11, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 1, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64 r64
    // adox
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | ND, 0, [3, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | ND, 0, [11, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32 r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | ND, 0, [11, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64 r64
    // adoxl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | ND, 0, [3, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | ND, 0, [11, 3, 3, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32 r32
    // adoxq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | ND, 0, [11, 4, 4, 0, 0], [2, 1, 3, 0, 0], 2, 4, 2, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64 r64
    // aesdec128kl
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xDD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // aesdec256kl
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xDF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // aesdecwide128kl
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 2, 2, 0, 0xD8, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // aesdecwide256kl
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 2, 2, 0, 0xD8, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // aesenc128kl
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xDC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // aesenc256kl
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xDE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // aesencwide128kl
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 2, 2, 0, 0xD8, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // aesencwide256kl
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 2, 2, 0, 0xD8, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // aor
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 3, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 3, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 3, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 3, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // axor
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 2, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 2, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 2, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 2, 0, 0xFC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // blcfill
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcfilll
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blcfillq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blci
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcic
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcicl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blcicq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcil
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blciq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcmsk
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcmskl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blcmskq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x02, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcs
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blcsl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blcsq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blsfill
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blsfilll
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blsfillq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blsic
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // blsicl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // blsicq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // cfcmovnp
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // cfcmovnpl
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // cfcmovnpq
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // cfcmovnpw
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // cfcmovp
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // cfcmovpl
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // cfcmovpq
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // cfcmovpw
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // clflushopt
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 1, 0, 0xAE, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // clrssbsy
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 2, 0, 0xAE, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // clwb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 1, 0, 0xAE, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // clzero
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFC, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFC, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax
    row(M64, 0, [16, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFC, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax
    // cmpaexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpaxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpbexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpbxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpcxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpgexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpgxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmplexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmplxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnaexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnaxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnbexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnbxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpncxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpngexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpngxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnlexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnlxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnoxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnpxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnsxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpnzxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpoxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmppexadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmppoxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmppxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xEA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmps
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // cmpsxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cmpxchg
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 1, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // cmpxchgb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 1, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 m
    // cmpxchgl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // cmpxchgw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    // cmpzxadd
    row(MODRM | M64, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32 m
    row(W | MODRM | M64, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 1, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 11, 0, 0], [3, 1, 2, 0, 0], 2, 2, 1, 0, 0xE4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64 m
    // cvtpd2pi
    row(MODRM | M64 | M32 | M16, 0, [6, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // cvtpi2pd
    row(MODRM | M64 | M32 | M16, 0, [5, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // cvtpi2ps
    row(MODRM | M64 | M32 | M16, 0, [5, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    // cvtps2pi
    row(MODRM | M64 | M32 | M16, 0, [6, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // cvttpd2pi
    row(MODRM | M64 | M32 | M16, 0, [6, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // cvttps2pi
    row(MODRM | M64 | M32 | M16, 0, [6, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // div
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(W | MODRM | EGPR | M64, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(W | MODRM | EGPR | M64, 0, [4, 16, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 rax
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 16, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 rax
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(W | MODRM | EGPR | M64, 0, [11, 16, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m rax
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 16, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m rax
    // divb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    // divl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    // divw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    // encodekey128
    row(MODRM | SIZED | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xFA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    // encodekey256
    row(MODRM | SIZED | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xFB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    // endbr64
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 2, 0, 0x1E, -1, -1, -1, [0xFA, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    // enqcmd
    row(MODRM | M32 | ADDRESSED, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 3, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64 | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 3, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 3, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // enqcmds
    row(MODRM | M32 | ADDRESSED, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64 | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // fldcww
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xD9, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // fnstcww
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xD9, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // fnstsww
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xDD, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // idiv
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(W | MODRM | EGPR | M64, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(W | MODRM | EGPR | M64, 0, [4, 16, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 rax
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 16, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 rax
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(W | MODRM | EGPR | M64, 0, [11, 16, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m rax
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 16, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m rax
    // idivb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 al
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 13, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m al
    // idivl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 eax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 15, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m eax
    // idivw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 ax
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 14, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 1, 0, 0xF7, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m ax
    // imulb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 0, 0, 0, 0xF6, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 0, 0, 0xF6, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // incsspd
    row(MODRM | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 2, 0, 0xAE, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // ins
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // invd
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // invept
    row(MODRM | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // invlpga
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 19, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax ecx
    row(M64, 0, [16, 19, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax ecx
    // invlpgb
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFE, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 19, 22, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFE, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax ecx edx
    row(M64, 0, [16, 19, 22, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xFE, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax ecx edx
    // invpcid
    row(MODRM | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x82, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x82, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0xF2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // invvpid
    row(MODRM | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // kmovb
    row(MODRM | M64, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | M64, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(MODRM | M64, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(MODRM | M64 | WHEN_EVEX, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(MODRM | M64, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(MODRM | M64, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    // kmovd
    row(MODRM | M64, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | M64, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(W | MODRM | M64, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(W | MODRM | M64 | WHEN_EVEX, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(W | MODRM | M64, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(W | MODRM | M64, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    // kmovq
    row(W | MODRM | M64, 0, [4, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 k
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 k
    row(W | MODRM | M64, 0, [9, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r64
    row(W | MODRM | M64, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(W | MODRM | M64 | WHEN_EVEX, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(W | MODRM | M64, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(W | MODRM | M64, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    // kmovw
    row(MODRM | M64, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 k
    row(MODRM | M64, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k r32
    row(MODRM | M64, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(MODRM | M64 | WHEN_EVEX, 0, [9, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k k
    row(MODRM | M64, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [9, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k m
    row(MODRM | M64, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m k
    // ldtilecfg
    row(MODRM | M64, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 1, 2, 0, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 2, 0, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // llwpcb
    row(MODRM | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 3, 9, 0, 0, 0x12, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | M64, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 3, 9, 0, 0, 0x12, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // loadiwkey
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xDC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    // lods
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // lzcnt
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // lzcntl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // lzcntw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // maskmovdqu
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0xF7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    // maskmovq
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xF7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    // movbe
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(MODRM | SIZED | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movbel
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | SIZED | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // movbew
    row(MODRM | EGPR | M64, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | SIZED | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0xF0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // movdir64b
    row(MODRM | M32 | ADDRESSED, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | M64 | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(MODRM | EGPR | M64 | WHEN_EVEX | ADDRESSED, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movdiri
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // movdiril
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // movdiriq
    row(W | MODRM | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0xF9, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // movdq2q
    row(MODRM | M64 | M32 | M16, 0, [6, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 3, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x mm
    // movmskpdl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    // movmskpdq
    row(MODRM | EGPR | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    // movmskpsl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    // movmskpsq
    row(MODRM | EGPR | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    // movq2dq
    row(MODRM | M64 | M32 | M16, 0, [5, 6, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm x
    // movrs
    row(MODRM | M64, 0, [11, 1, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 1, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r8
    row(MODRM | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movrsb
    row(MODRM | M64, 0, [11, 1, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 1, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r8
    // movrsl
    row(MODRM | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // movrsq
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movrsw
    row(MODRM | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 1, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0x8B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // movs
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // movsx
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [1, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [1, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r32
    row(W | MODRM | EGPR | M64, 0, [1, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [2, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r32
    row(W | MODRM | EGPR | M64, 0, [2, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r64
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [3, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movsxb
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [1, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [1, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r32
    row(W | MODRM | EGPR | M64, 0, [1, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movsxd
    row(MODRM | EGPR | M64, 0, [3, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 1, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r16
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [3, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r64
    row(MODRM | EGPR | M64, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 1, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movsxl
    row(MODRM | EGPR | M64, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [3, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r64
    row(MODRM | EGPR | M64, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 0, 0, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movsxw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [2, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r32
    row(W | MODRM | EGPR | M64, 0, [2, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movzx
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [1, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [1, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r32
    row(W | MODRM | EGPR | M64, 0, [1, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [2, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r32
    row(W | MODRM | EGPR | M64, 0, [2, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movzxb
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [1, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [1, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r32
    row(W | MODRM | EGPR | M64, 0, [1, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // movzxw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [2, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r32
    row(W | MODRM | EGPR | M64, 0, [2, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // nopl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x1F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x1F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // nopw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x1F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x1F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // outs
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // pavgusb
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xBF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pcmpestril
    row(MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // pcmpestriq
    row(W | MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(W | MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // pcmpestrml
    row(MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // pcmpestrmq
    row(W | MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(W | MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 0, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // pextrw
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 5, 3, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 mm r32
    row(MODRM | EGPR | M64, 0, [12, 5, 4, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 mm r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | EGPR | M64, 0, [12, 6, 4, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 0, 3, 1, 0, 0x15, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x m
    // pextrwl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 5, 3, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 mm r32
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    // pextrwq
    row(MODRM | EGPR | M64, 0, [12, 5, 4, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 mm r64
    row(MODRM | EGPR | M64, 0, [12, 6, 4, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    // pf2id
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x1D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x1D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pf2iw
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x1C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x1C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfacc
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfadd
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfcmpeq
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfcmpge
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfcmpgt
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfmax
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfmin
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x94, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x94, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfmul
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfnacc
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x8A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfpnacc
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x8E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x8E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfrcp
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x96, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x96, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfrcpit1
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfrcpit2
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfrsqit1
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfrsqrt
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x97, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x97, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfsub
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pfsubr
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pi2fd
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x0D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x0D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pi2fw
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x0C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0x0C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pinsrw
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 3, 5, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 3, 6, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x
    row(MODRM | EGPR | M64, 0, [12, 4, 5, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 mm
    row(MODRM | EGPR | M64, 0, [12, 4, 6, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 11, 5, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // pinsrwl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 3, 5, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [12, 3, 6, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x
    // pinsrwq
    row(MODRM | EGPR | M64, 0, [12, 4, 5, 0, 0], [5, 2, 1, 0, 0], 0, 1, 0, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 mm
    row(MODRM | EGPR | M64, 0, [12, 4, 6, 0, 0], [5, 2, 1, 0, 0], 0, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x
    // pmovmskb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [5, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm r32
    row(MODRM | EGPR | M64, 0, [5, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | EGPR | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    // pmovmskbl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [5, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm r32
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    // pmovmskbq
    row(MODRM | EGPR | M64, 0, [5, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm r64
    row(MODRM | EGPR | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    // pmulhrw
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xB7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // pop2pq
    row(W | MODRM | EGPR | M64 | ND, 2, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 2, 4, 0, 0, 0x8F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 3]), // r64 r64
    // pop2q
    row(MODRM | EGPR | M64 | ND, 2, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 2, 4, 0, 0, 0x8F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 3]), // r64 r64
    // prefetch
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x0D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // prefetchit0
    row(MODRM | EGPR | M64, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x18, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // prefetchit1
    row(MODRM | EGPR | M64, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x18, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // prefetchrst2
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x18, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // prefetchwt1
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x0D, 2, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // pswapd
    row(MODRM | M64 | M32 | M16, 0, [5, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xBB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // mm mm
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 5, 0, 0, 0], [2, 1, 0, 0, 0], 0, 4, 0, 0, 0xBB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m mm
    // push2pq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 2, 4, 0, 0, 0xFF, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 3]), // r64 r64
    // push2q
    row(MODRM | EGPR | M64 | ND, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 2, 4, 0, 0, 0xFF, 6, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 3]), // r64 r64
    // pvalidate
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 3, 0, 0x01, -1, -1, -1, [0xFF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 3, 0, 0x01, -1, -1, -1, [0xFF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax
    row(M32, 0, [15, 19, 22, 0, 0], [0, 0, 0, 0, 0], 0, 1, 3, 0, 0x01, -1, -1, -1, [0xFF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax ecx edx
    row(M64, 0, [16, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 3, 0, 0x01, -1, -1, -1, [0xFF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax
    row(M64, 0, [16, 19, 22, 0, 0], [0, 0, 0, 0, 0], 0, 1, 3, 0, 0x01, -1, -1, -1, [0xFF, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax ecx edx
    // rdsspd
    row(MODRM | EGPR | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 2, 0, 0x1E, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // rstorssp
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 2, 0, 0x01, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // salc
    row(M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // salcb
    row(M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // saveprevssp
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 2, 0, 0x01, -1, -1, -1, [0xEA, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    // scas
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // scmp
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // scmpb
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // scmpl
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // scmpq
    row(W | M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // scmpw
    row(SIZED | M64 | M32 | M16 | DATA16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // setab
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setaeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setael
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setaeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setal
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setaq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setbb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setbeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setbel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setbeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setbl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setbq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setcb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setcl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setcq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // seteb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x94, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x94, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // seteq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setgb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setgeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setgel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setgeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setgl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setgq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setlb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setleb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setlel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setleq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setll
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setlq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setna
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnab
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x96, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnae
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnaeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x92, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnael
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnaeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x42, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnal
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnaq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x46, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnbb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnbe
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnbeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x97, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnbel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnbeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x47, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnbl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnbq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setncb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x93, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setncl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setncq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x43, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setneb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x95, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x95, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setneq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setng
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setngb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnge
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setngeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setngel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setngeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4C, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setngl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setngq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4E, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnl
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnlb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnle
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnleb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnlel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnleq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4F, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnll
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnlq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4D, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnob
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x91, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x41, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x91, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x41, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnol
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x41, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnoq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x41, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnpb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnpl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnpq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnsb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x99, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x99, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnsl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnsq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setnzb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x95, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x95, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setnzl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setnzq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x45, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setob
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x90, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x40, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x90, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x40, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setol
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x40, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setoq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x40, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setpb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setpe
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setpeb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setpel
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setpeq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setpl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setpo
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setpob
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x9B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setpol
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setpoq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4B, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setpq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x4A, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setsb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x98, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x48, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x98, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x48, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setsl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x48, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setsq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x48, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // setssbsy
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 2, 0, 0x01, -1, -1, -1, [0xE8, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    // setzb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x94, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [1, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0x94, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // setzl
    row(MODRM | EGPR | M64 | ND, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    // setzq
    row(W | MODRM | EGPR | M64 | ND, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 4, 3, 0, 0x44, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // slod
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // slodb
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // slodl
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // slodq
    row(W | M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // slodw
    row(SIZED | M64 | M32 | M16 | DATA16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // slwpcb
    row(MODRM | M64 | M32 | M16, 0, [3, 0, 0, 0, 0], [2, 0, 0, 0, 0], 3, 9, 0, 0, 0x12, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32
    row(W | MODRM | M64, 0, [4, 0, 0, 0, 0], [2, 0, 0, 0, 0], 3, 9, 0, 0, 0x12, 1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64
    // smov
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // smovb
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // smovl
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // smovq
    row(W | M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // smovw
    row(SIZED | M64 | M32 | M16 | DATA16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xA5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // ssca
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sscab
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sscal
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sscaq
    row(W | M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sscaw
    row(SIZED | M64 | M32 | M16 | DATA16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // ssto
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sstob
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sstol
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sstoq
    row(W | M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sstow
    row(SIZED | M64 | M32 | M16 | DATA16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // stos
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 0, 0, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sttilecfg
    row(MODRM | M64, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 1, 2, 1, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 2, 2, 1, 0, 0x49, 0, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // sysexit
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x35, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sysexitl
    row(M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x35, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sysret
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // sysretl
    row(SIZED | M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // -
    // t1mskc
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // t1mskcl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // t1mskcq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 7, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // tcmmimfp16ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tcmmrlfp16ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tcvtrowd2ps
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 2, 2, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 2, 2, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tcvtrowps2bf16h
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 3, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tcvtrowps2bf16l
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 2, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 2, 2, 0x77, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tcvtrowps2phh
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 0, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 0, 2, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tcvtrowps2phl
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 1, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x77, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tdpbf16ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 2, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbf8ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 5, 0, 0, 0xFD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbhf8ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 5, 3, 0, 0xFD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbssd
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 3, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbsud
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 2, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbusd
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpbuud
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 0, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdpfp16ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 2, 3, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdphbf8ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 5, 2, 0, 0xFD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tdphf8ps
    row(MODRM | M64, 134, [10, 10, 10, 0, 0], [3, 2, 1, 0, 0], 1, 5, 1, 0, 0xFD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t t t
    // tileloadd
    row(MODRM | SIB | M64, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    row(MODRM | SIB | EGPR | M64 | WHEN_EVEX, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 3, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    // tileloaddrs
    row(MODRM | SIB | M64, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    row(MODRM | SIB | EGPR | M64 | WHEN_EVEX, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 3, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    // tileloaddrst1
    row(MODRM | SIB | M64, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    row(MODRM | SIB | EGPR | M64 | WHEN_EVEX, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 1, 0, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    // tileloaddt1
    row(MODRM | SIB | M64, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    row(MODRM | SIB | EGPR | M64 | WHEN_EVEX, 0, [11, 10, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 1, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m t
    // tilemovrow
    row(MODRM | EGPR | M64, 0, [3, 10, 8, 0, 0], [3, 2, 1, 0, 0], 2, 2, 1, 2, 0x4A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 t z
    row(MODRM | M64, 0, [12, 10, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 1, 2, 0x07, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 t z
    // tilerelease
    row(M64, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 1, 2, 0, 0, 0x49, -1, -1, -1, [0xC0, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    // tilestored
    row(MODRM | SIB | M64, 0, [10, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 2, 2, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t m
    row(MODRM | SIB | EGPR | M64 | WHEN_EVEX, 0, [10, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 2, 2, 0, 0x4B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t m
    // tilezero
    row(MODRM | M64, 0, [10, 0, 0, 0, 0], [1, 0, 0, 0, 0], 1, 2, 3, 0, 0x49, -1, 0, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // t
    // tzcnt
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // tzcntl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 0, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // tzcntw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [2, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 2, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 2, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 1, 0, 0xF4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r16
    // tzmsk
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // tzmskl
    row(MODRM | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // tzmskq
    row(W | MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 3, 0, 0, 0], 3, 9, 0, 0, 0x01, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // urdmsr
    row(MODRM | M64, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 3, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 3, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    // uwrmsr
    row(MODRM | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 2, 2, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 4, 2, 0, 0xF8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    // v4fmaddps
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m z z
    // v4fmaddss
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x9B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x x
    // v4fnmaddps
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m z z
    // v4fnmaddss
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0xAB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x x
    // vaddbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x58, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vbcstnebf162ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 2, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 2, 1, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vbcstnesh2ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 0, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 1, 0xB1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vbitrevb
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x81, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vbmacor16x16x16
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y y
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z z
    // vbmacxor16x16x16
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(W | MODRM | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y y
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x80, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z z
    // vcmpnge_uspd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vcmpnge_usph
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z k
    // vcmpnge_usps
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 1, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 2, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vcmpnge_ussd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x k
    // vcmpnge_ussh
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x k
    // vcmpnge_usss
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0xC2, -1, -1, 9, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x k
    // vcmpngt_uspd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vcmpngt_usph
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 0, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z k
    // vcmpngt_usps
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 1, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 2, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vcmpngt_ussd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x k
    // vcmpngt_ussh
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x k
    // vcmpngt_usss
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | M64 | M32 | M16 | SAE, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0xC2, -1, -1, 10, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x k
    // vcomisbf16
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x
    // vcomxsd
    row(W | MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x
    // vcomxsh
    row(MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x
    // vcomxss
    row(MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x
    // vcvt2ph2bf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vcvt2ph2bf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vcvt2ph2hf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vcvt2ph2hf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vcvt2ps2phx
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 1, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 2, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 1, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 2, 0x67, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z z
    // vcvtbf162ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvtbf162iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvtbiasph2bf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z y
    // vcvtbiasph2bf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z y
    // vcvtbiasph2hf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z y
    // vcvtbiasph2hf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z y
    // vcvthf82ph
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x1E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m z
    // vcvtneebf162ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 2, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 2, 1, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vcvtneeph2ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 1, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vcvtneobf162ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 1, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vcvtneoph2ps
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 0, 0, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 0, 1, 0xB0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    // vcvtph2bf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m y
    // vcvtph2bf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m y
    // vcvtph2bf8sx
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    // vcvtph2bf8sy
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m x
    // vcvtph2bf8x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    // vcvtph2bf8y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x74, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m x
    // vcvtph2hf8
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m y
    // vcvtph2hf8s
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m y
    // vcvtph2hf8sx
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    // vcvtph2hf8sy
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x1B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m x
    // vcvtph2hf8x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    // vcvtph2hf8y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x18, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m x
    // vcvtph2ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvtph2iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvtps2ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvtps2iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvtsd2sh
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x5A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 3, 0, 0x5A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsd2sil
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND | WHEN_EVEX, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    // vcvtsd2siq
    row(W | MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | EGPR | M64 | ROUND | WHEN_EVEX, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtsd2usi
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | ROUND, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtsh2sd
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x5A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x5A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsh2si
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | ROUND, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtsh2ss
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsh2usi
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | ROUND, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtsi2sd
    row(MODRM | M64 | M32 | M16, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 x x
    row(W | MODRM | M64, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 x x
    row(W | MODRM | EGPR | M64 | ROUND | WHEN_EVEX, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 3, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsi2sh
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r32 x x
    row(W | MODRM | EGPR | M64 | ROUND, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsi2shl
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r32 x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsi2shq
    row(W | MODRM | EGPR | M64 | ROUND, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(W | MODRM | EGPR | M64, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtsi2ss
    row(MODRM | M64 | M32 | M16, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 x x
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND | WHEN_EVEX, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r32 x x
    row(W | MODRM | M64, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 x x
    row(W | MODRM | EGPR | M64 | ROUND | WHEN_EVEX, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtss2sh
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | ROUND, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x1D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 0, 0, 0x1D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtss2sil
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND | WHEN_EVEX, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    // vcvtss2siq
    row(W | MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | EGPR | M64 | ROUND | WHEN_EVEX, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtss2usi
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | ROUND, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttbf162ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvttbf162iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvttpd2dqs
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m y
    // vcvttpd2dqsx
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x
    // vcvttpd2dqsy
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m x
    // vcvttpd2qqs
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z
    // vcvttpd2udqs
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z y
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m y
    // vcvttpd2udqsx
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x
    // vcvttpd2udqsy
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m x
    // vcvttpd2uqqs
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z
    // vcvttph2ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvttph2iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vcvttps2dq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | M64 | M32 | M16, 0, [6, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y
    row(MODRM | M64 | M32 | M16, 0, [7, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y x
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | WHEN_EVEX, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 2, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 1, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 2, 0x5B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvttps2dqs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvttps2ibs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvttps2iubs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvttps2qqs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [7, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 2, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m z
    // vcvttps2udqs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 0, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z
    // vcvttps2uqqs
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [7, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 2, 2, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m z
    // vcvttsd2sil
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | EGPR | M64 | M32 | M16 | SAE | WHEN_EVEX, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    // vcvttsd2siq
    row(W | MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | EGPR | M64 | SAE | WHEN_EVEX, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttsd2sis
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttsd2usi
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttsd2usis
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttsh2si
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttsh2usi
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttss2sil
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | EGPR | M64 | M32 | M16 | SAE | WHEN_EVEX, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    // vcvttss2siq
    row(W | MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | EGPR | M64 | SAE | WHEN_EVEX, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttss2sis
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttss2usi
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvttss2usis
    row(MODRM | EGPR | M64 | M32 | M16 | SAE, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(W | MODRM | EGPR | M64 | SAE, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r32
    row(W | MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m r64
    // vcvtusi2sh
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r32 x x
    row(W | MODRM | EGPR | M64 | ROUND, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtusi2shl
    row(MODRM | EGPR | M64 | M32 | M16 | ROUND, 0, [3, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r32 x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vcvtusi2shq
    row(W | MODRM | EGPR | M64 | ROUND, 0, [4, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 0, 0]), // r64 x x
    row(W | MODRM | EGPR | M64, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 2, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vdivbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vdpphps
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 0, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 1, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 2, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 0, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 1, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 0, 2, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z z
    // vfmadd132bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmadd213bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xA8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmadd231bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xB8, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmaddpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x69, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmaddps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmaddsd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfmaddss
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfmaddsubpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmaddsubps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmsub132bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmsub213bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmsub231bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfmsubaddpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmsubaddps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x5E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmsubpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x6D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmsubps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x6C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfmsubsd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfmsubss
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfnmadd132bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmadd213bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmadd231bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmaddpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfnmaddps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfnmaddsd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfnmaddss
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfnmsub132bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x9E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmsub213bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xAE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmsub231bf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0xBE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vfnmsubpd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x7D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfnmsubps
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y y
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 7, 7, 0], [4, 2, 3, 1, 0], 1, 3, 1, 1, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m y y
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 7, 7, 7, 0], [2, 4, 3, 1, 0], 1, 3, 1, 1, 0x7C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y y y
    // vfnmsubsd
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vfnmsubss
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 1, 3, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 6, 0], [2, 4, 3, 1, 0], 1, 3, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x x
    // vgetexpbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x42, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vgetmantbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 7, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 8, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 2, 0]), // i8 m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 2, 0]), // i8 m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x26, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 2, 0]), // i8 m z
    // vgetmantsh
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x27, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x27, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vmaskmovdqu
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0xF7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    // vmaxbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vminbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vminmaxsd
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vminmaxsh
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vminmaxss
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vmload
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDA, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDA, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax
    row(M64, 0, [16, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDA, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax
    // vmovhlps
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    // vmovhpd
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x17, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x17, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vmovhps
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 0, 0x17, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 0, 0x17, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vmovlhps
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    // vmovlpd
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 1, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 1, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vmovlps
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 0, 0x13, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 0, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 1, 0, 0, 0x12, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vmovmskpdl
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [7, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 1, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r32
    // vmovmskpdq
    row(MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | M64, 0, [7, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 1, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r64
    // vmovmskpsl
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [7, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 1, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r32
    // vmovmskpsq
    row(MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 0, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | M64, 0, [7, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 0, 1, 0x50, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r64
    // vmovntdq
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 1, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 1, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // y m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [8, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 2, 0xE7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // z m
    // vmovntdqa
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 1, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 1, 1, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 1, 1, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 1, 2, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vmovntpd
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 1, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m
    row(W | MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 1, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // y m
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [8, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 2, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // z m
    // vmovntps
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 0, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 0, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 0, 1, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [7, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 1, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // y m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [8, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 0, 2, 0x2B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // z m
    // vmovq
    row(W | MODRM | M64, 0, [4, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 x
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [4, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 x
    row(W | MODRM | M64, 0, [6, 4, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [6, 4, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | M64 | M32 | M16 | WHEN_EVEX, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 1, 1, 1, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m
    row(W | MODRM | M32 | M16 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0xD6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 1, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 2, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x
    row(W | MODRM | M32 | M16 | WHEN_EVEX, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x
    row(W | MODRM | EGPR | M64 | WHEN_EVEX, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x
    // vmovrsb
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vmovrsd
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vmovrsq
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 1, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 2, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vmovrsw
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 0, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 1, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y
    row(W | MODRM | MASK | ZERO | EGPR | M64, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 3, 2, 0x6F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z
    // vmovw
    row(MODRM | EGPR | M64 | M32 | M16, 0, [3, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [1, 2, 0, 0, 0], 2, 5, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [6, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 5, 1, 0, 0x7E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // x m
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x6E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x
    // vmread
    row(MODRM | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // vmreadl
    row(MODRM | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // vmreadq
    row(MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0x78, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // vmrun
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xD8, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xD8, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax
    row(M64, 0, [16, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xD8, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax
    // vmsave
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDB, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
    row(M32, 0, [15, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDB, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // eax
    row(M64, 0, [16, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xDB, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // rax
    // vmulbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x59, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vmwrite
    row(MODRM | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    row(MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // vmwritel
    row(MODRM | M32 | M16, 0, [3, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | M32 | M16, 0, [11, 3, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r32
    // vmwriteq
    row(MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(MODRM | EGPR | M64, 0, [11, 4, 0, 0, 0], [2, 1, 0, 0, 0], 0, 1, 0, 0, 0x79, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m r64
    // vp2intersectd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vp2intersectq
    row(W | MODRM | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 0, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 1, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x68, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vp4dpwssd
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m z z
    // vp4dpwssds
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 2, 3, 2, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m z z
    // vpbroadcastmb2q
    row(W | MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(W | MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(W | MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x2A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpbroadcastmw2d
    row(MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x3A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x3A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x3A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpcmpequb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpequd
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpequq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpequw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 0, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpestri
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpestril
    row(MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpestriq
    row(W | MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(W | MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x61, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpestrm
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpestrml
    row(MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpestrmq
    row(W | MODRM | M64, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(W | MODRM | M64, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x60, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpistri
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x63, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpistrm
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x62, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 1, 3, 1, 0, 0x62, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x
    // vpcmpleb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpled
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpleq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpleub
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpleud
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpleuq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpleuw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmplew
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 2, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpltb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpltd
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpltq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpltub
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpltud
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpltuq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpltuw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpltw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 1, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpneqb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpneqd
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpneqq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnequb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnequd
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpnequq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnequw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpneqw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 4, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnleb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnled
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpnleq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnleub
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnleud
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpnleuq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnleuw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnlew
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 6, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnltb
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnltd
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpnltq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnltub
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnltud
    row(MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 4, 2, 0, 0, 0, 0]), // m x k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 8, 2, 0, 0, 0, 0]), // m y k
    row(MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 16, 2, 0, 0, 0, 0]), // m z k
    // vpcmpnltuq
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 2, 3, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 4, 3, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x1E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 8, 3, 0, 0, 0, 0]), // m z k
    // vpcmpnltuw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3E, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcmpnltw
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [6, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [7, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y k
    row(W | MODRM | MASK | M64 | M32 | M16, 0, [8, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 6, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 0, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [4, 0, 0, 0, 0, 0, 0]), // m x k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 7, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 1, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [5, 0, 0, 0, 0, 0, 0]), // m y k
    row(W | MODRM | MASK | EGPR | M64 | M32 | M16, 0, [11, 8, 9, 0, 0], [2, 3, 1, 0, 0], 2, 3, 1, 2, 0x3F, -1, -1, 5, [0x00, 0x00, 0x00], 0, [6, 0, 0, 0, 0, 0, 0]), // m z k
    // vpcomb
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomd
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomeqb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomeqd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomeqq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomequb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomequd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomequq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomequw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomeqw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 4, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomled
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleub
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleud
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleuq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomleuw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomlew
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltub
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltud
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltuq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltuw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomltw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 0, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomneqb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCC, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomneqd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCE, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomneqq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCF, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomnequb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEC, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomnequd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEE, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomnequq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xEF, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomnequw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xED, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomneqw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 8, 0, 0, 0xCD, -1, -1, 5, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpcomq
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomub
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomud
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomuq
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xEF, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomuw
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xED, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpcomw
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 3, 8, 0, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpextrb
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 3, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | M64, 0, [12, 6, 4, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [12, 6, 4, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x14, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x m
    // vpextrd
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 3, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x16, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 2, 0]), // i8 x m
    // vpextrw
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 2, 1, 0, 0], 1, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 3, 0, 0], [5, 2, 1, 0, 0], 2, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    row(MODRM | M64, 0, [12, 6, 4, 0, 0], [5, 2, 1, 0, 0], 1, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [12, 6, 4, 0, 0], [5, 2, 1, 0, 0], 2, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 1, 3, 1, 0, 0x15, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x m
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 6, 11, 0, 0], [5, 1, 2, 0, 0], 2, 3, 1, 0, 0x15, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 x m
    // vpextrwl
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 3, 0, 0], [5, 2, 1, 0, 0], 1, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r32
    // vpextrwq
    row(MODRM | M64, 0, [12, 6, 4, 0, 0], [5, 2, 1, 0, 0], 1, 1, 1, 0, 0xC5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x r64
    // vpinsrb
    row(MODRM | M64 | M32 | M16, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | M64, 0, [12, 4, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x x
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [12, 4, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x20, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpinsrd
    row(MODRM | M64 | M32 | M16, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0x22, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x22, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0x22, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 1, 0, 0x22, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpinsrw
    row(MODRM | M64 | M32 | M16, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 1, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 2, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    row(MODRM | M64, 0, [12, 4, 6, 6, 0], [5, 2, 3, 1, 0], 1, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x x
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [12, 4, 6, 6, 0], [5, 2, 3, 1, 0], 2, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 1, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    row(MODRM | EGPR | M64 | M32 | M16 | WHEN_EVEX, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vpinsrwl
    row(MODRM | M64 | M32 | M16, 0, [12, 3, 6, 6, 0], [5, 2, 3, 1, 0], 1, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r32 x x
    // vpinsrwq
    row(MODRM | M64, 0, [12, 4, 6, 6, 0], [5, 2, 3, 1, 0], 1, 1, 1, 0, 0xC4, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 r64 x x
    // vpmadcsswd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [4, 2, 3, 1, 0], 3, 8, 0, 0, 0xA6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 3, 8, 0, 0, 0xA6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    // vpmadcswd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 6, 0], [4, 2, 3, 1, 0], 3, 8, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 6, 0], [4, 2, 3, 1, 0], 3, 8, 0, 0, 0xB6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x x
    // vpmovb2m
    row(MODRM | M64 | M32 | M16, 0, [6, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x k
    row(MODRM | M64 | M32 | M16, 0, [7, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y k
    row(MODRM | M64 | M32 | M16, 0, [8, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z k
    // vpmovd2m
    row(MODRM | M64 | M32 | M16, 0, [6, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x k
    row(MODRM | M64 | M32 | M16, 0, [7, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y k
    row(MODRM | M64 | M32 | M16, 0, [8, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z k
    // vpmovm2b
    row(MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpmovm2d
    row(MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpmovm2q
    row(W | MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(W | MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(W | MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x38, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpmovm2w
    row(W | MODRM | M64 | M32 | M16, 0, [9, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k x
    row(W | MODRM | M64 | M32 | M16, 0, [9, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k y
    row(W | MODRM | M64 | M32 | M16, 0, [9, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x28, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // k z
    // vpmovmskbl
    row(MODRM | M64 | M32 | M16, 0, [6, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r32
    row(MODRM | M64 | M32 | M16, 0, [7, 3, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 1, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r32
    // vpmovmskbq
    row(MODRM | M64, 0, [6, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 0, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x r64
    row(MODRM | M64, 0, [7, 4, 0, 0, 0], [2, 1, 0, 0, 0], 1, 1, 1, 1, 0xD7, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y r64
    // vpmovq2m
    row(W | MODRM | M64 | M32 | M16, 0, [6, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x k
    row(W | MODRM | M64 | M32 | M16, 0, [7, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y k
    row(W | MODRM | M64 | M32 | M16, 0, [8, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x39, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z k
    // vpmovw2m
    row(W | MODRM | M64 | M32 | M16, 0, [6, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 0, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x k
    row(W | MODRM | M64 | M32 | M16, 0, [7, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 1, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y k
    row(W | MODRM | M64 | M32 | M16, 0, [8, 9, 0, 0, 0], [2, 1, 0, 0, 0], 2, 2, 2, 2, 0x29, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z k
    // vprotb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x90, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 m x
    // vprotd
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x92, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC2, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 m x
    // vprotq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x93, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC3, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 m x
    // vprotw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x91, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 3, 8, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 1, 0]), // i8 m x
    // vpshab
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x98, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshad
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x9A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshaq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x9B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x9B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x9B, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshaw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x99, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x99, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x99, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshlb
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x94, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x94, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x94, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshld
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x96, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x96, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x96, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshlq
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x97, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x97, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x97, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vpshlw
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x95, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [6, 11, 6, 0, 0], [3, 2, 1, 0, 0], 3, 9, 0, 0, 0x95, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x m x
    row(W | MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 3, 9, 0, 0, 0x95, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vrcp14sd
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vrcp14ss
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4D, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vrcp28sd
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vrcp28ss
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vrcpbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x4C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vrcpss
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x53, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vreducebf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 7, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 8, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 2, 0]), // i8 m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 2, 0]), // i8 m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x56, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 2, 0]), // i8 m z
    // vreducesh
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x57, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x57, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vrndscalebf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 6, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 7, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [12, 8, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 0, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 2, 0]), // i8 m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 7, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 1, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 2, 0]), // i8 m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 8, 0, 0], [5, 2, 1, 0, 0], 2, 3, 3, 2, 0x08, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 2, 0]), // i8 m z
    // vrndscalesh
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x0A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 1, 0, 2, 0]), // i8 x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 2, 3, 0, 0, 0x0A, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vrsqrt14sd
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vrsqrt14ss
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0x4F, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vrsqrt28sd
    row(W | MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(W | MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x x
    // vrsqrt28ss
    row(MODRM | MASK | ZERO | M64 | M32 | M16 | SAE, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 2, 1, 0, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x x
    // vrsqrtbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 0, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 1, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 6, 0, 2, 0x4E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vrsqrtss
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 1, 2, 0, 0x52, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vscalefbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 0, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 1, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 6, 0, 2, 0x2C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vsha512msg1
    row(MODRM | M64 | M32 | M16, 0, [6, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 1, 0xCC, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y
    // vsha512msg2
    row(MODRM | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 1, 2, 3, 1, 0xCD, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    // vsha512rnds2
    row(MODRM | M64 | M32 | M16, 0, [6, 7, 7, 0, 0], [2, 3, 1, 0, 0], 1, 2, 3, 1, 0xCB, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x y y
    // vsm3msg1
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 2, 0, 0, 0xDA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 2, 0, 0, 0xDA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vsm3msg2
    row(MODRM | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 2, 1, 0, 0xDA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 1, 2, 1, 0, 0xDA, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m x x
    // vsm3rnds2
    row(MODRM | M64 | M32 | M16, 0, [12, 6, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0xDE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 x x x
    row(MODRM | M64 | M32 | M16, 0, [12, 11, 6, 6, 0], [5, 2, 3, 1, 0], 1, 3, 1, 0, 0xDE, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 2, 0]), // i8 m x x
    // vsqrtbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 0, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 1, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 1, 2, 0x51, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z
    // vsubbf16
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [6, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x x
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [7, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // y y y
    row(MODRM | MASK | ZERO | M64 | M32 | M16, 0, [8, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // z z z
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 6, 6, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 0, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [4, 8, 1, 0, 0, 0, 0]), // m x x
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 7, 7, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 1, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [5, 16, 1, 0, 0, 0, 0]), // m y y
    row(MODRM | MASK | ZERO | EGPR | M64 | M32 | M16, 0, [11, 8, 8, 0, 0], [2, 3, 1, 0, 0], 2, 5, 1, 2, 0x5C, -1, -1, -1, [0x00, 0x00, 0x00], 0, [6, 32, 1, 0, 0, 0, 0]), // m z z
    // vucomxsd
    row(W | MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(W | MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 3, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [3, 0, 0, 0, 0, 0, 0]), // m x
    // vucomxsh
    row(MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 5, 2, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [1, 0, 0, 0, 0, 0, 0]), // m x
    // vucomxss
    row(MODRM | M64 | M32 | M16 | SAE, 0, [6, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // x x
    row(MODRM | EGPR | M64 | M32 | M16, 0, [11, 6, 0, 0, 0], [2, 1, 0, 0, 0], 2, 1, 2, 0, 0x2E, -1, -1, -1, [0x00, 0x00, 0x00], 0, [2, 0, 0, 0, 0, 0, 0]), // m x
    // wrssd
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 0, 0, 0xF6, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 0, 0, 0x66, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // wrussd
    row(MODRM | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 2, 1, 0, 0xF5, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(MODRM | EGPR | M64 | WHEN_EVEX, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 2, 4, 1, 0, 0x65, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // xadd
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 1, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    row(W | MODRM | EGPR | M64, 0, [4, 4, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 r64
    row(W | MODRM | EGPR | M64, 0, [4, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r64 m
    // xaddb
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 1, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 r8
    row(MODRM | EGPR | M64 | M32 | M16, 0, [1, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC0, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r8 m
    // xaddl
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 3, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 r32
    row(MODRM | SIZED | EGPR | M64 | M32 | M16, 0, [3, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r32 m
    // xaddw
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 2, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 r16
    row(MODRM | SIZED | EGPR | M64 | M32 | M16 | DATA16, 0, [2, 11, 0, 0, 0], [1, 2, 0, 0, 0], 0, 1, 0, 0, 0xC1, -1, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // r16 m
    // xrstors
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0xC7, 3, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // xsavec
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0xC7, 4, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // xsaves
    row(MODRM | M64 | M32 | M16, 0, [11, 0, 0, 0, 0], [2, 0, 0, 0, 0], 0, 1, 0, 0, 0xC7, 5, -1, -1, [0x00, 0x00, 0x00], 0, [0, 0, 0, 0, 0, 0, 0]), // m
    // xsetbv
    row(M64 | M32 | M16, 0, [0, 0, 0, 0, 0], [0, 0, 0, 0, 0], 0, 1, 0, 0, 0x01, -1, -1, -1, [0xD1, 0x00, 0x00], 1, [0, 0, 0, 0, 0, 0, 0]), // -
];

/// The mnemonics of GNU as's table whose one-byte number has a range, in order:
/// of every encoder's instructions, not of the rows above alone.
#[rustfmt::skip]
pub(super) static NUMBER_NAMES: [&str; 263] = [
    "aad", "aam", "aeskeygenassist", "blendpd", "blendps", "bt", "btc", "btr", "bts", "cmppd",
    "cmpps", "cmpsd", "cmpss", "dppd", "dpps", "extractps", "gf2p8affineinvqb", "gf2p8affineqb",
    "hreset", "in", "insertps", "int", "kshiftlb", "kshiftld", "kshiftlq", "kshiftlw", "kshiftrb",
    "kshiftrd", "kshiftrq", "kshiftrw", "mpsadbw", "out", "palignr", "pblendw", "pclmulqdq",
    "pcmpestri", "pcmpestrm", "pcmpistri", "pcmpistrm", "pextrb", "pextrd", "pextrq", "pextrw",
    "pinsrb", "pinsrd", "pinsrq", "pinsrw", "pshufd", "pshufhw", "pshuflw", "pshufw", "pslld",
    "pslldq", "psllq", "psllw", "psrad", "psraw", "psrld", "psrldq", "psrlq", "psrlw", "rcl", "rcr",
    "rol", "ror", "rorx", "roundpd", "roundps", "roundsd", "roundss", "sal", "sar", "sha1rnds4",
    "shl", "shld", "shr", "shrd", "shufpd", "shufps", "tcvtrowd2ps", "tcvtrowps2bf16h",
    "tcvtrowps2bf16l", "tcvtrowps2phh", "tcvtrowps2phl", "tilemovrow", "vaeskeygenassist",
    "valignd", "valignq", "vblendpd", "vblendps", "vcmpbf16", "vcmppd", "vcmpph", "vcmpps",
    "vcmpsd", "vcmpsh", "vcmpss", "vcvtps2ph", "vdbpsadbw", "vdppd", "vdpps", "vextractf128",
    "vextractf32x4", "vextractf32x8", "vextractf64x2", "vextractf64x4", "vextracti128",
    "vextracti32x4", "vextracti32x8", "vextracti64x2", "vextracti64x4", "vextractps", "vfixupimmpd",
    "vfixupimmps", "vfixupimmsd", "vfixupimmss", "vfpclassbf16", "vfpclassbf16x", "vfpclassbf16y",
    "vfpclassbf16z", "vfpclasspd", "vfpclasspdx", "vfpclasspdy", "vfpclasspdz", "vfpclassph",
    "vfpclassphx", "vfpclassphy", "vfpclassphz", "vfpclassps", "vfpclasspsx", "vfpclasspsy",
    "vfpclasspsz", "vfpclasssd", "vfpclasssh", "vfpclassss", "vgetmantbf16", "vgetmantpd",
    "vgetmantph", "vgetmantps", "vgetmantsd", "vgetmantsh", "vgetmantss", "vgf2p8affineinvqb",
    "vgf2p8affineqb", "vinsertf128", "vinsertf32x4", "vinsertf32x8", "vinsertf64x2", "vinsertf64x4",
    "vinserti128", "vinserti32x4", "vinserti32x8", "vinserti64x2", "vinserti64x4", "vinsertps",
    "vminmaxbf16", "vminmaxpd", "vminmaxph", "vminmaxps", "vminmaxsd", "vminmaxsh", "vminmaxss",
    "vmpsadbw", "vpalignr", "vpblendd", "vpblendw", "vpclmulqdq", "vpcmpb", "vpcmpd", "vpcmpestri",
    "vpcmpestrm", "vpcmpistri", "vpcmpistrm", "vpcmpq", "vpcmpub", "vpcmpud", "vpcmpuq", "vpcmpuw",
    "vpcmpw", "vpcomb", "vpcomd", "vpcomq", "vpcomub", "vpcomud", "vpcomuq", "vpcomuw", "vpcomw",
    "vperm2f128", "vperm2i128", "vpermil2pd", "vpermil2ps", "vpermilpd", "vpermilps", "vpermpd",
    "vpermq", "vpextrb", "vpextrd", "vpextrq", "vpextrw", "vpinsrb", "vpinsrd", "vpinsrq",
    "vpinsrw", "vprold", "vprolq", "vprord", "vprorq", "vprotb", "vprotd", "vprotq", "vprotw",
    "vpshldd", "vpshldq", "vpshldw", "vpshrdd", "vpshrdq", "vpshrdw", "vpshufd", "vpshufhw",
    "vpshuflw", "vpslld", "vpslldq", "vpsllq", "vpsllw", "vpsrad", "vpsraq", "vpsraw", "vpsrld",
    "vpsrldq", "vpsrlq", "vpsrlw", "vpternlogd", "vpternlogq", "vrangepd", "vrangeps", "vrangesd",
    "vrangess", "vreducebf16", "vreducepd", "vreduceph", "vreduceps", "vreducesd", "vreducesh",
    "vreducess", "vrndscalebf16", "vrndscalepd", "vrndscaleph", "vrndscaleps", "vrndscalesd",
    "vrndscalesh", "vrndscaless", "vroundpd", "vroundps", "vroundsd", "vroundss", "vshuff32x4",
    "vshuff64x2", "vshufi32x4", "vshufi64x2", "vshufpd", "vshufps", "vsm3rnds2", "xabort",
];

/// Where the ranges of each mnemonic of `NUMBER_NAMES` begin in `NUMBERS`, and the
/// end.
#[rustfmt::skip]
pub(super) static NUMBER_STARTS: [u16; 264] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
    52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 73, 83, 93, 103, 104, 105, 106, 107, 108, 118,
    128, 129, 139, 140, 150, 151, 152, 153, 154, 155, 156, 157, 158, 159, 160, 161, 162, 163, 164,
    165, 166, 167, 168, 169, 170, 171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183,
    184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196, 197, 198, 199, 200, 201, 202,
    203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221,
    222, 223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235, 236, 242, 243, 244, 245,
    246, 247, 248, 249, 250, 251, 252, 253, 254, 255, 256, 257, 258, 259, 260, 261, 262, 263, 264,
    265, 266, 267, 268, 269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 280, 281, 282, 283,
    284, 285, 286, 287, 288, 289, 290, 291, 292, 293, 294, 295, 296, 297, 298, 299, 300, 301, 302,
    303, 304, 305, 306, 307, 308, 309, 310, 311, 312, 313, 314, 315, 316, 317, 318, 319, 320, 321,
    322, 323, 324, 325, 326, 327, 328, 329, 330, 331, 332, 333, 334, 335, 336, 337, 338, 339, 340,
    341, 342,
];

/// The ranges: of one form of a mnemonic (the kinds of its operands), or of every
/// form it has (no kinds).
#[rustfmt::skip]
pub(super) static NUMBERS: [Number; 342] = [
    number(M32 | M16, [0, 0, 0, 0, 0], 2), // aad *: 0..=255
    number(M32 | M16, [0, 0, 0, 0, 0], 2), // aam *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // aeskeygenassist *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // blendpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // blendps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // bt *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // btc *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // btr *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // bts *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // cmppd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // cmpps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // cmpsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // cmpss *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // dppd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // dpps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // extractps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // gf2p8affineinvqb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // gf2p8affineqb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // hreset *: 0..=255
    number(M64 | M32 | M16, [12, 14, 0, 0, 0], 2), // in i8 ax: 0..=255
    number(M64 | M32 | M16, [12, 15, 0, 0, 0], 2), // in i8 eax: 0..=255
    number(M64 | M32 | M16, [12, 0, 0, 0, 0], 2), // in i8: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // insertps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // int *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftlb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftld *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftlq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftlw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftrb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftrd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftrq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // kshiftrw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // mpsadbw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // out *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // palignr *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pblendw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pclmulqdq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pcmpestri *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pcmpestrm *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pcmpistri *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pcmpistrm *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pextrb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pextrd *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // pextrq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pextrw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pinsrb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pinsrd *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // pinsrq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pinsrw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pshufd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pshufhw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pshuflw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // pshufw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pslld *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // pslldq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psllq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psllw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psrad *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psraw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psrld *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psrldq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psrlq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // psrlw *: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // rcl i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // rcl i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // rcl i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // rcl i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // rcl i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // rcl i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // rcl i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // rcl i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // rcl i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // rcl i8 m r64: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // rcr i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // rcr i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // rcr i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // rcr i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // rcr i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // rcr i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // rcr i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // rcr i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // rcr i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // rcr i8 m r64: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 1), // rol i8 r16: -128..=255
    number(M64, [12, 2, 2, 0, 0], 1), // rol i8 r16 r16: -128..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 1), // rol i8 r32: -128..=255
    number(M64, [12, 3, 3, 0, 0], 1), // rol i8 r32 r32: -128..=255
    number(M64, [12, 4, 0, 0, 0], 1), // rol i8 r64: -128..=255
    number(M64, [12, 4, 4, 0, 0], 1), // rol i8 r64 r64: -128..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 1), // rol i8 m: -128..=255
    number(M64, [12, 11, 2, 0, 0], 1), // rol i8 m r16: -128..=255
    number(M64, [12, 11, 3, 0, 0], 1), // rol i8 m r32: -128..=255
    number(M64, [12, 11, 4, 0, 0], 1), // rol i8 m r64: -128..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 1), // ror i8 r16: -128..=255
    number(M64, [12, 2, 2, 0, 0], 1), // ror i8 r16 r16: -128..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 1), // ror i8 r32: -128..=255
    number(M64, [12, 3, 3, 0, 0], 1), // ror i8 r32 r32: -128..=255
    number(M64, [12, 4, 0, 0, 0], 1), // ror i8 r64: -128..=255
    number(M64, [12, 4, 4, 0, 0], 1), // ror i8 r64 r64: -128..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 1), // ror i8 m: -128..=255
    number(M64, [12, 11, 2, 0, 0], 1), // ror i8 m r16: -128..=255
    number(M64, [12, 11, 3, 0, 0], 1), // ror i8 m r32: -128..=255
    number(M64, [12, 11, 4, 0, 0], 1), // ror i8 m r64: -128..=255
    number(M64, [0, 0, 0, 0, 0], 1), // rorx *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // roundpd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // roundps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // roundsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // roundss *: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // sal i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // sal i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // sal i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // sal i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // sal i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // sal i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // sal i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // sal i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // sal i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // sal i8 m r64: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // sar i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // sar i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // sar i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // sar i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // sar i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // sar i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // sar i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // sar i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // sar i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // sar i8 m r64: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // sha1rnds4 *: -128..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // shl i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // shl i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // shl i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // shl i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // shl i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // shl i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // shl i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // shl i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // shl i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // shl i8 m r64: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // shld *: 0..=255
    number(M64 | M32 | M16, [12, 2, 0, 0, 0], 2), // shr i8 r16: 0..=255
    number(M64, [12, 2, 2, 0, 0], 2), // shr i8 r16 r16: 0..=255
    number(M64 | M32 | M16, [12, 3, 0, 0, 0], 2), // shr i8 r32: 0..=255
    number(M64, [12, 3, 3, 0, 0], 2), // shr i8 r32 r32: 0..=255
    number(M64, [12, 4, 0, 0, 0], 2), // shr i8 r64: 0..=255
    number(M64, [12, 4, 4, 0, 0], 2), // shr i8 r64 r64: 0..=255
    number(M64 | M32 | M16, [12, 11, 0, 0, 0], 2), // shr i8 m: 0..=255
    number(M64, [12, 11, 2, 0, 0], 2), // shr i8 m r16: 0..=255
    number(M64, [12, 11, 3, 0, 0], 2), // shr i8 m r32: 0..=255
    number(M64, [12, 11, 4, 0, 0], 2), // shr i8 m r64: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // shrd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // shufpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // shufps *: -128..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tcvtrowd2ps *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tcvtrowps2bf16h *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tcvtrowps2bf16l *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tcvtrowps2phh *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tcvtrowps2phl *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // tilemovrow *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vaeskeygenassist *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // valignd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // valignq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vblendpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vblendps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpbf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmppd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpsh *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcmpss *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vcvtps2ph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vdbpsadbw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vdppd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vdpps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractf128 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractf32x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractf32x8 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractf64x2 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractf64x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextracti128 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextracti32x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextracti32x8 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextracti64x2 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextracti64x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vextractps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfixupimmpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfixupimmps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfixupimmsd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfixupimmss *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vfpclassbf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vfpclassbf16x *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vfpclassbf16y *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vfpclassbf16z *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspdx *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspdy *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspdz *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassph *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassphx *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassphy *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassphz *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspsx *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspsy *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasspsz *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasssd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclasssh *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vfpclassss *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantbf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantpd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantsh *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgetmantss *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgf2p8affineinvqb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vgf2p8affineqb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertf128 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertf32x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertf32x8 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertf64x2 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertf64x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinserti128 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinserti32x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinserti32x8 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinserti64x2 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinserti64x4 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vinsertps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxbf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxpd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxsh *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vminmaxss *: 0..=255
    number(M64 | M32 | M16, [12, 6, 6, 6, 0], 1), // vmpsadbw i8 x x x: -128..=255
    number(M64 | M32 | M16, [12, 7, 7, 7, 0], 1), // vmpsadbw i8 y y y: -128..=255
    number(M64 | M32 | M16, [12, 8, 8, 8, 0], 2), // vmpsadbw i8 z z z: 0..=255
    number(M64 | M32 | M16, [12, 11, 6, 6, 0], 1), // vmpsadbw i8 m x x: -128..=255
    number(M64 | M32 | M16, [12, 11, 7, 7, 0], 1), // vmpsadbw i8 m y y: -128..=255
    number(M64 | M32 | M16, [12, 11, 8, 8, 0], 2), // vmpsadbw i8 m z z: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpalignr *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpblendd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpblendw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpclmulqdq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpestri *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpestrm *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpistri *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpistrm *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpub *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpud *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpuq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpuw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcmpw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomub *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomud *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomuq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomuw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpcomw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vperm2f128 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vperm2i128 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 4), // vpermil2pd *: 0..=15
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 4), // vpermil2ps *: 0..=15
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpermilpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpermilps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpermpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpermq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpextrb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpextrd *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // vpextrq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpextrw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpinsrb *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpinsrd *: 0..=255
    number(M64, [0, 0, 0, 0, 0], 2), // vpinsrq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpinsrw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprold *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprolq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprord *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprorq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprotb *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprotd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprotq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vprotw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshldd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshldq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshldw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshrdd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshrdq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpshrdw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpshufd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpshufhw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpshuflw *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpslld *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpslldq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsllq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsllw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsrad *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsraq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsraw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsrld *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsrldq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsrlq *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vpsrlw *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpternlogd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vpternlogq *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrangepd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrangeps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrangesd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrangess *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreducebf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreducepd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreduceph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreduceps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreducesd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreducesh *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vreducess *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscalebf16 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscalepd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscaleph *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscaleps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscalesd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscalesh *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vrndscaless *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vroundpd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vroundps *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vroundsd *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vroundss *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshuff32x4 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshuff64x2 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshufi32x4 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshufi64x2 *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshufpd *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 1), // vshufps *: -128..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // vsm3rnds2 *: 0..=255
    number(M64 | M32 | M16, [0, 0, 0, 0, 0], 2), // xabort *: 0..=255
];
