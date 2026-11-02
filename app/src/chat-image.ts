// Desktop chat's photo preparation. The host still accepts only its existing
// formats; raw HEIC/HEIF/AVIF bytes never leave this client.

export type PhotoKind = "HEIC" | "HEIF" | "AVIF";
export const INPUT_MAX = 40 * 1024 * 1024;
const MAX_PIXELS = 64_000_000;
const LONG_EDGE = 2048;
const JPEG_QUALITY = 0.85;

/** Match the same ftyp major brands as attach::refused_format. The name and
 * MIME are never evidence that a file is a photo. */
export function photoKind(header: Uint8Array): PhotoKind | null {
  if (header.length < 12 || header[4] !== 0x66 || header[5] !== 0x74 || header[6] !== 0x79 || header[7] !== 0x70) return null;
  const brand = String.fromCharCode(...header.subarray(8, 12));
  if (["heic", "heix", "hevc", "hevx", "heim", "heis", "hevm", "hevs"].includes(brand)) return "HEIC";
  if (brand === "mif1" || brand === "msf1") return "HEIF";
  if (brand === "avif" || brand === "avis") return "AVIF";
  return null;
}

export function photoDecision(kind: PhotoKind | null, size: number, uploadMax: number): "convert" | "upload" | "input-large" | "upload-large" {
  if (kind) return size > INPUT_MAX ? "input-large" : "convert";
  return size > uploadMax ? "upload-large" : "upload";
}

export function jpegName(name: string): string {
  const dot = name.lastIndexOf(".");
  return `${dot > 0 ? name.slice(0, dot) : name}.jpg`;
}

/** WebKit currently adds its own EXIF dimensions and Photoshop APP13 block
 *  even for a canvas JPEG. Strip every application/comment segment before
 *  upload so no source metadata or encoder metadata can survive. The image
 *  tables and scan bytes stay byte-for-byte as encoded. */
export function stripJpegMetadata(bytes: Uint8Array): Uint8Array {
  if (bytes[0] !== 0xff || bytes[1] !== 0xd8) throw new Error("The converted JPEG is invalid.");
  const parts: Uint8Array[] = [bytes.subarray(0, 2)];
  let size = 2;
  let pos = 2;
  while (pos < bytes.length) {
    const start = pos;
    if (bytes[pos++] !== 0xff) throw new Error("The converted JPEG is invalid.");
    while (bytes[pos] === 0xff) pos++;
    const marker = bytes[pos++];
    if (marker === 0xda || marker === 0xd9) {
      parts.push(bytes.subarray(start));
      size += bytes.length - start;
      const output = new Uint8Array(size);
      let offset = 0;
      for (const part of parts) { output.set(part, offset); offset += part.length; }
      return output;
    }
    if (marker === undefined || pos + 2 > bytes.length || marker === 0 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd8)) {
      throw new Error("The converted JPEG is invalid.");
    }
    const length = (bytes[pos] << 8) | bytes[pos + 1];
    const end = pos + length;
    if (length < 2 || end > bytes.length) throw new Error("The converted JPEG is invalid.");
    if (!((marker >= 0xe0 && marker <= 0xef) || marker === 0xfe)) {
      const part = bytes.subarray(start, end);
      parts.push(part);
      size += part.length;
    }
    pos = end;
  }
  throw new Error("The converted JPEG is invalid.");
}

/** Decode with WebKit's ImageIO-backed image path, which applies EXIF
 * orientation before drawing. Then remove the metadata WebKit's encoder adds. */
export async function convertPhoto(file: Blob, uploadMax: number, kind: PhotoKind): Promise<Blob> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    throw new Error(`This Mac's WebView could not read the ${kind} photo. Export it as JPEG or PNG first.`);
  }
  try {
    if (!bitmap.width || !bitmap.height || bitmap.width * bitmap.height > MAX_PIXELS) {
      throw new Error("This photo has too many pixels to attach. Export it as JPEG or PNG first.");
    }
    const scale = Math.min(1, LONG_EDGE / Math.max(bitmap.width, bitmap.height));
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const context = canvas.getContext("2d");
    if (!context) throw new Error("This Mac's WebView could not convert the photo. Export it as JPEG or PNG first.");
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    // 0.85 preserves ordinary photo detail while keeping typical 12–48 MP
    // captures comfortably below the host's 8 MiB cap after downscaling.
    const jpeg = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/jpeg", JPEG_QUALITY));
    if (!jpeg || jpeg.type !== "image/jpeg") throw new Error("This Mac's WebView could not convert the photo. Export it as JPEG or PNG first.");
    const cleaned = stripJpegMetadata(new Uint8Array(await jpeg.arrayBuffer()));
    if (cleaned.length > uploadMax) throw new Error(`The converted JPEG is over the ${(uploadMax / (1024 * 1024)).toFixed(0)} MB attachment limit.`);
    return new Blob([cleaned], { type: "image/jpeg" });
  } finally {
    bitmap.close();
  }
}
