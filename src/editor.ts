// The CodeMirror editor plus its two overlays: the onion skin (diff against
// another variant, deleted words drawn as ghosts) and the narration highlight.
import { minimalSetup } from "codemirror";
import { redo, undo } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorSelection, EditorState, Prec, StateEffect, StateField, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers, placeholder, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import type { DiffOp } from "./api";
import { livePreview, type ImageResolver } from "./livepreview";
import { classify, cycleLine, isScreenplay, type LineKind } from "./screenplay";

class GhostWidget extends WidgetType {
  constructor(readonly text: string) {
    super();
  }
  eq(other: GhostWidget) {
    return other.text === this.text;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "wp-ghost";
    span.textContent = this.text;
    return span;
  }
  ignoreEvent() {
    return false;
  }
}

/** One place where two versions differ: `theirs` (the compared version's words) became `mine`. */
export interface Hunk {
  from: number;
  to: number;
  theirs: string;
  mine: string;
  /** Stable enough to remember a "keep mine" across re-diffs: the change plus the words before it. */
  key: string;
}

/** Group a diff's runs of deletes and inserts into hunks, positioned in the new text. */
export function hunks(ops: DiffOp[]): Hunk[] {
  const out: Hunk[] = [];
  let pos = 0;
  let before = "";
  let cur: Hunk | null = null;
  for (const op of ops) {
    if (op.tag === "equal") {
      if (cur) out.push(cur), (cur = null);
      before = op.text.slice(-40);
      pos += op.text.length;
      continue;
    }
    cur ??= { from: pos, to: pos, theirs: "", mine: "", key: "" };
    if (op.tag === "delete") cur.theirs += op.text;
    else (cur.mine += op.text), (pos += op.text.length), (cur.to = pos);
    cur.key = `${before}\u0001${cur.theirs}\u0001${cur.mine}`;
  }
  if (cur) out.push(cur);
  return out;
}

/** A hunk worth a decision: one that changes words, not only spacing. */
export const visible = (h: Hunk) => !!(h.theirs + h.mine).trim();

/** Merge mode: each hunk gets buttons to take the other version's words or keep these. */
export interface Pick {
  kept: Set<string>;
  onKeep: (key: string) => void;
}

/** "Use theirs" / "Keep mine", at the end of a hunk. */
class PickWidget extends WidgetType {
  constructor(readonly hunk: Hunk, readonly pick: Pick) {
    super();
  }
  eq(other: PickWidget) {
    return other.hunk.key === this.hunk.key;
  }
  toDOM(view: EditorView) {
    const box = document.createElement("span");
    box.className = "wp-pick";
    box.contentEditable = "false";
    const { theirs, mine } = this.hunk;
    const button = (label: string, title: string, run: () => void) => {
      const b = document.createElement("button");
      b.type = "button";
      b.textContent = label;
      b.title = title;
      b.onmousedown = (e) => e.preventDefault();
      b.onclick = (e) => {
        e.preventDefault();
        e.stopPropagation();
        run();
      };
      box.append(b);
    };
    const short = (s: string) => {
      const t = s.trim().replace(/\s+/g, " ");
      return t.length > 60 ? `${t.slice(0, 57)}…` : t;
    };
    button("←", theirs.trim() ? `Use “${short(theirs)}”` : `Cut “${short(mine)}”`, () => {
      // The hunk ends where this widget sits; the text before it must still be `mine`.
      const end = view.posAtDOM(box);
      const from = end - mine.length;
      if (from < 0 || view.state.sliceDoc(from, end) !== mine) return;
      view.dispatch({ changes: { from, to: end, insert: theirs }, userEvent: "input.merge" });
    });
    button("✓", mine.trim() ? `Keep “${short(mine)}”` : `Leave “${short(theirs)}” out`, () => this.pick.onKeep(this.hunk.key));
    return box;
  }
  ignoreEvent() {
    return true;
  }
}

const setOnion = StateEffect.define<{ ops: DiffOp[] | null; pick: Pick | null }>();
const insertMark = Decoration.mark({ class: "wp-ins" });

