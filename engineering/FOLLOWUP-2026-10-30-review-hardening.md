# S82 — lokaler CI-Blocker und Review-Hardening (2026-10-30)

Dieser Kandidat bleibt Main `0ff1d3f12a75ce83f491088f0ec27526f03d6db9` plus
die zuvor validierte S81-Arbeit. Die Review-Punkte wurden am Code geprüft,
nicht als vorgefertigte Tatsachen übernommen. Die rote GitHub-Meldung wurde
ausschließlich lokal reproduziert und korrigiert; kein GitHub-Lauf diente als
Ersatz für lokale Tests.

## F1: Census und transaktionale Strided-Emission

Der exakte Blocker war `python3 scripts/audit_census.py --check`:
`panic_sites 924 -> 933` und `fns_over_500 97 -> 98`. Die Baseline wurde
nicht erhöht. Der Reducer verwendet nun `Option` vom Intrinsic-Mapping bis zur
Transformation. Ungültige Loads, Operationen, Shift-Formen, Konstanten und
interne Pattern-Indices verweigern die gesamte Umwandlung. Planung,
Preheader-, Body- und Reduce-Puffer sind vor der ersten IR-Mutation fertig;
auch die beiden Scalar-Header-Phi-Ziele werden vor dem Commit validiert.
Die bisherige Compact-Load-Panik und der Konstanten-Fallback auf null wurden
im selben Pfad entfernt. Die Phasen sind getrennt (`transform` unter 100
Zeilen), und direkte interne Refusal-Tests prüfen unveränderte Blöcke und
Fresh-ID-Zähler.

## F2: Klassifikationen ohne semantische Superliste

Nicht jede ähnliche Intrinsic-Liste beschreibt denselben Vertrag.
`may_read_memory`, `produces_vector_value`, `vector_result_width`, die
Alias-Footprints und Vec-SSA-/Raw-Reader-Mengen bleiben deshalb getrennt.
Eine Zusammenfassung zu einer Speicher- oder SIMD-Superliste wäre falsch.

Zwei echte Duplikate wurden dagegen zusammengeführt:

* Die RA-Hilfe verwendet zentrale `vector_result_width`-Metadaten und nur
  noch die expliziten scalar-result `HorizontalAdd*`-Ausnahmen. Ihre alte
  Handliste hatte bereits den AVX2-Strided-Load vermisst. Ein Test enthält
  die vollständige frühere Menge, alle drei Strided-Loads sowie Store- und
  Scalar-Negativkontrollen. Dadurch bleibt jeder alte positive Fall erhalten,
  während neue zentrale SIMD-Resultate nicht erneut vergessen werden können.
  Die Aufnahme weiterer zentral klassifizierter SIMD-Produzenten ist
  ausdrücklich intendiert und als solche getestet: Sie verstärkt nur die
  Schutzbedingung um den explizit opt-in Scalar-FP-Stack-Experimentierschalter;
  die Default-RA-Konfiguration aktiviert diesen Sonderweg nicht.
* Core- und allocating MachInst-Last-Write-Klassifikator konsumieren denselben
  Iterator für implizite Writes. Reihenfolge und Typed-Return-Tie-Break sind
  unverändert. Bestehende Call-/IPA-Tests und ein Division/Raw-Paritätstest
  prüfen beide Konsumenten.

## F3: Downcount, Scans und Source-Spans

Der Downcount-Guard wird als Ein-Instruction-Slice gematcht und nach allen
Beweisen mit seinen Spans geleert; `guard_cmp = 0` ist entfallen. Die
Definitionen von `iv` und `next` werden zunächst nur in ihren kleinen
Preheader-/Latch-Blöcken bestimmt und müssen dort jeweils eindeutig sein. Der
ohnehin notwendige globale Validierungsscan weist jede weitere Definition
außerhalb dieser Stellen sowie unzulässige Uses zurück. Dadurch entfallen
beide separaten vollständigen IR-Definitionscans, ohne ein großes
DefUse-Album zu erzeugen. CFG-Daten werden nicht über eine Mutation hinweg
wiederverwendet. Refusal-Tests umfassen Live-outs, ungültige Rekurrenzen und
zusätzliche `iv`-/`next`-Definitionen im Preheader, Latch und Exit.

Beim Strided-Preheader werden für neu angehängte Instruktionen Dummy-Spans
ergänzt, wenn das Array vorher exakt parallel war. Nicht-Debug-Blöcke bleiben
spanlos. Ein positiver Transformationstest prüft die Parallelität nach dem
Commit; der Downcount-Test prüft das Entfernen des letzten Guard-Spans.

## F4: orthogonale bereits enthaltene Änderungen

