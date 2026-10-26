// The World views: a force-directed knowledge graph and the timeline (a
// linked list of events, with branches). Both are plain DOM/SVG; main.ts
// owns the data and passes handlers in.
import { forceCenter, forceCollide, forceLink, forceManyBody, forceSimulation, type SimulationNodeDatum } from "d3-force";
import type { GraphEdge, GraphNode, Telling, TimelineEvent, WorldGraph } from "./api";

/** Each kind: legend label, folder plural, and a shape. Color is `--k-<kind>` in CSS. */
export const KINDS: { kind: string; label: string; plural: string; shape: Shape }[] = [
  { kind: "world", label: "World", plural: "World", shape: "hexagon" },
  { kind: "character", label: "Character", plural: "Characters", shape: "circle" },
  { kind: "location", label: "Location", plural: "Locations", shape: "square" },
  { kind: "event", label: "Event", plural: "Timeline", shape: "diamond" },
  { kind: "relationship", label: "Relationship", plural: "Relationships", shape: "ring" },
  { kind: "thing", label: "Thing", plural: "Things", shape: "triangle" },
  { kind: "scene", label: "Scene", plural: "Scenes", shape: "page" },
];

type Shape = "circle" | "square" | "diamond" | "triangle" | "hexagon" | "ring" | "page";

const kindMeta = (kind: string) => KINDS.find((k) => k.kind === kind) ?? { kind, label: kind, plural: kind, shape: "circle" as Shape };

/** SVG path for a shape of radius r centered on 0,0. */
export function shapePath(shape: Shape, r: number): string {
  switch (shape) {
    case "square":
      return `M${-r * 0.85},${-r * 0.85}h${r * 1.7}v${r * 1.7}h${-r * 1.7}z`;
    case "diamond":
      return `M0,${-r * 1.15}L${r * 1.15},0L0,${r * 1.15}L${-r * 1.15},0z`;
    case "triangle":
      return `M0,${-r * 1.15}L${r * 1.1},${r * 0.8}L${-r * 1.1},${r * 0.8}z`;
    case "hexagon": {
      const pts = Array.from({ length: 6 }, (_, i) => {
        const a = (Math.PI / 3) * i;
        return `${(Math.cos(a) * r * 1.1).toFixed(2)},${(Math.sin(a) * r * 1.1).toFixed(2)}`;
      });
      return `M${pts.join("L")}z`;
    }
    case "page":
      return `M${-r * 0.7},${-r * 0.9}h${r * 1.4}v${r * 1.8}h${-r * 1.4}z`;
    default: // circle and ring
      return `M${-r},0a${r},${r} 0 1,0 ${r * 2},0a${r},${r} 0 1,0 ${-r * 2},0`;
  }
}

/** A small inline SVG swatch for lists and legends. */
export function swatch(kind: string, size = 12): SVGSVGElement {
  const m = kindMeta(kind);
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("viewBox", "-8 -8 16 16");
  svg.setAttribute("width", String(size));
  svg.setAttribute("height", String(size));
  svg.classList.add("swatch");
  const p = document.createElementNS("http://www.w3.org/2000/svg", "path");
  p.setAttribute("d", shapePath(m.shape, 6));
  p.setAttribute("class", `mark k-${kind}${m.shape === "ring" ? " ring" : ""}`);
  svg.append(p);
  return svg;
}

const SVG = "http://www.w3.org/2000/svg";
const svgEl = <K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}) => {
  const e = document.createElementNS(SVG, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, String(v));
  return e;
};

type SimNode = GraphNode & SimulationNodeDatum & { degree: number };
type SimLink = { source: SimNode; target: SimNode; edge: GraphEdge };

export interface GraphHandlers {
  open: (node: GraphNode) => void;
}

/** State that survives re-renders (filters, search, zoom). */
const view = {
  // Scenes are many; they're opt-in (each entry's Connections panel lists them too).
  hidden: new Set<string>(["scene"]),
  query: "",
  scale: 1,
  tx: 0,
  ty: 0,
};

