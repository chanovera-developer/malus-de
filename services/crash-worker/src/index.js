import { EmailMessage } from "cloudflare:email";

const MAX_BODY_BYTES = 256 * 1024;
const RATE_WINDOW_MS = 10 * 60 * 1000;
const MAX_REPORTS_PER_WINDOW = 5;
const ipWindowCounters = new Map();

function timingSafeEqualString(a, b) {
  const enc = new TextEncoder();
  const ab = enc.encode(String(a));
  const bb = enc.encode(String(b));
  if (ab.length !== bb.length) return false;
  let diff = 0;
  for (let i = 0; i < ab.length; i++) {
    diff |= ab[i] ^ bb[i];
  }
  return diff === 0;
}

async function checkRateLimit(request, env) {
  const clientIp =
    request.headers.get("CF-Connecting-IP") ||
    request.headers.get("X-Forwarded-For")?.split(",")[0]?.trim() ||
    "unknown";

  if (env.RATE_LIMITER && typeof env.RATE_LIMITER.limit === "function") {
    const { success } = await env.RATE_LIMITER.limit({ key: clientIp });
    return success;
  }

  const now = Date.now();
  const entry = ipWindowCounters.get(clientIp);
  if (!entry || now - entry.windowStart > RATE_WINDOW_MS) {
    if (ipWindowCounters.size > 1000) {
      ipWindowCounters.clear();
    }
    ipWindowCounters.set(clientIp, { windowStart: now, count: 1 });
    return true;
  }
  if (entry.count >= MAX_REPORTS_PER_WINDOW) {
    return false;
  }
  entry.count += 1;
  return true;
}

function isAuthorized(request, env) {
  const expected = env.CRASH_API_TOKEN;
  if (!expected) return true;
  const headerToken = request.headers.get("X-Malus-Token");
  const auth = request.headers.get("Authorization") || "";
  const bearer = auth.startsWith("Bearer ") ? auth.slice(7).trim() : "";
  const provided = headerToken || bearer;
  return Boolean(provided) && timingSafeEqualString(provided, expected);
}

function sanitizeHeaderValue(value, maxLen = 140) {
  return String(value ?? "")
    .replace(/[\r\n\0]+/g, " ")
    .trim()
    .slice(0, maxLen);
}

function sanitizeFilename(value) {
  const clean = sanitizeHeaderValue(value, 80).replace(/[^A-Za-z0-9._-]/g, "_");
  if (!clean) return "malus-crash-report.txt";
  return clean.endsWith(".txt") ? clean : `${clean}.txt`;
}

function utf8ToBase64(text) {
  const bytes = new TextEncoder().encode(text);
  let binary = "";
  const chunkSize = 0x8000;
  for (let i = 0; i < bytes.length; i += chunkSize) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunkSize));
  }
  return btoa(binary).replace(/.{1,76}/g, "$&\r\n").trimEnd();
}

function encodeRfc2047(text) {
  const clean = sanitizeHeaderValue(text, 160);
  if (/^[\x20-\x7E]*$/.test(clean)) return clean;
  const base64 = utf8ToBase64(clean).replace(/\r\n/g, "");
  return `=?UTF-8?B?${base64}?=`;
}

function buildRawMimeEmail({ from, to, subject, bodyText, attachmentName, attachmentText }) {
  const boundary = `malus_${crypto.randomUUID().replace(/-/g, "")}`;
  const messageId = `<${crypto.randomUUID()}@chano.dev>`;
  const dateHeader = new Date().toUTCString();

  return [
    `From: Malus DE Crash Reporter <${from}>`,
    `To: <${to}>`,
    `Subject: ${encodeRfc2047(subject)}`,
    `Date: ${dateHeader}`,
    `Message-ID: ${messageId}`,
    `MIME-Version: 1.0`,
    `Content-Type: multipart/mixed; boundary="${boundary}"`,
    ``,
    `--${boundary}`,
    `Content-Type: text/plain; charset="UTF-8"`,
    `Content-Transfer-Encoding: base64`,
    ``,
    utf8ToBase64(bodyText),
    ``,
    `--${boundary}`,
    `Content-Type: text/plain; charset="UTF-8"; name="${attachmentName}"`,
    `Content-Disposition: attachment; filename="${attachmentName}"`,
    `Content-Transfer-Encoding: base64`,
    ``,
    utf8ToBase64(attachmentText),
    ``,
    `--${boundary}--`,
    ``,
  ].join("\r\n");
}

function jsonResponse(data, status = 200) {
  return new Response(JSON.stringify(data), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
    },
  });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === "/health") {
      return jsonResponse({ ok: true, service: "malus-de-crash-worker" });
    }

    if (request.method !== "POST" || (url.pathname !== "/report" && url.pathname !== "/")) {
      return jsonResponse({ error: "Method or path not allowed" }, 405);
    }

    if (!isAuthorized(request, env)) {
      return jsonResponse({ error: "Unauthorized" }, 401);
    }

    if (!(await checkRateLimit(request, env))) {
      return jsonResponse({ error: "Too many crash reports; try again later" }, 429);
    }

    const contentLength = Number(request.headers.get("content-length") || "0");
    if (contentLength > MAX_BODY_BYTES) {
      return jsonResponse({ error: "Report payload too large" }, 413);
    }

    let rawBody;
    try {
      rawBody = await request.text();
    } catch {
      return jsonResponse({ error: "Unable to read request body" }, 400);
    }

    if (rawBody.length > MAX_BODY_BYTES) {
      return jsonResponse({ error: "Report payload too large" }, 413);
    }

    let payload;
    try {
      payload = JSON.parse(rawBody);
    } catch {
      return jsonResponse({ error: "Invalid JSON body" }, 400);
    }

    const reportText = typeof payload?.report === "string" ? payload.report.trim() : "";
    if (!reportText) {
      return jsonResponse({ error: "Missing non-empty 'report' field" }, 400);
    }

    const component = sanitizeHeaderValue(payload?.component || "Malus DE", 64);
    const summary = sanitizeHeaderValue(payload?.summary || "Cierre inesperado", 140);
    const filename = sanitizeFilename(payload?.filename || "malus-crash-report.txt");

    const from = sanitizeHeaderValue(env.FROM_EMAIL || "crash-reporter@chano.dev", 120);
    const to = sanitizeHeaderValue(env.TO_EMAIL || "malus-de@chano.dev", 120);
    const subject = `[Malus DE Crash] ${component}: ${summary}`;

    const intro = [
      `Reporte enviado directamente desde malus-de-crash-reporter`,
      `Componente: ${component}`,
      `Resumen:    ${summary}`,
      `Archivo:    ${filename}`,
      ``,
      `---------------- INFORME ----------------`,
      ``,
      reportText,
      ``,
    ].join("\n");

    const rawMime = buildRawMimeEmail({
      from,
      to,
      subject,
      bodyText: intro,
      attachmentName: filename,
      attachmentText: reportText,
    });

    try {
      const message = new EmailMessage(from, to, rawMime);
      await env.SEND_EMAIL.send(message);
      return jsonResponse({ ok: true }, 200);
    } catch (err) {
      console.error("Email delivery failed:", err?.message || err);
      return jsonResponse({ error: "Email delivery failed" }, 502);
    }
  },
};

