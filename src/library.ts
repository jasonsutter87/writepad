// The Library view: measured style of reference books, the target blend, and
// the open manuscript, side by side. Numbers in a table; two charts for shape.
import type { Adaptation, LibraryReport, ScriptSummary, StyleSummary } from "./api";

const SVG = "http://www.w3.org/2000/svg";
/** Series colors, fixed order (validated categorical palette). More books than this go gray. */
// Orange (--s2) is left out: it's too close to the accent, which always means "you".
export const SERIES = ["--s1", "--s3", "--s6", "--s5", "--s4"];
export const seriesVar = (i: number) => (i < SERIES.length ? `var(${SERIES[i]})` : "var(--s-other)");

function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls = "", text = ""): HTMLElementTagNameMap[K] {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text) n.textContent = text;
  return n;
}
function s<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const n = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) n.setAttribute(k, String(v));
  return n;
}

const int = (n: number) => Math.round(n).toLocaleString();
const pct = (n: number) => `${Math.round(n * 100)}%`;
const one = (n: number) => (Math.round(n * 10) / 10).toFixed(1);

interface Series {
  key: string;
  label: string;
  sub: string;
  color: string;
  dash?: boolean;
  bold?: boolean;
  data: StyleSummary;
}

/** Books (in library order, so each keeps its color), then target, then the manuscript. */
function seriesOf(r: LibraryReport): Series[] {
  const out: Series[] = r.books.map((b, i) => ({ key: b.id, label: b.title, sub: b.author, color: seriesVar(i), data: b }));
  if (r.target) out.push({ key: "target", label: "Target", sub: r.target.author, color: "var(--ink)", dash: true, data: r.target });
  if (r.manuscript) out.push({ key: "mine", label: r.manuscript.title, sub: "your manuscript", color: "var(--accent)", bold: true, data: r.manuscript });
  return out;
}

const ROWS: { label: string; hint: string; value: (d: StyleSummary) => string }[] = [
  { label: "Words", hint: "Total, front matter skipped", value: (d) => int(d.words) },
  { label: "Chapters", hint: "Sections for a manuscript", value: (d) => int(d.chapters) },
  { label: "Chapter length", hint: "Median words (middle half)", value: (d) => `${int(d.chapter_words.median)} (${int(d.chapter_words.p25)}–${int(d.chapter_words.p75)})` },
  { label: "Sentence length", hint: "Median words · 90th percentile", value: (d) => `${int(d.sentence_words.median)} · ${int(d.sentence_words.p90)}` },
  { label: "Sentence variety", hint: "Spread of lengths (std ÷ mean); higher is more varied", value: (d) => d.sentence_spread.toFixed(2) },
  { label: "Paragraph length", hint: "Median words · 90th percentile", value: (d) => (d.paragraphs_known ? `${int(d.paragraph_words.median)} · ${int(d.paragraph_words.p90)}` : "— (scan, no paragraph breaks)") },
  { label: "Dialogue", hint: "Share of words inside quotes", value: (d) => pct(d.dialogue) },
  { label: "One-sentence paragraphs", hint: "Share of paragraphs", value: (d) => (d.paragraphs_known ? pct(d.one_sentence_paras) : "—") },
  { label: "-ly adverbs", hint: "Per 1,000 words", value: (d) => one(d.adverbs_per_1k) },
  { label: "Repeated openers", hint: "Sentences opening like the two before, per 1,000", value: (d) => one(d.repeated_openers_per_1k) },
  { label: "Readability", hint: "Flesch reading ease; higher is easier", value: (d) => int(d.flesch) },
  { label: "Chapters end on", hint: "Dialogue · a question · a short line", value: (d) => `${pct(d.endings.dialogue)} · ${pct(d.endings.question)} · ${pct(d.endings.short)}` },
];

function table(series: Series[]): HTMLElement {
  const wrap = h("div", "lib-table-wrap");
  const t = h("table", "lib-table");
  const head = h("tr");
  head.append(h("th"));
  for (const sr of series) {
    const th = h("th");
    const name = h("div", "lib-col");
    const dot = h("span", "lib-dot");
    dot.style.background = sr.color;
    if (sr.dash) dot.classList.add("dash");
    name.append(dot, document.createTextNode(sr.label));
    th.append(name, h("div", "lib-sub", sr.sub));
    if (sr.key === "mine") th.classList.add("mine");
    head.append(th);
  }
  t.append(head);
  for (const row of ROWS) {
    const tr = h("tr");
    const th = h("th", "lib-row", row.label);
    th.title = row.hint;
    th.append(h("div", "lib-sub", row.hint));
    tr.append(th);
    for (const sr of series) {
      const td = h("td", sr.key === "mine" ? "mine" : sr.key === "target" ? "target" : "", row.value(sr.data));
      tr.append(td);
    }
    t.append(tr);
  }
  wrap.append(t);
  return wrap;
}