let stopSim: (() => void) | null = null;

export function renderGraph(root: HTMLElement, data: WorldGraph, h: GraphHandlers) {
  stopSim?.();
  root.replaceChildren();

  // ---- toolbar: legend doubles as the type filter ----
  const bar = document.createElement("div");
  bar.className = "graph-bar";
  const present = new Set(data.nodes.map((n) => n.kind));
  for (const k of KINDS.filter((k) => present.has(k.kind))) {
    const chip = document.createElement("button");
    chip.type = "button";
    chip.className = `legend-chip${view.hidden.has(k.kind) ? " off" : ""}`;
    chip.title = `Show or hide ${k.plural.toLowerCase()}`;
    chip.append(swatch(k.kind), document.createTextNode(` ${k.plural} (${data.nodes.filter((n) => n.kind === k.kind).length})`));
    chip.onclick = () => {
      if (view.hidden.has(k.kind)) view.hidden.delete(k.kind);
      else view.hidden.add(k.kind);
      renderGraph(root, data, h);
    };
    bar.append(chip);
  }
  const search = document.createElement("input");
  search.placeholder = "Find…";
  search.value = view.query;
  search.className = "graph-search";
  bar.append(search);
  root.append(bar);

  const nodes: SimNode[] = data.nodes.filter((n) => !view.hidden.has(n.kind)).map((n) => ({ ...n, degree: 0 }));
  if (!nodes.length) {
    const empty = document.createElement("p");
    empty.className = "hint graph-empty";
    empty.textContent = data.nodes.length
      ? "Everything is filtered out."
      : "No world yet. Add characters, locations and events from the World tab, and they'll appear here with every scene they're in.";
    root.append(empty);
    return;
  }
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const links: SimLink[] = data.edges
    .filter((e) => byId.has(e.from) && byId.has(e.to))
    .map((e) => ({ source: byId.get(e.from)!, target: byId.get(e.to)!, edge: e }));
  for (const l of links) {
    l.source.degree++;
    l.target.degree++;
  }
  const neighbors = new Map<string, Set<string>>();
  for (const l of links) {
    (neighbors.get(l.source.id) ?? neighbors.set(l.source.id, new Set()).get(l.source.id)!).add(l.target.id);
    (neighbors.get(l.target.id) ?? neighbors.set(l.target.id, new Set()).get(l.target.id)!).add(l.source.id);
  }
  const radius = (n: SimNode) => (n.kind === "scene" ? 5 : 7 + Math.min(9, Math.sqrt(n.degree) * 1.6));

  // ---- svg ----
  const wrap = document.createElement("div");
  wrap.className = "graph-canvas";
  root.append(wrap);
  const { width, height } = wrap.getBoundingClientRect();
  const svg = svgEl("svg", { width: "100%", height: "100%" });
  const defs = svgEl("defs");
  const arrow = svgEl("marker", { id: "arrow", viewBox: "0 0 10 10", refX: 16, refY: 5, markerWidth: 6, markerHeight: 6, orient: "auto-start-reverse" });
  arrow.append(svgEl("path", { d: "M0,0L10,5L0,10z", class: "arrowhead" }));
  defs.append(arrow);
  svg.append(defs);
  const world = svgEl("g");
  svg.append(world);
  wrap.append(svg);
  const applyTransform = () => world.setAttribute("transform", `translate(${view.tx},${view.ty}) scale(${view.scale})`);
  if (!view.tx && !view.ty) {
    view.tx = width / 2;
    view.ty = height / 2;
  }
  applyTransform();

  const edgeLayer = svgEl("g", { class: "edges" });
  const nodeLayer = svgEl("g", { class: "nodes" });
  world.append(edgeLayer, nodeLayer);

  const lineEls = links.map((l) => {
    const line = svgEl("line", { class: `edge e-${l.edge.kind}` });
    if (l.edge.kind === "next") line.setAttribute("marker-end", "url(#arrow)");
    if (l.edge.kind === "mention") line.setAttribute("stroke-width", String(Math.min(3, 0.6 + Math.log2(1 + l.edge.weight) * 0.5)));
    edgeLayer.append(line);
    return line;
  });
  const relLabels = links.map((l) => {
    if (l.edge.kind !== "relation" || !l.edge.label) return null;
    const t = svgEl("text", { class: "edge-label" });
    t.textContent = l.edge.label;
    edgeLayer.append(t);
    return t;
  });

  const tip = document.createElement("div");
  tip.className = "graph-tip";
  tip.hidden = true;
  wrap.append(tip);

  const nodeEls = nodes.map((n) => {
    const g = svgEl("g", { class: `node n-${n.kind}`, tabindex: 0, role: "button" });
    g.setAttribute("aria-label", `${kindMeta(n.kind).label}: ${n.label}`);
    const m = kindMeta(n.kind);
    // An invisible larger hit target, then the mark with a surface ring.
    g.append(svgEl("circle", { r: radius(n) + 6, class: "hit" }));
    g.append(svgEl("path", { d: shapePath(m.shape, radius(n)), class: `mark k-${n.kind}${m.shape === "ring" ? " ring" : ""}` }));
    const label = svgEl("text", { y: radius(n) + 13, class: "node-label" });
    label.textContent = n.kind === "scene" ? n.label.split(" · ").pop()! : n.label;
    g.append(label);
    g.addEventListener("click", (e) => {
      if ((e as MouseEvent).detail >= 0 && !dragged) h.open(n);
    });
    g.addEventListener("keydown", (e) => {
      if ((e as KeyboardEvent).key === "Enter") h.open(n);
    });
    g.addEventListener("pointerenter", () => focus(n));
    g.addEventListener("pointerleave", () => focus(null));
    nodeLayer.append(g);
    return g;
  });

  function focus(n: SimNode | null) {
    const near = n ? neighbors.get(n.id) ?? new Set() : null;
    nodes.forEach((m, i) => nodeEls[i].classList.toggle("dim", !!n && m.id !== n.id && !near!.has(m.id)));
    links.forEach((l, i) => {
      const on = !n || l.source.id === n.id || l.target.id === n.id;
      lineEls[i].classList.toggle("dim", !on);
      relLabels[i]?.classList.toggle("show", !!n && on);
    });
    if (!n) {
      tip.hidden = true;
      return;
    }
    tip.replaceChildren();
    const head = document.createElement("div");
    head.className = "tip-head";
    head.append(swatch(n.kind), document.createTextNode(` ${n.label}`));
    const meta = document.createElement("div");
    meta.className = "tip-meta";
    meta.textContent = `${kindMeta(n.kind).label} · ${n.degree} connection${n.degree === 1 ? "" : "s"}`;
    tip.append(head, meta);
    if (n.summary) {
      const p = document.createElement("div");
      p.className = "tip-body";
      p.textContent = n.summary.length > 160 ? `${n.summary.slice(0, 160)}…` : n.summary;
      tip.append(p);
    }
    tip.hidden = false;
  }

  // ---- search highlights matches ----
  const applySearch = () => {
    const q = view.query.trim().toLowerCase();
    nodes.forEach((n, i) => nodeEls[i].classList.toggle("match", !!q && n.label.toLowerCase().includes(q)));
  };
  search.oninput = () => {
    view.query = search.value;
    applySearch();
  };
  applySearch();

  // ---- simulation ----
  const sim = forceSimulation<SimNode>(nodes)
    .force("link", forceLink<SimNode, SimLink>(links).distance((l) => (l.edge.kind === "mention" ? 90 : l.edge.kind === "next" ? 60 : 70)).strength((l) => (l.edge.kind === "mention" ? 0.15 : 0.5)))
    .force("charge", forceManyBody<SimNode>().strength((n) => (n.kind === "scene" ? -60 : -220)))
    .force("collide", forceCollide<SimNode>().radius((n) => radius(n) + 10))
    .force("center", forceCenter(0, 0));
  sim.on("tick", () => {
    links.forEach((l, i) => {
      const line = lineEls[i];
      line.setAttribute("x1", String(l.source.x));
      line.setAttribute("y1", String(l.source.y));
      line.setAttribute("x2", String(l.target.x));
      line.setAttribute("y2", String(l.target.y));
      const t = relLabels[i];
      if (t) {
        t.setAttribute("x", String(((l.source.x ?? 0) + (l.target.x ?? 0)) / 2));
        t.setAttribute("y", String(((l.source.y ?? 0) + (l.target.y ?? 0)) / 2 - 4));
      }
    });
    nodes.forEach((n, i) => nodeEls[i].setAttribute("transform", `translate(${n.x},${n.y})`));
  });
  stopSim = () => sim.stop();

  // ---- drag nodes, pan background, wheel zoom ----
  let dragged = false;
  let dragging: SimNode | null = null;
  let panning: { x: number; y: number } | null = null;
  const toWorld = (e: PointerEvent) => ({ x: (e.offsetX - view.tx) / view.scale, y: (e.offsetY - view.ty) / view.scale });
  svg.addEventListener("pointerdown", (e) => {
    const g = (e.target as Element).closest(".node");
    dragged = false;
    if (g) {
      dragging = nodes[nodeEls.indexOf(g as SVGGElement)];
      sim.alphaTarget(0.2).restart();
    } else {
      panning = { x: e.clientX - view.tx, y: e.clientY - view.ty };
    }
    svg.setPointerCapture(e.pointerId);
  });
  svg.addEventListener("pointermove", (e) => {
    if (dragging) {
      dragged = true;
      const p = toWorld(e);
      dragging.fx = p.x;
      dragging.fy = p.y;
    } else if (panning) {
      view.tx = e.clientX - panning.x;
      view.ty = e.clientY - panning.y;
      applyTransform();
    }
    if (!tip.hidden) {
      tip.style.left = `${e.offsetX + 14}px`;
      tip.style.top = `${e.offsetY + 14}px`;
    }
  });
  const release = () => {
    if (dragging) {
      dragging.fx = null;
      dragging.fy = null;
      sim.alphaTarget(0);
    }
    dragging = null;
    panning = null;
  };
  svg.addEventListener("pointerup", release);
  svg.addEventListener("pointercancel", release);
  svg.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      const k = Math.exp(-e.deltaY * 0.0015);
      const next = Math.min(4, Math.max(0.2, view.scale * k));
      view.tx = e.offsetX - ((e.offsetX - view.tx) * next) / view.scale;
      view.ty = e.offsetY - ((e.offsetY - view.ty) * next) / view.scale;
      view.scale = next;
      applyTransform();
    },
    { passive: false },
  );
}

