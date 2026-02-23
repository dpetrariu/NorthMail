# NorthMail

A modern email client for GNOME, built with GTK4/libadwaita and Rust.

**Use at your own risk.** This project is in early development and may contain bugs.

## Features

- Native GNOME look and feel with libadwaita
- Gmail support with GNOME Online Accounts integration
- Fast full-text search with SQLite FTS5
- Starred messages with per-account virtual folders
- Keyboard navigation for browsing messages
- Offline message access
- Real-time email notifications via IMAP IDLE

## Building

### Dependencies

- Rust (latest stable recommended)
- GTK4 4.12+
- libadwaita 1.5+
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

# Run (set GSETTINGS_SCHEMA_DIR so the app finds its schema)
GSETTINGS_SCHEMA_DIR=data ./target/release/northmail
```

To get the app icon in your taskbar/dock, install the desktop file and icons:

```bash
cp data/com.petrariu.NorthMail.desktop ~/.local/share/applications/
cp -r data/icons/hicolor/* ~/.local/share/icons/hicolor/
gtk4-update-icon-cache ~/.local/share/icons/hicolor/
```

### Install from Release

Download the latest package for your platform from [Releases](https://github.com/dpetrariu/NorthMail/releases).

| Package | Architecture | Platform |
|---------|-------------|----------|
| `NorthMail.flatpak` | x86_64 | Any Linux (Flatpak) |
| `NorthMail-aarch64.flatpak` | aarch64 | Any Linux (Flatpak) |
| `northmail_*_amd64.deb` | x86_64 | Debian / Ubuntu |
| `northmail_*_arm64.deb` | aarch64 | Debian / Ubuntu |
| `northmail-*.x86_64.rpm` | x86_64 | Fedora / RHEL |
| `northmail-*.aarch64.rpm` | aarch64 | Fedora / RHEL |

**Flatpak:**
```bash
flatpak install NorthMail.flatpak
flatpak run com.petrariu.NorthMail
```

**Debian/Ubuntu:**
```bash
sudo dpkg -i northmail_*_amd64.deb
sudo apt-get install -f   # install dependencies
```

**Fedora/RHEL:**
```bash
sudo dnf install northmail-*.x86_64.rpm
```

### Flatpak (build from source)

```bash
flatpak-builder --user --install build-aux/com.petrariu.NorthMail.json
flatpak run com.petrariu.NorthMail
```

## Gmail Setup

NorthMail uses GNOME Online Accounts for authentication:

1. Open GNOME Settings
2. Go to Online Accounts
3. Click "Google" and sign in
4. Enable "Mail" access
5. Open NorthMail - your account will be detected automatically

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
