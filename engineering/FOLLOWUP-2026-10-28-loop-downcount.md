# Follow-up 2026-10-29 (S60): Dead-IV-Downcount und unsigned-byte AND-Facts

## Ergebnis und aktueller Stand

Der Inflight-Patch ist auf `ms178/lccc`-Main
`ed928575365e1c56a03e040d53ab5dc5af83a860` rebasiert. Die zentrale neue
Transformation ist ein eng begrenzter, standardmäßig aktiver Downcount für
bereits invertierte, dead-IV-Schleifen mit konstanter positiver Tripzahl. Sie
reduziert im ursprünglichen Reproducer nachweislich dynamische Arbeit und
Laufzeit. Die ebenfalls enthaltene Erweiterung der Upper-Zero-Facts von 0x7f
auf 0xff ist semantisch begründet und exhaustive getestet, hat aber **keinen
robust nachgewiesenen Workloadgewinn** und wird nicht als
Performanceverbesserung verkauft.

Die Produktions- und Messergebnisse dieses Berichts gehören zum Code-Stand
`f166df999a7112cdd158e02b1d78f4e210704a44`. Der nachfolgende
Dokumentationscommit ändert keinen Compilercode. Primärdaten liegen unter
`/home/user/artifacts/S60-rebased-ed928575/`; das separate Auslieferungs-
Receipt bindet den abschließenden Fast-CI-Lauf an den finalen Tree.

Wichtig für die Einordnung: Der Reproducer-Gewinn ist kein pauschaler
Performancevorsprung gegen GCC. Die neuen workload-abgeleiteten Messungen
zeigen weiterhin deutliche Missed Optimizations, unter anderem bei SQLite-,
glibc- und zlib-ng-Kerneln. Correctness und präzise, negative wie positive
Messergebnisse haben Vorrang vor einer aggregierten Erfolgsmeldung.

## Rebase und Bewertung der fremden Änderungen

Die Rebase-Arbeit übernahm die fremden Qualifier-, Pointer-, GCC-Diagnostic-,
Driver-Query-, Builtin-, Macro-Hygiene-, Pedantic-/VLA- und
Option-Effect-Ledger-Änderungen. Diese sind Correctness- und
Diagnostikverschärfungen; kein fremder Produktionsfix wurde zugunsten dieses
Patches verworfen. Konflikte wurden dateiweise beziehungsweise hunkweise
aufgelöst, nicht pauschal zugunsten einer Seite.

Die neuesten Upstream-Commits `225c96e03`, `5d2736bbe` und der Merge
`ed9285753` erweitern `self_zext` um XMM→GPR-/Self-Zext-Fusion und
Upper-Zero-Facts nach 32-bit `mull`, `divl`, `idivl` und einoperandigem
`imull`; zusätzlich wird das Gate in CI-Mirrors und Lane-Plan verdrahtet.
Das Design ist komplementär zum hier gepflegten AND-Fix:

- Upstream beweist Self-Zext- und implizite 32-bit-Schreibzugriffsfakten.
- Dieser Patch beweist die Upper-Zero-Facts von nichtnegativen
  Immediate-AND-Masken und erhält sie über die aktuelle ANDL-/ANDQ-Auswahl.

Die beiden Mechanismen ersetzen einander nicht. Das neue Upstream-Gate wurde
zusätzlich ausgeführt und ist grün.

Ein wichtiger Zwischenbefund aus dem vorherigen Rebase bleibt relevant:
Upstream konnte inzwischen `andl` statt `andq` wählen. Eine nur an ANDQ
ansetzende Predicate ließ dann trotz gültiger Byte-Facts ein `movzbl` übrig.
Die Korrektur teilt deshalb die semantische Masken-Predicate zwischen beiden
Opcodes und prüft den Erfolg erst am tatsächlich eliminierten Reextension-
Ledger. Eine Opcode-Alternative allein wäre kein ausreichender Test.