export function stopGraph() {
  stopSim?.();
  stopSim = null;
}

// ---------- timeline ----------

export interface TimelineHandlers {
  open: (path: string) => void;
  openScene: (ms: string, block: string) => void;
  insert: (after: string | null, before: string | null) => void;
  /** A new first event for the timeline `before` starts; with neither, a new timeline. */
  start: (before: string | null, timeline: string | null) => void;
  move: (path: string, after: string | null, before: string | null) => void;
  remove: (path: string) => void;
}

/** What the timeline is read against, and how it's shown. */
export interface TimelineView {
  tellings: Telling[];
  /** "" (each scene where it is), a manuscript id, or `${id}\n${cut}`. */
  against: string;
  mode: "list" | "chart";
  setAgainst: (v: string) => void;
  setMode: (m: "list" | "chart") => void;
}

/** The event right after `e` on the same line, if it directly follows `e`. */
function nextInLane(events: TimelineEvent[], i: number): TimelineEvent | null {
  const e = events[i];
  return events.slice(i + 1).find((x) => x.after === e.path && x.lane === e.lane && x.timeline === e.timeline) ?? null;
}

const timelineName = (t: string) => t || "Main timeline";

function timelineControls(events: TimelineEvent[], v: TimelineView, h: TimelineHandlers): HTMLElement {
  const row = document.createElement("div");
  row.className = "timeline-controls";
  if (v.tellings.length) {
    const label = document.createElement("label");
    label.className = "against";
    label.append(document.createTextNode("Read against "));
    const sel = document.createElement("select");
    const opt = (value: string, text: string) => {
      const o = document.createElement("option");
      o.value = value;
      o.textContent = text;
      o.selected = value === v.against;
      return o;
    };
    sel.append(opt("", "each scene where it is"));
    for (const t of v.tellings) {
      const g = document.createElement("optgroup");
      g.label = t.title;
      g.append(opt(t.id, `${t.title}, as it is now`), ...t.cuts.map((c) => opt(`${t.id}\n${c}`, `cut: ${c}`)));
      sel.append(g);
    }
    sel.onchange = () => v.setAgainst(sel.value);
    label.append(sel);
    row.append(label);
  }
  const seg = document.createElement("div");
  seg.className = "seg";
  seg.setAttribute("role", "group");
  for (const [m, text] of [["list", "List"], ["chart", "Story vs telling"]] as const) {
    const b = document.createElement("button");
    b.type = "button";
    b.textContent = text;
    b.setAttribute("aria-pressed", String(v.mode === m));
    b.onclick = () => v.setMode(m);
    seg.append(b);
  }
  row.append(seg);
  const start = document.createElement("button");
  start.type = "button";
  start.textContent = "+ Event at the start";
  start.onclick = () => h.insert(null, events.find((e) => !e.after && e.lane === 0 && e.timeline === "")?.path ?? null);
  const fresh = document.createElement("button");
  fresh.type = "button";
  fresh.textContent = "+ New timeline";
  fresh.title = "A parallel world, another era, a story within the story";
  fresh.onclick = () => h.start(null, null);
  row.append(start, fresh);
  return row;
}

