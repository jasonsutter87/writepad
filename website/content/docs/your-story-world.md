---
title: "Your story world"
description: "Keep characters, places, things and relationships as notes, see who appears in each scene, and map how it all connects."
group: "Your story world"
weight: 10
---

The **World** tab keeps your story bible: characters, locations, relationships, things, and an overview of the world itself. Each entry is an ordinary markdown note in your vault (under `world/`), so you can write as much or as little about it as you like.

{{< shot "world" "The World tab listing characters and locations with a line about each" >}}

## Add an entry

Each group on the **World** tab (Characters, Locations and so on) has a **+** button. Pick the **Type**, give it a **Name** and click **Create**. The new note opens, ready for a description.

The note starts with a few fields at the top, between `---` lines:

- **aliases**: other names the entry goes by, like `["the old man", "his father"]`. WRITEpad counts these as mentions too.
- **relationships**: links to other entries, like `{ to: "[[Mrs. White]]", kind: "married to" }`.

The first line of the description is what the World tab shows under the name.

## Let WRITEpad suggest entries

Already have a draft? Click **Suggest…** on the World tab and pick a **Manuscript**. WRITEpad reads it with plain rules (no AI) and lists the characters and places it finds. Tick the ones you want and click **Add as drafts**. They arrive marked as drafts, for you to keep, edit or delete.

Tick **Also start a timeline: one event per scene, in order** to get a first [timeline](/docs/timeline/) too.

Have a spreadsheet of characters already? **Import CSV…** brings in characters, places and things from a `.csv` file.

## Who's in this scene

While you're on a page, **In this scene** (on the right) lists every character, place and thing the page mentions, with how many times, like *Mr. White ×7*. Click one to open its note. Below that are the [timeline events](/docs/timeline/) linked to the page, and **+ Event here** to add one.

## Connections

Open an entry's note and **Connections** (on the right) shows its relationships, every scene it appears in (with how often), and every note that links to it.

## The graph

Click **Graph** on the World tab to see everything at once: characters, locations, things, scenes and events, with lines for relationships and appearances. The chips at the top show or hide each kind. Click any dot to open it.

{{< shot "graph" "The world graph of The Monkey's Paw" >}}
