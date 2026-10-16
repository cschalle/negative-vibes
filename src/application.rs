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
            // The icon: found in the theme once installed; from the source tree when run with cargo.
            if let Some(display) = gtk::gdk::Display::default() {
                let dir = std::path::Path::new(crate::config::PKGDATADIR).join("icons");
                if dir.is_dir() {
                    gtk::IconTheme::for_display(&display).add_search_path(dir);
                }
                gtk::Window::set_default_icon_name(crate::config::APP_ID);
            }
            self.start_sane_worker();
            self.setup_actions();
        }

        fn activate(&self) {
            self.parent_activate();
            let app = self.obj();
            if let Some(win) = app.main_window() {
                win.present();
                return;
            }
            let window = crate::window::Window::new(app.upcast_ref::<gtk::Application>());
            app.set_accels_for_action("win.undo", &["<Ctrl>z"]);
            app.set_accels_for_action("win.redo", &["<Ctrl>y", "<Ctrl><Shift>z"]);
            app.set_accels_for_action("win.export", &["<Ctrl>e"]);
            app.set_accels_for_action("win.metadata", &["<Ctrl>i"]);
            app.set_accels_for_action("win.open-roll", &["<Ctrl>o"]);
            app.set_accels_for_action("win.preview-film", &["<Ctrl>r"]);
            app.set_accels_for_action("win.toggle-frames", &["F9"]);
            app.set_accels_for_action("win.toggle-inspector", &["<Shift>F9"]);
            app.set_accels_for_action("win.shortcuts", &["<Ctrl>question"]);
            app.set_accels_for_action("win.help", &["F1"]);
            app.set_accels_for_action("app.quit", &["<Ctrl>q"]);
            app.set_accels_for_action("win.preferences", &["<Ctrl>comma"]);
            app.set_accels_for_action("win.rotate-left", &["<Ctrl>Left"]);
            app.set_accels_for_action("win.rotate-right", &["<Ctrl>Right"]);
            app.set_accels_for_action("win.scan-selected", &["<Ctrl>Return"]);
            app.set_accels_for_action("win.scan-all", &["<Ctrl><Shift>Return"]);
            app.set_accels_for_action("win.zoom-in", &["<Ctrl>plus", "<Ctrl>equal", "<Ctrl>KP_Add"]);
            app.set_accels_for_action("win.zoom-out", &["<Ctrl>minus", "<Ctrl>KP_Subtract"]);
            app.set_accels_for_action("win.zoom-fit", &["<Ctrl>0", "<Ctrl>KP_0"]);
            app.set_accels_for_action("win.zoom-100", &["<Ctrl>1", "<Ctrl>KP_1"]);
            app.set_accels_for_action("win.detect-frames", &["<Ctrl><Shift>f"]);
            app.set_accels_for_action("win.next-frame", &["Page_Down"]);
            app.set_accels_for_action("win.prev-frame", &["Page_Up"]);
            app.set_accels_for_action("win.delete-frame", &["Delete"]);
            app.set_accels_for_action("win.duplicate-frame", &["<Ctrl>d"]);
            window.present();
            if let Some(limit) = std::env::var("NV_DEBUG_MINWIDTH").ok().and_then(|v| v.parse().ok()) {
                let w = window.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(2500), move || {
                    crate::window::print_wide_widgets(w.upcast_ref(), limit);
                    crate::window::print_min_widths(&w);
                    if let Some(a) = gtk::prelude::GtkWindowExt::application(&w) {
                        gio::prelude::ApplicationExt::quit(&a);
                    }
                });
            }
            {
                let w = window.clone();
                glib::idle_add_local_once(move || w.label_for_screen_readers());
            }
            // Headless check that the dialogs build without GTK complaints: NV_OPEN_DIALOGS=1.
            if std::env::var_os("NV_OPEN_DIALOGS").is_some() {
                let w = window.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(600), move || {
                    for name in ["preferences", "export", "guided", "metadata", "film-create", "calibrate", "about", "shortcuts"] {
                        gio::prelude::ActionGroupExt::activate_action(&w, name, None);
                    }
                    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                        if let Some(a) = gtk::prelude::GtkWindowExt::application(&w) {
                            gio::prelude::ApplicationExt::quit(&a);
                        }
                    });
                });
            }
            if window.should_show_guide() {
                let w = window.clone();
                glib::idle_add_local_once(move || gio::prelude::ActionGroupExt::activate_action(&w, "guided", None));
            }

            self.pump_events();

            if let Some(worker) = self.worker.get() {
                worker.probe();
            }

            // Headless import test: NV_IMPORT_FILES=a.pnm:b.pnm behaves like
            // Open Raw Scans with those files.
            if let Ok(list) = std::env::var("NV_IMPORT_FILES") {
                let win_weak = window.downgrade();
                glib::timeout_add_local_once(std::time::Duration::from_millis(500), move || {
                    let Some(win) = win_weak.upgrade() else { return };
                    win.import_raw_files_for_test(std::env::split_paths(&list).collect());
                });
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
            // Use the system SANE configuration (/etc/sane.d) unless the caller
            // set SANE_CONFIG_DIR. The plustek8200i backend is installed there;
            // forcing ~/.sane, which has no dll.conf, loads no backends at all.
            let quirks_path = super::quirks_file_path();
            let worker = AcquireWorker::spawn(quirks_path);
            self.worker.set(worker).ok();
        }

        fn setup_actions(&self) {
            let app = self.obj();

            // quit: close every window, so each saves what it has pending
            let quit = gio::SimpleAction::new("quit", None);
            let app_weak = app.downgrade();
            quit.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade() {
                    for w in gtk::prelude::GtkApplicationExt::windows(&app) {
                        gtk::prelude::GtkWindowExt::close(&w);
                    }
                }
            });
            app.add_action(&quit);

            // probe: re-enumerate SANE devices
            let probe = gio::SimpleAction::new("probe", None);
            let app_weak = app.downgrade();
            probe.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade()
                    && let Some(worker) = app.imp().worker.get()
                {
                    worker.probe();
                }
            });
            app.add_action(&probe);

            // select-scanner: let the user choose scanners (the scanner portal's dialog), then probe.
            // No window identifier yet: ashpd's GTK helper needs gtk4 0.11 and the app is on 0.9,
            // so the portal's dialog is not attached to our window until we export a handle.
            let select = gio::SimpleAction::new("select-scanner", None);
            let app_weak = app.downgrade();
            select.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade()
                    && let Some(worker) = app.imp().worker.get()
                {
                    worker.select_scanners(String::new());
                }
            });
            app.add_action(&select);

            // preview: start a preview scan on the current device
            let preview = gio::SimpleAction::new("preview", None);
            let app_weak = app.downgrade();
            preview.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade() {
                    let Some(worker) = app.imp().worker.get() else { return };
                    let Some(win) = app.main_window() else { return };

                    // Get device name from window state.
                    if let crate::window::imp::ScanUiState::Ready { device_name } = &*win.imp().scan_state.borrow() {
                        worker.start_scan(device_name.clone(), win.preview_dpi(), false, None);
                    } else {
                        win.scanner_not_ready();
                    }
                }
            });
            app.add_action(&preview);

            // scan: full-resolution scan of the current frame
            let scan = gio::SimpleAction::new("scan", None);
            let app_weak = app.downgrade();
            scan.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade() {
                    let Some(worker) = app.imp().worker.get() else { return };
                    let Some(win) = app.main_window() else { return };

                    if let crate::window::imp::ScanUiState::Ready { device_name } = &*win.imp().scan_state.borrow() {
                        worker.start_scan_with(device_name.clone(), win.final_scan_dpi(), win.scan_with_ir(), win.scan_area(), win.scan_passes());
                    } else {
                        win.scanner_not_ready();
                    }
                }
            });
            app.add_action(&scan);

            // cancel-scan: stop the acquisition in flight (the worker stops at its next read)
            let cancel = gio::SimpleAction::new("cancel-scan", None);
            let app_weak = app.downgrade();
            cancel.connect_activate(move |_, _| {
                if let Some(app) = app_weak.upgrade()
                    && let Some(worker) = app.imp().worker.get()
                {
                    worker.cancel();
                }
            });
            app.add_action(&cancel);
        }

        pub fn pump_events(&self) {
            let app = self.obj().clone();
            let rx = self.worker.get().unwrap().evt_rx.clone();

            glib::MainContext::default().spawn_local(async move {
                while let Ok(event) = rx.recv().await {
                    if let Some(win) = app.main_window() {
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
    /// The main window, whichever window has the focus: dialogs and the guide are
    /// windows of the application too, and scanner events and actions are for the main one.
    pub fn main_window(&self) -> Option<crate::window::Window> {
        gtk::prelude::GtkApplicationExt::windows(self).into_iter().find_map(|w| glib::prelude::Cast::downcast::<crate::window::Window>(w).ok())
    }

    fn new() -> Self {
        glib::Object::builder().property("application-id", crate::config::APP_ID).property("flags", gio::ApplicationFlags::empty()).build()
    }

    pub fn run() -> ExitCode {
        let app = Self::new();
        ApplicationExtManual::run(&app)
    }
}

fn quirks_file_path() -> std::path::PathBuf {
    if let Some(cfg_dir) = glib::user_config_dir().to_str().map(std::path::PathBuf::from) {
        let user = cfg_dir.join("negative-vibes").join("devices.toml");
        if user.exists() {
            return user;
        }
    }
    std::path::PathBuf::from(crate::config::PKGDATADIR).join("devices.toml")
}
