// Live preview: the page reads like formatted text while staying plain
// markdown. Headings, emphasis, lists, quotes, links and pictures are drawn as
// they'll look, and their marks (#, **, -, >, [](url)) fold away on every line
// except the ones the cursor is on, where you edit the markdown as written.
import { HighlightStyle, syntaxHighlighting, syntaxTree } from "@codemirror/language";
import type { EditorState, Extension, Range } from "@codemirror/state";
import { Decoration, EditorView, ViewPlugin, WidgetType, type DecorationSet, type ViewUpdate } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";
import { alignOf, isScreenplay } from "./screenplay";

/** Turns a link's target (vault-relative or a web address) into something an <img> can load. */
export type ImageResolver = (src: string) => Promise<string>;

class BulletWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const span = document.createElement("span");
    span.className = "cm-lp-bullet";
    span.textContent = "•";
    return span;
  }
}

class TaskWidget extends WidgetType {
  constructor(readonly done: boolean) {
    super();
  }
  eq(other: TaskWidget) {
    return other.done === this.done;
  }
  toDOM(view: EditorView) {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "cm-lp-task";
    box.checked = this.done;
    box.onmousedown = (e) => {
      e.preventDefault();
      // `[ ]` or `[x]`: flip the character between the brackets.
      const at = view.posAtDOM(box) + 1;
      const done = /[xX]/.test(view.state.sliceDoc(at, at + 1));
      view.dispatch({ changes: { from: at, to: at + 1, insert: done ? " " : "x" }, userEvent: "input.toggle" });
    };
    return box;
  }
  ignoreEvent() {
    return true;
  }
}

class RuleWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const hr = document.createElement("span");
    hr.className = "cm-lp-hr";
    return hr;
  }
}

class ImageWidget extends WidgetType {
  constructor(readonly src: string, readonly alt: string, readonly resolve: ImageResolver) {
    super();
  }
  eq(other: ImageWidget) {
    return other.src === this.src && other.alt === this.alt;
  }
  toDOM() {
    const wrap = document.createElement("span");
    wrap.className = "cm-lp-image";
    const img = document.createElement("img");
    img.alt = this.alt;
    img.title = this.alt || this.src;
    img.onerror = () => {
      wrap.classList.add("missing");
      wrap.textContent = `Picture not found: ${this.src}`;
    };
    this.resolve(this.src).then((url) => (img.src = url), () => img.onerror?.(new Event("error")));
    wrap.appendChild(img);
    return wrap;
  }
  ignoreEvent() {
    return false;
  }
}

const hide = Decoration.replace({});
const bullet = Decoration.replace({ widget: new BulletWidget() });
const rule = Decoration.replace({ widget: new RuleWidget() });
const linkText = Decoration.mark({ class: "cm-lp-link" });
const lineDeco = new Map<string, Decoration>();
const lineClass = (cls: string) => {
  let d = lineDeco.get(cls);
  if (!d) lineDeco.set(cls, (d = Decoration.line({ class: cls })));
  return d;
};

/** Lines any selection touches: those show their markdown. */
function activeLines(state: EditorState, focused: boolean): Set<number> {
  const out = new Set<number>();
  if (!focused) return out;
  for (const r of state.selection.ranges) {
    for (let n = state.doc.lineAt(r.from).number; n <= state.doc.lineAt(r.to).number; n++) out.add(n);
  }
  return out;
}

/** End of a leading `---` properties block, which isn't markdown to preview. */
function frontmatterEnd(state: EditorState): number {
  const doc = state.doc;
  if (doc.lines < 2 || doc.line(1).text.trim() !== "---") return 0;
  for (let n = 2; n <= Math.min(doc.lines, 200); n++) if (doc.line(n).text.trim() === "---") return doc.line(n).to;
  return 0;
}

