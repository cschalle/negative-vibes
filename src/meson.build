cargo_options = [
  '--manifest-path', meson.project_source_root() / 'Cargo.toml',
  '--target-dir', meson.project_build_root() / 'cargo-target',
]

if get_option('profile') == 'default'
  cargo_options += ['--release']
  rust_target = 'release'
else
  rust_target = 'debug'
endif

cargo_env = environment()
cargo_env.set('APP_ID', application_id)
cargo_env.set('PKGDATADIR', pkgdatadir)
cargo_env.set('CARGO_HOME', meson.project_build_root() / 'cargo-home')

cargo_build = custom_target(
  'cargo-build',
  build_by_default: true,
  build_always_stale: true,
  output: 'negative-vibes',
  console: true,
  install: true,
  install_dir: bindir,
  env: cargo_env,
  command: [
    find_program('bash'),
    meson.project_source_root() / 'build-aux/meson/cargo-build.sh',
    '@OUTPUT@',
    meson.project_build_root() / 'cargo-target' / rust_target / 'negative-vibes',
    cargo_bin,
    cargo_options,
  ],
)