function onionDecorations(ops: DiffOp[] | null, pick: Pick | null, docLength: number): DecorationSet {
  if (!ops) return Decoration.none;
  // A diff computed against older text than what's on screen: drop it, a fresh one is coming.
  const length = ops.reduce((n, o) => n + (o.tag === "delete" ? 0 : o.text.length), 0);
  if (length !== docLength) return Decoration.none;
  const ranges: Range<Decoration>[] = [];
  for (const h of hunks(ops)) {
    if (pick?.kept.has(h.key)) continue;
    if (h.theirs) ranges.push(Decoration.widget({ widget: new GhostWidget(h.theirs), side: -1 }).range(h.from));
    if (h.mine.trim()) ranges.push(insertMark.range(h.from, h.to));
    if (pick && visible(h)) ranges.push(Decoration.widget({ widget: new PickWidget(h, pick), side: 1 }).range(h.to));
  }
  return Decoration.set(ranges, true);
}

const onionField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) if (e.is(setOnion)) deco = onionDecorations(e.value.ops, e.value.pick, tr.state.doc.length);
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** A comment's place in the text; `active` is the one open in the panel. */
export interface CommentRange {
  id: string;
  from: number;
  to: number;
  active?: boolean;
}

const setComments = StateEffect.define<CommentRange[]>();

/** Commented words, highlighted. The marks move with the text as it's edited. */
const commentField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(setComments)) continue;
      const len = tr.state.doc.length;
      const marks = e.value
        .filter((c) => c.from >= 0 && c.from < c.to && c.to <= len)
        .sort((a, b) => a.from - b.from)
        .map((c) => Decoration.mark({ class: c.active ? "wp-comment active" : "wp-comment", attributes: { "data-comment": c.id } }).range(c.from, c.to));
      deco = Decoration.set(marks, true);
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

const setSpeaking = StateEffect.define<{ from: number; to: number } | null>();
const speakingMark = Decoration.mark({ class: "wp-speaking" });

const speakingField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(setSpeaking)) continue;
      const r = e.value;
      deco = r && r.to <= tr.state.doc.length ? Decoration.set([speakingMark.range(r.from, r.to)]) : Decoration.none;
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export interface CadenceFlag {
  from: number;
  to: number;
  kind: string;
  message: string;
}

const setCadence = StateEffect.define<CadenceFlag[] | null>();

/** Quiet underlines where the prose drifts from the target; hover for why. */
const cadenceField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(setCadence)) continue;
      const len = tr.state.doc.length;
      const ranges = (e.value ?? [])
        .filter((f) => f.from < f.to && f.to <= len)
        .map((f) => Decoration.mark({ class: `wp-cad wp-cad-${f.kind}`, attributes: { title: f.message } }).range(f.from, f.to));
      deco = Decoration.set(ranges, true);
    }
    return deco;
  },
  provide: (f) => EditorView.decorations.from(f),
});

/** Mark a leading `---` properties block so it reads as metadata, not a setext heading. */
const fmLine = Decoration.line({ class: "cm-fm" });
function frontmatter(view: EditorView): DecorationSet {
  const doc = view.state.doc;
  if (doc.lines < 2 || doc.line(1).text.trim() !== "---") return Decoration.none;
  for (let n = 2; n <= Math.min(doc.lines, 200); n++) {
    if (doc.line(n).text.trim() === "---") {
      const ranges = [];
      for (let i = 1; i <= n; i++) ranges.push(fmLine.range(doc.line(i).from));
      return Decoration.set(ranges);
    }
  }
  return Decoration.none;
}
const frontmatterPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = frontmatter(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged) this.decorations = frontmatter(u.view);
    }
  },
  { decorations: (p) => p.decorations },
);

/** Screenplay layout (Fountain) and alignment markers, as line classes. */
function layout(view: EditorView): DecorationSet {
  const doc = view.state.doc;
  const text = doc.toString();
  const screenplay = isScreenplay(text);
  if (!screenplay && !/^\s*>/m.test(text)) return Decoration.none;
  const lines = text.split("\n");
  const kinds = classify(lines, screenplay);
  const ranges: Range<Decoration>[] = [];
  // The rendered viewport (a margin beyond what's visible), so scrolling never shows unstyled lines.
  for (const { from, to } of [view.viewport]) {
    for (let n = doc.lineAt(from).number; n <= doc.lineAt(to).number; n++) {
      const k = kinds[n - 1];
      if (k && k !== "blank" && k !== "fm" && (screenplay || k === "center" || k === "right")) ranges.push(lineClass(k).range(doc.line(n).from));
    }
  }
  return Decoration.set(ranges);
}
const lineClasses = new Map<LineKind, Decoration>();
function lineClass(k: LineKind): Decoration {
  let d = lineClasses.get(k);
  if (!d) lineClasses.set(k, (d = Decoration.line({ class: `fx-${k}` })));
  return d;
}
const layoutPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = layout(view);
    }
    update(u: ViewUpdate) {
      if (u.docChanged || u.viewportChanged) this.decorations = layout(u.view);
    }
  },
  { decorations: (p) => p.decorations },
);

