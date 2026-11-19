#!/usr/bin/env python3
"""Score one vmlab run: §12.4's rubric, read from the record and the logs.

    score.py <run dir> [--json]

Prints a Markdown table (or the numbers as JSON). Every number comes from
what the run wrote: record.json (the driver's intents, proofs and input
counts), host/thread.json and host/turns.json (the conversation and the
agent's turns, as the owner's console read them), host/*-serve.log and
host/*-audit.jsonl (both VMs, the run's window).
"""
import argparse
import json
import re
import statistics
import sys
from pathlib import Path

FILE_TOOLS = re.compile(r"apply_patch|edit|write|create", re.I)


def load(d):
    rec = json.loads((d / "record.json").read_text())
    host = d / "host"

    def j(name, default):
        p = host / name
        return json.loads(p.read_text()) if p.exists() and p.read_text().strip() else default
    return rec, j("thread.json", []), j("turns.json", {}), host


def all_turns(turns):
    out = []
    for page in turns.values():
        for t in page.get("turns", []):
            steps = [s.get("at") for s in t.get("steps", []) if s.get("at")]
            out.append({"at": t.get("at"), "done": max(steps) if steps else t.get("at"),
                        "user": t.get("user") or "", "tools": t.get("tools", [])})
    return sorted((t for t in out if t["at"]), key=lambda t: t["at"])


def comment_to_action(thread, turns):
    """Per guest post: seconds to the `shared_with_agent` event that carried
    it, then to the agent's next turn starting and finishing."""
    shares = [e for e in thread if e["kind"] == "shared_with_agent"]
    rows = []
    for e in thread:
        if e["kind"] != "post" or not e.get("device"):
            continue
        sw = next((s for s in shares if s["first"] <= e["seq"] <= s["last"]), None)
        r = {"seq": e["seq"], "who": e.get("name"), "to_shared_s": None, "to_turn_s": None, "to_done_s": None}
        if sw:
            r["to_shared_s"] = sw["at"] - e["at"]
            t = next((t for t in turns if t["at"] >= sw["at"] - 2), None)
            if t:
                r["to_turn_s"] = t["at"] - e["at"]
                r["to_done_s"] = t["done"] - e["at"]
        rows.append(r)
    return rows


def notes_used(intents):
    """Use these sends with a note, and how many of each kind were followed
    by a change to a watched file before the next Use these."""
    uses = [i for i in intents if i["act"] == "use_these" and i["proved"] == "proved"]
    out = {"sends": len(uses), "with_note": 0, "changed_after_note": 0, "changed_after_no_note": 0}
    for u in uses:
        nxt = [i for i in intents if i["i"] > u["i"]]
        stop = next((i for i in nxt if i["act"] in ("use_these", "end_session", "stop_sharing")), None)
        window = [i for i in nxt if stop is None or i["i"] < stop["i"]] or nxt[:1]
        later = window[-1]["files"] if window else u["files"]
        changed = later != u["files"]
        if u.get("note"):
            out["with_note"] += 1
            out["changed_after_note"] += changed
        else:
            out["changed_after_no_note"] += changed
    return out


def lost_attention(intents):
    """Items the owner's UI should surface (a guest waiting to be let in, a
    request for another place) still pending two intents after they appeared."""
    seen, lost = {}, set()
    played = [i for i in intents if "pending" in i]
    for k, it in enumerate(played):
        for item in it["pending"]:
            seen.setdefault(item, k)
            if k - seen[item] >= 2:
                lost.add(item)
    return sorted(lost)


def log_errors(host):
    out = {}
    for f in sorted(host.glob("*-serve.log")):
        lines = [ln for ln in f.read_text(errors="replace").splitlines() if re.search(r"\[(warn|error)\]", ln)]
        out[f.name] = lines
    refusals = {}
    for f in sorted(host.glob("*-audit.jsonl")):
        n = []
        for ln in f.read_text(errors="replace").splitlines():
            try:
                r = json.loads(ln)
            except ValueError:
                continue
            if r.get("kind") == "refusal" or r.get("outcome") not in (None, "ok"):
                n.append(f"{r.get('cmd')}: {r.get('outcome')}")
        refusals[f.name] = n
    return out, refusals


