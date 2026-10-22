"""Build the WRITEpad demo vault (The Monkey's Paw) with the app's own CLI and core.

    python demo-source/build.py [--zip WRITEpad-Demo-Vault.zip]

Builds the `writepad` CLI and demo-source/helper, then writes every file through
the same code the app uses, taking a history snapshot after each step. The vault
is built fresh in a temp folder, zipped with its history if asked (that's the
release asset), and copied without its .git into the repo's demo/ folder, ready
to open.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
SRC = HERE / "src"
EXE = ".exe" if sys.platform == "win32" else ""
CLI = ROOT / "target" / "release" / f"writepad{EXE}"
HELP = ROOT / "target" / "release" / f"demo-helper{EXE}"
args = sys.argv[1:]
ZIP = None
if "--zip" in args:
    i = args.index("--zip")
    ZIP = Path(args[i + 1]).resolve()
    del args[i : i + 2]
if args:
    sys.exit(__doc__)
OUT = ROOT / "demo"
BUILD = Path(tempfile.mkdtemp(prefix="writepad-demo-"))
V = BUILD / "WRITEpad Demo"

subprocess.run(["cargo", "build", "--release", "-p", "writepad-cli"], cwd=ROOT, check=True)
subprocess.run(["cargo", "build", "--release", "--manifest-path", str(HERE / "helper" / "Cargo.toml"), "--target-dir", str(ROOT / "target")], check=True)
MS = "The Monkey's Paw"
SP = "The Monkey's Paw (screenplay)"
SX = "The Sexton's Bell"
TR = "Trifles (table read)"


def cli(tool, args, agent=None, vault=None):
    cmd = [str(CLI), "--vault", str(vault or V)]
    if agent is not None:
        cmd += ["--agent", agent]
    out = subprocess.run(cmd + ["call", tool, json.dumps(args)], capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(f"{tool} failed: {out.stderr}{out.stdout}")
    return json.loads(out.stdout) if out.stdout.strip() else None


def helper(*args):
    out = subprocess.run([str(HELP), str(V), *args], capture_output=True, text=True, encoding="utf-8")
    if out.returncode != 0:
        sys.exit(f"helper {args[0]} failed: {out.stderr}")


def note(rel, text):
    p = V / rel
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(text.lstrip("\n"), encoding="utf-8", newline="\n")


def fm(**props):
    lines = []
    for k, v in props.items():
        if isinstance(v, list) and v and isinstance(v[0], dict):
            lines.append(f"{k}:")
            lines += [f"  - {{ to: {json.dumps(r['to'])}, kind: {json.dumps(r['kind'])} }}" for r in v]
        else:
            lines.append(f"{k}: {json.dumps(v, ensure_ascii=False) if not isinstance(v, str) or k != 'type' else v}")
    return "---\n" + "\n".join(lines) + "\n---\n\n"


def scene(block):
    return f"[[{MS}#{block}]]"


V.mkdir(parents=True)

# 1. History on, then the story itself.
helper("history-on")
ms_id = cli("import", {"title": MS, "paths": [str(SRC / "Jacobs 1902.md")]})["manuscript"]
helper("snapshot", "Imported The Monkey's Paw (W. W. Jacobs, 1902)")

# 2. The world: people, places, things, relationships, and the timeline.
note("world/World.md", fm(type="world", manuscripts=[MS, SP]) + """
# The Monkey's Paw

