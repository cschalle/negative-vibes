const FILM_TYPE_LABELS: [&str; 4] = ["Colour Negative", "B&W Negative", "Slide (Ektachrome, other E-6)", "Kodachrome Slide"];
const FILM_TYPE_SHORT_LABELS: [&str; 4] = ["Colour Negative", "B&W Negative", "Slide", "Kodachrome"];

pub(crate) mod imp {
    use adw::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

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
        pub scan_btn: RefCell<Option<adw::SplitButton>>,
        pub progress_bar: RefCell<Option<gtk::ProgressBar>>,
        /// Small text above the progress bar saying what the scanner is doing.
        pub progress_label: RefCell<Option<gtk::Label>>,
        /// "pass 2 of 3" while a multi-pass acquisition runs; empty otherwise.
        pub pass_text: RefCell<String>,
        pub advance_banner: RefCell<Option<adw::Banner>>,

        // ── Inspector widget handles ───────────────────────────────────────
        pub image_type_row: RefCell<Option<adw::ComboRow>>,
        /// Header dropdown and empty-state toggles that mirror `image_type_row`.
        pub film_type_btn: RefCell<Option<gtk::DropDown>>,
        pub film_type_toggles: RefCell<Vec<gtk::ToggleButton>>,
        pub profile_row: RefCell<Option<adw::ComboRow>>,
        pub exposure_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_r_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_g_row: RefCell<Option<adw::SpinRow>>,
        pub film_base_b_row: RefCell<Option<adw::SpinRow>>,
        pub black_pt_row: RefCell<Option<adw::SpinRow>>,
        pub white_pt_row: RefCell<Option<adw::SpinRow>>,
        pub midtone_row: RefCell<Option<adw::SpinRow>>,
        pub contrast_row: RefCell<Option<adw::SpinRow>>,
        pub cast_row: RefCell<Option<adw::SpinRow>>,
        pub dust_mode_row: RefCell<Option<adw::ComboRow>>,
        pub dust_detection_row: RefCell<Option<adw::SpinRow>>,
        pub dust_note_row: RefCell<Option<adw::ActionRow>>,
        pub brush_row: RefCell<Option<adw::SpinRow>>,
        /// The stroke being painted, as fractions of the scan.
        pub stroke_points: RefCell<Vec<[f32; 2]>>,
        // ── 1:1 loupe ─────────────────────────────────────────────────────
        pub loupe_on: Cell<bool>,
        pub hq_on: Cell<bool>,
        pub hq_picture: RefCell<Option<gtk::Picture>>,
        pub hq_gen: Cell<u32>,
        pub hq_timer: RefCell<Option<glib::SourceId>>,
        pub loupe_frame: RefCell<Option<gtk::Frame>>,
        pub loupe_picture: RefCell<Option<gtk::Picture>>,
        /// Where the pointer last was on the picture, as fractions of it.
        pub loupe_pointer: Cell<Option<(f64, f64)>>,
        pub loupe_timer: RefCell<Option<glib::SourceId>>,
        pub loupe_gen: Cell<u32>,
        /// Parameters measured for the loupe: proxy id, the params, the frozen params.
        pub loupe_frozen: RefCell<Option<(u64, nv_process::ProcessParams, nv_process::ProcessParams)>>,
        pub dust_method_row: RefCell<Option<adw::ComboRow>>,
        pub dust_defect_row: RefCell<Option<adw::ComboRow>>,
        pub usm_rows: RefCell<Option<(adw::SwitchRow, adw::SpinRow, adw::SpinRow, adw::SpinRow)>>,
        pub noise_row: RefCell<Option<adw::ComboRow>>,
        pub local_contrast_row: RefCell<Option<adw::SpinRow>>,
        pub selective_rows: RefCell<Option<(adw::ComboRow, [adw::SpinRow; 3])>>,
        pub colour_rows: RefCell<Option<(adw::SpinRow, adw::SpinRow, adw::ComboRow)>>,
        /// Where inspector edits go: the roll's defaults or this frame.
        pub scope: Cell<Scope>,
        /// Undo history of the frames not on screen, by frame number
        /// (key 0 is an image that has no frame).
        pub histories: RefCell<std::collections::HashMap<u32, super::history::History>>,
        pub history_key: Cell<u32>,
        pub analysing: Cell<bool>,
        // ── Export ────────────────────────────────────────────────────────
        pub export_queue: std::cell::OnceCell<crate::export_queue::ExportQueue>,
        pub export_jobs: RefCell<Vec<super::export::JobInfo>>,
        pub next_export_id: Cell<u64>,
        // ── Google Photos ─────────────────────────────────────────────────
        /// The saved sign-in, once the keyring has been asked.
        pub google_token: RefCell<Option<String>>,
        pub google_token_checked: Cell<bool>,
        /// Set to stop a sign-in that is waiting for the browser.
        pub google_signin_cancel: RefCell<Option<std::sync::Arc<std::sync::atomic::AtomicBool>>>,
        pub queue_button: RefCell<Option<gtk::MenuButton>>,
        pub queue_list: RefCell<Option<gtk::ListBox>>,
        /// The scan in flight is for the raw file only: no export after it.
        pub raw_only: Cell<bool>,
        // ── Batch scanning ────────────────────────────────────────────────
        /// The frame whose final scan is in flight, and the rectangle of the
        /// scanner window it covers (`None`: all of it).
        pub scan_target: Cell<Option<u32>>,
        pub scan_area: Cell<Option<[f64; 4]>>,
        /// Frames still to scan in this batch.
        pub batch: RefCell<std::collections::VecDeque<u32>>,
        pub batch_total: Cell<u32>,
        pub batch_done: Cell<u32>,
        /// Do not export what this batch scans.
        pub batch_raw_only: Cell<bool>,
        /// Manual-advance scanner: after each frame, preview and scan the next.
        pub batch_loop: Cell<bool>,
        /// Frames the person asked to see the preview of, though they have a scan.
        pub preview_view: RefCell<std::collections::HashSet<u32>>,
        /// The preview that is coming is to be scanned at once.
        pub auto_scan_next: Cell<bool>,
        /// The scanner (not an import) is what the window is busy with.
        pub acquiring: Cell<bool>,
        pub autoscan_started: Cell<bool>,
        pub auto_previewed: Cell<bool>,
        /// When the scanner last did anything we heard of; a scan that stays silent is stuck.
        pub last_scan_event: Cell<Option<std::time::Instant>>,
        pub autostop_done: Cell<bool>,
        pub stop_at: Cell<Option<std::time::Instant>>,
        pub autoscan_done: Cell<bool>,
        /// Preferences: what a new roll starts with.
        pub prefs: RefCell<crate::prefs::Prefs>,
        /// The scan resolution when no roll sets one.
        pub scan_dpi: Cell<u32>,
        pub device_resolutions: RefCell<Vec<u32>>,
        /// The scanner's film window in mm, for the live file size.
        pub film_window_mm: Cell<Option<[f64; 2]>>,
        pub scan_res_row: RefCell<Option<adw::ComboRow>>,
        pub scan_size_row: RefCell<Option<adw::ActionRow>>,
        /// Orientation of a picture that has no frame (a loose file).
        pub loose_orientation: Cell<nv_process::Orientation>,
        pub loose_angle: Cell<f32>,
        pub angle_row: RefCell<Option<adw::SpinRow>>,
        /// Histogram of the picture entering the tone stage, for the levels.
        pub levels: RefCell<Option<[u32; 256]>>,
        pub levels_area: RefCell<Option<gtk::DrawingArea>>,
        pub curve_area: RefCell<Option<gtk::DrawingArea>>,
        /// 0 master, 1 red, 2 green, 3 blue.
        pub curve_channel: Cell<u8>,
        pub wheel_area: RefCell<Option<gtk::DrawingArea>>,
        pub wheel_region: Cell<usize>,
        pub film_profiles: RefCell<Vec<nv_film::FilmProfile>>,
        /// The Scan a Roll panel while it is open, docked in `guide_split`.
        pub guide: RefCell<Option<gtk::Widget>>,
        pub guide_split: RefCell<Option<adw::OverlaySplitView>>,
        pub scanner_profile_row: RefCell<Option<adw::ComboRow>>,
        pub navigator_area: RefCell<Option<gtk::DrawingArea>>,
        pub densi_on: Cell<bool>,
        pub densi_label: RefCell<Option<gtk::Label>>,
        /// The picture as last rendered (before orientation): width, height, RGB8.
        pub last_rgb: RefCell<(usize, usize, Vec<u8>)>,
        /// Parameters when a drag began; the edit is recorded when it ends.
        pub live_old: RefCell<Option<nv_process::ProcessParams>>,
        pub balance_rows: RefCell<Vec<adw::SpinRow>>,
        /// What a click on the canvas does.
        pub canvas_tool: Cell<CanvasTool>,
        pub tool_buttons: RefCell<Vec<(CanvasTool, gtk::ToggleButton)>>,

        // ── App state ─────────────────────────────────────────────────────
        pub roll: RefCell<Option<nv_store::Roll>>,
        pub scan_buf: RefCell<Option<nv_image::FrameBuf>>,
        /// Size of the scan on the canvas when `scan_buf` was let go: a frame
        /// of a few hundred megabytes is rendered from its proxy, and holding
        /// the whole raw beside the proxy only costs memory.
        pub scan_dims: Cell<Option<(u32, u32)>>,
        pub scan_state: RefCell<ScanUiState>,
        pub selected_frame: RefCell<Option<u32>>,

