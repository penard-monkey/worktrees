**20261114-160404-v1-json-flag-plan** — v1, json-flag-plan: 20/24 intents proved, 531 s

| Measure | Value | Detail |
|---|---|---|
| Comment to agent action | median 20 s (4/5 reached a turn) | #2 42 s → turn 42 s → done 49 s; #3 25 s → turn 25 s → done 32 s; #4 14 s → turn 14 s → done 21 s; #6 14 s → turn 14 s → done 29 s; #10 13 s → turn — → done — |
| Notes used | 1 of 2 sends had a note | a watched file changed after 1 with a note, 0 without |
| Blind spots | 1 | bret read_diff: V1 shows that a command ran, not the file it wrote (§1) |
| Lost attention | 0 | — |
| Acts per outcome | 46 clicks, keys and typed strings | open_share 1, allow 1, open_thread 1, send_task 0, wait 0, post 15, use_these 15, ask_share 5, answer_request 1, end_session 0, share_again 2, reboot_owner 2, stop_sharing 3 |
| Wrong place | 1 | 24 use_these reached an unshared session |
| Errors | 4 warn+ log lines, 0 audit refusals, 1 unproved acts | 24 dana use_these: shared posts 10..10 at seq 11; turns 1 -> None; the send ENDED the binding: it went to an unshared session; guest: claude_usage ×4 |
| Recovery | owner 31 s, guests bret 63 s | share kept: True, binding after: active |