Diese Punkte bleiben einzeln abgegrenzt und werden nicht als
Vektorisierungsarbeit ausgegeben:

* **ILP32-Predefines / Compatibility:** `LONG_MAX`, `SIZE_MAX`,
  `INTPTR_MAX` und ihre übrigen unpräfixierten Geschwister gehören zu
  `<limits.h>` beziehungsweise `<stdint.h>`, nicht zu den Compiler-Predefines.
  GCC exportiert sie weder unter LP64 noch ILP32. Das Entfernen korrigiert
  diese Bare-Use-Inkompatibilität; die `__…__`-Konstanten bleiben erhalten,
  und die libc-Header liefern weiterhin die üblichen Makros. Es gibt keine
  ABI-Änderung. `check_macro_parity.sh` verbietet die unpräfixierten Namen
  ausdrücklich in beiden Datenmodellen.
* **Cross-Linker-Diagnose:** AArch64/RISC-V emittieren die betreffenden
  DT_FLAGS/DT_FLAGS_1-Bits nicht. `-z origin`, `nodelete` und verwandte
  Optionen werden deshalb sichtbar als ignoriert gemeldet statt still
  akzeptiert; `--fatal-warnings` behält seine normale Bedeutung. Implementierte
  Optionen wie die geprüften Gegenkontrollen bleiben warnfrei.
* **Intrinsic-Diagnosen:** Die ARM-/i686-Formatstrings geben jetzt den
  tatsächlichen Opcode statt eines literalen `{op:?}` aus. Tests prüfen Name
  und Fehlen des Formatlochs.
* **RCX-Cache und RA:** Der generische RCX-Loader invalidiert einen alten
  Secondary-Cache-Anspruch, weil er den neuen Wert nicht parkt. Die
  S81-Diamant- und IPA-Regeln bleiben separat dokumentiert; S82 ändert nur
  die oben beschriebenen gemeinsamen Klassifikatoren.

## F5: Tests, Alias-Spannen und ISA-/Optimierungsmatrix

Die Diamant-Testform verwendet benannte Struct-Felder statt positionaler
Booleans. Die X4-Alias-Spannen wurden gegen den Emitter geprüft und bleiben
unverändert: exakt zwei DWORDs bei `+0`/`+8` haben das konservative Intervall
12 Bytes; der Compact-Load liest zwei volle 16-Byte-Hälften und benötigt 32
Bytes. Eine kleinere Compact-Spanne wäre falsch.

Die fokussierten Strided- und Saturating-Subtract-Gates umfassen jetzt O1 und
Os auf x86-64 und x86-64-v3. Da der dedizierte Reducer absichtlich O2+ und
nicht size-optimiert ist, verlangen diese Level korrekte Fallback-Ausgabe,
keine erzwungene Vektorform. GNU `as --64 -march=generic64` prüft die
SSE2-Assemblys als unabhängige ISA-Instanz; der O2-Gate behält zusätzlich die
Guard-Page-/Watchpoint- und GCC-Differentialtests.

## Validierung und Ergebnisgrenzen

Der exakte Endstand wurde mit O1-`fastbuild`, exakt `-j2` und `-D warnings`
für Host und Cross-Treiber gebaut. Clippy über Bibliothek und Tests ist nach
einer `unwrap_or_default`-Bereinigung warnfrei. Die aktuelle Testbinary
bestätigt 111/111 gezielt ausgewählte Tests; ein Lauf mit null selektierten
Tests gilt nicht als Gate. Strided/Saturating (O1/O2/Os, SSE2/AVX2, GAS und
Watchpoints), Downcount, Macro-Parität und das affine Kontrollmodell sind
ebenso grün wie Census und Scanner-Selftest. Der Census bleibt bei 917/924
Panic-Sites, 97/97 Funktionen über 500 Zeilen und ohne gewachsene Metrik.

Neun Cross- und 18 Host-/m32-Assemblyvergleiche gegen S81 sind byteidentisch.
AArch64, RISC-V und i686 bestehen zusätzlich alle neun statischen
O1/O2/O3-Laufzeitdifferentiale gegen GCC. Der Downcount-Stress weist bei
beiden Compilern 48/48 Admissions und byteidentische Assembly auf. Neun
verzahnte Compile-Time-Paare ergaben im Endlauf S82/S81 = 0,985 (Wall) und
0,982 (CPU), nach einem Vorlauf von 1,007; die Richtungsumkehr bei einem etwa
0,15-Sekunden-Probe zeigt die Messrausch-Grenze. Daraus wird kein
Compile-Time- oder sonstiger Performancegewinn behauptet. Die Beleglast für
die Scan-Reduktion liegt auf der direkt prüfbaren Entfernung der zwei
Ganzfunktionsdurchläufe und den semantischen Refusal-Tests.
