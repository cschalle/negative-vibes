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
