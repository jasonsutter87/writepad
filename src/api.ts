// Typed wrappers around the Rust commands in src-tauri/src/lib.rs.
import { invoke } from "@tauri-apps/api/core";
import type { CadenceFlag } from "./editor";

export interface Settings {
  vault: string | null;
  python: string;
  model_dir: string;
  voice: string;
  italic_voice: string;
  speed: number;
  paragraph_pause: number;
  dictation_model: string;
  dictation_language: string;
  dictation_polish: boolean;
  history_autosave: boolean;
  author: string;
  theme: ThemeMode;
  custom_theme: CustomTheme;
}

/** A character or location found in a manuscript by plain rules (crates/core/src/suggest.rs). */
export interface WorldSuggestion {
  kind: "character" | "location";
  name: string;
  aliases: string[];
  mentions: number;
  scenes: string[];
  why: string;
}

export interface WorldSuggestions {
  manuscript: string;
  title: string;
  screenplay: boolean;
  characters: WorldSuggestion[];
  locations: WorldSuggestion[];
  scenes: string[];
}

export type ThemeMode = "system" | "light" | "dark" | "custom";

export interface CustomTheme {
  base: "light" | "dark";
  sidebar: string;
  background: string;
  page: string;
  text: string;
  accent: string;
}

export interface Snapshot {
  id: string;
  message: string;
  time: number;
  auto: boolean;
}

export interface HistoryEntry {
  snapshot: Snapshot;
  text: string;
  label: string | null;
}

export interface BookChapter {
  title: string | null;
  text: string;
}

export interface Book {
  title: string;
  author: string;
  chapters: BookChapter[];
}

export interface Entity {
  path: string;
  name: string;
  kind: string;
  aliases: string[];
  summary: string;
  props: Record<string, unknown>;
  links: string[];
  after: string[];
  when: string | null;
  scenes: string[];
  status: string | null;
}

export interface GraphNode {
  id: string;
  label: string;
  kind: string;
  summary: string;
  manuscript: string | null;
  block: string | null;
}

export interface GraphEdge {
  from: string;
  to: string;
  kind: "link" | "relation" | "next" | "scene" | "mention";
  label: string;
  weight: number;
}

export interface WorldGraph {
  nodes: GraphNode[];
  edges: GraphEdge[];
  issues: { path: string; message: string }[];
}

export interface TimelineEvent {
  path: string;
  name: string;
  when: string | null;
  summary: string;
  status: string | null;
  after: string | null;
  /** "" for the main timeline, else its name. */
  timeline: string;
  lane: number;
  /** [manuscript id, block id, label, position in manuscript] */
  scenes: [string, string, string, number][];
  people: string[];
  /** Where the telling read against tells it first; null if it doesn't. */
  told_at: number | null;
  /** Out of order: told before events that happen earlier. */
  told_earlier: boolean;
  /** Out of order: told after events that happen later (a flashback). */
  told_later: boolean;
}

/** A manuscript and its cuts: what the timeline can be read against. */
export interface Telling {
  id: string;
  title: string;
  cuts: string[];
}

export interface SceneInfo {
  present: [Entity, number][];
  events: Entity[];
}

export interface EntityInfo {
  entity: Entity;
  /** [path, name, kind, outgoing] */
  relations: [string, string, string, boolean][];
  backlinks: [string, string][];
  /** [manuscript, block, label, mentions] */
  appears_in: [string, string, string, number][];
}

export interface AudioSegment {
  text: string;
  voice: string;
  pause: number;
}

export interface Dictation {
  text: string;
  polished: boolean;
  warning: string | null;
}

export interface Variant {
  id: string;
  label: string;
  /** "" for you; otherwise the agent that made this take. */
  author: string;
  text: string;
  created: number;
}

export interface Block {
  id: string;
  title: string;
  variants: Variant[];
  live: string;
  raw: boolean;
}

export interface Cut {
  name: string;
  order: string[];
  picks: Record<string, string>;
}

export interface Manuscript {
  id: string;
  title: string;
  blocks: Block[];
  cuts: Cut[];
  separator: boolean;
  rev: number;
}

export interface ManuscriptInfo {
  id: string;
  title: string;
  blocks: number;
  variants: number;
}

export interface SearchHit {
  kind: "note" | "take";
  path: string;
  ms: string;
  block: string;
  take: string;
  title: string;
  context: string;
  snippet: string;
  count: number;
  in_title: boolean;
}

export interface NoteEntry {
  path: string;
  name: string;
}

export interface DiffOp {
  tag: "equal" | "insert" | "delete";
  text: string;
}

