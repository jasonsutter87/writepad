---
title: "History"
description: "Keep snapshots of your whole vault, compare any page with how it used to read, and roll back."
group: "Write and revise"
weight: 70
---

**History** keeps snapshots of the whole vault: every note and every version of every page, as they were. You can compare a page with any snapshot, restore one page, or roll the whole vault back.

It uses git under the hood, so the history also works with any git tool. You don't need to know git to use it.

## Turn it on

In the **History** panel on the right, click **Turn on history**. It's off until you do, once per vault.

{{< shot "history" "The History panel with Turn on history" >}}

## Save a snapshot

Type a short description (*Describe this version*) and click **Save**. With **Autosave every 10 minutes** ticked, WRITEpad also saves one on its own while you write.

Before an app update installs, WRITEpad saves a snapshot too.

## Look back

The panel lists the snapshots of the note, or the version of a page, you have open, newest first. Click one to compare it with what you have now, as an [onion skin](/docs/onion-skin/). Click it again to stop comparing.

Each snapshot has two buttons:

- **Restore as new take** (on a page) brings the old text back as a new version, next to the current one, so nothing is overwritten. On a note, **Restore** puts the old text back.
- **Roll back whole vault** puts everything back as it was at that snapshot. Click twice to confirm.
