app_conf = configuration_data()
app_conf.set('APP_ID', application_id)

desktop_file = configure_file(
  input: 'io.github.cschalle.NegativeVibes.desktop.in',
  output: '@0@.desktop'.format(application_id),
  configuration: app_conf,
  install: true,
  install_dir: get_option('datadir') / 'applications',
)

desktop_file_validate = find_program('desktop-file-validate', required: false)
if desktop_file_validate.found()
  test('validate-desktop-file', desktop_file_validate,
    args: [desktop_file.full_path()],
  )
endif

metainfo_file = configure_file(
  input: 'io.github.cschalle.NegativeVibes.metainfo.xml.in',
  output: '@0@.metainfo.xml'.format(application_id),
  configuration: app_conf,
  install: true,
  install_dir: get_option('datadir') / 'metainfo',
)

appstream_validate = find_program('appstreamcli', required: false)
if appstream_validate.found()
  test('validate-metainfo', appstream_validate,
    args: ['validate', '--no-net', metainfo_file.full_path()],
  )
endif

install_data(
  'io.github.cschalle.NegativeVibes.gschema.xml',
  install_dir: get_option('datadir') / 'glib-2.0/schemas',
  rename: '@0@.gschema.xml'.format(application_id),
)

# Device quirks file
install_data(
  'devices.toml',
  install_dir: pkgdatadir,
)

# The application icon, at the sizes the icon theme looks for
foreach size : ['16', '32', '48', '64', '128', '256', '512']
  install_data(
    'icons/hicolor/@0@x@0@/apps/io.github.cschalle.NegativeVibes.png'.format(size),
    install_dir: get_option('datadir') / 'icons/hicolor/@0@x@0@/apps'.format(size),
    rename: '@0@.png'.format(application_id),
  )
endforeach
install_data(
  'icons/hicolor/scalable/apps/io.github.cschalle.NegativeVibes.svg',
  install_dir: get_option('datadir') / 'icons/hicolor/scalable/apps',
  rename: '@0@.svg'.format(application_id),
)
