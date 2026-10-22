# Demo vault: The Monkey's Paw

A vault that shows everything WRITEpad does, built around W. W. Jacobs's
*The Monkey's Paw* (1902, public domain). Download **WRITEpad-Demo-Vault.zip**
from the [Releases](https://github.com/jasonsutter87/writepad/releases) page,
unzip it, open the folder in **Settings → Vault**, and read **Start here**.

What's in it:

- **Manuscript** in seven scenes, with a second "Modern pass" take of *The
  first wish* (onion skin, italic voice), two agent takes (*line-editor*,
  *story-doctor*), and three saved cuts.
- **World**: characters, places, things, relationships, and a 16-event
  timeline linked to scenes, with a branch for the alternate ending and two
  agent drafts to review.
- **Screenplay** in Fountain with a cast for the table read, plus pronunciations.
- **Library**: *The Monkey's Paw*, Dickens's *The Signal-Man* and Poe's *The
  Tell-Tale Heart* blended 40/30/30 into a target; Book to script between the
  screenplay and the story.
- **Write toward a target**: *The Sexton's Bell*, an original story in three
  short chapters, each with a first draft and a revision toward the target
  (numbers in *Writing toward a target*).
- **Table read**: Glaspell's *Trifles* (1916) in Fountain, five voices.
- **History**: the vault is a git repo, one snapshot per build step.

## Rebuilding it

    python demo/build.py "path/to/WRITEpad Demo" --zip WRITEpad-Demo-Vault.zip

Needs Rust. It
builds the `writepad` CLI and `demo/helper` (history, cuts and the target,
which the CLI doesn't expose), then writes everything through them.

- `src/Jacobs 1902.md`: the story (Project Gutenberg #12122), one `##` per scene
- `src/Modern pass.md`: the revised *The first wish*
- `src/takes/`: the agents' takes
- `src/Draft 1.md`: the screenplay, one `##` per act (the `.fountain` is made from it)
- `src/library/`: *The Signal-Man* (Gutenberg #1289) and *The Tell-Tale Heart* (#2148), split into scenes
- `src/sexton/`: *The Sexton's Bell*, draft 1 and the revision
- `src/Trifles.md`: *Trifles* (Gutenberg #10623) in Fountain
- `src/notes/`: *Start here* and the notes
- `build.py`: world notes, timeline, cast, pronunciations and the build order
