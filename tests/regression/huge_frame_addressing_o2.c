/* Giant-frame addressing, compiled at -O2.  Shared cases live in
 * huge_frame/huge_frame_cases.inc (out of the corpus glob); this wrapper only
 * pins the optimisation level, because the anchor selection, the MachInst
 * fallback and the frame layout all differ per level.
 */
#include "huge_frame/huge_frame_cases.inc"
