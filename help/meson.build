# The manual, in Mallard, for Yelp (F1 and `help:negative-vibes`). Translated copies
# will be added with itstool once there are translations; until then only C is installed.
help_pages = [
  'colour', 'detail', 'dust', 'export', 'film-types', 'getting-started', 'index',
  'metadata', 'orientation', 'preferences', 'preview-and-frames', 'profiles',
  'scanning', 'shortcuts', 'tone', 'troubleshooting',
]
foreach page : help_pages
  install_data(
    'C/@0@.page'.format(page),
    install_dir: get_option('datadir') / 'help/C' / meson.project_name(),
  )
endforeach
