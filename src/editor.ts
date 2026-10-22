// The CodeMirror editor plus its two overlays: the onion skin (diff against
// another variant, deleted words drawn as ghosts) and the narration highlight.
import { minimalSetup } from "codemirror";
import { redo, undo } from "@codemirror/commands";
import { markdown } from "@codemirror/lang-markdown";
import { Compartment, EditorSelection, EditorState, Prec, StateEffect, StateField, type Extension, type Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, highlightActiveLine, highlightActiveLineGutter, keymap, lineNumbers, placeholder, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import type { DiffOp } from "./api";
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

const setOnion = StateEffect.define<DiffOp[] | null>();
const insertMark = Decoration.mark({ class: "wp-ins" });

function onionDecorations(ops: DiffOp[] | null, docLength: number): DecorationSet {
  if (!ops) return Decoration.none;
  const ranges: Range<Decoration>[] = [];
  let pos = 0;
  for (const op of ops) {
    if (op.tag === "delete") {
      ranges.push(Decoration.widget({ widget: new GhostWidget(op.text), side: -1 }).range(pos));
      continue;
    }
    if (op.tag === "insert" && op.text.trim()) ranges.push(insertMark.range(pos, pos + op.text.length));
    pos += op.text.length;
  }
  // A diff computed against older text than what's on screen: drop it, a fresh one is coming.
  if (pos !== docLength) return Decoration.none;
  return Decoration.set(ranges, true);
}

const onionField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(deco, tr) {
    deco = deco.map(tr.changes);
    for (const e of tr.effects) if (e.is(setOnion)) deco = onionDecorations(e.value, tr.state.doc.length);
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

const viewExtensions = (mode: ViewMode): Extension =>
  mode === "code" ? [lineNumbers(), highlightActiveLineGutter(), highlightActiveLine()] : [];

export class Editor {
  readonly view: EditorView;
  private extensions: Extension[];
  private viewConf = new Compartment();
  private mode: ViewMode = "draft";

  constructor(parent: HTMLElement, hooks: EditorHooks) {
    this.extensions = [
      minimalSetup,
      markdown(),
      EditorView.lineWrapping,
      placeholder("Start writing…"),
      onionField,
      speakingField,
      cadenceField,
      frontmatterPlugin,
      screenplayField,
      screenplayClass,
      layoutPlugin,
      // Ours beat the defaults (Enter, and Mod-i, which selects the parent syntax node there).
      Prec.high(keymap.of([
        { key: "Mod-b", run: (v) => toggleWrap(v, "**") },
        { key: "Mod-i", run: (v) => toggleWrap(v, "*") },
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
      }),
      EditorView.domEventHandlers({
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
    return [...this.extensions, this.viewConf.of(viewExtensions(this.mode))];
  }

  setViewMode(mode: ViewMode) {
    this.mode = mode;
    this.view.dispatch({ effects: this.viewConf.reconfigure(viewExtensions(mode)) });
  }

  /** Put the cursor at the end and start typing there. */
  focusEnd() {
    const end = this.view.state.doc.length;
    this.view.dispatch({ selection: { anchor: end }, scrollIntoView: true });
    this.view.focus();
  }

  get text(): string {
    return this.view.state.doc.toString();
  }

  /** Swap in a whole new document, with its own fresh undo history. */
  load(text: string) {
    this.view.setState(EditorState.create({ doc: text, extensions: this.all() }));
  }

  setOnion(ops: DiffOp[] | null) {
    this.view.dispatch({ effects: setOnion.of(ops) });
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
