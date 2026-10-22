// Split markdown prose into speakable sentences, keeping their document offsets
// so the editor can highlight each one as it's read.

export interface Sentence {
  from: number;
  to: number;
  /** What gets sent to the voice: markdown stripped, dashes turned into pauses. */
  speak: string;
  /** Wholly inside *italics*: thoughts, or the link talking. */
  italic: boolean;
  /** Last sentence of its paragraph (gets a longer pause after). */
  paraEnd: boolean;
}

const SENTENCE = /[^.!?…]+(?:[.!?…]+["'”’)\]*_]*|$)\s*/g;
const HAS_WORDS = /[\p{L}\p{N}]/u;
// A period here doesn't end the sentence: "Mr. Blackkeys", "J. R. R.", "e.g. this".
const ABBREVIATION = /(?:^|\s)(?:Mr|Mrs|Ms|Mx|Dr|Prof|St|Sr|Jr|vs|etc|e\.g|i\.e|[A-Z])\.\s*$/;

export function speakable(text: string): string {
  return text
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^\s*>>?\s*|\s*<\s*$/gm, "")
    .replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_m, target: string, alias?: string) => alias ?? target)
    .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
    .replace(/\*\*|__|\*|`/g, "")
    .replace(/\s*[—–]\s*/g, ", ")
    .replace(/…/g, "...")
    .replace(/\s+/g, " ")
    .trim();
}

/** Which characters of a paragraph sit inside single-asterisk italics. */
function italicMask(para: string): boolean[] {
  const mask: boolean[] = new Array(para.length).fill(false);
  let on = false;
  for (let i = 0; i < para.length; i++) {
    if (para[i] === "*") {
      if (para[i + 1] === "*") {
        i++; // bold marker, not italics
        continue;
      }
      on = !on;
      continue;
    }
    mask[i] = on;
  }
  return mask;
}

/**
 * Sentences of `doc` from the one containing `from` up to `to`.
 * Rules (`---`), blank lines and other wordless bits are skipped.
 */
export function sentences(doc: string, from = 0, to = doc.length): Sentence[] {
  const out: Sentence[] = [];
  const paraBreak = /\n[ \t]*\n/g;
  let start = 0;
  const paras: [number, number][] = [];
  for (const m of doc.matchAll(paraBreak)) {
    paras.push([start, m.index!]);
    start = m.index! + m[0].length;
  }
  paras.push([start, doc.length]);

  for (const [ps, pe] of paras) {
    if (pe <= from || ps >= to) continue;
    const para = doc.slice(ps, pe);
    const mask = italicMask(para);
    // Raw [start, end) pieces within the paragraph, abbreviations glued back on.
    const pieces: [number, number][] = [];
    let glue = false;
    for (const m of para.matchAll(SENTENCE)) {
      if (!m[0]) continue;
      const end = m.index! + m[0].length;
      if (glue) pieces[pieces.length - 1][1] = end;
      else pieces.push([m.index!, end]);
      glue = ABBREVIATION.test(m[0]);
    }
    const found: Sentence[] = [];
    for (const [ia, ib] of pieces) {
      const raw = para.slice(ia, ib);
      const a = ps + ia;
      const b = a + raw.trimEnd().length;
      if (b <= from || a >= to) continue;
      const speak = speakable(raw);
      if (!HAS_WORDS.test(speak)) continue;
      let italic = true;
      for (let i = ia; i < ib; i++) {
        if (HAS_WORDS.test(para[i]) && !mask[i]) {
          italic = false;
          break;
        }
      }
      found.push({ from: a, to: b, speak, italic, paraEnd: false });
    }
    if (found.length) {
      found[found.length - 1].paraEnd = true;
      out.push(...found);
    }
  }
  return out;
}
