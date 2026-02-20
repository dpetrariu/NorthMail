project('northmail',
  version: '0.1.0',
  meson_version: '>= 0.59.0',
  default_options: [
    'warning_level=2',
  ],
)

i18n = import('i18n')
gnome = import('gnome')

application_id = 'org.northmail.NorthMail'
version = meson.project_version()
prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')

# Dependencies (for documentation, actual building done by Cargo)
dependency('gtk4', version: '>= 4.14')
dependency('libadwaita-1', version: '>= 1.4')

# Translations
subdir('po')

# Install desktop file (with translations merged in)
i18n.merge_file(
  input: 'data/org.northmail.NorthMail.desktop.in',
  output: 'org.northmail.NorthMail.desktop',
  type: 'desktop',
  po_dir: 'po',
  install: true,
  install_dir: datadir / 'applications',
)

# Install metainfo
install_data(
  'data/org.northmail.NorthMail.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

# Install GSchema
install_data(
  'data/org.northmail.NorthMail.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

# Install icons
icon_sizes = ['16x16', '32x32', '48x48', '64x64', '128x128', '256x256', '512x512']
foreach size : icon_sizes
  install_data(
    'data' / 'icons' / 'hicolor' / size / 'apps' / 'org.northmail.NorthMail.png',
    install_dir: datadir / 'icons' / 'hicolor' / size / 'apps',
  )
endforeach

# Install scalable SVG icon
install_data(
  'data' / 'icons' / 'hicolor' / 'scalable' / 'apps' / 'org.northmail.NorthMail.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'scalable' / 'apps',
)

# Install symbolic icons
install_data(
  'data' / 'icons' / 'hicolor' / 'symbolic' / 'actions' / 'dock-left-symbolic.svg',
  install_dir: datadir / 'icons' / 'hicolor' / 'symbolic' / 'actions',
)

# Compile schemas for development
gnome.compile_schemas(
  depend_files: files('data/org.northmail.NorthMail.gschema.xml'),
)

# Build Rust project using Cargo
cargo = find_program('cargo')

cargo_options = ['--manifest-path', meson.project_source_root() / 'Cargo.toml']

if get_option('buildtype') == 'release'
  cargo_options += ['--release']
  cargo_target = 'release'
else
  cargo_target = 'debug'
endif

cargo_env = environment()
cargo_env.set('CARGO_HOME', meson.project_build_root() / 'cargo-home')

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'northmail',
  console: true,
  install: true,
  install_dir: bindir,
  env: cargo_env,
  command: [
    cargo, 'build',
    cargo_options,
    '&&',
    'cp',
    meson.project_source_root() / 'target' / cargo_target / 'northmail',
    '@OUTPUT@',
  ],
)

# Post-install script for desktop database and icon cache
gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
