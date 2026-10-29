import "./devbridge";
import "./styles.css";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { api, type AudioSegment, type Block, type Book, type Cadence, type Comment, type CustomTheme, type Cut, type Entity, type LibraryReport, type ScriptSummary, type HistoryEntry, type Lexicon, type Manuscript, type NoteEntry, type Prop, type Target, type Settings, type ThemeMode, type Variant, type WorldSuggestion, type WorldSuggestions } from "./api";
import { applyTheme, contrast, PRESETS } from "./theme";
import { Editor, type ViewMode } from "./editor";
import { Narrator, type Segment } from "./narrator";
import { sentences, speakable } from "./sentences";
import { KINDS, renderGraph, renderTimeline, stopGraph, swatch } from "./world";
import { renderLibrary, seriesVar } from "./library";
import { autoCast, characters, isScreenplay, pronounHints, tableRead } from "./screenplay";
import { initUpdater } from "./updater";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

type Doc = { kind: "note"; path: string } | { kind: "block"; blockId: string; variantId: string } | null;

let settings: Settings;
let ms: Manuscript | null = null;
let doc: Doc = null;
let compareId: string | null = null;
/** A past snapshot to onion-skin against, instead of another version. */
let compareSnap: { label: string; text: string } | null = null;
let ghost = 60;
let lexicon: Lexicon = {};
let notes: NoteEntry[] = [];
let voices: string[] = [];

// ---------- small helpers ----------

function debounce(fn: () => void, ms: number) {
  let t: number | undefined;
  const run = () => {
    if (t === undefined) return;
    clearTimeout(t);
    t = undefined;
    fn();
  };
  const call = () => {
    clearTimeout(t);
    t = window.setTimeout(run, ms);
  };
  call.flush = run;
  return call;
}

let toastTimer: number | undefined;
/** A message at the bottom of the window, optionally with one action (like Undo). */
function toast(msg: string, error = false, action?: { label: string; run: () => void }) {
  const box = $("toast");
  box.replaceChildren(document.createTextNode(msg));
  if (action) {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = action.label;
    b.onclick = () => {
      box.className = "";
      action.run();
    };
    box.append(b);
  }
  box.className = `show${error ? " error" : ""}${action ? " has-action" : ""}`;
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => (box.className = ""), action ? 10000 : error ? 7000 : 3000);
}
const fail = (e: unknown) => toast(String(e), true);

