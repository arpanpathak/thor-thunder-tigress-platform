// Files attached to a message. Images go to the model as images; PDFs and text
// files go as text, and a scanned PDF also as pictures of its pages.
//
// A file's contents live in this browser's IndexedDB, because localStorage is
// far too small for images. The conversation keeps only a short description of
// each file and, for pictures, a thumbnail. Nothing is uploaded except with the
// message that uses it.
import { element } from "./dom.js";

const PDF_LIBRARY = "./pdf-6.4.299.min.mjs";
const PDF_WORKER = "pdf.worker-6.4.299.min.mjs";

const MAX_FILE_BYTES = 25 << 20;
// About 40k tokens; past that the model can't hold the file and the question.
const MAX_TEXT = 160000;
// Long edge of an image sent to the model: enough to read a screenshot, small
// enough that a vision encoder turns it into a few thousand tokens.
const IMAGE_EDGE = 1536;
const THUMB_EDGE = 160;
// A PDF with less text than this per page is a scan: its pages also go as
// pictures. A scan has next to none; a short real page has a sentence.
const SCANNED_CHARS_PER_PAGE = 16;
const SCANNED_PAGES = 8;
const PDF_TEXT_PAGES = 400;

// ---- storage ----
const memory = new Map();
let database;

function openDatabase() {
  database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open("thor.chat.files", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("files");
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });

  return database;
}

async function transact(mode, work) {
  const db = await openDatabase();

  return new Promise((resolve, reject) => {
    const transaction = db.transaction("files", mode);
    const request = work(transaction.objectStore("files"));
    transaction.oncomplete = () => resolve(request?.result);
    transaction.onerror = () => reject(transaction.error);
    transaction.onabort = () => reject(transaction.error);
  });
}

// The copy in memory serves this tab even when IndexedDB is blocked (private
// windows in some browsers); IndexedDB is what survives a reload.
export const stored = {
  async put(id, payload) {
    memory.set(id, payload);
    try { await transact("readwrite", (files) => files.put(payload, id)); return true; } catch { return false; }
  },

  async get(id) {
    if (memory.has(id)) return memory.get(id);
    try { return (await transact("readonly", (files) => files.get(id))) ?? null; } catch { return null; }
  },

  async remove(ids) {
    ids.forEach((id) => memory.delete(id));
    try { await transact("readwrite", (files) => { ids.forEach((id) => files.delete(id)); }); } catch { /* nothing to free */ }
  },
};

// ---- reading files ----
function canvasOf(width, height) {
  const canvas = element("canvas");
  canvas.width = Math.max(1, Math.round(width));
  canvas.height = Math.max(1, Math.round(height));

  return canvas;
}

// Draws `source` no larger than `edge` on its long side, on white, so a
// transparent picture does not turn black as a JPEG.
function drawn(source, width, height, edge) {
  const scale = Math.min(1, edge / Math.max(width, height));
  const canvas = canvasOf(width * scale, height * scale);
  const context = canvas.getContext("2d");
  context.fillStyle = "#fff";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.drawImage(source, 0, 0, canvas.width, canvas.height);

  return canvas;
}

function thumbnail(source, width, height) {
  return drawn(source, width, height, THUMB_EDGE).toDataURL("image/jpeg", 0.72);
}

async function readImage(file) {
  let bitmap;

  try {
    bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  } catch {
    throw new Error(`${file.name}: this browser can't open that image format. Try a PNG or JPEG.`);
  }

  const canvas = drawn(bitmap, bitmap.width, bitmap.height, IMAGE_EDGE);
  // Screenshots and diagrams stay sharp as PNG unless that makes them large.
  const png = /png|gif|webp|bmp/.test(file.type) ? canvas.toDataURL("image/png") : "";
  const image = png && png.length < 1.5e6 ? png : canvas.toDataURL("image/jpeg", 0.9);
  const meta = { kind: "image", width: canvas.width, height: canvas.height, thumb: thumbnail(bitmap, bitmap.width, bitmap.height) };
  bitmap.close();

  return { meta, payload: { image } };
}

let pdfLibrary;

function pdfjs() {
  pdfLibrary ??= import(PDF_LIBRARY).then((library) => {
    library.GlobalWorkerOptions.workerSrc = PDF_WORKER;
    return library;
  }).catch((error) => { pdfLibrary = undefined; throw error; });

  return pdfLibrary;
}

async function renderPage(page, edge) {
  const base = page.getViewport({ scale: 1 });
  const viewport = page.getViewport({ scale: edge / Math.max(base.width, base.height) });
  const canvas = canvasOf(viewport.width, viewport.height);
  await page.render({ canvas, viewport }).promise;

  return canvas;
}

