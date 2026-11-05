# Same-pane Claude probes

These are minimal probe files for the interactive Claude and its children. The
field shapes come from a private Claude Code 2.1.318 run; PIDs, paths, names
and session IDs are test values.

The observed `claude -p` child had `kind: interactive`, `entrypoint: sdk-cli`
and the parent's `tmux` address, but no `status` before exit. Its fixture gives
it `status: busy` so the reader under test cannot discard it just because the
status is missing. The `sdk-py` entrypoint comes from the two security-guidance
review transcripts; those short-lived probe files were gone by measurement.
Its busy status is the reported failure case. The background and extra CLI
files check that parked jobs and hand-started sessions remain visible.
