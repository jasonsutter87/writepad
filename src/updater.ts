// Signed self-updates. The app checks GitHub Releases' latest.json a few
// seconds after launch and every six hours; Tauri verifies each download
// against the public key in tauri.conf.json before installing it.

import { getVersion } from "@tauri-apps/api/app";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T;

const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const FIRST_CHECK_DELAY_MS = 8_000;
const SNOOZE_KEY = "writepad.update.snoozed";

let found: Update | null = null;
let busy = false;
/** Runs before an update installs: save open edits, snapshot the vault. */
let prepare: (version: string) => Promise<void> = async () => {};

function snoozed(version: string): boolean {
  try {
    return localStorage.getItem(SNOOZE_KEY) === version;
  } catch {
    return false;
  }
}

function snooze(version: string) {
  try {
    localStorage.setItem(SNOOZE_KEY, version);
  } catch {
    /* storage unavailable: the card just comes back next launch */
  }
}

/** Fill and show the corner card. `percent` undefined hides the progress bar. */
function showCard(title: string, body: string, opts: { actions?: boolean; closable?: boolean; percent?: number | null } = {}) {
  $("uc-title").textContent = title;
  $("uc-body").textContent = body;
  $("uc-actions").hidden = !opts.actions;
  $("uc-notes").hidden = !found?.body;
  $("uc-close").hidden = opts.closable === false;
  const bar = $("uc-progress");
  bar.hidden = opts.percent === undefined;
  (bar.firstElementChild as HTMLElement).style.width = `${opts.percent ?? 5}%`;
  $("update-card").hidden = false;
}

const hideCard = () => ($("update-card").hidden = true);

/**
 * Look for a newer release. Quiet checks (manual = false) swallow failures,
 * such as being offline, and skip a version the user said "not now" to.
 * Returns a line for the Settings dialog.
 */
export async function checkForUpdates(manual: boolean): Promise<string> {
  if (busy) return "Updating…";
  try {
    found = await check();
  } catch (e) {
    return manual ? `Couldn't check for updates: ${String(e)}` : "";
  }
  if (!found) return "You're on the latest version.";
  if (manual || !snoozed(found.version)) {
    showCard(`WRITEpad ${found.version} is ready`, "We'll save your work, update, and restart. Takes about a minute.", { actions: true });
  }
  return `Version ${found.version} is available.`;
}

async function install() {
  const update = found;
  if (!update || busy) return;
  busy = true;
  try {
    showCard(`Saving your work…`, "", { closable: false });
    await prepare(update.version);

    let total = 0;
    let received = 0;
    showCard(`Downloading ${update.version}…`, "Starting…", { closable: false, percent: null });
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") total = event.data.contentLength ?? 0;
      if (event.event === "Progress") {
        received += event.data.chunkLength;
        const percent = total ? Math.min(100, Math.round((received / total) * 100)) : null;
        showCard(`Downloading ${update.version}…`, percent != null ? `${percent}%` : "", { closable: false, percent });
      }
      if (event.event === "Finished") showCard("Installing", "WRITEpad will restart in a moment.", { closable: false, percent: 100 });
    });
    await relaunch();
  } catch (e) {
    busy = false;
    showCard("Update didn't finish", `${String(e)}. Your vault wasn't changed.`);
  }
}

/** Wire up the card and the Settings row, and start checking in the background. */
export function initUpdater(beforeInstall: (version: string) => Promise<void>) {
  prepare = beforeInstall;

  $("uc-install").onclick = () => void install();
  $("uc-close").onclick = () => {
    if (found && !busy) snooze(found.version);
    hideCard();
  };
  $("uc-notes").onclick = () => {
    if (!found) return;
    $("whats-new-title").textContent = `What's new in ${found.version}`;
    $("whats-new-body").textContent = found.body ?? "";
    $<HTMLDialogElement>("dlg-whats-new").showModal();
  };

  getVersion().then((v) => ($("app-version").textContent = `Version ${v}`), () => {});
  const status = $("update-status");
  const button = $<HTMLButtonElement>("btn-check-updates");
  button.onclick = async () => {
    button.disabled = true;
    status.textContent = "Checking…";
    status.textContent = await checkForUpdates(true);
    button.disabled = false;
  };

  window.setTimeout(() => void checkForUpdates(false), FIRST_CHECK_DELAY_MS);
  window.setInterval(() => void checkForUpdates(false), CHECK_EVERY_MS);
}
