project(
  'negative-vibes',
  'rust',
  version: '0.1.0',
  meson_version: '>= 1.4',
  license: 'GPL-3.0-or-later',
)

gnome = import('gnome')
i18n = import('i18n')

if get_option('profile') == 'development'
  app_id = 'io.github.cschalle.NegativeVibes.Devel'
  version_suffix = '+devel'
else
  app_id = 'io.github.cschalle.NegativeVibes'
  version_suffix = ''
endif

application_id = app_id
version_with_suffix = meson.project_version() + version_suffix

pkgdatadir = get_option('prefix') / get_option('datadir') / meson.project_name()
bindir = get_option('prefix') / get_option('bindir')

cargo_bin = find_program('cargo', required: true)

# System library minimums
dependency('gtk4', version: '>= 4.14')
dependency('libadwaita-1', version: '>= 1.4')

subdir('data')
subdir('help')
subdir('po')
subdir('src')

gnome.post_install(
  glib_compile_schemas: true,
  gtk_update_icon_cache: true,
  update_desktop_database: true,
)
