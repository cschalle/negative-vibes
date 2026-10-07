pub(crate) mod imp {
    use std::cell::{Cell, RefCell};
    use adw::subclass::prelude::*;

    #[derive(Default)]
    pub struct Window {
        // ── Widget handles ────────────────────────────────────────────────
        pub outer_split: RefCell<Option<adw::OverlaySplitView>>,
        pub inner_split: RefCell<Option<adw::OverlaySplitView>>,
        pub main_stack: RefCell<Option<gtk::Stack>>,
        pub canvas_picture: RefCell<Option<gtk::Picture>>,
        pub frame_list: RefCell<Option<gtk::ListBox>>,
        pub toast_overlay: RefCell<Option<adw::ToastOverlay>>,
        pub header_title: RefCell<Option<adw::WindowTitle>>,
        pub preview_btn: RefCell<Option<gtk::Button>>,
        pub scan_btn: RefCell<Option<gtk::Button>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        pub advance_banner: RefCell<Option<adw::Banner>>,

        // ── Inspector widget handles ───────────────────────────────────────
        pub image_type_row: RefCell<Option<adw::ComboRow>>,
        pub profile_row: RefCell<Option<adw::ComboRow>>,
        pub exposure_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_r_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_g_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_b_row: RefCell<Option<adw::SpinRow>>,
        pub black_pt_row: RefCell<Option<adw::SpinRow>>,
        pub white_pt_row: RefCell<Option<adw::SpinRow>>,
        pub midtone_row: RefCell<Option<adw::SpinRow>>,
        pub contrast_row: RefCell<Option<adw::SpinRow>>,

        // ── App state ─────────────────────────────────────────────────────
        pub roll: RefCell<Option<nv_store::Roll>>,
        pub scan_buf: RefCell<Option<nv_image::FrameBuf>>,
        pub scan_state: RefCell<ScanUiState>,
        pub selected_frame: RefCell<Option<u32>>,

        // ── M3 processing ─────────────────────────────────────────────────
        /// Current processing parameters.
        pub proc_params: RefCell<Option<nv_process::ProcessParams>>,
        /// Undo/redo stack.  undo_idx points at the *current* state.
        pub undo_stack: RefCell<Vec<nv_process::ProcessParams>>,
        pub undo_idx: Cell<usize>,
        /// Set while syncing UI → params to suppress re-entrant param changes.
        pub updating_ui: Cell<bool>,
        /// True once film base has been estimated (ScanDone) or a raw file loaded.
        /// Gates the pipeline in refresh_canvas so widget-init signals don't arm it.
        pub film_base_ready: Cell<bool>,
    }

    #[derive(Default, Debug)]
    pub enum ScanUiState {
        #[default]
        NoScanner,
        Ready { device_name: String },
        #[allow(dead_code)]
        Previewing,
        #[allow(dead_code)]
        Scanning,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "NvWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build_ui();
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use nv_acquire::ScanEvent;
use nv_film::builtin_profiles;
use nv_image::FrameBuf;
use nv_process::{ProcessParams, params::{ConversionParams, ToneParams}};
use nv_store::{Roll, RollStore};

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Window {
    pub fn new(app: &gtk::Application) -> Self {
        glib::Object::builder()
            .property("application", app)
            .build()
    }

    // ── Build ─────────────────────────────────────────────────────────────────

    fn build_ui(&self) {
        self.set_default_size(1280, 800);

        // ── Header bar ────────────────────────────────────────────────────
        let header = adw::HeaderBar::new();

        let sidebar_btn = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Show Frames")
            .build();

        let inspector_btn = gtk::ToggleButton::builder()
            .icon_name("view-dual-symbolic")
            .tooltip_text("Show Inspector (Ctrl+I)")
            .build();

        let preview_btn = gtk::Button::builder()
            .label("Preview")
            .tooltip_text("Preview the current frame (Ctrl+R)")
            .sensitive(false)
            .build();

        let scan_btn = gtk::Button::builder()
            .label("Scan")
            .css_classes(["suggested-action"])
            .tooltip_text("Scan the current frame (Ctrl+Return)")
            .sensitive(false)
            .build();

        let open_btn = gtk::Button::builder()
            .icon_name("folder-open-symbolic")
            .tooltip_text("Open Roll…")
            .build();

        let title_widget = adw::WindowTitle::new("Negative Vibes", "");

        header.pack_start(&sidebar_btn);
        header.pack_start(&preview_btn);
        header.set_title_widget(Some(&title_widget));
        header.pack_end(&scan_btn);
        header.pack_end(&open_btn);
        header.pack_end(&inspector_btn);

        // ── Bottom bar ────────────────────────────────────────────────────
        let zoom_out = gtk::Button::builder().icon_name("zoom-out-symbolic").build();
        let zoom_label = gtk::Label::new(Some("Fit"));
        let zoom_in = gtk::Button::builder().icon_name("zoom-in-symbolic").build();
        let fit_btn = gtk::Button::builder()
            .label("Fit")
            .tooltip_text("Fit to window (Ctrl+0)")
            .build();
        let _ = (zoom_out, zoom_label, zoom_in, fit_btn); // TODO: zoom impl

        let progress_bar = gtk::ProgressBar::builder()
            .hexpand(true)
            .visible(false)
            .build();

        let bottom_bar = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(3)
            .margin_bottom(3)
            .build();
        bottom_bar.append(&progress_bar);

        // ── Canvas stack ──────────────────────────────────────────────────
        let no_scanner = adw::StatusPage::new();
        no_scanner.set_icon_name(Some("scanner-symbolic"));
        no_scanner.set_title("No Scanner Found");
        no_scanner.set_description(Some(
            "Make sure your scanner is connected and powered on.",
        ));
        let try_again = gtk::Button::builder()
            .label("Try Again")
            .css_classes(["pill", "suggested-action"])
            .build();
        let open_raw = gtk::Button::builder()
            .label("Open Raw Scans…")
            .css_classes(["pill"])
            .build();
        let open_roll = gtk::Button::builder()
            .label("Open Roll…")
            .css_classes(["pill"])
            .build();
        let btn_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(8)
            .halign(gtk::Align::Center)
            .build();
        btn_box.append(&try_again);
        btn_box.append(&open_raw);
        btn_box.append(&open_roll);
        no_scanner.set_child(Some(&btn_box));

        let insert_page = adw::StatusPage::new();
        insert_page.set_icon_name(Some("scanner-symbolic"));
        insert_page.set_title("Insert Film and Preview");
        insert_page.set_description(Some("Insert film and click Preview to begin."));
        let preview_big = gtk::Button::builder()
            .label("Preview")
            .css_classes(["pill", "suggested-action"])
            .halign(gtk::Align::Center)
            .build();
        insert_page.set_child(Some(&preview_big));

        let canvas_picture = gtk::Picture::builder()
            .hexpand(true)
            .vexpand(true)
            .content_fit(gtk::ContentFit::Contain)
            .build();
        let canvas_scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&canvas_picture)
            .build();

        let advance_banner = adw::Banner::new("Move the holder to the next frame");
        advance_banner.set_button_label(Some("Scan Next Frame"));
        advance_banner.set_revealed(false);

        let canvas_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .build();
        canvas_box.append(&advance_banner);
        canvas_box.append(&canvas_scroll);

        let main_stack = gtk::Stack::new();
        main_stack.add_named(&no_scanner, Some("no-scanner"));
        main_stack.add_named(&insert_page, Some("insert-film"));
        main_stack.add_named(&canvas_box, Some("canvas"));
        main_stack.set_visible_child_name("no-scanner");

        // ── Inspector panel ───────────────────────────────────────────────
        let inspector_box = self.build_inspector();

        // ── Toolbar view ──────────────────────────────────────────────────
        let toolbar_view = adw::ToolbarView::new();
        toolbar_view.add_top_bar(&header);
        toolbar_view.set_content(Some(&main_stack));
        toolbar_view.add_bottom_bar(&bottom_bar);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));

        // ── Inner split view (inspector) ──────────────────────────────────
        let inner_split = adw::OverlaySplitView::new();
        inner_split.set_sidebar_position(gtk::PackType::End);
        inner_split.set_content(Some(toast_overlay.upcast_ref::<gtk::Widget>()));
        inner_split.set_sidebar(Some(inspector_box.upcast_ref::<gtk::Widget>()));
        inner_split.set_show_sidebar(false);
        inner_split.set_sidebar_width_fraction(0.28);
        inner_split.set_min_sidebar_width(280.0);

        // ── Frames sidebar ────────────────────────────────────────────────
        let frame_list = gtk::ListBox::new();
        frame_list.set_selection_mode(gtk::SelectionMode::Single);
        frame_list.add_css_class("navigation-sidebar");
        frame_list.set_vexpand(true);

        let add_frame_btn = gtk::Button::builder()
            .label("Add Frame")
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&frame_list)
            .build();

        let sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(160)
            .build();
        sidebar_box.append(&sidebar_scroll);
        sidebar_box.append(&add_frame_btn);

        // ── Outer split view (frames sidebar) ────────────────────────────
        let outer_split = adw::OverlaySplitView::new();
        outer_split.set_sidebar_position(gtk::PackType::Start);
        outer_split.set_content(Some(inner_split.upcast_ref::<gtk::Widget>()));
        outer_split.set_sidebar(Some(sidebar_box.upcast_ref::<gtk::Widget>()));
        outer_split.set_sidebar_width_fraction(0.15);
        outer_split.set_min_sidebar_width(140.0);

        self.set_content(Some(&outer_split));

        // ── Store widget handles ──────────────────────────────────────────
        *self.imp().outer_split.borrow_mut() = Some(outer_split.clone());
        *self.imp().inner_split.borrow_mut() = Some(inner_split.clone());
        *self.imp().main_stack.borrow_mut() = Some(main_stack);
        *self.imp().canvas_picture.borrow_mut() = Some(canvas_picture);
        *self.imp().frame_list.borrow_mut() = Some(frame_list.clone());
        *self.imp().toast_overlay.borrow_mut() = Some(toast_overlay);
        *self.imp().header_title.borrow_mut() = Some(title_widget);
        *self.imp().preview_btn.borrow_mut() = Some(preview_btn.clone());
        *self.imp().scan_btn.borrow_mut() = Some(scan_btn.clone());
        *self.imp().progress_bar.borrow_mut() = Some(progress_bar);
        *self.imp().advance_banner.borrow_mut() = Some(advance_banner.clone());

        // Initialise default processing params (generic colour negative).
        let profiles = builtin_profiles();
        let default_params = nv_process::default_params_for(&profiles[0]);
        let mut stack = Vec::new();
        stack.push(default_params.clone());
        *self.imp().proc_params.borrow_mut() = Some(default_params);
        *self.imp().undo_stack.borrow_mut() = stack;
        self.imp().undo_idx.set(0);

        // ── Connect signals ───────────────────────────────────────────────
        sidebar_btn.bind_property("active", &outer_split, "show-sidebar")
            .sync_create()
            .build();

        inspector_btn.bind_property("active", &inner_split, "show-sidebar")
            .sync_create()
            .build();

        let win_weak = self.downgrade();
        try_again.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.emit_probe(); }
        });

        let win_weak = self.downgrade();
        preview_big.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.start_preview(); }
        });

        let win_weak = self.downgrade();
        preview_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.start_preview(); }
        });

        let win_weak = self.downgrade();
        open_raw.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.open_raw_scans(); }
        });

        let win_weak = self.downgrade();
        open_roll.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.open_roll_dialog(); }
        });

        let win_weak = self.downgrade();
        open_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.open_roll_dialog(); }
        });

        let win_weak = self.downgrade();
        add_frame_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.add_frame(); }
        });

        let win_weak = self.downgrade();
        advance_banner.connect_button_clicked(move |banner| {
            banner.set_revealed(false);
            if let Some(win) = win_weak.upgrade() { win.start_preview(); }
        });

        // ── Undo/redo GActions ────────────────────────────────────────────
        let undo_action = gio::SimpleAction::new("undo", None);
        let win_weak = self.downgrade();
        undo_action.connect_activate(move |_, _| {
            if let Some(win) = win_weak.upgrade() { win.do_undo(); }
        });
        self.add_action(&undo_action);

        let redo_action = gio::SimpleAction::new("redo", None);
        let win_weak = self.downgrade();
        redo_action.connect_activate(move |_, _| {
            if let Some(win) = win_weak.upgrade() { win.do_redo(); }
        });
        self.add_action(&redo_action);

        // Keyboard shortcuts are set in application.rs after the window is added to the app.
    }

    // ── Inspector panel builder ───────────────────────────────────────────────

    fn build_inspector(&self) -> gtk::Box {
        let inspector_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(300)
            .build();

        // Title bar
        let title_label = gtk::Label::builder()
            .label("Inspector")
            .css_classes(["title-4"])
            .margin_top(12)
            .margin_bottom(4)
            .margin_start(12)
            .halign(gtk::Align::Start)
            .build();
        inspector_box.append(&title_label);

        // Page stack
        let insp_stack = gtk::Stack::builder()
            .vexpand(true)
            .build();
        insp_stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);

        let switcher = gtk::StackSwitcher::builder()
            .stack(&insp_stack)
            .margin_start(8)
            .margin_end(8)
            .margin_bottom(4)
            .build();

        // ── Conversion page ───────────────────────────────────────────────
        let conv_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let conv_page = adw::PreferencesPage::new();

        // Image group
        let image_group = adw::PreferencesGroup::builder()
            .title("Image")
            .build();

        let image_type_row = adw::ComboRow::builder()
            .title("Type")
            .build();
        let type_model = gtk::StringList::new(&["Colour Negative", "B&W Negative", "Positive"]);
        image_type_row.set_model(Some(&type_model));
        image_group.add(&image_type_row);

        let profiles = builtin_profiles();
        let profile_names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        let profile_row = adw::ComboRow::builder()
            .title("Film Profile")
            .build();
        let profile_model = gtk::StringList::new(&profile_names);
        profile_row.set_model(Some(&profile_model));
        image_group.add(&profile_row);
        conv_page.add(&image_group);

        // Exposure group
        let exp_group = adw::PreferencesGroup::builder()
            .title("Exposure")
            .build();

        let exposure_adj = gtk::Adjustment::new(0.0, -3.0, 3.0, 0.1, 0.5, 0.0);
        let exposure_row = adw::SpinRow::builder()
            .title("Exposure")
            .subtitle("stops")
            .adjustment(&exposure_adj)
            .digits(1)
            .build();
        exp_group.add(&exposure_row);

        // Film base sub-group
        let base_group = adw::PreferencesGroup::builder()
            .title("Film Base")
            .description("Auto-estimated from rebate borders")
            .build();

        let fb_r_adj = gtk::Adjustment::new(0.6, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_r_row = adw::SpinRow::builder()
            .title("Red")
            .adjustment(&fb_r_adj)
            .digits(3)
            .build();
        base_group.add(&film_base_r_row);

        let fb_g_adj = gtk::Adjustment::new(0.3, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_g_row = adw::SpinRow::builder()
            .title("Green")
            .adjustment(&fb_g_adj)
            .digits(3)
            .build();
        base_group.add(&film_base_g_row);

        let fb_b_adj = gtk::Adjustment::new(0.18, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_b_row = adw::SpinRow::builder()
            .title("Blue")
            .adjustment(&fb_b_adj)
            .digits(3)
            .build();
        base_group.add(&film_base_b_row);

        conv_page.add(&exp_group);
        conv_page.add(&base_group);
        conv_scroll.set_child(Some(&conv_page));
        insp_stack.add_titled(&conv_scroll, Some("conversion"), "Conversion");

        // ── Tone page ─────────────────────────────────────────────────────
        let tone_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let tone_page = adw::PreferencesPage::new();
        let tone_group = adw::PreferencesGroup::builder()
            .title("Tone")
            .build();

        let bp_adj = gtk::Adjustment::new(0.0, 0.0, 1.0, 0.01, 0.1, 0.0);
        let black_pt_row = adw::SpinRow::builder()
            .title("Black Point")
            .adjustment(&bp_adj)
            .digits(2)
            .build();
        tone_group.add(&black_pt_row);

        let wp_adj = gtk::Adjustment::new(1.0, 0.0, 1.0, 0.01, 0.1, 0.0);
        let white_pt_row = adw::SpinRow::builder()
            .title("White Point")
            .adjustment(&wp_adj)
            .digits(2)
            .build();
        tone_group.add(&white_pt_row);

        let mt_adj = gtk::Adjustment::new(1.0, 0.25, 4.0, 0.05, 0.25, 0.0);
        let midtone_row = adw::SpinRow::builder()
            .title("Midtone")
            .subtitle("gamma")
            .adjustment(&mt_adj)
            .digits(2)
            .build();
        tone_group.add(&midtone_row);

        let ct_adj = gtk::Adjustment::new(0.0, -1.0, 1.0, 0.05, 0.25, 0.0);
        let contrast_row = adw::SpinRow::builder()
            .title("Contrast")
            .adjustment(&ct_adj)
            .digits(2)
            .build();
        tone_group.add(&contrast_row);

        tone_page.add(&tone_group);
        tone_scroll.set_child(Some(&tone_page));
        insp_stack.add_titled(&tone_scroll, Some("tone"), "Tone");

        inspector_box.append(&switcher);
        inspector_box.append(&insp_stack);

        // ── Store inspector handles ───────────────────────────────────────
        *self.imp().image_type_row.borrow_mut() = Some(image_type_row.clone());
        *self.imp().profile_row.borrow_mut() = Some(profile_row.clone());
        *self.imp().exposure_row.borrow_mut() = Some(exposure_row.clone());
        *self.imp().film_base_r_row.borrow_mut() = Some(film_base_r_row.clone());
        *self.imp().film_base_g_row.borrow_mut() = Some(film_base_g_row.clone());
        *self.imp().film_base_b_row.borrow_mut() = Some(film_base_b_row.clone());
        *self.imp().black_pt_row.borrow_mut() = Some(black_pt_row.clone());
        *self.imp().white_pt_row.borrow_mut() = Some(white_pt_row.clone());
        *self.imp().midtone_row.borrow_mut() = Some(midtone_row.clone());
        *self.imp().contrast_row.borrow_mut() = Some(contrast_row.clone());

        // ── Connect inspector signals ─────────────────────────────────────
        let win_weak = self.downgrade();
        image_type_row.connect_selected_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        profile_row.connect_selected_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        exposure_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        film_base_r_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        film_base_g_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        film_base_b_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        black_pt_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        white_pt_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        midtone_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });
        let win_weak = self.downgrade();
        contrast_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() { win.on_params_changed(); }
        });

        inspector_box
    }

    // ── Inspector <→ ProcessParams sync ──────────────────────────────────────

    /// Called whenever any inspector widget changes; reads all values and
    /// pushes a new undo state.
    fn on_params_changed(&self) {
        if self.imp().updating_ui.get() { return; }
        // Spin rows fire notify::value during widget realization; ignore those
        // until an actual scan is loaded.
        if self.imp().scan_buf.borrow().is_none() { return; }

        let new_params = self.read_params_from_inspector();
        let Some(new_params) = new_params else { return };

        // Push onto undo stack (truncate redo branch).
        let mut stack = self.imp().undo_stack.borrow_mut();
        let idx = self.imp().undo_idx.get();
        stack.truncate(idx + 1);
        stack.push(new_params.clone());
        self.imp().undo_idx.set(stack.len() - 1);
        drop(stack);

        *self.imp().proc_params.borrow_mut() = Some(new_params);
        self.refresh_canvas();
    }

    fn read_params_from_inspector(&self) -> Option<ProcessParams> {
        let profiles = builtin_profiles();

        let image_type = {
            let row = self.imp().image_type_row.borrow();
            let row = row.as_ref()?;
            match row.selected() {
                0 => nv_process::params::ImageType::ColourNegative,
                1 => nv_process::params::ImageType::BwNegative,
                _ => nv_process::params::ImageType::Positive,
            }
        };

        let profile = {
            let row = self.imp().profile_row.borrow();
            let row = row.as_ref()?;
            let idx = (row.selected() as usize).min(profiles.len() - 1);
            profiles[idx].clone()
        };

        let exposure = self.imp().exposure_row.borrow().as_ref()?.value() as f32;
        let fb_r = self.imp().film_base_r_row.borrow().as_ref()?.value() as f32;
        let fb_g = self.imp().film_base_g_row.borrow().as_ref()?.value() as f32;
        let fb_b = self.imp().film_base_b_row.borrow().as_ref()?.value() as f32;
        let black_pt = self.imp().black_pt_row.borrow().as_ref()?.value() as f32;
        let white_pt = self.imp().white_pt_row.borrow().as_ref()?.value() as f32;
        let midtone = self.imp().midtone_row.borrow().as_ref()?.value() as f32;
        let contrast = self.imp().contrast_row.borrow().as_ref()?.value() as f32;

        Some(ProcessParams {
            image_type,
            profile,
            conversion: ConversionParams {
                film_base: Some([fb_r, fb_g, fb_b]),
                exposure,
                tolerance: 0.0016,
                auto_ccr: true,
            },
            tone: ToneParams { black_point: black_pt, white_point: white_pt, midtone, contrast },
        })
    }

    /// Push new params (without incrementing undo from UI signals — used when
    /// loading a new frame, e.g. auto film-base estimation).
    fn set_params_and_sync_ui(&self, params: ProcessParams) {
        self.imp().updating_ui.set(true);

        // Set inspector widget values.
        let image_type_idx = match params.image_type {
            nv_process::params::ImageType::ColourNegative => 0u32,
            nv_process::params::ImageType::BwNegative => 1,
            nv_process::params::ImageType::Positive => 2,
        };
        if let Some(r) = self.imp().image_type_row.borrow().as_ref() {
            r.set_selected(image_type_idx);
        }

        // Find matching profile by id.
        let profiles = builtin_profiles();
        if let Some(idx) = profiles.iter().position(|p| p.id == params.profile.id) {
            if let Some(r) = self.imp().profile_row.borrow().as_ref() {
                r.set_selected(idx as u32);
            }
        }

        if let Some(r) = self.imp().exposure_row.borrow().as_ref() {
            r.set_value(params.conversion.exposure as f64);
        }
        let fb = params.conversion.film_base.unwrap_or(params.profile.base_rgb);
        if let Some(r) = self.imp().film_base_r_row.borrow().as_ref() { r.set_value(fb[0] as f64); }
        if let Some(r) = self.imp().film_base_g_row.borrow().as_ref() { r.set_value(fb[1] as f64); }
        if let Some(r) = self.imp().film_base_b_row.borrow().as_ref() { r.set_value(fb[2] as f64); }
        if let Some(r) = self.imp().black_pt_row.borrow().as_ref() { r.set_value(params.tone.black_point as f64); }
        if let Some(r) = self.imp().white_pt_row.borrow().as_ref() { r.set_value(params.tone.white_point as f64); }
        if let Some(r) = self.imp().midtone_row.borrow().as_ref() { r.set_value(params.tone.midtone as f64); }
        if let Some(r) = self.imp().contrast_row.borrow().as_ref() { r.set_value(params.tone.contrast as f64); }

        self.imp().updating_ui.set(false);

        // Reset undo stack to just this state.
        let mut stack = self.imp().undo_stack.borrow_mut();
        *stack = vec![params.clone()];
        drop(stack);
        self.imp().undo_idx.set(0);
        *self.imp().proc_params.borrow_mut() = Some(params);
    }

    // ── Undo / redo ───────────────────────────────────────────────────────────

    fn do_undo(&self) {
        let idx = self.imp().undo_idx.get();
        if idx == 0 { return; }
        let new_idx = idx - 1;
        self.imp().undo_idx.set(new_idx);
        let params = self.imp().undo_stack.borrow()[new_idx].clone();
        self.imp().updating_ui.set(true);
        self.set_params_and_sync_ui(params.clone());
        self.imp().updating_ui.set(false);
        *self.imp().proc_params.borrow_mut() = Some(params);
        // Restore undo stack (set_params_and_sync_ui resets it — fix that up).
        self.imp().undo_idx.set(new_idx);
        self.refresh_canvas();
    }

    fn do_redo(&self) {
        let stack_len = self.imp().undo_stack.borrow().len();
        let idx = self.imp().undo_idx.get();
        if idx + 1 >= stack_len { return; }
        let new_idx = idx + 1;
        self.imp().undo_idx.set(new_idx);
        let params = self.imp().undo_stack.borrow()[new_idx].clone();
        self.imp().updating_ui.set(true);
        self.set_params_and_sync_ui(params.clone());
        self.imp().updating_ui.set(false);
        *self.imp().proc_params.borrow_mut() = Some(params);
        self.imp().undo_idx.set(new_idx);
        self.refresh_canvas();
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    fn emit_probe(&self) {
        if let Some(app) = self.application() {
            app.activate_action("probe", None);
        }
    }

    fn start_preview(&self) {
        if let Some(app) = self.application() {
            app.activate_action("preview", None);
        }
    }

    fn add_frame(&self) {
        let mut roll_ref = self.imp().roll.borrow_mut();
        if let Some(roll) = roll_ref.as_mut() {
            roll.add_frame();
            drop(roll_ref);
            self.rebuild_frame_list();
            self.schedule_autosave();
        }
    }

    // ── Scanner event handler ─────────────────────────────────────────────────

    pub fn handle_scan_event(&self, event: ScanEvent) {
        use imp::ScanUiState;

        match event {
            ScanEvent::DeviceList(caps) => {
                if caps.is_empty() {
                    self.set_page("no-scanner");
                    *self.imp().scan_state.borrow_mut() = ScanUiState::NoScanner;
                    self.set_scan_controls_sensitive(false, false);
                } else {
                    let first = caps.into_iter().next().unwrap();
                    self.set_scan_controls_sensitive(true, false);
                    if self.imp().roll.borrow().is_none() {
                        self.set_page("insert-film");
                    }
                    *self.imp().scan_state.borrow_mut() = ScanUiState::Ready {
                        device_name: first.device_name.clone(),
                    };
                    if let Some(t) = self.imp().header_title.borrow().as_ref() {
                        t.set_subtitle(&format!("{} {}", first.vendor, first.model));
                    }
                }
            }

            ScanEvent::ScanStarted { width_px, height_px, bytes_per_line, depth, channels } => {
                let buf = FrameBuf::with_stride(width_px, height_px, depth, channels, bytes_per_line);
                *self.imp().scan_buf.borrow_mut() = Some(buf);
                self.imp().film_base_ready.set(false);
                self.set_page("canvas");
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(true);
                    pb.set_fraction(0.0);
                }
            }

            ScanEvent::DataChunk { data, .. } => {
                let mut buf_ref = self.imp().scan_buf.borrow_mut();
                if let Some(buf) = buf_ref.as_mut() {
                    buf.append(&data);
                    if buf.rows_available() % 32 == 0 {
                        drop(buf_ref);
                        self.refresh_canvas();
                    }
                }
            }

            ScanEvent::Progress(p) => {
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_fraction(p as f64);
                }
            }

            ScanEvent::ScanDone => {
                // Auto-estimate film base now that the full scan is in.
                self.auto_estimate_film_base();
                self.refresh_canvas();
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.on_scan_done();
            }

            ScanEvent::Cancelled => {
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
            }

            ScanEvent::Error(msg) => {
                self.show_toast(&format!("Scanner error: {msg}"));
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
            }

            ScanEvent::DeviceReady { .. }
            | ScanEvent::ColorFrameDone
            | ScanEvent::IrFrameDone => {}
        }
    }

    // ── Film base auto-estimation ─────────────────────────────────────────────

    fn auto_estimate_film_base(&self) {
        let buf_ref = self.imp().scan_buf.borrow();
        let Some(buf) = buf_ref.as_ref() else { return };

        let estimated = nv_process::estimate_film_base(buf);
        drop(buf_ref);

        // Update the film-base spin rows (suppressing re-entrant on_params_changed).
        self.imp().updating_ui.set(true);
        if let Some(r) = self.imp().film_base_r_row.borrow().as_ref() { r.set_value(estimated[0] as f64); }
        if let Some(r) = self.imp().film_base_g_row.borrow().as_ref() { r.set_value(estimated[1] as f64); }
        if let Some(r) = self.imp().film_base_b_row.borrow().as_ref() { r.set_value(estimated[2] as f64); }
        self.imp().updating_ui.set(false);

        // Update the params struct directly and arm the pipeline.
        let mut params_ref = self.imp().proc_params.borrow_mut();
        if let Some(params) = params_ref.as_mut() {
            params.conversion.film_base = Some(estimated);
        }
        drop(params_ref);
        self.imp().film_base_ready.set(true);
    }

    // ── Canvas ────────────────────────────────────────────────────────────────

    fn refresh_canvas(&self) {
        let buf_ref = self.imp().scan_buf.borrow();
        let Some(buf) = buf_ref.as_ref() else { return };
        let h = buf.rows_available();
        let w = buf.width;
        if w == 0 || h == 0 { return; }

        let params_ref = self.imp().proc_params.borrow();
        let film_base_ready = self.imp().film_base_ready.get();
        // Only run the full pipeline once film base has been properly estimated
        // (after ScanDone or file load). Before that use the fast sqrt-gamma
        // preview so mid-scan display and startup don't amplify noise.
        let rgb8 = if film_base_ready {
            match params_ref.as_ref() {
                Some(params) => nv_process::process(buf, params),
                None => buf.to_rgb8(),
            }
        } else {
            buf.to_rgb8()
        };
        drop(params_ref);
        drop(buf_ref);

        if rgb8.is_empty() { return; }

        // Expand RGB8 → RGBA8 (alpha=255).  GDK_MEMORY_R8G8B8 (3-byte packed) is
        // not natively supported by most GPU drivers and GTK's internal format
        // conversion can produce corrupted results on Wayland/Vulkan backends.
        // R8G8B8A8 is universally supported.
        let rgba8: Vec<u8> = rgb8
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255u8])
            .collect();
        let bytes = glib::Bytes::from_owned(rgba8);
        let texture = gdk::MemoryTexture::new(
            w as i32,
            h as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            (w * 4) as usize,
        );

        if let Some(pic) = self.imp().canvas_picture.borrow().as_ref() {
            pic.set_paintable(Some(texture.upcast_ref::<gdk::Paintable>()));
        }
    }

    // ── Roll management ───────────────────────────────────────────────────────

    pub fn new_roll(&self, name: &str, store_root: &std::path::Path) {
        match RollStore::create(store_root, name) {
            Ok(store) => {
                *self.imp().roll.borrow_mut() = Some(store.roll.clone());
                self.set_property("title", name);
                if let Some(t) = self.imp().header_title.borrow().as_ref() {
                    t.set_title(name);
                }
                self.rebuild_frame_list();
                self.set_page("insert-film");
            }
            Err(e) => self.show_toast(&format!("Could not create roll: {e}")),
        }
    }

    pub fn load_roll(&self, roll: Roll) {
        if let Some(t) = self.imp().header_title.borrow().as_ref() {
            t.set_title(&roll.name);
        }
        *self.imp().roll.borrow_mut() = Some(roll);
        self.rebuild_frame_list();
        self.set_page("canvas");
    }

    fn on_scan_done(&self) {
        let sel = *self.imp().selected_frame.borrow();
        if let Some(idx) = sel {
            let mut roll_ref = self.imp().roll.borrow_mut();
            if let Some(roll) = roll_ref.as_mut() {
                if let Some(frame) = roll.frames.iter_mut().find(|f| f.index == idx) {
                    frame.state = nv_store::FrameState::Previewed;
                }
            }
        }
        drop(self.imp().roll.borrow_mut());
        self.rebuild_frame_list();
        self.schedule_autosave();
    }

    fn rebuild_frame_list(&self) {
        let Some(list) = self.imp().frame_list.borrow().clone() else { return };

        while let Some(child) = list.first_child() {
            list.remove(&child);
        }

        let roll_ref = self.imp().roll.borrow();
        let Some(roll) = roll_ref.as_ref() else { return };

        for frame in &roll.frames {
            let row = self.make_frame_row(frame);
            list.append(&row);
        }
    }

    fn make_frame_row(&self, frame: &nv_store::Frame) -> gtk::ListBoxRow {
        let icon_name = match frame.state {
            nv_store::FrameState::Empty       => "circle-outline-thick-symbolic",
            nv_store::FrameState::Previewed   => "image-missing-symbolic",
            nv_store::FrameState::Scanned     => "emblem-ok-symbolic",
            nv_store::FrameState::NeedsRescan => "view-refresh-symbolic",
            nv_store::FrameState::Exported    => "emblem-shared-symbolic",
        };

        let icon = gtk::Image::from_icon_name(icon_name);
        let label = gtk::Label::new(Some(&format!("{}", frame.index)));
        label.set_xalign(0.0);
        label.set_hexpand(true);

        let row_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .margin_start(6)
            .margin_end(6)
            .margin_top(4)
            .margin_bottom(4)
            .build();
        row_box.append(&label);
        row_box.append(&icon);

        gtk::ListBoxRow::builder()
            .child(&row_box)
            .build()
    }

    fn schedule_autosave(&self) {
        let _ = self.save_roll();
    }

    fn save_roll(&self) -> Option<()> {
        None
    }

    // ── Open Raw Scans ────────────────────────────────────────────────────────

    fn open_raw_scans(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Open Raw Scans")
            .modal(true)
            .build();

        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Linear TIFF / PNM"));
        filter.add_mime_type("image/tiff");
        filter.add_pattern("*.tif");
        filter.add_pattern("*.tiff");
        filter.add_pattern("*.pnm");
        filter.add_pattern("*.ppm");
        filter.add_pattern("*.pgm");

        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));

        let win_weak = self.downgrade();
        dialog.open_multiple(Some(self), None::<&gio::Cancellable>, move |result| {
            let Some(win) = win_weak.upgrade() else { return };
            match result {
                Ok(files) => win.load_raw_files(files),
                Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
                Err(e) => win.show_toast(&format!("Could not open files: {e}")),
            }
        });
    }

    fn load_raw_files(&self, files: gio::ListModel) {
        let n = files.n_items();
        if n == 0 { return; }

        let mut roll = Roll::new("Imported Scans");

        for i in 0..n {
            let file = files.item(i).and_downcast::<gio::File>().unwrap();
            let path = match file.path() {
                Some(p) => p,
                None => continue,
            };

            match nv_store::RawReader::read(&path) {
                Ok(buf) => {
                    let idx = roll.add_frame().index;
                    if i == 0 {
                        self.load_frame_buf(buf);
                    }
                    if let Some(f) = roll.frames.iter_mut().find(|f| f.index == idx) {
                        f.state = nv_store::FrameState::Previewed;
                    }
                }
                Err(e) => {
                    let name = file.basename().map(|b| b.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".to_owned());
                    self.show_toast(&format!("Could not load {name}: {e}"));
                }
            }
        }

        self.load_roll(roll);
    }

    pub fn load_frame_buf_for_test(&self, buf: nv_image::FrameBuf) {
        self.load_frame_buf(buf);
    }

    fn load_frame_buf(&self, buf: nv_image::FrameBuf) {
        // Auto-estimate film base before displaying.
        let estimated = nv_process::estimate_film_base(&buf);
        *self.imp().scan_buf.borrow_mut() = Some(buf);
        self.set_page("canvas");

        // Update film-base in params to the estimated value.
        let mut params_ref = self.imp().proc_params.borrow_mut();
        if let Some(params) = params_ref.as_mut() {
            params.conversion.film_base = Some(estimated);
        }
        drop(params_ref);

        self.imp().updating_ui.set(true);
        if let Some(r) = self.imp().film_base_r_row.borrow().as_ref() { r.set_value(estimated[0] as f64); }
        if let Some(r) = self.imp().film_base_g_row.borrow().as_ref() { r.set_value(estimated[1] as f64); }
        if let Some(r) = self.imp().film_base_b_row.borrow().as_ref() { r.set_value(estimated[2] as f64); }
        self.imp().updating_ui.set(false);
        self.imp().film_base_ready.set(true);

        self.refresh_canvas();
    }

    // ── Open Roll ─────────────────────────────────────────────────────────────

    fn open_roll_dialog(&self) {
        let dialog = gtk::FileDialog::builder()
            .title("Open Roll")
            .modal(true)
            .build();

        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Roll folders (*.nvroll)"));
        filter.add_pattern("*.nvroll");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        dialog.set_filters(Some(&filters));

        let win_weak = self.downgrade();
        dialog.select_folder(Some(self), None::<&gio::Cancellable>, move |result| {
            let Some(win) = win_weak.upgrade() else { return };
            match result {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        win.open_roll_at(&path);
                    }
                }
                Err(e) if e.matches(gtk::DialogError::Dismissed) => {}
                Err(e) => win.show_toast(&format!("Could not open roll: {e}")),
            }
        });
    }

    fn open_roll_at(&self, path: &std::path::Path) {
        match RollStore::open(path) {
            Ok(store) => self.load_roll(store.roll),
            Err(e) => self.show_toast(&format!("Could not open roll: {e}")),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn set_page(&self, name: &str) {
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            stack.set_visible_child_name(name);
        }
    }

    fn set_scan_controls_sensitive(&self, preview: bool, scan: bool) {
        if let Some(b) = self.imp().preview_btn.borrow().as_ref() { b.set_sensitive(preview); }
        if let Some(b) = self.imp().scan_btn.borrow().as_ref() { b.set_sensitive(scan); }
    }

    fn show_toast(&self, msg: &str) {
        if let Some(o) = self.imp().toast_overlay.borrow().as_ref() {
            o.add_toast(adw::Toast::new(msg));
        }
    }

    pub fn show_advance_banner(&self) {
        if let Some(b) = self.imp().advance_banner.borrow().as_ref() {
            b.set_revealed(true);
        }
    }
}