## Downcount: Beweis und Transformationsgrenzen

Der Godbolt-Reproducer `tests/benchmark/programs/switch_lut_bool_bench.c`
hatte bei `-O2` einen redundanten Eintritts-Guard und einen Zähler der Form
`0, +1, iv < 8_000_000`. Der bereits standardmäßig laufende post-phi
`loop_invert` erzeugt den bottom-tested Self-Loop; pauschales Default-on für
`loop_rotate` wäre daher die falsche Maßnahme gewesen.

Die Erweiterung akzeptiert nur einen bereits invertierten Ein-Block-Loop mit
unsigned U32/U64-IV, Initialisierung 0, konstantem Limit `N > 0`, Schritt `+1`,
genau zwei IV-Definitionen (Preheader-Copy und Latch-Copy), genau einer
Next-Definition und keiner weiteren Verwendung von IV oder Next. Guard und
Latch müssen denselben Exit und Body besitzen; die Reihenfolge
`update < copy < latch-cmp` wird geprüft. Alle relevanten Definitionen und
Uses werden vor der ersten Mutation gezählt. Die frische Latch-Condition wird
von `apply` mit einem neuen Value erzeugt; das Ein-Instruktions-Guard-Shape
schließt zusätzliche Compare-Definitionen im akzeptierten Pfad aus.

Für diese Form gilt `remaining = N - iv`. Die Transformation ersetzt den
Initialwert durch `N`, `Add` durch `Sub` und den Bottom-Test durch
`remaining != 0`; die Eintrittswache ist wegen `N > 0` wahr und wird mit ihrem
Compare entfernt. Die Konstantenanalyse rechnet bis einschließlich `u64::MAX`
mit i128 und korrekten Bitpatterns. Nulltripps, Live-outs, zusätzliche Uses,
abweichende Schritte/Vergleiche, nichtkonstante Grenzen und nicht bewiesene
CFG-Formen werden refused. `CCC_NO_LOOP_DOWNCOUNT=1` ist der unabhängige
Kill-Switch; `CCC_NO_LOOP_INVERT=1` deaktiviert weiterhin den gesamten
Inverter.

Tests decken unter anderem `N = 0`, `u64::MAX`, signed Recurrences, Schritt 2,
nichtkonstante Limits und Live-outs ab. Ein vermeintlicher weiterer
Live-Compare-Test wurde verworfen, weil `find_one` diese Form bereits vor der
Inversion ablehnt; er wäre kein Downcount-Test.

## AND-Byte-Facts: Semantik und Testgrenzen

Für `0 <= m <= 0xff` liefert `x & m` bereits ein unsigned-byte Ergebnis; ein
anschließendes `movzbl` ist redundant. Die gemeinsame Predicate verwendet
0xff und ist für jede Maske 0x80..=0xff im Unit-Test exhaustiv geprüft. ANDL
und ANDQ verwenden dieselbe semantische Regel, nicht zwei auseinanderlaufende
Opcode-Sonderfälle.

Die Härtung für negative imm32 bleibt vollständig erhalten: Masken ab
`0x8000_0000` erzeugen bei ANDQ wegen der Sign-Extension keine
Upper-Zero-Facts. ANDL darf dagegen seinen garantierten Upper-32-Zero-Write
verwenden. `$-2147483648` und `$-1` sind ausdrücklich abgedeckt. Der frühere
Parserfehler am Betrag von `i32::MIN` ist durch direkte imm32-Behandlung
behoben; die alte Form `neg.parse::<i32>()` darf nicht zurückkehren.

Upper-32-, Partial- und Implicit-Write-Invalidierungen sowie negative
GAS-Tests bleiben erhalten. `(unsigned char)(x & 255)` wird bereits von einem
früheren Fold kanonisiert; das C-Gate erzwingt deshalb für 0xff keine
künstliche AND-Form. Die eigentliche Predicate und das
Redundant-Extension-Ledger werden separat strukturell geprüft.

