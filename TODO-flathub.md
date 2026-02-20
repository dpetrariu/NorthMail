# NorthMail — Flathub Readiness TODO

## Flathub Blockers (must fix)

1. [x] **No scalable SVG app icon** — ~~Flathub requires `hicolor/scalable/apps/org.northmail.NorthMail.svg`. Only PNGs exist.~~ Done: SVG icon added.
2. [x] **No LICENSE file** — ~~Cargo declares GPL-3.0-or-later but no actual license text file exists in the repo.~~ Done: GPL-3.0 LICENSE file exists.
3. [ ] **No real screenshots** — Metainfo references a broken GitHub URL (`northmail/northmail` instead of `dpetrariu/NorthMail`) and no `data/screenshots/` directory exists.

## High Priority

4. [x] **Rich-text compose is cosmetic only** — ~~The formatting toolbar (bold, italic, etc.) applies `GtkTextBuffer` tags but at send time only plain text is extracted. HTML is never sent.~~ Done: Rich-text compose now works.
5. [ ] **No email threading/conversation view** — Messages shown individually in date order, no grouping by thread.
6. [ ] **No email signatures** — No per-account signature config, nothing appended to outgoing messages.
7. [ ] **No manual IMAP/SMTP account setup** — Only GOA accounts work. The standalone OAuth2 flow shows "Not Implemented". Users with Fastmail, ProtonMail Bridge, etc. are locked out.
8. [x] **No translation infrastructure** — ~~`gettext-domain` is declared but no `.po`/`.pot` files, no `i18n` in meson, no gettext calls in Rust code. English-only.~~ Done: gettext-rs wired up, ~240 strings wrapped, German + French translations.
9. [ ] **No keyboard shortcuts window** — Only a handful of shortcuts exist (`Ctrl+Q`, `Ctrl+N`, `Ctrl+R`, `Ctrl+F`). No `Ctrl+Enter` to send, no `Delete` key, no shortcuts window (`GtkShortcutsWindow`).

## Medium Priority

10. [x] **No "Empty Trash" action** — ~~Messages can be moved to trash but can't be permanently deleted.~~ Done: Empty Trash implemented.
11. [x] **No create/rename/delete folders** — ~~Folder list is read-only.~~ Done: Folder management (create/rename/delete) added.
12. [ ] **Notification sound toggle does nothing** — Settings UI has the toggle but no code plays audio.
13. [ ] **No undo for destructive actions** — Delete/archive/spam show toasts but no "Undo" button (GNOME HIG pattern).
14. [ ] **No print support** — No `PrintOperation` or equivalent.
15. [ ] **Search is local-only** — No server-side IMAP SEARCH. Messages not yet synced are invisible to search.
16. [ ] **Flatpak runtime pinned to GNOME 47** — Current stable is 48.

## Low Priority / Bugs

17. [ ] **About dialog uses `"email"` icon** instead of `"org.northmail.NorthMail"`.
18. [ ] **`eprintln!` debug statements** left in draft auto-save code instead of using `tracing`.
19. [x] **Metainfo release date** — ~~is a placeholder (`2024-01-01`).~~ Done: Updated to 2026-02-18.
20. [ ] **Desktop file writing in Flatpak** — The icon setting writes to `~/.local/share/applications/` which may not be accessible inside a Flatpak sandbox without extra permissions.