// The editor's class, through CodeMirror's own attributes: it owns the class
// list and rewrites it, so toggling view.dom by hand can go stale between documents.
const screenplayField = StateField.define<boolean>({
  create: (s) => isScreenplay(s.doc.toString()),
  update: (v, tr) => (tr.docChanged ? isScreenplay(tr.newDoc.toString()) : v),
});
const screenplayClass = EditorView.editorAttributes.compute([screenplayField], (s) =>
  (s.field(screenplayField) ? { class: "wp-screenplay" } : {}) as Record<string, string>,
);

/** Wrap the selection in a marker (**, *), or unwrap it if it's already wrapped. */
function toggleWrap(view: EditorView, mark: string): boolean {
  const n = mark.length;
  const tr = view.state.changeByRange((r) => {
    const doc = view.state.doc;
    const wrapped = doc.sliceString(r.from - n, r.from) === mark && doc.sliceString(r.to, r.to + n) === mark;
    if (wrapped) {
      return { changes: [{ from: r.from - n, to: r.from }, { from: r.to, to: r.to + n }], range: EditorSelection.range(r.from - n, r.to - n) };
    }
    return { changes: [{ from: r.from, insert: mark }, { from: r.to, insert: mark }], range: EditorSelection.range(r.from + n, r.to + n) };
  });
  view.dispatch(view.state.update(tr, { scrollIntoView: true, userEvent: "input.format" }));
  return true;
}

/** Set the alignment of every selected line: plain, `> centered <` or `>> right`. */
export function setAlign(view: EditorView, align: "left" | "center" | "right"): boolean {
  const doc = view.state.doc;
  const seen = new Set<number>();
  const changes = [];
  for (const r of view.state.selection.ranges) {
    for (let n = doc.lineAt(r.from).number; n <= doc.lineAt(r.to).number; n++) {
      if (seen.has(n)) continue;
      seen.add(n);
      const line = doc.line(n);
      if (!line.text.trim()) continue;
      const bare = line.text.trim().replace(/^>>\s*/, "").replace(/^>\s*(.*?)\s*<$/, "$1");
      const next = align === "center" ? `> ${bare} <` : align === "right" ? `>> ${bare}` : bare;
      if (next !== line.text) changes.push({ from: line.from, to: line.to, insert: next });
    }
  }
  if (changes.length) view.dispatch({ changes, userEvent: "input.format" });
  return true;
}

/** The selected lines, each once, in order. */
function selectedLines(state: EditorState) {
  const seen = new Set<number>();
  const lines = [];
  for (const r of state.selection.ranges) {
    for (let n = state.doc.lineAt(r.from).number; n <= state.doc.lineAt(r.to).number; n++) {
      if (!seen.has(n)) lines.push(state.doc.line(n));
      seen.add(n);
    }
  }
  return lines.sort((a, b) => a.number - b.number);
}