England, around 1900. A retired couple and their grown son live at [[Laburnam Villa]], at the far end of an unfinished road where only two houses are let. An old friend back from [[India]] brings them [[The monkey's paw]].

**The rule of the paw:** three men may each have three wishes from it, and every wish comes true in a way that seems like coincidence. The [[The fakir|fakir]] made it to show that fate rules people's lives, and that those who interfere with it do so to their sorrow.

**Who's who:** [[Mr. White]], [[Mrs. White]], [[Herbert White]], [[Sergeant-Major Morris]], [[The man from Maw and Meggins]].
""")

people = [
    ("Mr. White", ["the old man", "his father"], [("Mrs. White", "married to"), ("Herbert White", "father of"), ("Sergeant-Major Morris", "old friend of")],
     "Retired, gentle, a bad chess player who plays riskily and sulks about the road. He takes the paw out of the fire and makes all three wishes. Credulous, then ashamed of it, then terrified.\n\n- Wants: nothing much. *\"It seems to me I've got all I want.\"* That's the tragedy: the first wish is Herbert's idea.\n- Last seen: running to the gate in [[The knocking]]."),
    ("Mrs. White", ["the old woman", "the old lady"], [("Herbert White", "mother of")],
     "Sharp, warm, the first to laugh at the paw and the first to reach for it in grief. She asks for the second wish, *\"Do you think I fear the child I have nursed?\"*, and unbolts the door."),
    ("Herbert White", ["Herbert"], [("Maw and Meggins", "works at")],
     "The son, in his twenties. Jokey and sceptical; suggests the two hundred pounds as a lark. Killed in the machinery at [[Maw and Meggins]] the next day. The firm pays exactly two hundred pounds."),
    ("Sergeant-Major Morris", ["Morris", "the sergeant-major", "the soldier"], [("India", "served in")],
     "Twenty-one years a soldier in [[India]]; left as \"a slip of a youth in the warehouse\". Has used his three wishes and won't say what they cost. Throws the paw on the fire and warns them twice."),
    ("The man from Maw and Meggins", ["the stranger"], [("Maw and Meggins", "speaks for")],
     "Well dressed, new silk hat, paces the gate three times before he opens it. Brings the news and the money, and says twice that he is only obeying orders."),
    ("The fakir", ["fakir"], [("India", "from")],
     "Never seen. A holy man who put the spell on the paw to prove a point about fate."),
]
for name, aliases, rels, body in people:
    note(f"world/characters/{name}.md", fm(type="character", aliases=aliases, relationships=[{"to": f"[[{t}]]", "kind": k} for t, k in rels]) + f"# {name}\n\n{body}\n")

places = [
    ("Laburnam Villa", ["the parlour"], "The Whites' house, \"so far out\" that the pathway's a bog and the road's a torrent. The parlour (fire, chessboard, piano, the bracket where the paw sits) is where nearly everything happens. The front door has a chain, a bottom bolt and a top bolt Mrs. White can't reach."),
    ("India", [], "Where [[Sergeant-Major Morris]] served, and where the [[The fakir|fakir]] made the paw. Mr. White would like to go \"just to look round a bit\"."),
    ("Maw and Meggins", ["Maw and Meggins"], "The firm where [[Herbert White]] works, and where he is \"caught in the machinery\". It disclaims all responsibility and pays two hundred pounds."),
    ("The new cemetery", ["new cemetery"], "\"Some two miles distant.\" Herbert is buried here, which is why the knocking takes so long to arrive."),
]
for name, aliases, body in places:
    note(f"world/locations/{name}.md", fm(type="location", aliases=aliases) + f"# {name}\n\n{body}\n")

note("world/things/The monkey's paw.md", fm(type="thing", aliases=["the paw", "the talisman", "monkey’s paw"]) + """
# The monkey's paw

"Just an ordinary little paw, dried to a mummy." Hold it up in your right hand and wish aloud. It twists in Mr. White's hand on the first wish.

| Owner | Wishes | What we know |
|---|---|---|
| The first man | 3 | The third was for death. |
| [[Sergeant-Major Morris]] | 3 | Granted. He won't say more. |
| [[Mr. White]] | 3 | £200, Herbert alive, Herbert gone. |
""")
note("world/things/Two hundred pounds.md", fm(type="thing", aliases=["two hundred pounds"]) + "# Two hundred pounds\n\nEnough to clear the mortgage on [[Laburnam Villa]]. Herbert's suggestion; [[Maw and Meggins]]' compensation.\n")

note("world/relationships/Mr. and Mrs. White.md", fm(type="relationship", between=["[[Mr. White]]", "[[Mrs. White]]"], kind="married nearly forty years") + "He presses her hand \"as he had been wont to do in their old courting-days\". By Part III he is afraid of her.\n")
note("world/relationships/Mr. White and Morris.md", fm(type="relationship", between=["[[Mr. White]]", "[[Sergeant-Major Morris]]"], kind="old friends") + "Morris went to India from the warehouse twenty-one years ago. White pays him \"a trifle\" for the paw against his wishes.\n")

events = [
    ("The fakir's spell", None, "Years before, India", ["The paw"], "The [[The fakir|fakir]] puts a spell on [[The monkey's paw]]: three men, three wishes each."),
    ("The first owner wishes for death", "The fakir's spell", "Years before", ["The paw"], "We never learn his first two wishes. His third is for death. That's how Morris got the paw."),
    ("Morris has his three wishes", "The first owner wishes for death", "During his years in India", ["The paw"], "[[Sergeant-Major Morris]] uses all three. His face whitens when he admits it."),
    ("Morris visits Laburnam Villa", "Morris has his three wishes", "A wet winter night", ["The visitor"], "[[Sergeant-Major Morris]] arrives at [[Laburnam Villa]], drinks three whiskies, and tells stories of [[India]]."),
    ("Mr. White saves the paw from the fire", "Morris visits Laburnam Villa", "The same night", ["The paw"], "Morris throws [[The monkey's paw]] on the fire; [[Mr. White]] snatches it off and keeps it."),
    ("First wish (two hundred pounds)", "Mr. White saves the paw from the fire", "The same night, after supper", ["The first wish"], "At [[Herbert White]]'s suggestion, [[Mr. White]] wishes for [[Two hundred pounds]]. The paw twists in his hand."),
    ("Herbert sees a face in the fire", "First wish (two hundred pounds)", "Late that night", ["The first wish"], "Alone, [[Herbert White]] sees a simian face in the coals and touches the paw by accident."),
    ("Herbert goes to work", "Herbert sees a face in the fire", "Next morning", ["Morning"], "\"Don't break into the money before I come back.\" [[Mrs. White]] watches him down the road."),
    ("Herbert is caught in the machinery", "Herbert goes to work", "Next day, at the works", ["Maw and Meggins"], "Offstage. We only hear about it from [[The man from Maw and Meggins]]."),
    ("The visit from Maw and Meggins", "Herbert is caught in the machinery", "Next day, dinner time", ["Maw and Meggins"], "He brings the news and the firm's compensation: exactly [[Two hundred pounds]]."),
    ("The funeral", "The visit from Maw and Meggins", "A few days later", ["The second wish"], "Herbert is buried in [[The new cemetery]], two miles away."),
    ("Second wish (my son alive again)", "The funeral", "About a week later; ten days after the death", ["The second wish"], "[[Mrs. White]] remembers the paw in the night. [[Mr. White]] wishes Herbert alive. Nothing happens, at first."),
    ("The knocking", "Second wish (my son alive again)", "The same night", ["The second wish", "The knocking"], "A quiet knock, then louder. [[Mrs. White]] runs to unbolt the door."),
    ("Third wish", "The knocking", "The same night", ["The knocking"], "As the bolt comes back, [[Mr. White]] finds the paw on the floor and breathes his last wish. We never hear it."),
    ("The empty road", "Third wish", "The same night", ["The knocking"], "The knocking stops. [[Mrs. White]] opens the door on \"a quiet and deserted road\" under a flickering street lamp, and wails."),
]
for name, after, when, scenes, body in events:
    props = {"type": "event"}
    if after:
        props["after"] = f"[[{after}]]"
    props["when"] = when
    props["scenes"] = [scene(s) for s in scenes]
    note(f"world/timeline/{name}.md", fm(**props) + body + "\n")

# Notes outside the world: research, craft, a reading list.
for name in ["Start here", "Themes", "Why the knocks work", "About W. W. Jacobs", "Adaptation notes"]:
    note(f"{'' if name == 'Start here' else 'notes/'}{name}.md", (SRC / "notes" / f"{name}.md").read_text(encoding="utf-8"))
helper("snapshot", "World: people, places, the paw, and the timeline")

# 3. A revision of one scene, in two passes, so History has versions of it.
fw = cli("fork", {"manuscript": ms_id, "block": "The first wish", "label": "Modern pass"}, agent="")["take"]
first = (V / ".writepad" / "manuscripts" / ms_id / "variants" / f"{fw}.md").read_text(encoding="utf-8")
first = first.replace("that ’ll just do it", "that’ll just do it").replace("eyed it dubiously", "turned it over").replace("I bet I never shall", "I bet I never will")
cli("write", {"manuscript": ms_id, "take": fw, "text": first}, agent="")
helper("snapshot", "Modern pass on The first wish: light touch")
cli("write", {"manuscript": ms_id, "take": fw, "text": (SRC / "Modern pass.md").read_text(encoding="utf-8").split("\n", 2)[2]}, agent="")
helper("snapshot", "Modern pass on The first wish: tighter, and his thought in italics")

# 4. Agents' takes (they show with a robot in the app).
cli("fork", {"manuscript": ms_id, "block": "The visitor", "label": "tighter opening", "text": (SRC / "takes" / "The visitor - tighter opening.md").read_text(encoding="utf-8")}, agent="line-editor")
cli("fork", {"manuscript": ms_id, "block": "The knocking", "label": "alt ending: the door opens", "text": (SRC / "takes" / "The knocking - alt ending.md").read_text(encoding="utf-8")}, agent="story-doctor")
cli("timeline_add", {"name": "Alt ending (the door opens)", "after": "The knocking", "when": "The same night (alternate ending)",
                     "scenes": [f"{MS}#The knocking"], "body": "Branch for the story-doctor's alternate take of [[The knocking]]: [[Mrs. White]] gets the bolt back before the third wish. See the take labelled \"alt ending: the door opens\"."}, agent="story-doctor")
cli("world_add", {"type": "character", "name": "The first owner", "aliases": ["The first man", "first man"],
                  "body": "# The first owner\n\nThe man who had the paw before [[Sergeant-Major Morris]]. \"I don't know what the first two were, but the third was for death.\" Told in [[The Monkey's Paw#The paw]]."}, agent="world-builder")
helper("snapshot", "Agents: line-editor and story-doctor takes, two world drafts")

# 5. Cuts: the original, the modern pass, and a cold open.
m = json.loads((V / ".writepad" / "manuscripts" / ms_id / "manuscript.json").read_text(encoding="utf-8"))
by_title = {b["title"]: b for b in m["blocks"]}
orig = next(v["id"] for v in by_title["The first wish"]["variants"] if v["label"] == "Jacobs 1902")
helper("set-live", ms_id, orig)
helper("save-cut", ms_id, "Jacobs 1902")
helper("set-live", ms_id, fw)
helper("save-cut", ms_id, "Modern pass")
order = [b["id"] for b in m["blocks"]]
knock = by_title["The knocking"]["id"]
cold = [order[0], knock] + [b for b in order[1:] if b != knock]
helper("set-order", ms_id, *cold)
helper("set-live", ms_id, orig)
helper("save-cut", ms_id, "Cold open")
helper("apply-cut", ms_id, "Jacobs 1902")
helper("snapshot", "Cuts: Jacobs 1902, Modern pass, Cold open")

# 6. The screenplay, its cast, and pronunciations.
sp_id = cli("import", {"title": SP, "paths": [str(SRC / "Draft 1.md")]})["manuscript"]
cast = {f"ms:{sp_id}": {"__narrator": "bf_emma", "MR. WHITE": "bm_george", "MRS. WHITE": "bf_isabella", "HERBERT": "am_michael",
                        "MORRIS": "bm_lewis", "STRANGER": "bm_daniel"}}
(V / ".writepad" / "cast.json").write_text(json.dumps(cast, indent=2), encoding="utf-8")
lex = {"Laburnam": "luh-BUR-num", "Meggins": "MEG-inz", "fakir": "fuh-KEER", "antimacassar": "AN-tee-muh-KASS-er"}
(V / ".writepad" / "lexicon.json").write_text(json.dumps(lex, indent=2), encoding="utf-8")
helper("snapshot", "Screenplay draft 1, cast for the table read, pronunciations")

# 7. Library: three stories measured into numbers and blended into a target,
#    and the screenplay lined up against the story it adapts.
lib = V / "library"
lib.mkdir(exist_ok=True)
cli("export", {"manuscript": ms_id, "format": "epub", "path": str(lib / "The Monkey's Paw.epub"), "author": "W. W. Jacobs"})
with tempfile.TemporaryDirectory() as tmp:  # the other two only pass through a vault on the way to EPUB
    for title, author in [("The Signal-Man", "Charles Dickens"), ("The Tell-Tale Heart", "Edgar Allan Poe")]:
        mid = cli("import", {"title": title, "paths": [str(SRC / "library" / f"{title}.md")]}, vault=tmp)["manuscript"]
        cli("export", {"manuscript": mid, "format": "epub", "path": str(lib / f"{title}.epub"), "author": author}, vault=tmp)
books = {}
for title, author in [(MS, "W. W. Jacobs"), ("The Signal-Man", "Charles Dickens"), ("The Tell-Tale Heart", "Edgar Allan Poe")]:
    books[title] = cli("library_add", {"path": str(lib / f"{title}.epub"), "title": title, "author": author})
helper("weights", json.dumps({books[MS]["id"]: 40, books["The Signal-Man"]["id"]: 30, books["The Tell-Tale Heart"]["id"]: 30}))
screenplay = (SRC / "Draft 1.md").read_text(encoding="utf-8")
fountain = "\n".join(l for l in screenplay.splitlines() if not l.startswith("## ")).lstrip("\n")
(lib / "The Monkey's Paw.fountain").write_text(fountain + "\n", encoding="utf-8", newline="\n")
script = cli("script_add", {"source": str(lib / "The Monkey's Paw.fountain"), "title": SP, "writer": "WRITEpad demo"})
for lock in V.rglob("*.lock"):
    lock.unlink()
helper("snapshot", "Library: Jacobs, Dickens and Poe blended 40/30/30; screenplay added")

# 8. An original story written toward that target: a draft, then a revision
#    that closes the gaps Compare showed. Its own small world.
sx_id = cli("import", {"title": SX, "paths": [str(SRC / "sexton" / "Draft 1.md")]})["manuscript"]
helper("snapshot", "The Sexton's Bell: draft 1, written without looking at the numbers")
sx = json.loads((V / ".writepad" / "manuscripts" / sx_id / "manuscript.json").read_text(encoding="utf-8"))
revised = (SRC / "sexton" / "Revised toward target.md").read_text(encoding="utf-8")
for b in sx["blocks"]:
    if b.get("raw"):
        continue
    text = revised.split(f"## {b['title']}\n\n", 1)[1].split("\n## ", 1)[0].rstrip() + "\n"
    take = cli("fork", {"manuscript": sx_id, "block": b["id"], "label": "Revised toward target", "text": text}, agent="")["take"]
    helper("set-live", sx_id, take)
note("world/The Sexton's Bell.md", fm(type="world", manuscripts=[SX]) + """
# The Sexton's Bell

A churchyard on the edge of a railway cutting, some time in the 1880s. Graves in the bell-row have "safety coffin" bells: a cord from the dead man's hand to a bell above ground, in case he wakes.

Written toward the Library's 40/30/30 target.
""")
for name, kind, aliases, body in [
    ("Tom", "character", ["Tom"], "The narrator. Under-sexton at nineteen, sexton for forty years after. Keeps his promise to [[Mrs. Fenn]]."),
    ("Abel Crane", "character", ["Abel", "Crane"], "Sexton of [[St. Aldric's]] for thirty years. Tied off every cord in [[The bell-row]] so the trains wouldn't wake him."),
    ("Mrs. Fenn", "character", ["Mrs. Fenn"], "The corn-chandler's widow. Pays in gold for a promise: \"If it rings, you will go to him.\""),
    ("St. Aldric's", "location", ["St. Aldric's"], "A churchyard along the top of a railway cutting. The goods train passes at twenty to twelve."),
    ("The bell-row", "location", ["bell-row"], "Twelve graves on the north wall with bells instead of stones. Then thirteen."),
]:
    note(f"world/{'characters' if kind == 'character' else 'locations'}/{name}.md", fm(type=kind, aliases=aliases) + f"# {name}\n\n{body}\n")
for name, after, scenes, body in [
    ("Fenn is buried with a bell", None, ["The bell-row"], "[[Tom]] promises [[Mrs. Fenn]] he will go to her husband if the bell rings."),
    ("Abel ties off Fenn's cord", "Fenn is buried with a bell", ["The quiet night"], "[[Abel Crane]] with his arm down the pipe. \"Rat in the pipe.\""),
    ("The bell rings with no train", "Abel ties off Fenn's cord", ["The quiet night"], "Snow closes the line. On the third night the twelfth bell rings."),
    ("Tom opens the grave", "The bell rings with no train", ["The cord"], "The knot is untied, held loose in Fenn's fingers."),
    ("Abel is buried in the bell-row", "Tom opens the grave", ["The cord"], "[[Tom]] threads the cord and does not tie it off."),
]:
    props = {"type": "event"}
    if after:
        props["after"] = f"[[{after}]]"
    props["when"] = "1880s, winter"
    props["scenes"] = [f"[[{SX}#{x}]]" for x in scenes]
    note(f"world/timeline/{name}.md", fm(**props) + body + "\n")
note("notes/Writing toward a target.md", (SRC / "notes" / "Writing toward a target.md").read_text(encoding="utf-8"))
helper("snapshot", "The Sexton's Bell: revised toward the 40/30/30 target, with its world")

# 9. A public-domain play laid out as Fountain: five voices for a table read.
tr_id = cli("import", {"title": TR, "paths": [str(SRC / "Trifles.md")]})["manuscript"]
cast[f"ms:{tr_id}"] = {"__narrator": "af_heart", "COUNTY ATTORNEY": "am_adam", "SHERIFF": "am_eric", "HALE": "am_fenrir",
                        "MRS PETERS": "af_sarah", "MRS HALE": "af_bella"}
(V / ".writepad" / "cast.json").write_text(json.dumps(cast, indent=2), encoding="utf-8")
helper("snapshot", "Trifles (Glaspell, 1916) as a five-voice table read")

for lock in V.rglob("*.lock"):
    lock.unlink()
if ZIP:
    shutil.make_archive(str(ZIP.with_suffix("")), "zip", V.parent, V.name)
    print("zip:", ZIP)
# The repo's copy: generated, so replaced wholesale. No .git (a repo can't hold one).
shutil.rmtree(OUT, ignore_errors=True)
shutil.copytree(V, OUT, ignore=shutil.ignore_patterns(".git"))
shutil.rmtree(BUILD, ignore_errors=True)
print("vault:", OUT)
print("manuscript:", ms_id, "screenplay:", sp_id)
print("books:", {k: v["id"] for k, v in books.items()}, "script:", script["id"], "sexton:", sx_id, "trifles:", tr_id)
