// Screenplays written in Fountain: which line is which (scene heading, action,
// character cue, parenthetical, dialogue, transition), so the editor can lay
// them out like a script and the narrator can read them as a table read.
import { sentences } from "./sentences";
import type { Segment } from "./narrator";

export type LineKind = "blank" | "fm" | "heading" | "action" | "cue" | "paren" | "dialogue" | "transition" | "center" | "right";

const HEADING = /^(?:INT|EXT|EST|INT\.?\/EXT|EXT\.?\/INT|I\/E|E\/I)[.\s/]/i;
const caps = (t: string) => /\p{L}/u.test(t) && t === t.toUpperCase();

/** A screenplay has at least one capitalised scene heading at the start of a line. */
export function isScreenplay(text: string): boolean {
  return /^(?:INT|EXT|EST|I\/E)[./ ]/m.test(text) || /^\.[A-Z][A-Z]/m.test(text);
}

/** The character's name in a cue, without (V.O.), (CONT'D), Fountain's @ and ^. */
export function cueName(line: string): string {
  return line.trim().replace(/^@/, "").replace(/\^$/, "").replace(/\(.*?\)/g, "").trim();
}

function frontmatterEnd(lines: string[]): number {
  if (lines[0]?.trim() !== "---") return 0;
  for (let i = 1; i < Math.min(lines.length, 200); i++) if (lines[i].trim() === "---") return i + 1;
  return 0;
}

/** Alignment markers outside screenplays: `> centered <` and `>> right`. */
export function alignOf(line: string): "center" | "right" | null {
  const t = line.trim();
  if (t.startsWith(">>")) return "right";
  if (t.startsWith(">") && t.endsWith("<") && t.length > 1) return "center";
  return null;
}

/** Classify every line. Outside a screenplay only alignment is recognised. */
export function classify(lines: string[], screenplay: boolean): LineKind[] {
  const out: LineKind[] = [];
  const fm = frontmatterEnd(lines);
  let inSpeech = false;
  for (let i = 0; i < lines.length; i++) {
    const t = lines[i].trim();
    const prevBlank = i === 0 || i === fm || !lines[i - 1].trim();
    const nextText = i + 1 < lines.length && !!lines[i + 1].trim();
    if (i < fm) {
      out.push("fm");
      continue;
    }
    if (!t) {
      inSpeech = false;
      out.push("blank");
      continue;
    }
    const align = alignOf(t);
    if (!screenplay) {
      out.push(align ?? "action");
      continue;
    }
    if (align === "center") {
      out.push("center");
    } else if (prevBlank && (HEADING.test(t) || /^\.[^.]/.test(t))) {
      inSpeech = false;
      out.push("heading");
    } else if (inSpeech) {
      out.push(t.startsWith("(") ? "paren" : "dialogue");
    } else if (t.startsWith(">") || (caps(t) && /(?:TO:|IN:|OUT\.)$/.test(t) && prevBlank)) {
      out.push("transition");
    } else if (prevBlank && nextText && (t.startsWith("@") || (caps(t) && cueName(t).length > 0 && cueName(t).split(/\s+/).length <= 4 && !/[.!?,]$/.test(cueName(t))))) {
      inSpeech = true;
      out.push("cue");
    } else {
      out.push("action");
    }
  }
  return out;
}

/** Characters who speak, most lines first. */
export function characters(text: string): { name: string; lines: number }[] {
  const lines = text.split("\n");
  const kinds = classify(lines, true);
  const count = new Map<string, number>();
  let who = "";
  kinds.forEach((k, i) => {
    if (k === "cue") who = cueName(lines[i]);
    else if (k === "dialogue" && who) count.set(who, (count.get(who) ?? 0) + 1);
  });
  return [...count].map(([name, lines]) => ({ name, lines })).sort((a, b) => b.lines - a.lines);
}