        // ── Final scan ────────────────────────────────────────────────────
        /// What the acquisition in flight is for; decides where its data goes.
        pub scan_kind: Cell<ScanKind>,
        /// Full-resolution data of a final scan in progress. Kept apart from
        /// `scan_buf` so the preview stays on the canvas as the editing proxy.
        pub final_buf: RefCell<Option<nv_image::FrameBuf>>,
        /// The infrared plane of a final scan in progress (a second frame).
        pub final_ir: RefCell<Option<nv_image::FrameBuf>>,
        /// The data now arriving is the infrared frame.
        pub receiving_ir: Cell<bool>,
        /// The scanner can capture infrared, and the switch for it.
        pub device_ir: Cell<bool>,
        /// Session default for capturing infrared when a roll has no setting.
        pub with_ir: Cell<bool>,
        pub ir_row: RefCell<Option<adw::SwitchRow>>,
        pub samples: Cell<u8>,
        pub long_exposure: Cell<Option<f32>>,
        pub device_exposure: Cell<bool>,
        pub passes_rows: RefCell<Option<(adw::SpinRow, adw::SwitchRow, adw::SpinRow)>>,
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
        /// Undo history of the frame on screen.
        pub history: RefCell<super::history::History>,
        /// Converted pictures kept between renders (tone edits skip stage 4).
        pub render_cache: std::sync::Arc<nv_process::RenderCache>,
        /// Set while syncing UI → params to suppress re-entrant param changes.
        pub updating_ui: Cell<bool>,
        /// True once film base has been estimated (ScanDone) or a raw file loaded.
        /// Gates the pipeline in refresh_canvas so widget-init signals don't arm it.
        pub film_base_ready: Cell<bool>,
        /// The scan on the canvas, decoded and reduced for fast rendering.
        pub proxy: RefCell<Option<std::sync::Arc<nv_process::Proxy>>>,
        pub render_busy: Cell<bool>,
        pub render_dirty: Cell<bool>,
        /// Histogram of the last render, drawn in the inspector.
        pub histogram: RefCell<Option<nv_process::Histogram>>,
        pub histogram_area: RefCell<Option<gtk::DrawingArea>>,
    }

    pub use super::history::Scope;

    /// The tool on the canvas: draw a frame, or pick a pixel for a pipette.
    #[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
    pub enum CanvasTool {
        #[default]
        Frame,
        FilmBase,
        Black,
        Grey,
        White,
        /// Paint where dust removal must leave the picture alone.
        Protect,
        /// Paint where it must repair.
        Heal,
        /// Take painted strokes back.
        Unmask,
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
        Ready {
            device_name: String,
        },
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

mod batch;
mod calibrate_ui;
mod chrome;
pub use chrome::{print_min_widths, print_wide_widgets};
mod colour_ui;
mod export;
mod export_dialog;
mod film_ui;
mod frames;
mod google_ui;
mod guided_ui;
mod history;
mod loupe;
mod mask;
mod metadata_ui;
mod navigator;
mod orient;
mod preferences_ui;
mod render;
mod scan_settings;
mod settings;
mod tone_ui;
mod zoom;

use adw::prelude::*;
use adw::subclass::prelude::ObjectSubclassIsExt;
use gtk::gdk;
use nv_acquire::ScanEvent;
use nv_film::builtin_profiles;
use nv_image::FrameBuf;
use nv_process::{ProcessParams, params::ToneParams};
use nv_store::{Roll, RollStore};

/// How close, in pixels, a press must be to a frame rectangle's edge to grab it.
const HANDLE_REACH: f64 = 10.0;

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl Window {
    pub fn new(app: &gtk::Application) -> Self {
        glib::Object::builder().property("application", app).build()
    }

    // ── Build ─────────────────────────────────────────────────────────────────

    fn build_ui(&self) {
        // NV_WINDOW_SIZE=WxH starts the window at that size, to check narrow layouts.
        let (dw, dh) = std::env::var("NV_WINDOW_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))).unwrap_or((1280, 800));
        self.set_default_size(dw, dh);

        // ── Header bar ────────────────────────────────────────────────────
        let header = adw::HeaderBar::new();

        let sidebar_btn = gtk::ToggleButton::builder().icon_name("sidebar-show-symbolic").tooltip_text("Show Frames (F9)").build();

        let inspector_btn = gtk::ToggleButton::builder().icon_name("view-dual-symbolic").tooltip_text("Show Inspector (Shift+F9)").build();

        let preview_btn = gtk::Button::builder().label("Preview").tooltip_text("Preview the film (Ctrl+R)").sensitive(false).build();

        // Film type is the first choice for every roll, so it sits beside Preview. It is a view of
        // the Inspector's Type row, bound to it below.
        let film_type_btn = gtk::DropDown::builder().model(&gtk::StringList::new(&FILM_TYPE_LABELS)).tooltip_text("Film type").build();

        let scan_menu = gio::Menu::new();
        scan_menu.append(Some("Scan Selected Frames"), Some("win.scan-selected"));
        scan_menu.append(Some("Scan All Frames"), Some("win.scan-all"));
        scan_menu.append(Some("Scan Raw Only"), Some("win.scan-raw-only"));
        let scan_btn = adw::SplitButton::builder()
            .label("Scan")
            .css_classes(["suggested-action"])
            .tooltip_text("Scan the current frame (Ctrl+Return)")
            .menu_model(&scan_menu)
            .action_name("win.scan-selected")
            .sensitive(false)
            .build();

        let export_btn = gtk::Button::builder().label("Export").action_name("win.export").tooltip_text("Export frames (Ctrl+E)").build();
        let queue_btn = self.build_queue_button();

        let main_menu = gio::Menu::new();
        let roll_menu = gio::Menu::new();
        roll_menu.append(Some("Scan a Roll…"), Some("win.guided"));
        roll_menu.append(Some("Open Roll…"), Some("win.open-roll"));
        roll_menu.append(Some("Export…"), Some("win.export"));
        roll_menu.append(Some("Metadata…"), Some("win.metadata"));
        main_menu.append_section(None, &roll_menu);
        let view_menu = gio::Menu::new();
        view_menu.append(Some("Show Frames"), Some("win.toggle-frames"));
        view_menu.append(Some("Show Inspector"), Some("win.toggle-inspector"));
        main_menu.append_section(None, &view_menu);
        main_menu.append(Some("Calibrate with IT8 Target…"), Some("win.calibrate"));
        let film_menu = gio::Menu::new();
        film_menu.append(Some("Create Film Profile…"), Some("win.film-create"));
        film_menu.append(Some("Import Film Profile…"), Some("win.film-import"));
        film_menu.append(Some("Export Current Film Profile…"), Some("win.film-export"));
        film_menu.append(Some("Delete Current Film Profile"), Some("win.film-delete"));
        main_menu.append_section(None, &film_menu);
        let app_menu = gio::Menu::new();
        app_menu.append(Some("Preferences"), Some("win.preferences"));
        app_menu.append(Some("Keyboard Shortcuts"), Some("win.shortcuts"));
        app_menu.append(Some("Help"), Some("win.help"));
        app_menu.append(Some("About Negative Vibes"), Some("win.about"));
        app_menu.append(Some("Quit"), Some("app.quit"));
        main_menu.append_section(None, &app_menu);
        let menu_btn = gtk::MenuButton::builder().icon_name("open-menu-symbolic").menu_model(&main_menu).tooltip_text("Main Menu").build();

        let open_btn = gtk::Button::builder().icon_name("folder-open-symbolic").tooltip_text("Open Roll… (Ctrl+O)").build();

        let title_widget = adw::WindowTitle::new("Negative Vibes", "");

        header.pack_start(&sidebar_btn);
        header.pack_start(&film_type_btn);
        *self.imp().film_type_btn.borrow_mut() = Some(film_type_btn.clone());
        header.pack_start(&preview_btn);
        header.set_title_widget(Some(&title_widget));
        header.pack_end(&menu_btn);
        header.pack_end(&scan_btn);
        header.pack_end(&export_btn);
        header.pack_end(&queue_btn);
        header.pack_end(&open_btn);
        header.pack_end(&inspector_btn);

        // ── Bottom bar ────────────────────────────────────────────────────
        let zoom_out = gtk::Button::builder().icon_name("zoom-out-symbolic").action_name("win.zoom-out").tooltip_text("Zoom Out (Ctrl+−)").css_classes(["flat"]).build();
        let zoom_label = gtk::Button::builder().label("Fit").action_name("win.zoom-fit").tooltip_text("Fit to Window (Ctrl+0)").css_classes(["flat"]).width_request(64).build();
        let zoom_in = gtk::Button::builder().icon_name("zoom-in-symbolic").action_name("win.zoom-in").tooltip_text("Zoom In (Ctrl++)").css_classes(["flat"]).build();
        let zoom_100 = gtk::Button::builder().label("1:1").action_name("win.zoom-100").tooltip_text("Actual Pixels (Ctrl+1)").css_classes(["flat"]).build();
        let info_label = gtk::Label::builder().css_classes(["dim-label", "numeric"]).build();

        let progress_bar = gtk::ProgressBar::builder().hexpand(true).valign(gtk::Align::Center).visible(false).build();
        let progress_label = gtk::Label::builder().xalign(0.0).ellipsize(gtk::pango::EllipsizeMode::End).css_classes(["caption", "dim-label"]).visible(false).build();
        progress_bar.bind_property("visible", &progress_label, "visible").sync_create().build();
        let progress_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(2).hexpand(true).valign(gtk::Align::Center).build();
        progress_box.append(&progress_label);
        progress_box.append(&progress_bar);

        let bottom_bar = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(6).margin_start(6).margin_end(6).margin_top(3).margin_bottom(3).build();
        bottom_bar.append(&zoom_out);
        bottom_bar.append(&zoom_label);
        bottom_bar.append(&zoom_in);
        bottom_bar.append(&zoom_100);
        let turn_box = gtk::Box::builder().css_classes(["linked"]).margin_start(12).build();
        for (icon, action, tip) in [
            ("object-rotate-left-symbolic", "win.rotate-left", "Rotate Left (Ctrl+←)"),
            ("object-rotate-right-symbolic", "win.rotate-right", "Rotate Right (Ctrl+→)"),
            ("object-flip-horizontal-symbolic", "win.flip-horizontal", "Flip Horizontally"),
            ("object-flip-vertical-symbolic", "win.flip-vertical", "Flip Vertically"),
        ] {
            turn_box.append(&gtk::Button::builder().icon_name(icon).action_name(action).tooltip_text(tip).build());
        }
        bottom_bar.append(&turn_box);
        let loupe_btn = gtk::ToggleButton::builder().label("1:1").tooltip_text("Loupe: the picture at the scan's resolution under the pointer").margin_start(12).build();
        let weak = self.downgrade();
        loupe_btn.connect_toggled(move |b| {
            if let Some(win) = weak.upgrade() {
                win.set_loupe(b.is_active());
            }
        });
        bottom_bar.append(&loupe_btn);
        let densi_btn = gtk::ToggleButton::builder().label("RGB").tooltip_text("Densitometer: the colour under the pointer, before and after correction").margin_start(6).build();
        let densi_label = gtk::Label::builder().margin_start(8).css_classes(["monospace", "dim-label"]).visible(false).build();
        let weak = self.downgrade();
        densi_btn.connect_toggled(move |b| {
            if let Some(win) = weak.upgrade() {
                win.imp().densi_on.set(b.is_active());
                if let Some(l) = win.imp().densi_label.borrow().as_ref() {
                    l.set_visible(b.is_active());
                }
            }
        });
        bottom_bar.append(&densi_btn);
        bottom_bar.append(&self.build_navigator());
        bottom_bar.append(&densi_label);
        *self.imp().densi_label.borrow_mut() = Some(densi_label);
        let hq_btn = gtk::ToggleButton::builder().label("HQ").tooltip_text("Full-quality view of what is on screen, at 1:1 or more").margin_start(6).build();
        let weak = self.downgrade();
        hq_btn.connect_toggled(move |b| {
            if let Some(win) = weak.upgrade() {
                win.set_hq(b.is_active());
            }
        });
        bottom_bar.append(&hq_btn);
        let tool_box = gtk::Box::builder().css_classes(["linked"]).margin_start(12).build();
        let mut tool_buttons: Vec<(imp::CanvasTool, gtk::ToggleButton)> = Vec::new();
        for (tool, label, tip) in [
            (imp::CanvasTool::Frame, "Frame", "Draw the area to scan; drag its edges to resize, Shift keeps the shape"),
            (imp::CanvasTool::FilmBase, "Base", "Pick the film base: click unexposed film (the rebate)"),
            (imp::CanvasTool::Black, "Black", "Pick black: click the darkest part that should be black"),
            (imp::CanvasTool::Grey, "Grey", "Pick grey: click something that should be neutral"),
            (imp::CanvasTool::White, "White", "Pick white: click the brightest part that should be white"),
            (imp::CanvasTool::Protect, "Protect", "Paint where dust removal must leave the picture alone"),
            (imp::CanvasTool::Heal, "Heal", "Paint where dust removal must repair"),
            (imp::CanvasTool::Unmask, "Unmask", "Paint over a mask stroke to remove it"),
        ] {
            let b = gtk::ToggleButton::builder().label(label).tooltip_text(tip).active(tool == imp::CanvasTool::Frame).build();
            if let Some((_, first)) = tool_buttons.first() {
                b.set_group(Some(first));
            }
            tool_box.append(&b);
            tool_buttons.push((tool, b));
        }
        bottom_bar.append(&tool_box);
        // Progress and the picture's details get a row of their own above the controls:
        // the controls scroll when the window is narrow, and these must not scroll away.
        info_label.set_visible(false);
        info_label.connect_label_notify(|l| l.set_visible(!l.label().is_empty()));

        // ── Canvas stack ──────────────────────────────────────────────────
        let no_scanner = adw::StatusPage::new();
        no_scanner.set_icon_name(Some("scanner-symbolic"));
        no_scanner.set_title("No Scanner Found");
        no_scanner.set_description(Some("Make sure your scanner is connected and powered on."));
        let try_again = gtk::Button::builder().label("Try Again").css_classes(["pill", "suggested-action"]).build();
        let open_raw = gtk::Button::builder().label("Open Raw Scans…").css_classes(["pill"]).build();
        let open_roll = gtk::Button::builder().label("Open Roll…").css_classes(["pill"]).build();
        let btn_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(8).halign(gtk::Align::Center).build();
        btn_box.append(&try_again);
        btn_box.append(&open_raw);
        btn_box.append(&open_roll);
        no_scanner.set_child(Some(&btn_box));

        let insert_page = adw::StatusPage::new();
        insert_page.set_icon_name(Some("scanner-symbolic"));
        insert_page.set_title("Insert Film and Preview");
        insert_page.set_description(Some("Insert film and click Preview to begin."));
        let preview_big = gtk::Button::builder().label("Preview").css_classes(["pill", "suggested-action"]).halign(gtk::Align::Center).build();
        // The film type toggle sits above the big Preview button (DESIGN.md §8.3).
        let type_toggles = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).css_classes(["linked"]).halign(gtk::Align::Center).build();
        let toggle_buttons: Vec<gtk::ToggleButton> = FILM_TYPE_SHORT_LABELS.iter().map(|l| gtk::ToggleButton::with_label(l)).collect();
        for b in &toggle_buttons {
            if let Some(first) = toggle_buttons.first().filter(|f| *f != b) {
                b.set_group(Some(first));
            }
            type_toggles.append(b);
        }
        let insert_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).spacing(18).halign(gtk::Align::Center).build();
        *self.imp().film_type_toggles.borrow_mut() = toggle_buttons.clone();
        insert_box.append(&type_toggles);
        insert_box.append(&preview_big);
        insert_page.set_child(Some(&insert_box));

        let canvas_picture = gtk::Picture::builder().hexpand(true).vexpand(true).content_fit(gtk::ContentFit::Contain).build();
        let frame_overlay = gtk::DrawingArea::builder().hexpand(true).vexpand(true).tooltip_text("Drag to choose the area to scan; right-click to scan the whole window").build();
        frame_overlay.set_cursor_from_name(Some("crosshair"));
        let canvas_overlay = gtk::Overlay::new();
        canvas_overlay.set_child(Some(&canvas_picture));
        // The full-quality view goes under the frame and the strokes.
        let hq = self.build_hq();
        canvas_overlay.add_overlay(&hq);
        canvas_overlay.add_overlay(&frame_overlay);
        let canvas_scroll = gtk::ScrolledWindow::builder().hexpand(true).vexpand(true).child(&canvas_overlay).build();

        let advance_banner = adw::Banner::new("Move the holder to the next frame");
        advance_banner.set_button_label(Some("Scan Next Frame"));
        advance_banner.set_revealed(false);

        let canvas_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).build();
        canvas_box.append(&advance_banner);
        let loupe = self.build_loupe();
        let viewport_overlay = gtk::Overlay::new();
        viewport_overlay.set_child(Some(&canvas_scroll));
        viewport_overlay.add_overlay(&loupe);
        viewport_overlay.set_vexpand(true);
        *self.imp().loupe_frame.borrow_mut() = Some(loupe);
        canvas_box.append(&viewport_overlay);

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
        // The Scan a Roll guide docks beside the picture, so it never needs a window of its own
        // (the compositor, not the app, decides where a Wayland window goes).
        let guide_split = adw::OverlaySplitView::new();
        guide_split.set_sidebar_position(gtk::PackType::End);
        guide_split.set_content(Some(&main_stack));
        guide_split.set_show_sidebar(false);
        guide_split.set_sidebar_width_fraction(0.34);
        guide_split.set_min_sidebar_width(340.0);
        guide_split.set_max_sidebar_width(460.0);
        *self.imp().guide_split.borrow_mut() = Some(guide_split.clone());
        toolbar_view.set_content(Some(&guide_split));
        // The bar has grown wider than a narrow window; letting it scroll keeps its minimum
        // width small, so the side panels are not squeezed to nothing.
        let bottom_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&bottom_bar)
            .build();
        let status_row = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(12).margin_start(12).margin_end(12).margin_top(3).build();
        status_row.append(&progress_box);
        status_row.append(&info_label);
        let bottom_stack = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_stack.append(&status_row);
        bottom_stack.append(&bottom_scroll);
        toolbar_view.add_bottom_bar(&bottom_stack);

        let toast_overlay = adw::ToastOverlay::new();
        toast_overlay.set_child(Some(&toolbar_view));

        // ── Inner split view (inspector) ──────────────────────────────────
        let inner_split = adw::OverlaySplitView::new();
        inner_split.set_sidebar_position(gtk::PackType::End);
        inner_split.set_content(Some(toast_overlay.upcast_ref::<gtk::Widget>()));
        inner_split.set_sidebar(Some(inspector_box.upcast_ref::<gtk::Widget>()));
        inner_split.set_show_sidebar(false);
        inner_split.set_sidebar_width_fraction(0.28);
        inner_split.set_min_sidebar_width(240.0);

        // ── Frames sidebar ────────────────────────────────────────────────
        let frame_list = gtk::ListBox::new();
        frame_list.set_selection_mode(gtk::SelectionMode::Single);
        frame_list.add_css_class("navigation-sidebar");
        frame_list.set_vexpand(true);

        let add_frame_btn = gtk::Button::builder().label("Add Frame").hexpand(true).build();
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
        frame_menu.append(Some("Show Preview or Scan"), Some("win.toggle-view"));
        frame_menu.append(Some("Duplicate Frame"), Some("win.duplicate-frame"));
        frame_menu.append(Some("Keep Frame"), Some("win.toggle-keep"));
        frame_menu.append(Some("Reject Frame"), Some("win.toggle-reject"));
        frame_menu.append(Some("Delete Frame"), Some("win.delete-frame"));
        let frame_menu_btn = gtk::MenuButton::builder().icon_name("view-more-symbolic").menu_model(&frame_menu).tooltip_text("Frame Actions").build();

        let frame_buttons = gtk::Box::builder().spacing(6).margin_start(8).margin_end(8).margin_top(4).margin_bottom(8).build();
        frame_buttons.append(&add_frame_btn);
        frame_buttons.append(&frame_menu_btn);

        let sidebar_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&frame_list).build();

        let sidebar_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(200).build();
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

        // Narrow windows (a phone, a small tile): the side panels slide over the picture
        // instead of taking room from it (HIG: adaptive down to 360 px).
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 860sp").expect("a valid condition"));
        narrow.add_setter(&outer_split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&inner_split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&guide_split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&outer_split, "show-sidebar", Some(&false.to_value()));
        // The header bar keeps what is needed to preview and scan; the rest is in the main menu.
        for w in [title_widget.upcast_ref::<gtk::Widget>(), export_btn.upcast_ref(), open_btn.upcast_ref(), inspector_btn.upcast_ref()] {
            narrow.add_setter(w, "visible", Some(&false.to_value()));
        }
        self.add_breakpoint(narrow);

        // ── Store widget handles ──────────────────────────────────────────
        *self.imp().outer_split.borrow_mut() = Some(outer_split.clone());
        *self.imp().inner_split.borrow_mut() = Some(inner_split.clone());
        *self.imp().main_stack.borrow_mut() = Some(main_stack);
        *self.imp().canvas_picture.borrow_mut() = Some(canvas_picture);
        for (tool, b) in &tool_buttons {
            let (tool, weak) = (*tool, self.downgrade());
            b.connect_toggled(move |b| {
                if let (true, Some(win)) = (b.is_active(), weak.upgrade()) {
                    win.imp().canvas_tool.set(tool);
                }
            });
        }
        *self.imp().tool_buttons.borrow_mut() = tool_buttons;
        *self.imp().canvas_overlay.borrow_mut() = Some(canvas_overlay.clone());
        *self.imp().canvas_scroll.borrow_mut() = Some(canvas_scroll.clone());
        *self.imp().zoom_label.borrow_mut() = Some(zoom_label.clone());
        *self.imp().info_label.borrow_mut() = Some(info_label.clone());
        *self.imp().detect_btn.borrow_mut() = Some(detect_btn.clone());
        *self.imp().next_frame_btn.borrow_mut() = Some(next_frame_btn.clone());
        *self.imp().frame_overlay.borrow_mut() = Some(frame_overlay.clone());
        self.setup_frame_overlay(&frame_overlay);
        self.setup_loupe_pointer(&frame_overlay);
        *self.imp().frame_list.borrow_mut() = Some(frame_list.clone());
        *self.imp().toast_overlay.borrow_mut() = Some(toast_overlay);
        *self.imp().header_title.borrow_mut() = Some(title_widget);
        *self.imp().preview_btn.borrow_mut() = Some(preview_btn.clone());
        *self.imp().scan_btn.borrow_mut() = Some(scan_btn.clone());
        *self.imp().progress_bar.borrow_mut() = Some(progress_bar);
        *self.imp().progress_label.borrow_mut() = Some(progress_label);
        *self.imp().advance_banner.borrow_mut() = Some(advance_banner.clone());

        // Initialise default processing params (generic colour negative).
        let profiles = builtin_profiles();
        let default_params = nv_process::default_params_for(&profiles[0]);
        let _ = &profiles;
        *self.imp().history.borrow_mut() = history::History::new(default_params.clone(), history::Scope::Roll);
        *self.imp().proc_params.borrow_mut() = Some(default_params);

        // ── Connect signals ───────────────────────────────────────────────
        // Both ways: a sidebar that slides over the picture closes itself when it is clicked away.
        sidebar_btn.bind_property("active", &outer_split, "show-sidebar").sync_create().bidirectional().build();

        inspector_btn.bind_property("active", &inner_split, "show-sidebar").sync_create().bidirectional().build();
        self.setup_chrome(&sidebar_btn, &inspector_btn);

        let win_weak = self.downgrade();
        try_again.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.emit_probe();
            }
        });

        let win_weak = self.downgrade();
        preview_big.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.start_preview();
            }
        });

        let win_weak = self.downgrade();
        preview_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                // While the scanner is busy this button is Stop.
                if win.imp().busy.get() {
                    win.stop();
                } else {
                    win.start_preview();
                }
            }
        });

        let win_weak = self.downgrade();
        open_raw.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.open_raw_scans();
            }
        });

        let win_weak = self.downgrade();
        open_roll.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.open_roll_dialog();
            }
        });

        let win_weak = self.downgrade();
        open_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.open_roll_dialog();
            }
        });

        let win_weak = self.downgrade();
        add_frame_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.add_frame();
            }
        });

        let win_weak = self.downgrade();
        advance_banner.connect_button_clicked(move |banner| {
            banner.set_revealed(false);
            if let Some(win) = win_weak.upgrade() {
                win.imp().advance_pending.set(true);
                // In a Scan All loop the next frame is scanned as soon as it is previewed.
                if win.imp().batch_loop.get() {
                    win.imp().auto_scan_next.set(true);
                }
                win.start_preview();
            }
        });

        // ── Undo/redo GActions ────────────────────────────────────────────
        let undo_action = gio::SimpleAction::new("undo", None);
        let win_weak = self.downgrade();
        undo_action.connect_activate(move |_, _| {
            if let Some(win) = win_weak.upgrade() {
                win.do_undo();
            }
        });
        self.add_action(&undo_action);

        let redo_action = gio::SimpleAction::new("redo", None);
        let win_weak = self.downgrade();
        redo_action.connect_activate(move |_, _| {
            if let Some(win) = win_weak.upgrade() {
                win.do_redo();
            }
        });
        self.add_action(&redo_action);

        *self.imp().queue_button.borrow_mut() = Some(queue_btn.clone());
        self.setup_export();
        self.setup_batch();
        self.setup_orientation();
        self.setup_preferences();
        self.setup_frames(&frame_list);
        self.setup_zoom(&canvas_scroll);

        // Write pending changes before the window goes away.
        let win_weak = self.downgrade();
        self.connect_close_request(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.flush_autosave();
            }
            glib::Propagation::Proceed
        });

        // Keyboard shortcuts are set in application.rs after the window is added to the app.
    }

    // ── Inspector panel builder ───────────────────────────────────────────────

    fn build_inspector(&self) -> gtk::Box {
        let inspector_box = gtk::Box::builder().orientation(gtk::Orientation::Vertical).width_request(240).build();

        // Title bar
        let title_label = gtk::Label::builder().label("Inspector").css_classes(["title-4"]).margin_top(12).margin_bottom(4).margin_start(12).halign(gtk::Align::Start).build();
        inspector_box.append(&title_label);

        // Page stack
        let insp_stack = gtk::Stack::builder().vexpand(true).build();
        insp_stack.set_transition_type(gtk::StackTransitionType::SlideLeftRight);

        let switcher = gtk::StackSwitcher::builder().stack(&insp_stack).margin_start(8).margin_end(8).margin_bottom(4).build();

        // ── Conversion page ───────────────────────────────────────────────
        let conv_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let conv_page = adw::PreferencesPage::new();

        // Image group
        let image_group = adw::PreferencesGroup::builder().title("Image").build();

        let image_type_row = adw::ComboRow::builder().title("Type").build();
        let type_model = gtk::StringList::new(&FILM_TYPE_LABELS);
        image_type_row.set_model(Some(&type_model));
        image_group.add(&image_type_row);
        let angle_row = adw::SpinRow::builder()
            .title("Straighten")
            .subtitle("Degrees clockwise; the picture is enlarged a little to fill the frame")
            .digits(1)
            .adjustment(&gtk::Adjustment::new(0.0, -(nv_process::straighten::MAX_ANGLE as f64), nv_process::straighten::MAX_ANGLE as f64, 0.1, 1.0, 0.0))
            .build();
        let weak = self.downgrade();
        angle_row.connect_value_notify(move |r| {
            if let Some(win) = weak.upgrade() {
                if !win.imp().updating_ui.get() {
                    win.set_angle(r.value() as f32);
                }
            }
        });
        image_group.add(&angle_row);
        *self.imp().angle_row.borrow_mut() = Some(angle_row);

        let profiles = nv_film::all_profiles();
        *self.imp().film_profiles.borrow_mut() = profiles.clone();
        let profile_names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        let profile_row = adw::ComboRow::builder().title("Film Profile").build();
        let profile_model = gtk::StringList::new(&profile_names);
        profile_row.set_model(Some(&profile_model));
        image_group.add(&profile_row);
        conv_page.add(&image_group);

        // Exposure group
        let exp_group = adw::PreferencesGroup::builder().title("Exposure").build();

        let exposure_adj = gtk::Adjustment::new(0.0, -3.0, 3.0, 0.1, 0.5, 0.0);
        let exposure_row = adw::SpinRow::builder().title("Exposure").subtitle("stops").adjustment(&exposure_adj).digits(1).build();
        exp_group.add(&exposure_row);

        // Film base sub-group
        let base_group = adw::PreferencesGroup::builder().title("Film Base").description("Auto-estimated from rebate borders").build();

        let fb_r_adj = gtk::Adjustment::new(0.6, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_r_row = adw::SpinRow::builder().title("Red").adjustment(&fb_r_adj).digits(3).build();
        base_group.add(&film_base_r_row);

        let fb_g_adj = gtk::Adjustment::new(0.3, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_g_row = adw::SpinRow::builder().title("Green").adjustment(&fb_g_adj).digits(3).build();
        base_group.add(&film_base_g_row);

        let fb_b_adj = gtk::Adjustment::new(0.18, 0.0, 1.0, 0.01, 0.1, 0.0);
        let film_base_b_row = adw::SpinRow::builder().title("Blue").adjustment(&fb_b_adj).digits(3).build();
        base_group.add(&film_base_b_row);

        let colour_group = adw::PreferencesGroup::builder().title("Colour").build();
        let cast_adj = gtk::Adjustment::new(1.0, 0.0, 1.0, 0.05, 0.25, 0.0);
        let cast_row =
            adw::SpinRow::builder().title("Cast Removal").subtitle("Line red and blue up with green along the picture's neutral axis").adjustment(&cast_adj).digits(2).build();
        colour_group.add(&cast_row);

        let roll_group = adw::PreferencesGroup::builder()
            .title("Roll")
            .description("Fix the film base, density range and colour axis from the whole roll, so every frame is converted alike")
            .build();
        let analyse_row = adw::ActionRow::builder().title("Roll Analysis").build();
        let analyse_btn = gtk::Button::builder().label("Analyse").valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
        let analyse_reset = gtk::Button::builder().label("Reset").valign(gtk::Align::Center).build();
        analyse_row.add_suffix(&analyse_reset);
        analyse_row.add_suffix(&analyse_btn);
        roll_group.add(&analyse_row);
        let weak = self.downgrade();
        analyse_btn.connect_clicked(move |_| {
            if let Some(win) = weak.upgrade() {
                win.analyse_roll();
            }
        });
        let weak = self.downgrade();
        analyse_reset.connect_clicked(move |_| {
            if let Some(win) = weak.upgrade() {
                win.set_roll_analysis(None);
            }
        });

        conv_page.add(&exp_group);
        conv_page.add(&colour_group);
        conv_page.add(&roll_group);
        conv_page.add(&base_group);
        conv_scroll.set_child(Some(&conv_page));
        insp_stack.add_titled(&conv_scroll, Some("conversion"), "Conversion");

        // ── Tone page ─────────────────────────────────────────────────────
        let tone_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).build();
        let tone_page = adw::PreferencesPage::new();
        let tone_group = adw::PreferencesGroup::builder().title("Tone").build();

        let bp_adj = gtk::Adjustment::new(0.0, 0.0, 1.0, 0.01, 0.1, 0.0);
        let black_pt_row = adw::SpinRow::builder().title("Black Point").adjustment(&bp_adj).digits(2).build();
        tone_group.add(&black_pt_row);

        let wp_adj = gtk::Adjustment::new(1.0, 0.0, 1.0, 0.01, 0.1, 0.0);
        let white_pt_row = adw::SpinRow::builder().title("White Point").adjustment(&wp_adj).digits(2).build();
        tone_group.add(&white_pt_row);

        let mt_adj = gtk::Adjustment::new(1.0, 0.25, 4.0, 0.05, 0.25, 0.0);
        let midtone_row = adw::SpinRow::builder().title("Midtone").subtitle("gamma").adjustment(&mt_adj).digits(2).build();
        tone_group.add(&midtone_row);

        let ct_adj = gtk::Adjustment::new(0.0, -1.0, 1.0, 0.05, 0.25, 0.0);
        let contrast_row = adw::SpinRow::builder().title("Contrast").adjustment(&ct_adj).digits(2).build();
        tone_group.add(&contrast_row);

        let auto_row = adw::ActionRow::builder().title("Auto Levels").subtitle("Set black, white and midtone from the picture").build();
        let auto_btn = gtk::Button::builder().label("Auto").valign(gtk::Align::Center).css_classes(["suggested-action"]).build();
        let reset_btn = gtk::Button::builder().label("Reset").valign(gtk::Align::Center).build();
        auto_row.add_suffix(&reset_btn);
        auto_row.add_suffix(&auto_btn);
        tone_group.add(&auto_row);
        let win_weak = self.downgrade();
        auto_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.auto_levels();
            }
        });
        let win_weak = self.downgrade();
        reset_btn.connect_clicked(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.reset_tone();
            }
        });

        let balance_group = adw::PreferencesGroup::builder().title("Colour Balance").description("Channel gains; the Grey pick in the bottom bar sets them").build();
        let mut balance_rows = Vec::new();
        for name in ["Red", "Green", "Blue"] {
            let adj = gtk::Adjustment::new(1.0, 0.25, 4.0, 0.01, 0.1, 0.0);
            let row = adw::SpinRow::builder().title(name).adjustment(&adj).digits(3).build();
            balance_group.add(&row);
            balance_rows.push(row);
        }

        let levels_group = adw::PreferencesGroup::builder().title("Levels").description("Drag the black and white points").build();
        let levels_area = self.build_levels();
        *self.imp().levels_area.borrow_mut() = Some(levels_area.clone());
        levels_group.add(&levels_area);

        let curves_group = adw::PreferencesGroup::builder().title("Curves").build();
        curves_group.add(&self.build_curves());

        tone_page.add(&levels_group);
        tone_page.add(&tone_group);
        tone_page.add(&balance_group);
        tone_page.add(&curves_group);
        tone_scroll.set_child(Some(&tone_page));
        insp_stack.add_titled(&tone_scroll, Some("tone"), "Tone");

        let scope_box = gtk::Box::builder().css_classes(["linked"]).margin_start(12).margin_end(12).margin_bottom(6).homogeneous(true).build();
        let roll_btn = gtk::ToggleButton::builder().label("Whole Roll").tooltip_text("Edits become the roll's defaults").active(true).build();
        let frame_btn = gtk::ToggleButton::builder().label("This Frame").tooltip_text("Edits apply to this frame only").group(&roll_btn).build();
        scope_box.append(&roll_btn);
        scope_box.append(&frame_btn);
        chrome::let_buttons_shrink(&scope_box);
        let weak = self.downgrade();
        roll_btn.connect_toggled(move |b| {
            if let (true, Some(win)) = (b.is_active(), weak.upgrade()) {
                win.set_scope(imp::Scope::Roll);
            }
        });
        let weak = self.downgrade();
        frame_btn.connect_toggled(move |b| {
            if let (true, Some(win)) = (b.is_active(), weak.upgrade()) {
                win.set_scope(imp::Scope::Frame);
            }
        });
        inspector_box.append(&scope_box);

        let histogram_area = self.build_histogram();
        *self.imp().histogram_area.borrow_mut() = Some(histogram_area.clone());
        inspector_box.append(&histogram_area);
        // ── Detail page: dust and scratches ───────────────────────────────
        let detail_page = adw::PreferencesPage::new();
        let dust_group =
            adw::PreferencesGroup::builder().title("Dust and Scratches").description("Found in the infrared plane, which dust blocks and the picture does not").build();
        let dust_mode_row = adw::ComboRow::builder().title("Mode").model(&gtk::StringList::new(&["Off", "Correct", "Mark"])).selected(1).build();
        let dust_detection_row =
            adw::SpinRow::builder().title("Detection").subtitle("Higher finds smaller and fainter marks").adjustment(&gtk::Adjustment::new(12.0, 1.0, 20.0, 1.0, 4.0, 0.0)).build();
        let dust_note_row = adw::ActionRow::builder().title("Infrared").subtitle("This scan has no infrared plane").build();
        dust_note_row.add_css_class("property");
        let dust_method_row = adw::ComboRow::builder()
            .title("Method")
            .subtitle("Infrared needs the infrared plane; software looks for specks in the picture itself")
            .model(&gtk::StringList::new(&["Infrared", "Software"]))
            .build();
        let dust_defect_row = adw::ComboRow::builder()
            .title("Defect Type")
            .subtitle("Dust on the film is dark in the scan")
            .model(&gtk::StringList::new(&["Dark", "Bright", "Both"]))
            .visible(false)
            .build();
        dust_group.add(&dust_mode_row);
        dust_group.add(&dust_method_row);
        dust_group.add(&dust_defect_row);
        dust_group.add(&dust_detection_row);
        dust_group.add(&dust_note_row);
        detail_page.add(&dust_group);

        let mask_group = adw::PreferencesGroup::builder()
            .title("Masks")
            .description("Use the Protect, Heal and Unmask tools in the bottom bar. Protect keeps dust removal off fine detail; Heal repairs where you paint.")
            .build();
        let brush_row = adw::SpinRow::builder()
            .title("Brush Size")
            .subtitle("Percent of the picture's longer side")
            .adjustment(&gtk::Adjustment::new(0.8, 0.1, 6.0, 0.1, 0.5, 0.0))
            .digits(1)
            .build();
        let clear_row = adw::ActionRow::builder().title("Clear Masks").subtitle("Remove every stroke from this frame").build();
        let clear_btn = gtk::Button::builder().label("Clear").valign(gtk::Align::Center).build();
        clear_row.add_suffix(&clear_btn);
        let weak = self.downgrade();
        clear_btn.connect_clicked(move |_| {
            if let Some(win) = weak.upgrade() {
                win.clear_masks();
            }
        });
        mask_group.add(&brush_row);
        mask_group.add(&clear_row);
        detail_page.add(&mask_group);
        *self.imp().brush_row.borrow_mut() = Some(brush_row);

        let sharp_group = adw::PreferencesGroup::builder().title("Sharpening").description("Unsharp mask, on brightness only").build();
        let usm_on = adw::SwitchRow::builder().title("Unsharp Mask").build();
        let usm_amount = adw::SpinRow::builder().title("Amount").subtitle("Percent").adjustment(&gtk::Adjustment::new(150.0, 0.0, 500.0, 10.0, 50.0, 0.0)).build();
        let usm_radius =
            adw::SpinRow::builder().title("Radius").subtitle("Pixels of the finished picture").adjustment(&gtk::Adjustment::new(1.4, 0.3, 10.0, 0.1, 0.5, 0.0)).digits(1).build();
        let usm_threshold = adw::SpinRow::builder()
            .title("Threshold")
            .subtitle("Leave differences smaller than this alone")
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 50.0, 1.0, 5.0, 0.0))
            .build();
        for r in [usm_on.upcast_ref::<gtk::Widget>(), usm_amount.upcast_ref(), usm_radius.upcast_ref(), usm_threshold.upcast_ref()] {
            sharp_group.add(r);
        }
        detail_page.add(&sharp_group);

        let noise_group = adw::PreferencesGroup::builder().title("Grain, Noise and Local Contrast").build();
        let noise_row = adw::ComboRow::builder()
            .title("Reduction")
            .subtitle("Shown at 1:1 and applied in exports; too slow for the whole-frame preview")
            .model(&gtk::StringList::new(&["Off", "Light", "Medium", "Strong"]))
            .build();
        noise_group.add(&noise_row);
        let local_contrast_row = adw::SpinRow::builder()
            .title("Local Contrast")
            .subtitle("Percent. Brings out detail in flat regions, shadows and highlights")
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 100.0, 5.0, 20.0, 0.0))
            .build();
        noise_group.add(&local_contrast_row);
        *self.imp().local_contrast_row.borrow_mut() = Some(local_contrast_row.clone());
        let weak = self.downgrade();
        local_contrast_row.connect_value_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        detail_page.add(&noise_group);

        let colour_group = adw::PreferencesGroup::builder().title("Colour").build();
        let saturation_row = adw::SpinRow::builder()
            .title("Saturation")
            .subtitle("-100 is grey, +100 twice the colour")
            .adjustment(&gtk::Adjustment::new(0.0, -100.0, 100.0, 5.0, 20.0, 0.0))
            .build();
        let acr_row = adw::SpinRow::builder()
            .title("Restore Faded Colour")
            .subtitle("Percent. Revives dull colour, leaves vivid colour alone")
            .adjustment(&gtk::Adjustment::new(0.0, 0.0, 100.0, 5.0, 20.0, 0.0))
            .build();
        let mut names: Vec<&str> = vec!["In Colour"];
        names.extend(nv_process::GREY_PRESETS.iter().map(|(n, _)| *n));
        let grey_row =
            adw::ComboRow::builder().title("Convert to Grey").subtitle("With the effect of a coloured filter on black and white film").model(&gtk::StringList::new(&names)).build();
        let scanner_row = adw::ComboRow::builder().title("Scanner Profile").subtitle("From an IT8 target, applied to slides").model(&gtk::StringList::new(&["None"])).build();
        for r in [saturation_row.upcast_ref::<gtk::Widget>(), acr_row.upcast_ref(), grey_row.upcast_ref(), scanner_row.upcast_ref()] {
            colour_group.add(r);
        }
        let weak = self.downgrade();
        scanner_row.connect_selected_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        *self.imp().scanner_profile_row.borrow_mut() = Some(scanner_row);
        for r in [&saturation_row, &acr_row] {
            let weak = self.downgrade();
            r.connect_value_notify(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.on_params_changed();
                }
            });
        }
        let weak = self.downgrade();
        grey_row.connect_selected_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        *self.imp().colour_rows.borrow_mut() = Some((saturation_row, acr_row, grey_row));
        self.refresh_profile_list();
        detail_page.add(&colour_group);
        let sel_group = adw::PreferencesGroup::builder().title("Selective Colour").description("Change one colour range without touching the others").build();
        let names: Vec<&str> = nv_process::colour::RANGES.iter().map(|r| r.0).collect();
        let range_row = adw::ComboRow::builder().title("Colour Range").model(&gtk::StringList::new(&names)).build();
        let mk = |title: &str, sub: &str| adw::SpinRow::builder().title(title).subtitle(sub).adjustment(&gtk::Adjustment::new(0.0, -100.0, 100.0, 5.0, 20.0, 0.0)).build();
        let hue_row = mk("Hue", "Towards the neighbouring colours");
        let sat_row = mk("Saturation", "How colourful");
        let light_row = mk("Lightness", "How light");
        let mut pn: Vec<&str> = vec!["Custom"];
        pn.extend(nv_process::colour::SELECTIVE_PRESETS.iter().map(|p| p.0));
        let preset_row = adw::ComboRow::builder().title("Preset").model(&gtk::StringList::new(&pn)).build();
        let reset_sel = gtk::Button::builder().label("Reset Selective Colour").halign(gtk::Align::End).margin_top(6).build();
        for r in [range_row.upcast_ref::<gtk::Widget>(), hue_row.upcast_ref(), sat_row.upcast_ref(), light_row.upcast_ref(), preset_row.upcast_ref()] {
            sel_group.add(r);
        }
        sel_group.add(&reset_sel);
        for r in [&hue_row, &sat_row, &light_row] {
            let weak = self.downgrade();
            r.connect_value_notify(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.on_params_changed();
                }
            });
        }
        {
            let weak = self.downgrade();
            range_row.connect_selected_notify(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.show_selective_range();
                }
            });
            let weak = self.downgrade();
            preset_row.connect_selected_notify(move |r| {
                let (Some(win), Some(i)) = (weak.upgrade(), (r.selected() as usize).checked_sub(1)) else { return };
                if win.imp().updating_ui.get() {
                    return;
                }
                let set = nv_process::colour::SELECTIVE_PRESETS[i].1;
                win.begin_live_edit();
                win.update_live(|p| p.colour.selective = set);
                win.end_live_edit();
                win.show_selective_range();
            });
            let weak = self.downgrade();
            reset_sel.connect_clicked(move |_| {
                let Some(win) = weak.upgrade() else { return };
                win.begin_live_edit();
                win.update_live(|p| p.colour.selective = [[0.0; 3]; 6]);
                win.end_live_edit();
                win.show_selective_range();
            });
        }
        *self.imp().selective_rows.borrow_mut() = Some((range_row, [hue_row, sat_row, light_row]));
        detail_page.add(&sel_group);

        let wheel_group = adw::PreferencesGroup::builder().title("Colour Balance").description("For shadows, midtones and highlights separately").build();
        wheel_group.add(&self.build_colour_wheel());
        detail_page.add(&wheel_group);
        let detail_scroll = gtk::ScrolledWindow::builder().hscrollbar_policy(gtk::PolicyType::Never).vexpand(true).child(&detail_page).build();
        insp_stack.add_titled(&detail_scroll, Some("detail"), "Detail");
        *self.imp().dust_mode_row.borrow_mut() = Some(dust_mode_row.clone());
        *self.imp().dust_detection_row.borrow_mut() = Some(dust_detection_row.clone());
        *self.imp().dust_note_row.borrow_mut() = Some(dust_note_row.clone());
        *self.imp().dust_method_row.borrow_mut() = Some(dust_method_row.clone());
        *self.imp().dust_defect_row.borrow_mut() = Some(dust_defect_row.clone());
        *self.imp().usm_rows.borrow_mut() = Some((usm_on.clone(), usm_amount.clone(), usm_radius.clone(), usm_threshold.clone()));
        *self.imp().noise_row.borrow_mut() = Some(noise_row.clone());
        let weak = self.downgrade();
        dust_mode_row.connect_selected_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        let weak = self.downgrade();
        dust_detection_row.connect_value_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        for row in [&dust_method_row, &dust_defect_row, &noise_row] {
            let weak = self.downgrade();
            row.connect_selected_notify(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.update_dust_availability();
                    win.on_params_changed();
                }
            });
        }
        let weak = self.downgrade();
        usm_on.connect_active_notify(move |_| {
            if let Some(win) = weak.upgrade() {
                win.on_params_changed();
            }
        });
        for row in [&usm_amount, &usm_radius, &usm_threshold] {
            let weak = self.downgrade();
            row.connect_value_notify(move |_| {
                if let Some(win) = weak.upgrade() {
                    win.on_params_changed();
                }
            });
        }

        insp_stack.add_titled(&self.build_scan_page(), Some("scan"), "Scan");
        // The page tabs scroll rather than set the least width of the whole window.
        let switcher_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .child(&switcher)
            .build();
        inspector_box.append(&switcher_scroll);
        inspector_box.append(&insp_stack);

        // ── Store inspector handles ───────────────────────────────────────
        *self.imp().image_type_row.borrow_mut() = Some(image_type_row.clone());
        // The Type row stays the single source of truth; the header dropdown and the
        // empty-state toggles follow it and write back to it.
        if let Some(btn) = self.imp().film_type_btn.borrow().as_ref() {
            image_type_row.bind_property("selected", btn, "selected").bidirectional().sync_create().build();
        }
        let toggle_buttons = self.imp().film_type_toggles.borrow().clone();
        for (i, b) in toggle_buttons.iter().enumerate() {
            let row = image_type_row.clone();
            b.connect_toggled(move |b| {
                if b.is_active() {
                    row.set_selected(i as u32);
                }
            });
        }
        let sync_toggles = {
            let toggle_buttons = toggle_buttons.clone();
            move |row: &adw::ComboRow| {
                if let Some(b) = toggle_buttons.get(row.selected() as usize) {
                    b.set_active(true);
                }
            }
        };
        sync_toggles(&image_type_row);
        image_type_row.connect_selected_notify(sync_toggles);
        *self.imp().profile_row.borrow_mut() = Some(profile_row.clone());
        *self.imp().exposure_row.borrow_mut() = Some(exposure_row.clone());
        *self.imp().film_base_r_row.borrow_mut() = Some(film_base_r_row.clone());
        *self.imp().film_base_g_row.borrow_mut() = Some(film_base_g_row.clone());
        *self.imp().film_base_b_row.borrow_mut() = Some(film_base_b_row.clone());
        *self.imp().black_pt_row.borrow_mut() = Some(black_pt_row.clone());
        *self.imp().white_pt_row.borrow_mut() = Some(white_pt_row.clone());
        *self.imp().midtone_row.borrow_mut() = Some(midtone_row.clone());
        *self.imp().contrast_row.borrow_mut() = Some(contrast_row.clone());
        *self.imp().cast_row.borrow_mut() = Some(cast_row.clone());
        *self.imp().balance_rows.borrow_mut() = balance_rows.clone();

        // ── Connect inspector signals ─────────────────────────────────────
        let win_weak = self.downgrade();
        image_type_row.connect_selected_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
                win.update_dust_availability();
                // Kodachrome has no use for the infrared pass. While the widgets are
                // being filled from a frame, load_params_for refreshes this itself.
                if !win.imp().updating_ui.get() {
                    win.update_scan_size();
                }
            }
        });
        let win_weak = self.downgrade();
        profile_row.connect_selected_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        exposure_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        film_base_r_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        film_base_g_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        film_base_b_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        black_pt_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        white_pt_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        midtone_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        contrast_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        let win_weak = self.downgrade();
        cast_row.connect_value_notify(move |_| {
            if let Some(win) = win_weak.upgrade() {
                win.on_params_changed();
            }
        });
        for row in &balance_rows {
            let win_weak = self.downgrade();
            row.connect_value_notify(move |_| {
                if let Some(win) = win_weak.upgrade() {
                    win.on_params_changed();
                }
            });
        }

        inspector_box
    }

    // ── Inspector <→ ProcessParams sync ──────────────────────────────────────

    fn on_params_changed(&self) {
        if self.imp().updating_ui.get() {
            return;
        }
        // Spin rows fire notify::value during widget realization; ignore those
        // until an actual scan is loaded.
        if !self.has_image() {
            return;
        }

        let Some(new_params) = self.read_params_from_inspector() else { return };
        let old = self.imp().proc_params.borrow().clone();
        if old.as_ref() == Some(&new_params) {
            return;
        }

        self.imp().history.borrow_mut().push(new_params.clone(), self.imp().scope.get());

        *self.imp().proc_params.borrow_mut() = Some(new_params.clone());
        if let Some(old) = old {
            self.record_edit(&old, &new_params);
        }
        self.refresh_canvas();
    }

    fn read_params_from_inspector(&self) -> Option<ProcessParams> {
        let profiles = self.film_profile_list();

        let image_type = {
            let row = self.imp().image_type_row.borrow();
            let row = row.as_ref()?;
            match row.selected() {
                0 => nv_process::params::ImageType::ColourNegative,
                1 => nv_process::params::ImageType::BwNegative,
                2 => nv_process::params::ImageType::Positive,
                _ => nv_process::params::ImageType::Kodachrome,
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
        let cast_removal = self.imp().cast_row.borrow().as_ref()?.value() as f32;
        let dust_mode = match self.imp().dust_mode_row.borrow().as_ref()?.selected() {
            0 => nv_process::DustMode::Off,
            2 => nv_process::DustMode::Mark,
            _ => nv_process::DustMode::Correct,
        };
        let dust_detection = self.imp().dust_detection_row.borrow().as_ref()?.value() as u8;
        let dust_method = if self.imp().dust_method_row.borrow().as_ref()?.selected() == 1 { nv_process::DustMethod::Software } else { nv_process::DustMethod::Infrared };
        let dust_defect = match self.imp().dust_defect_row.borrow().as_ref()?.selected() {
            1 => nv_process::DefectType::Bright,
            2 => nv_process::DefectType::Both,
            _ => nv_process::DefectType::Dark,
        };
        let (usm_enabled, usm_amount, usm_radius, usm_threshold) = {
            let rows = self.imp().usm_rows.borrow();
            let (on, a, r, t) = rows.as_ref()?;
            (on.is_active(), a.value() as f32, r.value() as f32, t.value() as u8)
        };
        let noise = match self.imp().noise_row.borrow().as_ref()?.selected() {
            1 => nv_process::NoiseLevel::Light,
            2 => nv_process::NoiseLevel::Medium,
            3 => nv_process::NoiseLevel::Strong,
            _ => nv_process::NoiseLevel::Off,
        };
        let balance = {
            let rows = self.imp().balance_rows.borrow();
            if rows.len() != 3 {
                return None;
            }
            [rows[0].value() as f32, rows[1].value() as f32, rows[2].value() as f32]
        };

        // Start from what is current: roll-wide values, the region and the
        // like are not in the inspector and must survive an edit.
        let mut p = self.imp().proc_params.borrow().clone().unwrap_or_else(settings::base_params);
        p.image_type = image_type;
        p.profile = profile;
        p.conversion.film_base = Some([fb_r, fb_g, fb_b]);
        p.conversion.exposure = exposure;
        p.conversion.cast_removal = cast_removal;
        p.dust.mode = dust_mode;
        p.dust.detection = dust_detection;
        p.dust.method = dust_method;
        p.dust.defect = dust_defect;
        p.detail.usm = nv_process::UsmParams { enabled: usm_enabled, amount: usm_amount, radius: usm_radius, threshold: usm_threshold };
        p.detail.noise = noise;
        p.detail.local_contrast = self.imp().local_contrast_row.borrow().as_ref()?.value() as f32;
        if let Some((range, rows)) = self.imp().selective_rows.borrow().as_ref() {
            let i = (range.selected() as usize).min(5);
            p.colour.selective[i] = [rows[0].value() as f32 / 100.0, rows[1].value() as f32 / 100.0, rows[2].value() as f32 / 100.0];
        }
        if let Some(row) = self.imp().scanner_profile_row.borrow().as_ref() {
            let chosen = row.selected_item().and_downcast::<gtk::StringObject>().map(|s| s.string().to_string()).filter(|n| n != "None");
            p.input = match chosen {
                None => None,
                // Keep the one already loaded when it is the one chosen.
                Some(n) if p.input.as_ref().is_some_and(|i| i.name == n) => p.input.clone(),
                Some(n) => nv_calibrate::load_profile(&n).ok(),
            };
        }
        if let Some((sat, acr, grey)) = self.imp().colour_rows.borrow().as_ref() {
            p.colour.saturation = sat.value() as f32 / 100.0;
            p.colour.acr = acr.value() as f32;
            match grey.selected() as usize {
                0 => p.colour.grey.enabled = false,
                i => {
                    p.colour.grey.enabled = true;
                    p.colour.grey.weights = nv_process::GREY_PRESETS[i - 1].1;
                }
            }
        }
        p.tone = ToneParams { black_point: black_pt, white_point: white_pt, midtone, contrast, balance, ..p.tone.clone() };
        Some(p)
    }

    /// Show `params` in the inspector without treating it as a user edit.
    fn sync_widgets(&self, params: &ProcessParams) {
        self.imp().updating_ui.set(true);

        if let Some(r) = self.imp().angle_row.borrow().as_ref() {
            r.set_value(self.angle() as f64);
        }
        let image_type_idx = match params.image_type {
            nv_process::params::ImageType::ColourNegative => 0u32,
            nv_process::params::ImageType::BwNegative => 1,
            nv_process::params::ImageType::Positive => 2,
            nv_process::params::ImageType::Kodachrome => 3,
        };
        if let Some(r) = self.imp().image_type_row.borrow().as_ref() {
            r.set_selected(image_type_idx);
        }

        let profiles = self.film_profile_list();
        if let Some(idx) = profiles.iter().position(|p| p.id == params.profile.id) {
            if let Some(r) = self.imp().profile_row.borrow().as_ref() {
                r.set_selected(idx as u32);
            }
        }

        if let Some(r) = self.imp().exposure_row.borrow().as_ref() {
            r.set_value(params.conversion.exposure as f64);
        }
        let fb = params.conversion.film_base.unwrap_or(params.profile.base_rgb);
        if let Some(r) = self.imp().film_base_r_row.borrow().as_ref() {
            r.set_value(fb[0] as f64);
        }
        if let Some(r) = self.imp().film_base_g_row.borrow().as_ref() {
            r.set_value(fb[1] as f64);
        }
        if let Some(r) = self.imp().film_base_b_row.borrow().as_ref() {
            r.set_value(fb[2] as f64);
        }
        if let Some(r) = self.imp().black_pt_row.borrow().as_ref() {
            r.set_value(params.tone.black_point as f64);
        }
        if let Some(r) = self.imp().white_pt_row.borrow().as_ref() {
            r.set_value(params.tone.white_point as f64);
        }
        if let Some(r) = self.imp().midtone_row.borrow().as_ref() {
            r.set_value(params.tone.midtone as f64);
        }
        if let Some(r) = self.imp().contrast_row.borrow().as_ref() {
            r.set_value(params.tone.contrast as f64);
        }
        if let Some(r) = self.imp().cast_row.borrow().as_ref() {
            r.set_value(params.conversion.cast_removal as f64);
        }
        if let Some(r) = self.imp().dust_mode_row.borrow().as_ref() {
            r.set_selected(match params.dust.mode {
                nv_process::DustMode::Off => 0,
                nv_process::DustMode::Correct => 1,
                nv_process::DustMode::Mark => 2,
            });
        }
        if let Some(r) = self.imp().dust_detection_row.borrow().as_ref() {
            r.set_value(params.dust.detection as f64);
        }
        if let Some(r) = self.imp().dust_method_row.borrow().as_ref() {
            r.set_selected(u32::from(params.dust.method == nv_process::DustMethod::Software));
        }
        if let Some(r) = self.imp().dust_defect_row.borrow().as_ref() {
            r.set_selected(match params.dust.defect {
                nv_process::DefectType::Dark => 0,
                nv_process::DefectType::Bright => 1,
                nv_process::DefectType::Both => 2,
            });
        }
        if let Some((on, a, r, t)) = self.imp().usm_rows.borrow().as_ref() {
            on.set_active(params.detail.usm.enabled);
            a.set_value(params.detail.usm.amount as f64);
            r.set_value(params.detail.usm.radius as f64);
            t.set_value(params.detail.usm.threshold as f64);
        }
        if let Some(r) = self.imp().local_contrast_row.borrow().as_ref() {
            r.set_value(params.detail.local_contrast as f64);
        }
        self.fill_selective_rows(&params.colour.selective);
        if let Some(row) = self.imp().scanner_profile_row.borrow().as_ref() {
            let names: Vec<String> =
                (0..row.model().map_or(0, |m| m.n_items())).filter_map(|i| row.model()?.item(i).and_downcast::<gtk::StringObject>().map(|s| s.string().to_string())).collect();
            row.set_selected(params.input.as_ref().and_then(|i| names.iter().position(|n| *n == i.name)).unwrap_or(0) as u32);
        }
        if let Some((sat, acr, grey)) = self.imp().colour_rows.borrow().as_ref() {
            sat.set_value(params.colour.saturation as f64 * 100.0);
            acr.set_value(params.colour.acr as f64);
            let pick = if params.colour.grey.enabled { nv_process::GREY_PRESETS.iter().position(|(_, w)| *w == params.colour.grey.weights).map_or(0, |i| i + 1) } else { 0 };
            grey.set_selected(pick as u32);
        }
        if let Some(r) = self.imp().noise_row.borrow().as_ref() {
            r.set_selected(match params.detail.noise {
                nv_process::NoiseLevel::Off => 0,
                nv_process::NoiseLevel::Light => 1,
                nv_process::NoiseLevel::Medium => 2,
                nv_process::NoiseLevel::Strong => 3,
            });
        }
        for (row, v) in self.imp().balance_rows.borrow().iter().zip(params.tone.balance) {
            row.set_value(v as f64);
        }

        self.imp().updating_ui.set(false);
    }

    fn commit_params(&self, params: ProcessParams) {
        let old = self.imp().proc_params.borrow().clone();
        self.sync_widgets(&params);
        self.imp().history.borrow_mut().push(params.clone(), self.imp().scope.get());
        *self.imp().proc_params.borrow_mut() = Some(params.clone());
        if let Some(old) = old {
            self.record_edit(&old, &params);
        }
        self.refresh_canvas();
    }

    /// Auto levels (F§5.1) from the picture on the canvas.
    fn auto_levels(&self) {
        let (Some(proxy), Some(mut params)) = (self.imp().proxy.borrow().clone(), self.imp().proc_params.borrow().clone()) else {
            self.show_toast("Open or preview a frame first");
            return;
        };
        params.tone = nv_process::auto_tone(&proxy, &params);
        self.commit_params(params);
    }

    /// Apply a pipette at `(fx, fy)` (fractions of the picture) and go back
    /// to the frame tool: a pick is one deliberate act, and the next click on
    /// the canvas is more likely to be a mistake than a second pick.
    fn pick_pixel(&self, tool: imp::CanvasTool, fx: f64, fy: f64) {
        use imp::CanvasTool;
        let (Some(proxy), Some(mut params)) = (self.imp().proxy.borrow().clone(), self.imp().proc_params.borrow().clone()) else { return };
        const RADIUS: usize = 3; // a 7 × 7 proxy-pixel patch, to average out grain

        let ok = match tool {
            CanvasTool::Frame | CanvasTool::Protect | CanvasTool::Heal | CanvasTool::Unmask => false,
            CanvasTool::FilmBase => match nv_process::sample_proxy(&proxy, fx, fy, RADIUS) {
                Some(base) => {
                    params.conversion.film_base = Some(base);
                    true
                }
                None => false,
            },
            CanvasTool::Black | CanvasTool::Grey | CanvasTool::White => match nv_process::sample_converted(&proxy, &params, fx, fy, RADIUS) {
                Some(s) => {
                    params.tone = match tool {
                        CanvasTool::Black => nv_process::pipette::black(&params.tone, s),
                        CanvasTool::White => nv_process::pipette::white(&params.tone, s),
                        _ => nv_process::pipette::grey(&params.tone, s),
                    };
                    true
                }
                None => false,
            },
        };
        if ok {
            self.commit_params(params);
        }
        self.set_canvas_tool(CanvasTool::Frame);
    }

    fn set_canvas_tool(&self, tool: imp::CanvasTool) {
        for (t, b) in self.imp().tool_buttons.borrow().iter() {
            if *t == tool {
                b.set_active(true);
            }
        }
    }

    fn reset_tone(&self) {
        let Some(mut params) = self.imp().proc_params.borrow().clone() else { return };
        params.tone = ToneParams::default();
        self.commit_params(params);
    }

    // ── Undo / redo ───────────────────────────────────────────────────────────
    /// Show `params` after an undo or redo, putting the roll or the frame
    /// back where the original edit went, whatever the scope is now.
    fn apply_history_step(&self, params: ProcessParams, scope: history::Scope) {
        let old = self.imp().proc_params.borrow().clone();
        self.sync_widgets(&params);
        *self.imp().proc_params.borrow_mut() = Some(params.clone());
        if let Some(old) = old {
            self.record_edit_in(&old, &params, scope);
        }
        self.refresh_canvas();
    }

    fn do_undo(&self) {
        let step = self.imp().history.borrow_mut().undo();
        if let Some((params, scope)) = step {
            self.apply_history_step(params, scope);
        }
    }

    fn do_redo(&self) {
        let step = self.imp().history.borrow_mut().redo();
        if let Some((params, scope)) = step {
            self.apply_history_step(params, scope);
        }
    }

    // ── Frame rectangle ───────────────────────────────────────────────────────

    /// Size of the picture on the canvas, as shown: the scan's size, turned.
    fn canvas_image_size(&self) -> Option<(f64, f64)> {
        let (w, h) = self.scan_image_size()?;
        let (dw, dh) = self.orientation().output_size(w as usize, h as usize);
        Some((dw as f64, dh as f64))
    }

    /// The frame to scan, as fractions of the full scan window.
    pub fn frame_rect(&self) -> Option<[f64; 4]> {
        self.imp().frame_rect.get()
    }

    fn set_frame_rect(&self, rect: Option<[f64; 4]>) {
        self.imp().frame_rect.set(rect);
        self.update_scan_size();
        if let Some(a) = self.imp().frame_overlay.borrow().as_ref() {
            a.queue_draw();
        }
    }

    fn setup_frame_overlay(&self, area: &gtk::DrawingArea) {
        use crate::canvas_geom as geom;

        // Draw: dim everything outside the rectangle, outline the rectangle,
        // then the brush strokes over the picture.
        let win_weak = self.downgrade();
        area.set_draw_func(move |_, cr, wid, hei| {
            let Some(win) = win_weak.upgrade() else { return };
            let Some((iw, ih)) = win.canvas_image_size() else { return };
            let content = geom::content_rect(wid as f64, hei as f64, iw, ih);
            let (cx, cy, cw, ch) = content;
            if let Some(rect) = win.frame_rect() {
                let rect = win.orientation().rect_from_source(rect);
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

                // Handles at the corners and edge midpoints, only when there is room for them.
                if win.imp().canvas_tool.get() == imp::CanvasTool::Frame && w > 40.0 && h > 40.0 {
                    for (hx, hy) in [(x, y), (x + w / 2.0, y), (x + w, y), (x, y + h / 2.0), (x + w, y + h / 2.0), (x, y + h), (x + w / 2.0, y + h), (x + w, y + h)] {
                        cr.rectangle(hx - 3.5, hy - 3.5, 7.0, 7.0);
                        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                        let _ = cr.fill_preserve();
                        cr.set_source_rgba(0.0, 0.0, 0.0, 0.8);
                        cr.set_line_width(1.0);
                        let _ = cr.stroke();
                    }
                }
            }
            win.draw_strokes(cr, content, (iw, ih));
        });

        // Drag out a new rectangle.
        let drag = gtk::GestureDrag::builder().button(gdk::BUTTON_PRIMARY).build();
        let start = std::rc::Rc::new(std::cell::Cell::new(None::<(f64, f64)>));
        // An existing rectangle grabbed by an edge, corner or its inside: the grab and the
        // rectangle as it was, in view space.
        let grabbed = std::rc::Rc::new(std::cell::Cell::new(None::<(geom::Grab, [f64; 4])>));

        let win_weak = self.downgrade();
        let start_c = start.clone();
        let grabbed_c = grabbed.clone();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(win) = win_weak.upgrade() else { return };
            grabbed_c.set(None);
            let tool = win.imp().canvas_tool.get();
            if mask::is_mask_tool(tool) {
                start_c.set(None);
                if let Some((fx, fy)) = g.widget().and_then(|w| win.canvas_fraction(&w, x, y)) {
                    win.begin_stroke(fx, fy);
                }
                return;
            }
            if tool != imp::CanvasTool::Frame {
                start_c.set(None);
                return;
            }
            let (Some((iw, ih)), Some(widget)) = (win.canvas_image_size(), g.widget()) else { return };
            let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            if let Some(view) = win.frame_rect().map(|r| win.orientation().rect_from_source(r)) {
                if let Some(grab) = geom::hit_test(view, x, y, c, HANDLE_REACH) {
                    start_c.set(None);
                    grabbed_c.set(Some((grab, view)));
                    return;
                }
            }
            start_c.set(geom::point_to_frac(x, y, c));
        });

        let win_weak = self.downgrade();
        let start_c = start.clone();
        let grabbed_c = grabbed.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            let Some(win) = win_weak.upgrade() else { return };
            if let (Some((grab, view)), Some((iw, ih)), Some(widget)) = (grabbed_c.get(), win.canvas_image_size(), g.widget()) {
                // Dragging a handle edits the rectangle; Shift keeps its shape.
                let (_, _, cw, ch) = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
                if cw > 0.0 && ch > 0.0 {
                    let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
                    let aspect = shift.then(|| (view[2] - view[0]) * iw / ((view[3] - view[1]) * ih));
                    let edited = geom::drag_rect(view, grab, dx / cw, dy / ch, aspect, iw / ih);
                    win.set_frame_rect(Some(win.orientation().rect_to_source(edited)));
                }
                return;
            }
            if mask::is_mask_tool(win.imp().canvas_tool.get()) {
                if let (Some(w), Some((sx, sy))) = (g.widget(), g.start_point()) {
                    if let Some((fx, fy)) = win.canvas_fraction(&w, sx + dx, sy + dy) {
                        win.extend_stroke(fx, fy);
                    }
                }
                return;
            }
            let (Some(a), Some((iw, ih)), Some(widget), Some((sx, sy))) = (start_c.get(), win.canvas_image_size(), g.widget(), g.start_point()) else { return };
            let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            if let Some(b) = geom::point_to_frac(sx + dx, sy + dy, c) {
                win.set_frame_rect(Some(win.orientation().rect_to_source(geom::normalised(a, b))));
            }
        });

        let win_weak = self.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            let Some(win) = win_weak.upgrade() else { return };
            start.set(None);
            if grabbed.take().is_some() {
                win.persist_frame_rect();
                return;
            }
            if mask::is_mask_tool(win.imp().canvas_tool.get()) {
                win.end_stroke();
                return;
            }
            // A click, not a drag: leave no sliver behind.
            if let Some([x0, y0, x1, y1]) = win.frame_rect() {
                if x1 - x0 < geom::MIN_SIDE || y1 - y0 < geom::MIN_SIDE {
                    win.set_frame_rect(None);
                }
            }
            win.persist_frame_rect();
        });
        area.add_controller(drag);

        // Show what a press would grab: resize arrows on the edges, a hand inside.
        let motion = gtk::EventControllerMotion::new();
        let win_weak = self.downgrade();
        motion.connect_motion(move |m, x, y| {
            let (Some(win), Some(widget)) = (win_weak.upgrade(), m.widget()) else { return };
            let mut name = None;
            if win.imp().canvas_tool.get() == imp::CanvasTool::Frame {
                if let (Some(rect), Some((iw, ih))) = (win.frame_rect(), win.canvas_image_size()) {
                    let c = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
                    let view = win.orientation().rect_from_source(rect);
                    name = geom::hit_test(view, x, y, c, HANDLE_REACH).map(|g| match g {
                        geom::Grab::Move => "move",
                        geom::Grab::Edge(-1, -1) => "nw-resize",
                        geom::Grab::Edge(1, -1) => "ne-resize",
                        geom::Grab::Edge(-1, 1) => "sw-resize",
                        geom::Grab::Edge(1, 1) => "se-resize",
                        geom::Grab::Edge(_, 0) => "ew-resize",
                        geom::Grab::Edge(_, _) => "ns-resize",
                    });
                }
            }
            widget.set_cursor_from_name(name);
        });
        area.add_controller(motion);

        // A click with a pipette tool picks the pixel under it.
        let pick = gtk::GestureClick::builder().button(gdk::BUTTON_PRIMARY).build();
        let win_weak = self.downgrade();
        pick.connect_pressed(move |g, _, x, y| {
            let Some(win) = win_weak.upgrade() else { return };
            let (tool, Some((iw, ih)), Some(widget)) = (win.imp().canvas_tool.get(), win.canvas_image_size(), g.widget()) else { return };
            if tool == imp::CanvasTool::Frame || mask::is_mask_tool(tool) {
                return;
            }
            let (cx, cy, cw, ch) = geom::content_rect(widget.width() as f64, widget.height() as f64, iw, ih);
            if x < cx || y < cy || x > cx + cw || y > cy + ch {
                return; // in the letterbox, not on the picture
            }
            if let Some((fx, fy)) = geom::point_to_frac(x, y, (cx, cy, cw, ch)) {
                // The picture may be turned: the pixel to read is in the scan.
                let p = win.orientation().rect_to_source([fx, fy, fx, fy]);
                win.pick_pixel(tool, p[0], p[1]);
            }
        });
        area.add_controller(pick);

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
            self.refresh_region();
            self.rebuild_frame_list();
            self.spawn_thumbnails(vec![idx]);
            self.schedule_autosave();
        }
    }

    /// Record the drawn rectangle in the frame's geometry, in preview pixels.
    fn store_frame_crop(&self, idx: u32) {
        let Some((iw, ih)) = self.scan_image_size() else { return };
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
        self.imp().last_scan_event.set(Some(std::time::Instant::now()));
        self.imp().scan_kind.set(imp::ScanKind::Preview);
        self.imp().preview_ready.set(false);
        self.imp().acquiring.set(true);
        self.set_busy(true);
        if let Some(app) = self.application() {
            app.activate_action("preview", None);
        }
    }

    /// Resolution for final scans: the roll's setting, else the session's.
    pub fn final_scan_dpi(&self) -> u32 {
        self.scan_dpi()
    }

    /// Scan the frame on the canvas, inside the rectangle drawn on it.
    pub(super) fn start_final_scan(&self) {
        if !self.imp().preview_ready.get() {
            return;
        }
        // Make sure there is a roll and a frame to put the result in before
        // the scanner spends minutes producing it.
        let Some(frame_idx) = self.ensure_roll_and_frame(false) else { return };
        self.store_frame_crop(frame_idx);
        self.schedule_autosave();
        self.flush_autosave();
        // A raw-only batch keeps its choice for every frame; a single raw-only
        // scan sets it for this frame alone (cleared when the scan is saved).
        self.imp().raw_only.set(self.imp().batch_raw_only.get());
        self.start_final_scan_of(frame_idx, self.frame_rect());
    }

    /// Start the final scan of frame `idx` over `area` of the scanner window.
    pub(super) fn start_final_scan_of(&self, idx: u32, area: Option<[f64; 4]>) {
        self.imp().last_scan_event.set(Some(std::time::Instant::now()));
        self.imp().scan_target.set(Some(idx));
        self.imp().scan_area.set(area);
        self.imp().final_dpi.set(self.final_scan_dpi());
        self.imp().scan_kind.set(imp::ScanKind::Final);
        self.imp().acquiring.set(true);
        self.set_busy(true);
        if let Some(app) = self.application() {
            app.activate_action("scan", None);
        }
    }

    /// The rectangle the scan in flight covers.
    pub fn scan_area(&self) -> Option<[f64; 4]> {
        self.imp().scan_area.get()
    }

    /// Make a roll folder on disk (in the user's Pictures folder) and make it
    /// the open roll. With `keep_frames`, frames already in memory move into it.
    fn create_roll_folder(&self, prefix: &str, keep_frames: bool) -> bool {
        let parent = glib::user_special_dir(glib::UserDirectory::Pictures).unwrap_or_else(glib::home_dir).join("Negative Vibes");
        let stamp = glib::DateTime::now_local().ok().and_then(|d| d.format("%Y-%m-%d %H-%M-%S").ok()).map(|s| s.to_string()).unwrap_or_else(|| "roll".to_owned());
        let name = format!("{prefix} {stamp}");
        let existing = if keep_frames { self.imp().roll.borrow().clone() } else { None };
        let created = std::fs::create_dir_all(&parent).map_err(nv_store::StoreError::from).and_then(|_| RollStore::create(&parent, &name));
        match created {
            Ok(mut store) => {
                // A new roll starts with the preferences.
                {
                    let prefs = self.imp().prefs.borrow();
                    store.roll.settings.export = prefs.export.clone();
                    store.roll.settings.resolution_dpi = Some(self.imp().scan_dpi.get());
                    store.roll.settings.with_ir = Some(prefs.scan_ir);
                    store.roll.settings.samples = Some(prefs.samples);
                    store.roll.settings.long_exposure = prefs.long_exposure;
                    if let Some(profile) = prefs.scanner_profile.as_deref().and_then(|n| nv_calibrate::load_profile(n).ok()) {
                        store.roll.settings.process.input = Some(profile);
                    }
                }
                if let Some(old) = existing {
                    store.roll.frames = old.frames;
                    store.roll.settings.process = old.settings.process;
                    // What the person chose before the folder existed (the Export dialog, say) stays.
                    store.roll.settings.export = old.settings.export;
                }
                *self.imp().roll_root.borrow_mut() = Some(store.root().to_owned());
                *self.imp().roll.borrow_mut() = Some(store.roll);
                *self.imp().selected_frame.borrow_mut() = None;
                self.imp().thumbs.borrow_mut().clear();
                if !keep_frames {
                    self.imp().histories.borrow_mut().clear();
                    *self.imp().history.borrow_mut() = Default::default();
                    self.imp().history_key.set(0);
                }
                if let Some(t) = self.imp().header_title.borrow().as_ref() {
                    t.set_title(&name);
                }
                true
            }
            Err(e) => {
                self.show_error("The roll folder could not be created", &e.to_string());
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

        self.imp().last_scan_event.set(Some(std::time::Instant::now()));
        match event {
            ScanEvent::DeviceList(caps) => {
                if caps.is_empty() {
                    self.set_page("no-scanner");
                    *self.imp().scan_state.borrow_mut() = ScanUiState::NoScanner;
                    self.set_scan_controls_sensitive(false, false);
                } else {
                    let first = caps.into_iter().next().unwrap();
                    *self.imp().device_resolutions.borrow_mut() = first.resolutions.clone();
                    self.imp().film_window_mm.set(first.film_window_mm);
                    self.imp().device_ir.set(matches!(first.ir, nv_device::IrStyle::MultiFrame | nv_device::IrStyle::Rgbi));
                    if let Some(r) = self.imp().ir_row.borrow().as_ref() {
                        r.set_visible(self.imp().device_ir.get());
                    }
                    self.imp().device_exposure.set(first.exposure_option.as_deref().is_some_and(|n| n.starts_with("exposure")));
                    self.update_passes_rows();
                    self.fill_resolutions();
                    let manual = first.holder == nv_device::HolderKind::ManualAdvance;
                    self.imp().manual_advance.set(manual);
                    if let Some(b) = self.imp().detect_btn.borrow().as_ref() {
                        b.set_visible(!manual);
                    }
                    if let Some(b) = self.imp().next_frame_btn.borrow().as_ref() {
                        b.set_visible(manual);
                    }
                    if self.imp().roll.borrow().is_none() {
                        self.set_page("insert-film");
                    }
                    *self.imp().scan_state.borrow_mut() = ScanUiState::Ready { device_name: first.device_name.clone() };
                    if let Some(t) = self.imp().header_title.borrow().as_ref() {
                        t.set_subtitle(&format!("{} {}", first.vendor, first.model));
                    }
                    self.update_scan_controls();
                    // Headless hook: NV_AUTOSCAN=raw|scan|all previews as soon as the scanner is ready,
                    // then scans as named (see on_scan_done).
                    if self.imp().prefs.borrow().auto_preview && std::env::var_os("NV_AUTOSCAN").is_none() && !self.imp().busy.get() && !self.imp().auto_previewed.replace(true) {
                        self.start_preview();
                    }
                    if std::env::var_os("NV_AUTOSCAN").is_some() && !self.imp().autoscan_started.replace(true) {
                        eprintln!("nv: NV_AUTOSCAN: previewing");
                        self.start_preview();
                    }
                }
            }

            ScanEvent::ScanStarted { width_px, height_px, bytes_per_line, depth, channels } => {
                let buf = FrameBuf::with_stride(width_px, height_px, depth, channels, bytes_per_line);
                if self.imp().scan_kind.get() == imp::ScanKind::Final {
                    *self.imp().final_buf.borrow_mut() = Some(buf);
                } else {
                    *self.imp().scan_buf.borrow_mut() = Some(buf);
                    self.imp().scan_dims.set(None);
                    self.imp().film_base_ready.set(false);
                    *self.imp().displayed_file.borrow_mut() = None;
                    *self.imp().proxy.borrow_mut() = None;
                    self.set_page("canvas");
                }
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(true);
                    pb.set_fraction(0.0);
                }
                let what = if self.imp().scan_kind.get() == imp::ScanKind::Final { "Colour scan running" } else { "Preview running" };
                self.set_progress_label(what);
            }

            ScanEvent::DataChunk { data, .. } => {
                if self.imp().scan_kind.get() == imp::ScanKind::Final {
                    let target = if self.imp().receiving_ir.get() { &self.imp().final_ir } else { &self.imp().final_buf };
                    if let Some(buf) = target.borrow_mut().as_mut() {
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
                // Headless hook for the cancel check (H4): press Stop at 30% of a final scan.
                if std::env::var_os("NV_AUTOSCAN_STOP").is_some() && self.imp().scan_kind.get() == imp::ScanKind::Final && p >= 0.3 && !self.imp().autostop_done.replace(true) {
                    eprintln!("nv: NV_AUTOSCAN_STOP: stopping at {:.0}%", p * 100.0);
                    self.imp().stop_at.set(Some(std::time::Instant::now()));
                    self.stop();
                }
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_fraction(p as f64);
                }
            }

            ScanEvent::ScanDone if self.imp().scan_kind.get() == imp::ScanKind::Final => {
                self.imp().pass_text.borrow_mut().clear();
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.finish_final_scan();
            }

            ScanEvent::ScanDone => {
                self.imp().pass_text.borrow_mut().clear();
                self.rebuild_proxy();
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
                self.end_batch();
                self.abort_acquisition();
                if let Some(t) = self.imp().stop_at.take() {
                    eprintln!("nv: NV_AUTOSCAN_STOP: stopped after {:.2} s; scanning again", t.elapsed().as_secs_f64());
                    self.scan_selected(true);
                }
            }

            ScanEvent::Error(msg) => {
                self.show_error("The scanner reported a problem", &msg);
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(false);
                }
                self.end_batch();
                self.abort_acquisition();
            }

            ScanEvent::IrFrameStarted { width_px, height_px, bytes_per_line, depth } => {
                if self.imp().scan_kind.get() == imp::ScanKind::Final {
                    *self.imp().final_ir.borrow_mut() = Some(FrameBuf::with_stride(width_px, height_px, depth, 1, bytes_per_line));
                    self.imp().receiving_ir.set(true);
                }
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_fraction(0.0);
                }
                self.set_progress_label("IR scan running");
            }

            ScanEvent::PassStarted { index, total, exposure } => {
                if let Some(pb) = self.imp().progress_bar.borrow().as_ref() {
                    pb.set_visible(true);
                }
                *self.imp().pass_text.borrow_mut() =
                    if exposure > 1.0 { format!("pass {} of {total}, long exposure", index + 1) } else { format!("pass {} of {total}", index + 1) };
                self.set_progress_label("Scan starting");
            }

            ScanEvent::Merging => {
                self.imp().pass_text.borrow_mut().clear();
                self.set_progress_label("Merging the passes…");
            }

            ScanEvent::ColorFrameDone | ScanEvent::IrFrameDone => {
                self.imp().receiving_ir.set(false);
            }

            ScanEvent::DeviceReady { .. } => {}
        }
    }

    /// Say what the scanner is doing above the progress bar, with the pass it is on if any.
    fn set_progress_label(&self, what: &str) {
        let pass = self.imp().pass_text.borrow();
        let text = if pass.is_empty() { what.to_owned() } else { format!("{what} ({pass})") };
        if let Some(l) = self.imp().progress_label.borrow().as_ref() {
            l.set_text(&text);
        }
    }

    /// A scan was asked for but the application had no ready scanner to give it to:
    /// do not leave the window waiting for something that was never started.
    pub fn scanner_not_ready(&self) {
        self.abort_acquisition();
        self.show_toast("The scanner is not ready");
    }

    fn abort_acquisition(&self) {
        *self.imp().final_buf.borrow_mut() = None;
        *self.imp().final_ir.borrow_mut() = None;
        self.imp().receiving_ir.set(false);
        self.imp().pass_text.borrow_mut().clear();
        // A failed or cancelled final scan leaves the preview on screen, so
        // Scan can be tried again; a failed preview leaves nothing to scan.
        let preview_ok = self.imp().scan_kind.get() == imp::ScanKind::Final && self.imp().preview_ready.get();
        self.imp().scan_kind.set(imp::ScanKind::Preview);
        self.imp().advance_pending.set(false);
        self.imp().preview_ready.set(preview_ok);
        self.set_busy(false);
    }

    /// Write the finished final scan into the roll's raw folder, off the UI
    /// thread (a 1200 dpi 16-bit frame is hundreds of megabytes).
    fn finish_final_scan(&self) {
        // A sound for the end of a scan, or of a whole batch.
        if self.imp().prefs.borrow().completion_sound && self.imp().batch.borrow().is_empty() {
            if let Some(display) = gdk::Display::default() {
                display.beep();
            }
        }
        let Some(buf) = self.imp().final_buf.borrow_mut().take() else {
            self.abort_acquisition();
            return;
        };
        let mut ir = self.imp().final_ir.borrow_mut().take();
        if ir.as_ref().is_some_and(|i| i.width != buf.width || i.effective_height() != buf.effective_height() || i.depth != buf.depth) {
            self.show_toast("The infrared plane does not match the colour scan: saving colour only");
            ir = None;
        }
        let has_ir = ir.is_some() || buf.channels == 4;
        let dpi = self.imp().final_dpi.get();
        let frame_idx = self.imp().scan_target.get();
        let root = self.imp().roll_root.borrow().clone();
        let (Some(frame_idx), Some(root)) = (frame_idx, root) else {
            self.show_toast("Scan finished but there is no roll to save it in");
            self.abort_acquisition();
            return;
        };

        let store = RollStore::from_parts(root, nv_store::Roll::new(""));
        let path = store.scan_path(frame_idx);
        let (w, h, depth, channels) = (buf.width, buf.effective_height(), buf.depth, buf.channels + u8::from(ir.is_some()));
        let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();

        self.show_toast("Saving scan…");
        let win_weak = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let write_path = path.clone();
            let result = gio::spawn_blocking(move || nv_store::RawWriter::write_scan_with_ir(&buf, ir.as_ref(), &write_path)).await;
            let Some(win) = win_weak.upgrade() else { return };
            let mut saved = false;
            match result {
                Ok(Ok(())) => {
                    win.record_scan(frame_idx, filename, dpi, depth, channels, w, h, has_ir);
                    win.show_toast(&format!("Saved frame {frame_idx} ({w}×{h}, {dpi} dpi)"));
                    if win.imp().manual_advance.get() {
                        win.show_advance_banner();
                    }
                    // Export while the next frame scans, unless this was a raw-only scan.
                    let raw_only = win.imp().raw_only.get();
                    if !raw_only && win.export_settings().after_scan {
                        win.queue_export(&[frame_idx]);
                    }
                    saved = true;
                    if std::env::var("NV_AUTOSCAN").is_ok_and(|m| m == "raw") {
                        eprintln!("nv: NV_AUTOSCAN: raw saved for frame {frame_idx}");
                        win.flush_autosave();
                        win.application().map(|a| a.quit());
                    }
                }
                Ok(Err(e)) => win.show_error("The scan could not be saved", &e.to_string()),
                Err(_) => win.show_error("The scan could not be saved", "The writer stopped unexpectedly."),
            }
            win.abort_acquisition();
            win.update_info_label();
            // Show what was just scanned instead of the preview it was framed on. A batch
            // shows its last frame when it is over, and the next frame of one scans first.
            if saved && win.imp().batch.borrow().is_empty() {
                win.imp().preview_view.borrow_mut().remove(&frame_idx);
                if win.selected() == Some(frame_idx) {
                    win.select_frame(frame_idx);
                }
            }
            // The scanner is free: on to the next frame of a batch.
            if saved {
                win.continue_batch();
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn record_scan(&self, idx: u32, filename: String, dpi: u32, depth: u8, channels: u8, w: u32, h: u32, has_ir: bool) {
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
                        scanned_at: glib::DateTime::now_utc().ok().and_then(|d| d.format_iso8601().ok()).map(|s| s.to_string()).unwrap_or_default(),
                    },
                    has_ir,
                    width_px: w,
                    height_px: h,
                });
            }
        }
        self.rebuild_frame_list();
        self.schedule_autosave();
    }

    // ── Film base auto-estimation ─────────────────────────────────────────────
    // ── Canvas ────────────────────────────────────────────────────────────────
    fn refresh_canvas(&self) {
        self.schedule_loupe();
        self.schedule_hq();
        // A finished scan renders from the proxy, in the background.
        if self.imp().film_base_ready.get() && self.imp().proxy.borrow().is_some() {
            self.request_render();
            self.apply_zoom();
            return;
        }

        // Data is still arriving: the fast display curve over the rows so far.
        let (w, h, rgb8) = {
            let buf = self.imp().scan_buf.borrow();
            let Some(buf) = buf.as_ref() else { return };
            let h = buf.rows_available().min(buf.effective_height());
            if buf.width == 0 || h == 0 {
                return;
            }
            (buf.width as usize, h as usize, buf.to_rgb8())
        };
        self.show_rgb8(w, h, &rgb8);
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
            Err(e) => self.show_error("The roll could not be created", &e.to_string()),
        }
    }

    /// Open `roll`, which lives in `root` on disk.
    pub fn load_roll(&self, roll: Roll, root: Option<std::path::PathBuf>) {
        self.flush_autosave();
        if let Some(t) = self.imp().header_title.borrow().as_ref() {
            t.set_title(&roll.name);
        }
        *self.imp().histories.borrow_mut() = root.as_deref().map(Self::load_histories).unwrap_or_default();
        *self.imp().history.borrow_mut() = Default::default();
        self.imp().history_key.set(0);
        *self.imp().roll.borrow_mut() = Some(roll);
        *self.imp().roll_root.borrow_mut() = root;
        *self.imp().selected_frame.borrow_mut() = None;
        self.show_opened_roll();
    }
    /// A preview finished: it becomes the selected frame's image.
    fn on_scan_done(&self) {
        self.adopt_preview();
        self.update_info_label();
        // A Scan All loop on a manual-advance scanner: scan what was just previewed.
        if self.imp().auto_scan_next.replace(false) {
            self.start_final_scan();
        }
        // Headless hook: the preview is in; do what NV_AUTOSCAN says, once.
        if let Ok(mode) = std::env::var("NV_AUTOSCAN") {
            if !self.imp().autoscan_done.replace(true) {
                eprintln!("nv: NV_AUTOSCAN: preview done, running {mode}");
                match mode.as_str() {
                    "raw" => self.scan_selected(true),
                    "all" => self.scan_all(),
                    _ => self.scan_selected(false),
                }
            }
        }
    }
    fn save_roll(&self) -> Option<()> {
        let root = self.imp().roll_root.borrow().clone()?;
        let roll = self.imp().roll.borrow().clone()?;
        if let Err(e) = RollStore::from_parts(root.clone(), roll).save() {
            self.show_error("The roll could not be saved", &e.to_string());
            return None;
        }
        self.save_histories(&root);
        Some(())
    }

    /// Undo history of every frame, next to `roll.json`. Failing to write it
    /// loses undo, not the roll, so it only logs.
    fn save_histories(&self, root: &std::path::Path) {
        let imp = self.imp();
        const KEPT: usize = 30;
        let mut all: std::collections::BTreeMap<u32, history::History> = imp.histories.borrow().iter().map(|(k, h)| (*k, h.trimmed(KEPT))).collect();
        if imp.history_key.get() != 0 {
            all.insert(imp.history_key.get(), imp.history.borrow().trimmed(KEPT));
        }
        let write = || -> std::io::Result<()> {
            let tmp = root.join(".history.json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&all)?)?;
            std::fs::rename(tmp, root.join("history.json"))
        };
        if let Err(e) = write() {
            eprintln!("nv: could not save undo history: {e}");
        }
    }

    /// The undo histories saved with the roll in `root`, if any. A file that
    /// cannot be read (older version, damage) just means no history.
    fn load_histories(root: &std::path::Path) -> std::collections::HashMap<u32, history::History> {
        std::fs::read(root.join("history.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    // ── Open Raw Scans ────────────────────────────────────────────────────────

    fn open_raw_scans(&self) {
        let dialog = gtk::FileDialog::builder().title("Open Raw Scans").modal(true).build();

        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Linear TIFF / DNG / PNM"));
        filter.add_mime_type("image/tiff");
        filter.add_pattern("*.tif");
        filter.add_pattern("*.tiff");
        filter.add_pattern("*.dng");
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
        let sources: Vec<std::path::PathBuf> = (0..files.n_items()).filter_map(|i| files.item(i).and_downcast::<gio::File>()?.path()).collect();
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
        let proxy = nv_process::Proxy::from_raw(&buf, nv_process::DEFAULT_PROXY_PIXELS);
        self.load_frame_buf_with_proxy(buf, proxy);
    }

    /// Show a big scan from its working-size copy alone: the frame is not held
    /// in memory, only its size.
    fn load_proxy_only(&self, proxy: nv_process::Proxy, size: (u32, u32)) {
        *self.imp().scan_buf.borrow_mut() = None;
        self.imp().scan_dims.set(Some(size));
        *self.imp().proxy.borrow_mut() = Some(std::sync::Arc::new(proxy));
        self.set_page("canvas");
        let frame = *self.imp().selected_frame.borrow();
        self.load_params_for(frame);
    }

    /// Show a scan whose proxy was already built (on a worker thread).
    fn load_frame_buf_with_proxy(&self, buf: nv_image::FrameBuf, proxy: nv_process::Proxy) {
        // Only a preview-sized scan stays in memory whole: everything that
        // needs the pixels again (detection, saving the preview) is about
        // previews, and a big raw is read from its file when exported.
        const KEEP_WHOLE_BYTES: usize = 96 << 20;
        self.imp().scan_dims.set(Some((buf.width, buf.effective_height())));
        *self.imp().scan_buf.borrow_mut() = (buf.bytes_received() <= KEEP_WHOLE_BYTES).then_some(buf);
        *self.imp().proxy.borrow_mut() = Some(std::sync::Arc::new(proxy));
        self.set_page("canvas");
        let frame = *self.imp().selected_frame.borrow();
        self.load_params_for(frame);
    }

    // ── Open Roll ─────────────────────────────────────────────────────────────

    fn open_roll_dialog(&self) {
        let dialog = gtk::FileDialog::builder().title("Open Roll").modal(true).build();

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
                Err(e) => win.show_error("The roll could not be opened", &e.to_string()),
            }
        });
    }
    fn open_roll_at(&self, path: &std::path::Path) {
        match RollStore::open(path) {
            Ok(store) => {
                let root = store.root().to_owned();
                self.load_roll(store.roll, Some(root));
            }
            Err(e) => self.show_error("The roll could not be opened", &e.to_string()),
        }
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn set_page(&self, name: &str) {
        if let Some(stack) = self.imp().main_stack.borrow().as_ref() {
            stack.set_visible_child_name(name);
        }
    }

    fn set_scan_controls_sensitive(&self, preview: bool, scan: bool) {
        if let Some(b) = self.imp().preview_btn.borrow().as_ref() {
            b.set_sensitive(preview);
        }
        if let Some(b) = self.imp().scan_btn.borrow().as_ref() {
            b.set_sensitive(scan);
        }
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