export function renderTimeline(root: HTMLElement, events: TimelineEvent[], h: TimelineHandlers, confirmButton: (label: string, fn: () => void) => HTMLElement, v: TimelineView) {
  root.replaceChildren();
  const head = document.createElement("div");
  head.className = "timeline-head";
  const title = document.createElement("h2");
  title.textContent = "Timeline";
  const hint = document.createElement("p");
  hint.className = "hint";
  hint.textContent = "Events in the order they happen. Drag one onto another to move it after that event. The § chips show where each is told, so flashbacks stand out; read against a cut to see one telling's order.";
  head.append(title, hint, timelineControls(events, v, h));
  root.append(head);

  if (!events.length) {
    const p = document.createElement("p");
    p.className = "hint";
    p.textContent = "No events yet. Add the first one, or use “+ Event here” while you're in a scene.";
    root.append(p);
    return;
  }
  if (v.mode === "chart") return renderTelling(root, events, h, v);

  const byPath = new Map(events.map((e) => [e.path, e]));
  const named = events.some((e) => e.timeline !== "");
  const list = document.createElement("ol");
  list.className = "timeline";
  events.forEach((e, i) => {
    if (named && (i === 0 || events[i - 1].timeline !== e.timeline)) {
      const sec = document.createElement("li");
      sec.className = "timeline-name";
      const n = document.createElement("h3");
      n.textContent = timelineName(e.timeline);
      const count = document.createElement("span");
      count.className = "hint";
      const size = events.filter((x) => x.timeline === e.timeline).length;
      count.textContent = `${size} event${size === 1 ? "" : "s"}`;
      const add = document.createElement("button");
      add.type = "button";
      add.textContent = "+ at the start";
      add.onclick = () => h.start(e.path, e.timeline);
      sec.append(n, count, add);
      list.append(sec);
    }
    const untold = v.against !== "" && e.told_at === null;
    const li = document.createElement("li");
    li.className = `event lane-${Math.min(e.lane, 4)}${e.status === "draft" ? " draft" : ""}${untold ? " untold" : ""}`;
    li.style.setProperty("--lane", String(e.lane));
    li.draggable = true;

    const card = document.createElement("div");
    card.className = "event-card";
    const top = document.createElement("div");
    top.className = "event-top";
    const name = document.createElement("button");
    name.type = "button";
    name.className = "event-name";
    name.append(swatch("event"), document.createTextNode(` ${e.name}`));
    name.onclick = () => h.open(e.path);
    top.append(name);
    if (e.when) {
      const when = document.createElement("span");
      when.className = "event-when";
      when.textContent = e.when;
      top.append(when);
    }
    const parent = e.after ? byPath.get(e.after) : undefined;
    if (parent && (parent.timeline !== e.timeline || (e.lane > 0 && parent.lane !== e.lane))) {
      const br = document.createElement("span");
      br.className = "badge";
      br.textContent = parent.timeline !== e.timeline ? `branches from ${parent.name} (${timelineName(parent.timeline)})` : `branches from ${parent.name}`;
      top.append(br);
    }
    if (e.status === "draft") {
      const d = document.createElement("span");
      d.className = "badge draft";
      d.textContent = "draft";
      top.append(d);
    }
    if (e.status === "cycle") {
      const d = document.createElement("span");
      d.className = "badge warn";
      d.textContent = "⚠ loops back on itself";
      top.append(d);
    }
    card.append(top);
    if (e.summary) {
      const p = document.createElement("p");
      p.className = "event-summary";
      p.textContent = e.summary;
      card.append(p);
    }
    const chips = document.createElement("div");
    chips.className = "event-chips";
    for (const [ms, block, label] of e.scenes) {
      const c = document.createElement("button");
      c.type = "button";
      c.className = "chip scene-chip";
      c.textContent = `§ ${label.split(" · ").pop()}`;
      c.title = `Open ${label}`;
      c.onclick = () => h.openScene(ms, block);
      chips.append(c);
    }
    if (e.told_earlier || e.told_later) {
      const f = document.createElement("span");
      f.className = `badge ${e.told_earlier ? "early" : "late"}`;
      f.title = e.told_earlier
        ? "Told out of order: before events that happen ahead of it."
        : "Told out of order: after events that happen later. A flashback.";
      f.textContent = e.told_earlier ? "told earlier" : "told later";
      chips.append(f);
    }
    if (untold) {
      const f = document.createElement("span");
      f.className = "badge";
      f.title = "The telling you're reading against doesn't tell this event.";
      f.textContent = "not in this telling";
      chips.append(f);
    }
    for (const p of e.people) {
      const c = document.createElement("span");
      c.className = "person";
      c.append(swatch("character", 10), document.createTextNode(` ${p}`));
      chips.append(c);
    }
    if (chips.childElementCount) card.append(chips);
    const actions = document.createElement("div");
    actions.className = "event-actions";
    actions.append(confirmButton("Remove", () => h.remove(e.path)));
    card.append(actions);
    li.append(card);

    // Insert point after this event (between it and the next on its line).
    const add = document.createElement("button");
    add.type = "button";
    add.className = "insert";
    add.title = "Add an event here";
    add.textContent = "+";
    add.onclick = () => h.insert(e.path, nextInLane(events, i)?.path ?? null);
    li.append(add);

    li.ondragstart = (ev) => {
      ev.dataTransfer!.setData("text/plain", e.path);
      li.classList.add("dragging");
    };
    li.ondragend = () => li.classList.remove("dragging");
    li.ondragover = (ev) => {
      ev.preventDefault();
      li.classList.add("drop");
    };
    li.ondragleave = () => li.classList.remove("drop");
    li.ondrop = (ev) => {
      ev.preventDefault();
      li.classList.remove("drop");
      const moving = ev.dataTransfer!.getData("text/plain");
      if (moving && moving !== e.path) h.move(moving, e.path, nextInLane(events, i)?.path ?? null);
    };
    list.append(li);
  });
  root.append(list);
}