const HEADING = /^(#{1,6})\s+/;

/** The heading level (1–6) of the line the cursor is on, or 0 for body text. */
export function headingLevel(state: EditorState): number {
  const m = HEADING.exec(state.doc.lineAt(state.selection.main.head).text);
  return m ? m[1].length : 0;
}

/** Make the selected lines a heading of this level (0 = body text); the same level again turns it off. */
export function setHeading(view: EditorView, level: number): boolean {
  const lines = selectedLines(view.state);
  if (level && lines.every((l) => HEADING.exec(l.text)?.[1].length === level)) level = 0;
  const changes = lines.map((l) => {
    const bare = l.text.replace(HEADING, "");
    return { from: l.from, to: l.to, insert: level ? `${"#".repeat(level)} ${bare}` : bare };
  });
  view.dispatch({ changes, userEvent: "input.format" });
  return true;
}

export type BlockKind = "bullet" | "number" | "task" | "quote";
const BLOCK_PREFIX = /^(\s*)(?:[-*+]\s+\[[ xX]\]\s+|[-*+]\s+|\d+[.)]\s+|>(?!>)\s?(?!.*<\s*$))?/;
const kindOf = (text: string): BlockKind | null =>
  /^\s*[-*+]\s+\[[ xX]\]\s/.test(text) ? "task" : /^\s*[-*+]\s/.test(text) ? "bullet" : /^\s*\d+[.)]\s/.test(text) ? "number" : /^\s*>(?!>)(?!.*<\s*$)/.test(text) ? "quote" : null;

/** Turn the selected lines into a list or quote, or back to plain lines if they already are one. */
export function toggleBlock(view: EditorView, kind: BlockKind): boolean {
  const lines = selectedLines(view.state).filter((l) => l.text.trim() || view.state.selection.main.empty);
  const off = lines.length > 0 && lines.every((l) => kindOf(l.text) === kind);
  let n = 0;
  const changes = lines.map((l) => {
    const [whole, indent] = BLOCK_PREFIX.exec(l.text)!;
    const prefix = off ? "" : kind === "bullet" ? "- " : kind === "number" ? `${++n}. ` : kind === "task" ? "- [ ] " : "> ";
    return { from: l.from, to: l.from + whole.length, insert: indent + prefix };
  });
  view.dispatch({ changes, userEvent: "input.format" });
  return true;
}

/** Make the selection a link (or start an empty one) and put the cursor where the address goes. */
export function insertLink(view: EditorView): boolean {
  const { from, to } = view.state.selection.main;
  const text = view.state.sliceDoc(from, to);
  const url = /^https?:\/\/\S+$/.test(text) ? text : "";
  const label = url ? "" : text;
  const insert = `[${label}](${url})`;
  // With words selected, type the address next; with an address selected, the words.
  const at = label || !url ? from + label.length + 3 : from + 1;
  view.dispatch({ changes: { from, to, insert }, selection: { anchor: at }, scrollIntoView: true, userEvent: "input.format" });
  return true;
}

/** Put a block (a picture, a divider) on its own lines at the cursor, with a blank line on each side. */
export function insertBlock(view: EditorView, block: string): boolean {
  const { from, to } = view.state.selection.main;
  const doc = view.state.doc;
  const before = doc.sliceString(Math.max(0, from - 2), from);
  const after = doc.sliceString(to, to + 2);
  const lead = from === 0 ? "" : before.endsWith("\n\n") ? "" : before.endsWith("\n") ? "\n" : "\n\n";
  const trail = after.startsWith("\n\n") ? "" : after.startsWith("\n") ? "\n" : "\n\n";
  const insert = lead + block + trail;
  view.dispatch({ changes: { from, to, insert }, selection: { anchor: from + insert.length }, scrollIntoView: true, userEvent: "input" });
  return true;
}

const IMAGE_TYPES = /^image\/(png|jpe?g|gif|webp|svg\+xml|avif|bmp)$/;
const imageFiles = (list: FileList | null | undefined) => Array.from(list ?? []).filter((f) => IMAGE_TYPES.test(f.type));

/** In a screenplay, Tab turns the current line into the next kind of line. */
function cycle(view: EditorView, dir: number): boolean {
  const text = view.state.doc.toString();
  if (!isScreenplay(text)) return false;
  const line = view.state.doc.lineAt(view.state.selection.main.head);
  if (!line.text.trim()) return true; // nothing to reshape yet; keep focus here
  const kind = classify(text.split("\n"), true)[line.number - 1];
  const next = cycleLine(line.text, kind, dir);
  // A cue, heading or transition needs a blank line above it.
  const prev = line.number > 1 ? view.state.doc.line(line.number - 1) : null;
  const pad = prev && prev.text.trim() && next.kind !== "paren" && next.kind !== "action" ? "\n" : "";
  view.dispatch({
    changes: { from: line.from, to: line.to, insert: pad + next.text },
    selection: { anchor: line.from + pad.length + next.text.length },
    userEvent: "input.format",
  });
  return true;
}