def sources(errs):
    """warn+ lines counted by what wrote them (`<vm>: <word before the colon>`),
    so a usage endpoint's 429s are not read as the conversation failing."""
    out = {}
    for f, lines in errs.items():
        vm = f.split("-")[0]
        for ln in lines:
            m = re.search(r"\] \[\d+\] ([^:]+):", ln)
            k = f"{vm}: {m.group(1) if m else '?'}"
            out[k] = out.get(k, 0) + 1
    return dict(sorted(out.items(), key=lambda kv: -kv[1]))


def fmt(v, unit="s"):
    return "—" if v is None else f"{v:.0f} {unit}" if isinstance(v, (int, float)) else str(v)


def score(d):
    rec, thread, turns_raw, host = load(d)
    intents = rec.get("intents", [])
    turns = all_turns(turns_raw)
    c2a = comment_to_action(thread, turns)
    reached = [r["to_turn_s"] for r in c2a if r["to_turn_s"] is not None]
    notes = notes_used(intents)
    blind = [i for i in intents if i.get("impossible")]
    lost = lost_attention(intents)
    acted = [i for i in intents if i.get("inputs") and not i.get("impossible")]
    per = {}
    for i in acted:
        n = sum(sum(v.values()) for v in i["inputs"].values())
        per.setdefault(i["act"], []).append(n)
    wrong = [i for i in intents if i.get("wrong_place")]
    mangled = [i for i in intents if i.get("text_intact") is False]
    misrouted = [i for i in intents if i.get("sent_to_unshared")]
    errs, refusals = log_errors(host)
    reboot = next((i for i in intents if i["act"] == "reboot_owner"), None)
    rcv = (reboot or {}).get("recovery", {})
    unproved = [i for i in intents if i["proved"] == "unproved" and not i.get("impossible")]
    # Other hosts' shares (§12.2: a guest who hosts too), host/thread-<key>.json.
    others = {}
    for f in sorted(host.glob("thread-*.json")):
        key = f.stem.split("-", 1)[1]
        th = json.loads(f.read_text() or "[]")
        tp = host / f"turns-{key}.json"
        rows = comment_to_action(th, all_turns(json.loads(tp.read_text() or "{}") if tp.exists() else {}))
        got = [r["to_turn_s"] for r in rows if r["to_turn_s"] is not None]
        others[key] = {"comment_to_action": rows, "median_s": statistics.median(got) if got else None,
                       "reached": f"{len(got)}/{len(rows)}"}
    return {
        "other_shares": others,
        "replays": sum(1 for i in intents if any(j["i"] == i["i"] for j in intents if j is not i)) // 2,
        "manual": [f"{i['i']} {i['act']}" for i in intents if i.get("manual")],
        "run": rec["run"], "variant": rec["variant"], "task": rec["task"],
        "intents": len(intents), "proved": sum(i["proved"] == "proved" for i in intents),
        "unproved": [f"{i['i']} {i['who']} {i['act']}: {i['proof']}" for i in unproved],
        "comment_to_action": c2a,
        "comment_to_action_median_s": statistics.median(reached) if reached else None,
        "comments_reached": f"{len(reached)}/{len(c2a)}",
        "notes": notes,
        "blind_spots": [f"{i['who']} {i['act']}: {i['impossible']}" for i in blind],
        "lost_attention": lost,
        "acts_per_outcome": {k: sum(v) for k, v in per.items()},
        "inputs_total": sum(sum(sum(v.values()) for v in i["inputs"].values()) for i in acted),
        "wrong_place": [f"{i['i']} {i['act']}" for i in wrong]
        + [f"{i['i']} {i['act']} reached an unshared session" for i in misrouted],
        "text_changed_on_the_way": [f"seq {i.get('seq')}: {i.get('text')!r} -> {i.get('landed_text')!r}" for i in mangled],
        "log_errors": {k: len(v) for k, v in errs.items()},
        "log_error_sources": sources(errs),
        "log_error_samples": {k: v[:3] for k, v in errs.items()},
        "audit_refusals": {k: len(v) for k, v in refusals.items()},
        "driver_errors": rec.get("errors", []),
        "recovery": rcv,
        "duration_s": round((rec.get("ended_at") or 0) - rec["started_at"]),
    }


