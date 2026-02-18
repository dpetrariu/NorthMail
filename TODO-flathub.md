# NorthMail — Flathub Readiness TODO

## Flathub Blockers (must fix)

1. [ ] **No scalable SVG app icon** — Flathub requires `hicolor/scalable/apps/org.northmail.NorthMail.svg`. Only PNGs exist.
2. [ ] **No LICENSE file** — Cargo declares GPL-3.0-or-later but no actual license text file exists in the repo.
3. [ ] **No real screenshots** — Metainfo references a broken GitHub URL (`northmail/northmail` instead of `dpetrariu/NorthMail`) and no `data/screenshots/` directory exists.

## High Priority

4. [ ] **Rich-text compose is cosmetic only** — The formatting toolbar (bold, italic, etc.) applies `GtkTextBuffer` tags but at send time only plain text is extracted. HTML is never sent.
5. [ ] **No email threading/conversation view** — Messages shown individually in date order, no grouping by thread.
6. [ ] **No email signatures** — No per-account signature config, nothing appended to outgoing messages.
7. [ ] **No manual IMAP/SMTP account setup** — Only GOA accounts work. The standalone OAuth2 flow shows "Not Implemented". Users with Fastmail, ProtonMail Bridge, etc. are locked out.
8. [ ] **No translation infrastructure** — `gettext-domain` is declared but no `.po`/`.pot` files, no `i18n` in meson, no gettext calls in Rust code. English-only.
9. [ ] **No keyboard shortcuts window** — Only a handful of shortcuts exist (`Ctrl+Q`, `Ctrl+N`, `Ctrl+R`, `Ctrl+F`). No `Ctrl+Enter` to send, no `Delete` key, no shortcuts window (`GtkShortcutsWindow`).

## Medium Priority

10. [ ] **No "Empty Trash" action** — Messages can be moved to trash but can't be permanently deleted.
11. [ ] **No create/rename/delete folders** — Folder list is read-only.
12. [ ] **Notification sound toggle does nothing** — Settings UI has the toggle but no code plays audio.
13. [ ] **No undo for destructive actions** — Delete/archive/spam show toasts but no "Undo" button (GNOME HIG pattern).
14. [ ] **No print support** — No `PrintOperation` or equivalent.
15. [ ] **Search is local-only** — No server-side IMAP SEARCH. Messages not yet synced are invisible to search.
16. [ ] **Flatpak runtime pinned to GNOME 47** — Current stable is 48.

## Low Priority / Bugs

17. [ ] **About dialog uses `"email"` icon** instead of `"org.northmail.NorthMail"`.
18. [ ] **`eprintln!` debug statements** left in draft auto-save code instead of using `tracing`.
19. [ ] **Metainfo release date** is a placeholder (`2024-01-01`).
20. [ ] **Desktop file writing in Flatpak** — The icon setting writes to `~/.local/share/applications/` which may not be accessible inside a Flatpak sandbox without extra permissions.
