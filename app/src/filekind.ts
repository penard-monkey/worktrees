// What the dock's file viewer should DO with a path, decided from its name
// alone (extension, or a bare name like `Makefile`). Pure + dependency-free so
// both the viewer and the tree icons can ask the same question.
//
// `kind` picks the renderer; `lang` picks the highlighter grammar (see
// highlight.ts); `mime` is only meaningful for images (data: URI prefix).

export type FileKind = "markdown" | "image" | "code" | "text" | "binary";

export type FileInfo = {
  kind: FileKind;
  /** highlight.ts grammar id, "" = no highlighting */
  lang: string;
  /** image mime for the data: URI, "" otherwise */
  mime: string;
  /** short label for the viewer header ("TypeScript", "PNG", …) */
  label: string;
};

// ext → [lang, label]. One table, so adding a language is one line.
const CODE: Record<string, [string, string]> = {
  ts: ["ts", "TypeScript"], tsx: ["ts", "TypeScript"], mts: ["ts", "TypeScript"], cts: ["ts", "TypeScript"],
  js: ["ts", "JavaScript"], jsx: ["ts", "JavaScript"], mjs: ["ts", "JavaScript"], cjs: ["ts", "JavaScript"],
  rs: ["rust", "Rust"],
  py: ["python", "Python"], pyi: ["python", "Python"],
  go: ["go", "Go"],
  rb: ["ruby", "Ruby"],
  java: ["clike", "Java"], kt: ["clike", "Kotlin"], swift: ["clike", "Swift"], scala: ["clike", "Scala"],
  c: ["clike", "C"], h: ["clike", "C"], cc: ["clike", "C++"], cpp: ["clike", "C++"], hpp: ["clike", "C++"],
  m: ["clike", "MATLAB/Obj-C"], cs: ["clike", "C#"], php: ["clike", "PHP"], dart: ["clike", "Dart"],
  json: ["json", "JSON"], jsonc: ["json", "JSON"], json5: ["json", "JSON"],
  yaml: ["yaml", "YAML"], yml: ["yaml", "YAML"],
  toml: ["toml", "TOML"], ini: ["ini", "INI"], cfg: ["ini", "Config"], conf: ["ini", "Config"],
  properties: ["ini", "Properties"],
  sh: ["shell", "Shell"], bash: ["shell", "Bash"], zsh: ["shell", "Zsh"], fish: ["shell", "Fish"],
  bats: ["shell", "bats"], zprofile: ["shell", "Shell"], bashrc: ["shell", "Shell"],
  css: ["css", "CSS"], scss: ["scss", "SCSS"], sass: ["scss", "Sass"], less: ["scss", "Less"],
  html: ["xml", "HTML"], htm: ["xml", "HTML"], xml: ["xml", "XML"], svg: ["xml", "SVG"],
  sql: ["sql", "SQL"],
  vim: ["shell", "Vim"], lua: ["clike", "Lua"], nix: ["clike", "Nix"],
  gitignore: ["shell", "gitignore"], gitattributes: ["shell", "git"], editorconfig: ["ini", "EditorConfig"],
  env: ["shell", "env"], diff: ["diff", "Diff"], patch: ["diff", "Patch"],
  zshrc: ["shell", "Zsh"], vimrc: ["shell", "Vim"], profile: ["shell", "Shell"],
  npmrc: ["ini", "npm config"], prettierrc: ["json", "Prettier"], eslintrc: ["json", "ESLint"],
  mod: ["shell", "Go module"], sum: ["shell", "Go checksums"],
};

// Bare filenames with no extension that still have a grammar.
const BY_NAME: Record<string, [string, string]> = {
  makefile: ["shell", "Makefile"],
  // Lockfiles share an extension but not a format: only these two are TOML.
  "cargo.lock": ["toml", "Lockfile"],
  "poetry.lock": ["toml", "Lockfile"],
  "flake.lock": ["json", "Lockfile"],
  "composer.lock": ["json", "Lockfile"],
  "package-lock.json": ["json", "Lockfile"],
  "yarn.lock": ["", "Lockfile"],
  "gemfile.lock": ["", "Lockfile"],
  "pnpm-lock.yaml": ["yaml", "Lockfile"],
  "go.mod": ["shell", "Go module"],
  "go.sum": ["", "Go checksums"],
  gemfile: ["ruby", "Gemfile"],
  rakefile: ["ruby", "Rakefile"],
  dockerfile: ["shell", "Dockerfile"],
  "docker-compose.yml": ["yaml", "Compose"],
  justfile: ["shell", "Justfile"],
  brewfile: ["ruby", "Brewfile"],
  procfile: ["shell", "Procfile"],
};