/** Enter after dialogue or a heading starts a new paragraph (a blank line), as a script expects. */
function scriptEnter(view: EditorView): boolean {
  const text = view.state.doc.toString();
  if (!isScreenplay(text)) return false;
  const sel = view.state.selection.main;
  const line = view.state.doc.lineAt(sel.head);
  if (!sel.empty || sel.head !== line.to || !line.text.trim()) return false;
  const kind = classify(text.split("\n"), true)[line.number - 1];
  if (kind !== "dialogue" && kind !== "heading" && kind !== "transition") return false;
  view.dispatch({ changes: { from: sel.head, insert: "\n\n" }, selection: { anchor: sel.head + 2 }, scrollIntoView: true, userEvent: "input" });
  return true;
}

export interface EditorHooks {
  onChange: (text: string) => void;
  onPlay: () => void;
  onStop: () => boolean;
  /** Cmd/Ctrl-click on a [[wikilink]]. */
  onLink: (target: string) => void;
  /** The selected text changed ("" when nothing is selected). */
  onSelect?: (text: string) => void;
  /** A click on commented words. */
  onComment?: (id: string) => void;
  /** Where a picture's link points, as something an <img> can show. */
  imageUrl: ImageResolver;
  /** A picture pasted or dropped in: save it to the vault and give back its link, or null. */
  saveImage?: (file: File) => Promise<string | null>;
}

/** The `[[target]]` (alias stripped) under `pos`, if any. */
function linkAt(state: EditorState, pos: number): string | null {
  const line = state.doc.lineAt(pos);
  for (const m of line.text.matchAll(/\[\[([^\]|]+)(?:\|[^\]]*)?\]\]/g)) {
    const a = line.from + m.index!;
    if (pos >= a && pos <= a + m[0].length) return m[1].trim();
  }
  return null;
}

/** How the page looks: a sheet of paper, plain draft, or code with line numbers. */
export type ViewMode = "page" | "draft" | "code";

// Code shows the markdown exactly as written; Page and Draft preview it.
const viewExtensions = (mode: ViewMode, images: ImageResolver): Extension =>
  mode === "code" ? [lineNumbers(), highlightActiveLineGutter(), highlightActiveLine()] : livePreview(images);

export class Editor {
  readonly view: EditorView;
  private extensions: Extension[];
  private viewConf = new Compartment();
  private mode: ViewMode = "draft";
  private images: ImageResolver;

