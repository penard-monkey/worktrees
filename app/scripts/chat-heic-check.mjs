// Pure attachment decisions. Keep the browser decoder out of this check.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import ts from "typescript";

const source = readFileSync(resolve(import.meta.dirname, "../src/chat-image.ts"), "utf8");
const js = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
const C = await import(`data:text/javascript,${encodeURIComponent(js)}`);
const header = (brand) => Uint8Array.from([0, 0, 0, 24, 102, 116, 121, 112, ...brand.split("").map((c) => c.charCodeAt(0))]);
let count = 0;
const check = (condition, message) => { if (!condition) throw Error(message); count++; };

for (const brand of ["heic", "heix", "hevc", "hevx", "heim", "heis", "hevm", "hevs"])
  check(C.photoKind(header(brand)) === "HEIC", `${brand} must be HEIC`);
for (const brand of ["mif1", "msf1"]) check(C.photoKind(header(brand)) === "HEIF", `${brand} must be HEIF`);
for (const brand of ["avif", "avis"]) check(C.photoKind(header(brand)) === "AVIF", `${brand} must be AVIF`);
check(C.photoKind(header("mp42")) === null, "MP4 must not be decoded as a photo");
check(C.photoKind(new Uint8Array(12)) === null, "missing ftyp is ordinary bytes");
check(C.photoKind(Uint8Array.from([102, 116, 121, 112, ..."heic".split("").map((c) => c.charCodeAt(0))])) === null, "ftyp offset matters");
const uploadMax = 8 * 1024 * 1024;
check(C.photoDecision("HEIC", C.INPUT_MAX, uploadMax) === "convert", "HEIC input bound is inclusive");
check(C.photoDecision("HEIC", C.INPUT_MAX + 1, uploadMax) === "input-large", "HEIC input bound is enforced");
check(C.photoDecision(null, uploadMax + 1, uploadMax) === "upload-large", "ordinary upload bound remains 8 MiB");
check(C.photoDecision(null, uploadMax, uploadMax) === "upload", "ordinary upload bound is inclusive");
check(C.jpegName("IMG_0001.HEIC") === "IMG_0001.jpg", "extension is rewritten");
check(C.jpegName("pasted image") === "pasted image.jpg", "extensionless name gains jpg");
check(C.jpegName(".heic") === ".heic.jpg", "dotfile name stays visible");
const encoded = Uint8Array.from([
  0xff, 0xd8,
  0xff, 0xe1, 0, 10, 69, 120, 105, 102, 0, 0, 65, 66,
  0xff, 0xed, 0, 6, 73, 80, 84, 67,
  0xff, 0xfe, 0, 5, 65, 66, 67,
  0xff, 0xdb, 0, 4, 0, 0,
  0xff, 0xda, 0, 4, 0, 0, 17, 34, 0xff, 0xd9,
]);
const clean = C.stripJpegMetadata(encoded);
check(!new TextDecoder("latin1").decode(clean).includes("Exif"), "EXIF is removed");
check(!new TextDecoder("latin1").decode(clean).includes("IPTC"), "APP13 is removed");
check(clean[0] === 0xff && clean[1] === 0xd8 && clean.at(-2) === 0xff && clean.at(-1) === 0xd9, "JPEG image bytes survive");
console.log(`chat-heic-check: ${count} decisions pass`);
