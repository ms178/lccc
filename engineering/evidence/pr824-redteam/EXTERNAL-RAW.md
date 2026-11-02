# Bulk historical evidence transport

The three raw archives listed in `EXTERNAL-RAW.json` were moved **byte-for-byte**
out of the cumulative patch into its companion workspace artifact directory,
`artifacts/pr824-raw/`. Their contents were not discarded or regenerated. This
removes 9,903,698 bytes of compressed measurement payload from the source tree;
small reports, tests, harnesses and provenance remain in the patch. No CI gate
or executable source referenced these archive paths.

Verify the companion files from the delivered workspace:

```sh
python3 lccc/scripts/verify_evidence_hashes.py \
  --root "$PWD" --manifest artifacts/pr824-raw/MANIFEST.json
```

They also remain exact Git blobs in preserved S34 commit
`b9629da366b8a65b3831c50e8c4b666a3d666be9`. After restoring the preserved S34
session bundle and its prerequisite base (see the delivery recovery notes),
`git show COMMIT:FORMER_PATH > ARCHIVE` reconstructs each file; use the former
paths and SHA256 values in the manifest. A clone with only the cumulative patch
does **not** automatically contain that historical commit or the companion
archives. The metadata records integrity and location, not a claim that old
compiler binaries or external service responses are reproducible today.
