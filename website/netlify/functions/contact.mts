// The contact form posts here, not straight to Netlify Forms. This checks the
// message, enforces the limits below, then forwards it to the "contact" form
// with a signature (see ../shared/relay.ts).
//
//   - Netlify's own rate limit: 5 posts a minute per IP, answered with a 429.
//   - Daily caps kept in Netlify Blobs: per IP, and for the whole site, so a
//     flood from many addresses still stops at SITE_PER_DAY.
//   - A honeypot field and a minimum time on the page; bots that trip either
//     are told "sent" and dropped.
//
// Works without JavaScript: a normal form post gets a 303 to the thanks page
// or back to /contact/?error=<code>. The page script asks for JSON instead.

import type { Config, Context } from "@netlify/functions";
import { getStore } from "@netlify/blobs";
import { createHash } from "node:crypto";
import { sign } from "../shared/relay.ts";

const PER_IP_PER_DAY = 5;
const SITE_PER_DAY = 100;
const MIN_FILL_MS = 3_000;
const MAX_BODY_BYTES = 20_000;
const MAX_LINKS = 3;
const LIMITS = { name: 100, email: 200, message: 5_000 };
const TOPICS = new Set(["question", "bug", "publish", "other"]);
const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;

type Code = "sent" | "invalid" | "links" | "busy" | "failed";

export default async (req: Request, context: Context) => {
  if (req.method !== "POST") return new Response("Method not allowed", { status: 405, headers: { Allow: "POST" } });

  const json = (req.headers.get("accept") ?? "").includes("application/json");
  const reply = (status: number, code: Code) =>
    json
      ? Response.json({ ok: code === "sent", code }, { status })
      : Response.redirect(new URL(code === "sent" ? "/contact/thanks/" : `/contact/?error=${code}#form`, req.url), 303);

  if (Number(req.headers.get("content-length") ?? 0) > MAX_BODY_BYTES) return reply(413, "invalid");

  let form: FormData;
  try {
    form = await req.formData();
  } catch {
    return reply(400, "invalid");
  }
  const field = (k: string) => String(form.get(k) ?? "").trim();
  const name = field("name");
  const email = field("email");
  const topic = field("topic") || "other";
  const message = field("message");

  // Bots: fill every field, or post faster than a person can type. Look sent, drop it.
  const started = Number(field("started"));
  if (field("company") || (started && Date.now() - started < MIN_FILL_MS)) return reply(200, "sent");

  if (!email || !message || !EMAIL.test(email) || !TOPICS.has(topic)) return reply(422, "invalid");
  if (name.length > LIMITS.name || email.length > LIMITS.email || message.length > LIMITS.message) return reply(422, "invalid");
  if ((message.match(/https?:\/\//gi) ?? []).length > MAX_LINKS) return reply(422, "links");

  const secret = Netlify.env.get("FORM_RELAY_SECRET");
  if (!secret) {
    console.error("FORM_RELAY_SECRET is not set; contact messages can't be forwarded.");
    return reply(500, "failed");
  }

  // Daily caps. Blobs aren't atomic, so a burst can overshoot by a few; the
  // per-minute rate limit above keeps that small.
  const store = getStore("contact-limits");
  const day = new Date().toISOString().slice(0, 10);
  const who = createHash("sha256").update(`${secret}:${context.ip}`).digest("hex").slice(0, 32);
  const ipKey = `${day}/ip/${who}`;
  const siteKey = `${day}/site`;
  const [ipCount, siteCount] = await Promise.all([ipKey, siteKey].map(async (k) => Number((await store.get(k)) ?? 0)));
  if (ipCount >= PER_IP_PER_DAY || siteCount >= SITE_PER_DAY) return reply(429, "busy");
  await Promise.all([store.set(ipKey, String(ipCount + 1)), store.set(siteKey, String(siteCount + 1))]);

  const body = new URLSearchParams({ "form-name": "contact", name, email, topic, message, relay: sign(secret, message) });
  const site = Netlify.env.get("URL") ?? new URL(req.url).origin;
  const res = await fetch(new URL("/", site), {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body,
  });
  if (!res.ok) {
    console.error(`Netlify Forms answered ${res.status}`);
    return reply(502, "failed");
  }
  return reply(200, "sent");
};

export const config: Config = {
  path: "/api/contact",
  method: "POST",
  rateLimit: {
    windowLimit: 5,
    windowSize: 60,
    aggregateBy: ["ip", "domain"],
  },
};
