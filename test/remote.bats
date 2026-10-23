#!/usr/bin/env bats
# `worktrees remote …`: this machine as another host's paired device. The
# protocol (pairing, the pinned TLS pipe, handshake v1) is tested in Rust,
# against serve's own pairing listener; here, only what the CLI decides
# before any network: its arguments, an empty store, and a link refused
# BEFORE anything is dialled.

load 'helpers/common'

setup() { common_setup; CFG="$BATS_TEST_TMPDIR/cfg"; }

PIN="abababababababababababababababababababababababababababababababab"
SECRET="aaaaaaaaaaaaaaaaaaaaaaaaaa"

@test "remote with no verb prints its usage and exits 2, outside any repo" {
  run_wt -C "$BATS_TEST_TMPDIR" remote
  [ "$status" -eq 2 ]
  grep -qF "worktrees remote pair <link>" <<<"$output"
}

@test "remote hosts on a fresh config dir lists none, and writes nothing" {
  run_wt -C "$BATS_TEST_TMPDIR" remote hosts --config-dir "$CFG"
  [ "$status" -eq 0 ]
  grep -qF "No paired hosts" <<<"$output"
  [ ! -e "$CFG/remote-hosts.json" ]
}

@test "remote pair refuses a public address in the link before dialling, in the parser's words" {
  run_wt -C "$BATS_TEST_TMPDIR" remote pair "worktrees-pair:v1?addr=8.8.8.8:7461&pin=$PIN&secret=$SECRET&host=Mac&exp=9999999999" --config-dir "$CFG"
  [ "$status" -eq 1 ]
  grep -qF "not-private: 8.8.8.8 is public" <<<"$output"
  [ ! -e "$CFG/remote-keys" ]
}

@test "remote pair refuses an expired link and a zone" {
  run_wt -C "$BATS_TEST_TMPDIR" remote pair "worktrees-pair:v1?addr=192.168.1.9:7461&pin=$PIN&secret=$SECRET&host=Mac&exp=1000" --config-dir "$CFG"
  [ "$status" -eq 1 ]
  grep -qF "expired" <<<"$output"
  run_wt -C "$BATS_TEST_TMPDIR" remote pair "worktrees-pair:v1?addr=[fe80::1%254]:7461&pin=$PIN&secret=$SECRET&host=Mac&exp=9999999999" --config-dir "$CFG"
  [ "$status" -eq 1 ]
  grep -qF "zone" <<<"$output"
}

@test "remote call names an unknown host and refuses arguments that are not one JSON object" {
  run_wt -C "$BATS_TEST_TMPDIR" remote call nobody list_workspace --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF 'no paired host is called "nobody"' <<<"$output"
  run_wt -C "$BATS_TEST_TMPDIR" remote call nobody list_workspace '[1]' --config-dir "$CFG"
  [ "$status" -eq 2 ]
}

@test "remote use names an unknown host, and 'this' needs no host at all" {
  run_wt -C "$BATS_TEST_TMPDIR" remote use nobody --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF 'no paired host is called "nobody"' <<<"$output"
  run_wt -C "$BATS_TEST_TMPDIR" remote use this --config-dir "$CFG"
  [ "$status" -eq 0 ]
  grep -qF "this machine's own engine" <<<"$output"
}

@test "remote refuses an unknown verb and an unknown flag" {
  run_wt -C "$BATS_TEST_TMPDIR" remote frobnicate
  [ "$status" -eq 2 ]
  run_wt -C "$BATS_TEST_TMPDIR" remote hosts --bogus
  [ "$status" -eq 2 ]
  grep -qF "unknown flag --bogus" <<<"$output"
}