def table(s):
    n = s["notes"]
    acts = ", ".join(f"{k} {v}" for k, v in s["acts_per_outcome"].items())
    c2a = "; ".join(f"#{r['seq']} {fmt(r['to_shared_s'])} → turn {fmt(r['to_turn_s'])} → done {fmt(r['to_done_s'])}"
                    for r in s["comment_to_action"])
    rcv = s["recovery"]
    rows = [
        ("Comment to agent action", f"median {fmt(s['comment_to_action_median_s'])} ({s['comments_reached']} reached a turn)", c2a or "no guest posts"),
        ("Notes used", f"{n['with_note']} of {n['sends']} sends had a note",
         f"a watched file changed after {n['changed_after_note']} with a note, {n['changed_after_no_note']} without"),
        ("Blind spots", str(len(s["blind_spots"])), "; ".join(s["blind_spots"]) or "—"),
        ("Lost attention", str(len(s["lost_attention"])), ", ".join(s["lost_attention"]) or "—"),
        ("Acts per outcome", f"{s['inputs_total']} clicks, keys and typed strings", acts),
        ("Wrong place", str(len(s["wrong_place"])), ", ".join(s["wrong_place"]) or "—"),
        ("Errors", f"{sum(s['log_errors'].values())} warn+ log lines, {sum(s['audit_refusals'].values())} audit refusals, "
                   f"{len(s['unproved'])} unproved acts",
         "; ".join(s["unproved"] + [f"{k} ×{v}" for k, v in s["log_error_sources"].items()]) or "—"),
        ("Recovery", f"owner {fmt(rcv.get('owner_serve_s'))}, guests {', '.join(f'{k} {fmt(v)}' for k, v in (rcv.get('guests_back_s') or {}).items()) or '—'}",
         f"share kept: {rcv.get('share_kept')}, binding after: {rcv.get('binding_after')}"
         + (", app reopened by hand" if rcv.get("app_reopened_by_hand") else "") if rcv else "no reboot in this run"),
    ]
    for key, o in s.get("other_shares", {}).items():
        rows.insert(1, (f"Comment to agent action ({key}'s share)", f"median {fmt(o['median_s'])} ({o['reached']} reached a turn)",
                        "; ".join(f"#{r['seq']} {fmt(r['to_shared_s'])} → turn {fmt(r['to_turn_s'])} → done {fmt(r['to_done_s'])}"
                                  for r in o["comment_to_action"]) or "no guest posts"))
    if s.get("manual") or s.get("replays"):
        rows.append(("Replays", f"{s.get('replays', 0)} intents replayed, {len(s.get('manual', []))} done by hand",
                     ", ".join(s.get("manual", [])) or "—"))
    out = [f"**{s['run']}** — {s['variant']}, {s['task']}: {s['proved']}/{s['intents']} intents proved, {s['duration_s']} s",
           "", "| Measure | Value | Detail |", "|---|---|---|"]
    out += [f"| {a} | {b} | {c.replace('|', '/')} |" for a, b, c in rows]
    if s["text_changed_on_the_way"]:
        out += ["", "Text changed between the composer and the thread: " + "; ".join(s["text_changed_on_the_way"])]
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("run_dir")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    s = score(Path(a.run_dir).expanduser())
    print(json.dumps(s, indent=1, default=str) if a.json else table(s))


if __name__ == "__main__":
    sys.exit(main())