/**
 * Story vs telling: events in the order they happen (left) joined to the
 * order they're told (right). Lines that run together are told in order;
 * the ones that cut across the rest are told out of order.
 */
/** 1st, 2nd, 3rd, 4th … 11th, 12th, 13th … 21st, 22nd. */
function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13;
  return `${n}${teen ? "th" : ["th", "st", "nd", "rd"][n % 10] ?? "th"}`;
}

function renderTelling(root: HTMLElement, events: TimelineEvent[], h: TimelineHandlers, v: TimelineView) {
  const told = events.filter((e) => e.told_at !== null);
  const untold = events.length - told.length;
  const note = document.createElement("p");
  note.className = "hint telling-note";
  const early = told.filter((e) => e.told_earlier).length;
  const late = told.filter((e) => e.told_later).length;
  const out = [early && `${early} told earlier`, late && `${late} told later (flashbacks)`].filter(Boolean).join(", ");
  note.textContent =
    `Left: when it happens. Right: when it's told. ${out ? `Out of order, in color: ${out}.` : "Everything is told in the order it happens."}` +
    (untold ? ` ${untold} event${untold === 1 ? " isn't" : "s aren't"} told here${v.against ? " in this telling" : " in any scene"}.` : "");
  root.append(note);
  if (!told.length) return;

  const rowH = 22;
  const top = 26;
  const width = Math.max(560, Math.min(root.clientWidth - 80, 1100));
  const labelW = Math.min(300, Math.floor(width * 0.36));
  const x1 = labelW + 10;
  const x2 = width - labelW - 10;
  const height = top + told.length * rowH + 8;
  const telling = told.map((e, i) => ({ e, i })).sort((a, b) => a.e.told_at! - b.e.told_at! || a.i - b.i);
  const right = new Map(telling.map((t, j) => [t.e.path, j]));

  const chart = svgEl("svg", { class: "telling", width, height, viewBox: `0 0 ${width} ${height}`, role: "img", "aria-label": "Story order against telling order" });
  const header = (x: number, anchor: string, text: string) => {
    const t = svgEl("text", { x, y: 14, "text-anchor": anchor, class: "telling-head" });
    t.textContent = text;
    chart.append(t);
  };
  header(x1, "end", "Story order");
  header(x2, "start", "Telling order");
  const y = (row: number) => top + row * rowH + rowH / 2;
  const fit = (text: string) => (text.length > 40 ? `${text.slice(0, 39)}…` : text);

  told.forEach((e, i) => {
    const j = right.get(e.path)!;
    const g = svgEl("g", { class: `telling-row${e.told_earlier ? " early" : ""}${e.told_later ? " late" : ""}${e.lane > 0 ? " branch" : ""}`, tabindex: 0 });
    const tip = svgEl("title", {});
    tip.textContent = `${e.name}${e.timeline ? ` (${e.timeline})` : ""}: happens ${ordinal(i + 1)}, told ${ordinal(j + 1)}`;
    const mid = (x1 + x2) / 2;
    g.append(
      tip,
      svgEl("path", { d: `M ${x1 + 6} ${y(i)} C ${mid} ${y(i)}, ${mid} ${y(j)}, ${x2 - 6} ${y(j)}`, class: "telling-line" }),
      svgEl("circle", { cx: x1 + 3, cy: y(i), r: 3, class: "telling-dot" }),
      svgEl("circle", { cx: x2 - 3, cy: y(j), r: 3, class: "telling-dot" }),
    );
    const left = svgEl("text", { x: x1 - 4, y: y(i) + 4, "text-anchor": "end", class: "telling-label" });
    left.textContent = fit(e.name);
    const rightLabel = svgEl("text", { x: x2 + 4, y: y(j) + 4, "text-anchor": "start", class: "telling-label" });
    rightLabel.textContent = fit(e.name);
    g.append(left, rightLabel);
    g.addEventListener("click", () => h.open(e.path));
    g.addEventListener("keydown", (ev) => {
      if ((ev as KeyboardEvent).key === "Enter") h.open(e.path);
    });
    chart.append(g);
  });
  const wrap = document.createElement("div");
  wrap.className = "telling-wrap";
  wrap.append(chart);
  root.append(wrap);
}