## Reproduzierbare Messungen

### Fokussierte Codegen-Orakel

Gleiche Translation Units, `-O2`, alle Funktionen; statische Instructions,
keine native Performancebehauptung:

| Reproducer | LCCC | GCC 16.2 | Clang 23.1.0 CE | ICX 2026.2 lokal | ICC 2021.10 CE |
|---|---:|---:|---:|---:|---:|
| `switch_lut_bool_bench.c` | 33 | 29 | 32 | 36 | 52 |
| `andq-width-bench.c` | 45 | 31 | 45 | 48 | 57 |

Remote-Godbolt-ICX-latest liegt bei 39/48. Über beide Quellen summiert:
GCC 60, Clang 77, LCCC 78, lokales ICX 84, Remote-ICX 87 und ICC 109
Instructions. Bericht und Assemblys: `final-godbolt.{json,md,log}` und
`final-godbolt/` im S60-Artefaktverzeichnis.

### Downcount: native A/B und Callgrind

Identische Quelle und identisches LCCC-Binary, Default gegen
`CCC_NO_LOOP_DOWNCOUNT=1`, jeweils identische Ausgabe; 101 interleaved,
auf CPU 0 gepinnte Runden:

| Arm | Minimum | Median | Mittelwert |
|---|---:|---:|---:|
| Downcount | 16,17 ms | 16,49 ms | 16,61 ms |
| Kontrolle | 16,30 ms | 16,62 ms | 16,72 ms |

Das Medianverhältnis Kontrolle/Downcount ist **1,0082**, das
Minimumverhältnis **1,0081**; der Sign-Test ergibt `p = 0,0165059`. Damit
beträgt der reproduzierte Reproducer-Vorteil **0,81 % weniger
Medianlaufzeit**, mit übereinstimmender Minimum- und Medianrichtung.

Valgrind 3.28.0, `--tool=callgrind --cache-sim=yes --branch-sim=yes`:

- Downcount: **144.133.114 Ir**.
- Kontrolle: **152.133.095 Ir**.
- Differenz: **7.999.981 Ir**; Verhältnis **0,9474145912**.

Ir ist ein simuliertes Instruction-Count-Modell, keine native Laufzeit und
kein PMU-/Raptor-Lake-Uops-Messwert. Kleine Abweichungen vom idealen
8-Millionen-Loopdelta stammen aus dem ebenfalls gezählten Programm-/Loader-
Teil; es werden ausschließlich die tatsächlich aufgezeichneten Gesamtzähler
berichtet.

Der statische A/B-Ranker lief über **85 Benchmark-Dateien und 414 Funktionen**:
null Fehler, sechs verbesserte Funktionen, keine Instruction-Regression,
aggregiert **−18 Instructions und −6 Branches**. Loads, Stores, Spills und
Vector-Zähler blieben unverändert. Rohdaten: `downcount/` einschließlich
`native-101-pinned.json`, `callgrind-summary.json` und `corpus/summary.json`.

### AND 0xff gegen die exakte 0x7f-Kontrolle

Ein separates Worktree mit exakt der Predicate-Änderung auf 0x7f wurde mit
Fastbuild/O1/`-j2` gebaut; der Produktivtree wurde nicht umgeschaltet. Die
Assembly-Differenz besteht auf aktuellem Main nur aus `andl` + `movzbl` gegen
`andl` + `movl` in den beiden Leaf-Funktionen.

Die 101 CPU-gepinnten Runden ergaben 21,71 ms Median für 0xff gegen 22,02 ms
für 0x7f, Medianverhältnis **1,0141** und `p = 0,00507738`. Das
Minimumverhältnis war jedoch **0,9996**: Median und Minimum zeigen nicht
dieselbe Richtung. Das Harness sagt deshalb ausdrücklich „do not conclude“.
Callgrind bestätigt die fehlende dynamische Arbeitseinsparung:
**144.133.122 Ir in beiden Arms**, sämtliche aufgezeichneten Zähler identisch.
Der 85-Dateien-/414-Funktionen-Ranker fand null geänderte Funktionen.

