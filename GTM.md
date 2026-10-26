# WRITEpad go-to-market checklist

Started 2026-10-26. Ordered by what blocks what.

## 1. Before anyone else installs it

- [ ] **Domain.** `writepad.app` belongs to someone else. Pick one and check it's free (getwritepad.com, writepad.studio, …).
  - [ ] Connect it in Netlify (writepad-app → Domain management).
  - [ ] Update the site's `baseURL` (website/hugo.toml), the app's help links (`SITE` in src-tauri/src/lib.rs) and src/devbridge.ts.
- [ ] **Updater live.**
  - [ ] Add GitHub secrets `TAURI_SIGNING_PRIVATE_KEY` (from `~/.tauri/writepad-updater.key`) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
  - [ ] Release **0.1.6**, the first version that can update itself. Everyone on 0.1.4 or older has to download it by hand once.
  - [ ] Release a tiny **0.1.7** and confirm your own Mac shows the update card.
  - [ ] Set `version` in website/hugo.toml to match each release.
- [ ] **Back up the signing key** (`~/.tauri/writepad-updater.key` and its password) somewhere safe. If it's lost, installed copies can never update again.
- [ ] **Demo vault on the release.** The release workflow now zips and attaches `WRITEpad-Demo-Vault.zip`; check it's there after 0.1.6 goes out.
- [ ] **Linux.** Releases now build `.AppImage`, `.deb` and `.rpm`. Check the first PR's Linux build passes, then try the package on a Linux machine.
- [ ] **Test pass before launch:** install on a fresh Windows machine, and test the 0.1.6 → 0.1.7 update on your Mac.

## 2. Cut the install friction

- [ ] **Mac signing and notarization.** Apple Developer, $99/yr. Without it every Mac user goes through System Settings → "Open Anyway".
- [ ] **Windows signing** (e.g. Azure Trusted Signing). Without it SmartScreen warns on every install.

## 3. Getting paid: the $5 Publish unlock

Today the app has **no unlock**: EPUB and MP3 export are free in every build, but the pricing page sells them for $5.

- [ ] **Decide how the unlock works** (circle one):
  - **A. Lemon Squeezy license key.** Paste the key once; the app checks it with Lemon Squeezy one time, then works offline forever. Least to build.
  - **B. Own signed key.** A Netlify function signs the buyer's email on purchase; the app checks the signature offline. Most private, more to build.
  - **C. Honour system.** No lock, just a "Support WRITEpad: $5" button.
- [ ] Set up checkout (Lemon Squeezy or Gumroad) and put its link in `buyURL` (website/hugo.toml). The buttons say "Coming soon" until then.
- [ ] Build the unlock in the app: "Unlock Publish" in Export, enter a key in Settings, and a way to restore it on a new computer.
- [ ] Say plainly on the pricing page that it's open source, and what the $5 supports.

## 4. Site and trust

- [ ] Email notification for the `contact` form (Netlify → Forms → Form notifications).
- [ ] Google Search Console: verify the domain, submit `sitemap.xml`.
- [ ] Privacy-friendly analytics (Plausible or Netlify Analytics).
- [ ] Privacy policy: cover the optional "Polish with Claude" (dictation sent to Anthropic) and the license check, if there is one.
- [ ] An email address on the new domain.

## 5. Launch

- [ ] A 30–60 second demo video or GIF: onion skin, a cut, Listen.
- [ ] Posts: Show HN, Product Hunt, Reddit (r/writing, r/scrivener, r/screenwriting), Indie Hackers.
- [ ] Blog, about one post a week. Accurate "Scrivener alternative"–style pages bring in high-intent searches.
- [ ] The **Gospels vault** as a showcase: four tellings of one story shows off cuts and timelines.
- [ ] An email signup on the site, for release news.

## Done so far

- [x] Feature list ready for launch: spell check, find & replace, comments, focus mode, words today, selection word count
- [x] First-run welcome: Try the demo (bundled in the app), Start a new book, Open a folder
- [x] Narration setup guides people without Python to python.org and retries
- [x] Help links in the app: ? buttons and a Help menu
- [x] Delete page, with Undo
- [x] PR builds for Windows and Linux; a fix for the Windows PR build
- [x] Website with help docs (18 pages) and a blog (4 posts)
- [x] Netlify deploys the site from GitHub `main` (base directory `website`)
- [x] Contact form flood protection: rate limit, daily caps, honeypot, signed relay, `NETLIFY_API_TOKEN` cleanup
- [x] Auto-update built into the app (ships with 0.1.6)
