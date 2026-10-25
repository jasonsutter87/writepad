// Signs contact messages on their way from the contact function to Netlify
// Forms, so submission-created can tell them apart from anything posted to
// the form endpoint directly (which skips the rate limits).

import { createHash, createHmac, timingSafeEqual } from "node:crypto";

/** How long a signature stays valid: enough for Netlify to process the post. */
const MAX_AGE_MS = 15 * 60 * 1000;

const normalize = (message: string) => message.replace(/\r\n?/g, "\n").trim();

function mac(secret: string, time: string, message: string): string {
  const digest = createHash("sha256").update(normalize(message)).digest("hex");
  return createHmac("sha256", secret).update(`${time}\n${digest}`).digest("hex");
}

/** "<time>.<hmac>" over the time and the message text. */
export function sign(secret: string, message: string, now = Date.now()): string {
  const time = String(now);
  return `${time}.${mac(secret, time, message)}`;
}

export function verify(secret: string, relay: string | undefined, message: string, now = Date.now()): boolean {
  const [time, given] = (relay ?? "").split(".");
  if (!time || !given || !(now - Number(time) < MAX_AGE_MS)) return false;
  const want = Buffer.from(mac(secret, time, message));
  const got = Buffer.from(given);
  return got.length === want.length && timingSafeEqual(got, want);
}