async function readPdf(file) {
  let library;

  try { library = await pdfjs(); } catch { throw new Error("The PDF reader did not load. Check the connection and try again."); }

  const task = library.getDocument({ data: new Uint8Array(await file.arrayBuffer()), enableXfa: false });
  let pdf;

  try {
    pdf = await task.promise;
  } catch (error) {
    task.destroy();
    throw new Error(error?.name === "PasswordException" ? `${file.name} is password-protected.` : `${file.name} could not be read as a PDF.`);
  }

  try {
    const pages = pdf.numPages;
    const reading = Math.min(pages, PDF_TEXT_PAGES);
    let text = "";
    let truncated = pages > reading;

    for (let number = 1; number <= reading; number += 1) {
      const page = await pdf.getPage(number);
      const content = await page.getTextContent();
      const pageText = content.items.map((item) => (item.str ?? "") + (item.hasEOL ? "\n" : "")).join("").trim();
      text += `${text ? "\n\n" : ""}--- page ${number} ---\n${pageText}`;

      if (text.length > MAX_TEXT) { text = text.slice(0, MAX_TEXT); truncated = true; break; }
    }

    const words = text.replace(/--- page \d+ ---/g, "").replace(/\s/g, "").length;
    const scanned = words < SCANNED_CHARS_PER_PAGE * reading;
    const images = [];

    if (scanned) {
      for (let number = 1; number <= Math.min(pages, SCANNED_PAGES); number += 1) {
        images.push((await renderPage(await pdf.getPage(number), IMAGE_EDGE)).toDataURL("image/jpeg", 0.85));
      }
    }

    const cover = await renderPage(await pdf.getPage(1), THUMB_EDGE * 2);
    const meta = {
      kind: "pdf", pages, scanned, truncated, chars: words ? text.length : 0,
      thumb: thumbnail(cover, cover.width, cover.height),
    };

    return { meta, payload: { text: words ? text : "", images } };
  } finally {
    task.destroy();
  }
}

async function readText(file) {
  const bytes = new Uint8Array(await file.arrayBuffer());

  if (bytes.subarray(0, 8192).includes(0)) {
    throw new Error(`${file.name} isn't an image, a PDF or a text file, so the model can't read it.`);
  }

  let text = new TextDecoder().decode(bytes);
  const truncated = text.length > MAX_TEXT;
  if (truncated) text = text.slice(0, MAX_TEXT);

  return { meta: { kind: "text", chars: text.length, truncated }, payload: { text } };
}

function isPdf(file) {
  return file.type === "application/pdf" || /\.pdf$/i.test(file.name);
}

// SVG is text the model can read; other image types are pictures.
function isImage(file) {
  return file.type.startsWith("image/") && file.type !== "image/svg+xml";
}

/** Reads a picked, dropped or pasted file into `{ meta, payload }`, or throws an Error to show. */
export async function readFile(file) {
  if (file.size > MAX_FILE_BYTES) throw new Error(`${file.name} is larger than 25 MB.`);

  const read = isImage(file) ? await readImage(file) : isPdf(file) ? await readPdf(file) : await readText(file);
  const name = file.name || (read.meta.kind === "image" ? "pasted image" : "pasted file");

  return { meta: { id: crypto.randomUUID(), name, size: file.size, ...read.meta }, payload: read.payload };
}

// ---- describing files ----
function amount(bytes) {
  return bytes < 1024 ? `${bytes} B` : bytes < 1 << 20 ? `${Math.round(bytes / 1024)} KB` : `${(bytes / (1 << 20)).toFixed(1)} MB`;
}

function tokens(chars) {
  const count = Math.ceil(chars / 4);

  return count < 1000 ? `≈ ${count} tokens` : `≈ ${Math.round(count / 1000)}k tokens`;
}

/** One line about a file: its size, pages and how much of the model's context it takes. */
export function describe(meta) {
  const parts = [];

  if (meta.kind === "image") parts.push(`${meta.width}×${meta.height}`);
  if (meta.kind === "pdf") parts.push(`${meta.pages} page${meta.pages === 1 ? "" : "s"}`);
  if (meta.kind === "pdf" && meta.scanned) parts.push(`scanned: first ${Math.min(meta.pages, SCANNED_PAGES)} as pictures`);
  if (meta.chars) parts.push(tokens(meta.chars));
  if (meta.truncated) parts.push("cut to fit");
  if (!parts.length) parts.push(amount(meta.size));

  return parts.join(" · ");
}

/** Whether a message carries pictures: images, or the pages of a scanned PDF. */
export function hasPictures(attachments = []) {
  return attachments.some((meta) => meta.kind === "image" || (meta.kind === "pdf" && meta.scanned));
}

// ---- the message the model reads ----
function fileName(name) {
  return name.replace(/["<>\n]/g, "_");
}

/**
 * The `content` of a user message for the API: a string when it has no files,
 * else image parts followed by one text part holding the files' text and the
 * message. A model that can't see gets a note in place of each picture.
 */
export async function apiContent(message, canSee) {
  const attachments = message.attachments || [];
  if (!attachments.length) return message.content;

  const pictures = [];
  const texts = [];

  for (const meta of attachments) {
    const payload = await stored.get(meta.id);
    const name = fileName(meta.name);

    if (!payload) { texts.push(`[${name} was attached here, but it is no longer stored in this browser]`); continue; }

    const images = meta.kind === "image" ? [payload.image] : payload.images || [];

    if (images.length && canSee) images.forEach((url) => pictures.push({ type: "image_url", image_url: { url } }));
    else if (images.length) texts.push(`[${name}: ${meta.kind === "image" ? "an image" : "a scanned PDF"} the current model can't see]`);

    if (payload.text) {
      const pages = meta.kind === "pdf" ? ` pages="${meta.pages}"` : "";
      const cut = meta.truncated ? "\n[the rest of the file was cut to fit]" : "";
      texts.push(`<file name="${name}"${pages}>\n${payload.text}${cut}\n</file>`);
    }
  }

  const text = [...texts, message.content].filter(Boolean).join("\n\n");

  return [...pictures, { type: "text", text: text || "(the attached files)" }];
}
