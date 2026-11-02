Derived fixtures, **not live captures**. On 2026-11-02, `worktrees new lane
--ai claude --model haiku --no-attach` in a scratch repository refused the
launch: 19 agents were live and `[limits] max_live_agents` was 18. No
`--force` was used, and no owner pane or transcript was read.

`first`, `toggled`, `on-next`, and `text-typed` start from the corresponding
2.1.314 `question-multiselect*.txt` captures. The tab bar gains `☐ Size`
after Toppings, and the row below the free-text box changes from `Submit` to
`Next`. `on-next` leaves only Cheese ticked, matching the scripted answer.
The box glyphs, field row, highlight positions and review layout come from
those captured screens; **Next in a form is inferred**, not measured here.
The real desktop card screenshot reported in the brief also showed `Next`
attached to a typed field row in a multi-question form.

`second` and `review` start from 2.1.314 `question-two-second.txt` and
`question-two-review.txt`: their first tab is renamed from Colour to Toppings,
and the review's first answer is Cheese and Mushrooms. They show the expected
advance and final review layout. They do not establish the exact 2.1.316
styling or spacing for this combined case. The `.e.txt` ANSI captures were
not synthesized.