// Pictures in pages: web addresses load as they are; anything else is a file in the vault,
// read once per vault and kept as a blob URL so retyping nearby doesn't reload it.
const imageCache = new Map<string, Promise<string>>();
const IMAGE_MIME: Record<string, string> = { svg: "image/svg+xml", jpg: "image/jpeg" };
function imageUrl(src: string): Promise<string> {
  if (/^(https?:|data:)/i.test(src)) return Promise.resolve(src);
  const path = decodeURI(src.replace(/^<|>$/g, "")).replace(/^\.?\//, "");
  const key = `${settings?.vault}\n${path}`;
  let url = imageCache.get(key);
  if (!url) {
    const ext = path.split(".").pop()!.toLowerCase();
    url = api.readImage(path).then((bytes) => URL.createObjectURL(new Blob([bytes], { type: IMAGE_MIME[ext] ?? `image/${ext}` })));
    url.catch(() => imageCache.delete(key));
    imageCache.set(key, url);
  }
  return url;
}

async function pickImage() {
  const picked = await open({ multiple: false, title: "Insert a picture", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp"] }] });
  if (typeof picked !== "string") return;
  const name = picked.split(/[\\/]/).pop()!.replace(/\.[^.]+$/, "");
  editor.insertImage(await api.importImage(picked), name);
}

const words = (s: string) => (s.match(/[\p{L}\p{N}’']+/gu) ?? []).length;

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Partial<HTMLElementTagNameMap[K]> = {}, ...kids: (Node | string)[]) {
  const node: HTMLElementTagNameMap[K] = document.createElement(tag);
  Object.assign(node, props);
  node.append(...kids);
  return node;
}

/** A clickable row (list item, property) that the keyboard can reach and open with Enter or Space. */
function clickable(node: HTMLElement, run: () => unknown) {
  node.onclick = run;
  node.tabIndex = 0;
  node.addEventListener("keydown", (e) => {
    if ((e.key === "Enter" || e.key === " ") && e.target === node) {
      e.preventDefault();
      node.click();
    }
  });
}

/** A button that needs a second click within 3s, instead of a confirm dialog. */
function confirmButton(label: string, onConfirm: () => void, cls = "ghost danger") {
  const b = el("button", { className: cls, textContent: label, type: "button" });
  let armed = false;
  b.onclick = (e) => {
    e.stopPropagation();
    if (armed) return onConfirm();
    armed = true;
    b.textContent = "Sure?";
    setTimeout(() => ((armed = false), (b.textContent = label)), 3000);
  };
  return b;
}

// ---------- current document ----------

function currentBlock(): Block | null {
  const d = doc;
  return d?.kind === "block" ? ms?.blocks.find((b) => b.id === d.blockId) ?? null : null;
}
function currentVariant(): Variant | null {
  const d = doc;
  const b = currentBlock();
  return b && d?.kind === "block" ? b.variants.find((v) => v.id === d.variantId) ?? null : null;
}
function compareVariant(): Variant | null {
  return currentBlock()?.variants.find((v) => v.id === compareId) ?? null;
}

/** What the onion skin compares against: a snapshot, or another version. */
function compareSource(): { label: string; text: string } | null {
  if (compareSnap) return compareSnap;
  const v = compareVariant();
  return v ? { label: v.label, text: v.text } : null;
}

let lastSave: Promise<unknown> = Promise.resolve();
/** Edits since the last snapshot, for autosave. */
let dirty = false;

let pendingTake: { ms: string; take: string; text: string } | null = null;
const saveTake = debounce(() => {
  dirty = true;
  if (pendingTake) lastSave = api.writeTake(pendingTake.ms, pendingTake.take, pendingTake.text).catch(fail);
  pendingTake = null;
}, 500);

/** Adopt a manuscript fresh from disk (after an operation, or another writer's change). */
function applyMs(m: Manuscript) {
  // Keep typing that hasn't reached disk yet.
  const v = currentVariant();
  if (v && pendingTake?.take === v.id) {
    const fresh = m.blocks.flatMap((b) => b.variants).find((x) => x.id === v.id);
    if (fresh) fresh.text = editor.text;
  }
  ms = m;
  if (compareId && !compareVariant()) compareId = null;
  renderOutline();
  renderCuts();
  renderProgress();
  if (doc?.kind === "block") {
    if (!currentVariant()) return openBlock(doc.blockId); // our take was deleted
    renderVariants();
    renderCompare();
    renderProps();
  }
}

/** Run a structural change and adopt the result. */
async function op(change: Promise<Manuscript>): Promise<Manuscript | null> {
  try {
    const m = await change;
    applyMs(m);
    return m;
  } catch (e) {
    fail(e);
    return null;
  }
}

let pendingNote: { path: string; text: string } | null = null;
const saveNote = debounce(() => {
  dirty = true;
  if (pendingNote) lastSave = api.writeNote(pendingNote.path, pendingNote.text).catch(fail);
  pendingNote = null;
}, 600);

/** Write any pending edits now; resolves once they're on disk. */
function flushSaves(): Promise<unknown> {
  saveCommentsSoon.flush();
  saveTake.flush();
  saveNote.flush();
  return lastSave;
}

// ---------- editor + narration ----------

// ---------- today's words ----------

/** Net words added today, kept per computer: { day, n }. */
const TODAY_KEY = "wp-today-words";
const today = () => new Date().toLocaleDateString("en-CA");
let todayWords = (() => {
  try {
    const t = JSON.parse(localStorage.getItem(TODAY_KEY) ?? "null");
    return t?.day === today() ? Number(t.n) || 0 : 0;
  } catch {
    return 0;
  }
})();
/** Words in the open page or note when it was last counted. */
let countedWords = 0;

function renderToday() {
  const box = $("today-words");
  box.hidden = !settings?.vault;
  box.textContent = `${todayWords.toLocaleString()} word${Math.abs(todayWords) === 1 ? "" : "s"} today`;
}

/** Count what the writer just typed (or cut) toward today. */
function countToday(text: string) {
  const n = words(text);
  const day = today();
  try {
    const saved = JSON.parse(localStorage.getItem(TODAY_KEY) ?? "null");
    if (saved?.day !== day) todayWords = 0;
  } catch {
    /* ignore */
  }
  todayWords += n - countedWords;
  countedWords = n;
  try {
    localStorage.setItem(TODAY_KEY, JSON.stringify({ day, n: todayWords }));
  } catch {
    /* storage unavailable: the count lasts until the app closes */
  }
  renderToday();
}

// ---------- comments ----------

/** Every comment in the open manuscript (all takes). */
let comments: Comment[] = [];
/** The comment open for typing, if any. */
let editingComment: string | null = null;
/** Comments whose words can't be found in the text any more. */
const detached = new Set<string>();

function takeComments(): Comment[] {
  const d = doc;
  return d?.kind === "block" ? comments.filter((c) => c.take === d.variantId) : [];
}

async function loadComments() {
  comments = ms ? await api.listComments(ms.id).catch(() => []) : [];
  anchorComments();
}

/**
 * Put this take's comments on its words: where they were if the words are
 * still there, else the nearest place the words appear, else nowhere.
 */
function anchorComments() {
  const text = editor.text;
  detached.clear();
  for (const c of takeComments()) {
    if (text.slice(c.from, c.to) === c.quote) continue;
    let best = -1;
    for (let i = text.indexOf(c.quote); i >= 0 && c.quote; i = text.indexOf(c.quote, i + 1)) {
      if (best < 0 || Math.abs(i - c.from) < Math.abs(best - c.from)) best = i;
    }
    if (best >= 0) [c.from, c.to] = [best, best + c.quote.length];
    else detached.add(c.id);
  }
  showComments();
}

function showComments() {
  editor.setComments(
    takeComments()
      .filter((c) => !c.resolved && !detached.has(c.id))
      .map((c) => ({ id: c.id, from: c.from, to: c.to, active: c.id === editingComment })),
  );
  renderComments();
}

/** Save this take's comments, with their places moved by the latest edits. */
async function saveComments() {
  const d = doc;
  if (d?.kind !== "block" || !ms) return;
  const at = editor.commentRanges();
  const text = editor.text;
  for (const c of takeComments()) {
    const r = at.get(c.id);
    if (r) [c.from, c.to, c.quote] = [r.from, r.to, text.slice(r.from, r.to)];
    else if (!c.resolved && !detached.has(c.id)) detached.add(c.id);
  }
  await api.setComments(ms.id, d.variantId, takeComments().filter((c) => c.text.trim() || c.id === editingComment)).catch(fail);
}
const saveCommentsSoon = debounce(() => saveComments(), 1000);

function addComment() {
  const d = doc;
  const sel = editor.selection();
  if (d?.kind !== "block" || !ms) return;
  if (!sel || !sel.text.trim()) return toast("Select the words you want to comment on first.");
  const c: Comment = { id: crypto.randomUUID().replace(/-/g, "").slice(0, 12), take: d.variantId, quote: sel.text, from: sel.from, to: sel.to, text: "", author: "", created: Math.floor(Date.now() / 1000), resolved: false };
  comments.push(c);
  editingComment = c.id;
  showComments();
  $("comments").querySelector<HTMLTextAreaElement>(`[data-id="${c.id}"] textarea`)?.focus();
}

function openComment(id: string) {
  const c = comments.find((x) => x.id === id);
  if (!c) return;
  editingComment = null;
  showComments();
  const row = $("comments").querySelector<HTMLElement>(`[data-id="${id}"]`);
  row?.scrollIntoView({ block: "nearest" });
  row?.classList.add("flash");
  setTimeout(() => row?.classList.remove("flash"), 900);
}

function renderComments() {
  const box = $("comments");
  const list = takeComments();
  const open = list.filter((c) => !c.resolved);
  const done = list.filter((c) => c.resolved);
  box.replaceChildren();
  if (!list.length) box.append(el("p", { className: "hint", textContent: "Select some words and add a note about them. Comments stay out of the text and the export." }));
  const row = (c: Comment) => {
    const li = el("div", { className: `comment${c.resolved ? " resolved" : ""}${detached.has(c.id) ? " detached" : ""}` });
    li.dataset.id = c.id;
    const quote = el("button", { type: "button", className: "comment-quote", textContent: `“${c.quote.length > 80 ? `${c.quote.slice(0, 80)}…` : c.quote}”`, title: detached.has(c.id) ? "These words were changed or removed" : "Show these words" });
    quote.onclick = () => {
      const r = editor.commentRanges().get(c.id);
      if (r) editor.select(r.from, r.to);
    };
    li.append(quote);
    if (detached.has(c.id)) li.append(el("p", { className: "hint", textContent: "The words it was on have changed." }));
    if (c.id === editingComment) {
      const ta = el("textarea", { value: c.text, placeholder: "Your note… (Enter saves, Shift+Enter for a new line)", rows: 3 });
      const finish = (keep: boolean) => {
        editingComment = null;
        if (keep && ta.value.trim()) c.text = ta.value.trim();
        if (!c.text.trim()) comments = comments.filter((x) => x !== c);
        showComments();
        saveComments();
      };
      ta.onkeydown = (e) => {
        if (e.key === "Enter" && !e.shiftKey) (e.preventDefault(), finish(true));
        if (e.key === "Escape") (e.preventDefault(), e.stopPropagation(), finish(false));
      };
      ta.onblur = () => editingComment === c.id && finish(true);
      li.append(ta);
    } else {
      const body = el("p", { className: "comment-text", textContent: c.text, title: "Click to edit" });
      body.onclick = () => {
        editingComment = c.id;
        showComments();
        $("comments").querySelector<HTMLTextAreaElement>(`[data-id="${c.id}"] textarea`)?.focus();
      };
      li.append(body);
    }
    const resolve = el("button", { type: "button", className: "ghost", textContent: c.resolved ? "Reopen" : "Resolve", title: c.resolved ? "Show it in the text again" : "Done with it: hide it from the text, keep it here" });
    resolve.onclick = () => {
      c.resolved = !c.resolved;
      showComments();
      saveComments();
    };
    const del = Object.assign(
      confirmButton("×", () => {
        comments = comments.filter((x) => x !== c);
        showComments();
        saveComments();
      }),
      { title: "Delete this comment (click twice)" },
    );
    li.append(
      el("div", { className: "comment-meta" }, el("span", { textContent: `${c.author ? `🤖 ${c.author}` : "You"} · ${when(c.created)}` }), el("span", { className: "spacer" }), resolve, del),
    );
    return li;
  };
  box.append(...open.map(row));
  if (done.length) {
    const more = el("details", { className: "comments-done" }, el("summary", { textContent: `Resolved (${done.length})` }), ...done.map(row));
    box.append(more);
  }
}

// ---------- focus mode ----------

function setFocus(on: boolean) {
  if (on && document.body.dataset.mode !== "block" && document.body.dataset.mode !== "note") return toast("Open a page or a note to focus on it.");
  document.body.classList.toggle("focus", on);
  if (on) editor.focus();
}

const editor = new Editor($("editor"), {
  onComment(id) {
    openComment(id);
  },
  imageUrl,
  saveImage: (file) => api.saveImage(file).catch((e) => (fail(e), null)),
  onSelect(text) {
    $<HTMLSelectElement>("fmt-heading").value = String(editor.headingLevel);
    const n = text ? words(text) : 0;
    $("sel-words").textContent = n ? `${n.toLocaleString()} word${n === 1 ? "" : "s"} selected` : "";
  },
  onChange(text) {
    countToday(text);
    if (takeComments().length) saveCommentsSoon();
    if (narrator.playing) narrator.stop();
    if (doc?.kind === "note") {
      pendingNote = { path: doc.path, text };
      saveNote();
      refreshCast();
      refreshDiff();
    } else {
      const v = currentVariant();
      if (!v || !ms) return;
      v.text = text;
      pendingTake = { ms: ms.id, take: v.id, text };
      saveTake();
      refreshCadence();
      refreshCast();
      refreshDiff();
    }
  },
  onPlay: () => togglePlay(),
  onStop: () => {
    if (!narrator.playing) return false;
    narrator.stop();
    return true;
  },
  onLink: (target) => openPageLink(target).catch(fail),
});

const narrator = new Narrator(() => ({
  voice: settings.voice,
  italicVoice: settings.italic_voice,
  speed: settings.speed,
  paragraphPause: settings.paragraph_pause,
}));
narrator.onSegment = (seg) => {
  editor.setSpeaking(seg?.highlight ? seg : null);
  $("now-playing").textContent = seg ? seg.label ?? "" : "";
};
narrator.onPlaying = (playing) => {
  $("btn-play").textContent = playing ? "■ Stop" : "▶ Listen";
  document.body.classList.toggle("playing", playing);
};
narrator.onError = narrationFail;

/** Narration errors that mean Kokoro isn't installed get a one-click fix. */
function narrationFail(e: unknown, prefix = "") {
  const msg = String(e);
  if (/Kokoro failed to load|Couldn't start Python|narration worker exited/.test(msg)) {
    toast(`${prefix}Narration isn't set up on this computer yet.`, true, { label: "Set up narration", run: () => setupNarration() });
  } else toast(prefix + msg, true);
}

let settingUp = false;
async function setupNarration() {
  if (settingUp) return;
  settingUp = true;
  const btn = $<HTMLButtonElement>("btn-setup-narration");
  btn.disabled = true;
  toast("Setting up narration…");
  try {
    const s = await api.setupNarration();
    settings.python = s.python;
    settings.model_dir = s.model_dir;
    $<HTMLInputElement>("set-python").value = s.python;
    $<HTMLInputElement>("set-models").value = s.model_dir;
    await loadVoices();
    if (voices.length) toast("Narration is ready. Press Listen.");
  } catch (e) {
    if (/needs Python 3\.10/.test(String(e))) showPythonHelp();
    else toast(String(e), true);
  } finally {
    settingUp = false;
    btn.disabled = false;
  }
}

/** No Python 3.10+: say what to install, then try the setup again. */
function showPythonHelp() {
  const dlg = $<HTMLDialogElement>("dlg-python");
  const windows = navigator.userAgent.includes("Windows");
  dlg.querySelectorAll<HTMLElement>(".win-only").forEach((x) => (x.hidden = !windows));
  dlg.querySelectorAll<HTMLElement>(".mac-only").forEach((x) => (x.hidden = windows));
  dlg.returnValue = "";
  dlg.onclose = () => {
    if (dlg.returnValue === "retry") setupNarration();
  };
  dlg.showModal();
}
listen<{ message: string }>("narration-setup", (e) => toast(e.payload.message));

function togglePlay() {
  if (narrator.playing) return narrator.stop();
  const { from, to } = editor.playRange();
  const segs: Segment[] = isScreenplay(editor.text)
    ? tableRead(editor.text, from, to, { narrator: cast["__narrator"] ?? settings.voice, cast, fallback: settings.voice })
    : sentences(editor.text, from, to).map((s) => ({ ...s, highlight: true }));
  if (!segs.length) return toast("Nothing to read from here. Put the cursor in some text.");
  narrator.play(segs);
}

function playAB() {
  const a = compareSource();
  if (!a) return toast("Pick a version to compare with first.");
  const bLabel = currentVariant()?.label ?? "now";
  const segA: Segment[] = sentences(a.text).map((s) => ({ ...s, highlight: false, label: `A · ${a.label}` }));
  const segB: Segment[] = sentences(editor.text).map((s) => ({ ...s, highlight: true, label: `B · ${bLabel}` }));
  if (segA.length) segA[segA.length - 1].pauseAfter = 1.4;
  narrator.play([...segA, ...segB]);
}

let diffSeq = 0;
const refreshDiff = debounce(async () => {
  const a = compareSource();
  const seq = ++diffSeq;
  if (!a || ghost === 0 || !doc) {
    editor.setOnion(null);
    $("diff-stats").textContent = "";
    return;
  }
  const text = editor.text;
  try {
    const ops = await api.diffWords(a.text, text);
    if (seq !== diffSeq || editor.text !== text) return;
    editor.setOnion(ops);
    const added = ops.filter((o) => o.tag === "insert").reduce((n, o) => n + words(o.text), 0);
    const cut = ops.filter((o) => o.tag === "delete").reduce((n, o) => n + words(o.text), 0);
    $("diff-stats").textContent = added || cut ? `+${added} words, −${cut} words vs ${a.label}` : `Same as ${a.label}`;
  } catch (e) {
    fail(e);
  }
}, 200);

function setGhost(value: number) {
  ghost = value;
  $("editor").style.setProperty("--ghost", String(value / 100));
  refreshDiff();
  refreshDiff.flush();
}

// ---------- dictation ----------

type DictState = "idle" | "recording" | "working";
let dictState: DictState = "idle";
let dictStarting: Promise<void> | null = null;
/** Tapped (not held) Right ⌥ or the mic button: keep listening until the next tap. */
let handsFree = false;

function setDictState(next: DictState) {
  dictState = next;
  document.body.classList.toggle("dictating", next === "recording");
  document.body.classList.toggle("transcribing", next === "working");
  $("btn-dictate").textContent = next === "recording" ? "● Listening" : next === "working" ? "… Writing" : "🎙 Dictate";
}

function startDictation(free: boolean) {
  if (dictState !== "idle") return;
  if (document.body.dataset.mode === "none") return toast("Open a note or a block to dictate into.");
  narrator.stop(); // don't let the mic hear the narrator
  handsFree = free;
  setDictState("recording");
  dictStarting = api.dictateStart().catch((e) => {
    setDictState("idle");
    fail(e);
  });
}

async function stopDictation() {
  if (dictState !== "recording") return;
  setDictState("working");
  await dictStarting;
  try {
    const r = await api.dictateStop(editor.textBeforeCursor());
    if (r.warning) toast(r.warning, true);
    if (r.text) editor.insertDictation(r.text);
    else if (!r.warning) toast("Didn't catch any speech.");
  } catch (e) {
    fail(e);
  }
  setDictState("idle");
}

async function cancelDictation() {
  if (dictState !== "recording") return;
  setDictState("idle");
  await dictStarting;
  api.dictateCancel().catch(fail);
}

/** Right ⌥ alone: hold to talk, tap to toggle. Any other key during the hold makes it a chord and cancels. */
function wireDictationKey() {
  let downAt = 0;
  let chord = false;
  window.addEventListener(
    "keydown",
    (e) => {
      if (e.code === "AltRight") {
        if (e.repeat) return;
        if (dictState === "recording" && handsFree) {
          downAt = 0;
          stopDictation();
          return;
        }
        downAt = performance.now();
        chord = false;
        startDictation(false);
        return;
      }
      if (downAt && !chord) {
        chord = true;
        if (!handsFree) cancelDictation();
      }
    },
    true,
  );
  window.addEventListener(
    "keyup",
    (e) => {
      if (e.code !== "AltRight" || !downAt) return;
      const held = performance.now() - downAt;
      downAt = 0;
      if (chord) return;
      if (held < 300) handsFree = true;
      else stopDictation();
    },
    true,
  );
}

// ---------- opening things ----------

type Mode = "none" | "note" | "block" | "graph" | "timeline" | "library";

function setMode(mode: Mode) {
  if (document.body.dataset.mode === "graph" && mode !== "graph") stopGraph();
  // The take buttons act on a block; they must never outlive it.
  if (mode !== "block") {
    $("variants").replaceChildren();
    $("props").replaceChildren();
  }
  document.body.dataset.mode = mode;
  if (mode !== "block" && mode !== "note") document.body.classList.remove("focus");
  $("sel-words").textContent = "";
  $("empty").hidden = mode !== "none";
  $("graph-view").hidden = mode !== "graph";
  $("timeline-view").hidden = mode !== "timeline";
  $("library-view").hidden = mode !== "library";
  if (mode !== "note") $("connections-section").hidden = true;
}

function openBlock(blockId: string, variantId?: string) {
  flushSaves();
  narrator.stop();
  const block = ms?.blocks.find((b) => b.id === blockId);
  if (!block) return;
  const variant = block.variants.find((v) => v.id === (variantId ?? block.live)) ?? block.variants[block.variants.length - 1];
  doc = { kind: "block", blockId, variantId: variant.id };
  // Default comparison: the version just before this one.
  if (!block.variants.some((v) => v.id === compareId && v.id !== variant.id)) {
    const i = block.variants.indexOf(variant);
    compareId = block.variants[i - 1]?.id ?? block.variants[i + 1]?.id ?? null;
  }
  compareSnap = null;
  setMode("block");
  $("note-actions").replaceChildren(
    Object.assign(
      confirmButton("Delete page", () => deletePage(block.id)),
      { title: "Delete this page and all its versions (click twice; Undo brings it back). To set it aside instead, leave it out of the cut." },
    ),
  );
  const title = $<HTMLInputElement>("doc-title");
  title.value = block.title;
  title.readOnly = false;
  editor.load(variant.text);
  countedWords = words(variant.text);
  editingComment = null;
  anchorComments();
  $("cadence").replaceChildren();
  runCadence();
  loadCast().catch(() => {});
  renderVariants();
  renderCompare();
  renderOutline();
  propEdit = null;
  propDraft = null;
  renderProps();
  setGhost(ghost);
  loadHistory();
  loadSceneCast();
}

async function openNote(path: string) {
  flushSaves();
  narrator.stop();
  const text = await api.readNote(path);
  doc = { kind: "note", path };
  setMode("note");
  const title = $<HTMLInputElement>("doc-title");
  title.value = path.replace(/\.md$/, "");
  title.readOnly = true;
  compareSnap = null;
  editor.load(text);
  countedWords = words(text);
  loadCast().catch(() => {});
  editor.setOnion(null);
  $("note-actions").replaceChildren(confirmButton("Delete note", () => deleteNote(path)));
  renderNotes();
  renderOutline();
  renderCompare();
  loadHistory();
  loadConnections();
  renderWorld();
  editor.focus();
}

async function openLink(target: string) {
  const t = target.toLowerCase().replace(/\.md$/, "");
  let note = notes.find((n) => n.name.toLowerCase() === t || n.path.toLowerCase().replace(/\.md$/, "") === t);
  if (!note) {
    const path = await api.createNote(target);
    await loadNotes();
    note = notes.find((n) => n.path === path);
  }
  if (note) {
    showTab("notes");
    await openNote(note.path);
  }
}

/** Open a `[[link]]`: `[[Manuscript#Page]]`, a page of this manuscript, or a note. */
async function openPageLink(target: string) {
  const norm = (s: string) => s.trim().toLowerCase();
  const hash = target.indexOf("#");
  if (hash >= 0) {
    const [msName, page] = [target.slice(0, hash), target.slice(hash + 1)];
    const info = (await api.listManuscripts()).find((m) => norm(m.title) === norm(msName) || m.id === msName.trim());
    const m = info && (info.id === ms?.id ? ms : await api.loadManuscript(info.id));
    const block = m?.blocks.find((b) => norm(b.title) === norm(page));
    if (info && block) return openScene(info.id, block.id);
    return toast(`No page “${target}”.`, true);
  }
  const page = ms?.blocks.find((b) => norm(b.title) === norm(target));
  if (page && doc?.kind === "block") return openBlock(page.id);
  return openLink(target);
}

async function openManuscript(id: string) {
  flushSaves();
  ms = await api.loadManuscript(id);
  compareId = null;
  comments = await api.listComments(id).catch(() => []);
  document.body.classList.add("has-ms");
  $("ms-picker").hidden = true;
  $("ms-outline").hidden = false;
  $("ms-title").textContent = ms.title;
  renderCuts();
  const first = ms.blocks.find((b) => !b.raw) ?? ms.blocks[0];
  if (first) await openBlock(first.id);
}

function closeManuscript() {
  flushSaves();
  narrator.stop();
  ms = null;
  if (doc?.kind === "block") {
    doc = null;
    setMode("none");
    $("empty-msg").textContent = "Pick a manuscript or a note.";
    $("btn-empty-vault").hidden = true;
  }
  document.body.classList.remove("has-ms");
  $("ms-picker").hidden = false;
  $("ms-outline").hidden = true;
  loadManuscripts().catch(fail);
}

// ---------- rendering ----------

function renderOutline() {
  const list = $("outline");
  list.replaceChildren();
  if (!ms) return;
  const left = ms.blocks.filter((b) => b.out).length;
  for (const block of ms.blocks) {
    if (block.out && block === ms.blocks.find((b) => b.out)) {
      list.append(
        el("li", {
          className: "outline-divider",
          textContent: `Left out of this cut · ${left}`,
          title: "Not exported (as text or audio) or counted. Drag a page above this line, or press + on it, to put it back.",
        }),
      );
    }
    const live = block.variants.find((v) => v.id === block.live);
    const toggle = el("button", {
      type: "button",
      className: "ghost out-toggle",
      textContent: block.out ? "+" : "−",
      title: block.out ? "Put back in this cut" : "Leave out of this cut",
    });
    toggle.onclick = (e) => {
      e.stopPropagation();
      if (ms) op(api.setBlockOut(ms.id, block.id, !block.out));
    };
    const li = el(
      "li",
      { draggable: true, className: [doc?.kind === "block" && doc.blockId === block.id ? "active" : "", block.out ? "out" : ""].join(" ").trim() },
      el("span", { className: "grip", textContent: "⋮⋮" }),
      el("span", { className: "name", textContent: block.title }),
      el("span", {
        className: "meta",
        textContent: block.variants.length > 1 ? `${block.variants.length} · ${live?.label ?? ""}` : "",
        title: "versions · live version",
      }),
      toggle,
    );
    li.dataset.id = block.id;
    clickable(li, () => openBlock(block.id));
    li.ondragstart = (e) => {
      e.dataTransfer!.setData("text/plain", block.id);
      li.classList.add("dragging");
    };
    li.ondragend = () => li.classList.remove("dragging");
    li.ondragover = (e) => {
      e.preventDefault();
      const r = li.getBoundingClientRect();
      const below = e.clientY > r.top + r.height / 2;
      li.classList.toggle("drop-above", !below);
      li.classList.toggle("drop-below", below);
    };
    li.ondragleave = () => li.classList.remove("drop-above", "drop-below");
    li.ondrop = (e) => {
      e.preventDefault();
      const below = li.classList.contains("drop-below");
      li.classList.remove("drop-above", "drop-below");
      moveBlock(e.dataTransfer!.getData("text/plain"), block.id, below);
    };
    list.append(li);
  }
  list.querySelector("li.active")?.scrollIntoView({ block: "nearest" });
}

/** A new chapter after the one being edited, opened and ready to type. */
async function addChapter() {
  if (!ms) return;
  flushSaves();
  const script = ms.blocks.some((b) => b.title.startsWith("Act ") || b.title.startsWith("Scene "));
  const word = script ? "Scene" : "Chapter";
  const n = ms.blocks.filter((b) => !b.raw).length + 1;
  const before = new Set(ms.blocks.map((b) => b.id));
  const after = doc?.kind === "block" ? doc.blockId : null;
  ms = await api.addBlock(ms.id, `${word} ${n}`, after);
  const added = ms.blocks.find((b) => !before.has(b.id));
  if (added) openBlock(added.id);
  editor.focusEnd();
  loadManuscripts().catch(() => {});
}

/** Drop a page next to another; dropped among the pages left out, it's left out too, and back among the rest, it's put back. */
async function moveBlock(id: string, targetId: string, after: boolean) {
  if (!ms || id === targetId) return;
  const [moved, target] = [id, targetId].map((x) => ms!.blocks.find((b) => b.id === x));
  if (!moved || !target) return;
  if (moved.out !== target.out && !(await op(api.setBlockOut(ms.id, id, target.out)))) return;
  const order = ms.blocks.map((b) => b.id).filter((b) => b !== id);
  order.splice(order.indexOf(targetId) + (after ? 1 : 0), 0, id);
  op(api.setOrder(ms.id, order));
}

function renderVariants() {
  const box = $("variants");
  box.replaceChildren();
  const block = currentBlock();
  const viewing = currentVariant();
  if (!block || !viewing) return;
  /** Still looking at this exact take? (Guards every button below.) */
  const here = () => {
    const d = doc;
    const ok = d?.kind === "block" && d.blockId === block.id && d.variantId === viewing.id;
    if (!ok) renderVariants();
    return ok;
  };
  for (const v of block.variants) {
    const chip = el("button", {
      type: "button",
      className: `chip${v.id === viewing.id ? " viewing" : ""}${v.id === block.live ? " live" : ""}${v.id === compareId ? " compare" : ""}`,
      textContent: v.author ? `🤖 ${v.label}` : v.label,
      title: `${v.author ? `Written by ${v.author}. ` : ""}${v.id === block.live ? "Live in the current cut. " : ""}Click to edit, double-click to rename.`,
    });
    chip.onclick = () => openBlock(block.id, v.id);
    chip.ondblclick = () => renameVariant(chip, v);
    box.append(chip);
  }
  const fork = el("button", { type: "button", className: "ghost", textContent: "+ Fork", title: "Copy this version to try something different" });
  fork.onclick = () => here() && addTakeAfter(block, viewing, `take ${block.variants.length + 1}`, editor.text);
  box.append(fork);
  if (viewing.id !== block.live) {
    const live = el("button", { type: "button", className: "ghost", textContent: "★ Make live", title: "Use this version in the current cut" });
    live.onclick = () => here() && op(api.setLive(ms!.id, viewing.id));
    box.append(live);
  }
  if (block.variants.length > 1) {
    box.append(
      confirmButton("Delete", async () => {
        if (!here() || !ms) return;
        const msId = ms.id;
        await flushSaves();
        if (compareId === viewing.id) compareId = null;
        if (!(await op(api.deleteTake(msId, viewing.id)))) return;
        openBlock(block.id);
        toast(`Deleted “${viewing.label}” from ${block.title}.`, false, {
          label: "Undo",
          run: async () => {
            if (await op(api.restoreTake(msId, viewing.id))) {
              if (ms?.id === msId) openBlock(block.id, viewing.id);
              toast(`Restored “${viewing.label}”.`);
            }
          },
        });
      }),
    );
  }
}

/** Add a take right after `after`, open it, and compare it against `after`. */
async function addTakeAfter(block: Block, after: Variant, label: string, text: string) {
  if (!ms) return;
  await flushSaves();
  const before = new Set(block.variants.map((v) => v.id));
  const m = await op(api.addTake(ms.id, block.id, after.id, label, text));
  const added = m?.blocks.find((b) => b.id === block.id)?.variants.find((v) => !before.has(v.id));
  if (!added) return;
  compareId = after.id;
  openBlock(block.id, added.id);
  editor.focus();
}

function renameVariant(chip: HTMLElement, v: Variant) {
  const input = el("input", { value: v.label, className: "chip-edit" });
  chip.replaceWith(input);
  input.select();
  let finished = false;
  const done = (save: boolean) => {
    if (finished) return;
    finished = true;
    const label = input.value.trim();
    if (save && label && label !== v.label && ms) op(api.renameTake(ms.id, v.id, label));
    else renderVariants();
  };
  input.onkeydown = (e) => {
    if (e.key === "Enter") done(true);
    if (e.key === "Escape") done(false);
  };
  input.onblur = () => done(true);
}

function renderCompare() {
  const sel = $<HTMLSelectElement>("sel-compare");
  sel.replaceChildren(el("option", { value: "", textContent: "Nothing" }));
  const block = currentBlock();
  const viewing = currentVariant();
  for (const v of block?.variants ?? []) {
    if (v.id === viewing?.id) continue;
    sel.append(el("option", { value: v.id, textContent: v.label, selected: !compareSnap && v.id === compareId }));
  }
  if (compareSnap) sel.append(el("option", { value: "snap", textContent: `🕘 ${compareSnap.label}`, selected: true }));
  $<HTMLButtonElement>("btn-ab").disabled = !compareSource();
}

// ---------- history ----------

let historyOn = false;
let history: HistoryEntry[] = [];
let lastSnapAt = Date.now();

function when(secs: number): string {
  const ago = Date.now() / 1000 - secs;
  if (ago < 60) return "just now";
  if (ago < 3600) return `${Math.floor(ago / 60)} min ago`;
  const d = new Date(secs * 1000);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  if (ago < 86400 && d.getDate() === new Date().getDate()) return `today ${time}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

let historySeq = 0;
async function loadHistory() {
  const seq = ++historySeq;
  const list = $("history-list");
  try {
    historyOn = settings.vault ? (await api.historyStatus()).enabled : false;
  } catch {
    historyOn = false;
  }
  $("history-off").hidden = historyOn;
  $("history-on").hidden = !historyOn;
  if (!historyOn || !doc) return list.replaceChildren();
  try {
    const d = doc;
    const entries =
      d.kind === "note" ? await api.noteHistory(d.path) : ms ? await api.takeHistory(ms.id, d.variantId) : [];
    if (seq !== historySeq) return;
    history = entries;
  } catch (e) {
    history = [];
    fail(e);
  }
  renderHistory();
}

function renderHistory() {
  const list = $("history-list");
  list.replaceChildren();
  if (!history.length) {
    list.append(el("li", { className: "hint", textContent: "No saved versions of this yet. Save one, or keep writing and autosave will." }));
    return;
  }
  for (const h of history) {
    const label = h.snapshot.message;
    const same = h.text === editor.text;
    const li = el(
      "li",
      { className: `${h.snapshot.auto ? "auto" : ""}${compareSnap?.text === h.text ? " active" : ""}` },
      el(
        "div",
        { className: "when" },
        el("span", { className: "msg", textContent: label }),
        el("span", { className: "meta", textContent: when(h.snapshot.time) }),
      ),
    );
    if (h.label || same) {
      li.append(el("span", { className: "meta", textContent: [h.label, same ? "same as now" : ""].filter(Boolean).join(" · ") }));
    }
    const actions = el("div", { className: "actions" });
    const restore = el("button", { type: "button", textContent: doc?.kind === "block" ? "Restore as new take" : "Restore" });
    restore.onclick = (e) => {
      e.stopPropagation();
      restoreEntry(h);
    };
    actions.append(
      restore,
      confirmButton("Roll back whole vault", () => rollBack(h.snapshot.id, h.snapshot.message), "ghost danger"),
    );
    li.append(actions);
    clickable(li, () => {
      compareSnap = compareSnap?.text === h.text ? null : { label: `${label}, ${when(h.snapshot.time)}`, text: h.text };
      if (ghost === 0) {
        $<HTMLInputElement>("rng-ghost").value = "60";
        ghost = 60;
      }
      renderCompare();
      renderHistory();
      setGhost(ghost);
    });
    list.append(li);
  }
}

function restoreEntry(h: HistoryEntry) {
  const stamp = when(h.snapshot.time);
  if (doc?.kind === "block") {
    const block = currentBlock();
    const current = currentVariant();
    if (!block || !current) return;
    addTakeAfter(block, current, `restored ${stamp}`, h.text).then(() =>
      toast("Restored as a new take. Your current version is untouched."),
    );
  } else if (doc?.kind === "note") {
    // One undoable edit, so ⌘Z brings the present back.
    editor.view.dispatch({ changes: { from: 0, to: editor.view.state.doc.length, insert: h.text } });
    compareSnap = null;
    renderCompare();
    setGhost(ghost);
    toast(`Restored the version from ${stamp}. ⌘Z undoes it.`);
  }
}

async function rollBack(id: string, message: string) {
  await flushSaves();
  narrator.stop();
  try {
    await api.historyRestoreAll(id);
    toast(`Rolled the vault back to “${message}”. The version before is saved in history.`);
    const reopen = doc;
    if (ms) ms = await api.loadManuscript(ms.id);
    await Promise.all([loadNotes(), loadManuscripts()]);
    if (reopen?.kind === "note") await openNote(reopen.path);
    else if (reopen?.kind === "block" && ms) openBlock(reopen.blockId);
  } catch (e) {
    fail(e);
  }
}

async function takeSnapshot(message: string, quiet = false) {
  await flushSaves();
  try {
    const snap = await api.historySnapshot(message);
    lastSnapAt = Date.now();
    dirty = false;
    if (!quiet) toast(snap ? `Saved version “${snap.message}”.` : "Nothing changed since the last version.");
    loadHistory();
  } catch (e) {
    if (!quiet) fail(e);
  }
}

// Autosave: a snapshot after 10 minutes of edits.
window.setInterval(() => {
  if (historyOn && settings.history_autosave && dirty && Date.now() - lastSnapAt > 10 * 60_000) takeSnapshot("", true);
}, 60_000);

/** Move a note to the trash, with Undo. */
/** Delete a page with every version, open its neighbour, and offer Undo. */
async function deletePage(blockId: string) {
  if (!ms) return;
  const msId = ms.id;
  const i = ms.blocks.findIndex((b) => b.id === blockId);
  const title = ms.blocks[i]?.title ?? "page";
  await flushSaves();
  const m = await op(api.deleteBlock(msId, blockId));
  if (!m) return;
  const next = m.blocks[Math.min(i, m.blocks.length - 1)];
  if (next) openBlock(next.id);
  loadManuscripts().catch(() => {});
  toast(`Deleted “${title}”.`, false, {
    label: "Undo",
    run: async () => {
      if (!(await op(api.restoreBlock(msId, blockId)))) return;
      if (ms?.id === msId) openBlock(blockId);
      loadManuscripts().catch(() => {});
      toast(`Restored “${title}”.`);
    },
  });
}

async function deleteNote(path: string) {
  await flushSaves();
  pendingNote = null;
  try {
    const id = await api.deleteNote(path);
    if (doc?.kind === "note" && doc.path === path) {
      doc = null;
      setMode("none");
      $("empty-msg").textContent = "Pick a manuscript or a note.";
      $("btn-empty-vault").hidden = true;
    }
    await Promise.all([loadNotes(), loadWorld()]).catch(() => {});
    refreshWorldViews().catch(() => {});
    const name = path.split("/").pop()!.replace(/\.md$/, "");
    toast(`Deleted “${name}”.`, false, {
      label: "Undo",
      run: async () => {
        try {
          const back = await api.restoreNote(id);
          await Promise.all([loadNotes(), loadWorld()]).catch(() => {});
          refreshWorldViews().catch(() => {});
          toast(`Restored “${name}”.`);
          if (!document.body.dataset.mode || document.body.dataset.mode === "none") openNote(back).catch(fail);
        } catch (e) {
          fail(e);
        }
      },
    });
  } catch (e) {
    fail(e);
  }
}

// ---------- world ----------

let worldEntities: Entity[] = [];
let worldIssues: { path: string; message: string }[] = [];

async function loadWorld() {
  if (!settings.vault) return;
  const [es, g] = await Promise.all([api.worldEntities(), api.worldGraph()]);
  worldEntities = es;
  worldIssues = g.issues;
  renderWorld();
}

function renderWorld() {
  const box = $("world-groups");
  box.replaceChildren();
  const known = KINDS.filter((k) => k.kind !== "scene").map((k) => k.kind);
  const kinds = [...known, ...new Set(worldEntities.map((e) => e.kind).filter((k) => !known.includes(k)))];
  for (const kind of kinds) {
    const items = worldEntities.filter((e) => e.kind === kind).sort((a, b) => a.name.localeCompare(b.name));
    const meta = KINDS.find((k) => k.kind === kind);
    if (!items.length && !meta) continue;
    const group = el("div", { className: "world-group" });
    const add = el("button", { type: "button", className: "ghost", textContent: "+", title: `New ${meta?.label.toLowerCase() ?? kind}` });
    add.onclick = () => (kind === "event" ? newEvent(lastEventPath(), null) : newEntry(kind));
    const head = el("div", { className: "world-group-head" }, swatch(kind), el("span", { className: "count", textContent: `${meta?.plural ?? kind} ${items.length || ""}` }));
    if (meta) head.append(add);
    group.append(head);
    const list = el("ul", { className: "list" });
    for (const e of items) {
      const li = el(
        "li",
        { className: doc?.kind === "note" && doc.path === e.path ? "active" : "", title: e.summary || e.path },
        el("span", { className: "name", textContent: e.name }),
      );
      if (e.status === "draft") li.append(el("span", { className: "badge draft", textContent: "draft" }));
      clickable(li, () => openNote(e.path).catch(fail));
      list.append(li);
    }
    group.append(list);
    box.append(group);
  }
  const issues = $<HTMLDetailsElement>("world-issues");
  issues.hidden = !worldIssues.length;
  issues.querySelector("summary")!.textContent = `⚠ ${worldIssues.length} to look at`;
  const ul = issues.querySelector("ul")!;
  ul.replaceChildren();
  for (const i of worldIssues) {
    const li = el("li", { textContent: i.message });
    const missing = i.message.match(/^\[\[(.+?)\]\] is linked/);
    if (missing) {
      li.title = "Create this note";
      clickable(li, () => newEntry("character", missing[1]));
    } else if (i.path) {
      clickable(li, () => openNote(i.path).catch(fail));
    }
    ul.append(li);
  }
}

/** Ask for a new entry's name (and type / time). */
function askWorld(opts: { title: string; kind?: string; name?: string; event?: boolean; timeline?: boolean }): Promise<{ kind: string; name: string; when: string; timeline: string } | null> {
  const dlg = $<HTMLDialogElement>("dlg-world");
  $("world-dlg-title").textContent = opts.title;
  $("world-kind-field").hidden = !!opts.event;
  $("world-when-field").hidden = !opts.event;
  $("world-timeline-field").hidden = !opts.timeline;
  const tl = $<HTMLInputElement>("world-timeline");
  tl.value = "";
  const kind = $<HTMLSelectElement>("world-kind");
  if (opts.kind) kind.value = opts.kind;
  const name = $<HTMLInputElement>("world-name");
  name.value = opts.name ?? "";
  name.placeholder = opts.event ? "The van doesn't brake" : "June";
  $<HTMLInputElement>("world-when").value = "";
  dlg.returnValue = "";
  dlg.showModal();
  (opts.timeline ? tl : name).focus();
  // Enter means Create (the form's first button is Cancel).
  name.onkeydown = tl.onkeydown = (e) => {
    if (e.key === "Enter") {
      e.preventDefault();
      dlg.close("ok");
    }
  };
  return new Promise((resolve) => {
    dlg.onclose = () => {
      const n = name.value.trim();
      const t = tl.value.trim();
      const ok = dlg.returnValue === "ok" && n && (!opts.timeline || t);
      resolve(ok ? { kind: opts.event ? "event" : kind.value, name: n, when: $<HTMLInputElement>("world-when").value.trim(), timeline: t } : null);
    };
  });
}

async function newEntry(kind: string, name?: string) {
  const r = await askWorld({ title: "New world entry", kind, name });
  if (!r) return;
  try {
    const path = await api.worldAdd(r.kind, r.name);
    await loadWorld();
    showTab("world");
    await openNote(path);
  } catch (e) {
    fail(e);
  }
}

let timelineCache: { path: string; lane: number; timeline: string; after: string | null }[] = [];
const lastEventPath = () => [...timelineCache].reverse().find((e) => e.lane === 0 && e.timeline === "")?.path ?? null;
/** What the timeline is read against ("" or a manuscript id, plus "\n" and a cut), and how it's shown. */
let timelineAgainst = "";
let timelineMode: "list" | "chart" = "list";

async function newEvent(after: string | null, before: string | null, scene: [string, string] | null = null) {
  const r = await askWorld({ title: scene ? "New event in this scene" : "New event", event: true });
  if (!r) return;
  try {
    await api.worldInsertEvent(r.name, after, before, r.when || null, scene);
    await refreshWorldViews();
    toast(`Added “${r.name}” to the timeline.`);
  } catch (e) {
    fail(e);
  }
}

/** A new first event for the timeline `before` starts (named `timeline`), or with neither, a whole new timeline. */
async function startTimeline(before: string | null, timeline: string | null) {
  const fresh = before === null;
  const r = await askWorld({ title: fresh ? "New timeline" : `New first event: ${timeline || "Main timeline"}`, event: true, timeline: fresh });
  if (!r) return;
  const after = before ? timelineCache.find((e) => e.path === before)?.after ?? null : null;
  try {
    await api.worldInsertEvent(r.name, after, before, r.when || null, null, fresh ? r.timeline : timeline);
    await refreshWorldViews();
    toast(`Added “${r.name}” to the timeline.`);
  } catch (e) {
    fail(e);
  }
}

async function openGraph() {
  await flushSaves();
  narrator.stop();
  doc = null;
  setMode("graph");
  try {
    const g = await api.worldGraph();
    worldIssues = g.issues;
    renderGraph($("graph-view"), g, {
      open: (n) => (n.kind === "scene" && n.manuscript && n.block ? openScene(n.manuscript, n.block) : openNote(n.id).catch(fail)),
    });
  } catch (e) {
    fail(e);
  }
}

async function openTimeline() {
  await flushSaves();
  narrator.stop();
  doc = null;
  setMode("timeline");
  try {
    const tellings = await api.worldTellings().catch(() => []);
    // A telling that's gone (deleted manuscript or cut) falls back to each scene where it is.
    const [msId, cut] = timelineAgainst.split("\n");
    if (msId && !tellings.some((t) => t.id === msId && (!cut || t.cuts.includes(cut)))) timelineAgainst = "";
    const events = await (timelineAgainst ? api.worldTimeline(msId, cut ?? null) : api.worldTimeline());
    timelineCache = events;
    renderTimeline(
      $("timeline-view"),
      events,
      {
        open: (path) => openNote(path).catch(fail),
        openScene: (m, b) => openScene(m, b),
        insert: (after, before) => newEvent(after, before),
        start: (before, timeline) => startTimeline(before, timeline),
        move: (path, after, before) => api.worldMoveEvent(path, after, before).then(refreshWorldViews).catch(fail),
        remove: (path) => deleteNote(path),
      },
      (label, fn) => confirmButton(label, fn),
      {
        tellings,
        against: timelineAgainst,
        mode: timelineMode,
        setAgainst: (v) => {
          timelineAgainst = v;
          openTimeline();
        },
        setMode: (m) => {
          timelineMode = m;
          openTimeline();
        },
      },
    );
  } catch (e) {
    fail(e);
  }
}

/** Re-render whatever world view is showing, plus the side panels. */
/** World → Suggest…: names and places found in a manuscript by plain rules, added as drafts. */
async function openSuggest() {
  if (!settings.vault) return toast("Open a vault first.");
  const all = await api.listManuscripts();
  if (!all.length) return toast("Import or write a manuscript first; suggestions come from its text.");
  const dlg = $<HTMLDialogElement>("dlg-suggest");
  const sel = $<HTMLSelectElement>("suggest-ms");
  const list = $("suggest-list");
  const timeline = $<HTMLInputElement>("suggest-timeline");
  const addBtn = $<HTMLButtonElement>("btn-suggest-add");
  sel.replaceChildren(...all.map((m) => el("option", { value: m.id, textContent: m.title, selected: m.id === ms?.id })));
  timeline.checked = false;
  let found: WorldSuggestions | null = null;
  const boxes = new Map<HTMLInputElement, WorldSuggestion>();
  const count = () => {
    const n = [...boxes.keys()].filter((b) => b.checked).length;
    addBtn.textContent = n ? `Add ${n} as draft${n === 1 ? "" : "s"}` : timeline.checked ? "Add the timeline" : "Add as drafts";
    addBtn.disabled = !n && !timeline.checked;
  };
  timeline.onchange = count;
  const render = async () => {
    boxes.clear();
    list.replaceChildren(el("p", { className: "hint", textContent: "Reading…" }));
    found = await api.worldSuggest(sel.value);
    $("suggest-mode").textContent = found.screenplay
      ? "Read as a screenplay: characters from the names above dialogue, places from scene headings."
      : "Read as prose: names that recur away from the start of a sentence, who “said” what, and places. Names like “the old man” aren't found; add those yourself.";
    list.replaceChildren();
    for (const [label, items] of [["Characters", found.characters], ["Locations", found.locations]] as const) {
      list.append(el("h3", { className: "dialog-section", textContent: `${label} (${items.length})` }));
      if (!items.length) list.append(el("p", { className: "hint", textContent: "Nothing new: everything found is already in World." }));
      for (const s of items) {
        const box = el("input", { type: "checkbox", checked: true });
        box.onchange = count;
        boxes.set(box, s);
        const where = s.scenes.length > 3 ? `${s.scenes.length} scenes` : s.scenes.join(", ");
        const meta = [`${s.mentions}×`, s.why, s.aliases.length ? `also “${s.aliases.join("”, “")}”` : "", where].filter(Boolean).join(" · ");
        list.append(el("label", { className: "suggest-item" }, box, el("span", { className: "name", textContent: s.name }), el("span", { className: "meta", textContent: meta })));
      }
    }
    count();
  };
  sel.onchange = () => render().catch(fail);
  dlg.returnValue = "";
  dlg.showModal();
  await render();
  const ok = await new Promise<boolean>((resolve) => (dlg.onclose = () => resolve(dlg.returnValue === "ok")));
  const result = found as WorldSuggestions | null; // set inside render(), which TS can't see
  if (!ok || !result) return;
  const picks = [...boxes].filter(([b]) => b.checked).map(([, s]) => s);
  const created = await api.worldSuggestApply(result.manuscript, picks, timeline.checked);
  await loadWorld();
  await refreshWorldViews();
  toast(created.length ? `Added ${created.length} draft${created.length === 1 ? "" : "s"} to World. Keep, edit or delete each one there.` : "Nothing new to add.");
}

/** World → Import CSV…: type,name,aliases,description rows into world notes. */
async function importWorldCsv() {
  if (!settings.vault) return toast("Open a vault first.");
  const picked = await open({ multiple: false, title: "Import characters, places and things", filters: [{ name: "CSV", extensions: ["csv"] }] });
  if (typeof picked !== "string") return;
  const r = await api.worldImportCsv(picked);
  await loadWorld();
  await refreshWorldViews();
  const file = picked.split(/[\\/]/).pop();
  if (r.skipped.length) toast(`Imported ${r.created.length} from ${file}. Skipped ${r.skipped.length}: ${r.skipped[0]}`, true);
  else toast(`Imported ${r.created.length} from ${file}.`);
}

async function refreshWorldViews() {
  const mode = document.body.dataset.mode;
  if (mode === "timeline") await openTimeline();
  else if (mode === "graph") await openGraph();
  else {
    timelineCache = await api.worldTimeline().catch(() => timelineCache);
    loadSceneCast();
    loadConnections();
  }
  if (!$("pane-world").hidden) loadWorld().catch(() => {});
}

async function openScene(msId: string, blockId: string) {
  if (ms?.id !== msId) await openManuscript(msId);
  showTab("manuscripts");
  openBlock(blockId);
}

let castSeq = 0;
async function loadSceneCast() {
  const box = $("scene-cast");
  const d = doc;
  if (d?.kind !== "block" || !ms) return box.replaceChildren();
  const seq = ++castSeq;
  try {
    const info = await api.worldScene(ms.id, d.blockId);
    if (seq !== castSeq) return;
    box.replaceChildren();
    if (!info.present.length && !info.events.length) {
      box.append(el("p", { className: "hint", textContent: "No world entries are named here yet. Add characters and places in the World tab; their names and aliases are found automatically." }));
      return;
    }
    const cast = el("div", { className: "cast" });
    for (const [e, n] of info.present) {
      const chip = el("button", { type: "button", className: "cast-chip", title: e.summary || e.name });
      chip.append(swatch(e.kind), document.createTextNode(e.name), el("span", { className: "n", textContent: `×${n}` }));
      chip.onclick = () => openNote(e.path).catch(fail);
      cast.append(chip);
    }
    box.append(cast);
    if (info.events.length) {
      box.append(el("p", { className: "hint", textContent: "Events here:" }));
      const ul = el("ul", { className: "list" });
      for (const e of info.events) {
        const li = el("li", {}, swatch("event"), el("span", { className: "name", textContent: e.name }));
        clickable(li, () => openNote(e.path).catch(fail));
        ul.append(li);
      }
      box.append(ul);
    }
  } catch (e) {
    fail(e);
  }
}

let connSeq = 0;
async function loadConnections() {
  const section = $("connections-section");
  const box = $("connections");
  const d = doc;
  if (d?.kind !== "note") {
    section.hidden = true;
    return;
  }
  const seq = ++connSeq;
  const info = await api.worldEntity(d.path).catch(() => null);
  if (seq !== connSeq) return;
  section.hidden = !info;
  box.replaceChildren();
  if (!info) return;
  const group = (label: string, items: HTMLElement[]) => {
    if (!items.length) return;
    const ul = el("ul", { className: "list" });
    ul.append(...items);
    box.append(el("div", { className: "conn-group" }, el("div", { className: "conn-label", textContent: label }), ul));
  };
  const item = (text: string, kind: string, open: () => void, meta = "") => {
    const li = el("li", {}, swatch(kind), el("span", { className: "name", textContent: text }));
    if (meta) li.append(el("span", { className: "meta", textContent: meta }));
    clickable(li, open);
    return li;
  };
  const kindOf = (path: string) => worldEntities.find((e) => e.path === path)?.kind ?? "note";
  group(
    "Relationships",
    info.relations.map(([path, name, kind, out]) => item(out ? `${kind} ${name}` : `${name} ${kind} this`, kindOf(path), () => openNote(path).catch(fail))),
  );
  group(
    `Appears in ${info.appears_in.length} scene${info.appears_in.length === 1 ? "" : "s"}`,
    info.appears_in.map(([m, b, label, n]) => item(label.split(" · ").pop()!, "scene", () => openScene(m, b), n > 1 ? `×${n}` : "")),
  );
  group(
    "Linked from",
    info.backlinks.map(([path, name]) => item(name, kindOf(path), () => openNote(path).catch(fail))),
  );
  if (!box.childElementCount) box.append(el("p", { className: "hint", textContent: "Nothing connects here yet. Link other entries with [[Name]], or add relationships." }));
}

// ---------- other writers ----------

/** Swap the editor's text for what's on disk, keeping the cursor roughly in place. */
function replaceEditorText(text: string) {
  const view = editor.view;
  const head = Math.min(view.state.selection.main.head, text.length);
  view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: head } });
}

/** Files changed on disk: agents, sync tools, another editor, or us. */
const worldChanged = debounce(() => refreshWorldViews().catch(() => {}), 400);

async function onVaultChanged(paths: string[]) {
  if (paths.some((p) => p.startsWith(".writepad/library/")) && !$("pane-library").hidden) loadLibrary().catch(() => {});
  if (document.body.dataset.mode === "library" && paths.some((p) => p.startsWith(".writepad/manuscripts/"))) refreshLibrary();
  if (paths.some((p) => p.startsWith("world/") || p.startsWith(".writepad/manuscripts/"))) worldChanged();
  const notesTouched = paths.some((p) => p.endsWith(".md") && !p.startsWith(".writepad/"));
  if (notesTouched) {
    await loadNotes().catch(() => {});
    const d = doc;
    if (d?.kind === "note" && paths.includes(d.path) && !pendingNote) {
      const disk = await api.readNote(d.path).catch(() => null);
      if (disk !== null && disk !== editor.text) {
        replaceEditorText(disk);
        toast(`${d.path} changed on disk. Showing the new version (⌘Z to undo).`);
      }
    }
  }
  if (!ms) {
    if (paths.some((p) => p.startsWith(".writepad/manuscripts/"))) loadManuscripts().catch(() => {});
    return;
  }
  const mine = `.writepad/manuscripts/${ms.id}/`;
  if (!paths.some((p) => p.startsWith(mine))) return;
  // An agent left a comment: show it, unless the writer is typing one.
  if (paths.includes(`${mine}comments.json`) && !editingComment) {
    await flushSaves();
    await loadComments();
  }
  const fresh = await api.loadManuscript(ms.id).catch(() => null);
  if (!fresh || !ms) return;
  const before = new Map(ms.blocks.flatMap((b) => b.variants.map((v) => [v.id, v] as const)));
  const added = fresh.blocks.flatMap((b) => b.variants.map((v) => ({ b, v }))).filter(({ v }) => !before.has(v.id));
  const viewing = currentVariant();
  const viewingFresh = viewing && fresh.blocks.flatMap((b) => b.variants).find((v) => v.id === viewing.id);
  const textChanged = viewingFresh && !pendingTake && viewingFresh.text !== editor.text;
  const anyText = fresh.blocks.some((b) => b.variants.some((v) => before.get(v.id)?.text !== v.text));
  if (fresh.rev === ms.rev && !anyText) return;
  applyMs(fresh);
  if (textChanged && viewingFresh) {
    replaceEditorText(viewingFresh.text);
    toast(`“${viewingFresh.label}” was changed${viewingFresh.author ? ` by ${viewingFresh.author}` : ""} (⌘Z to undo).`);
  }
  for (const { b, v } of added.slice(0, 3)) {
    if (v.author) toast(`${v.author} added “${v.label}” to ${b.title}.`);
  }
}

listen<string[]>("vault-changed", (e) => onVaultChanged(e.payload));

// Edit ▸ Undo/Redo (⌘Z / ⇧⌘Z). Text fields keep the browser's own undo;
// otherwise it goes to the editor's history.
/** The help page for what's on screen. */
function helpTopic(): string {
  switch (document.body.dataset.mode) {
    case "block":
      return isScreenplay(editor.text) ? "screenplays" : "the-writing-desk";
    case "note":
      return "notes";
    case "graph":
      return "your-story-world";
    case "timeline":
      return "timeline";
    case "library":
      return "library-and-targets";
    default:
      return "";
  }
}

function openHelp(page: string) {
  api.openHelp(page).catch(fail);
}

// Any "?" with data-help opens that topic; the top bar's (empty) one follows what's on screen.
document.addEventListener("click", (e) => {
  const b = (e.target as HTMLElement).closest<HTMLElement>("[data-help]");
  if (!b) return;
  e.preventDefault();
  e.stopPropagation();
  openHelp(b.dataset.help || helpTopic());
});

listen<string>("menu", (e) => {
  if (e.payload === "help") return openHelp(helpTopic());
  if (e.payload === "focus") return setFocus(!document.body.classList.contains("focus"));
  const active = document.activeElement as HTMLElement | null;
  const inField = active && (active.tagName === "INPUT" || active.tagName === "TEXTAREA") && !active.closest(".cm-editor");
  if (inField) {
    document.execCommand(e.payload);
    return;
  }
  if (document.body.dataset.mode !== "note" && document.body.dataset.mode !== "block") return;
  if (e.payload === "undo") editor.undo();
  else editor.redo();
  editor.focus();
});

// ---------- export ----------

/** The current cut of the open manuscript, or the open note, as a book. */
function currentBook(): Book | null {
  if (ms) {
    return {
      title: ms.title,
      author: settings.author,
      chapters: ms.blocks.filter((b) => !b.out).map((b) => ({
        title: b.raw ? null : b.title,
        text: (b.variants.find((v) => v.id === b.live) ?? b.variants[b.variants.length - 1])?.text ?? "",
      })),
    };
  }
  if (doc?.kind === "note") {
    const name = doc.path.replace(/\.md$/, "").split("/").pop()!;
    return { title: name, author: settings.author, chapters: [{ title: null, text: editor.text }] };
  }
  return null;
}

let exportCancelled = false;

/**
 * The cast for the whole manuscript or note, not just the open block: what's
 * saved, plus a voice for anyone who hasn't been cast yet. New voices are
 * saved, so later table reads sound like the exported audio.
 */
async function castForExport(text: string): Promise<Record<string, string>> {
  const scope = currentCastScope();
  if (!scope) return {};
  let full: Record<string, string> = scope === castScope ? cast : await api.getCast(scope).catch(() => ({}));
  const names = characters(text).map((c) => c.name);
  if (voices.length && names.some((n) => !full[n])) {
    full = { ...autoCast(names, voices, [settings.voice, settings.italic_voice], full, pronounHints(text, names)), __narrator: full["__narrator"] ?? settings.voice };
    await api.setCast(scope, full).catch(fail);
    if (scope === castScope) cast = full;
  }
  return full;
}

async function exportAudio(book: Book, dir: string) {
  const progress = $("export-progress");
  const chapters = book.chapters.filter((c) => sentences(c.text).length);
  const started = Date.now();
  // A screenplay exports as its table read: headings and action in the
  // narrator's voice, each character's lines in their own.
  const whole = book.chapters.map((c) => c.text).join("\n");
  const script = isScreenplay(whole);
  const castMap = script ? await castForExport(whole) : {};
  const voicesFor = { narrator: castMap["__narrator"] ?? settings.voice, cast: castMap, fallback: settings.voice };
  const safe = (s: string) => s.replace(/[\/:*?"<>|]+/g, "-").slice(0, 60).trim();
  for (let i = 0; i < chapters.length; i++) {
    if (exportCancelled) {
      progress.textContent = `Stopped after ${i} of ${chapters.length} chapters.`;
      return;
    }
    const c = chapters[i];
    const name = c.title ?? (i === 0 ? "Opening" : `Part ${i + 1}`);
    progress.textContent = `${script ? "Performing" : "Narrating"} ${i + 1} of ${chapters.length}: ${name}…`;
    const segs: AudioSegment[] = [];
    if (c.title) segs.push({ text: `${speakable(c.title)}.`, voice: voicesFor.narrator, pause: 1.0 });
    if (script) {
      for (const s of tableRead(c.text, 0, c.text.length, voicesFor)) {
        segs.push({ text: s.speak, voice: s.voice ?? voicesFor.narrator, pause: s.pauseAfter ?? 0 });
      }
    } else {
      for (const s of sentences(c.text)) {
        segs.push({
          text: s.speak,
          voice: s.italic ? settings.italic_voice : settings.voice,
          pause: s.paraEnd ? settings.paragraph_pause : 0,
        });
      }
    }
    const file = `${dir}/${String(i + 1).padStart(2, "0")} - ${safe(name)}.mp3`;
    await api.exportAudioChapter(segs, settings.speed, file, name, book.title, book.author);
  }
  const mins = Math.round((Date.now() - started) / 60_000);
  progress.textContent = `Done: ${chapters.length} MP3 files${mins ? ` in ${mins} min` : ""}.`;
  toast(`Audiobook saved to ${dir}`);
}

async function runExport() {
  const book = currentBook();
  if (!book) return toast("Open a manuscript or a note to export.");
  await flushSaves();
  const format = (document.querySelector<HTMLInputElement>('input[name="fmt"]:checked')?.value ?? "epub");
  settings.author = $<HTMLInputElement>("export-author").value.trim();
  book.author = settings.author;
  api.saveSettings(settings).catch(fail);
  const progress = $("export-progress");
  const button = $<HTMLButtonElement>("btn-do-export");
  button.disabled = true;
  exportCancelled = false;
  try {
    if (format === "mp3") {
      const dir = await open({ directory: true, title: "Choose a folder for the audiobook" });
      if (typeof dir !== "string") return;
      await exportAudio(book, dir);
    } else {
      const names: Record<string, string> = { md: "Markdown", txt: "Text", epub: "EPUB", pdf: "PDF", docx: "Word", odt: "OpenDocument", rtf: "Rich Text", html: "Web page" };
      const path = await save({ defaultPath: `${book.title}.${format}`, filters: [{ name: names[format], extensions: [format] }] });
      if (!path) return;
      progress.textContent = format === "pdf" ? "Typesetting…" : "Writing…";
      await api.exportBook(book, format, path);
      progress.textContent = `Saved ${path}`;
      toast(`Exported ${path.split("/").pop()}`);
    }
  } catch (e) {
    progress.textContent = "";
    fail(e);
  } finally {
    button.disabled = false;
  }
}

/** Is this cut the current one: the same pages in, in its order, with its live takes? */
function cutMatches(cut: { order: string[]; picks: Record<string, string> }) {
  if (!ms) return false;
  const kept = ms.blocks.filter((b) => !b.out);
  return cut.order.length === kept.length && kept.every((b, i) => cut.order[i] === b.id && (cut.picks[b.id] ?? b.live) === b.live);
}

function renderCuts() {
  const list = $("list-cuts");
  list.replaceChildren();
  if (!ms) return;
  if (!ms.cuts.length) {
    list.append(el("li", { className: "hint", textContent: "Save the pages that are in, their order and ★ live versions, as a named cut." }));
  }
  ms.cuts.forEach((cut, at) => {
    const li = el(
      "li",
      { className: cutMatches(cut) ? "active" : "", title: `Apply this cut: its ${cut.order.length} pages in its order, the rest left out` },
      el("span", { className: "name", textContent: cut.name }),
      el("span", { className: "meta", textContent: String(cut.order.length) }),
      Object.assign(confirmButton("×", () => deleteCut(cut, at)), { title: "Delete this cut (click twice; Undo brings it back). Pages are untouched." }),
    );
    clickable(li, () => applyCut(cut.name));
    list.append(li);
  });
}

async function applyCut(name: string) {
  if (!ms) return;
  await flushSaves();
  const m = await op(api.cutOp(ms.id, "apply", name));
  if (!m) return;
  const left = m.blocks.filter((b) => b.out).length;
  toast(`Applied cut “${name}”.${left ? ` ${left} page${left === 1 ? "" : "s"} left out.` : ""}`);
}

/** Deleting a cut never touches pages, and can be undone for a few seconds. */
async function deleteCut(cut: Cut, at: number) {
  if (!ms) return;
  const id = ms.id;
  if (!(await op(api.cutOp(id, "delete", cut.name)))) return;
  toast(`Deleted cut “${cut.name}”. Your pages are untouched.`, false, {
    label: "Undo",
    run: () => op(api.restoreCut(id, cut, at)).then((m) => m && toast(`Cut “${cut.name}” is back.`)),
  });
}

function renderNotes() {
  const list = $("list-notes");
  list.replaceChildren();
  for (const n of notes) {
    const li = el("li", {
      className: doc?.kind === "note" && doc.path === n.path ? "active" : "",
      title: n.path,
    }, el("span", { className: "name", textContent: n.path.replace(/\.md$/, "") }));
    clickable(li, () => openNote(n.path).catch(fail));
    list.append(li);
  }
  if (!notes.length) list.append(el("li", { className: "hint", textContent: "No notes yet." }));
}

function renderLexicon() {
  const list = $("lexicon");
  list.replaceChildren();
  for (const [word, say] of Object.entries(lexicon).sort(([a], [b]) => a.localeCompare(b))) {
    const li = el(
      "li",
      {},
      el("span", { className: "name", textContent: word }),
      el("span", { className: "meta", textContent: say }),
      Object.assign(
        confirmButton("×", () => {
          delete lexicon[word];
          saveLexicon();
        }),
        { title: "Remove this pronunciation (click twice)" },
      ),
    );
    li.title = "Hear it";
    clickable(li, () => narrator.play(sentences(word).map((s) => ({ ...s, highlight: false }))));
    list.append(li);
  }
}

function saveLexicon() {
  api.setLexicon(lexicon).catch(fail);
  narrator.clearRenders();
  renderLexicon();
}

function renderVoices() {
  for (const [id, current] of [["sel-voice", settings.voice], ["set-italic", settings.italic_voice]] as const) {
    const sel = $<HTMLSelectElement>(id);
    sel.replaceChildren();
    const list = voices.length ? voices : [current];
    for (const v of list) sel.append(el("option", { value: v, textContent: v, selected: v === current }));
  }
}

// ---------- loading ----------

async function loadManuscripts() {
  const list = $("list-manuscripts");
  list.replaceChildren();
  const all = await api.listManuscripts();
  for (const m of all) {
    const li = el(
      "li",
      {},
      el("span", { className: "name", textContent: m.title }),
      el("span", { className: "meta", textContent: `${m.blocks} blocks · ${m.variants} versions` }),
    );
    const remove = confirmButton("×", () => deleteManuscript(m.id, m.title).catch(fail));
    remove.title = "Delete this manuscript (click twice; Undo brings it back)";
    li.append(remove);
    clickable(li, () => openManuscript(m.id).catch(fail));
    list.append(li);
  }
  if (!all.length) list.append(el("li", { className: "hint", textContent: "Import a script, or every draft of one, to start." }));
}

/** Move a whole manuscript to the trash, with Undo. */
async function deleteManuscript(id: string, title: string) {
  await flushSaves();
  if (ms?.id === id) closeManuscript();
  await api.deleteManuscript(id);
  await loadManuscripts();
  refreshLibrary();
  toast(`Deleted “${title}”.`, false, {
    label: "Undo",
    run: () =>
      api
        .restoreManuscript(id)
        .then(loadManuscripts)
        .then(() => toast(`Restored “${title}”.`))
        .catch(fail),
  });
}

/** A blank manuscript or screenplay, opened and ready to type in. */
async function newManuscript() {
  const dlg = $<HTMLDialogElement>("dlg-new");
  const title = $<HTMLInputElement>("new-title");
  title.value = "";
  dlg.returnValue = "";
  dlg.showModal();
  title.focus();
  const ok = await new Promise<boolean>((resolve) => (dlg.onclose = () => resolve(dlg.returnValue === "ok")));
  if (!ok) return;
  const screenplay = dlg.querySelector<HTMLInputElement>('input[name="new-kind"]:checked')?.value === "script";
  const id = await api.createManuscript(title.value.trim() || (screenplay ? "Untitled screenplay" : "Untitled"), screenplay);
  await loadManuscripts();
  showTab("manuscripts");
  await openManuscript(id);
  editor.focusEnd();
}

/** Page, Draft or Code; remembered on this computer. */
function setViewMode(mode: ViewMode) {
  document.body.dataset.view = mode;
  editor.setViewMode(mode);
  document.querySelectorAll<HTMLButtonElement>(".view-switch button").forEach((b) => b.classList.toggle("active", b.dataset.view === mode));
  try {
    localStorage.setItem("wp-view", mode);
  } catch {}
}

async function loadNotes() {
  notes = await api.listNotes();
  renderNotes();
}

async function loadVault() {
  renderToday();
  const vault = settings.vault;
  $("btn-vault").textContent = vault ? vault.split(/[\\/]/).pop()! : "No vault";
  $("btn-vault").title = vault ?? "Open a vault folder";
  $("welcome").hidden = !!vault;
  $("empty-msg").hidden = !vault;
  if (!vault) {
    setMode("none");
    return;
  }
  closeManuscript();
  if (doc?.kind === "note") doc = null;
  setMode("none");
  $("empty-msg").textContent = "Pick a manuscript or a note. Import drafts to line up versions of a script.";
  $("btn-empty-vault").hidden = true;
  lexicon = await api.getLexicon();
  renderLexicon();
  await Promise.all([loadManuscripts(), loadNotes()]);
}

async function chooseVault() {
  const dir = await open({ directory: true, title: "Choose a vault folder" });
  if (typeof dir !== "string") return;
  await useVault(dir);
}

async function useVault(dir: string) {
  settings.vault = dir;
  await api.saveSettings(settings);
  $<HTMLInputElement>("set-vault").value = dir;
  await loadVault();
}

/** First run: the demo, opened at its "Start here" note. */
async function tryDemo() {
  await useVault(await api.openDemo());
  if (notes.some((n) => n.path === "Start here.md")) {
    showTab("notes");
    await openNote("Start here.md");
  }
  toast("This is the demo vault, a copy in Documents/WRITEpad Demo. Change anything you like.");
}

/** First run: a folder for a first book, and a blank manuscript in it. */
async function newBook() {
  await useVault(await api.newBookFolder());
  await newManuscript();
}

async function loadVoices() {
  const sel = $<HTMLSelectElement>("sel-voice");
  sel.replaceChildren(el("option", { textContent: "loading voices…" }));
  try {
    voices = await api.voices();
  } catch (e) {
    voices = [];
    narrationFail(e);
  }
  renderVoices();
  loadCast().catch(() => {});
}

// ---------- cast (table reads) ----------

/** The current cast: character → voice, plus "__narrator". Per manuscript, or per note. */
let cast: Record<string, string> = {};
let castScope = "";

function currentCastScope(): string | null {
  if (doc?.kind === "note") return `note:${doc.path}`;
  if (doc?.kind === "block" && ms) return `ms:${ms.id}`;
  return null;
}

async function loadCast() {
  const scope = currentCastScope();
  const screenplay = !!scope && isScreenplay(editor.text);
  $("cast-section").hidden = !screenplay;
  $("fmt-script").textContent = screenplay ? "Screenplay · Tab changes a line's type · ▶ Listen for a table read" : "";
  if (!screenplay || !scope) return;
  if (scope !== castScope) {
    castScope = scope;
    cast = await api.getCast(scope).catch(() => ({}));
  }
  // New speakers get a voice of their own; existing casting is kept.
  const names = characters(editor.text).map((c) => c.name);
  if (voices.length && names.some((n) => !cast[n])) {
    cast = { ...autoCast(names, voices, [settings.voice, settings.italic_voice], cast, pronounHints(editor.text, names)), __narrator: cast["__narrator"] ?? settings.voice };
    api.setCast(scope, cast).catch(fail);
  }
  renderCast(names);
}

function renderCast(names: string[]) {
  const box = $("cast");
  const pick = (label: string, key: string, current: string, cls = "") => {
    const sel = el("select");
    for (const v of voices.length ? voices : [current]) sel.append(new Option(v, v));
    sel.value = current;
    sel.onchange = () => {
      cast[key] = sel.value;
      api.setCast(castScope, cast).catch(fail);
    };
    return el("label", { className: "cast-pick" }, el("span", { className: cls, textContent: label }), sel);
  };
  const counts = new Map(characters(editor.text).map((c) => [c.name, c.lines]));
  box.replaceChildren(
    pick("Narrator", "__narrator", cast["__narrator"] ?? settings.voice, "narrator"),
    ...names.slice(0, 30).map((n) => {
      const row = pick(n, n, cast[n] ?? settings.voice);
      row.firstElementChild!.append(el("span", { className: "n", textContent: String(counts.get(n) ?? "") }));
      return row;
    }),
  );
}

const refreshCast = debounce(() => loadCast().catch(() => {}), 1200);

// ---------- library + cadence ----------

let library: LibraryReport | null = null;
let scripts: ScriptSummary[] = [];
let cadenceMarks = (() => {
  try {
    return localStorage.getItem("wp-cadence-marks") !== "off";
  } catch {
    return true;
  }
})();

const refreshCadence = debounce(() => runCadence(), 700);

/** Measure the open scene against the target and mark where it drifts. */
async function runCadence() {
  if (doc?.kind !== "block") return;
  const text = editor.text;
  const c = await api.cadence(text, ms?.id ?? null).catch(() => null);
  if (!c || doc?.kind !== "block" || editor.text !== text) return;
  editor.setCadence(cadenceMarks ? c.flags : null);
  renderCadence(c);
  renderProgress();
}

// ---------- page properties ----------

/** The row being edited, and the rows as edited so far (with any new one). */
let propEdit: number | null = null;
let propDraft: Prop[] | null = null;

/** Text with each `[[link]]` made clickable. */
function linked(text: string): Node[] {
  return text.split(/(\[\[[^\]]+\]\])/).filter(Boolean).map((part) => {
    const m = part.match(/^\[\[([^\]]+)\]\]$/);
    if (!m) return document.createTextNode(part);
    const a = el("a", { href: "#", textContent: m[1], title: `Open ${m[1]}` });
    a.onclick = (e) => {
      e.preventDefault();
      e.stopPropagation();
      openPageLink(m[1]).catch(fail);
    };
    return a;
  });
}

function renderProps() {
  const box = $("props");
  box.replaceChildren();
  const block = currentBlock();
  if (doc?.kind !== "block" || !block || !ms) return;
  // Names used elsewhere in this manuscript, offered while typing one.
  const keys = [...new Set(ms.blocks.flatMap((b) => b.props.map((p) => p.key)))].sort();
  $("prop-keys").replaceChildren(...keys.map((k) => el("option", { value: k })));
  const rows = propDraft ?? block.props;
  rows.forEach((p, i) => {
    if (i === propEdit) return box.append(propEditor(block.id, rows, i));
    const row = el("div", { className: "prop", title: "Click to edit" }, el("span", { className: "k", textContent: p.key }), el("span", { className: "v" }, ...linked(p.value)));
    clickable(row, () => {
      propDraft = rows.map((x) => ({ ...x }));
      propEdit = i;
      renderProps();
    });
    box.append(row);
  });
  if (propEdit === null) {
    const add = el("button", { className: "ghost add-prop", type: "button", textContent: "+ Property", title: "Add to this page: verses, POV, setting, [[a note]] or [[Manuscript#Page]]. Search finds them; export leaves them out." });
    add.onclick = () => {
      propDraft = [...block.props.map((x) => ({ ...x })), { key: "", value: "" }];
      propEdit = propDraft.length - 1;
      renderProps();
    };
    box.append(add);
  }
}

function propEditor(blockId: string, rows: Prop[], i: number): HTMLElement {
  const k = el("input", { className: "k", value: rows[i].key, placeholder: "name", spellcheck: false });
  k.setAttribute("list", "prop-keys");
  const v = el("input", { className: "v", value: rows[i].value, placeholder: "value, [[a note]] or [[Manuscript#Page]]" });
  const del = el("button", { className: "ghost", type: "button", textContent: "×", title: "Remove this property" });
  const row = el("div", { className: "prop editing" }, k, v, del);
  let done = false;
  const finish = (how: "save" | "remove" | "cancel") => {
    if (done) return;
    done = true;
    const next = rows.map((x) => ({ ...x }));
    if (how === "save") next[i] = { key: k.value.trim(), value: v.value.trim() };
    if (how === "remove") next.splice(i, 1);
    propEdit = null;
    propDraft = null;
    const block = currentBlock();
    const same = block && JSON.stringify(next.filter((p) => p.key)) === JSON.stringify(block.props);
    if (how === "cancel" || !ms || !block || block.id !== blockId || same) return renderProps();
    op(api.setBlockProps(ms.id, blockId, next));
  };
  del.onmousedown = (e) => e.preventDefault(); // keep focus, so blur doesn't save first
  del.onclick = () => finish("remove");
  for (const input of [k, v]) {
    input.onkeydown = (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (input === k && !v.value) v.focus();
        else finish("save");
      }
      if (e.key === "Escape") finish("cancel");
    };
  }
  row.addEventListener("focusout", (e) => {
    if (!row.contains(e.relatedTarget as Node | null)) finish("save");
  });
  requestAnimationFrame(() => (rows[i].key ? v : k).focus());
  return row;
}

// ---------- your target ----------

function openTargetDialog() {
  if (!ms) return;
  const id = ms.id;
  const t = ms.target ?? {};
  const dlg = $<HTMLDialogElement>("dlg-target");
  const field = (name: string) => $<HTMLInputElement>(`tg-${name}`);
  const show = (n: number | null | undefined) => (n == null ? "" : String(n));
  field("chapter-min").value = show(t.chapter_min);
  field("chapter-max").value = show(t.chapter_max);
  field("sentence").value = show(t.sentence);
  field("dialogue").value = t.dialogue == null ? "" : String(Math.round(t.dialogue * 100));
  field("total").value = show(t.total);
  dlg.returnValue = "";
  dlg.showModal();
  dlg.onclose = () => {
    if (dlg.returnValue !== "ok" && dlg.returnValue !== "clear") return;
    const num = (name: string) => {
      const n = parseFloat(field(name).value);
      return Number.isFinite(n) && n > 0 ? n : null;
    };
    const dialogue = num("dialogue");
    const target: Target = dlg.returnValue === "clear" ? {} : {
      chapter_min: num("chapter-min") && Math.round(num("chapter-min")!),
      chapter_max: num("chapter-max") && Math.round(num("chapter-max")!),
      sentence: num("sentence"),
      dialogue: dialogue == null ? null : Math.min(dialogue, 100) / 100,
      total: num("total") && Math.round(num("total")!),
    };
    op(api.setTarget(id, target)).then(() => runCadence());
  };
}

/** Words in the manuscript's live takes against the author's goal for it. */
function renderProgress() {
  const box = $("ms-progress");
  const goal = ms?.target?.total;
  box.hidden = !goal;
  if (!ms || !goal) return;
  const n = ms.blocks.filter((b) => !b.out).reduce((sum, b) => sum + words(b.variants.find((v) => v.id === b.live)?.text ?? ""), 0);
  box.querySelector<HTMLElement>(".bar span")!.style.width = `${Math.min(100, (100 * n) / goal)}%`;
  box.querySelector(".label")!.textContent = `${n.toLocaleString()} of ${goal.toLocaleString()} words`;
  box.title = `${Math.round((100 * n) / goal)}% of your target, counting the live take of each page`;
}

function renderCadence(c: Cadence) {
  const bar = $("cadence");
  const stat = (label: string, value: string, target?: string) => {
    const span = el("span", {}, el("b", { textContent: value }), ` ${label}`);
    if (target) span.append(el("span", { textContent: ` (target ${target})` }));
    return span;
  };
  const kids: Node[] = [];
  const band = c.chapter_band;
  const round = (n: number) => Math.round(n).toLocaleString();
  const chapters = band && (Math.round(band[0]) === Math.round(band[2]) ? round(band[0]) : `${round(band[0])}–${round(band[2])}`);
  kids.push(stat("words", c.words.toLocaleString(), chapters ? `chapters ${chapters}` : undefined));
  if (c.sentences) kids.push(stat("median sentence", String(c.sentence_median), c.sentence_target != null ? String(c.sentence_target) : undefined));
  kids.push(stat("dialogue", `${Math.round(c.dialogue * 100)}%`, c.dialogue_target != null ? `${Math.round(c.dialogue_target * 100)}%` : undefined));
  if (c.target_name == null) {
    const b = el("button", { className: "ghost", type: "button", textContent: "Set a target…", title: "Your own numbers, or blend books in the Library" });
    b.onclick = () => openTargetDialog();
    kids.push(b);
  } else {
    const b = el("button", { className: "ghost", type: "button", textContent: "Target…", title: `Measuring against ${c.target_name}` });
    b.onclick = () => openTargetDialog();
    kids.push(b);
  }
  if (c.blend) {
    const n = c.flags.length;
    const b = el("button", {
      className: "ghost",
      type: "button",
      textContent: cadenceMarks ? `${n} mark${n === 1 ? "" : "s"} · hide` : `${n} mark${n === 1 ? "" : "s"} · show`,
      title: "Underlines where this scene drifts from your target. Hover one to see why.",
    });
    b.onclick = () => {
      cadenceMarks = !cadenceMarks;
      try {
        localStorage.setItem("wp-cadence-marks", cadenceMarks ? "on" : "off");
      } catch {}
      editor.setCadence(cadenceMarks ? c.flags : null);
      renderCadence(c);
    };
    kids.push(b);
  }
  bar.replaceChildren(...kids);
}

/** The manuscript the Library measures: the open one, else the last opened, else the first. */
let libraryMs: string | null = null;

async function loadLibrary() {
  libraryMs = ms?.id ?? libraryMs ?? (await api.listManuscripts().catch(() => []))[0]?.id ?? null;
  [library, scripts] = await Promise.all([api.libraryReport(libraryMs), api.scriptsReport().catch(() => [])]);
  renderLibraryPane();
  if (document.body.dataset.mode === "library")
    renderLibrary($("library-view"), library, scripts, { add: () => addBooks().catch(fail), adapt: (b, sc) => api.adaptation(b, sc) });
}

const refreshLibrary = debounce(() => loadLibrary().catch(() => {}), 1500);

const saveWeights = debounce(() => {
  if (!library) return;
  api
    .librarySetWeights(library.weights)
    .then(() => loadLibrary())
    .then(() => runCadence())
    .catch(fail);
}, 400);

function renderLibraryPane() {
  renderScriptsPane();
  const list = $("library-books");
  list.replaceChildren();
  if (!library) return;
  if (!library.books.length) {
    list.append(el("li", { className: "hint", textContent: "No books yet. Add EPUBs of authors whose style you want to measure against." }));
    return;
  }
  library.books.forEach((b, i) => {
    const dot = el("span", { className: "lib-dot" });
    dot.style.background = seriesVar(i);
    const weight = el("input", { className: "weight", type: "number", min: "0", max: "100", step: "5", value: String(library!.weights[b.id] ?? 0), title: "Weight in your target (0 = not in it)" });
    weight.oninput = () => {
      const v = Math.max(0, Number(weight.value) || 0);
      if (v > 0) library!.weights[b.id] = v;
      else delete library!.weights[b.id];
      saveWeights();
    };
    const remove = confirmButton("×", () => api.libraryRemove(b.id).then(loadLibrary).then(() => runCadence()).catch(fail));
    remove.title = "Remove from the library (click twice)";
    list.append(
      el(
        "li",
        { className: "book" },
        dot,
        el("span", { className: "t", textContent: b.title }),
        weight,
        remove,
        el("span", { className: "a", textContent: `${b.author || "Unknown author"} · ${b.words.toLocaleString()} words · ${b.chapters} chapters` }),
      ),
    );
  });
}

function renderScriptsPane() {
  const list = $("library-scripts");
  list.replaceChildren();
  if (!scripts.length) {
    list.append(el("li", { className: "hint", textContent: "No scripts yet. Add one from a link or a file." }));
    return;
  }
  scripts.forEach((sc, i) => {
    const dot = el("span", { className: "lib-dot" });
    dot.style.background = seriesVar(i);
    const remove = confirmButton("×", () => api.scriptRemove(sc.id).then(loadLibrary).catch(fail));
    remove.title = "Remove from the library (click twice)";
    const openBtn = el("button", { className: "ghost", type: "button", textContent: "Open", title: "Open as a manuscript to read it, or hear a table read. Copies the script's text into this vault." });
    openBtn.onclick = () => openScript(sc.id, sc.title).catch(fail);
    list.append(
      el(
        "li",
        { className: "book" },
        dot,
        el("span", { className: "t", textContent: sc.title }),
        openBtn,
        remove,
        el("span", { className: "a", textContent: `${sc.writer || "Unknown writer"} · ${Math.round(sc.pages)} pages · ${sc.scenes} scenes` }),
      ),
    );
  });
}

async function openScript(id: string, title: string) {
  toast(`Opening ${title}…`);
  const msId = await api.scriptOpen(id);
  await loadManuscripts();
  showTab("manuscripts");
  await openManuscript(msId);
  toast(`${title} is open as a manuscript. ▶ Listen plays it as a table read; the Cast panel picks the voices.`);
}

async function addScript() {
  const dlg = $<HTMLDialogElement>("dlg-script");
  const url = $<HTMLInputElement>("script-url");
  const fileLabel = $("script-file");
  let file: string | null = null;
  url.value = "";
  fileLabel.textContent = "";
  $("btn-script-file").onclick = async () => {
    const picked = await open({ multiple: false, filters: [{ name: "Screenplay", extensions: ["txt", "fountain", "pdf", "html"] }] });
    if (typeof picked === "string") {
      file = picked;
      fileLabel.textContent = picked.split("/").pop() ?? picked;
      url.value = "";
    }
  };
  dlg.returnValue = "";
  dlg.showModal();
  url.focus();
  const source = await new Promise<string | null>((resolve) => {
    dlg.onclose = () => resolve(dlg.returnValue === "ok" ? url.value.trim() || file : null);
  });
  if (!source) return;
  toast("Reading the script…");
  const sc = await api.scriptAdd(source);
  toast(`Measured ${sc.title}: ${Math.round(sc.pages)} pages, ${sc.scenes} scenes.`);
  await loadLibrary();
}

async function addBooks() {
  const picked = await open({ multiple: true, filters: [{ name: "EPUB", extensions: ["epub"] }] });
  const paths = picked === null ? [] : Array.isArray(picked) ? picked : [picked];
  if (!paths.length) return;
  toast(`Measuring ${paths.length} book${paths.length === 1 ? "" : "s"}…`);
  const failed = await api.libraryAdd(paths);
  if (failed.length) toast(failed.join("\n"), true);
  else toast(`Measured ${paths.length} book${paths.length === 1 ? "" : "s"}.`);
  await loadLibrary();
  showTab("library");
}

async function openLibrary() {
  await flushSaves();
  narrator.stop();
  doc = null;
  setMode("library");
  await loadLibrary().catch(fail);
}

// ---------- search ----------

let searchSeq = 0;
let sidebarTab = "manuscripts";

function showSearch(on: boolean) {
  $("pane-search").hidden = !on;
  for (const t of ["manuscripts", "notes", "world", "library"]) $(`pane-${t}`).hidden = on || t !== sidebarTab;
  document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((b) => b.classList.toggle("active", !on && b.dataset.tab === sidebarTab));
}

/** The snippet with each query word marked. */
function marked(text: string, words: string[]): Node[] {
  // Any dash matches any dash, as in search itself.
  const esc = words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&").replace(/[-–—]/g, "[-–—]"));
  const parts = text.split(new RegExp(`(${esc.join("|")})`, "gi"));
  return parts.map((p, i) => (i % 2 ? el("mark", { textContent: p }) : document.createTextNode(p)));
}

async function runSearch() {
  const query = $<HTMLInputElement>("search").value.trim();
  const seq = ++searchSeq;
  if (!query) return showSearch(false);
  const hits = await api.search(query);
  if (seq !== searchSeq) return;
  showSearch(true);
  const words = query.split(/\s+/);
  const n = hits.length;
  $("search-count").textContent = n ? `${n === 200 ? "200+" : n} ${n === 1 ? "match" : "matches"}` : "Nothing matches.";
  const list = $("list-search");
  list.replaceChildren();
  for (const h of hits) {
    const head = el("div", { className: "hit-head" },
      el("span", { className: "name", textContent: h.title }),
      el("span", { className: "meta", textContent: h.kind === "note" ? "note" : h.context }));
    const li = el("li", { title: h.kind === "note" ? h.path : `${h.context} — ${h.title}` }, head, el("div", { className: "snippet" }, ...marked(h.snippet, words)));
    clickable(li, async () => {
      list.querySelectorAll("li.active").forEach((x) => x.classList.remove("active"));
      li.classList.add("active");
      if (h.kind === "note") await openNote(h.path);
      else {
        if (ms?.id !== h.ms) await openManuscript(h.ms);
        openBlock(h.block, h.take);
      }
      editor.find(words[0]);
    });
    list.append(li);
  }
}

function showTab(tab: string) {
  sidebarTab = tab;
  const search = $<HTMLInputElement>("search");
  if (search.value) {
    search.value = "";
    searchSeq++;
    showSearch(false);
  }
  document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((b) => b.classList.toggle("active", b.dataset.tab === tab));
  $("pane-manuscripts").hidden = tab !== "manuscripts";
  $("pane-notes").hidden = tab !== "notes";
  $("pane-world").hidden = tab !== "world";
  $("pane-library").hidden = tab !== "library";
  if (tab === "world") loadWorld().catch(fail);
  if (tab === "library") loadLibrary().catch(fail);
}

// ---------- wiring ----------

function wire() {
  document.querySelectorAll<HTMLButtonElement>(".tabs button").forEach((b) => (b.onclick = () => showTab(b.dataset.tab!)));
  const search = $<HTMLInputElement>("search");
  search.oninput = debounce(() => runSearch().catch(fail), 150);
  search.onkeydown = (e) => {
    if (e.key === "Escape") {
      search.value = "";
      runSearch().catch(fail);
    }
    if (e.key === "Enter") $("list-search").querySelector<HTMLElement>("li")?.click();
  };
  document.addEventListener("keydown", (e) => {
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      search.focus();
      search.select();
    }
    if ((e.metaKey || e.ctrlKey) && e.altKey && e.code === "KeyM" && document.body.dataset.mode === "block") {
      e.preventDefault();
      addComment();
    }
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === "d") {
      e.preventDefault();
      setFocus(!document.body.classList.contains("focus"));
    }
    // Esc leaves focus mode, unless it's busy closing something else first.
    if (e.key === "Escape" && document.body.classList.contains("focus") && !narrator.playing && !document.querySelector(".cm-search")) {
      setFocus(false);
    }
  });
  $("btn-focus").onclick = () => setFocus(true);
  $("btn-comment").onclick = () => addComment();
  $("btn-unfocus").onclick = () => setFocus(false);
  $("btn-vault").onclick = () => chooseVault().catch(fail);
  $("btn-empty-vault").onclick = () => chooseVault().catch(fail);
  $("btn-open-folder").onclick = () => chooseVault().catch(fail);
  $("btn-get-python").onclick = () => openHelp("python");
  $("btn-try-demo").onclick = () => tryDemo().catch(fail);
  $("btn-new-book").onclick = () => newBook().catch(fail);
  $("btn-back").onclick = closeManuscript;
  $("btn-library-add").onclick = () => addBooks().catch(fail);
  $("btn-script-add").onclick = () => addScript().catch(fail);
  $("fmt-b").onclick = () => editor.bold();
  $("fmt-i").onclick = () => editor.italic();
  $("fmt-s").onclick = () => editor.strike();
  $("fmt-code").onclick = () => editor.code();
  $("fmt-link").onclick = () => editor.link();
  $("fmt-bullet").onclick = () => editor.block("bullet");
  $("fmt-number").onclick = () => editor.block("number");
  $("fmt-task").onclick = () => editor.block("task");
  $("fmt-quote").onclick = () => editor.block("quote");
  $("fmt-hr").onclick = () => editor.divider();
  $("fmt-image").onclick = () => pickImage().catch(fail);
  $<HTMLSelectElement>("fmt-heading").onchange = (e) => editor.heading(Number((e.target as HTMLSelectElement).value));
  $("fmt-left").onclick = () => editor.align("left");
  $("fmt-center").onclick = () => editor.align("center");
  $("fmt-right").onclick = () => editor.align("right");
  $("btn-recast").onclick = () => {
    const names = characters(editor.text).map((c) => c.name);
    cast = { ...autoCast(names, voices.slice().sort(() => Math.random() - 0.5), [settings.voice, settings.italic_voice], {}, pronounHints(editor.text, names)), __narrator: cast["__narrator"] ?? settings.voice };
    api.setCast(castScope, cast).catch(fail);
    renderCast(names);
  };
  $("btn-library-open").onclick = () => openLibrary().catch(fail);
  $("btn-play").onclick = () => togglePlay();
  $("btn-graph").onclick = () => openGraph();
  $("btn-timeline").onclick = () => openTimeline();
  $("btn-world-suggest").onclick = () => openSuggest().catch(fail);
  $("btn-world-csv").onclick = () => importWorldCsv().catch(fail);
  $("btn-event-here").onclick = async () => {
    const d = doc;
    if (d?.kind !== "block" || !ms) return;
    timelineCache = await api.worldTimeline().catch(() => timelineCache);
    newEvent(lastEventPath(), null, [ms.id, d.blockId]);
  };
  $("btn-dictate").onmousedown = (e) => e.preventDefault(); // keep the editor's cursor
  $("btn-dictate").onclick = () => (dictState === "recording" ? stopDictation() : startDictation(true));
  wireDictationKey();
  $("btn-ab").onclick = playAB;

  const speed = $<HTMLInputElement>("rng-speed");
  const saveSettingsSoon = debounce(() => api.saveSettings(settings).catch(fail), 400);
  speed.value = String(settings.speed);
  $("lbl-speed").textContent = `${settings.speed.toFixed(2)}×`;
  speed.oninput = () => {
    settings.speed = Number(speed.value);
    $("lbl-speed").textContent = `${settings.speed.toFixed(2)}×`;
    saveSettingsSoon();
  };
  $<HTMLSelectElement>("sel-voice").onchange = (e) => {
    settings.voice = (e.target as HTMLSelectElement).value;
    saveSettingsSoon();
  };

  const ghostInput = $<HTMLInputElement>("rng-ghost");
  ghostInput.value = String(ghost);
  ghostInput.oninput = () => setGhost(Number(ghostInput.value));
  $<HTMLSelectElement>("sel-compare").onchange = (e) => {
    const value = (e.target as HTMLSelectElement).value;
    if (value === "snap") return;
    compareSnap = null;
    compareId = value || null;
    renderHistory();
    renderVariants();
    renderCompare();
    refreshDiff();
    refreshDiff.flush();
  };

  $<HTMLInputElement>("doc-title").oninput = (e) => {
    const block = currentBlock();
    if (!block) return;
    block.title = (e.target as HTMLInputElement).value;
    renderOutline();
    saveTitle();
  };
  const saveTitle = debounce(() => {
    const block = currentBlock();
    if (block && ms) op(api.setBlockTitle(ms.id, block.id, block.title));
  }, 500);

  // Cuts
  $("btn-save-cut").onclick = () => {
    if (!ms) return;
    const input = $<HTMLInputElement>("cut-name");
    const name = input.value.trim() || `Cut ${ms.cuts.length + 1}`;
    input.value = "";
    op(api.cutOp(ms.id, "save", name)).then((m) => m && toast(`Saved cut “${name}”.`));
  };
  // History
  $("btn-history-on").onclick = async () => {
    await flushSaves();
    try {
      await api.historyEnable();
      lastSnapAt = Date.now();
      dirty = false;
      toast("History is on. Every version from here on is kept.");
      loadHistory();
    } catch (e) {
      fail(e);
    }
  };
  $("btn-snap").onclick = () => {
    const input = $<HTMLInputElement>("snap-msg");
    const message = input.value.trim() || "Saved version";
    input.value = "";
    takeSnapshot(message);
  };
  $<HTMLInputElement>("snap-msg").onkeydown = (e) => {
    if (e.key === "Enter") $("btn-snap").click();
  };
  const autosave = $<HTMLInputElement>("chk-autosave");
  autosave.checked = settings.history_autosave;
  autosave.onchange = () => {
    settings.history_autosave = autosave.checked;
    api.saveSettings(settings).catch(fail);
  };

  // Export
  const dlgExport = $<HTMLDialogElement>("dlg-export");
  $("btn-export-open").onclick = () => {
    const book = currentBook();
    if (!book) return toast("Open a manuscript or a note to export.");
    const left = ms ? ms.blocks.filter((b) => b.out).length : 0;
    $("export-what").textContent = ms
      ? `The current cut of “${ms.title}”: ${book.chapters.length} pages, in outline order, using each page's ★ live version.${left ? ` ${left} left out.` : ""}`
      : `The note “${book.title}”.`;
    $<HTMLInputElement>("export-author").value = settings.author;
    $("export-progress").textContent = "";
    dlgExport.showModal();
  };
  $("btn-do-export").onclick = () => runExport();
  dlgExport.onclose = () => (exportCancelled = true);

  // Pronunciations
  $("btn-lex-add").onclick = () => {
    const word = $<HTMLInputElement>("lex-word");
    const say = $<HTMLInputElement>("lex-say");
    if (!word.value.trim() || !say.value.trim()) return;
    lexicon[word.value.trim()] = say.value.trim();
    word.value = say.value = "";
    saveLexicon();
  };

  // Import
  const dlgImport = $<HTMLDialogElement>("dlg-import");
  let importPaths: string[] = [];
  $("btn-add-block").onclick = () => addChapter().catch(fail);
  $("btn-new-ms").onclick = () => newManuscript().catch(fail);
  document.querySelectorAll<HTMLButtonElement>(".view-switch button").forEach((b) => (b.onclick = () => setViewMode(b.dataset.view as ViewMode)));
  let savedView: string | null = null;
  try {
    savedView = localStorage.getItem("wp-view");
  } catch {}
  setViewMode(savedView === "draft" || savedView === "code" ? savedView : "page");
  $("btn-import").onclick = () => {
    importPaths = [];
    $("import-files").replaceChildren();
    $<HTMLInputElement>("import-title").value = "";
    dlgImport.showModal();
  };
  $("btn-pick-files").onclick = async () => {
    const picked = await open({ multiple: true, filters: [{ name: "Manuscripts", extensions: ["md", "markdown", "txt", "epub", "pdf"] }] });
    if (!picked) return;
    importPaths = (Array.isArray(picked) ? picked : [picked]).sort((a, b) => a.localeCompare(b, undefined, { numeric: true }));
    $("import-files").replaceChildren(...importPaths.map((p) => el("li", { textContent: p.split("/").pop()! })));
    const title = $<HTMLInputElement>("import-title");
    if (!title.value) {
      const stem = importPaths[0].split("/").pop()!.replace(/\.\w+$/, "").replace(/[_-]draft[_-]?\d+$/i, "").replace(/_-_.*$/, "");
      title.value = stem.replace(/[_-]+/g, " ").replace(/\b\w/g, (c) => c.toUpperCase());
    }
  };
  $("btn-do-import").onclick = async () => {
    if (!importPaths.length) return toast("Choose at least one file.");
    const title = $<HTMLInputElement>("import-title").value.trim() || "Untitled";
    try {
      const m = await api.importDrafts(title, importPaths);
      dlgImport.close();
      await openManuscript(m.id);
      toast(`Imported ${m.blocks.length} blocks from ${importPaths.length} file(s).`);
    } catch (e) {
      fail(e);
    }
  };

  // New note
  const dlgNote = $<HTMLDialogElement>("dlg-note");
  $("btn-new-note").onclick = () => {
    $<HTMLInputElement>("note-name").value = "";
    dlgNote.showModal();
  };
  dlgNote.onclose = async () => {
    const name = $<HTMLInputElement>("note-name").value.trim();
    if (dlgNote.returnValue !== "ok" || !name) return;
    try {
      const path = await api.createNote(name);
      await loadNotes();
      await openNote(path);
    } catch (e) {
      fail(e);
    }
  };

  // Settings
  const dlgSettings = $<HTMLDialogElement>("dlg-settings");
  $("btn-settings").onclick = () => {
    $<HTMLInputElement>("set-vault").value = settings.vault ?? "";
    $<HTMLInputElement>("set-python").value = settings.python;
    $<HTMLInputElement>("set-models").value = settings.model_dir;
    $<HTMLInputElement>("set-pause").value = String(settings.paragraph_pause);
    $<HTMLInputElement>("set-whisper").value = settings.dictation_model;
    $<HTMLInputElement>("set-lang").value = settings.dictation_language;
    $<HTMLInputElement>("set-polish").checked = settings.dictation_polish;
    themeDraft = { theme: settings.theme, custom_theme: { ...settings.custom_theme } };
    showThemeDraft();
    renderVoices();
    dlgSettings.showModal();
  };
  wireThemePicker();
  $("btn-set-vault").onclick = () => chooseVault().catch(fail);
  $("btn-setup-narration").onclick = () => setupNarration();
  dlgSettings.onclose = async () => {
    if (dlgSettings.returnValue !== "ok") {
      applyTheme(settings); // drop the preview
      return;
    }
    settings.theme = themeDraft.theme;
    settings.custom_theme = themeDraft.custom_theme;
    const before = `${settings.python}|${settings.model_dir}`;
    settings.python = $<HTMLInputElement>("set-python").value.trim();
    settings.model_dir = $<HTMLInputElement>("set-models").value.trim();
    settings.italic_voice = $<HTMLSelectElement>("set-italic").value || settings.italic_voice;
    settings.paragraph_pause = Number($<HTMLInputElement>("set-pause").value) || 0;
    settings.dictation_model = $<HTMLInputElement>("set-whisper").value.trim() || "small";
    settings.dictation_language = $<HTMLInputElement>("set-lang").value.trim() || "auto";
    settings.dictation_polish = $<HTMLInputElement>("set-polish").checked;
    try {
      await api.saveSettings(settings);
      if (before !== `${settings.python}|${settings.model_dir}`) loadVoices();
    } catch (e) {
      fail(e);
    }
  };

  window.addEventListener("beforeunload", flushSaves);
}

// ---------- theme (Settings → Appearance) ----------

/** The theme being edited in Settings, previewed live until Save or Cancel. */
let themeDraft: Pick<Settings, "theme" | "custom_theme">;
const THEME_COLORS = ["sidebar", "background", "page", "text", "accent"] as const;

function showThemeDraft() {
  $<HTMLSelectElement>("set-theme").value = themeDraft.theme;
  $("custom-theme").hidden = themeDraft.theme !== "custom";
  const c = themeDraft.custom_theme;
  $<HTMLSelectElement>("ct-base").value = c.base;
  for (const k of THEME_COLORS) $<HTMLInputElement>(`ct-${k}`).value = c[k];
  const ratio = contrast(c.text, c.page);
  const warn = $("ct-contrast");
  warn.hidden = themeDraft.theme !== "custom" || ratio >= 4.5;
  warn.textContent = `Text on the page is ${ratio.toFixed(1)}:1. Below 4.5:1 it gets hard to read for long.`;
  applyTheme(themeDraft);
}

function wireThemePicker() {
  $<HTMLSelectElement>("set-theme").onchange = (e) => {
    themeDraft.theme = (e.target as HTMLSelectElement).value as ThemeMode;
    showThemeDraft();
  };
  $<HTMLSelectElement>("ct-base").onchange = (e) => {
    themeDraft.custom_theme.base = (e.target as HTMLSelectElement).value as CustomTheme["base"];
    showThemeDraft();
  };
  for (const k of THEME_COLORS) {
    $<HTMLInputElement>(`ct-${k}`).oninput = (e) => {
      themeDraft.custom_theme[k] = (e.target as HTMLInputElement).value;
      showThemeDraft();
    };
  }
  $("theme-presets").replaceChildren(
    ...PRESETS.map((p) => {
      const b = el("button", { type: "button", title: `Start from ${p.name}` });
      const chip = el("span", { className: "chip" });
      chip.style.background = `linear-gradient(135deg, ${p.theme.sidebar} 50%, ${p.theme.accent} 50%)`;
      b.append(chip, p.name);
      b.onclick = () => {
        themeDraft.custom_theme = { ...p.theme };
        showThemeDraft();
      };
      return b;
    }),
  );
}

async function main() {
  settings = await api.getSettings();
  applyTheme(settings);
  wire();
  // Before an update installs: write pending edits, and keep a version if history is on.
  initUpdater(async (version) => {
    await flushSaves();
    if (historyOn) await takeSnapshot(`Before updating to ${version}`, true);
  });
  renderVoices();
  setMode("none");
  if (settings.vault) await loadVault().catch(fail);
  else {
    $("welcome").hidden = false;
    $("empty-msg").hidden = true;
  }
  loadVoices(); // warms the model in the background
}

main().catch(fail);