function legend(series: Series[]): HTMLElement {
  const box = h("div", "lib-legend");
  for (const sr of series) {
    const item = h("span", "lib-legend-item");
    const line = s("svg", { width: 22, height: 10 });
    line.append(s("line", { x1: 1, x2: 21, y1: 5, y2: 5, stroke: sr.color, "stroke-width": sr.bold ? 3 : 2, "stroke-dasharray": sr.dash ? "4 3" : "none", "stroke-linecap": "round" }));
    item.append(line, document.createTextNode(sr.label));
    box.append(item);
  }
  return box;
}

/** Share of sentences at each length: one line per series, crosshair tooltip. */
function sentenceChart(series: Series[]): HTMLElement {
  const box = h("section", "lib-chart");
  box.append(h("h3", "", "Sentence lengths"), h("p", "hint", "Share of sentences at each length. A long right tail means long sentences are common."), legend(series));
  const W = 720, H = 260, L = 44, R = 12, T = 12, B = 34;
  const N = 60;
  const peak = Math.max(0.01, ...series.flatMap((sr) => sr.data.sentence_curve));
  const step = peak > 0.06 ? 0.02 : 0.01;
  const maxY = Math.ceil(peak / step) * step;
  const x = (i: number) => L + ((W - L - R) * i) / (N - 1);
  const y = (v: number) => T + (H - T - B) * (1 - v / maxY);
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, class: "lib-svg", role: "img", "aria-label": "Sentence length distribution" });
  // Recessive grid and axes.
  for (let v = 0; v <= maxY + 1e-9; v += step) {
    svg.append(s("line", { x1: L, x2: W - R, y1: y(v), y2: y(v), class: "grid" }));
    const t = s("text", { x: L - 6, y: y(v) + 4, class: "axis", "text-anchor": "end" });
    t.textContent = `${Math.round(v * 100)}%`;
    svg.append(t);
  }
  for (const n of [1, 10, 20, 30, 40, 50, 60]) {
    const t = s("text", { x: x(n - 1), y: H - B + 16, class: "axis", "text-anchor": "middle" });
    t.textContent = n === 60 ? "60+" : String(n);
    svg.append(t);
  }
  const xl = s("text", { x: (L + W - R) / 2, y: H - 4, class: "axis", "text-anchor": "middle" });
  xl.textContent = "words per sentence";
  svg.append(xl);
  for (const sr of series) {
    const d = sr.data.sentence_curve.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`).join("");
    svg.append(s("path", { d, fill: "none", stroke: sr.color, "stroke-width": sr.bold ? 3 : 2, "stroke-dasharray": sr.dash ? "5 4" : "none", "stroke-linejoin": "round", "stroke-linecap": "round" }));
  }
  const cross = s("line", { y1: T, y2: H - B, class: "cross", visibility: "hidden" });
  svg.append(cross);
  const hit = s("rect", { x: L, y: T, width: W - L - R, height: H - T - B, fill: "transparent" });
  svg.append(hit);
  const tip = h("div", "graph-tip lib-tip");
  tip.hidden = true;
  const plot = h("div", "lib-plot");
  plot.append(svg, tip);
  hit.addEventListener("mousemove", (e) => {
    const r = svg.getBoundingClientRect();
    const px = ((e.clientX - r.left) / r.width) * W;
    const i = Math.max(0, Math.min(N - 1, Math.round(((px - L) / (W - L - R)) * (N - 1))));
    cross.setAttribute("x1", String(x(i)));
    cross.setAttribute("x2", String(x(i)));
    cross.setAttribute("visibility", "visible");
    tip.replaceChildren(h("div", "tip-head", i === N - 1 ? "60+ words" : `${i + 1} word${i ? "s" : ""}`));
    for (const sr of series) {
      const row = h("div", "lib-tip-row");
      const dot = h("span", "lib-dot");
      dot.style.background = sr.color;
      row.append(dot, h("span", "", sr.label), h("b", "", `${(sr.data.sentence_curve[i] * 100).toFixed(1)}%`));
      tip.append(row);
    }
    tip.hidden = false;
    const left = ((x(i) / W) * r.width) + 14;
    tip.style.left = `${left > r.width - 230 ? left - 250 : left}px`;
    tip.style.top = "10px";
  });
  hit.addEventListener("mouseleave", () => {
    tip.hidden = true;
    cross.setAttribute("visibility", "hidden");
  });
  box.append(plot);
  return box;
}

/** Every chapter as a dot, one row per book, with the target's middle half shaded. */
function chapterChart(series: Series[], target: StyleSummary | null): HTMLElement {
  const box = h("section", "lib-chart");
  box.append(h("h3", "", "Chapter lengths"), h("p", "hint", target ? "One dot per chapter. The band is the middle half of your target's chapters; the line is its median." : "One dot per chapter."));
  const rows = series.filter((sr) => sr.data.chapter_list.length);
  const W = 720, L = 150, R = 16, T = 8, rowH = 30;
  const H = T + rows.length * rowH + 30;
  const max = Math.max(1000, ...rows.flatMap((sr) => sr.data.chapter_list));
  const x = (v: number) => L + ((W - L - R) * v) / max;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, class: "lib-svg", role: "img", "aria-label": "Chapter lengths" });
  const tickStep = max > 8000 ? 2000 : max > 4000 ? 1000 : 500;
  for (let v = 0; v <= max; v += tickStep) {
    svg.append(s("line", { x1: x(v), x2: x(v), y1: T, y2: H - 26, class: "grid" }));
    const t = s("text", { x: x(v), y: H - 10, class: "axis", "text-anchor": "middle" });
    t.textContent = v >= 1000 ? `${v / 1000}k` : String(v);
    svg.append(t);
  }
  if (target) {
    const q = target.chapter_words;
    svg.append(s("rect", { x: x(q.p25), y: T, width: Math.max(1, x(q.p75) - x(q.p25)), height: rows.length * rowH, class: "band" }));
    svg.append(s("line", { x1: x(q.median), x2: x(q.median), y1: T, y2: T + rows.length * rowH, class: "band-median" }));
  }
  const tip = h("div", "graph-tip lib-tip");
  tip.hidden = true;
  const plot = h("div", "lib-plot");
  rows.forEach((sr, r) => {
    const cy = T + r * rowH + rowH / 2;
    const label = s("text", { x: L - 10, y: cy + 4, class: "row-label", "text-anchor": "end" });
    label.textContent = sr.label.length > 22 ? sr.label.slice(0, 21) + "…" : sr.label;
    svg.append(label);
    sr.data.chapter_list.forEach((w, i) => {
      const dot = s("circle", { cx: x(w), cy, r: 4.5, fill: sr.color, class: "chap-dot" });
      const hitDot = s("circle", { cx: x(w), cy, r: 9, fill: "transparent" });
      hitDot.addEventListener("mouseenter", () => {
        tip.replaceChildren(h("div", "tip-head", `${sr.label}, ${sr.key === "mine" ? "section" : "chapter"} ${i + 1}`), h("div", "tip-meta", `${int(w)} words`));
        tip.hidden = false;
        const rect = svg.getBoundingClientRect();
        tip.style.left = `${Math.min(rect.width - 200, (x(w) / W) * rect.width + 12)}px`;
        tip.style.top = `${(cy / H) * rect.height + 10}px`;
        dot.setAttribute("r", "6.5");
      });
      hitDot.addEventListener("mouseleave", () => {
        tip.hidden = true;
        dot.setAttribute("r", "4.5");
      });
      svg.append(dot, hitDot);
    });
  });
  plot.append(svg, tip);
  box.append(plot);
  return box;
}

interface Insight {
  label: string;
  mine: number;
  target: number;
  fmt: (n: number) => string;
  status: "on" | "over" | "under";
  gap: number;
  verdict: string;
  books: { label: string; value: number; color: string }[];
}

/**
 * The manuscript against the target, one measure at a time, worst gap first.
 * Plain counting and fixed tolerances; the words describe, they don't judge.
 */
function insights(r: LibraryReport): Insight[] {
  const m = r.manuscript!;
  const t = r.target!;
  const weighted = r.books.map((b, i) => ({ b, color: seriesVar(i) })).filter(({ b }) => (r.weights[b.id] ?? 0) > 0);
  const out: Insight[] = [];
  const add = (
    label: string,
    f: (d: StyleSummary) => number,
    fmt: (n: number) => string,
    tol: number,
    say: { over: string; under: string; on: string },
    known = (d: StyleSummary) => d === d,
  ) => {
    if (!known(m) || !known(t)) return;
    const a = f(m);
    const b = f(t);
    const status = Math.abs(a - b) <= tol ? "on" : a > b ? "over" : "under";
    out.push({
      label,
      mine: a,
      target: b,
      fmt,
      status,
      gap: status === "on" ? 0 : Math.abs(a - b) / Math.max(Math.abs(b), tol, 1e-9),
      verdict: say[status],
      books: weighted.filter(({ b }) => known(b)).map(({ b, color }) => ({ label: b.title, value: f(b), color })),
    });
  };
  const paras = (d: StyleSummary) => d.paragraphs_known;
  add("Typical sentence", (d) => d.sentence_words.median, (n) => `${int(n)} words`, 1, {
    over: "Your typical sentence runs longer than the target's.",
    under: "Your typical sentence is shorter than the target's.",
    on: "Your typical sentence length matches the target.",
  });
  add("Long sentences", (d) => d.sentence_words.p90, (n) => `${int(n)} words`, 2, {
    over: "Your longest sentences (the top 10%) run longer than the target's.",
    under: "Your longest sentences (the top 10%) stop shorter than the target's.",
    on: "Your longest sentences are about as long as the target's.",
  });
  add("Sentence variety", (d) => d.sentence_spread, (n) => n.toFixed(2), 0.05, {
    over: "Your sentence lengths swing more than the target's.",
    under: "Your sentence lengths are more even. The target mixes very short lines with long ones more often.",
    on: "Your mix of short and long sentences matches the target.",
  });
  add("Dialogue", (d) => d.dialogue, pct, 0.03, {
    over: "More of your text is speech than the target's.",
    under: "Less of your text is speech than the target's.",
    on: "Your share of dialogue matches the target.",
  });
  add("One-sentence paragraphs", (d) => d.one_sentence_paras, pct, 0.03, {
    over: "You break out single lines more often than the target.",
    under: "You give single lines their own paragraph less often than the target.",
    on: "You set single lines apart about as often as the target.",
  }, paras);
  add("Paragraph length", (d) => d.paragraph_words.median, (n) => `${int(n)} words`, 5, {
    over: "Your paragraphs run longer than the target's.",
    under: "Your paragraphs are shorter than the target's.",
    on: "Your paragraph length matches the target.",
  }, paras);
  add("Repeated openers", (d) => d.repeated_openers_per_1k, (n) => `${one(n)} per 1,000`, 2, {
    over: "More of your sentences open with the same word as the two before. The editor underlines these.",
    under: "You repeat sentence openings less than the target.",
    on: "You repeat sentence openings about as often as the target.",
  });
  add("-ly adverbs", (d) => d.adverbs_per_1k, (n) => `${one(n)} per 1,000`, 1.5, {
    over: "You use more -ly adverbs than the target.",
    under: "You use fewer -ly adverbs than the target.",
    on: "You use -ly adverbs about as often as the target.",
  });
  add("Readability", (d) => d.flesch, (n) => int(n), 3, {
    over: "Your prose reads easier than the target's (shorter words and sentences).",
    under: "Your prose reads harder than the target's (longer words and sentences).",
    on: "Your prose is about as easy to read as the target's.",
  });
  add("Chapters ending on a short line", (d) => d.endings.short, pct, 0.05, {
    over: "You end on a short, punchy line more often than the target.",
    under: "The target ends on a short, punchy line more often than you do.",
    on: "You end on a short line about as often as the target.",
  });
  add("Chapters ending on dialogue", (d) => d.endings.dialogue, pct, 0.05, {
    over: "You end on dialogue more often than the target.",
    under: "The target ends on dialogue more often than you do.",
    on: "You end on dialogue about as often as the target.",
  });
  return out.sort((a, b) => b.gap - a.gap);
}

/** A one-line gauge: books as small ticks, the target as a tall line, you as a dot. */
function gauge(i: Insight): SVGSVGElement {
  const W = 240, H = 26, L = 6, R = 6, cy = 14;
  const vals = [i.mine, i.target, ...i.books.map((b) => b.value)];
  const lo = Math.min(0, ...vals);
  const hi = Math.max(...vals) * 1.15 || 1;
  const x = (v: number) => L + ((W - L - R) * (v - lo)) / (hi - lo);
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, class: "gauge", role: "img", "aria-label": `${i.label}: you ${i.fmt(i.mine)}, target ${i.fmt(i.target)}` });
  svg.append(s("line", { x1: L, x2: W - R, y1: cy, y2: cy, class: "track" }));
  for (const b of i.books) {
    const tick = s("line", { x1: x(b.value), x2: x(b.value), y1: cy - 5, y2: cy + 5, stroke: b.color, "stroke-width": 3, "stroke-linecap": "round" });
    const t = s("title");
    t.textContent = `${b.label}: ${i.fmt(b.value)}`;
    tick.append(t);
    svg.append(tick);
  }
  svg.append(s("line", { x1: x(i.target), x2: x(i.target), y1: cy - 10, y2: cy + 10, class: "target-tick" }));
  svg.append(s("circle", { cx: x(i.mine), cy, r: 6, class: "mine-dot" }));
  return svg;
}

function insightCards(r: LibraryReport): HTMLElement {
  const list = insights(r);
  const box = h("section", "insights");
  const off = list.filter((i) => i.status !== "on");
  const head = h("div", "insights-head");
  head.append(h("h3", "", "Against your target"));
  head.append(
    h(
      "p",
      "hint",
      `${r.manuscript!.title}: on target in ${list.length - off.length} of ${list.length} measures.` +
        (off.length ? ` Biggest gaps: ${off.slice(0, 3).map((i) => i.label.toLowerCase()).join(", ")}.` : "") +
        " Updates as you write.",
    ),
  );
  const key = s("svg", { width: 300, height: 20, class: "gauge-key", "aria-hidden": "true" });
  key.append(s("circle", { cx: 8, cy: 10, r: 5, class: "mine-dot" }));
  const k1 = s("text", { x: 18, y: 14 });
  k1.textContent = "you";
  key.append(k1, s("line", { x1: 60, x2: 60, y1: 2, y2: 18, class: "target-tick" }));
  const k2 = s("text", { x: 68, y: 14 });
  k2.textContent = "target";
  key.append(k2, s("line", { x1: 124, x2: 124, y1: 6, y2: 14, stroke: "var(--muted)", "stroke-width": 3, "stroke-linecap": "round" }));
  const k3 = s("text", { x: 132, y: 14 });
  k3.textContent = "each reference book (its color)";
  key.append(k3);
  head.append(key);
  box.append(head);
  const grid = h("div", "insight-grid");
  for (const i of list) {
    const card = h("div", `insight ${i.status}`);
    const top = h("div", "insight-top");
    top.append(h("span", "insight-label", i.label), h("span", "insight-status", i.status === "on" ? "On target" : i.status === "over" ? "Above target" : "Below target"));
    const nums = h("div", "insight-nums");
    nums.append(h("b", "", i.fmt(i.mine)), h("span", "", ` you · target ${i.fmt(i.target)}`));
    card.append(top, nums, gauge(i), h("p", "insight-verdict", i.verdict));
    if (i.books.length > 1) card.append(h("p", "insight-books", i.books.map((b) => `${b.label} ${i.fmt(b.value)}`).join(" · ")));
    grid.append(card);
  }
  box.append(grid);
  return box;
}

// ---------------------------------------------------------------- scripts

/** Script lengths the way they're spoken of: in eighths of a page, then pages. */
const pages = (n: number) => {
  if (n >= 1) return `${one(n)} pages`;
  const e = Math.max(1, Math.round(n * 8));
  if (e === 8) return "1 page";
  const g = e % 4 === 0 ? 4 : e % 2 === 0 ? 2 : 1;
  return `${e / g}/${8 / g} page`;
};

const SCRIPT_ROWS: { label: string; hint: string; value: (d: ScriptSummary) => string }[] = [
  { label: "Length", hint: "Pages at ~55 lines a page (about a minute each)", value: (d) => `${int(d.pages)} pages` },
  { label: "Scenes", hint: "Scene headings (INT./EXT.)", value: (d) => int(d.scenes) },
  { label: "Scene length", hint: "Median (middle half)", value: (d) => `${pages(d.scene_pages.median)} (${pages(d.scene_pages.p25)}–${pages(d.scene_pages.p75)})` },
  { label: "Dialogue", hint: "Share of words that are spoken", value: (d) => pct(d.dialogue) },
  { label: "Speech length", hint: "Median words · 90th percentile", value: (d) => `${int(d.speech_words.median)} · ${int(d.speech_words.p90)}` },
  { label: "Action blocks", hint: "Median lines · 90th percentile", value: (d) => `${int(d.action_lines.median)} · ${int(d.action_lines.p90)}` },
  { label: "Speeches per page", hint: "How fast the talk volleys", value: (d) => one(d.speeches_per_page) },
  { label: "Interiors · nights", hint: "Share of scenes", value: (d) => `${pct(d.interior)} · ${pct(d.night)}` },
  { label: "Locations", hint: "Distinct places · scenes that move somewhere new", value: (d) => `${int(d.locations)} · ${pct(d.location_changes)}` },
  { label: "Lead's share", hint: "Of all dialogue", value: (d) => (d.characters[0] ? `${d.characters[0].name} ${pct(d.characters[0].share)}` : "—") },
];

function scriptTable(scripts: ScriptSummary[]): HTMLElement {
  const wrap = h("div", "lib-table-wrap");
  const t = h("table", "lib-table");
  const head = h("tr");
  head.append(h("th"));
  scripts.forEach((sc, i) => {
    const th = h("th");
    const name = h("div", "lib-col");
    const dot = h("span", "lib-dot");
    dot.style.background = seriesVar(i);
    name.append(dot, document.createTextNode(sc.title));
    th.append(name, h("div", "lib-sub", sc.writer || "Unknown writer"));
    head.append(th);
  });
  t.append(head);
  for (const row of SCRIPT_ROWS) {
    const tr = h("tr");
    const th = h("th", "lib-row", row.label);
    th.append(h("div", "lib-sub", row.hint));
    tr.append(th, ...scripts.map((sc) => h("td", "", row.value(sc))));
    t.append(tr);
  }
  wrap.append(t);
  return wrap;
}

/** Each script as a strip of scenes in order, width by length; the dark part of each is its dialogue share. */
function pacingChart(scripts: ScriptSummary[]): HTMLElement {
  const box = h("section", "lib-chart");
  box.append(h("h3", "", "Pacing"), h("p", "hint", "Every scene in order, as wide as it is long. The solid part of each is how much of it is dialogue; the pale part is action."));
  const W = 820, L = 150, R = 8, rowH = 40, T = 4;
  const H = T + scripts.length * rowH + 26;
  const max = Math.max(...scripts.map((s) => s.pages), 1);
  const x = (p: number) => L + ((W - L - R) * p) / max;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, class: "lib-svg", role: "img", "aria-label": "Scene pacing" });
  for (let p = 0; p <= max; p += 20) {
    svg.append(s("line", { x1: x(p), x2: x(p), y1: T, y2: H - 22, class: "grid" }));
    const t = s("text", { x: x(p), y: H - 8, class: "axis", "text-anchor": "middle" });
    t.textContent = `p.${p}`;
    svg.append(t);
  }
  const tip = h("div", "graph-tip lib-tip");
  tip.hidden = true;
  scripts.forEach((sc, r) => {
    const y = T + r * rowH + 6;
    const bh = rowH - 14;
    const label = s("text", { x: L - 10, y: y + bh / 2 + 4, class: "row-label", "text-anchor": "end" });
    label.textContent = sc.title.length > 22 ? sc.title.slice(0, 21) + "…" : sc.title;
    svg.append(label);
    for (const p of sc.pacing) {
      const x0 = x(p.start);
      const w = Math.max(0.6, x(p.start + p.pages) - x0 - 0.6);
      const g = s("g", { class: "pace" });
      g.append(s("rect", { x: x0, y, width: w, height: bh, fill: seriesVar(r), opacity: 0.28 }));
      g.append(s("rect", { x: x0, y: y + bh * (1 - p.dialogue), width: w, height: bh * p.dialogue, fill: seriesVar(r) }));
      g.addEventListener("mouseenter", () => {
        tip.replaceChildren(h("div", "tip-head", p.heading), h("div", "tip-meta", `p.${Math.floor(p.start) + 1} · ${pages(p.pages)} · ${pct(p.dialogue)} dialogue`));
        tip.hidden = false;
        const rect = svg.getBoundingClientRect();
        tip.style.left = `${Math.min(rect.width - 240, (x0 / W) * rect.width)}px`;
        tip.style.top = `${((y + bh + 6) / H) * rect.height}px`;
      });
      g.addEventListener("mouseleave", () => (tip.hidden = true));
      svg.append(g);
    }
  });
  const plot = h("div", "lib-plot");
  plot.append(svg, tip);
  box.append(plot);
  return box;
}

/** Who talks: top characters' share of the dialogue, one small chart per script. */
function castChart(scripts: ScriptSummary[]): HTMLElement {
  const box = h("section", "lib-chart");
  box.append(h("h3", "", "Who talks"), h("p", "hint", "Share of all dialogue, and the page where each character first speaks."));
  const grid = h("div", "cast-grid");
  scripts.forEach((sc, i) => {
    const col = h("div", "cast-col");
    col.append(h("div", "cast-title", sc.title));
    const top = Math.max(...sc.characters.map((c) => c.share), 0.01);
    for (const c of sc.characters) {
      const row = h("div", "cast-row");
      const bar = h("span", "cast-bar");
      bar.style.width = `${(c.share / top) * 100}%`;
      bar.style.background = seriesVar(i);
      const track = h("span", "cast-track");
      track.append(bar);
      row.append(h("span", "cast-name", c.name), track, h("span", "cast-val", `${pct(c.share)} · p.${Math.floor(c.first_page)}`));
      col.append(row);
    }
    grid.append(col);
  });
  box.append(grid);
  return box;
}

// ---------------------------------------------------------------- adaptation

/** Book chapters on top, film scenes below, a ribbon from each scene to the chapter it draws on. */
function ribbonChart(a: Adaptation): HTMLElement {
  const W = 820, H = 230, L = 8, R = 8, top = 34, chH = 22, scY = 170, scH = 22;
  const gap = 1.5;
  const svg = s("svg", { viewBox: `0 0 ${W} ${H}`, class: "lib-svg ribbon", role: "img", "aria-label": "Book chapters to film scenes" });
  const span = W - L - R;
  const totalWords = a.chapters.reduce((t, c) => t + c.words, 0) || 1;
  const totalPages = a.scenes.reduce((t, c) => t + c.pages, 0) || 1;
  const chX: number[] = [];
  let at = L;
  for (const c of a.chapters) {
    chX.push(at);
    at += (span * c.words) / totalWords;
  }
  const chW = (i: number) => (span * a.chapters[i].words) / totalWords - gap;
  const scX: number[] = [];
  at = L;
  for (const sc of a.scenes) {
    scX.push(at);
    at += (span * sc.pages) / totalPages;
  }
  const scW = (i: number) => Math.max(0.5, (span * a.scenes[i].pages) / totalPages - 0.5);
  const tip = h("div", "graph-tip lib-tip");
  tip.hidden = true;
  const ribbons: SVGPathElement[][] = a.chapters.map(() => []);
  const ribbonLayer = s("g");
  // Within a chapter, its scenes take slices in film order.
  a.chapters.forEach((c, ci) => {
    const total = c.scenes.reduce((t, k) => t + a.scenes[k].pages, 0) || 1;
    let off = 0;
    for (const k of c.scenes) {
      const w = (chW(ci) * a.scenes[k].pages) / total;
      const x1 = chX[ci] + off, x2 = x1 + w;
      off += w;
      const x3 = scX[k], x4 = scX[k] + scW(k);
      const y1 = top + chH, y2 = scY, m = (y1 + y2) / 2;
      const d = `M${x1},${y1} C${x1},${m} ${x3},${m} ${x3},${y2} L${x4},${y2} C${x4},${m} ${x2},${m} ${x2},${y1} Z`;
      const p = s("path", { d, class: a.scenes[k].bridged ? "rib bridged" : "rib" });
      ribbons[ci].push(p);
      ribbonLayer.append(p);
    }
  });
  svg.append(ribbonLayer);
  const label = (x: number, y: number, text: string, cls = "axis") => {
    const t = s("text", { x, y, class: cls });
    t.textContent = text;
    svg.append(t);
  };
  label(L, top - 12, `${a.book}: ${int(a.book_words)} words, by chapter`);
  label(L, scY + scH + 18, `${a.script}: ${int(a.script_pages)} pages, by scene`);
  const show = (html: Node[], x: number, y: number) => {
    tip.replaceChildren(...html);
    tip.hidden = false;
    const rect = svg.getBoundingClientRect();
    tip.style.left = `${Math.min(rect.width - 250, (x / W) * rect.width)}px`;
    tip.style.top = `${(y / H) * rect.height}px`;
  };
  a.chapters.forEach((c, ci) => {
    const cut = !c.scenes.length;
    const r = s("rect", { x: chX[ci], y: top, width: Math.max(0.5, chW(ci)), height: chH, class: cut ? "ch cut" : "ch" });
    if (chW(ci) > 16) {
      const t = s("text", { x: chX[ci] + chW(ci) / 2, y: top + 15, class: "ch-num", "text-anchor": "middle" });
      t.textContent = String(ci + 1);
      svg.append(r, t);
    } else svg.append(r);
    const hit = s("rect", { x: chX[ci], y: top - 4, width: Math.max(4, chW(ci)), height: chH + 8, fill: "transparent" });
    hit.addEventListener("mouseenter", () => {
      svg.classList.add("focus");
      ribbons[ci].forEach((p) => p.classList.add("on"));
      show([h("div", "tip-head", c.title), h("div", "tip-meta", cut ? `${int(c.words)} words · cut from the film` : `${int(c.words)} words → ${c.scenes.length} scene${c.scenes.length === 1 ? "" : "s"}, ${pages(c.pages)}`)], chX[ci], top + chH + 8);
    });
    hit.addEventListener("mouseleave", () => {
      svg.classList.remove("focus");
      ribbons[ci].forEach((p) => p.classList.remove("on"));
      tip.hidden = true;
    });
    svg.append(hit);
  });
  a.scenes.forEach((sc, k) => {
    const r = s("rect", { x: scX[k], y: scY, width: scW(k), height: scH, class: sc.chapter === null ? "sc film-only" : "sc" });
    r.addEventListener("mouseenter", () => {
      const from = sc.chapter === null ? "No clear source in the book" : `From ${a.chapters[sc.chapter].title}${sc.bridged ? " (between scenes from it)" : ""}`;
      show([h("div", "tip-head", sc.heading), h("div", "tip-meta", `${pages(sc.pages)} · ${from}${sc.verbatim > 0 ? ` · ${pct(sc.verbatim)} of its dialogue word for word` : ""}`)], scX[k], scY - 60);
    });
    r.addEventListener("mouseleave", () => (tip.hidden = true));
    svg.append(r);
  });
  const plot = h("div", "lib-plot");
  plot.append(svg, tip);
  return plot;
}

function adaptationView(a: Adaptation): HTMLElement {
  const box = h("div", "adapt");
  const cut = a.chapters.filter((c) => !c.scenes.length);
  const cutWords = cut.reduce((t, c) => t + c.words, 0);
  const sentences = [
    `${int(a.book_words)} words became ${int(a.script_pages)} pages: about ${int(a.book_words / Math.max(1, a.script_pages))} words of book per page of script.`,
    cut.length ? `${cut.length} chapter${cut.length === 1 ? " was" : "s were"} cut entirely (${cut.map((c) => c.title.replace(/^Chapter /, "")).join(", ")}; ${int(cutWords)} words).` : "Every chapter made it into the film in some form.",
    `${a.invented_scenes} short scenes (${pages(a.invented_pages)}) have no clear source in the book: transitions, establishing shots, or new material.`,
    `${pct(a.verbatim)} of the film's dialogue is word for word from the book.`,
  ];
  box.append(h("p", "adapt-summary", sentences.join(" ")));
  box.append(ribbonChart(a));
  const legend = h("p", "hint", "Hover a chapter to see where it went, or a scene to see where it came from. Faint ribbons are short scenes placed by the scenes around them; gray scenes have no clear source.");
  box.append(legend);
  // Per-chapter compression.
  const wrap = h("div", "lib-table-wrap");
  const t = h("table", "lib-table compact");
  const hr = h("tr");
  for (const c of ["Chapter", "Words", "Scenes", "Script pages", "Words per page"]) hr.append(h("th", "", c));
  t.append(hr);
  for (const c of a.chapters) {
    const tr = h("tr", c.scenes.length ? "" : "cut");
    tr.append(h("td", "", c.title), h("td", "", int(c.words)), h("td", "", c.scenes.length ? int(c.scenes.length) : "cut"), h("td", "", c.scenes.length ? one(c.pages) : "—"), h("td", "", c.pages > 0 ? int(c.words / c.pages) : "—"));
    t.append(tr);
  }
  wrap.append(t);
  const cols = h("div", "adapt-cols");
  const left = h("div");
  left.append(h("h3", "", "Chapter by chapter"), wrap);
  const right = h("div");
  right.append(h("h3", "", "Lines that survived word for word"));
  const ul = h("ul", "kept");
  for (const k of a.kept) {
    const li = h("li");
    li.append(h("b", "", k.speaker), document.createTextNode(` ${k.text.length > 220 ? k.text.slice(0, 219) + "…" : k.text}`));
    if (k.chapter !== null) li.append(h("span", "lib-sub", ` · ${a.chapters[k.chapter].title}`));
    ul.append(li);
  }
  if (!a.kept.length) ul.append(h("li", "hint", "None found."));
  right.append(ul);
  cols.append(left, right);
  box.append(cols);
  return box;
}