Damit bleibt 0xff eine präzisere Faktenregel, aber kein belegter Workload- oder
Laufzeitgewinn. Rohdaten: `andq-width/`; der Verzeichnisname ist historisch,
der aktuelle Assembly-Befund betrifft ANDL.

### Direkte native Oracle-Vergleiche

Alle Binaries erzeugten je Reproducer identische Ausgaben. Jeder Vergleich
verwendet 101 CPU-gepinnte, interleaved Runden; dies sind Mikrobenchmarks,
keine allgemeine Compiler-Rangliste.

| Reproducer / Vergleich | LCCC-Mediandelta | Min-/Median-Konsistenz |
|---|---:|---|
| Switch gegen GCC 16.2 | **2,86 % schneller** | beide zugunsten LCCC |
| Switch gegen Clang 23.1.4 | **56,83 % schneller** | beide zugunsten LCCC |
| Switch gegen ICX 2026.2 | **48,80 % schneller** | beide zugunsten LCCC |
| AND gegen GCC 16.2 | **12,70 % langsamer** | beide zugunsten GCC |
| AND gegen Clang 23.1.4 | **1,03 % langsamer** | beide zugunsten Clang |
| AND gegen ICX 2026.2 | **4,11 % schneller** | beide zugunsten LCCC |

Die Switch-Callgrind-Daten erklären die Clang-/ICX-Differenz: Ihre
Tabellenformen haben etwa 1,96 Millionen konditionale Mispredicts gegenüber
etwa 2.900 bei LCCC/GCC, obwohl ihre Ir-Zähler niedriger sind. Beim
AND-Reproducer hat GCC 136.133.977 Ir gegenüber 144.133.183 bei LCCC; das
stützt den dortigen GCC-Vorsprung. Ir allein ist in beiden Richtungen kein
Laufzeitbeweis. Vollständige Zähler und Rohzeiten: `oracle-native/`.

### Workload-abgeleitete Kernel

Der vorhandene `benchmark_native_batched.py`-Harness maß sieben gepinnte,
selbstprüfende Workload-Kernel mit LCCC, GCC 16.2 und Clang 23.1.4. Pro
Vergleich sind sieben randomisierte, gepaarte Runden auf CPU 0 enthalten;
jede Wiederholung prüft die exakte Ausgabe. Verwendet wurden
`-O2 -march=x86-64-v3 -fwrapv -ffp-contract=off`. Das Verhältnis ist
LCCC/GCC, Werte über 1 bedeuten also langsameres LCCC. Die Intervalle sind
exploratorische Bootstrap-Intervalle, keine allgemeine Projektrangliste.

| Kernel | LCCC/GCC Median | Exploratorisches 95-%-Intervall | Einordnung |
|---|---:|---:|---|
| zlib-ng Adler-32 combine | 0,9298 | [0,9270; 0,9444] | LCCC etwa 7,0 % schneller |
| zlib-ng compare256 | 1,1514 | [1,0876; 1,2379] | LCCC etwa 15,1 % langsamer |
| zlib-ng slide hash | 1,1297 | [1,1207; 1,1746] | LCCC etwa 13,0 % langsamer |
| Expat UTF-8 content | 1,0741 | [1,0552; 1,0933] | LCCC etwa 7,4 % langsamer |
| SQLite VDBE serial | 1,1700 | [1,1614; 1,1737] | LCCC etwa 17,0 % langsamer |
| Linux xxHash64 | 1,0110 | [1,0058; 1,1130] | nahe Parität im Median, breite Streuung |
| glibc dl-hash | 1,1514 | [1,1428; 1,1895] | LCCC etwa 15,1 % langsamer |