  constructor(parent: HTMLElement, hooks: EditorHooks) {
    this.images = hooks.imageUrl;
    // Pictures pasted or dropped in go into the vault, and a link to each goes where they landed.
    const takeImages = (view: EditorView, files: File[], at?: number) => {
      if (!files.length || !hooks.saveImage) return false;
      if (at != null) view.dispatch({ selection: { anchor: at } });
      (async () => {
        for (const f of files) {
          const path = await hooks.saveImage!(f);
          if (path) insertBlock(view, `![${f.name.replace(/\.[^.]+$/, "").replace(/^image$/, "")}](${path})`);
        }
      })();
      return true;
    };
    this.extensions = [
      minimalSetup,
      // GitHub-flavoured: adds ~~strikethrough~~, - [ ] tasks and tables.
      markdown({ base: markdownLanguage }),
      EditorView.lineWrapping,
      placeholder("Start writing…"),
      onionField,
      commentField,
      speakingField,
      cadenceField,
      frontmatterPlugin,
      screenplayField,
      screenplayClass,
      layoutPlugin,
      // The system spell checker underlines misspellings as you type.
      EditorView.contentAttributes.of({ spellcheck: "true", autocorrect: "off", autocapitalize: "sentences" }),
      // Find and replace in this page: ⌘F opens it, ⌘G / ⇧⌘G step through matches.
      search({ top: true }),
      EditorState.phrases.of({
        Find: "Find",
        Replace: "Replace",
        next: "Next",
        previous: "Previous",
        all: "All",
        "match case": "Match case",
        regexp: "Regex",
        "by word": "Whole words",
        replace: "Replace",
        "replace all": "Replace all",
        close: "Close",
      }),
      keymap.of(searchKeymap),
      // Ours beat the defaults (Enter, and Mod-i, which selects the parent syntax node there).
      Prec.high(keymap.of([
        { key: "Mod-b", run: (v) => toggleWrap(v, "**") },
        { key: "Mod-i", run: (v) => toggleWrap(v, "*") },
        { key: "Mod-Shift-x", run: (v) => toggleWrap(v, "~~") },
        { key: "Mod-e", run: (v) => toggleWrap(v, "`") },
        { key: "Mod-k", run: insertLink },
        { key: "Mod-Shift-8", run: (v) => toggleBlock(v, "bullet") },
        { key: "Mod-Shift-7", run: (v) => toggleBlock(v, "number") },
        { key: "Mod-Shift-9", run: (v) => toggleBlock(v, "task") },
        { key: "Mod-Shift-.", run: (v) => toggleBlock(v, "quote") },
        ...[0, 1, 2, 3, 4, 5, 6].map((n) => ({ key: `Mod-Alt-${n}`, run: (v: EditorView) => setHeading(v, n) })),
        { key: "Mod-Shift-e", run: (v) => setAlign(v, "center") },
        { key: "Mod-Shift-r", run: (v) => setAlign(v, "right") },
        { key: "Mod-Shift-l", run: (v) => setAlign(v, "left") },
        { key: "Tab", run: (v) => cycle(v, 1) },
        { key: "Shift-Tab", run: (v) => cycle(v, -1) },
        { key: "Enter", run: scriptEnter },
        { key: "Mod-Enter", run: () => (hooks.onPlay(), true) },
        { key: "Escape", run: () => hooks.onStop() },
      ])),
      EditorView.updateListener.of((u) => {
        if (u.docChanged) hooks.onChange(u.state.doc.toString());
        if (u.selectionSet || u.docChanged) {
          const sel = u.state.selection.main;
          hooks.onSelect?.(sel.empty ? "" : u.state.sliceDoc(sel.from, sel.to));
        }
      }),
      EditorView.domEventHandlers({
        paste: (e, view) => {
          const files = imageFiles(e.clipboardData?.files);
          if (!files.length) return false;
          e.preventDefault();
          return takeImages(view, files);
        },
        drop: (e, view) => {
          const files = imageFiles(e.dataTransfer?.files);
          if (!files.length) return false;
          e.preventDefault();
          return takeImages(view, files, view.posAtCoords({ x: e.clientX, y: e.clientY }) ?? undefined);
        },
        click: (e) => {
          const id = (e.target as HTMLElement).closest<HTMLElement>("[data-comment]")?.dataset.comment;
          if (id && hooks.onComment) hooks.onComment(id);
          return false;
        },
        mousedown: (e, view) => {
          if (!(e.metaKey || e.ctrlKey)) return false;
          const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
          const target = pos == null ? null : linkAt(view.state, pos);
          if (!target) return false;
          e.preventDefault();
          hooks.onLink(target);
          return true;
        },
      }),
    ];
    this.view = new EditorView({ parent, state: EditorState.create({ extensions: this.all() }) });
  }

  private all(): Extension[] {
    return [...this.extensions, this.viewConf.of(viewExtensions(this.mode, this.images))];
  }

  setViewMode(mode: ViewMode) {
    this.mode = mode;
    this.view.dispatch({ effects: this.viewConf.reconfigure(viewExtensions(mode, this.images)) });
  }

  /** Put the cursor at the end and start typing there. */
  focusEnd() {
    const end = this.view.state.doc.length;
    this.view.dispatch({ selection: { anchor: end }, scrollIntoView: true });
    this.view.focus();
  }

  /** Select the first place `word` occurs, ignoring case, and scroll to it. */
  find(word: string) {
    const at = word ? this.view.state.doc.toString().toLowerCase().indexOf(word.toLowerCase()) : -1;
    if (at < 0) return;
    this.view.dispatch({ selection: { anchor: at, head: at + word.length }, scrollIntoView: true });
  }

  get text(): string {
    return this.view.state.doc.toString();
  }

  /** Swap in a whole new document, with its own fresh undo history. */
  load(text: string) {
    this.view.setState(EditorState.create({ doc: text, extensions: this.all() }));
  }

  /** Highlight these comments' words (replacing any shown before). */
  setComments(ranges: CommentRange[]) {
    this.view.dispatch({ effects: setComments.of(ranges) });
  }