// Extension-less names that are plain prose, not source.
const PROSE_NAMES = new Set(["license", "licence", "notice", "authors", "contributors", "copying", "changelog", "readme", "todo"]);

const IMG: Record<string, [string, string]> = {
  png: ["image/png", "PNG"],
  jpg: ["image/jpeg", "JPEG"], jpeg: ["image/jpeg", "JPEG"],
  gif: ["image/gif", "GIF"],
  webp: ["image/webp", "WebP"],
  avif: ["image/avif", "AVIF"],
  bmp: ["image/bmp", "BMP"],
  ico: ["image/x-icon", "Icon"],
};

// Extensions we know are NOT text, so the viewer can say what it is instead of
// falling back to "binary file" after a wasted read.
const BINARY: Record<string, string> = {
  pdf: "PDF", zip: "Zip", gz: "Gzip", tgz: "Tarball", tar: "Tarball", bz2: "Bzip2", xz: "xz", zst: "zstd",
  mp4: "Video", mov: "Video", webm: "Video", avi: "Video", mkv: "Video",
  mp3: "Audio", wav: "Audio", flac: "Audio", aac: "Audio", ogg: "Audio",
  woff: "Font", woff2: "Font", ttf: "Font", otf: "Font",
  so: "Library", dylib: "Library", dll: "Library", a: "Archive", o: "Object",
  wasm: "WASM", bin: "Binary", exe: "Executable", db: "Database", sqlite: "Database",
};

export const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/** Lowercased extension without the dot ("" when the name has none). */
export function ext(path: string): string {
  const n = basename(path);
  const i = n.lastIndexOf(".");
  return i > 0 ? n.slice(i + 1).toLowerCase() : "";
}

export function fileInfo(path: string): FileInfo {
  const name = basename(path).toLowerCase();
  const e = ext(path);

  // `lang: ""` — markdown SOURCE has no grammar; prose run through a code
  // highlighter comes out worse than plain (every capitalized word colours).
  if (e === "md" || e === "markdown" || e === "mdx")
    return { kind: "markdown", lang: "", mime: "", label: "Markdown" };

  if (PROSE_NAMES.has(name)) return { kind: "text", lang: "", mime: "", label: "Text" };

  // SVG is both an image and readable source — treat it as an image (the
  // viewer offers a Source tab, which is where the markup belongs).
  if (e === "svg") return { kind: "image", lang: "xml", mime: "image/svg+xml", label: "SVG" };

  const img = IMG[e];
  if (img) return { kind: "image", lang: "", mime: img[0], label: img[1] };

  const bin = BINARY[e] ?? (name === ".ds_store" ? "Finder metadata" : undefined);
  if (bin) return { kind: "binary", lang: "", mime: "", label: bin };

  const byName = BY_NAME[name];
  if (byName) return { kind: "code", lang: byName[0], mime: "", label: byName[1] };

  // `.env.local`, `Dockerfile.dev`, `tsconfig.build.json`: fall back through
  // the segments so a suffixed variant keeps its base type.
  const code = CODE[e] ?? CODE[name.replace(/^\./, "")] ?? segmentFallback(name);
  if (code) return { kind: "code", lang: code[0], mime: "", label: code[1] };

  if (e === "txt" || e === "log" || e === "" || e === "text")
    return { kind: "text", lang: "", mime: "", label: e === "log" ? "Log" : "Text" };

  // Unknown extension: assume text and let the backend's NUL sniff correct us.
  return { kind: "text", lang: "", mime: "", label: e.toUpperCase() || "File" };
}

/** `.env.local` → the `.env` rule; `Dockerfile.dev` → the Dockerfile rule. */
function segmentFallback(name: string): [string, string] | undefined {
  const parts = name.replace(/^\./, "").split(".");
  for (let i = parts.length - 1; i > 0; i -= 1) {
    const head = parts.slice(0, i).join(".");
    const hit = BY_NAME[head] ?? CODE[head];
    if (hit) return hit;
  }
  return undefined;
}

/** Human byte size for the viewer header. Rounds UP through each threshold so
 *  1048575 reads "1.0 MB", not "1024 KB". */
export function humanSize(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1024) return `${Math.round(n)} B`;
  const kb = n / 1024;
  if (kb < 1024 && Number((kb).toFixed(kb < 10 ? 1 : 0)) < 1024)
    return `${kb.toFixed(kb < 10 ? 1 : 0)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