export type Lexicon = Record<string, string>;

export interface Quant {
  p10: number;
  p25: number;
  median: number;
  p75: number;
  p90: number;
  p99: number;
  mean: number;
}

/** Measured style of one book, the target blend, or a manuscript. */
export interface StyleSummary {
  id: string;
  title: string;
  author: string;
  words: number;
  chapters: number;
  chapter_words: Quant;
  sentence_words: Quant;
  paragraph_words: Quant;
  paragraphs_known: boolean;
  sentence_spread: number;
  dialogue: number;
  one_sentence_paras: number;
  adverbs_per_1k: number;
  flesch: number;
  repeated_openers_per_1k: number;
  endings: { dialogue: number; question: number; short: number; other: number };
  sentence_curve: number[];
  chapter_list: number[];
}

export interface LibraryReport {
  books: StyleSummary[];
  weights: Record<string, number>;
  target: StyleSummary | null;
  manuscript: StyleSummary | null;
}

export interface Cadence {
  words: number;
  sentences: number;
  sentence_median: number;
  dialogue: number;
  chapter_band: [number, number, number] | null;
  sentence_target: number | null;
  dialogue_target: number | null;
  target_name: string | null;
  flags: CadenceFlag[];
}

export interface ScriptSummary {
  id: string;
  title: string;
  writer: string;
  source: string;
  pages: number;
  scenes: number;
  scene_pages: Quant;
  dialogue: number;
  speech_words: Quant;
  action_lines: Quant;
  speeches_per_page: number;
  interior: number;
  night: number;
  locations: number;
  location_changes: number;
  characters: { name: string; share: number; speeches: number; first_page: number }[];
  pacing: { start: number; pages: number; dialogue: number; heading: string }[];
}

export interface Adaptation {
  book: string;
  script: string;
  book_words: number;
  script_pages: number;
  chapters: { title: string; words: number; scenes: number[]; pages: number }[];
  scenes: { heading: string; pages: number; chapter: number | null; score: number; bridged: boolean; verbatim: number }[];
  verbatim: number;
  kept: { speaker: string; text: string; chapter: number | null }[];
  invented_scenes: number;
  invented_pages: number;
  cut_chapters: number;
}