function adaptationSection(r: LibraryReport, scripts: ScriptSummary[], load: (book: string, script: string) => Promise<Adaptation>): HTMLElement {
  const box = h("section", "lib-chart wide");
  box.append(h("h3", "", "Book to script"), h("p", "hint", "Lines a book up against its film: which chapters each scene draws on, what was cut, and which lines survived. Reads both from their sources; nothing extra is stored."));
  const row = h("div", "row adapt-pick");
  const bookSel = h("select");
  for (const b of r.books) bookSel.append(new Option(b.title, b.id));
  const scriptSel = h("select");
  for (const sc of scripts) scriptSel.append(new Option(sc.title, sc.id));
  // Pair them up by title when we can.
  const pair = scripts.find((sc) => r.books.some((b) => b.title.toLowerCase() === sc.title.toLowerCase()));
  if (pair) {
    scriptSel.value = pair.id;
    bookSel.value = r.books.find((b) => b.title.toLowerCase() === pair.title.toLowerCase())!.id;
  }
  row.append(bookSel, h("span", "hint", "→"), scriptSel);
  const out = h("div");
  const run = () => {
    out.replaceChildren(h("p", "hint", "Reading both and lining them up…"));
    load(bookSel.value, scriptSel.value)
      .then((a) => out.replaceChildren(adaptationView(a)))
      .catch((e) => out.replaceChildren(h("p", "hint", String(e))));
  };
  bookSel.onchange = run;
  scriptSel.onchange = run;
  box.append(row, out);
  if (pair) run();
  else out.append(h("p", "hint", "Pick a book and its script."));
  return box;
}