ICX ist in dieser Matrix nicht als erfolgreicher Arm enthalten: Bei
`zlib_ng_slide_hash` erzeugte ICX `_intel_fast_memset` und
`_intel_fast_memcpy`, die der neutrale GCC-Linkdriver des Harness nicht
auflöst. Der abgebrochene Versuch und `link.json` sind als
`workload-native-icx-*` erhalten. Das ist eine Link-/Runtime-Anforderung des
Harness, kein als LCCC-Defekt umgedeuteter Befund. ICX bleibt durch die
direkten nativen Vergleiche und Codegen-Orakel abgedeckt.

Der registry-aware statische Survey umfasste alle 63 registrierten Fälle.
61 besitzen vollständige GCC-/Clang-CE-/LCCC-/lokale-ICX-Daten. Bei
`llvm_word_counts` und `llvm_scalar_facts` kann Compiler Explorer die
repository-relativen Includes nicht auflösen; der Runner endete deshalb
korrekt fail-closed mit zwei Fehlern. Diese beiden Fälle wurden zusätzlich
mit denselben Statistikfunktionen und nativen GCC 16.2, Clang 23.1.4 sowie
ICX 2026.2 vermessen (`workload-codegen-native-include/`). Ein exakt
expandierter Include-Adapter erhielt sämtliche lokale LCCC-Metriken; ein
späterer CE-Versuch wurde mit HTTP 429 abgelehnt und nicht als erfolgreiche
Remote-Messung ausgegeben.

Über die 61 direkt vollständigen Fälle summieren sich 11.080 LCCC-, 10.989
GCC-, 12.550 Clang- und 14.544 ICX-Instructions. Diese Summe darf die
Ausreißer nicht verstecken: `ring_fifo` liegt bei 47 gegenüber 13 GCC-
Instructions, `gzip_crc32_slice8` bei 517 gegenüber 205 und
`percpu_ptr_walk` bei 141 gegenüber 57. Diese statischen Befunde und die
nativen SQLite-/glibc-/zlib-ng-Abstände sind konkrete Follow-up-Ziele, keine
nebensächliche Fußnote.

## Correctness, Integration und Red-Team

Auf dem rebasierten Code-Stand wurden ausgeführt:

- Fastbuild mit O1, `CARGO_INCREMENTAL=0`, exakt `-j2` und aktivem Swap;
  sechs Fastbuild-Binaries erfolgreich gebaut.
- `cargo test --profile fastbuild --all-targets --locked -j2`:
  **6.902 PASS, 0 FAIL, 10 ignored** über Library und Binary-Targets.
- Vollständige Regression mit GCC 16.2: **1.269 PASS, 0 FAIL**, zwei bekannte
  GCC-Compare-Skips, null Run-Skips, **1.271 Fälle**.
- Differential-Fuzzing mit GCC 16.2 bei `-O2`, Seed 0 und Count 128:
  `differential`, `phi_cfg`, `m32`, `regparm`, `slot_rmw` und `alias_m32`
  zusammen **768/768**, null FAIL/SKIP.
- Downcount-, AND-, Load-/Cast-, affine-Loop- und neue
  `redundant_self_zext`-Gates: PASS. Das Downcount-Gate umfasst 25
  GCC-Paritätszellen über fünf Optimierungsstufen und
  `N = 0, 1, 2, 17, 250000` sowie Live-out und Kill-Switch.
- Strikte Linker-Suite mit GAS/BFD 2.47, LLD 23.1.5 und mold 3.0.0:
  **325 PASS, 0 FAIL/WARN/SKIP**. Dieselbe vollständige Suite mit exakt
  mold 2.42.1: ebenfalls **325/0/0/0**. Der native Multiarch-BFD-Wrapper und
  der echte Kernel-`relocs`-Pfad wurden explizit gesetzt.
- CI-/Standalone-Parität: **444 Kommandos, 27 Invocation Contracts**.
  Audit-Census, Lane-Plan und Workflow-Shell-Checker: PASS.