/** "INT. HAB - DAY" → "Interior. Hab. Day." Spoken headings, not spelled-out capitals. */
function spokenHeading(t: string): string {
  const words = t
    .replace(/^\./, "")
    .replace(/^INT\.?\s*\/\s*EXT\.?/i, "Interior, exterior.")
    .replace(/^EXT\.?\/INT\.?/i, "Exterior, interior.")
    .replace(/^I\/E\.?/i, "Interior, exterior.")
    .replace(/^INT\.?/i, "Interior.")
    .replace(/^EXT\.?/i, "Exterior.")
    .replace(/^EST\.?/i, "Establishing.")
    .split(/\s+-\s+/)
    .map((p) => p.trim().toLowerCase().replace(/(^|[\s.,])(\p{L})/gu, (_m, a: string, b: string) => a + b.toUpperCase()))
    .join(". ");
  return /[.!?]$/.test(words) ? words : `${words}.`;
}

const title = (w: string) => (/\p{Lu}{2,}/u.test(w) ? w[0] + w.slice(1).toLowerCase() : w);

/**
 * Shouted introductions read as words: "WIDE ON A RUSTED TRUCK" →
 * "Wide On A Rusted Truck". A run of two or more capitalised words is
 * calmed; a lone acronym (NASA, EVA, JPL) is left alone unless it's long.
 */
export function unshout(s: string): string {
  return s
    .replace(/\b(?:[\p{Lu}\d'’-]*\p{Lu}[\p{Lu}\d'’-]*)(?:\s+[\p{Lu}\d'’-]*\p{Lu}[\p{Lu}\d'’-]*)+\b/gu, (run) => run.split(/(\s+)/).map(title).join(""))
    .replace(/\b\p{Lu}{5,}\b/gu, title);
}

/**
 * What the script itself says about a character: "he" or "she" in the action
 * lines that mention them. Only a clear majority counts; names are never used.
 */
export function pronounHints(text: string, names: string[]): Record<string, "he" | "she"> {
  const lines = text.split("\n");
  const kinds = classify(lines, true);
  const paras: string[] = [];
  let cur = "";
  kinds.forEach((k, i) => {
    if (k === "action") cur += " " + lines[i];
    else if (cur) {
      paras.push(cur);
      cur = "";
    }
  });
  if (cur) paras.push(cur);
  const out: Record<string, "he" | "she"> = {};
  for (const name of names) {
    const first = name.split(/\s+/)[0];
    if (!first || first.length < 2) continue;
    const re = new RegExp(`\\b${first.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\b`, "i");
    let he = 0;
    let she = 0;
    for (const p of paras) {
      if (!re.test(p)) continue;
      // Only the sentence that names them and the one after: a paragraph can hold several people.
      const sents = p.split(/(?<=[.!?])\s+/);
      sents.forEach((sent, i) => {
        if (!re.test(sent)) return;
        const near = sent + " " + (sents[i + 1] ?? "");
        he += (near.match(/\b(?:he|him|his|himself)\b/gi) ?? []).length;
        she += (near.match(/\b(?:she|her|hers|herself)\b/gi) ?? []).length;
      });
    }
    if (he >= 2 && he >= 2 * she) out[name] = "he";
    else if (she >= 2 && she >= 2 * he) out[name] = "she";
  }
  return out;
}

export interface Voices {
  narrator: string;
  cast: Record<string, string>;
  fallback: string;
}

/**
 * A table read: headings and action in the narrator's voice, each character's
 * lines in theirs. Parentheticals and transitions are skipped.
 */
