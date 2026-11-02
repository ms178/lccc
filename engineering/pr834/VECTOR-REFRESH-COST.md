# Narrow-copy proof refresh cost

The baseline 10,000-instruction single-function fixture exercises 2,500 narrow
forwardings and 2,500 whole-function refreshes. Five preliminary native runs
and a separate Callgrind run are retained. The small structural cost regression
fails before the change (8 independent rewrites caused 8 refreshes); it is not
a walltime gate.

The change is not a stale dirty-once-per-pass cache. Only proven narrow scalar
self-merge read renames receive the optimization: destinations, write masks,
CFG and instruction shape remain unchanged. Removed reads leave conservative
cached liveness. Added source reads force refresh before querying that private
register. A negative cached proof after removals refreshes and retries, so lost
precision cannot silently suppress a rewrite. Other rewrites remain eager.
Forward traversal never revisits an earlier function. ABI fallback inference
only affects argument/return registers; this proof queries XMM8–15 exclusively.

Two direct backedge tests require refresh for added reads and retry after
removed reads. Sixty-four mixed CFGs compare the exact output with an eager
reference instantiation of the same rewrite implementation. Existing positive
controls and the 48-copy budget are unchanged. All 4,763 library tests pass;
9 are ignored by default (8 pre-existing plus the explicitly executed manual
10k benchmark). The manual benchmark asserts all 2,500 forwardings occur.

The subsequent complete captures are recorded in
[MEASUREMENT-RESULTS.md](MEASUREMENT-RESULTS.md): two all45 native compiler
protocols, a six-form interleaved diagnostic, all45 compiler-parent Callgrind,
five bounded program pairs per form and the completed full stress profiles.
They do not relabel earlier runs or establish native program-speed gains.
Final local/hosted gate status belongs to the delivery receipt, not this
historical implementation note.
