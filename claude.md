# NorthMail

A native GNOME email client built with Rust, GTK4, and libadwaita. The goal is to match the functionality and usability of Apple Mail on macOS - a clean, fast, reliable email client for receiving, reading, organizing, and sending emails.

## Vision

NorthMail aims to be the email client that GNOME has been missing - one that feels native, integrates seamlessly with the desktop, and just works. No bloat, no unnecessary features, just email done right.

## Core Principles

- **Native GNOME experience**: Follow GNOME HIG strictly. Use libadwaita patterns. Integrate with GNOME Online Accounts.
- **Performance first**: Handle 60,000+ message folders without breaking a sweat. Progressive loading, efficient sync.
- **Reliability**: Email is critical. Never lose data. Handle network issues gracefully.
- **Simplicity**: Do one thing well. No smart folders, no AI summarization (yet), no calendar integration.

## What We're Building

### Must Have
- Gmail and Microsoft/Outlook account support via GNOME Online Accounts
- IMAP sync with XOAUTH2 authentication
- Folder navigation with unread counts
- Message list with virtual scrolling for large folders
- Message viewing with HTML rendering (sandboxed WebKitGTK)
- Compose and send emails via SMTP
- Search with SQLite FTS5
- Offline reading (maildir-style body storage)
- IMAP IDLE for real-time updates
- Keyboard navigation

### Won't Have (for now)
- Smart folders / saved searches
- AI summarization or smart features
- Calendar/contacts integration
- Multiple identity management
- PGP/GPG encryption
- Custom IMAP/SMTP server configuration (only OAuth providers)

## Tech Stack

| Component | Technology |
|-----------|------------|
| Language | Rust |
| UI Framework | GTK4 + libadwaita |
| Async Runtime | async-std (for IMAP), glib main loop (for UI) |
| IMAP | Custom simple_client with async-native-tls |
| SMTP | lettre |
| Auth | GNOME Online Accounts (goa crate) + libsecret fallback |
| Database | SQLite with sqlx |
| HTML Rendering | WebKitGTK 6 (sandboxed) |
| Build | Cargo workspace + Meson for GNOME integration |

## Project Structure

```
northmail/
├── crates/
│   ├── northmail-core/     # Business logic, sync engine, storage
│   ├── northmail-imap/     # IMAP protocol implementation
│   ├── northmail-smtp/     # SMTP via lettre
│   ├── northmail-auth/     # OAuth2 + GNOME Online Accounts
│   └── northmail-gtk/      # GTK4/libadwaita UI
├── data/                   # Icons, .desktop, GSchema, metainfo
└── build-aux/              # Meson + Flatpak configs
```

## Current State

Working:
- GOA integration for Gmail and Microsoft accounts
- XOAUTH2 IMAP authentication
- Folder listing and selection
- Progressive message header loading (handles 60k+ messages)
- MIME header decoding (RFC 2047 with charset support)
- Sync progress indicator
- Message selection signal

In Progress:
- Message body fetching and display
- Message content rendering

Not Started:
- Compose window
- SMTP sending
- Search
- IMAP IDLE
- Offline storage
- Flatpak packaging

## Development Notes

### Running
```bash
cargo run -p northmail-gtk
```

### HTML Email Rendering
For proper HTML email rendering, install WebKitGTK 6 and build with the `webkit` feature:
```bash
# Fedora
sudo dnf install webkitgtk6.0-devel

# Build with WebKit support
cargo run -p northmail-gtk --features webkit
```

Without WebKit, HTML emails are displayed as stripped plain text.

### Authentication
The app uses GNOME Online Accounts for authentication. Add your Gmail or Microsoft account in GNOME Settings > Online Accounts before running the app.

### IMAP Connection
We use a custom SimpleImapClient instead of async-imap because:
1. Better control over XOAUTH2 flow
2. Works reliably with async-std
3. Simpler error handling for our use case

### Cross-thread Communication
GTK runs on the main thread. IMAP sync runs on a background thread. We use:
- `std::sync::mpsc` for thread communication
- `glib::timeout_add_local` to poll for messages on the main thread
- GObject signals for widget-to-widget communication

### Large Folder Handling
Folders can have 60,000+ messages. We:
1. Load in batches of 50 messages
2. Show progress (e.g., "Loading 150 of 62,456 messages")
3. Update UI incrementally as batches arrive
4. Use virtual scrolling in the message list

## Code Conventions

- Use `tracing` for logging (debug!, info!, warn!, error!)
- GObject subclassing for all GTK widgets
- Async functions for all I/O operations
- Explicit error handling with custom error types
- No unwrap() in production code paths

## Testing

For now, manual testing with real Gmail/Microsoft accounts. The app should:
1. Show all GOA-configured mail accounts
2. List folders with correct unread counts
3. Load messages progressively without freezing
4. Display message headers correctly (including non-ASCII)
5. Handle network disconnection gracefully
