# WRITEpad

A writing desk for working a script over: onion-skinned drafts, A/B versions,
reorderable blocks, Obsidian-style notes, and local narration so you can hear
what you wrote.

Tauri 2 (Rust core) + CodeMirror 6. Everything stays on your machine.

## Download

Installers for Mac (Apple Silicon and Intel) and Windows are on the
[Releases](https://github.com/jasonsutter87/writepad/releases) page. They aren't
signed yet: on a Mac, open the app once, then System Settings → Privacy &
Security → **Open Anyway**; on Windows, SmartScreen → **More info → Run anyway**.
Narration needs Python 3.10+; press Listen and choose **Set up narration**, and
WRITEpad installs Kokoro and downloads the voices itself. Everything else works
out of the box.

**Try the demo vault.** *WRITEpad-Demo-Vault.zip* on the Releases page is a
tour of every feature, built around *The Monkey's Paw* (public domain). Unzip
it, open the folder in Settings, and read *Start here*. From a clone, just open
the [demo/](demo/) folder (the same vault, without its history);
[demo-source/](demo-source/) is what builds it.

To cut a release: bump `version` in `src-tauri/tauri.conf.json`, then
`git tag v0.1.1 && git push --tags`. The **Release** workflow builds all three
and publishes them.

### Updates

Installed copies check
`https://github.com/jasonsutter87/writepad/releases/latest/download/latest.json`
a few seconds after launch and every six hours. When a newer release is out, a
card offers to update: it saves open edits, takes a history snapshot if history
is on, downloads, installs and restarts. Settings → Updates checks by hand.

Every update is signed. The Release workflow signs with the
`TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
repository secrets, and the app installs only packages that match the public
key in `src-tauri/tauri.conf.json`. The private key is
`~/.tauri/writepad-updater.key`; its password is in the macOS Keychain as
`writepad-updater-key-password`. **Back both up.** Lose the key and installed
copies can never update themselves again.

A local `tauri build` signs too, so it needs the key:

    export TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.tauri/writepad-updater.key)"
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(security find-generic-password -a "$USER" -s writepad-updater-key-password -w)"
    npm run tauri build

## Run

    npm install
    npm run tauri dev          # the app
    cargo build --release -p writepad-cli   # the headless CLI + MCP server
    npm run tauri build -- --bundles app    # WRITEpad.app under target/release/bundle/macos

Dictation builds against [murmur](https://github.com/jasonsutter87/murmur), so
clone both into the same folder:

    git clone https://github.com/jasonsutter87/writepad
    git clone https://github.com/jasonsutter87/murmur

### Windows

Install first (once):

- [Rust](https://rustup.rs) (the default MSVC toolchain)
- Visual Studio Build Tools with **Desktop development with C++**
- [Node.js](https://nodejs.org) LTS, [Git](https://git-scm.com), and
  [CMake](https://cmake.org/download/) (tick "add to PATH"; whisper.cpp needs it)
- WebView2 ships with Windows 10/11

Then, in PowerShell:

    cd writepad
    npm install
    npm run tauri dev               # or: npm run tauri build  -> an installer under target\release\bundle

Narration needs Python 3.10+ with Kokoro, and the two model files. The app's
**Set up narration** button (Settings, or the toast when Listen fails) does all
of this for you. By hand:

    py -m venv $HOME\Documents\ChooseYourAdventure\.venv-tts
    & $HOME\Documents\ChooseYourAdventure\.venv-tts\Scripts\pip install kokoro-onnx soundfile
    mkdir $HOME\Documents\ChooseYourAdventure\tts_models
    cd $HOME\Documents\ChooseYourAdventure\tts_models
    curl.exe -LO https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
    curl.exe -LO https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin

Those are the default locations; put them anywhere and set the paths in
Settings. Dictation downloads its Whisper model on first use. Hold-to-talk is
Right Alt. PDF export uses Edge, which every Windows install has.

## Headless: agents

`writepad mcp` is an MCP server over stdio. Each agent session runs its own
process; they coordinate through the vault's file locks, so 5, 10, 20 agents
can work on one manuscript at once. Register it with Claude Code:

    claude mcp add writepad -- /path/to/writepad/target/release/writepad mcp

It uses the vault open in the app unless given `--vault DIR` (or
`WRITEPAD_VAULT`). Tools: `list_manuscripts`, `outline`, `read`, `fork`, `edit`,
`write`, `compare`, `set_live`, `add_block`, `search`, `list_notes`, `read_note`,
`write_note`, `append_note`, `snapshot`, `history`, `export`, `import`.

Rules the tools enforce: an agent writes only takes it created (`force` to
override when asked), every take carries its agent's name (🤖 in the app),
`write_note` never replaces a note silently, `append_note` is safe to share.
The app watches the vault and shows agents' takes and edits as they land.

Same tools from a shell: `writepad tools`, `writepad call outline '{"manuscript":"the-last-thinker"}'`.

## Page properties

Under a page's title, **+ Property** adds a name and a value, such as
`verses: Matthew 1:18–25`, `POV: June` or `see: [[Research/1920s]]`.
`[[Links]]` open notes, other pages (`[[Page]]`) or pages of another manuscript
(`[[Manuscript#Page]]`). Properties belong to the page, not to one take, are
found by search, and are never exported. Names you've used before are offered
as you type. Agents use `set_props`, and `outline` lists them.

## Search

The box above the sidebar tabs (⌘⇧F) searches every note and every take of
every manuscript, not just the live ones, so a word only one draft uses still
turns up; each hit names its take. Every word must match, ignoring case.
Matches in a title or a page's properties come first, then the words as
typed, then the most matches. Hyphens and dashes count as the same.

A search that reads as a reference (`Matthew 1:1-2`, `john 3:16`, `Luke 15`)
also finds every page whose properties cite an overlapping range, and puts
them first: `verses: Matthew 1:1–17; Luke 3:23–38` answers `Matthew 1:5`
and `Luke 3`. Properties can list several books, ranges across chapters
(`1:18–2:3`) and follow-ons that keep the book (`Mark 1:1-8, 2:3`); book
names match from three letters (`matt`). Clicking a hit opens that take with the match selected,
and the results stay put so you can step through them. Esc or a tab clears it.

## Storage (v2)

    .writepad/manuscripts/<id>/manuscript.json    order, titles, live takes, cuts
    .writepad/manuscripts/<id>/variants/<take>.md one file per take

Text writes touch only their take's file; structural changes run under a lock
against a fresh read, so concurrent writers never lose each other's work.
Older single-file manuscripts migrate on first open (kept as `<id>.json.v1`).

## Ideas

- **Vault**: any folder of markdown (an Obsidian vault works). Notes are plain
  `.md` files; `⌘-click` a `[[wikilink]]` to follow or create it.
- **Manuscript**: a script split into blocks on `## ` headings. *Import drafts…*
  takes several drafts of one script and lines up same-titled sections as
  versions of one block (oldest first, newest live). Stored in
  `<vault>/.writepad/manuscripts/`.
- **Versions**: each block holds a stack of versions. *Fork* copies the current
  one to try something; *Make live* puts it in the current cut.
- **Onion skin**: pick a version to compare with. Words you added are tinted;
  words the other version had are drawn as ghosts. The Ghost slider fades them.
- **Cuts**: drag blocks in the outline to reorder. Save the order plus the
  live versions as a named cut; *Export live cut* writes `<id>.cut.md`.

## Dictation (murmur-core)

- Hold **Right ⌥** and talk; let go and the text lands at the cursor. Tap it
  instead to go hands-free until the next tap. Or click **🎙 Dictate**.
- Local Whisper via `murmur-core` (path dependency on `../murmur/core`), using
  the models murmur already downloaded to `~/Library/Application Support/murmur/models`.
- **Polish with Claude** (Settings, off by default): drops ums and false starts,
  applies "no wait" corrections and spoken formatting, keeps every word. Uses
  murmur's key file or `ANTHROPIC_API_KEY`.

## EPUB and PDF import

*Import drafts, EPUB, PDF…* takes all three, mixed freely (same-titled sections
line up as versions).

- **EPUB**: one block per chapter, titled from the table of contents; italics
  and scene breaks kept.
- **PDF**: structure rebuilt from layout (`pdf.rs`): larger type is a heading
  and starts a block, a taller gap or first-line indent starts a paragraph,
  running heads and page numbers are dropped, hyphenation is rejoined. No
  headings → blocks of 10 pages. Italics are lost; scanned PDFs need OCR.

## Library and cadence

**Library** measures books you want to learn from. Add EPUBs; WRITEpad keeps
only numbers (`.writepad/library/<book>.json`), never the text. Books and scripts
kept inside the vault are remembered by vault-relative path, so a shared vault
still finds them:

- chapter, sentence and paragraph lengths as distributions, not just averages
- dialogue share, one-sentence paragraphs, -ly adverbs, repeated sentence
  openers, Flesch readability
- how chapters end: on dialogue, a question, or a short line

Front matter (copyright, "Also by", contents…) is skipped, here and on EPUB
import. Gamebook choice lines ("If you run, turn to 12.") aren't counted as prose.

Give books a **weight** and they blend into your **target** (say 60 Weir / 40
Cline). **Compare** opens with **Against your target**: one card per measure
(your number, the target's, where each book sits, and a plain sentence),
biggest gaps first, recalculated as you write. Below that, every book, the
target and your manuscript in one table, plus sentence-length and
chapter-length charts.

**Your own numbers.** **Set a target…** (or **Target…**) under a scene's
title takes a chapter length (a range or one number), a median sentence, a
dialogue share and a word goal for the whole manuscript. They're saved with
the manuscript, win over the Library blend wherever both give a number, and
need no books at all. The word goal adds a progress bar under the
manuscript's name in the outline, counting each page's live take. Agents use
`set_target`.

Scanned EPUBs (one page per file, no paragraph breaks) are rejoined and cut at
"Chapter 1, 2, …"; their paragraph numbers are left out rather than guessed.

While you write, the strip under a scene's title shows its words against the
target's chapter range, its median sentence and dialogue share against the
target, and underlines **stretches** that drift:

- four or more long sentences in a row (wavy)
- a single sentence longer than ~99% of the target's
- a paragraph far denser than the target's
- three sentences in a row opening with the same word (four for I/you/he/she…)

Hover a mark for why. They're prompts, not rules: a long sentence can be the
right one. Hide them from the strip. Agents get the same numbers through the
`profile`, `library_add` and `cadence` tools.

## Scripts

Add a screenplay to the Library from a link (IMSDb pages work) or a .txt,
.fountain or .pdf. It's read into scene headings, action, character cues,
parentheticals and dialogue, using capitals and blank lines rather than
indentation, and measured: pages (55 lines each; The Martian estimates 119.8
against its real 118), scene lengths in eighths, dialogue vs action, speech and
action-block lengths, speeches per page, interiors and nights, locations, and
each character's share of the dialogue. Again, numbers only.

**Book to script** lines a book up against its film. TF-IDF matches each
scene to the chapter it draws on, with a pass that prefers the book's order
but lets the film jump (it finds The Martian's Chapter 12 flashback to the
storm). Short bridges between scenes from one chapter join that chapter. You
get chapters cut, pages per chapter, scenes with no clear source, and the
dialogue kept word for word (six-word runs found in the book). Both sources
are reread each time; nothing extra is stored.

## Appearance

Settings → **Theme**: match the system, light, dark, or **Custom**. A custom
theme starts from the light or dark palette and sets five colors (sidebar,
background, page, text, accent), like Slack's; muted text, rules and the desk
are mixed from those, and a warning shows if text and page are too close to
read comfortably. Presets: Paper, Night, Ocean, Rose, Forest, Midnight.

## Formatting and screenplays

⌘B and ⌘I wrap the selection in `**bold**` and `*italic*`. Alignment is saved
as plain-text markers: `> centered <` (⌘⇧E) and `>> right` (⌘⇧R); ⌘⇧L clears
it. Exports center and right-align those lines; narration ignores the markers.

A document with a scene heading (`INT.`, `EXT.`…) at the start of a line is a
**screenplay** in [Fountain](https://fountain.io): the editor lays it out like a
script (Courier, headings bold, character names and dialogue indented,
transitions right). **Tab** turns the current line into the next kind
(action → character → parenthetical → transition → scene heading), and Enter
after dialogue or a heading starts a new paragraph. Type a name, press Tab, and
it becomes a character cue.

**Table read.** ▶ Listen on a screenplay reads headings and action in the
narrator's voice and each character's lines in their own. The **Cast** panel
picks voices: new speakers are cast automatically, matching a male or female
voice only when the script's own action lines say "he" or "she" about them
(never from the name), and alternating otherwise; "Recast everyone" shuffles.
Casts are saved per manuscript or note in `.writepad/cast.json`.

In the Library, **Open** on a script copies it into the vault as a manuscript
(Fountain, in about five-page blocks), ready to read or table-read.

## World

The story's places, people, relationships and timeline, as ordinary notes
under `world/` (Obsidian-compatible frontmatter):

    world/World.md                 type: world   (manuscripts: ["The Last Thinker"] scopes it)
    world/characters/June.md       aliases, relationships: [{ to: "[[Dolores]]", kind: learns from }]
    world/locations/Unit 12.md
    world/relationships/…          between: ["[[A]]", "[[B]]"], kind
    world/timeline/The van.md      after: "[[previous event]]", when, scenes: ["[[The Last Thinker#1]]"]
                                   timeline: "1999" (optional), takes: ["Draft 2"] (optional)

- **Graph**: links, relationships, the event chain, and where each name or
  alias appears in the manuscript. Color + shape + label per type
  (validated palette); scenes are an opt-in layer.
- **Timeline**: a linked list. Each event points at the one before it;
  two events after the same one are a branch. Insert between, drag to
  move, remove (the gap closes). § chips show where each event is told.
- **Named timelines**: `timeline: 1999` on an event starts a timeline, and
  the events after it inherit it: a parallel world, another era, a story
  within the story. Each is its own section with its own start. An event
  that names a different timeline from the one it follows branches into it
  ("branches from X"). **+ New timeline** starts one; dragging an event
  after another moves it onto that event's timeline.
- **Read against** a manuscript or one of its **cuts**: events are compared
  with that telling's order. The ones outside the longest run told in order
  are flagged **told earlier** or **told later** (a flashback); events the
  telling leaves out are dimmed. A cut tells the blocks in its order, with
  its picked takes. `takes: [Luke]` on an event means only those takes tell
  it, for a block whose versions don't all include it.
- **Story vs telling** draws both orders side by side, one line per event;
  the lines that cut across the rest are the ones told out of order.
- **In this scene** (while writing) lists who and what is named in the block;
  **Connections** (on an entry) lists relationships, every scene it's in, and
  backlinks. **To look at** flags links with no note yet and names that never
  appear.
- Agents: `world`, `world_entry`, `scene_cast`, `timeline` (with
  `manuscript` and `cut` to read against a telling), `world_add`,
  `timeline_add` (with `timeline` to start or branch into one). What they
  add arrives as `status: draft`.

### Filling in the world without AI

**World → Suggest…** reads a manuscript with plain rules and lists the
characters and locations it finds, each with how often and where. Untick the
ones you don't want; the rest become drafts in World to keep, edit or delete.

- **Screenplays** are exact: the names above dialogue are the characters, and
  scene headings are the places (`INT. WRIGHT FARMHOUSE - KITCHEN` → Wright
  Farmhouse).
- **Prose** is rules, not a model: capitalised names that recur away from the
  start of a sentence or carry a title (Mrs., Sergeant-Major, St.), who "said"
  what, and places after in/at/to or named like one (Laburnam Villa). Variants
  merge (Herbert → Herbert White, Morris → Sergeant-Major Morris); "Mr. White"
  stays apart from "Herbert White", since families share surnames. Roles like
  "the old man" aren't found.
- **Also start a timeline** adds one draft event per scene, in order, to rename.

**World → Import CSV…** takes a spreadsheet with a header row
`type,name,aliases,description` (aliases separated by `;`; types character,
location, thing) and makes a note for each row.

Agents and the CLI: `world_suggest`, `world_suggest_apply`, `world_import_csv`.

## History (git)

Snapshots are commits in the vault's own git repository (*Turn on history*
runs `git init` if the vault isn't one yet), so any git tool can read them and
the repo can be pushed for backup or sync.

- **Save** a named version, or let **autosave** snapshot after 10 minutes of edits.
- The History panel lists past versions of the open note or block (blocks are
  read out of the manuscript JSON in each snapshot). Click one to onion-skin
  against it; **Restore as new take** (blocks) or **Restore** (notes, undoable).
- **Roll back whole vault** snapshots the present first, restores every
  tracked file, and commits that as a new snapshot. History is never rewritten.

## Export

**Export** (top bar) writes the open manuscript's live cut (outline order,
★ live versions) or the open note:

- **Word (.docx)**, **OpenDocument (.odt)** and **Rich Text (.rtf)**: a title
  page, each chapter on a new page under a real Heading 1 (so Word's navigation
  pane lists them), first paragraphs unindented, bold, italics, centered lines
  and scene breaks kept. Georgia 12 pt, 1.5 spacing, US Letter with 1″ margins.
  Nothing extra to install.
- **Web page (.html)**: the whole book as one readable page.
- **EPUB 3**: title page, nav, one XHTML file per chapter, italics kept.
- **PDF**: 5.5×8.5″ book typeset by headless Chrome (needs Chrome, Chromium,
  Edge or Brave installed).
- **Markdown**, **plain text**.
- **MP3 audiobook**: one file per chapter, narrated with your voices, speed,
  italic voice and pronunciations, encoded with LAME (96 kbps mono, ID3 tags).
  Sentences come from the narration cache, so anything you've already listened
  to doesn't render again.

## Narration

- `⌘↵` (or *Listen*) reads from the cursor, or just the selection. `Esc` stops.
  The sentence being read is highlighted.
- Lines wholly in *italics* use a second voice (Settings).
- *A/B listen* plays the compared version, then this one.
- **Pronunciations** fix names: respell (`WER-bose`) or IPA between slashes.
  Stored in `<vault>/.writepad/lexicon.json`.
- Engine: Kokoro via `tts/kokoro_worker.py`, a long-lived Python process the
  Rust side talks to over JSON lines. Sentences are cached as WAVs in the app
  cache dir, so after an edit only changed sentences re-render. Defaults point
  at `~/Documents/ChooseYourAdventure/.venv-tts` and `tts_models/`; change them
  in Settings.

## Layout

    crates/core/     writepad-core: manuscript store, history, import/export, diff, narration
    crates/cli/      writepad: CLI + MCP server (tools.rs, mcp.rs)
    src-tauri/src/   the app: commands (lib.rs), watch.rs, dictate.rs, settings.rs
    src/             main.ts (UI) · editor.ts (CodeMirror + overlays) · narrator.ts · sentences.ts
    tts/             kokoro_worker.py

## Next

- Pure-Rust narration (ONNX via `ort` + espeak-ng), dropping the Python worker
- Per-character voices for dialogue
- Graph view of links / gamebook "turn to N" branches; backlinks
