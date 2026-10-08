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

        // ── Final scan ────────────────────────────────────────────────────
        /// What the acquisition in flight is for; decides where its data goes.
        pub scan_kind: Cell<ScanKind>,
        /// Full-resolution data of a final scan in progress. Kept apart from
        /// `scan_buf` so the preview stays on the canvas as the editing proxy.
        pub final_buf: RefCell<Option<nv_image::FrameBuf>>,
        /// Resolution of the final scan in flight, for the roll metadata.
        pub final_dpi: Cell<u32>,
        /// True once a preview has completed, which is what enables Scan.
        pub preview_ready: Cell<bool>,
        /// Frame rectangle drawn on the preview, as fractions of the image
        /// `[x0, y0, x1, y1]`. `None` means the whole scan window.
        pub frame_rect: Cell<Option<[f64; 4]>>,
        /// Overlay that draws `frame_rect` and takes the drag gesture.
        pub frame_overlay: RefCell<Option<gtk::DrawingArea>>,
        /// Folder of the open roll on disk, if it has one.
        pub roll_root: RefCell<Option<std::path::PathBuf>>,
        /// File the canvas currently shows, so picking another frame cut from
        /// the same preview does not reload it.
        pub displayed_file: RefCell<Option<std::path::PathBuf>>,
        /// Bumped on every load request; a finished load applies only if it
        /// is still the latest.
        pub load_gen: Cell<u32>,
        /// An acquisition or import is running; the frame list is locked.
        pub busy: Cell<bool>,
        /// Set while the list is changed by code, not by the user.
        pub suppress_select: Cell<bool>,
        /// The next preview belongs to a new frame (manual-advance flow).
        pub advance_pending: Cell<bool>,
        /// The device's holder moves by hand between frames.
        pub manual_advance: Cell<bool>,
        pub autosave_id: RefCell<Option<glib::SourceId>>,
        /// Sidebar thumbnails by frame number.
        pub thumbs: RefCell<std::collections::HashMap<u32, gtk::gdk::Texture>>,

        // ── Zoom ──────────────────────────────────────────────────────────
        /// Display scale; `None` fits the image to the window.
        pub zoom: Cell<Option<f64>>,
        pub canvas_overlay: RefCell<Option<gtk::Overlay>>,
        pub canvas_scroll: RefCell<Option<gtk::ScrolledWindow>>,
        pub zoom_label: RefCell<Option<gtk::Button>>,
        pub info_label: RefCell<Option<gtk::Label>>,
        pub detect_btn: RefCell<Option<gtk::Button>>,
        pub next_frame_btn: RefCell<Option<gtk::Button>>,

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

    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ScanKind {
        #[default]
        Preview,
        Final,
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

mod frames;
mod zoom;

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
        let zoom_out = gtk::Button::builder()
            .icon_name("zoom-out-symbolic")
            .action_name("win.zoom-out")
            .tooltip_text("Zoom Out (Ctrl+−)")
            .css_classes(["flat"])
            .build();
        let zoom_label = gtk::Button::builder()
            .label("Fit")
            .action_name("win.zoom-fit")
            .tooltip_text("Fit to Window (Ctrl+0)")
            .css_classes(["flat"])
            .width_request(64)
            .build();
        let zoom_in = gtk::Button::builder()
            .icon_name("zoom-in-symbolic")
            .action_name("win.zoom-in")
            .tooltip_text("Zoom In (Ctrl++)")
            .css_classes(["flat"])
            .build();
        let zoom_100 = gtk::Button::builder()
            .label("1:1")
            .action_name("win.zoom-100")
            .tooltip_text("Actual Pixels (Ctrl+1)")
            .css_classes(["flat"])
            .build();
        let info_label = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();

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
        bottom_bar.append(&zoom_out);
        bottom_bar.append(&zoom_label);
        bottom_bar.append(&zoom_in);
        bottom_bar.append(&zoom_100);
        bottom_bar.append(&progress_bar);
        let spacer = gtk::Box::builder().hexpand(true).build();
        bottom_bar.append(&spacer);
        bottom_bar.append(&info_label);

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
        let frame_overlay = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .tooltip_text("Drag to choose the area to scan; right-click to scan the whole window")
            .build();
        frame_overlay.set_cursor_from_name(Some("crosshair"));
        let canvas_overlay = gtk::Overlay::new();
        canvas_overlay.set_child(Some(&canvas_picture));
        canvas_overlay.add_overlay(&frame_overlay);
        let canvas_scroll = gtk::ScrolledWindow::builder()
            .hexpand(true)
            .vexpand(true)
            .child(&canvas_overlay)
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
            .hexpand(true)
            .build();
        let detect_btn = gtk::Button::builder()
            .label("Detect Frames")
            .action_name("win.detect-frames")
            .tooltip_text("Find the frames on the preview (Ctrl+Shift+F)")
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .build();
        let next_frame_btn = gtk::Button::builder()
            .label("Next Frame")
            .action_name("win.advance-holder")
            .tooltip_text("Move the holder to the next frame")
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .visible(false)
            .build();

        let frame_menu = gio::Menu::new();
        frame_menu.append(Some("Duplicate Frame"), Some("win.duplicate-frame"));
        frame_menu.append(Some("Keep Frame"), Some("win.toggle-keep"));
        frame_menu.append(Some("Reject Frame"), Some("win.toggle-reject"));
        frame_menu.append(Some("Delete Frame"), Some("win.delete-frame"));
        let frame_menu_btn = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&frame_menu)
            .tooltip_text("Frame Actions")
            .build();

        let frame_buttons = gtk::Box::builder()
            .spacing(6)
            .margin_start(8)
            .margin_end(8)
            .margin_top(4)
            .margin_bottom(8)
            .build();
        frame_buttons.append(&add_frame_btn);
        frame_buttons.append(&frame_menu_btn);

        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&frame_list)
            .build();

        let sidebar_box = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .width_request(200)
            .build();
        sidebar_box.append(&sidebar_scroll);
        sidebar_box.append(&detect_btn);
        sidebar_box.append(&next_frame_btn);
        sidebar_box.append(&frame_buttons);

        // ── Outer split view (frames sidebar) ────────────────────────────
        let outer_split = adw::OverlaySplitView::new();
        outer_split.set_sidebar_position(gtk::PackType::Start);
        outer_split.set_content(Some(inner_split.upcast_ref::<gtk::Widget>()));
        outer_split.set_sidebar(Some(sidebar_box.upcast_ref::<gtk::Widget>()));
        outer_split.set_sidebar_width_fraction(0.18);
        outer_split.set_min_sidebar_width(180.0);

        self.set_content(Some(&outer_split));

        // ── Store widget handles ──────────────────────────────────────────
        *self.imp().outer_split.borrow_mut() = Some(outer_split.clone());
        *self.imp().inner_split.borrow_mut() = Some(inner_split.clone());
        *self.imp().main_stack.borrow_mut() = Some(main_stack);
        *self.imp().canvas_picture.borrow_mut() = Some(canvas_picture);
        *self.imp().canvas_overlay.borrow_mut() = Some(canvas_overlay.clone());
        *self.imp().canvas_scroll.borrow_mut() = Some(canvas_scroll.clone());
        *self.imp().zoom_label.borrow_mut() = Some(zoom_label.clone());
        *self.imp().info_label.borrow_mut() = Some(info_label.clone());
        *self.imp().detect_btn.borrow_mut() = Some(detect_btn.clone());
        *self.imp().next_frame_btn.borrow_mut() = Some(next_frame_btn.clone());
        *self.imp().frame_overlay.borrow_mut() = Some(frame_overlay.clone());
        self.setup_frame_overlay(&frame_overlay);
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
        scan_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() { win.start_final_scan(); }
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
            if let Some(win) = win_weak.upgrade() {
                win.imp().advance_pending.set(true);
                win.start_preview();
            }
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

        self.setup_frames(&frame_list);
        self.setup_zoom(&canvas_scroll);

        // Write pending changes before the window goes away.
        let win_weak = self.downgrade();
        self.connect_close_request(move |_| {
            if let Some(win) = win_weak.upgrade() { win.flush_autosave(); }
            glib::Propagation::Proceed
        });

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

    // ── Frame rectangle ───────────────────────────────────────────────────────

    /// Size of the image on the canvas, if there is one.
    fn canvas_image_size(&self) -> Option<(f64, f64)> {
        let buf = self.imp().scan_buf.borrow();
        let buf = buf.as_ref()?;
        let h = buf.effective_height();
        (buf.width > 0 && h > 0).then(|| (buf.width as f64, h as f64))
    }

    /// The frame to scan, as fractions of the full scan window.
    pub fn frame_rect(&self) -> Option<[f64; 4]> {
        self.imp().frame_rect.get()
    }

    fn set_frame_rect(&self, rect: Option<[f64; 4]>) {
        self.imp().frame_rect.set(rect);
        if let Some(a) = self.imp().frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
    }

    fn setup_frame_overlay(&self, area: &gtk::DrawingArea) {
        use crate::canvas_geom as geom;

        // Draw: dim everything outside the rectangle, outline the rectangle.
        let win_weak = self.downgrade();
        area.set_draw_func(move |_, cr, wid, hei| {
            let Some(win) = win_weak.upgrade() else { return };
            let (Some(rect), Some((iw, ih))) = (win.frame_rect(), win.canvas_image_size()) else { return };
            let (cx, cy, cw, ch) = geom::content_rect(wid as f64, hei as f64, iw, ih);
            let (x, y) = (cx + rect[0] * cw, cy + rect[1] * ch);
            let (w, h) = ((rect[2] - rect[0]) * cw, (rect[3] - rect[1]) * ch);

            cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
            cr.rectangle(cx, cy, cw, ch);
            cr.rectangle(x, y, w, h);
            cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
            let _ = cr.fill();

            cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
            cr.set_line_width(3.0);
            cr.rectangle(x, y, w, h);
            let _ = cr.stroke();
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.set_line_width(1.5);
            cr.rectangle(x, y, w, h);
            let _ = cr.stroke();
        });

        // Drag out a new rectangle.
        let drag = gtk::GestureDrag::builder().button(gdk::BUTTON_PRIMARY).build();
        let start = std::rc::Rc::new(std::cell::Cell::new(None::<(f64, f64)>));

        let win_weak = self.downgrade();
        let start_c = start.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(win) = win_weak.upgrade() else { return };
            let (Some((iw, ih)), Some(widget)) = (win.canvas_image_size(), g.widget()) else { return };
            let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            start_c.set(geom::point_to_frac(x, y, c));
        });

        let win_weak = self.downgrade();
        let start_c = start.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(win) = win_weak.upgrade() else { return };
            let (Some(a), Some((iw, ih)), Some(widget), Some((sx, sy))) =
                (start_c.get(), win.canvas_image_size(), g.widget(), g.start_point()) else { return };
            let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            if let Some(b) = geom::point_to_frac(sx + dx, sy + dy, c) {
                win.set_frame_rect(Some(geom::normalised(a, b)));
            }
        });

        let win_weak = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            let Some(win) = win_weak.upgrade() else { return };
            start.set(None);
            // A click, not a drag: leave no sliver behind.
            if let Some([x0, y0, x1, y1]) = win.frame_rect() {
                if x1 - x0 < geom::MIN_SIDE || y1 - y0 < geom::MIN_SIDE {
                    win.set_frame_rect(None);
                }
            }
            win.persist_frame_rect();
        });
        area.add_controller(drag);

        // Right-click: back to the whole window.
        let clear = gtk::GestureClick::builder().button(gdk::BUTTON_SECONDARY).build();
        let win_weak = self.downgrade();
        clear.connect_pressed(move |_, _, _, _| {
            if let Some(win) = win_weak.upgrade() {
                win.set_frame_rect(None);
                win.persist_frame_rect();
            }
        });
        area.add_controller(clear);
    }

    /// Save the rectangle on the selected frame (a preview is on screen).
    fn persist_frame_rect(&self) {
        let Some(idx) = *self.imp().selected_frame.borrow() else { return };
        if self.imp().preview_ready.get() {
            self.store_frame_crop(idx);
            self.rebuild_frame_list();
            self.spawn_thumbnails(vec![idx]);
            self.schedule_autosave();
        }
    }

    /// Record the drawn rectangle in the frame's geometry, in preview pixels.
    fn store_frame_crop(&self, idx: u32) {
        let Some((iw, ih)) = self.canvas_image_size() else { return };
        let crop = self.frame_rect().map(|r| crate::canvas_geom::rect_to_crop(r, iw as u32, ih as u32));
        if let Some(f) = self.imp().roll.borrow_mut().as_mut().and_then(|r| r.frame_mut(idx)) {
            f.geometry.crop = crop;
        }
    }

    // ── Actions ───────────────────────────────────────────────────────────────

    fn emit_probe(&self) {
        if let Some(app) = self.application() {
            app.activate_action("probe", None);
        }
    }

    fn start_preview(&self) {
        self.imp().scan_kind.set(imp::ScanKind::Preview);
        self.imp().preview_ready.set(false);
        self.set_busy(true);
        if let Some(app) = self.application() {
            app.activate_action("preview", None);
        }
    }

    /// Resolution for final scans: the roll's setting, else 1200 dpi.
    pub fn final_scan_dpi(&self) -> u32 {
        self.imp()
            .roll
            .borrow()
            .as_ref()
            .and_then(|r| r.settings.resolution_dpi)
            .unwrap_or(1200)
    }

    fn start_final_scan(&self) {
        if !self.imp().preview_ready.get() {
            return;
        }
        // Make sure there is a roll and a frame to put the result in before
        // the scanner spends minutes producing it.
        let Some(frame_idx) = self.ensure_roll_and_frame(false) else { return };
        self.store_frame_crop(frame_idx);
        self.schedule_autosave();
        self.flush_autosave();
        self.imp().final_dpi.set(self.final_scan_dpi());
        self.imp().scan_kind.set(imp::ScanKind::Final);
        self.set_busy(true);
        if let Some(app) = self.application() {
            app.activate_action("scan", None);
        }
    }

    /// Make a roll folder on disk (in the user's Pictures folder) and make it
    /// the open roll. With `keep_frames`, frames already in memory move into it.
    fn create_roll_folder(&self, prefix: &str, keep_frames: bool) -> bool {
        let parent = glib::user_special_dir(glib::UserDirectory::Pictures)
            .unwrap_or_else(glib::home_dir)
            .join("Negative Vibes");
        let stamp = glib::DateTime::now_local()
            .ok()
            .and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "roll".to_owned());
        let name = format!("{prefix} {stamp}");
        let existing = if keep_frames { self.imp().roll.borrow().clone() } else { None };
        let created = std::fs::create_dir_all(&parent)
            .map_err(nv_store::StoreError::from)
            .and_then(|_| RollStore::create(&parent, &name));
        match created {
            Ok(mut store) => {
                if let Some(old) = existing {
                    store.roll.frames = old.frames;
                    store.roll.settings = old.settings;
                }
                *self.imp().roll_root.borrow_mut() = Some(store.root().to_owned());
                *self.imp().roll.borrow_mut() = Some(store.roll);
                *self.imp().selected_frame.borrow_mut() = None;
                self.imp().thumbs.borrow_mut().clear();
                if let Some(t) = self.imp().header_title.borrow().as_ref() {
                    t.set_title(&name);
                }
                true
            }
            Err(e) => {
                self.show_toast(&format!("Could not create roll folder: {e}"));
                false
            }
        }
    }

    /// Returns the frame to work on, creating a roll and a frame when there
    /// are none. With `new_frame` a fresh frame is always added (the next
    /// frame of a manual-advance roll); otherwise the selected one is used.
    fn ensure_roll_and_frame(&self, new_frame: bool) -> Option<u32> {
        if self.imp().roll_root.borrow().is_none() && !self.create_roll_folder("Roll", true) {
            return None;
        }

        let mut roll_ref = self.imp().roll.borrow_mut();
        let roll = roll_ref.as_mut()?;
        let selected = *self.imp().selected_frame.borrow();
        let idx = match selected.filter(|i| roll.frame(*i).is_some()) {
            Some(i) if !new_frame => i,
            _ if new_frame => roll.add_frame().index,
            _ => match roll.frames.first() {
                Some(f) => f.index,
                None => roll.add_frame().index,
            },
        };
        drop(roll_ref);
        *self.imp().selected_frame.borrow_mut() = Some(idx);
        self.rebuild_frame_list();
        Some(idx)
    }
    fn add_frame(&self) {
        if self.imp().busy.get() {
            return;
        }
        let Some(idx) = self.ensure_roll_and_frame(true) else { return };
        self.select_frame(idx);
        self.schedule_autosave();
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
                    let manual = first.holder == nv_device::HolderKind::ManualAdvance;
                    self.imp().manual_advance.set(manual);
                    if let Some(b) = self.imp().detect_btn.borrow().as_ref() { b.set_visible(!manual); }
                    if let Some(b) = self.imp().next_frame_btn.borrow().as_ref() { b.set_visible(manual); }
                    if self.imp().roll.borrow().is_none() {
                        self.set_page("insert-film");
                    }
                    *self.imp().scan_state.borrow_mut() = ScanUiState::Ready {
                        device_name: first.device_name.clone(),
                    };
                    if let Some(t) = self.imp().header_title.borrow().as_ref() {
                        t.set_subtitle(&format!("{} {}", first.vendor, first.model));
                    }
                    self.update_scan_controls();
                }
            }

            ScanEvent::ScanStarted { width_px, height_px, bytes_per_line, depth, channels } => {
                let buf = FrameBuf::with_stride(width_px, height_px, depth, channels, bytes_per_line);
                if self.imp().scan_kind.get() == imp::ScanKind::Final {
                    *self.imp().final_buf.borrow_mut() = Some(buf);
                } else {
                    *self.imp().scan_buf.borrow_mut() = Some(buf);
                    self.imp().film_base_ready.set(false);
                    *self.imp().displayed_file.borrow_mut() = None;
                    self.set_page("canvas");
                }
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(true);
                    pb.set_fraction(0.0);
                }
            }

            ScanEvent::DataChunk { data, .. } => {
                if self.imp().scan_kind.get() == imp::ScanKind::Final {
                    if let Some(buf) = self.imp().final_buf.borrow_mut().as_mut() {
                        buf.append(&data);
                    }
                    return;
                }
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

            ScanEvent::ScanDone if self.imp().scan_kind.get() == imp::ScanKind::Final => {
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.finish_final_scan();
            }

            ScanEvent::ScanDone => {
                // Auto-estimate film base now that the full scan is in.
                self.auto_estimate_film_base();
                self.refresh_canvas();
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.imp().preview_ready.set(true);
                self.set_busy(false);
                self.on_scan_done();
            }

            ScanEvent::Cancelled => {
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.abort_acquisition();
            }

            ScanEvent::Error(msg) => {
                self.show_toast(&format!("Scanner error: {msg}"));
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.abort_acquisition();
            }

            ScanEvent::DeviceReady { .. }
            | ScanEvent::ColorFrameDone
            | ScanEvent::IrFrameDone => {}
        }
    }

    fn abort_acquisition(&self) {
        *self.imp().final_buf.borrow_mut() = None;
        // A failed or cancelled final scan leaves the preview on screen, so
        // Scan can be tried again; a failed preview leaves nothing to scan.
        let preview_ok = self.imp().scan_kind.get() == imp::ScanKind::Final
            && self.imp().preview_ready.get();
        self.imp().scan_kind.set(imp::ScanKind::Preview);
        self.imp().advance_pending.set(false);
        self.imp().preview_ready.set(preview_ok);
        self.set_busy(false);
    }

    /// Write the finished final scan into the roll's raw folder, off the UI
    /// thread (a 1200 dpi 16-bit frame is hundreds of megabytes).
    fn finish_final_scan(&self) {
        let Some(buf) = self.imp().final_buf.borrow_mut().take() else {
            self.abort_acquisition();
            return;
        };
        let dpi = self.imp().final_dpi.get();
        let frame_idx = *self.imp().selected_frame.borrow();
        let root = self.imp().roll_root.borrow().clone();
        let (Some(frame_idx), Some(root)) = (frame_idx, root) else {
            self.show_toast("Scan finished but there is no roll to save it in");
            self.abort_acquisition();
            return;
        };

        let store = RollStore::from_parts(root, nv_store::Roll::new(""));
        let path = store.scan_path(frame_idx);
        let (w, h, depth, channels) = (buf.width, buf.effective_height(), buf.depth, buf.channels);
        let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        self.show_toast("Saving scan…");
        let win_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let write_path = path.clone();
            let result = gio::spawn_blocking(move || nv_store::RawWriter::write_scan(&buf, &write_path)).await;
            let Some(win) = win_weak.upgrade() else { return };
            match result {
                Ok(Ok(())) => {
                    win.record_scan(frame_idx, filename, dpi, depth, channels, w, h);
                    win.show_toast(&format!("Saved frame {frame_idx} ({w}×{h}, {dpi} dpi)"));
                    if win.imp().manual_advance.get() {
                        win.show_advance_banner();
                    }
                }
                Ok(Err(e)) => win.show_toast(&format!("Could not save scan: {e}")),
                Err(_) => win.show_toast("Could not save scan: writer thread panicked"),
            }
            win.abort_acquisition();
            win.update_info_label();
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn record_scan(&self, idx: u32, filename: String, dpi: u32, depth: u8, channels: u8, w: u32, h: u32) {
        {
            let mut roll_ref = self.imp().roll.borrow_mut();
            if let Some(frame) = roll_ref.as_mut().and_then(|r| r.frames.iter_mut().find(|f| f.index == idx)) {
                frame.state = nv_store::FrameState::Scanned;
                frame.raw = Some(nv_store::RawRef {
                    filename,
                    scan: nv_store::model::ScanSettings {
                        resolution_dpi: dpi,
                        depth,
                        channels,
                        scanned_at: glib::DateTime::now_utc()
                            .ok()
                            .and_then(|d| d.format_iso8601().ok())
                            .map(|s| s.to_string())
                            .unwrap_or_default(),
                    },
                    has_ir: false,
                    width_px: w,
                    height_px: h,
                });
            }
        }
        self.rebuild_frame_list();
        self.schedule_autosave();
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
        self.apply_zoom();
    }

    // ── Roll management ───────────────────────────────────────────────────────

    pub fn new_roll(&self, name: &str, store_root: &std::path::Path) {
        match RollStore::create(store_root, name) {
            Ok(store) => {
                *self.imp().roll.borrow_mut() = Some(store.roll.clone());
                *self.imp().roll_root.borrow_mut() = Some(store.root().to_owned());
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

    /// Open `roll`, which lives in `root` on disk.
    pub fn load_roll(&self, roll: Roll, root: Option<std::path::PathBuf>) {
        self.flush_autosave();
        if let Some(t) = self.imp().header_title.borrow().as_ref() {
            t.set_title(&roll.name);
        }
        *self.imp().roll.borrow_mut() = Some(roll);
        *self.imp().roll_root.borrow_mut() = root;
        *self.imp().selected_frame.borrow_mut() = None;
        self.show_opened_roll();
    }
    /// A preview finished: it becomes the selected frame's image.
    fn on_scan_done(&self) {
        self.adopt_preview();
        self.update_info_label();
    }
    fn save_roll(&self) -> Option<()> {
        let root = self.imp().roll_root.borrow().clone()?;
        let roll = self.imp().roll.borrow().clone()?;
        if let Err(e) = RollStore::from_parts(root, roll).save() {
            self.show_toast(&format!("Could not save roll: {e}"));
            return None;
        }
        Some(())
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
        let sources: Vec<std::path::PathBuf> = (0..files.n_items())
            .filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path())
            .collect();
        self.import_raw_files(sources);
    }

    /// Headless hook for `NV_IMPORT_FILES`: import as Open Raw Scans does.
    pub fn import_raw_files_for_test(&self, sources: Vec<std::path::PathBuf>) {
        self.import_raw_files(sources);
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
            Ok(store) => {
                let root = store.root().to_owned();
                self.load_roll(store.roll, Some(root));
            }
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
