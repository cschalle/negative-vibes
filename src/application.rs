mod imp {
    use std::cell::OnceCell;

    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use nv_acquire::AcquireWorker;

    #[derive(Default)]
    pub struct Application {
        pub worker: OnceCell<AcquireWorker>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Application {
        const NAME: &'static str = "NvApplication";
        type Type = super::Application;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for Application {}

    impl ApplicationImpl for Application {
        fn startup(&self) {
            self.parent_startup();
            self.start_sane_worker();
            self.setup_actions();
        }

        fn activate(&self) {
            self.parent_activate();
            let app = self.obj();
            if let Some(win) = app.active_window() {
                win.present();
                return;
            }
            let window = crate::window::Window::new(app.upcast_ref::<gtk::Application>());
            app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
            app.set_accels_for_action("win.redo", &["<Ctrl>y", "<Ctrl><Shift>z"]);
            window.present();

            self.pump_events();

            if let Some(worker) = self.worker.get() {
                worker.probe();
            }

            // Headless pipeline test: if NV_OPEN_FILE is set, load it directly
            // instead of waiting for the user to click "Open Raw Scans".
            if let Ok(path) = std::env::var("NV_OPEN_FILE") {
                let win_weak = window.downgrade();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    let Some(win) = win_weak.upgrade() else { return };
                    eprintln!("nv: NV_OPEN_FILE: loading {path}");
                    match nv_store::RawReader::read(std::path::Path::new(&path)) {
                        Ok(buf) => win.load_frame_buf_for_test(buf),
                        Err(e) => eprintln!("nv: NV_OPEN_FILE: failed to load {path}: {e}"),
                    }
                });
            }
        }
    }

    impl GtkApplicationImpl for Application {}
    impl AdwApplicationImpl for Application {}

    impl Application {
        fn start_sane_worker(&self) {
            // If SANE_CONFIG_DIR is not already set, default to ~/.sane which
            // contains the plustek8200i backend config and dll.conf.
            // SANE_CONFIG_DIR is read by libsane at runtime (not at dlopen time),
            // so setting it here before sane_init is called is sufficient.
            // LD_LIBRARY_PATH must be set before process start for dlopen to use it;
            // the launch script (or desktop file) must set it for custom backends.
            unsafe {
                if std::env::var_os("SANE_CONFIG_DIR").is_none() {
                    if let Some(home) = glib::home_dir().to_str().map(str::to_owned) {
                        std::env::set_var("SANE_CONFIG_DIR", format!("{home}/.sane"));
                    }
                }
            }

            let quirks_path = super::quirks_file_path();
            let worker = AcquireWorker::spawn(quirks_path);
            self.worker.set(worker).ok();
        }

        fn setup_actions(&self) {
            let app = self.obj();

            // probe: re-enumerate SANE devices
            let probe = gio::SimpleAction::new("probe", None);
            let app_weak = app.downgrade();
            probe.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade() {
                    if let Some(worker) = app.imp().worker.get() {
                        worker.probe();
                    }
                }
            });
            app.add_action(&probe);

            // preview: start a preview scan on the current device
            let preview = gio::SimpleAction::new("preview", None);
            let app_weak = app.downgrade();
            preview.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade() {
                    let Some(worker) = app.imp().worker.get() else { return };
                    let Some(win) = app.active_window()
                        .and_downcast::<crate::window::Window>() else { return };

                    // Get device name from window state.
                    if let crate::window::imp::ScanUiState::Ready { device_name } =
                        &*win.imp().scan_state.borrow()
                    {
                        worker.start_scan(device_name.clone(), 300, false);
                    }
                }
            });
            app.add_action(&preview);
        }

        pub fn pump_events(&self) {
            let app = self.obj().clone();
            let rx = self.worker.get().unwrap().evt_rx.clone();

            glib::MainContext::default().spawn_local(async move {
                while let Ok(event) = rx.recv().await {
                    if let Some(win) = app
                        .active_window()
                        .and_downcast::<crate::window::Window>()
                    {
                        win.handle_scan_event(event);
                    }
                }
            });
        }
    }
}

use glib::ExitCode;
use gtk::prelude::ApplicationExtManual;

glib::wrapper! {
    pub struct Application(ObjectSubclass<imp::Application>)
        @extends adw::Application, gtk::Application, gio::Application,
        @implements gio::ActionMap, gio::ActionGroup;
}

impl Application {
    fn new() -> Self {
        glib::Object::builder()
            .property("application-id", crate::config::APP_ID)
            .property("flags", gio::ApplicationFlags::empty())
            .build()
    }

    pub fn run() -> ExitCode {
        let app = Self::new();
        ApplicationExtManual::run(&app)
    }
}

fn quirks_file_path() -> std::path::PathBuf {
    if let Some(cfg_dir) = glib::user_config_dir().to_str().map(std::path::PathBuf::from) {
        let user = cfg_dir.join("negative-vibes").join("devices.toml");
        if user.exists() { return user; }
    }
    std::path::PathBuf::from(crate::config::PKGDATADIR).join("devices.toml")
}