export const api = {
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),

  listNotes: () => invoke<NoteEntry[]>("list_notes"),
  search: (query: string) => invoke<SearchHit[]>("search_vault", { query }),
  readNote: (path: string) => invoke<string>("read_note", { path }),
  writeNote: (path: string, text: string) => invoke<void>("write_note", { path, text }),
  createNote: (name: string) => invoke<string>("create_note", { name }),

  listManuscripts: () => invoke<ManuscriptInfo[]>("list_manuscripts"),
  loadManuscript: (id: string) => invoke<Manuscript>("load_manuscript", { id }),
  writeTake: (manuscript: string, take: string, text: string) => invoke<void>("write_take", { manuscript, take, text }),
  addTake: (manuscript: string, block: string, after: string | null, label: string, text: string) =>
    invoke<Manuscript>("add_take", { manuscript, block, after, label, text }),
  deleteTake: (manuscript: string, take: string) => invoke<Manuscript>("delete_take", { manuscript, take }),
  restoreTake: (manuscript: string, take: string) => invoke<Manuscript>("restore_take", { manuscript, take }),
  setLive: (manuscript: string, take: string) => invoke<Manuscript>("set_live", { manuscript, take }),
  renameTake: (manuscript: string, take: string, label: string) => invoke<Manuscript>("rename_take", { manuscript, take, label }),
  setBlockTitle: (manuscript: string, block: string, title: string) => invoke<Manuscript>("set_block_title", { manuscript, block, title }),
  setOrder: (manuscript: string, order: string[]) => invoke<Manuscript>("set_order", { manuscript, order }),
  addBlock: (manuscript: string, title: string, after: string | null) => invoke<Manuscript>("add_block", { manuscript, title, after }),
  deleteBlock: (manuscript: string, block: string) => invoke<Manuscript>("delete_block", { manuscript, block }),
  cutOp: (manuscript: string, op: "save" | "apply" | "delete", name: string) => invoke<Manuscript>("cut_op", { manuscript, op, name }),
  importDrafts: (title: string, paths: string[]) => invoke<Manuscript>("import_drafts", { title, paths }),

  diffWords: (old: string, next: string) => invoke<DiffOp[]>("diff_words", { old, new: next }),

  voices: () => invoke<string[]>("voices"),
  setupNarration: () => invoke<Settings>("setup_narration"),
  synth: (text: string, voice: string, speed: number) => invoke<ArrayBuffer>("synth", { text, voice, speed }),
  getLexicon: () => invoke<Lexicon>("get_lexicon"),
  setLexicon: (lexicon: Lexicon) => invoke<void>("set_lexicon", { lexicon }),

  dictateStart: () => invoke<void>("dictate_start"),
  dictateStop: (before: string) => invoke<Dictation>("dictate_stop", { before }),
  dictateCancel: () => invoke<void>("dictate_cancel"),

  historyStatus: () => invoke<{ enabled: boolean; branch: string | null }>("history_status"),
  historyEnable: () => invoke<void>("history_enable"),
  historySnapshot: (message: string) => invoke<Snapshot | null>("history_snapshot", { message }),
  historyLog: () => invoke<Snapshot[]>("history_log"),
  noteHistory: (path: string) => invoke<HistoryEntry[]>("note_history", { path }),
  takeHistory: (manuscript: string, take: string) => invoke<HistoryEntry[]>("take_history", { manuscript, take }),
  historyRestoreAll: (id: string) => invoke<void>("history_restore_all", { id }),

  worldGraph: () => invoke<WorldGraph>("world_graph"),
  worldTimeline: (manuscript: string | null = null, cut: string | null = null) => invoke<TimelineEvent[]>("world_timeline", { manuscript, cut }),
  worldTellings: () => invoke<Telling[]>("world_tellings"),
  worldEntities: () => invoke<Entity[]>("world_entities"),
  worldEntity: (path: string) => invoke<EntityInfo | null>("world_entity", { path }),
  worldScene: (manuscript: string, block: string) => invoke<SceneInfo>("world_scene", { manuscript, block }),
  libraryReport: (manuscript: string | null) => invoke<LibraryReport>("library_report", { manuscript }),
  libraryAdd: (paths: string[]) => invoke<string[]>("library_add", { paths }),
  libraryRemove: (id: string) => invoke<void>("library_remove", { id }),
  librarySetWeights: (weights: Record<string, number>) => invoke<void>("library_set_weights", { weights }),
  cadence: (text: string) => invoke<Cadence>("cadence", { text }),
  scriptOpen: (id: string) => invoke<string>("script_open", { id }),
  getCast: (scope: string) => invoke<Record<string, string>>("get_cast", { scope }),
  setCast: (scope: string, cast: Record<string, string>) => invoke<void>("set_cast", { scope, cast }),
  scriptAdd: (source: string) => invoke<ScriptSummary>("script_add", { source }),
  scriptRemove: (id: string) => invoke<void>("script_remove", { id }),
  scriptsReport: () => invoke<ScriptSummary[]>("scripts_report"),
  adaptation: (book: string, scriptId: string) => invoke<Adaptation>("adaptation", { book, scriptId }),
  worldAdd: (kind: string, name: string) => invoke<string>("world_add", { kind, name }),
  worldSuggest: (manuscript: string) => invoke<WorldSuggestions>("world_suggest", { manuscript }),
  worldSuggestApply: (manuscript: string, picks: WorldSuggestion[], timeline: boolean) =>
    invoke<string[]>("world_suggest_apply", { manuscript, picks, timeline }),
  worldImportCsv: (path: string) => invoke<{ created: string[]; skipped: string[] }>("world_import_csv", { path }),
  worldInsertEvent: (name: string, after: string | null, before: string | null, when: string | null, scene: [string, string] | null, timeline: string | null = null) =>
    invoke<string>("world_insert_event", { name, after, before, when, scene, timeline }),
  worldMoveEvent: (path: string, after: string | null, before: string | null) => invoke<void>("world_move_event", { path, after, before }),
  worldDeleteEvent: (path: string) => invoke<string>("world_delete_event", { path }),
  createManuscript: (title: string, screenplay: boolean) => invoke<string>("create_manuscript", { title, screenplay }),
  deleteManuscript: (id: string) => invoke<void>("delete_manuscript", { id }),
  restoreManuscript: (id: string) => invoke<void>("restore_manuscript", { id }),
  deleteNote: (path: string) => invoke<string>("delete_note", { path }),
  restoreNote: (id: string) => invoke<string>("restore_note", { id }),

  exportBook: (book: Book, format: string, path: string) => invoke<void>("export_book", { book, format, path }),
  exportAudioChapter: (segments: AudioSegment[], speed: number, path: string, title: string, album: string, artist: string) =>
    invoke<void>("export_audio_chapter", { segments, speed, path, title, album, artist }),
};