function build(view: EditorView, resolve: ImageResolver): DecorationSet {
  const { state } = view;
  const doc = state.doc;
  if (isScreenplay(doc.toString())) return Decoration.none;
  const active = activeLines(state, view.hasFocus);
  const fmEnd = frontmatterEnd(state);
  const ranges: Range<Decoration>[] = [];
  const shown = (pos: number) => active.has(doc.lineAt(pos).number);
  const hideMark = (from: number, to: number, trailingSpace = false) => {
    if (trailingSpace && doc.sliceString(to, to + 1) === " ") to++;
    if (from < to && !shown(from)) ranges.push(hide.range(from, to));
  };

  for (const { from, to } of view.visibleRanges) {
    // Alignment (`> centered <`, `>> right`): the layout plugin places the line; fold the markers.
    for (let n = doc.lineAt(from).number; n <= doc.lineAt(to).number; n++) {
      const line = doc.line(n);
      if (line.from < fmEnd || active.has(n) || !alignOf(line.text)) continue;
      const lead = /^\s*>>?\s*/.exec(line.text)![0].length;
      const trail = /\s*<\s*$/.exec(line.text)?.[0].length ?? 0;
      if (lead) ranges.push(hide.range(line.from, line.from + lead));
      if (trail && alignOf(line.text) === "center" && line.to - trail > line.from + lead) ranges.push(hide.range(line.to - trail, line.to));
    }

    syntaxTree(state).iterate({
      from,
      to,
      enter: (node) => {
        if (node.from < fmEnd) return node.name === "Document";
        const name = node.name;
        const heading = /^(?:ATX|Setext)Heading(\d)$/.exec(name);
        if (heading) {
          ranges.push(lineClass(`cm-lp-h${heading[1]}`).range(doc.lineAt(node.from).from));
          return;
        }
        switch (name) {
          case "HeaderMark": {
            // Setext underlines (=== / ---) sit on their own line; fold them too.
            const setext = node.node.parent?.name.startsWith("Setext");
            hideMark(node.from, node.to, !setext);
            return;
          }
          case "EmphasisMark":
          case "StrikethroughMark":
          case "CodeMark":
            if (node.node.parent?.name !== "FencedCode") hideMark(node.from, node.to);
            return;
          case "Blockquote": {
            for (let n = doc.lineAt(node.from).number; n <= doc.lineAt(node.to).number; n++) {
              const line = doc.line(n);
              if (!alignOf(line.text)) ranges.push(lineClass("cm-lp-quote").range(line.from));
            }
            return;
          }
          case "QuoteMark":
            if (!alignOf(doc.lineAt(node.from).text)) hideMark(node.from, node.to, true);
            return;
          case "FencedCode":
            for (let n = doc.lineAt(node.from).number; n <= doc.lineAt(node.to).number; n++) ranges.push(lineClass("cm-lp-code").range(doc.line(n).from));
            return false;
          case "HorizontalRule":
            if (!shown(node.from)) ranges.push(rule.range(node.from, node.to));
            return;
          case "ListMark": {
            if (shown(node.from)) return;
            const item = node.node.parent;
            if (item?.getChild("Task")) ranges.push(hide.range(node.from, Math.min(node.to + 1, doc.lineAt(node.from).to)));
            else if (item?.parent?.name === "BulletList") ranges.push(bullet.range(node.from, node.to));
            return;
          }
          case "TaskMarker":
            if (!shown(node.from)) {
              const done = /[xX]/.test(doc.sliceString(node.from + 1, node.to - 1));
              ranges.push(Decoration.replace({ widget: new TaskWidget(done) }).range(node.from, node.to));
              if (done) ranges.push(Decoration.mark({ class: "cm-lp-done" }).range(Math.min(node.to + 1, doc.lineAt(node.to).to), doc.lineAt(node.to).to));
            }
            return;
          case "Image": {
            if (shown(node.from)) return false;
            const marks = node.node.getChildren("LinkMark");
            const url = node.node.getChild("URL");
            if (!url || marks.length < 2) return false;
            const alt = doc.sliceString(marks[0].to, marks[1].from);
            ranges.push(Decoration.replace({ widget: new ImageWidget(doc.sliceString(url.from, url.to), alt, resolve) }).range(node.from, node.to));
            return false;
          }
          case "Link": {
            const marks = node.node.getChildren("LinkMark");
            if (!node.node.getChild("URL") || marks.length < 2) return;
            ranges.push(linkText.range(marks[0].to, marks[1].from));
            hideMark(marks[0].from, marks[0].to);
            hideMark(marks[1].from, node.to);
            return;
          }
        }
      },
    });
  }
  return Decoration.set(ranges, true);
}

const highlight = HighlightStyle.define([
  { tag: t.heading, fontWeight: "700" },
  { tag: t.strong, fontWeight: "700" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.monospace, fontFamily: "ui-monospace, Menlo, monospace", fontSize: "0.9em" },
  { tag: [t.processingInstruction, t.meta, t.url], color: "var(--muted)" },
]);

export function livePreview(resolve: ImageResolver): Extension {
  const plugin = ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = build(view, resolve);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged || u.selectionSet || u.focusChanged || syntaxTree(u.startState) !== syntaxTree(u.state)) {
          this.decorations = build(u.view, resolve);
        }
      }
    },
    { decorations: (p) => p.decorations },
  );
  return [plugin, syntaxHighlighting(highlight), EditorView.editorAttributes.of({ class: "wp-live" })];
}
