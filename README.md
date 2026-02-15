# NorthMail

A modern email client for GNOME, built with GTK4/libadwaita and Rust.

## Features

- Native GNOME look and feel with libadwaita
- Gmail support with GNOME Online Accounts integration
- Adaptive layout for different screen sizes
- Fast full-text search with SQLite FTS5
- Offline message access
- Real-time email notifications via IMAP IDLE

## Building

### Dependencies

- Rust 1.75+
- GTK4 4.16+
- libadwaita 1.6+
- GNOME Online Accounts 3.50+
- libsecret 0.20+
- WebKitGTK 6.0
- SQLite 3.40+
- OpenSSL

#### Fedora

```bash
sudo dnf install -y gcc gcc-c++ gtk4-devel libadwaita-devel \
    gnome-online-accounts-devel libsecret-devel webkitgtk6.0-devel \
    sqlite-devel openssl-devel
```

#### Ubuntu/Debian

```bash
sudo apt install -y build-essential libgtk-4-dev libadwaita-1-dev \
    libgoa-1.0-dev libsecret-1-dev libwebkitgtk-6.0-dev \
    libsqlite3-dev libssl-dev
```

### From Source

```bash
# Clone the repository
git clone https://github.com/dpetrariu/NorthMail.git
cd NorthMail

# Compile GSettings schema (required before first run)
glib-compile-schemas data/

# Build with Cargo
cargo build --release

# Run
./target/release/northmail
```

### Flatpak

```bash
# Build and install
flatpak-builder --user --install build-aux/org.northmail.NorthMail.json

# Run
flatpak run org.northmail.NorthMail
```

## Gmail Setup

### Using GNOME Online Accounts (Recommended)

1. Open GNOME Settings
2. Go to Online Accounts
3. Click "Google" and sign in
4. Enable "Mail" access
5. Open NorthMail - your account will be detected automatically

### Using Standalone OAuth2

If you're not using GNOME, NorthMail can authenticate directly with Google:

1. Open NorthMail
2. Click "Add Account"
3. Follow the browser authentication flow

## Project Structure

```
northmail/
├── crates/
│   ├── northmail-auth/    # OAuth2/GOA authentication
│   ├── northmail-core/    # Business logic and storage
│   ├── northmail-gtk/     # GTK4/libadwaita UI
│   ├── northmail-imap/    # IMAP protocol
│   └── northmail-smtp/    # SMTP protocol
├── data/                  # Desktop files, icons, schemas
└── build-aux/             # Meson and Flatpak configs
```

## Contributing

This project is in its initial stage of development. Contributions cannot be considered at this moment.

## License

This project is licensed under the GPL-3.0-or-later license.