export function renderLibrary(
  root: HTMLElement,
  r: LibraryReport,
  scripts: ScriptSummary[],
  actions: { add: () => void; adapt: (book: string, script: string) => Promise<Adaptation> },
) {
  root.replaceChildren();
  const head = h("div", "timeline-head");
  head.append(h("h2", "", "Library"));
  if (!r.books.length && scripts.length) {
    root.append(head, h("h3", "lib-section", "Scripts"), scriptTable(scripts), pacingChart(scripts), castChart(scripts));
    return;
  }
  if (!r.books.length) {
    head.append(h("p", "hint", "Add EPUBs of books whose style you want to learn from. WRITEpad measures them (chapter and sentence lengths, dialogue, how chapters end) and keeps only the numbers, never the text."));
    const b = h("button", "primary", "Add books…");
    b.onclick = actions.add;
    head.append(b);
    root.append(head);
    return;
  }
  head.append(
    h("p", "hint", r.target ? `Target: ${r.target.author}. Weights are set in the sidebar.` : "Give books a weight in the sidebar to blend them into a target. Your scenes are then measured against it as you write."),
  );
  root.append(head);
  const series = seriesOf(r);
  if (r.target && r.manuscript) root.append(insightCards(r));
  root.append(table(series), sentenceChart(series), chapterChart(series, r.target));
  if (!r.manuscript) root.append(h("p", "hint", "Open a manuscript to see it measured here too."));
  if (scripts.length) {
    root.append(h("h2", "lib-section", "Scripts"), scriptTable(scripts), pacingChart(scripts), castChart(scripts));
    root.append(adaptationSection(r, scripts, actions.adapt));
  }
}
