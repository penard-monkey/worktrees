var C = 5e3, L = 262144, B = 6048e5, c = "[redacted]", K = /[A-Za-z0-9._%+-]{1,64}@(?:[A-Za-z0-9-]{1,63}\.){1,8}[A-Za-z]{2,24}/g, J = /\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b/g, G = /\b(?:[A-Fa-f0-9]{1,4}:){2,7}[A-Fa-f0-9]{1,4}\b/g, U = /\b(authorization|bearer|api[-_ ]?key|access[-_ ]?token|refresh[-_ ]?token|token|secret|password|passwd|session[-_ ]?id)\b["']?\s*[:=]\s*["']?([^\s"',;)]+)/gi, F = /\b(?:[sr]k_(?:live|test)_[A-Za-z0-9]{8,}|sk-[A-Za-z0-9_-]{20,}|gh[pousr]_[A-Za-z0-9]{20,}|xox[abprs]-[A-Za-z0-9-]{10,}|AKIA[0-9A-Z]{16})\b/g, Q = /\b[A-Za-z0-9+/_-]{32,}={0,2}\b/g, ee = /((?:https?:\/\/|\/)[^\s"'<>?]*)\?[^\s"'<>]*/g, te = /(\/(?:Users|home))\/[^/\s"']+/g, re = /([A-Za-z]:\\Users\\)[^\\\s"']+/g, j = /\b(?:\d[ -]?){12,18}\d\b/g;
function ne(e) {
  if (e.length !== 13) return !1;
  const t = Number(e);
  return t >= 1e12 && t < 2e12;
}
function ie(e) {
  let t = 0, r = !1;
  for (let n = e.length - 1; n >= 0; n--) {
    let i = e.charCodeAt(n) - 48;
    if (i < 0 || i > 9) return !1;
    r && (i *= 2, i > 9 && (i -= 9)), t += i, r = !r;
  }
  return t % 10 === 0;
}
function oe(e) {
  if (new Set(e).size < 12) return !1;
  const t = /\d/.test(e), r = /[A-Za-z]/.test(e);
  return t && r;
}
function V(e) {
  const t = e.replace(/[ -]/g, "");
  return ne(t) ? e : ie(t) ? c : e;
}
var _ = 8e3;
function l(e) {
  if (!e) return e;
  let t = e.length > _ ? `${e.slice(0, _)}… [truncated]` : e;
  return t = t.replace(U, (r, n) => `${n}=${c}`), t = t.replace(F, c), t = t.replace(K, c), t = t.replace(ee, (r, n) => `${n}?${c}`), t = t.replace(te, (r, n) => `${n}/${c}`), t = t.replace(re, (r, n) => `${n}${c}`), t = t.replace(J, c), t = t.replace(G, c), t = t.replace(j, V), t = t.replace(Q, (r) => oe(r) ? c : r), t;
}
function se(e) {
  if (!e) return e;
  let t = e;
  return t = t.replace(U, (r, n) => `${n}=${c}`), t = t.replace(F, c), t = t.replace(j, V), t;
}
function ae(e) {
  return e.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, "");
}
function ce(e) {
  const t = e.userAgentData?.brands?.filter((n) => !/not.?a.?brand/i.test(n.brand)) ?? [], r = t.find((n) => !/chromium/i.test(n.brand)) ?? t[0];
  return r ? {
    name: r.brand,
    version: r.version
  } : {};
}
async function de() {
  const e = navigator, t = {
    locale: navigator.language,
    timezone: b(() => Intl.DateTimeFormat().resolvedOptions().timeZone),
    viewport: {
      width: Math.round(window.innerWidth),
      height: Math.round(window.innerHeight)
    },
    devicePixelRatio: window.devicePixelRatio,
    pagePath: b(() => new URL(window.location.href).pathname),
    referrerHost: b(() => document.referrer ? new URL(document.referrer).host : void 0)
  };
  e.deviceMemory && (t.memoryGb = e.deviceMemory);
  const r = ce(e);
  r.name && (t.browserName = r.name, t.browserVersion = r.version), e.userAgentData?.platform && (t.platform = e.userAgentData.platform);
  const n = await $(() => e.userAgentData?.getHighEntropyValues?.([
    "platformVersion",
    "model",
    "fullVersionList"
  ]));
  return n && (typeof n.platformVersion == "string" && (t.osVersion = n.platformVersion), typeof n.model == "string" && n.model && (t.deviceModel = n.model)), await $(() => e.xr?.isSessionSupported?.("immersive-vr")) && (t.immersive = !0, !t.deviceModel && /Mac OS X/.test(navigator.userAgent) && "ongesturechange" in window && (t.deviceModel = "Apple Vision Pro", t.osName = "visionOS")), t;
}
function b(e) {
  try {
    return e();
  } catch {
    return;
  }
}
async function $(e) {
  try {
    return await e() ?? void 0;
  } catch {
    return;
  }
}
var x = {
  console: 200,
  errors: 25,
  network: 60
}, v = 2e3, q = 1500, S = class {
  max;
  items = [];
  constructor(e) {
    this.max = e;
  }
  push(e) {
    this.items.push(e), this.items.length > this.max && this.items.shift();
  }
  snapshot() {
    return [...this.items];
  }
  clear() {
    this.items.length = 0;
  }
  get size() {
    return this.items.length;
  }
}, A = new S(x.console), E = new S(x.errors), D = new S(x.network), z = !1, y = !1, m = null;
function d(e, t) {
  return e.length > t ? `${e.slice(0, t)}… [+${e.length - t} chars]` : e;
}
function H(e, t = 0) {
  if (e === null) return "null";
  if (e === void 0) return "undefined";
  const r = typeof e;
  if (r === "string") return e;
  if (r === "number" || r === "boolean" || r === "bigint") return String(e);
  if (r === "function") return `[Function ${e.name || "anonymous"}]`;
  if (r === "symbol") return String(e);
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  if (typeof Element < "u" && e instanceof Element) return `<${e.tagName.toLowerCase()}>`;
  if (t > 2) return "[nested]";
  try {
    const n = /* @__PURE__ */ new WeakSet();
    return JSON.stringify(e, (i, o) => {
      if (typeof o == "object" && o !== null) {
        if (n.has(o)) return "[circular]";
        n.add(o);
      }
      return o;
    }) ?? String(e);
  } catch {
    return "[unserializable]";
  }
}
function le(e, t) {
  if (!y) return;
  const r = t.map((n) => H(n)).join(" ");
  r && A.push({
    t: Date.now(),
    level: e,
    text: d(l(r), v)
  });
}
function p(e) {
  try {
    const t = new URL(e, window.location.href);
    return d(t.origin === window.location.origin ? t.pathname : `${t.origin}${t.pathname}`, 300);
  } catch {
    return d(l(String(e).split("?")[0] ?? ""), 300);
  }
}
function M(e) {
  if (!m) return !1;
  try {
    return new URL(e, window.location.href).host === m;
  } catch {
    return !1;
  }
}
function g(e) {
  y && D.push(e);
}
function O(e) {
  y && E.push(e);
}
function ue() {
  for (const e of [
    "log",
    "info",
    "warn",
    "error",
    "debug"
  ]) {
    const t = console[e]?.bind(console);
    t && (console[e] = (...r) => {
      try {
        le(e, r);
      } catch {
      }
      t(...r);
    });
  }
}
function fe() {
  window.addEventListener("error", (e) => {
    O({
      t: Date.now(),
      kind: "error",
      message: d(l(e.message ?? "unknown error"), v),
      source: e.filename ? p(e.filename) : void 0,
      line: e.lineno || void 0,
      column: e.colno || void 0,
      stack: e.error instanceof Error && e.error.stack ? d(l(e.error.stack), q) : void 0
    });
  }), window.addEventListener("unhandledrejection", (e) => {
    const t = e.reason;
    O({
      t: Date.now(),
      kind: "unhandledrejection",
      message: d(l(t instanceof Error ? t.message : H(t)), v),
      stack: t instanceof Error && t.stack ? d(l(t.stack), q) : void 0
    });
  });
}
function he() {
  if (typeof window.fetch != "function") return;
  const e = window.fetch.bind(window);
  window.fetch = async (t, r) => {
    const n = typeof t == "string" ? t : t instanceof URL ? t.href : t.url, i = (r?.method ?? (t instanceof Request ? t.method : "GET")).toUpperCase(), o = performance.now();
    if (M(n)) return e(t, r);
    try {
      const s = await e(t, r);
      return g({
        t: Date.now(),
        method: i,
        path: p(n),
        status: s.status,
        durationMs: Math.round(performance.now() - o),
        initiator: "fetch"
      }), s;
    } catch (s) {
      throw g({
        t: Date.now(),
        method: i,
        path: p(n),
        status: "failed",
        durationMs: Math.round(performance.now() - o),
        initiator: "fetch"
      }), s;
    }
  };
}
function pe() {
  if (typeof XMLHttpRequest != "function") return;
  const e = XMLHttpRequest.prototype, t = e.open, r = e.send;
  e.open = function(n, i, ...o) {
    return this.__orfis = {
      method: String(n).toUpperCase(),
      url: String(i),
      started: 0
    }, t.apply(this, [
      n,
      i,
      ...o
    ]);
  }, e.send = function(...n) {
    const i = this.__orfis;
    if (i && !M(i.url)) {
      i.started = performance.now();
      const o = (s) => {
        g({
          t: Date.now(),
          method: i.method,
          path: p(i.url),
          status: s ? "failed" : this.status,
          durationMs: Math.round(performance.now() - i.started),
          initiator: "xhr"
        });
      };
      this.addEventListener("loadend", () => o(this.status === 0), { once: !0 });
    }
    return r.apply(this, n);
  };
}
function me() {
  if (typeof PerformanceObserver == "function")
    try {
      new PerformanceObserver((e) => {
        for (const t of e.getEntries()) {
          const r = t;
          r.initiatorType === "fetch" || r.initiatorType === "xmlhttprequest" || M(r.name) || r.transferSize !== 0 || r.duration === 0 || r.decodedBodySize > 0 || (r.responseStatus ?? 0) > 0 || g({
            t: Date.now(),
            method: "GET",
            path: p(r.name),
            status: "failed",
            durationMs: Math.round(r.duration),
            initiator: "resource"
          });
        }
      }).observe({
        type: "resource",
        buffered: !0
      });
    } catch {
    }
}
function ge(e = {}) {
  if (!(typeof window > "u")) {
    if (y = !0, e.apiUrl) try {
      m = new URL(e.apiUrl, window.location.href).host;
    } catch {
      m = null;
    }
    z || (z = !0, ue(), fe(), he(), pe(), me());
  }
}
function ye() {
  return {
    console: A.snapshot(),
    jserrors: E.snapshot(),
    network: D.snapshot()
  };
}
function be() {
  return {
    console: A.size,
    jserrors: E.size,
    network: D.size
  };
}
function we(e) {
  const t = ye(), r = [
    {
      kind: "jserrors",
      entries: t.jserrors
    },
    {
      kind: "network",
      entries: t.network
    },
    {
      kind: "console",
      entries: t.console
    }
  ].filter((i) => i.entries.length > 0), n = () => new TextEncoder().encode(JSON.stringify(r)).length;
  for (let i = 0; i < 1e4 && n() > e; i++) {
    const o = [...r].reverse().find((s) => s.entries.length > 0);
    if (!o) break;
    o.entries.shift();
  }
  return r.filter((i) => i.entries.length > 0);
}
var h = "orfis:outbox:v1", ve = 12, ke = 1e6, X = B, xe = 2e4, Se = 18e5, Ae = 20, Ee = 6e4, De = class {
  options;
  fetchImpl;
  memory = [];
  usingMemory = !1;
  probed = null;
  timer = null;
  flushing = !1;
  started = !1;
  constructor(e = {}) {
    this.options = e, this.fetchImpl = e.fetchImpl ?? ((...t) => fetch(...t));
  }
  start() {
    this.started || (this.started = !0, typeof window < "u" && (window.addEventListener("online", this.onOnline), document.addEventListener("visibilitychange", this.onVisible)), this.flush());
  }
  stop() {
    this.started && (this.started = !1, typeof window < "u" && (window.removeEventListener("online", this.onOnline), document.removeEventListener("visibilitychange", this.onVisible)), this.timer !== null && (clearTimeout(this.timer), this.timer = null));
  }
  get pending() {
    return this.read().length;
  }
  async send(e, t) {
    const r = await this.post(e, t);
    if (r.kind === "sent")
      return this.options.onSent?.(r.id), this.flush(), r;
    if (r.kind === "terminal") return {
      kind: "rejected",
      status: r.status
    };
    const n = {
      id: Le(),
      endpoint: e,
      payload: t,
      createdAt: Date.now(),
      attempts: 1,
      nextAttemptAt: Date.now() + R(1, r.retryAfterMs),
      leaseUntil: 0
    }, i = this.enqueue(n);
    return i.stored ? (this.schedule(), i.durable ? {
      kind: "queued",
      stored: !0,
      durable: !0
    } : {
      kind: "queued",
      stored: !0,
      durable: !1,
      reason: "memory-fallback"
    }) : {
      kind: "failed",
      stored: !1,
      durable: !1,
      reason: i.reason
    };
  }
  async flush() {
    if (!this.flushing) {
      this.flushing = !0;
      try {
        if (P()) return;
        const e = Date.now(), t = this.read().filter((r) => r.nextAttemptAt <= e && r.leaseUntil <= e);
        for (const r of t) {
          if (P()) break;
          if (!this.claim(r.id)) continue;
          const n = await this.post(r.endpoint, {
            ...r.payload,
            bufferedMs: Me(r.createdAt)
          });
          if (n.kind === "sent") {
            this.remove(r.id), this.options.onSent?.(n.id);
            continue;
          }
          if (n.kind === "terminal") {
            this.remove(r.id);
            continue;
          }
          this.defer(r.id, n.retryAfterMs);
        }
      } finally {
        this.flushing = !1, this.schedule();
      }
    }
  }
  async post(e, t) {
    let r;
    try {
      r = await this.fetchImpl(e, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(t)
      });
    } catch {
      return { kind: "retry" };
    }
    if (r.status === 202) {
      const n = await r.json().catch(() => null);
      return n && typeof n.id == "string" && n.id.length > 0 && (n.status === "accepted" || n.status === "duplicate") ? {
        kind: "sent",
        id: n.id
      } : { kind: "retry" };
    }
    if (r.status >= 200 && r.status < 300) return { kind: "retry" };
    if (r.status === 429) {
      const n = Number(r.headers.get("retry-after"));
      return {
        kind: "retry",
        retryAfterMs: Number.isFinite(n) && n > 0 ? n * 1e3 : void 0
      };
    }
    return r.status === 408 || r.status === 425 || r.status >= 500 ? { kind: "retry" } : {
      kind: "terminal",
      status: r.status
    };
  }
  storage() {
    if (this.usingMemory) return null;
    if (this.options.storage !== void 0)
      return this.options.storage || (this.usingMemory = !0), this.options.storage;
    if (this.probed) return this.probed;
    try {
      const e = window.localStorage;
      return e.setItem(`${h}:probe`, "1"), e.removeItem(`${h}:probe`), this.probed = e, e;
    } catch {
      return this.usingMemory = !0, null;
    }
  }
  read() {
    const e = this.storage();
    if (!e)
      return this.memory = w(this.memory), this.memory;
    let t;
    try {
      const n = e.getItem(h);
      if (!n) return [];
      const i = JSON.parse(n);
      if (!Array.isArray(i)) return [];
      t = i.filter(Ce);
    } catch {
      return [];
    }
    const r = w(t);
    return r.length !== t.length && this.write(r), r;
  }
  write(e) {
    const t = this.storage();
    if (!t)
      return this.memory = e, !0;
    let r = e;
    for (let n = 0; n < 6; n++) try {
      return t.setItem(h, JSON.stringify(r)), !0;
    } catch {
      const i = N(r);
      if (!i) {
        try {
          t.removeItem(h);
        } catch {
        }
        return !1;
      }
      r = i;
    }
    return !1;
  }
  enqueue(e) {
    let t = [...this.read(), e];
    for (; t.length > ve || Te(t) > ke; ) {
      const r = N(t);
      if (!r) return {
        stored: !1,
        reason: "quota"
      };
      t = r;
    }
    return t.some((r) => r.id === e.id) ? this.write(t) ? this.read().some((r) => r.id === e.id) ? {
      stored: !0,
      durable: !this.usingMemory
    } : {
      stored: !1,
      reason: "not-retained"
    } : {
      stored: !1,
      reason: "quota"
    } : {
      stored: !1,
      reason: "quota"
    };
  }
  remove(e) {
    this.write(this.read().filter((t) => t.id !== e));
  }
  claim(e) {
    const t = this.read(), r = t.find((n) => n.id === e);
    return !r || r.leaseUntil > Date.now() ? !1 : (r.leaseUntil = Date.now() + Ee, this.write(t));
  }
  defer(e, t) {
    const r = this.read(), n = r.find((i) => i.id === e);
    n && (n.attempts += 1, n.leaseUntil = 0, n.nextAttemptAt = Date.now() + R(n.attempts, t), this.write(w(r)));
  }
  schedule() {
    if (this.timer !== null && (clearTimeout(this.timer), this.timer = null), !this.started) return;
    const e = this.read().reduce((r, n) => r === null || n.nextAttemptAt < r ? n.nextAttemptAt : r, null);
    if (e === null) return;
    const t = Math.max(e - Date.now(), 1e3);
    this.timer = setTimeout(() => {
      this.timer = null, this.flush();
    }, t);
  }
  onOnline = () => {
    this.flush();
  };
  onVisible = () => {
    document.visibilityState === "visible" && this.flush();
  };
};
function Me(e) {
  return Math.min(Math.max(Math.round(Date.now() - e), 0), X);
}
function w(e) {
  const t = Date.now() - X;
  return e.filter((r) => r.createdAt > t && r.attempts < Ae);
}
function R(e, t) {
  const r = Math.min(xe * 2 ** (e - 1), Se) * (0.8 + Math.random() * 0.4);
  return Math.max(r, t ?? 0);
}
function N(e) {
  const t = e.find((n) => n.payload.diagnostics?.length);
  if (t) return e.map((n) => n === t ? {
    ...n,
    payload: {
      ...n.payload,
      diagnostics: void 0
    }
  } : n);
  if (e.length === 0) return null;
  const r = e.reduce((n, i) => n.createdAt <= i.createdAt ? n : i);
  return e.filter((n) => n !== r);
}
function Te(e) {
  return new TextEncoder().encode(JSON.stringify(e)).length;
}
function P() {
  return typeof navigator < "u" && navigator.onLine === !1;
}
function Ce(e) {
  if (typeof e != "object" || e === null) return !1;
  const t = e;
  return typeof t.id == "string" && typeof t.endpoint == "string" && typeof t.createdAt == "number" && typeof t.attempts == "number" && typeof t.nextAttemptAt == "number" && typeof t.leaseUntil == "number" && typeof t.payload == "object" && t.payload !== null && typeof t.payload.message == "string";
}
function Le() {
  const e = Math.random().toString(36).slice(2, 10);
  return `${Date.now().toString(36)}-${e}`;
}
var _e = `
:host {
  all: initial;
  --orfis-ink: #16181d;
  --orfis-muted: #5d6470;
  --orfis-ground: #ffffff;
  --orfis-sunk: #f4f2ee;
  --orfis-line: #d9d5cd;
  --orfis-accent: #275a4e;
  --orfis-accent-ink: #ffffff;
  --orfis-danger: #a13a29;
  font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif;
  color: var(--orfis-ink);
}
@media (prefers-color-scheme: dark) {
  :host {
    --orfis-ink: #eceae5;
    --orfis-muted: #9aa0ab;
    --orfis-ground: #1b1e24;
    --orfis-sunk: #242830;
    --orfis-line: #363b44;
    --orfis-accent: #7fc7b3;
    --orfis-accent-ink: #10221d;
    --orfis-danger: #e88b78;
  }
}
:host([data-orfis-theme="light"]) {
  --orfis-ink: #16181d;
  --orfis-muted: #5d6470;
  --orfis-ground: #ffffff;
  --orfis-sunk: #f4f2ee;
  --orfis-line: #d9d5cd;
  --orfis-accent: #275a4e;
  --orfis-accent-ink: #ffffff;
  --orfis-danger: #a13a29;
}
:host([data-orfis-theme="dark"]) {
  --orfis-ink: #eceae5;
  --orfis-muted: #9aa0ab;
  --orfis-ground: #1b1e24;
  --orfis-sunk: #242830;
  --orfis-line: #363b44;
  --orfis-accent: #7fc7b3;
  --orfis-accent-ink: #10221d;
  --orfis-danger: #e88b78;
}
* { box-sizing: border-box; }

.launcher {
  position: fixed; inset-block-end: 1.25rem; inset-inline-end: 1.25rem; z-index: 2147483000;
  padding: .7rem 1.15rem; border: 0; border-radius: 999px; cursor: pointer;
  background: var(--orfis-accent); color: var(--orfis-accent-ink);
  font: 600 .9rem/1 inherit; box-shadow: 0 6px 20px rgb(0 0 0 / .18);
}
.launcher:hover { filter: brightness(1.06); }

.backdrop {
  position: fixed; inset: 0; z-index: 2147483001; display: grid; place-items: center;
  padding: 1rem; background: rgb(12 14 18 / .45); backdrop-filter: blur(2px);
}
.panel {
  width: min(30rem, 100%); max-height: min(42rem, 90vh); overflow-y: auto;
  display: flex; flex-direction: column; gap: 1rem;
  padding: 1.5rem; border-radius: 14px; border: 1px solid var(--orfis-line);
  background: var(--orfis-ground); box-shadow: 0 24px 60px rgb(0 0 0 / .3);
}
h2 { margin: 0; font-size: 1.15rem; font-weight: 650; }
p  { margin: 0; color: var(--orfis-muted); font-size: .875rem; line-height: 1.5; }

.types { display: flex; gap: .5rem; border: 0; padding: 0; margin: 0; }
.types legend { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
.type {
  flex: 1; display: flex; align-items: center; justify-content: center; gap: .5rem;
  padding: .6rem; border: 1px solid var(--orfis-line); border-radius: 10px; cursor: pointer;
  font-size: .9rem; background: var(--orfis-ground);
}
.type:has(input:checked) { border-color: var(--orfis-accent); background: var(--orfis-sunk); font-weight: 600; }
.type:has(input:focus-visible) { outline: 2px solid var(--orfis-accent); outline-offset: 2px; }
.type input { accent-color: var(--orfis-accent); margin: 0; }

label.field { display: flex; flex-direction: column; gap: .35rem; font-size: .82rem; color: var(--orfis-muted); }
textarea, input[type="email"] {
  width: 100%; padding: .65rem .75rem; border-radius: 10px; font: inherit; font-size: .9rem;
  border: 1px solid var(--orfis-line); background: var(--orfis-ground); color: var(--orfis-ink);
}
textarea { min-height: 7.5rem; resize: vertical; }
textarea:focus-visible, input:focus-visible { outline: 2px solid var(--orfis-accent); outline-offset: 1px; }

.disclosure { border: 1px solid var(--orfis-line); border-radius: 10px; background: var(--orfis-sunk); padding: .75rem; }
.disclosure > p { font-size: .8rem; }
.disclosure-actions { display: flex; align-items: center; gap: 1rem; margin-block-start: .5rem; flex-wrap: wrap; }
.linky { border: 0; background: none; padding: 0; cursor: pointer; font: inherit; font-size: .8rem;
         color: var(--orfis-accent); text-decoration: underline; }
.optout { display: flex; align-items: center; gap: .4rem; font-size: .8rem; color: var(--orfis-muted); }
pre {
  margin: .6rem 0 0; padding: .6rem; max-height: 11rem; overflow: auto; border-radius: 8px;
  background: var(--orfis-ground); border: 1px solid var(--orfis-line);
  font: .72rem/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; white-space: pre-wrap; word-break: break-word;
}

.actions { display: flex; justify-content: flex-end; gap: .6rem; align-items: center; }
button.primary, button.ghost {
  padding: .6rem 1.15rem; border-radius: 999px; font: 600 .875rem/1 inherit; cursor: pointer;
}
button.primary { border: 0; background: var(--orfis-accent); color: var(--orfis-accent-ink); }
button.ghost { border: 1px solid var(--orfis-line); background: transparent; color: var(--orfis-ink); }
button:disabled { opacity: .55; cursor: not-allowed; }
button:focus-visible { outline: 2px solid var(--orfis-accent); outline-offset: 2px; }

.error { color: var(--orfis-danger); font-size: .82rem; }
.counter { margin-inline-end: auto; font-size: .75rem; color: var(--orfis-muted); font-variant-numeric: tabular-nums; }
.honeypot { position: absolute; left: -9999px; width: 1px; height: 1px; opacity: 0; }
.done { display: flex; flex-direction: column; gap: .5rem; }

@media (prefers-reduced-motion: no-preference) {
  .panel { animation: rise .18s ease-out; }
  @keyframes rise { from { transform: translateY(8px); opacity: 0; } to { transform: none; opacity: 1; } }
}
`, $e = "We could not send that. Check your connection and try again.", qe = "We could not accept that one. Please try again, or get in touch another way.", ze = class {
  options;
  host;
  refs;
  outbox;
  device = {};
  openedAt = 0;
  lastFocused = null;
  extra = [];
  extraPending = null;
  destroyed = !1;
  constructor(e) {
    this.options = {
      launcher: !0,
      launcherLabel: "Feedback",
      title: "Send feedback",
      captureDiagnostics: !0,
      collectDeviceContext: !0,
      askForEmail: !0,
      theme: "system",
      ...e
    }, this.options.captureDiagnostics && ge({ apiUrl: e.apiUrl }), this.outbox = new De({ onSent: (n) => this.options.onSubmitted?.(n) }), this.outbox.start(), this.host = document.createElement("div"), this.host.setAttribute("data-orfis", ""), this.host.dataset.orfisTheme = this.options.theme;
    const t = this.host.attachShadow({ mode: "open" }), r = document.createElement("style");
    r.textContent = _e, t.append(r), this.refs = {
      root: t,
      dialog: null
    }, document.body.append(this.host), this.options.launcher && this.renderLauncher(), this.options.collectDeviceContext && de().then((n) => {
      this.device = n;
    });
  }
  renderLauncher() {
    const e = document.createElement("button");
    e.className = "launcher", e.type = "button", e.textContent = this.options.launcherLabel ?? "Feedback", e.addEventListener("click", () => this.open()), this.refs.root.append(e);
  }
  open() {
    if (this.destroyed || this.refs.dialog) return;
    this.lastFocused = document.activeElement, this.openedAt = Date.now();
    const e = document.createElement("div");
    e.className = "backdrop", e.addEventListener("mousedown", (r) => {
      r.target === e && this.close();
    });
    const t = document.createElement("div");
    t.className = "panel", t.setAttribute("role", "dialog"), t.setAttribute("aria-modal", "true"), t.setAttribute("aria-label", this.options.title ?? "Send feedback"), t.append(this.buildForm()), e.append(t), this.refs.root.append(e), this.refs.dialog = e, document.addEventListener("keydown", this.onKeydown, !0), t.querySelector("#orfis-message")?.focus(), this.options.onOpen?.(), this.refreshDisclosure();
  }
  async refreshDisclosure() {
    if (this.options.extraDiagnostics) {
      const i = (async () => {
        try {
          this.extra = await this.options.extraDiagnostics();
        } catch {
          this.extra = [];
        }
      })();
      this.extraPending = i, await i, this.extraPending === i && (this.extraPending = null);
    }
    const e = this.refs.dialog?.querySelector("form");
    if (!e) return;
    const t = e.querySelector(".disclosure");
    if (!t || t.hidden) return;
    const r = e.querySelector("[data-diag-summary]");
    r && (r.textContent = this.describeDiagnostics());
    const n = e.querySelector("pre");
    n && (n.textContent = JSON.stringify(this.buildDiagnosticsPreview(), null, 2));
  }
  close() {
    this.refs.dialog && (this.refs.dialog.remove(), this.refs.dialog = null, document.removeEventListener("keydown", this.onKeydown, !0), this.lastFocused instanceof HTMLElement && this.lastFocused.focus(), this.options.onClose?.());
  }
  destroy() {
    this.destroyed || (this.destroyed = !0, this.close(), this.outbox.stop(), this.host.remove());
  }
  setTheme(e) {
    this.options.theme = e, this.host.dataset.orfisTheme = e;
  }
  get pending() {
    return this.outbox.pending;
  }
  onKeydown = (e) => {
    if (e.isComposing) return;
    if (e.key === "Escape") {
      e.stopPropagation(), this.close();
      return;
    }
    if (e.key !== "Tab" || !this.refs.dialog) return;
    const t = [...this.refs.dialog.querySelectorAll('button, textarea, input, [href], select, [tabindex]:not([tabindex="-1"])')].filter((o) => !o.hasAttribute("disabled") && o.offsetParent !== null);
    if (t.length === 0) return;
    const r = t[0], n = t[t.length - 1], i = this.refs.root.activeElement;
    e.shiftKey && i === r ? (e.preventDefault(), n.focus()) : !e.shiftKey && i === n && (e.preventDefault(), r.focus());
  };
  buildForm() {
    const e = document.createElement("form");
    e.noValidate = !0, e.innerHTML = `
      <h2>${Ne(this.options.title ?? "Send feedback")}</h2>
      <p>One question, then tell us what happened. We read all of it.</p>
      <fieldset class="types">
        <legend>What kind of message is this?</legend>
        <label class="type"><input type="radio" name="type" value="feedback" checked> Feedback</label>
        <label class="type"><input type="radio" name="type" value="bug"> Bug</label>
      </fieldset>
      <label class="field" for="orfis-message">Your message
        <textarea id="orfis-message" name="message" required maxlength="${C}"
          placeholder="What happened, or what would you change?"></textarea>
      </label>
      ${this.options.askForEmail ? `<label class="field" for="orfis-email">Email (optional — only if you want a reply)
        <input id="orfis-email" type="email" name="contactEmail" autocomplete="email">
      </label>` : ""}
      <div class="disclosure" hidden>
        <p><strong>Diagnostic data will be included</strong> so we can reproduce this.
           <span data-diag-summary></span>
           Addresses, tokens, email addresses and file names are removed before it is sent, and
           the contents of network requests are never read.</p>
        <div class="disclosure-actions">
          <button type="button" class="linky" data-toggle-data>View what is included</button>
          <label class="optout"><input type="checkbox" name="noDiagnostics"> Do not include it</label>
        </div>
        <pre hidden></pre>
      </div>
      <input class="honeypot" type="text" name="website" tabindex="-1" autocomplete="off" aria-hidden="true">
      <p class="error" hidden></p>
      <div class="actions">
        <span class="counter"></span>
        <button type="button" class="ghost" data-cancel>Cancel</button>
        <button type="submit" class="primary">Send</button>
      </div>`;
    const t = e.querySelector("#orfis-message"), r = e.querySelector(".counter"), n = e.querySelector(".disclosure"), i = e.querySelector("pre"), o = e.querySelector(".error"), s = () => {
      const a = 10 - t.value.trim().length;
      r.textContent = a > 0 ? `${a} more character${a === 1 ? "" : "s"}` : "";
    };
    t.addEventListener("input", s), s();
    for (const a of e.querySelectorAll('input[name="type"]')) a.addEventListener("change", () => {
      n.hidden = a.value !== "bug" || !a.checked || !this.canAttach(), !n.hidden && this.refreshDisclosure();
    });
    return e.querySelector("[data-toggle-data]").addEventListener("click", (a) => {
      const u = a.currentTarget;
      i.hidden = !i.hidden, u.textContent = i.hidden ? "View what is included" : "Hide";
    }), e.querySelector("[data-cancel]").addEventListener("click", () => this.close()), e.addEventListener("submit", (a) => {
      a.preventDefault(), this.submit(e, o);
    }), e;
  }
  async submit(e, t) {
    const r = new FormData(e), n = ae(String(r.get("message") ?? "")).trim(), i = String(r.get("type") ?? "feedback"), o = r.get("noDiagnostics") === "on";
    if (t.hidden = !0, n.length < 10) {
      t.textContent = "Please write at least 10 characters so we can act on it.", t.hidden = !1, e.querySelector("#orfis-message")?.focus();
      return;
    }
    const s = e.querySelector('button[type="submit"]');
    s.disabled = !0, s.textContent = "Sending…";
    const a = String(r.get("contactEmail") ?? "").trim(), u = i === "bug" && !o;
    u && await this.extraPending;
    const W = se(n).slice(0, C), T = u ? this.buildDiagnosticsPayload() : [], Y = String(r.get("website") ?? ""), Z = {
      key: this.options.key,
      type: i,
      message: W,
      surface: this.options.surface,
      appVersion: this.options.appVersion,
      licenseRef: this.options.licenseRef,
      userRef: this.options.userRef,
      contactEmail: a || void 0,
      device: this.options.collectDeviceContext ? u ? this.device : Re(this.device) : void 0,
      diagnostics: T.length > 0 ? T : void 0,
      honeypot: Y || void 0,
      elapsedMs: this.options.collectDeviceContext ? Date.now() - this.openedAt : void 0
    }, f = await this.outbox.send(`${this.options.apiUrl.replace(/\/$/, "")}/v1/feedback`, Z);
    if (f.kind === "sent") {
      this.renderThanks();
      return;
    }
    if (f.kind === "queued") {
      this.options.onQueued?.(f), this.renderThanks({
        queued: !0,
        durable: f.durable
      });
      return;
    }
    t.textContent = f.kind === "rejected" ? qe : $e, t.hidden = !1, s.disabled = !1, s.textContent = "Send";
  }
  canAttach() {
    return !!(this.options.collectDeviceContext || this.options.captureDiagnostics || this.options.extraDiagnostics);
  }
  buildDiagnosticsPayload() {
    const e = Oe(this.extra.filter((n) => n.entries.length > 0), L), t = k(e), r = this.options.captureDiagnostics !== !1 ? we(Math.max(L - t, 0)) : [];
    return [...e, ...r];
  }
  buildDiagnosticsPreview() {
    const e = this.options.collectDeviceContext ? { device: this.device } : {};
    for (const t of this.buildDiagnosticsPayload()) e[t.kind] = t.entries;
    return e;
  }
  describeDiagnostics() {
    const e = [];
    if (this.options.captureDiagnostics !== !1) {
      const r = be();
      r.jserrors && e.push(`${r.jserrors} error${r.jserrors === 1 ? "" : "s"}`), r.network && e.push(`${r.network} network request${r.network === 1 ? "" : "s"}`), r.console && e.push(`${r.console} console line${r.console === 1 ? "" : "s"}`);
    }
    const t = this.extra.reduce((r, n) => r + n.entries.length, 0);
    return t && e.push(`${t} app log line${t === 1 ? "" : "s"}`), this.options.collectDeviceContext ? e.length === 0 ? "This is your device and app version — nothing else was recorded." : `That is your device, plus ${I(e)}.` : e.length === 0 ? "Nothing has been recorded so far." : `That is ${I(e)}.`;
  }
  renderThanks({ queued: e = !1, durable: t = !0 } = {}) {
    const r = this.refs.dialog?.querySelector(".panel");
    if (!r) return;
    r.innerHTML = `
      <div class="done">
        <h2>Thank you</h2>
        <p>${e ? t ? "We could not send that just now, so your note is saved on this device and will be retried automatically. Nothing else is needed from you." : "We could not send that just now. Your note is held in memory only: it will be retried while this window stays open, and lost if the window closes first." : "Your note is in the queue. Nothing else is needed from you."}</p>
      </div>
      <div class="actions"><button type="button" class="primary" data-close>Close</button></div>`;
    const n = r.querySelector("[data-close]");
    n.addEventListener("click", () => this.close()), n.focus();
  }
};
function k(e) {
  return new TextEncoder().encode(JSON.stringify(e)).length;
}
function Oe(e, t) {
  if (k(e) <= t) return e;
  const r = e.map((n) => ({
    ...n,
    entries: [...n.entries]
  }));
  for (let n = 0; n < 1e4 && k(r) > t; n++) {
    const i = [...r].reverse().find((o) => o.entries.length > 0);
    if (!i) break;
    i.entries.splice(0, Math.max(1, Math.floor(i.entries.length / 10)));
  }
  return r.filter((n) => n.entries.length > 0);
}
function I(e) {
  return e.length === 1 ? e[0] : `${e.slice(0, -1).join(", ")} and ${e.at(-1)}`;
}
function Re(e) {
  const { browserName: t, osName: r, locale: n, viewport: i, pagePath: o, immersive: s, deviceModel: a } = e;
  return {
    browserName: t,
    osName: r,
    locale: n,
    viewport: i,
    pagePath: o,
    immersive: s,
    deviceModel: a
  };
}
function Ne(e) {
  return e.replace(/[&<>"']/g, (t) => `&#${t.charCodeAt(0)};`);
}
function Pe(e) {
  return new ze(e);
}
function Ie() {
  const e = document.currentScript, t = e?.dataset.orfisKey, r = e?.dataset.orfisApi;
  if (!t || !r) return;
  const n = () => Pe({
    key: t,
    apiUrl: r,
    surface: e?.dataset.orfisSurface ?? "website",
    appVersion: e?.dataset.orfisVersion,
    launcherLabel: e?.dataset.orfisLabel
  });
  document.readyState === "loading" ? document.addEventListener("DOMContentLoaded", n, { once: !0 }) : n();
}
Ie();
export {
  ze as OrfisWidget,
  de as collectDeviceContext,
  Pe as init
};

//# sourceMappingURL=orfis.es.js.map