**20261114-184255-v1-release-json-flag-plan** — v1-release, json-flag-plan: 38/54 intents proved, 1916 s

| Measure | Value | Detail |
|---|---|---|
| Comment to agent action | median 140 s (7/7 reached a turn) | #3 150 s → turn 150 s → done 156 s; #4 138 s → turn 138 s → done 144 s; #5 118 s → turn 118 s → done 124 s; #7 141 s → turn 140 s → done 154 s; #8 121 s → turn 120 s → done 134 s; #13 426 s → turn 426 s → done 446 s; #14 406 s → turn 406 s → done 426 s |
| Comment to agent action (cy's share) | median 13 s (1/1 reached a turn) | #2 13 s → turn 13 s → done 25 s |
| Notes used | 3 of 4 sends had a note | a watched file changed after 2 with a note, 0 without |
| Blind spots | 1 | bret read_diff: V1 shows that a command ran, not the file it wrote (§1) |
| Lost attention | 0 | — |
| Acts per outcome | 105 clicks, keys and typed strings | open_share 5, allow 2, open_thread 2, send_task 0, wait 0, post 27, use_these 49, open_lane 2, ask_share 5, answer_request 1, end_session 0, share_again 4, reboot_owner 2, stop_sharing 6 |
| Wrong place | 3 | 14 use_these, 22 post, 35 send_task reached an unshared session |
| Errors | 22 warn+ log lines, 5 audit refusals, 8 unproved acts | 9 dana use_these: no shared_with_agent event; 14 dana use_these: no shared_with_agent event; 22 dana post: no post by this guest reached the thread; 23 cy use_these: no shared_with_agent event; 35 dana use_these: no shared_with_agent event; 35 dana send_task: turns 1 -> None; 36 dana use_these: no shared_with_agent event; 37 dana use_these: no shared_with_agent event; guest: claude_usage ×11; third: claude_usage ×11 |
| Recovery | owner 29 s, guests bret 61 s, cy 61 s | share kept: True, binding after: active |
| Replays | 12 intents replayed, 1 done by hand | 37 use_these |
