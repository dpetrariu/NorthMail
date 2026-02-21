project('northmail',
  version: '0.1.0',
  meson_version: '>= 0.59.0',
  default_options: [
    'warning_level=2',
  ],
)

i18n = import('i18n')
gnome = import('gnome')

application_id = 'com.petrariu.NorthMail'
version = meson.project_version()
prefix = get_option('prefix')
bindir = prefix / get_option('bindir')
datadir = prefix / get_option('datadir')
localedir = prefix / get_option('localedir')

# Dependencies
dependency('gtk4', version: '>= 4.16')
dependency('libadwaita-1', version: '>= 1.6')
dependency('goa-1.0', version: '>= 3.50')
dependency('libsecret-1', version: '>= 0.20')
dependency('webkit2gtk-6.0', version: '>= 2.44')
dependency('sqlite3', version: '>= 3.40')

# Install desktop file
install_data(
  'data' / 'com.petrariu.NorthMail.desktop',
  install_dir: datadir / 'applications',
)

# Install metainfo
install_data(
  'data' / 'com.petrariu.NorthMail.metainfo.xml',
  install_dir: datadir / 'metainfo',
)

# Install GSchema
install_data(
  'data' / 'com.petrariu.NorthMail.gschema.xml',
  install_dir: datadir / 'glib-2.0' / 'schemas',
)

# Compile schemas
gnome.compile_schemas(
  depend_files: files('data' / 'com.petrariu.NorthMail.gschema.xml'),
)

# Build Rust project
cargo = find_program('cargo')

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'northmail',
  console: true,
  install: true,
  install_dir: bindir,
  command: [
    cargo, 'build',
    '--manifest-path', meson.project_source_root() / 'Cargo.toml',
    '--release',
    '&&',
    'cp',
    meson.project_source_root() / 'target' / 'release' / 'northmail',
    '@OUTPUT@',
  ],
)

# Post-install script
gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
