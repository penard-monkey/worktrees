// The dev stack's token door: `pnpm dev:serve` only (VITE_SERVE=1, `apply:
// "serve"` — it does not exist in a build).
//
// The browser frontend needs `{port, token}` from the stack's `serve.json`
// (mode 0600; `app/scripts/server-stack.sh` prints its path and never the
// token). The rules it is held to: the token is never in a URL (page or socket),
// never in storage, never in a bundle. So the page POSTs to `SERVE_ENDPOINT` and
// keeps the answer in a closure (`ipcws.ts::bootSocket`); the file is read
// fresh per request, so restarting the stack — a new token — needs no vite
// restart.
//
// THE THREAT (measured in vite 7.3.6's source, .planning/findings.md §3): vite's
// default `server.cors` answers ANY `http(s)://localhost:<any port>` or
// `127.0.0.1` origin with `Access-Control-Allow-Origin`, and its cors
// middleware runs BEFORE a plugin's. So under defaults, any other page on
// loopback — another dev server, a page it serves, anything — could fetch this
// endpoint cross-origin and READ the token: a capability for everything the
// engine can do on this machine. Same-origin protects nothing here unless this
// middleware makes it. It refuses unless all of:
//
//   - the method is POST (no `<script src>`/`<img>`/navigation can reach it);
//   - `Sec-Fetch-Site: same-origin` — set by the browser, not by page script;
//     a page on another localhost PORT is `same-site`, and that is refused;
//   - `Origin` is exactly this server's own `http://<Host>`;
//   - `Host` is loopback on the port this socket accepted on (DNS rebinding;
//     vite's `allowedHosts` checks too, and this does not lean on it);
//   - the peer is a loopback address;
//
// and it strips every `Access-Control-*` header the cors middleware already set,
// so even an answer that got through could not be read cross-origin. A local
// process of the same user is out of scope: it can read `serve.json` itself.
import fs from "node:fs";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { Plugin } from "vite";

// Must equal `ipcws.ts::SERVE_ENDPOINT` (transport-check.mjs compares them).
const SERVE_ENDPOINT = "/__worktrees/serve";

const LOOPBACK_PEERS = new Set(["127.0.0.1", "::1", "::ffff:127.0.0.1"]);

function refuse(res: ServerResponse, status: number, body = "") {
  res.statusCode = status;
  res.setHeader("Cache-Control", "no-store");
  res.setHeader("Content-Type", "text/plain; charset=utf-8");
  res.end(body);
}

/** Is this request the dev page itself, asking? Pure, for the check. */
export function sameOriginAsk(req: {
  method?: string;
  headers: Record<string, string | string[] | undefined>;
  peer?: string;
  localPort?: number;
}): boolean {
  const h = (k: string) => {
    const v = req.headers[k];
    return typeof v === "string" ? v : undefined;
  };
  if (req.method !== "POST") return false;
  if (!req.peer || !LOOPBACK_PEERS.has(req.peer)) return false;
  const host = h("host");
  if (!host || (host !== `localhost:${req.localPort}` && host !== `127.0.0.1:${req.localPort}`)) return false;
  if (h("sec-fetch-site") !== "same-origin") return false;
  return h("origin") === `http://${host}`;
}

export function serveDoor(): Plugin {
  return {
    name: "worktrees-serve-door",
    apply: "serve",
    configureServer(server) {
      // A dev server listening beyond loopback (TAURI_DEV_HOST) would put this
      // endpoint on the LAN. Refuse to start rather than serve it there.
      const host = server.config.server.host;
      if (host !== undefined && host !== false && host !== "localhost" && host !== "127.0.0.1") {
        throw new Error(`dev:serve refuses server.host=${String(host)}: the token door is loopback-only`);
      }
      server.middlewares.use(SERVE_ENDPOINT, (req: IncomingMessage, res: ServerResponse) => {
        for (const k of res.getHeaderNames()) if (k.startsWith("access-control-")) res.removeHeader(k);
        const ask = {
          method: req.method,
          headers: req.headers,
          peer: req.socket.remoteAddress,
          localPort: req.socket.localPort,
        };
        if (!sameOriginAsk(ask)) return refuse(res, 403);
        const file = process.env.WT_SERVE_JSON;
        if (!file) {
          return refuse(res, 503,
            "WT_SERVE_JSON is not set. Start the stack with app/scripts/server-stack.sh --allow-origin <this page's origin> " +
            "and run dev:serve with WT_SERVE_JSON=<the path it prints>.");
        }
        let sf: { port?: unknown; token?: unknown };
        try {
          sf = JSON.parse(fs.readFileSync(file, "utf8"));
        } catch (e) {
          return refuse(res, 503, `cannot read ${file}: ${(e as Error).message} — is the stack running?`);
        }
        if (typeof sf.port !== "number" || typeof sf.token !== "string") return refuse(res, 503, `${file} has no port/token`);
        res.statusCode = 200;
        res.setHeader("Cache-Control", "no-store");
        res.setHeader("Content-Type", "application/json");
        res.setHeader("X-Content-Type-Options", "nosniff");
        res.end(JSON.stringify({ port: sf.port, token: sf.token }));
      });
    },
  };
}