- Oracle-Verifizierung, ICX-2026.2-Macroprobe, Cross-GCC/QEMU und native
  i686-/x86-64-Werkzeuge: bereit beziehungsweise grün. Das
  `provenance=unknown` des generischen Verifiers wird nicht als Hashbeweis
  umgedeutet; separate Installations- und Hashnachweise bleiben maßgeblich.

GDB-Disassembly bestätigt die erzeugten `andl`/`movl`-Folgen und den
Downcount-Loop. Objtool lief mit `SRCARCH=x86 --stackval --dry-run` und
meldet weiterhin je Objekt `call without frame pointer save/setup` in `main`;
RC 0 wird nicht als warning-freier PASS umgedeutet. `pahole` meldet für beide
LCCC-Objekte fehlende BTF beziehungsweise „no supported type information“
und liefert damit keinen positiven Layoutnachweis. Diese Ausgaben liegen in
`audit/`. Memcheck ergab für Downcount und AND jeweils null Fehler und keine
Leaks; Callgrind-Daten liegen separat vor.

Der abschließende `ci_local.sh --fast`-Lauf einschließlich rustfmt und Clippy
wird auf dem finalen Dokumentations-Tree ausgeführt und im separaten Receipt
protokolliert. Ein lokaler Full-/Slow-CI-Lauf ist ausdrücklich nicht Teil
dieser Arbeit; die langsamen Gates bleiben der GitHub-CI vorbehalten.

## Offene Arbeit und Prioritäten

1. **Reale Missed Optimizations zuerst reduzieren:** Die native
   SQLite-/glibc-/zlib-ng-Abstände und die statischen Ausreißer `ring_fifo`,
   `gzip_crc32_slice8` und `percpu_ptr_walk` mit minimierten Reproducern,
   Assembly und Callgrind untersuchen. Erst danach eine Optimierung mit
   kontrolliertem nativem A/B zulassen.
2. **RA/IPA statt Peephole-Kosmetik:** GCC hält bei lokalen `noinline`
   Funktionen mehr Werte in Caller-saved-Registern, während LCCC Calls häufig
   konservativ als vollständige Clobber behandelt. Keine
   Caller-saved-Annahme ohne vollständigen Funktions-Clobber-/ABI-Beweis.
3. **Downcount nicht vorschnell verallgemeinern:** Live-out-, Mehrblock-,
   Signed-/Wrap- oder variabel begrenzte Recurrences benötigen jeweils einen
   eigenen Beweis und eigene Negative-Tests.
4. **AND-0xff nicht als Performancearbeit verkaufen:** Native Replikation,
   Callgrind und Corpus liefern weiterhin keinen belastbaren Gewinn.
5. **Objtool-/Frame-Pointer-Befund separat untersuchen:** Er ist kein
   Nachweis gegen diese Loop-/AND-Patches, aber auch kein warning-freier
   Abschluss. Fehlende BTF-/pahole-Typinformation bleibt eine eigene
   Tooling-/Debuginfo-Aufgabe.
6. **Harness-Schulden gezielt beheben:** Der batched native Harness braucht
   ein definiertes Modell für compiler-eigene Linkruntimes wie ICX; der
   registry-aware CE-Survey braucht eine geprüfte Include-Expansion oder
   einen vollständig nativen Oracle-Pfad für repository-relative Wrapper.
   Keine stillschweigenden Skips und keine pauschale Intel-Library-Verlinkung
   für alle Arms.
7. **Daueraufgaben fortsetzen:** GCC-17-git-Torture für i686/x86-64 samt
   `lccc-ld`, verbleibende Miscompiles und Missed Optimizations sowie echte
   End-to-End-Builds von zlib-ng, gzip, Expat, SQLite, Linux und glibc. Die
   63 registrierten beziehungsweise 85 Dateien des statischen Corpus und die
   Mikrobenchmarks ersetzen diese vollständigen Workloads nicht.
