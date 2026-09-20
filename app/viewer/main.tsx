// Entry point of the docs viewer bundle.
//
// This bundle is NOT the app. It is built by `vite.viewer.config.ts` into a
// single classic IIFE, it is never an input to the app's own build, and
// `app/scripts/viewer-boundary-check.mjs` fails if mermaid ever reaches
// `app/src`. See CLAUDE.md, "Design tokens" — mermaid is the second admitted
// exception to the no-UI-libraries rule and this file is the inside of its
// boundary.
//
// Styles are INLINED and injected as one <style> rather than shipped as a
// sibling .css file, so `viewer.js` is the only asset the server has to serve
// (the contract has exactly one bundle route). It costs nothing on the CSP: a
// diagram library that injects 4 379 characters of its own <style> into the SVG
// it returns already requires `style-src 'unsafe-inline'`, and adding a hash
// for our sheet ALONGSIDE it would silently void that keyword and break every
// diagram while the prose kept rendering (docs-render §4, measured: 5
// style-src-elem + 73 style-src-attr violations, diagram gone, everything else
// perfect).
//
// Order matters: tokens define the custom properties, App.css consumes them,
// viewer.css overrides the app-shell rules that are false of a document.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import tokens from "../src/tokens.css?inline";
import app from "../src/App.css?inline";
import viewer from "./viewer.css?inline";
import { Viewer } from "./Viewer";

const style = document.createElement("style");
style.textContent = `${tokens}\n${app}\n${viewer}`;
document.head.appendChild(style);

const host = document.getElementById("root");
if (host) {
  createRoot(host).render(
    <StrictMode>
      <Viewer />
    </StrictMode>,
  );
}