  /** Where each highlighted comment's words are now, after edits. */
  commentRanges(): Map<string, { from: number; to: number }> {
    const out = new Map<string, { from: number; to: number }>();
    this.view.state.field(commentField).between(0, this.view.state.doc.length, (from, to, d) => {
      const id = d.spec.attributes?.["data-comment"];
      if (id) out.set(id, { from, to });
    });
    return out;
  }

  /** The selection, if any text is selected. */
  selection(): { from: number; to: number; text: string } | null {
    const s = this.view.state.selection.main;
    return s.empty ? null : { from: s.from, to: s.to, text: this.view.state.sliceDoc(s.from, s.to) };
  }

  /** Select a range and bring it into view. */
  select(from: number, to: number) {
    this.view.dispatch({ selection: { anchor: from, head: to }, scrollIntoView: true });
    this.view.focus();
  }

  /** Overlay a diff; with `pick`, each difference gets merge buttons. */
  setOnion(ops: DiffOp[] | null, pick: Pick | null = null) {
    this.view.dispatch({ effects: setOnion.of({ ops, pick }) });
  }

  setSpeaking(range: { from: number; to: number } | null) {
    const effects: StateEffect<unknown>[] = [setSpeaking.of(range)];
    if (range && range.to <= this.view.state.doc.length) {
      effects.push(EditorView.scrollIntoView(range.from, { y: "nearest", yMargin: 80 }));
    }
    this.view.dispatch({ effects });
  }

  setCadence(flags: CadenceFlag[] | null) {
    this.view.dispatch({ effects: setCadence.of(flags) });
  }

  /** The selection if there is one, otherwise everything from the cursor on. */
  playRange(): { from: number; to: number } {
    const sel = this.view.state.selection.main;
    return sel.empty ? { from: sel.head, to: this.view.state.doc.length } : { from: sel.from, to: sel.to };
  }

  /** The text just before the cursor (context for the dictation polish pass). */
  textBeforeCursor(max = 1500): string {
    const from = this.view.state.selection.main.from;
    return this.view.state.doc.sliceString(Math.max(0, from - max), from);
  }

  /** Insert dictated text at the cursor, fixing up the space and capital at the seam. */
  insertDictation(text: string) {
    const { from, to } = this.view.state.selection.main;
    const before = this.view.state.doc.sliceString(Math.max(0, from - 200), from);
    let t = text.trim();
    const prev = before.slice(-1);
    if (/(^|[.!?]["”’)]?\s+|\n\s*)$/.test(before) || before.trim() === "") {
      t = t.charAt(0).toUpperCase() + t.slice(1);
    }
    if (prev && !/[\s(\[“‘"—-]/.test(prev) && !/^[,.;:!?)\]”’]/.test(t)) t = " " + t;
    this.view.dispatch({
      changes: { from, to, insert: t },
      selection: { anchor: from + t.length },
      scrollIntoView: true,
      userEvent: "input.dictate",
    });
    this.view.focus();
  }

  focus() {
    this.view.focus();
  }

  bold() {
    toggleWrap(this.view, "**");
    this.view.focus();
  }

  italic() {
    toggleWrap(this.view, "*");
    this.view.focus();
  }

  strike() {
    toggleWrap(this.view, "~~");
    this.view.focus();
  }

  code() {
    toggleWrap(this.view, "`");
    this.view.focus();
  }

  heading(level: number) {
    setHeading(this.view, level);
    this.view.focus();
  }

  /** The heading level of the cursor's line, 0 for body text. */
  get headingLevel(): number {
    return headingLevel(this.view.state);
  }

  block(kind: BlockKind) {
    toggleBlock(this.view, kind);
    this.view.focus();
  }

  link() {
    insertLink(this.view);
    this.view.focus();
  }

  divider() {
    insertBlock(this.view, "---");
    this.view.focus();
  }

  /** Put a picture at the cursor, on its own line. */
  insertImage(path: string, alt = "") {
    insertBlock(this.view, `![${alt}](${path.includes(" ") ? `<${path}>` : path})`);
    this.view.focus();
  }

  align(a: "left" | "center" | "right") {
    setAlign(this.view, a);
    this.view.focus();
  }

  undo(): boolean {
    return undo(this.view);
  }

  redo(): boolean {
    return redo(this.view);
  }
}
