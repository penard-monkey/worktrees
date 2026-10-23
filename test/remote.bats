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

# A kept host, written by hand: the record only (no key — nothing below gets
# as far as needing one).
keep_host() {
  mkdir -p "$CFG"
  cat >"$CFG/remote-hosts.json" <<JSON
{"v": 1, "chosen": null, "hosts": [{"id": "h_aaaaaaaaaaaaaaaaaaaaaaaaaa", "name": "Mac mini", "addr": "192.168.1.5:7461", "pin": "$PIN", "discovery_id": "aaaaaaaaaaaaaaaaaaaaaaaaaa", "device_id": "d_abc", "principal_id": "owner", "expiry": {"idle_days": 30}, "paired_at": 1800000000}]}
JSON
}

OTHER_PIN="cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"

@test "remote readdress refuses a malformed code, a public address and a zone before looking for the host" {
  run_wt -C "$BATS_TEST_TMPDIR" remote readdress nobody "worktrees-pair:v1?addr=192.168.1.9:7461" --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF "not an address code" <<<"$output"
  run_wt -C "$BATS_TEST_TMPDIR" remote readdress nobody "worktrees-addr:v1?addr=8.8.8.8:7461&pin=$PIN" --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF "not-private: 8.8.8.8 is public" <<<"$output"
  run_wt -C "$BATS_TEST_TMPDIR" remote readdress nobody "worktrees-addr:v1?addr=[fe80::1%4]:7461&pin=$PIN" --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF "zone" <<<"$output"
  [ ! -e "$CFG/remote-hosts.json" ]
}

@test "remote readdress names an unknown host" {
  keep_host
  run_wt -C "$BATS_TEST_TMPDIR" remote readdress nobody "worktrees-addr:v1?addr=192.168.1.9:7461&pin=$PIN" --config-dir "$CFG"
  [ "$status" -eq 2 ]
  grep -qF 'no paired host is called "nobody"' <<<"$output"
}

@test "remote readdress refuses a code for another host's pin, dials nothing, and keeps the record byte for byte" {
  keep_host
  cp "$CFG/remote-hosts.json" "$BATS_TEST_TMPDIR/before.json"
  run_wt -C "$BATS_TEST_TMPDIR" remote readdress "Mac mini" "worktrees-addr:v1?addr=192.168.1.9:7461&pin=$OTHER_PIN" --config-dir "$CFG"
  [ "$status" -eq 1 ]
  grep -qF "for another host" <<<"$output"
  grep -qF "Nothing was dialled" <<<"$output"
  cmp "$CFG/remote-hosts.json" "$BATS_TEST_TMPDIR/before.json"
}