export function tableRead(doc: string, from: number, to: number, v: Voices): Segment[] {
  const lines = doc.split("\n");
  const kinds = classify(lines, true);
  const starts: number[] = [];
  let at = 0;
  for (const l of lines) {
    starts.push(at);
    at += l.length + 1;
  }
  const end = (i: number) => starts[i] + lines[i].length;
  const out: Segment[] = [];
  const add = (a: number, b: number, voice: string, label: string, pauseAfter: number, transform?: (s: string) => string) => {
    if (b <= from || a >= to) return;
    const found: Segment[] = sentences(doc.slice(a, b)).map((s) => ({
      ...s,
      from: s.from + a,
      to: s.to + a,
      speak: transform ? transform(s.speak) : s.speak,
      italic: false,
      paraEnd: false,
      highlight: true,
      voice,
      label,
    }));
    if (found.length) found[found.length - 1].pauseAfter = pauseAfter;
    out.push(...found);
  };
  let who = "";
  let i = 0;
  while (i < lines.length) {
    const k = kinds[i];
    if (k === "heading") {
      if (end(i) > from && starts[i] < to) {
        out.push({ from: starts[i], to: end(i), speak: spokenHeading(lines[i].trim()), italic: false, paraEnd: false, highlight: true, voice: v.narrator, label: "Scene", pauseAfter: 0.5 });
      }
      i++;
    } else if (k === "action" || k === "center") {
      let j = i;
      while (j + 1 < lines.length && kinds[j + 1] === k) j++;
      add(starts[i], end(j), v.narrator, "Narrator", 0.35, unshout);
      i = j + 1;
    } else if (k === "cue") {
      who = cueName(lines[i]);
      i++;
    } else if (k === "dialogue") {
      let j = i;
      while (j + 1 < lines.length && kinds[j + 1] === "dialogue") j++;
      add(starts[i], end(j), v.cast[who] ?? v.fallback, who, 0.25);
      i = j + 1;
    } else {
      i++;
    }
  }
  return out;
}

/**
 * Give every speaking character a voice nobody else near them has, most
 * talkative first. Alternates voice families so neighbours sound different.
 */
export function autoCast(
  names: string[],
  voices: string[],
  avoid: string[],
  keep: Record<string, string> = {},
  hints: Record<string, "he" | "she"> = {},
): Record<string, string> {
  const english = voices.filter((v) => /^[ab][fm]_/.test(v) && v !== "am_santa" && !avoid.includes(v));
  const families = ["am_", "af_", "bm_", "bf_"].map((p) => english.filter((v) => v.startsWith(p)));
  const pool: string[] = [];
  for (let k = 0; families.some((f) => k < f.length); k++) for (const f of families) if (k < f.length) pool.push(f[k]);
  const cast: Record<string, string> = {};
  const used = new Set(Object.values(keep));
  let next = 0;
  for (const n of names) {
    if (keep[n]) {
      cast[n] = keep[n];
      continue;
    }
    const want = hints[n] === "he" ? /^[ab]m_/ : hints[n] === "she" ? /^[ab]f_/ : null;
    const free = pool.filter((p) => !used.has(p));
    const fitting = want ? free.filter((p) => want.test(p)) : free;
    const choices = fitting.length ? fitting : free.length ? free : pool;
    const pick = choices[want ? 0 : next++ % Math.max(1, choices.length)];
    if (!pick) break;
    cast[n] = pick;
    used.add(pick);
  }
  return cast;
}

/** Tab cycles a line's kind; the text is reshaped to match. */
const CYCLE: LineKind[] = ["action", "cue", "paren", "transition", "heading"];

export function cycleLine(text: string, kind: LineKind, dir = 1): { text: string; kind: LineKind } {
  const k = CYCLE.includes(kind) ? kind : kind === "dialogue" ? "paren" : "action";
  const next = CYCLE[(CYCLE.indexOf(k) + dir + CYCLE.length) % CYCLE.length];
  let base = text.trim().replace(/^[.@>]\s*/, "").replace(/^\((.*)\)$/, "$1").replace(/^(?:INT|EXT)\.\s*/i, "");
  if (!base) base = "";
  switch (next) {
    case "cue":
      return { text: base.toUpperCase(), kind: next };
    case "paren":
      return { text: `(${base.toLowerCase()})`, kind: next };
    case "transition":
      return { text: `> ${base.toUpperCase()}`, kind: next };
    case "heading":
      return { text: `INT. ${base.toUpperCase()}`, kind: next };
    default:
      return { text: base.charAt(0).toUpperCase() + base.slice(1).toLowerCase(), kind: "action" };
  }
}
