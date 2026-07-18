mod workspace;

use std::{
    collections::HashSet,
    fs,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use dither_core::{
    AssetKind, CrtPhase, DitherAlgorithm, Document, FourColor, HalftoneShape, Ink, MapPattern,
    Monochrome, PaletteSettings, Recipe, RenderedDocument, RenderedImage, Resampling, Separation,
    ThreeColor, ToneBand, TriTone, built_in_presets,
};
use dither_io::{ExportFormat, ExportOptions, IoError};
use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, TextStyle,
    TextureHandle, Vec2,
};
use rfd::FileDialog;
#[cfg(target_os = "macos")]
use muda::{
    Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};
use workspace::{
    BrowserSort, ExportRecord, ExportSettings, PersistentState, ProjectFile, SavedExportFormat,
};

const PREVIEW_SIZE: NonZeroU32 = NonZeroU32::new(1400).unwrap();
const THUMBNAIL_SIZE: NonZeroU32 = NonZeroU32::new(128).unwrap();
const CANVAS: Color32 = Color32::from_rgb(11, 11, 10);
const PANEL: Color32 = Color32::from_rgb(24, 24, 21);
const PANEL_RAISED: Color32 = Color32::from_rgb(31, 31, 27);
const BORDER: Color32 = Color32::from_rgb(52, 52, 45);
const PAPER: Color32 = Color32::from_rgb(238, 229, 205);
const MUTED: Color32 = Color32::from_rgb(155, 150, 138);
const ACCENT: Color32 = Color32::from_rgb(197, 53, 40);

#[cfg(target_os = "macos")]
fn install_native_menu() -> muda::Result<Menu> {
    let command = Some(Modifiers::SUPER);
    let item = |id: &str, text: &str, key: Option<Code>| {
        MenuItem::with_id(
            id,
            text,
            true,
            key.map(|key| Accelerator::new(command, key)),
        )
    };
    let app = Submenu::with_items(
        "Dither",
        true,
        &[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ],
    )?;
    let file = Submenu::with_items(
        "File",
        true,
        &[
            &item("open", "Open…", Some(Code::KeyO)),
            &item("open-new-tab", "Open in New Tab…", None),
            &PredefinedMenuItem::separator(),
            &item("open-project", "Open Project…", None),
            &item("save-project", "Save Project As…", Some(Code::KeyS)),
            &PredefinedMenuItem::separator(),
            &item("reload-source", "Reload Source", None),
            &item("relink-source", "Relink Source…", None),
            &PredefinedMenuItem::separator(),
            &item("export", "Export…", Some(Code::KeyE)),
        ],
    )?;
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &item("undo", "Undo", Some(Code::KeyZ)),
            &MenuItem::with_id(
                "redo",
                "Redo",
                true,
                Some(Accelerator::new(
                    Some(Modifiers::SUPER | Modifiers::SHIFT),
                    Code::KeyZ,
                )),
            ),
            &PredefinedMenuItem::separator(),
            &item("duplicate-tab", "Duplicate Tab", None),
            &item("close-tab", "Close Tab", Some(Code::KeyW)),
        ],
    )?;
    let view = Submenu::with_items(
        "View",
        true,
        &[
            &item("toggle-files", "Toggle Files", None),
            &PredefinedMenuItem::separator(),
            &item("view-effect", "Effect", None),
            &item("view-original", "Original", None),
            &item("view-snapshot", "Snapshot", None),
            &PredefinedMenuItem::separator(),
            &item("view-fit", "Fit", None),
            &item("view-actual-size", "Actual Size", None),
            &item("capture-snapshot", "Capture Snapshot", None),
        ],
    )?;
    let window = Submenu::with_items(
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(None),
            &PredefinedMenuItem::fullscreen(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::bring_all_to_front(None),
        ],
    )?;
    let menu = Menu::with_items(&[&app, &file, &edit, &view, &window])?;
    menu.init_for_nsapp();
    window.set_as_windows_menu_for_nsapp();
    Ok(menu)
}

#[derive(Clone)]
struct PlateTexture {
    name: String,
    color: Color32,
    grayscale: TextureHandle,
    inked: TextureHandle,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum PlateView {
    #[default]
    Composite,
    Grayscale(usize),
    Inked(usize),
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum InspectorTab {
    #[default]
    Adjust,
    Plates,
    Output,
}

struct BrowserEntry {
    path: PathBuf,
    modified: Option<SystemTime>,
    thumbnail: Option<TextureHandle>,
    error: Option<String>,
}

struct BrowserState {
    folder: Option<PathBuf>,
    entries: Vec<BrowserEntry>,
    filter: String,
    generation: u64,
    watch: bool,
    known_paths: HashSet<PathBuf>,
    last_scan: Instant,
}

impl Default for BrowserState {
    fn default() -> Self {
        Self {
            folder: None,
            entries: Vec::new(),
            filter: String::new(),
            generation: 0,
            watch: false,
            known_paths: HashSet::new(),
            last_scan: Instant::now(),
        }
    }
}

#[derive(Clone)]
struct SavedTab {
    document: Document,
    preview: Option<TextureHandle>,
    original_preview: Option<TextureHandle>,
    plate_previews: Vec<PlateTexture>,
    recipe: Recipe,
    undo: Vec<Recipe>,
    redo: Vec<Recipe>,
    pending_edit: Option<Recipe>,
    show_original: bool,
    show_comparison: bool,
    comparison: Option<TextureHandle>,
    comparison_recipe: Option<Recipe>,
    scene_rect: egui::Rect,
    plate_view: PlateView,
    project_path: Option<PathBuf>,
    export: ExportSettings,
    dirty: bool,
}

struct BatchResult {
    source: PathBuf,
    outputs: Vec<PathBuf>,
    error: Option<String>,
}

fn configure_fonts(context: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "plex-sans-condensed".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexSansCondensed-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "plex-sans-condensed-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexSansCondensed-SemiBold.ttf"
        ))),
    );
    fonts.font_data.insert(
        "plex-mono".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/fonts/IBMPlexMono-Regular.ttf"
        ))),
    );
    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "plex-sans-condensed".into());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .insert(0, "plex-mono".into());
    let mut heading_fonts = vec!["plex-sans-condensed-semibold".into()];
    heading_fonts.extend(fonts.families[&FontFamily::Proportional].clone());
    fonts
        .families
        .insert(FontFamily::Name(Arc::from("plex-heading")), heading_fonts);
    context.set_fonts(fonts);
}

fn main() -> eframe::Result {
    eframe::run_native(
        "Dither",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1280.0, 820.0])
                .with_min_inner_size([900.0, 620.0]),
            ..Default::default()
        },
        Box::new(|context| Ok(Box::new(Editor::new(context)))),
    )
}

struct Editor {
    #[cfg(target_os = "macos")]
    _native_menu: Menu,
    document: Option<Document>,
    preview: Option<TextureHandle>,
    original_preview: Option<TextureHandle>,
    plate_previews: Vec<PlateTexture>,
    plate_view: PlateView,
    status: String,
    confirm: Option<PendingExport>,
    jobs: Sender<JobResult>,
    results: Receiver<JobResult>,
    open_request: u64,
    preview_request: u64,
    preview_due: Option<Instant>,
    preview_in_flight: bool,
    opening: bool,
    exporting: bool,
    export_progress: Arc<AtomicU8>,
    export_cancel: Arc<AtomicBool>,
    export_settings: ExportSettings,
    export_history_open: bool,
    inspector_tab: InspectorTab,
    library_open: bool,
    recipe: Recipe,
    undo: Vec<Recipe>,
    redo: Vec<Recipe>,
    pending_edit: Option<Recipe>,
    show_original: bool,
    show_comparison: bool,
    scene_rect: egui::Rect,
    comparison: Option<TextureHandle>,
    comparison_recipe: Option<Recipe>,
    browser: BrowserState,
    persistent: PersistentState,
    tabs: Vec<Option<SavedTab>>,
    tab_labels: Vec<String>,
    active_tab: usize,
    open_in_new_tab: bool,
    preserve_recipe_on_replace: bool,
    project_path: Option<PathBuf>,
    dirty: bool,
    source_modified: Option<SystemTime>,
    last_source_check: Instant,
    batch_results: Vec<BatchResult>,
    batch_running: bool,
    pending_open: Option<PathBuf>,
}

enum JobResult {
    Opened {
        request: u64,
        new_tab: bool,
        recipe: Box<Recipe>,
        result: Box<Result<(Document, RenderedImage), String>>,
    },
    Previewed {
        request: u64,
        image: RenderedDocument,
    },
    Exported {
        path: PathBuf,
        format: ExportFormat,
        options: ExportOptions,
        result: Result<(), IoError>,
        separated: bool,
        outputs: Vec<PathBuf>,
    },
    Thumbnail {
        generation: u64,
        path: PathBuf,
        result: Box<Result<RenderedImage, String>>,
    },
    BatchFinished {
        results: Vec<BatchResult>,
    },
}

struct PendingExport {
    path: PathBuf,
    format: ExportFormat,
    options: ExportOptions,
    reason: String,
    separated: bool,
}

impl Editor {
    fn new(context: &eframe::CreationContext<'_>) -> Self {
        context.egui_ctx.set_theme(egui::Theme::Dark);
        configure_fonts(&context.egui_ctx);
        let mut style = (*context.egui_ctx.style_of(egui::Theme::Dark)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(PAPER);
        style.visuals.weak_text_color = Some(MUTED);
        style.visuals.panel_fill = PANEL;
        style.visuals.window_fill = PANEL_RAISED;
        style.visuals.extreme_bg_color = CANVAS;
        style.visuals.faint_bg_color = Color32::from_rgb(35, 35, 30);
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(42, 42, 37);
        style.visuals.widgets.inactive.fg_stroke.color = PAPER;
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 57, 49);
        style.visuals.widgets.hovered.fg_stroke.color = PAPER;
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.visuals.selection.bg_fill = ACCENT;
        style.visuals.selection.stroke.color = PAPER;
        style.animation_time = 0.14;
        style.text_styles.insert(
            TextStyle::Heading,
            FontId::new(21.0, FontFamily::Name(Arc::from("plex-heading"))),
        );
        style
            .text_styles
            .insert(TextStyle::Body, FontId::new(15.0, FontFamily::Proportional));
        style.text_styles.insert(
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        );
        style.text_styles.insert(
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        );
        style.spacing.item_spacing = Vec2::new(8.0, 9.0);
        style.spacing.button_padding = Vec2::new(12.0, 7.0);
        context.egui_ctx.set_style_of(egui::Theme::Dark, style);

        let (jobs, results) = mpsc::channel();
        let recipe = load_recipe();
        let persistent: PersistentState = workspace::load_json(&state_path()).unwrap_or_default();
        let export_settings = persistent.default_export.clone();
        let mut editor = Self {
            #[cfg(target_os = "macos")]
            _native_menu: install_native_menu().expect("failed to install macOS menu"),
            document: None,
            preview: None,
            original_preview: None,
            plate_previews: Vec::new(),
            plate_view: PlateView::Composite,
            status: "Open an image to begin".into(),
            confirm: None,
            jobs,
            results,
            open_request: 0,
            preview_request: 0,
            preview_due: None,
            preview_in_flight: false,
            opening: false,
            exporting: false,
            export_progress: Arc::new(AtomicU8::new(0)),
            export_cancel: Arc::new(AtomicBool::new(false)),
            export_settings,
            export_history_open: false,
            inspector_tab: InspectorTab::Adjust,
            library_open: false,
            recipe,
            undo: Vec::new(),
            redo: Vec::new(),
            pending_edit: None,
            show_original: false,
            show_comparison: false,
            scene_rect: egui::Rect::ZERO,
            comparison: None,
            comparison_recipe: None,
            browser: BrowserState::default(),
            persistent,
            tabs: Vec::new(),
            tab_labels: Vec::new(),
            active_tab: 0,
            open_in_new_tab: false,
            preserve_recipe_on_replace: true,
            project_path: None,
            dirty: false,
            source_modified: None,
            last_source_check: Instant::now(),
            batch_results: Vec::new(),
            batch_running: false,
            pending_open: None,
        };
        if let Some(path) = std::env::args_os().nth(1).map(PathBuf::from) {
            editor.open_path(path, &context.egui_ctx);
        } else if let Ok(project) = workspace::load_json::<ProjectFile>(&recovery_path())
            && project.source.exists()
        {
            editor.open_project_data(project, true, &context.egui_ctx);
            editor.status = "Recovered the previous autosaved session".into();
        } else if let Some(folder) = editor.persistent.last_folder.clone() {
            editor.load_folder(folder, &context.egui_ctx);
        }
        editor
    }

    fn open(&mut self, context: &egui::Context) {
        let extensions: Vec<_> = dither_io::supported_extensions().collect();
        let Some(path) = FileDialog::new()
            .set_title("Open original image")
            .add_filter("Supported images", &extensions)
            .pick_file()
        else {
            return;
        };

        self.open_path(path, context);
    }

    fn open_path(&mut self, path: PathBuf, context: &egui::Context) {
        if self.dirty && self.document.is_some() {
            self.pending_open = Some(path);
            self.status = "Save or discard the current project before replacing it".into();
            return;
        }
        let recipe = if self.preserve_recipe_on_replace {
            self.document
                .as_ref()
                .map(|document| document.recipe.clone())
                .unwrap_or_else(|| self.recipe.clone())
        } else {
            Recipe::default()
        };
        self.open_path_with_recipe(path, recipe, self.open_in_new_tab, context);
    }

    fn open_path_with_recipe(
        &mut self,
        path: PathBuf,
        recipe: Recipe,
        new_tab: bool,
        context: &egui::Context,
    ) {
        self.open_request = self.open_request.wrapping_add(1);
        let request = self.open_request;
        let sender = self.jobs.clone();
        let repaint = context.clone();
        self.opening = true;
        self.status = format!("Opening {}…", path.display());
        thread::spawn(move || {
            let result = dither_io::open(path)
                .map(|source| {
                    let document = Document::new(source);
                    let preview = document.render_source_preview(PREVIEW_SIZE);
                    (document, preview)
                })
                .map_err(|error| error.to_string());
            let _ = sender.send(JobResult::Opened {
                request,
                new_tab,
                recipe: Box::new(recipe),
                result: Box::new(result),
            });
            repaint.request_repaint();
        });
    }

    fn open_project_data(&mut self, project: ProjectFile, new_tab: bool, context: &egui::Context) {
        self.export_settings = project.export;
        self.project_path = None;
        self.open_path_with_recipe(project.source, project.recipe, new_tab, context);
    }

    fn schedule_preview(&mut self, context: &egui::Context) {
        self.preview_request = self.preview_request.wrapping_add(1);
        self.preview_due = Some(Instant::now() + Duration::from_millis(75));
        context.request_repaint_after(Duration::from_millis(75));
    }

    fn launch_preview(&mut self, context: &egui::Context) {
        if self.preview_in_flight {
            return;
        }
        let Some(document) = &self.document else {
            return;
        };
        let document = document.clone();
        let request = self.preview_request;
        let sender = self.jobs.clone();
        let repaint = context.clone();
        self.preview_due = None;
        self.preview_in_flight = true;
        thread::spawn(move || {
            let image = document.render_document_preview(PREVIEW_SIZE);
            let _ = sender.send(JobResult::Previewed { request, image });
            repaint.request_repaint();
        });
    }

    fn process_jobs(&mut self, context: &egui::Context) {
        while let Ok(job) = self.results.try_recv() {
            match job {
                JobResult::Opened {
                    request,
                    new_tab,
                    recipe,
                    result,
                } if request == self.open_request => {
                    self.opening = false;
                    match *result {
                        Ok((mut document, original)) => {
                            if new_tab && self.document.is_some() {
                                self.stash_active_tab();
                                self.tabs.push(None);
                                self.active_tab = self.tabs.len() - 1;
                            } else if self.tabs.is_empty() {
                                self.tabs.push(None);
                                self.active_tab = 0;
                            }
                            document.recipe = *recipe;
                            let asset_errors = load_recipe_assets(&mut document);
                            let source = document.source();
                            let source_path = source.info.path.clone();
                            self.status = format!(
                                "{} × {}  ·  {}-bit {}",
                                source.width(),
                                source.height(),
                                source.info.bit_depth,
                                source.info.format
                            );
                            if !asset_errors.is_empty() {
                                self.status.push_str(&format!(
                                    "  ·  {} asset(s) unavailable",
                                    asset_errors.len()
                                ));
                            }
                            self.document = Some(document);
                            self.recipe = self.document.as_ref().unwrap().recipe.clone();
                            self.preview = None;
                            self.plate_previews.clear();
                            self.plate_view = PlateView::Composite;
                            self.original_preview = Some(context.load_texture(
                                "original-preview",
                                preview_image(&original),
                                egui::TextureOptions::LINEAR,
                            ));
                            self.undo.clear();
                            self.redo.clear();
                            self.pending_edit = None;
                            self.show_original = false;
                            self.show_comparison = false;
                            self.comparison = None;
                            self.comparison_recipe = None;
                            self.scene_rect = egui::Rect::ZERO;
                            self.project_path = None;
                            self.export_settings = self.persistent.default_export.clone();
                            self.dirty = false;
                            self.source_modified = file_modified(&source_path);
                            self.persistent.remember_file(source_path.clone());
                            self.set_active_tab_label(&source_path);
                            self.save_persistent();
                            self.schedule_preview(context);
                            self.autosave_recovery();
                        }
                        Err(error) => self.status = format!("Open failed: {error}"),
                    }
                }
                JobResult::Opened { .. } => {}
                JobResult::Previewed { request, image } => {
                    self.preview_in_flight = false;
                    if request == self.preview_request {
                        let color = preview_image(&image.composite);
                        self.preview = Some(context.load_texture(
                            "document-preview",
                            color,
                            egui::TextureOptions::LINEAR,
                        ));
                        self.plate_previews = image
                            .plates
                            .iter()
                            .map(|plate| PlateTexture {
                                name: plate.name.clone(),
                                color: rgb_color(plate.ink.color),
                                grayscale: context.load_texture(
                                    format!("plate-{}-gray", plate.name),
                                    plate_image(
                                        plate.coverage(),
                                        image.composite.width(),
                                        image.composite.height(),
                                        [1.0; 3],
                                    ),
                                    egui::TextureOptions::NEAREST,
                                ),
                                inked: context.load_texture(
                                    format!("plate-{}-ink", plate.name),
                                    plate_image(
                                        plate.coverage(),
                                        image.composite.width(),
                                        image.composite.height(),
                                        plate.ink.color,
                                    ),
                                    egui::TextureOptions::NEAREST,
                                ),
                            })
                            .collect();
                    }
                }
                JobResult::Exported {
                    path,
                    format,
                    options,
                    result,
                    separated,
                    outputs,
                } => {
                    self.exporting = false;
                    match result {
                        Ok(()) => {
                            self.status = if separated {
                                format!("Exported composite and plates at {}", path.display())
                            } else {
                                format!("Exported {}", path.display())
                            };
                            self.persistent.remember_export(ExportRecord {
                                source: self
                                    .document
                                    .as_ref()
                                    .map(|document| document.source().info.path.clone())
                                    .unwrap_or_default(),
                                outputs,
                                completed_unix_seconds: unix_seconds(),
                            });
                            self.save_persistent();
                        }
                        Err(error) => {
                            if let Some(options) = confirmed_options(&error, options) {
                                self.confirm = Some(PendingExport {
                                    path,
                                    format,
                                    options,
                                    reason: error.to_string(),
                                    separated,
                                });
                                self.status = "Export needs confirmation".into();
                            } else {
                                self.status = format!("Export failed: {error}");
                            }
                        }
                    }
                }
                JobResult::Thumbnail {
                    generation,
                    path,
                    result,
                } if generation == self.browser.generation => {
                    if let Some(entry) = self
                        .browser
                        .entries
                        .iter_mut()
                        .find(|entry| entry.path == path)
                    {
                        match *result {
                            Ok(image) => {
                                entry.thumbnail = Some(context.load_texture(
                                    format!("thumb-{}", path.display()),
                                    preview_image(&image),
                                    egui::TextureOptions::LINEAR,
                                ));
                            }
                            Err(error) => entry.error = Some(error),
                        }
                    }
                }
                JobResult::Thumbnail { .. } => {}
                JobResult::BatchFinished { results } => {
                    self.batch_running = false;
                    self.batch_results = results;
                    self.status = format!(
                        "Batch complete: {} succeeded, {} failed",
                        self.batch_results
                            .iter()
                            .filter(|result| result.error.is_none())
                            .count(),
                        self.batch_results
                            .iter()
                            .filter(|result| result.error.is_some())
                            .count()
                    );
                }
            }
        }
        if self.preview_due.is_some_and(|due| Instant::now() >= due) {
            self.launch_preview(context);
        }
        if self.exporting {
            context.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn run_configured_export(&mut self) {
        let Some(destination) = self.document.as_ref().and_then(|document| {
            self.export_settings
                .destination(&document.source().info.path)
        }) else {
            return;
        };
        self.run_export(
            destination,
            self.export_settings.format.into(),
            ExportOptions::default(),
            self.export_settings.plates,
        );
    }

    fn run_export(
        &mut self,
        path: PathBuf,
        format: ExportFormat,
        options: ExportOptions,
        separated: bool,
    ) {
        let Some(document) = &self.document else {
            return;
        };
        if let Err(error) = dither_io::preflight(document.source(), &path, format, options) {
            if let Some(options) = confirmed_options(&error, options) {
                self.confirm = Some(PendingExport {
                    path,
                    format,
                    options,
                    reason: error.to_string(),
                    separated,
                });
                self.status = "Export needs confirmation".into();
            } else {
                self.status = format!("Export failed: {error}");
            }
            return;
        }

        let document = document.clone();
        let sender = self.jobs.clone();
        self.export_cancel = Arc::new(AtomicBool::new(false));
        self.export_progress = Arc::new(AtomicU8::new(0));
        let cancel = self.export_cancel.clone();
        let progress = self.export_progress.clone();
        self.exporting = true;
        self.status = if separated {
            "Rendering full-resolution composite and plates…".into()
        } else {
            "Rendering full-resolution export…".into()
        };
        thread::spawn(move || {
            let exported = if separated {
                dither_io::export_with_plates_cancellable(
                    &document, &path, format, options, &cancel, &progress,
                )
            } else {
                dither_io::export_cancellable(&document, &path, format, options, &cancel, &progress)
                    .map(|()| vec![path.clone()])
            };
            let outputs = exported.as_ref().cloned().unwrap_or_default();
            let result = exported.map(|_| ());
            let _ = sender.send(JobResult::Exported {
                path,
                format,
                options,
                result,
                separated,
                outputs,
            });
        });
    }

    fn stash_active_tab(&mut self) {
        let Some(document) = self.document.take() else {
            return;
        };
        if self.tabs.len() <= self.active_tab {
            self.tabs.resize_with(self.active_tab + 1, || None);
        }
        self.tabs[self.active_tab] = Some(SavedTab {
            document,
            preview: self.preview.take(),
            original_preview: self.original_preview.take(),
            plate_previews: std::mem::take(&mut self.plate_previews),
            recipe: self.recipe.clone(),
            undo: std::mem::take(&mut self.undo),
            redo: std::mem::take(&mut self.redo),
            pending_edit: self.pending_edit.take(),
            show_original: self.show_original,
            show_comparison: self.show_comparison,
            comparison: self.comparison.take(),
            comparison_recipe: self.comparison_recipe.take(),
            scene_rect: self.scene_rect,
            plate_view: self.plate_view,
            project_path: self.project_path.take(),
            export: self.export_settings.clone(),
            dirty: self.dirty,
        });
    }

    fn switch_tab(&mut self, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() || self.tabs[index].is_none() {
            return;
        }
        self.stash_active_tab();
        let tab = self.tabs[index].take().unwrap();
        self.active_tab = index;
        self.document = Some(tab.document);
        self.preview = tab.preview;
        self.original_preview = tab.original_preview;
        self.plate_previews = tab.plate_previews;
        self.recipe = tab.recipe;
        self.undo = tab.undo;
        self.redo = tab.redo;
        self.pending_edit = tab.pending_edit;
        self.show_original = tab.show_original;
        self.show_comparison = tab.show_comparison;
        self.comparison = tab.comparison;
        self.comparison_recipe = tab.comparison_recipe;
        self.scene_rect = tab.scene_rect;
        self.plate_view = tab.plate_view;
        self.project_path = tab.project_path;
        self.export_settings = tab.export;
        self.dirty = tab.dirty;
        self.source_modified = self
            .document
            .as_ref()
            .and_then(|document| file_modified(&document.source().info.path));
    }

    fn close_active_tab(&mut self) {
        if self.tabs.len() <= 1 {
            self.document = None;
            self.preview = None;
            self.original_preview = None;
            self.plate_previews.clear();
            self.comparison = None;
            self.comparison_recipe = None;
            self.show_comparison = false;
            self.tabs.clear();
            self.tab_labels.clear();
            return;
        }
        self.tabs.remove(self.active_tab);
        self.tab_labels.remove(self.active_tab);
        let target = self.active_tab.min(self.tabs.len() - 1);
        if self.tabs[target].is_none() {
            self.active_tab = target;
            return;
        }
        let tab = self.tabs[target].take().unwrap();
        self.active_tab = target;
        self.document = Some(tab.document);
        self.preview = tab.preview;
        self.original_preview = tab.original_preview;
        self.plate_previews = tab.plate_previews;
        self.recipe = tab.recipe;
        self.undo = tab.undo;
        self.redo = tab.redo;
        self.pending_edit = tab.pending_edit;
        self.show_original = tab.show_original;
        self.show_comparison = tab.show_comparison;
        self.comparison = tab.comparison;
        self.comparison_recipe = tab.comparison_recipe;
        self.scene_rect = tab.scene_rect;
        self.plate_view = tab.plate_view;
        self.project_path = tab.project_path;
        self.export_settings = tab.export;
        self.dirty = tab.dirty;
    }

    fn duplicate_active_tab(&mut self) {
        if self.document.is_none() {
            return;
        }
        self.stash_active_tab();
        let Some(tab) = self
            .tabs
            .get(self.active_tab)
            .and_then(Option::as_ref)
            .cloned()
        else {
            return;
        };
        let label = format!("{} copy", self.tab_labels[self.active_tab]);
        self.tabs.push(Some(tab));
        self.tab_labels.push(label);
        self.switch_tab(self.tabs.len() - 1);
        self.dirty = true;
        self.project_path = None;
    }

    fn relink_source(&mut self, context: &egui::Context) {
        let Some(recipe) = self
            .document
            .as_ref()
            .map(|document| document.recipe.clone())
        else {
            return;
        };
        let extensions: Vec<_> = dither_io::supported_extensions().collect();
        if let Some(path) = FileDialog::new()
            .set_title("Relink source image")
            .add_filter("Supported images", &extensions)
            .pick_file()
        {
            self.open_path_with_recipe(path, recipe, false, context);
        }
    }

    fn set_active_tab_label(&mut self, path: &Path) {
        if self.tab_labels.len() <= self.active_tab {
            self.tab_labels
                .resize(self.active_tab + 1, "Untitled".into());
        }
        self.tab_labels[self.active_tab] = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
    }

    fn current_project(&self) -> Option<ProjectFile> {
        self.document.as_ref().map(|document| ProjectFile {
            source: document.source().info.path.clone(),
            recipe: document.recipe.clone(),
            export: self.export_settings.clone(),
            ..ProjectFile::default()
        })
    }

    fn autosave_recovery(&self) {
        if let Some(project) = self.current_project() {
            let _ = workspace::save_json(&recovery_path(), &project);
        }
    }

    fn save_persistent(&self) {
        let _ = workspace::save_json(&state_path(), &self.persistent);
    }

    fn load_folder(&mut self, folder: PathBuf, context: &egui::Context) {
        self.library_open = true;
        let mut entries = scan_folder(&folder, self.persistent.browser_sort);
        self.browser.generation = self.browser.generation.wrapping_add(1);
        let generation = self.browser.generation;
        self.browser.known_paths = entries.iter().map(|entry| entry.path.clone()).collect();
        self.browser.folder = Some(folder.clone());
        self.browser.last_scan = Instant::now();
        self.persistent.last_folder = Some(folder);
        self.save_persistent();
        let sender = self.jobs.clone();
        let repaint = context.clone();
        let paths: Vec<_> = entries.iter().map(|entry| entry.path.clone()).collect();
        self.browser.entries = std::mem::take(&mut entries);
        thread::spawn(move || {
            for path in paths {
                let result = dither_io::open(&path)
                    .map(|source| Document::new(source).render_source_preview(THUMBNAIL_SIZE))
                    .map_err(|error| error.to_string());
                let _ = sender.send(JobResult::Thumbnail {
                    generation,
                    path,
                    result: Box::new(result),
                });
                repaint.request_repaint();
            }
        });
    }

    fn navigate_folder(&mut self, delta: isize, context: &egui::Context) {
        let Some(current) = self
            .document
            .as_ref()
            .map(|document| document.source().info.path.clone())
        else {
            return;
        };
        let visible: Vec<_> = self
            .browser
            .entries
            .iter()
            .filter(|entry| browser_matches(entry, &self.browser.filter))
            .map(|entry| entry.path.clone())
            .collect();
        let Some(index) = visible.iter().position(|path| path == &current) else {
            return;
        };
        let next = (index as isize + delta).rem_euclid(visible.len() as isize) as usize;
        self.open_path(visible[next].clone(), context);
    }

    fn choose_folder(&mut self, context: &egui::Context) {
        if let Some(folder) = FileDialog::new()
            .set_title("Open image folder")
            .pick_folder()
        {
            self.load_folder(folder, context);
        }
    }

    fn save_project_as(&mut self) {
        let Some(project) = self.current_project() else {
            return;
        };
        let Some(mut path) = FileDialog::new()
            .set_title("Save Dither project")
            .set_file_name("project.dither")
            .add_filter("Dither project", &["dither"])
            .save_file()
        else {
            return;
        };
        if path.extension().is_none() {
            path.set_extension("dither");
        }
        match workspace::save_json(&path, &project) {
            Ok(()) => {
                self.project_path = Some(path.clone());
                self.dirty = false;
                self.status = format!("Saved project {}", path.display());
            }
            Err(error) => self.status = format!("Project save failed: {error}"),
        }
    }

    fn open_project(&mut self, context: &egui::Context) {
        let Some(path) = FileDialog::new()
            .set_title("Open Dither project")
            .add_filter("Dither project", &["dither"])
            .pick_file()
        else {
            return;
        };
        match workspace::load_json::<ProjectFile>(&path) {
            Ok(project) => {
                self.project_path = Some(path);
                self.open_project_data(project, self.open_in_new_tab, context);
            }
            Err(error) => self.status = format!("Project open failed: {error}"),
        }
    }

    fn reload_source(&mut self, context: &egui::Context) {
        if let Some(document) = &self.document {
            self.open_path_with_recipe(
                document.source().info.path.clone(),
                document.recipe.clone(),
                false,
                context,
            );
        }
    }

    fn run_batch(&mut self, context: &egui::Context) {
        if self.batch_running {
            return;
        }
        let paths: Vec<_> = self
            .browser
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect();
        if paths.is_empty() {
            return;
        }
        let recipe = self
            .document
            .as_ref()
            .map(|document| document.recipe.clone())
            .unwrap_or_default();
        let export = self.export_settings.clone();
        let sender = self.jobs.clone();
        let repaint = context.clone();
        self.batch_running = true;
        thread::spawn(move || {
            let results = paths
                .into_iter()
                .map(|source_path| {
                    let result = (|| {
                        let source = dither_io::open(&source_path)?;
                        let mut document = Document::new(source);
                        document.recipe = recipe.clone();
                        let asset_errors = load_recipe_assets(&mut document);
                        if !asset_errors.is_empty() {
                            return Err(IoError::InvalidImage(asset_errors.join("; ")));
                        }
                        let destination = export.destination(&source_path).ok_or_else(|| {
                            IoError::InvalidImage("source has no output directory".into())
                        })?;
                        let format: ExportFormat = export.format.into();
                        if export.plates {
                            dither_io::export_with_plates(
                                &document,
                                &destination,
                                format,
                                ExportOptions::default(),
                            )
                        } else {
                            dither_io::export(
                                &document,
                                &destination,
                                format,
                                ExportOptions::default(),
                            )
                            .map(|()| vec![destination])
                        }
                    })();
                    match result {
                        Ok(outputs) => BatchResult {
                            source: source_path,
                            outputs,
                            error: None,
                        },
                        Err(error) => BatchResult {
                            source: source_path,
                            outputs: Vec::new(),
                            error: Some(error.to_string()),
                        },
                    }
                })
                .collect();
            let _ = sender.send(JobResult::BatchFinished { results });
            repaint.request_repaint();
        });
    }

    fn record_edit(&mut self, previous: Recipe) {
        self.pending_edit.get_or_insert(previous);
        self.recipe = self.document.as_ref().unwrap().recipe.clone();
    }

    fn finish_edit(&mut self, context: &egui::Context) {
        if context.egui_is_using_pointer() {
            return;
        }
        let Some(previous) = self.pending_edit.take() else {
            return;
        };
        if self
            .document
            .as_ref()
            .is_some_and(|document| document.recipe != previous)
        {
            self.undo.push(previous);
            self.redo.clear();
            self.dirty = true;
            save_recipe(&self.recipe);
            self.autosave_recovery();
            if let (Some(path), Some(project)) = (&self.project_path, self.current_project()) {
                let _ = workspace::save_json(path, &project);
            }
        }
    }

    fn undo(&mut self, context: &egui::Context) {
        self.finish_edit(context);
        let Some(recipe) = self.undo.pop() else {
            return;
        };
        let document = self.document.as_mut().unwrap();
        self.redo.push(document.recipe.clone());
        document.recipe = recipe.clone();
        let _ = load_recipe_assets(document);
        self.recipe = recipe;
        save_recipe(&self.recipe);
        self.schedule_preview(context);
    }

    fn redo(&mut self, context: &egui::Context) {
        self.finish_edit(context);
        let Some(recipe) = self.redo.pop() else {
            return;
        };
        let document = self.document.as_mut().unwrap();
        self.undo.push(document.recipe.clone());
        document.recipe = recipe.clone();
        let _ = load_recipe_assets(document);
        self.recipe = recipe;
        save_recipe(&self.recipe);
        self.schedule_preview(context);
    }

    fn controls(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(document) = &mut self.document else {
            ui.add_space(16.0);
            ui.label(RichText::new("NO DOCUMENT").color(Color32::from_gray(125)));
            return false;
        };
        let mut changed = false;

        ui.horizontal(|ui| {
            ui.label(section_label("DITHER"));
            if ui.small_button("Reset all").clicked() {
                document.recipe = Recipe::default();
                let _ = load_recipe_assets(document);
                changed = true;
            }
        });
        changed |= algorithm_selector(ui, &mut document.recipe.dither.algorithm);
        changed |= resampling_selector(ui, &mut document.recipe.resampling);
        changed |= slider(
            ui,
            "Diffusion strength",
            &mut document.recipe.dither.strength,
            0.0..=1.0,
        );
        changed |= variation(ui, "Blue-noise seed", &mut document.recipe.dither.seed);
        control_row(ui, "Preset", |ui| {
            egui::ComboBox::from_id_salt("recipe-preset")
                .width(ui.available_width())
                .selected_text("Choose preset…")
                .show_ui(ui, |ui| {
                    for (name, recipe) in built_in_presets() {
                        if ui.selectable_label(false, *name).clicked() {
                            document.recipe = recipe.clone();
                            let _ = load_recipe_assets(document);
                            changed = true;
                        }
                    }
                });
        });

        ui.add_space(18.0);
        ui.label(section_label("PRE-DITHER"));
        changed |= slider(
            ui,
            "Brightness",
            &mut document.recipe.preprocess.brightness,
            -1.0..=1.0,
        );
        changed |= slider(
            ui,
            "Contrast",
            &mut document.recipe.preprocess.contrast,
            0.0..=3.0,
        );
        changed |= slider(
            ui,
            "Gamma",
            &mut document.recipe.preprocess.gamma,
            0.1..=4.0,
        );
        changed |= slider(
            ui,
            "Blur",
            &mut document.recipe.preprocess.blur_radius,
            0.0..=32.0,
        );
        changed |= slider(
            ui,
            "Sharpen",
            &mut document.recipe.preprocess.sharpen,
            0.0..=3.0,
        );
        changed |= slider(
            ui,
            "Black point",
            &mut document.recipe.preprocess.black_point,
            0.0..=0.99,
        );
        changed |= slider(
            ui,
            "White point",
            &mut document.recipe.preprocess.white_point,
            0.01..=1.5,
        );
        changed |= slider(
            ui,
            "Denoise",
            &mut document.recipe.preprocess.denoise,
            0.0..=1.0,
        );
        changed |= ui
            .checkbox(&mut document.recipe.preprocess.invert, "Invert")
            .changed();

        ui.add_space(18.0);
        ui.label(section_label("COLOR / PLATES"));
        changed |= mode_selector(ui, &mut document.recipe.separation);
        ui.add_space(8.0);

        let mut extract_palette = false;
        let indexed_mode = matches!(document.recipe.separation, Separation::Indexed(_));
        match &mut document.recipe.separation {
            Separation::Monochrome(settings) => {
                changed |= slider(ui, "Threshold", &mut settings.threshold, 0.0..=1.0);
                changed |= slider(ui, "Tonal width", &mut settings.softness, 0.01..=1.0);
                changed |= ink(ui, "Ink", &mut settings.ink);
            }
            Separation::ThreeColor(settings) | Separation::Rgb(settings) => {
                changed |= slider(ui, "Threshold", &mut settings.threshold, 0.0..=1.0);
                changed |= slider(ui, "Tonal width", &mut settings.softness, 0.01..=1.0);
                changed |= ink(ui, "Plate 1", &mut settings.cyan);
                changed |= ink(ui, "Plate 2", &mut settings.magenta);
                changed |= ink(ui, "Plate 3", &mut settings.yellow);
            }
            Separation::Cmyk(settings) => {
                changed |= ink(ui, "Cyan", &mut settings.cyan);
                changed |= ink(ui, "Magenta", &mut settings.magenta);
                changed |= ink(ui, "Yellow", &mut settings.yellow);
                changed |= ink(ui, "Black", &mut settings.black);
            }
            Separation::TriTone(settings) => {
                changed |= tone_band(ui, "Shadows", &mut settings.shadows);
                changed |= tone_band(ui, "Midtones", &mut settings.midtones);
                changed |= tone_band(ui, "Highlights", &mut settings.highlights);
            }
            Separation::Tonal(settings)
            | Separation::Indexed(settings)
            | Separation::Custom(settings) => {
                changed |= palette_controls(ui, settings);
                if indexed_mode && ui.button("Extract palette from image").clicked() {
                    extract_palette = true;
                }
            }
        }
        if extract_palette {
            let size = match &document.recipe.separation {
                Separation::Indexed(settings) => settings.size,
                _ => 8,
            };
            let colors = document.extract_palette(size);
            if let Separation::Indexed(settings) = &mut document.recipe.separation {
                settings.colors = colors;
                sync_palette_inks(settings);
            }
            changed = true;
        }

        ui.add_space(18.0);
        ui.label(section_label("PRINT"));
        changed |= slider(ui, "DPI", &mut document.recipe.print.dpi, 36.0..=2400.0);
        changed |= slider(ui, "LPI", &mut document.recipe.print.lpi, 5.0..=300.0);
        changed |= control_row(ui, "Bleed px", |ui| {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Slider::new(&mut document.recipe.print.bleed_pixels, 0..=16),
            )
        })
        .changed();
        changed |= control_row(ui, "Trapping px", |ui| {
            ui.add_sized(
                [ui.available_width(), 18.0],
                egui::Slider::new(&mut document.recipe.print.trapping_pixels, 0..=16),
            )
        })
        .changed();

        ui.add_space(18.0);
        ui.label(section_label("TRI-TONE TEXTURES / DISPLACE"));
        let (asset_changed, asset_error) = asset_control(
            ui,
            document,
            AssetKind::PaperTexture,
            "Paper / photocopy texture",
        );
        changed |= asset_changed;
        if let Some(error) = asset_error {
            self.status = format!("Texture import failed: {error}");
        }
        let (asset_changed, asset_error) =
            asset_control(ui, document, AssetKind::DisplacementMap, "Displacement map");
        changed |= asset_changed;
        if let Some(error) = asset_error {
            self.status = format!("Displacement import failed: {error}");
        }
        let (asset_changed, asset_error) = asset_control(
            ui,
            document,
            AssetKind::DistressMask,
            "Distress texture / brush mask",
        );
        changed |= asset_changed;
        if let Some(error) = asset_error {
            self.status = format!("Distress import failed: {error}");
        }
        changed |= ui
            .checkbox(
                &mut document.recipe.displacement.enabled,
                "Enable displacement",
            )
            .changed();
        changed |= map_pattern_selector(ui, &mut document.recipe.displacement.pattern);
        changed |= slider(
            ui,
            "Map scale",
            &mut document.recipe.displacement.pattern_scale,
            2.0..=256.0,
        );
        changed |=
            randomizable_variation(ui, "Map variation", &mut document.recipe.displacement.seed);
        changed |= slider(
            ui,
            "X strength",
            &mut document.recipe.displacement.x_strength,
            -128.0..=128.0,
        );
        changed |= slider(
            ui,
            "Y strength",
            &mut document.recipe.displacement.y_strength,
            -128.0..=128.0,
        );
        changed |= slider(
            ui,
            "Distress",
            &mut document.recipe.displacement.distress_amount,
            0.0..=1.0,
        );

        ui.add_space(18.0);
        ui.label(section_label("HIGHLIGHT GLOW"));
        changed |= ui
            .checkbox(&mut document.recipe.glow.enabled, "Enable glow")
            .changed();
        changed |= slider(
            ui,
            "Threshold",
            &mut document.recipe.glow.threshold,
            0.0..=1.0,
        );
        changed |= slider(ui, "Radius", &mut document.recipe.glow.radius, 0.0..=64.0);
        changed |= slider(ui, "Falloff", &mut document.recipe.glow.falloff, 1.0..=4.0);
        changed |= slider(
            ui,
            "Intensity",
            &mut document.recipe.glow.intensity,
            0.0..=4.0,
        );
        changed |= slider(ui, "Gamma", &mut document.recipe.glow.gamma, 0.1..=4.0);
        changed |= slider(
            ui,
            "Saturation",
            &mut document.recipe.glow.saturation,
            0.0..=3.0,
        );
        control_row(ui, "Tint", |ui| {
            changed |= ui
                .color_edit_button_rgb(&mut document.recipe.glow.tint)
                .changed();
        });

        ui.add_space(18.0);
        ui.label(section_label("CRT PHASER"));
        changed |= ui
            .checkbox(&mut document.recipe.crt.enabled, "Enable CRT")
            .changed();
        changed |= crt_phase_selector(ui, &mut document.recipe.crt.phase);
        changed |= slider(
            ui,
            "Wave strength",
            &mut document.recipe.crt.wave_strength,
            0.0..=128.0,
        );
        changed |= slider(
            ui,
            "Wave frequency",
            &mut document.recipe.crt.wave_frequency,
            0.1..=64.0,
        );
        changed |= slider(
            ui,
            "Scanlines",
            &mut document.recipe.crt.scanlines,
            0.0..=1.0,
        );
        changed |= slider(
            ui,
            "RGB bleed",
            &mut document.recipe.crt.rgb_bleed,
            0.0..=24.0,
        );
        changed |= slider(
            ui,
            "Sync tearing",
            &mut document.recipe.crt.sync_tearing,
            0.0..=128.0,
        );
        changed |= slider(
            ui,
            "Phosphor mask",
            &mut document.recipe.crt.phosphor_mask,
            0.0..=1.0,
        );
        changed |= slider(ui, "Bloom", &mut document.recipe.crt.bloom, 0.0..=2.0);

        ui.add_space(18.0);
        ui.label(section_label("SURFACE"));
        changed |= slider(ui, "Grain", &mut document.recipe.grain.amount, 0.0..=0.8);
        changed |= slider(
            ui,
            "Grain scale",
            &mut document.recipe.grain.scale,
            0.25..=12.0,
        );
        changed |= randomizable_variation(ui, "Grain variation", &mut document.recipe.grain.seed);
        changed |= slider(ui, "Paper", &mut document.recipe.paper.amount, 0.0..=0.5);
        changed |= slider(
            ui,
            "Paper scale",
            &mut document.recipe.paper.scale,
            0.5..=24.0,
        );
        changed |= variation(ui, "Paper variation", &mut document.recipe.paper.seed);
        control_row(ui, "Paper tone", |ui| {
            changed |= ui
                .color_edit_button_rgb(&mut document.recipe.paper_color)
                .changed();
        });
        changed
    }
}

impl eframe::App for Editor {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let context = ui.ctx().clone();
        #[cfg(target_os = "macos")]
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id.as_ref() {
                "open" => self.open(&context),
                "open-new-tab" => {
                    self.open_in_new_tab = true;
                    self.open(&context);
                    self.open_in_new_tab = false;
                }
                "open-project" => self.open_project(&context),
                "save-project" => self.save_project_as(),
                "reload-source" => self.reload_source(&context),
                "relink-source" => self.relink_source(&context),
                "export" => self.inspector_tab = InspectorTab::Output,
                "undo" => self.undo(&context),
                "redo" => self.redo(&context),
                "duplicate-tab" => self.duplicate_active_tab(),
                "close-tab" => self.close_active_tab(),
                "toggle-files" => self.library_open = !self.library_open,
                "view-effect" => {
                    self.show_original = false;
                    self.show_comparison = false;
                }
                "view-original" => {
                    self.show_original = true;
                    self.show_comparison = false;
                }
                "view-snapshot" if self.comparison.is_some() => {
                    self.show_original = false;
                    self.show_comparison = true;
                }
                "view-fit" => self.scene_rect = egui::Rect::ZERO,
                "view-actual-size" => {
                    self.scene_rect =
                        egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::splat(1.0));
                }
                "capture-snapshot" => {
                    self.comparison = self.preview.clone();
                    self.comparison_recipe = self
                        .document
                        .as_ref()
                        .map(|document| document.recipe.clone());
                    self.show_comparison = false;
                    self.status = "Comparison snapshot captured".into();
                }
                _ => {}
            }
        }
        self.process_jobs(&context);
        if let Some(path) = context.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .find_map(|file| file.path.clone())
        }) {
            self.open_path(path, &context);
        }
        if context.input_mut(|input| {
            input.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND,
                egui::Key::O,
            ))
        }) {
            self.open(&context);
        }
        if self.document.is_some()
            && !self.exporting
            && context.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::S,
                ))
            })
        {
            self.inspector_tab = InspectorTab::Output;
        }
        if self.document.is_some()
            && context.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers {
                        command: true,
                        shift: true,
                        ..Default::default()
                    },
                    egui::Key::Z,
                ))
            })
        {
            self.redo(&context);
        } else if self.document.is_some()
            && context.input_mut(|input| {
                input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Z,
                ))
            })
        {
            self.undo(&context);
        }

        if self.last_source_check.elapsed() >= Duration::from_secs(1) {
            self.last_source_check = Instant::now();
            if let Some(document) = &self.document {
                let modified = file_modified(&document.source().info.path);
                if self.source_modified.is_some() && modified > self.source_modified {
                    self.status = "Source changed on disk — use Reload Source to update".into();
                }
            }
            if self.browser.watch
                && self.browser.last_scan.elapsed() >= Duration::from_secs(2)
                && let Some(folder) = self.browser.folder.clone()
            {
                let current: HashSet<_> = scan_folder(&folder, self.persistent.browser_sort)
                    .into_iter()
                    .map(|entry| entry.path)
                    .collect();
                if current != self.browser.known_paths {
                    self.load_folder(folder, &context);
                }
            }
        }

        if !self.tabs.is_empty() {
            let mut switch_to = None;
            let mut close_tab = None;
            let mut open_new_tab = false;
            egui::Panel::top("document-tabs")
                .exact_size(32.0)
                .frame(egui::Frame::new().fill(Color32::from_rgb(20, 20, 18)))
                .show(ui, |ui| {
                    ui.spacing_mut().button_padding.y = 5.0;
                    egui::ScrollArea::horizontal()
                        .id_salt("document-tabs-scroll")
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for (index, label) in self.tab_labels.iter().enumerate() {
                                    let selected = index == self.active_tab;
                                    egui::Frame::new()
                                        .fill(if selected {
                                            Color32::from_rgb(103, 31, 25)
                                        } else {
                                            Color32::TRANSPARENT
                                        })
                                        .inner_margin(egui::Margin::symmetric(8, 2))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                if ui.selectable_label(selected, label).clicked() {
                                                    switch_to = Some(index);
                                                }
                                                if ui.small_button("×").clicked() {
                                                    close_tab = Some(index);
                                                }
                                            });
                                        });
                                }
                                if ui
                                    .small_button("+")
                                    .on_hover_text("Open in new tab")
                                    .clicked()
                                {
                                    open_new_tab = true;
                                }
                            });
                        });
                });
            if let Some(index) = switch_to {
                self.switch_tab(index);
            }
            if let Some(index) = close_tab {
                if index != self.active_tab {
                    self.switch_tab(index);
                }
                self.close_active_tab();
            }
            if open_new_tab {
                self.open_in_new_tab = true;
                self.open(&context);
                self.open_in_new_tab = false;
            }
        }

        let mut browser_open = None;
        let mut browser_folder = None;
        let mut library_open = self.library_open;
        egui::Panel::left("library")
            .default_size(220.0)
            .min_size(176.0)
            .max_size(300.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(12.0)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show_collapsible(ui, &mut library_open, |ui| {
                ui.horizontal(|ui| {
                    ui.label(section_label("LIBRARY"));
                    if ui.small_button("Open folder…").clicked() {
                        browser_folder = Some(());
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.browser.filter)
                        .hint_text("Filter files")
                        .desired_width(f32::INFINITY),
                );
                control_row(ui, "Sort", |ui| {
                    egui::ComboBox::from_id_salt("browser-sort")
                        .width(ui.available_width())
                        .selected_text(format!("{:?}", self.persistent.browser_sort))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.persistent.browser_sort,
                                BrowserSort::Name,
                                "Name",
                            );
                            ui.selectable_value(
                                &mut self.persistent.browser_sort,
                                BrowserSort::Modified,
                                "Modified",
                            );
                            ui.selectable_value(
                                &mut self.persistent.browser_sort,
                                BrowserSort::FileType,
                                "Type",
                            );
                        });
                });
                ui.checkbox(&mut self.browser.watch, "Watch folder");
                if ui.small_button("★ Favorite current folder").clicked()
                    && let Some(folder) = self.browser.folder.clone()
                    && !self.persistent.favorite_folders.contains(&folder)
                {
                    self.persistent.favorite_folders.push(folder);
                }
                for folder in &self.persistent.favorite_folders {
                    if ui
                        .small_button(format!(
                            "★ {}",
                            folder.file_name().unwrap_or_default().to_string_lossy()
                        ))
                        .clicked()
                    {
                        browser_open = Some(
                            isize::MAX
                                - self
                                    .persistent
                                    .favorite_folders
                                    .iter()
                                    .position(|item| item == folder)
                                    .unwrap_or(0) as isize,
                        );
                    }
                }
                ui.checkbox(
                    &mut self.preserve_recipe_on_replace,
                    "Preserve recipe when replacing",
                );
                ui.horizontal(|ui| {
                    if ui.button("◀ Previous").clicked() {
                        browser_open = Some(-1_isize);
                    }
                    if ui.button("Next ▶").clicked() {
                        browser_open = Some(1_isize);
                    }
                });
                egui::CollapsingHeader::new("Recent files").show(ui, |ui| {
                    for path in &self.persistent.recent_files {
                        if ui
                            .small_button(path.file_name().unwrap_or_default().to_string_lossy())
                            .clicked()
                        {
                            browser_open = Some(
                                isize::MIN
                                    + self
                                        .persistent
                                        .recent_files
                                        .iter()
                                        .position(|item| item == path)
                                        .unwrap_or(0)
                                        as isize,
                            );
                        }
                    }
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for entry in self
                        .browser
                        .entries
                        .iter()
                        .filter(|entry| browser_matches(entry, &self.browser.filter))
                    {
                        ui.horizontal(|ui| {
                            if let Some(texture) = &entry.thumbnail {
                                ui.add(
                                    egui::Image::new(texture).fit_to_exact_size(Vec2::splat(52.0)),
                                );
                            } else {
                                ui.allocate_space(Vec2::splat(52.0));
                            }
                            if ui
                                .selectable_label(
                                    self.document.as_ref().is_some_and(|document| {
                                        document.source().info.path == entry.path
                                    }),
                                    entry.path.file_name().unwrap_or_default().to_string_lossy(),
                                )
                                .clicked()
                            {
                                browser_open = Some(
                                    self.browser
                                        .entries
                                        .iter()
                                        .position(|item| item.path == entry.path)
                                        .unwrap_or(0) as isize
                                        + 10_000,
                                );
                            }
                        });
                    }
                });
            });
        self.library_open = library_open;
        if browser_folder.is_some() {
            self.choose_folder(&context);
        }
        if let Some(action) = browser_open {
            if action == -1 || action == 1 {
                self.navigate_folder(action, &context);
            } else if action >= isize::MAX - 100 {
                let index = (isize::MAX - action) as usize;
                if let Some(folder) = self.persistent.favorite_folders.get(index).cloned() {
                    self.load_folder(folder, &context);
                }
            } else if action >= 10_000 {
                if let Some(entry) = self.browser.entries.get((action - 10_000) as usize) {
                    self.open_path(entry.path.clone(), &context);
                }
            } else if action <= isize::MIN + 100 {
                let index = (action - isize::MIN) as usize;
                if let Some(path) = self.persistent.recent_files.get(index).cloned() {
                    self.open_path(path, &context);
                }
            }
        }
        egui::Panel::bottom("status")
            .exact_size(24.0)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(18, 18, 16))
                    .inner_margin(egui::Margin::symmetric(10, 3))
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status).small().color(MUTED));
                    if self.dirty {
                        ui.label(RichText::new("UNSAVED").small().color(ACCENT));
                    }
                });
            });

        egui::Panel::right("controls")
            .default_size(340.0)
            .min_size(300.0)
            .max_size(440.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL_RAISED)
                    .inner_margin(14.0)
                    .stroke(Stroke::new(1.0, BORDER)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.inspector_tab, InspectorTab::Adjust, "Adjust");
                    ui.selectable_value(
                        &mut self.inspector_tab,
                        InspectorTab::Plates,
                        format!("Plates  {:02}", self.plate_previews.len()),
                    );
                    ui.selectable_value(&mut self.inspector_tab, InspectorTab::Output, "Output");
                });
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if self.inspector_tab == InspectorTab::Plates {
                        ui.label(section_label("⊕ PLATE DESK"));
                        egui::CollapsingHeader::new("Plate inspector").show(ui, |ui| {
                            ui.selectable_value(
                                &mut self.plate_view,
                                PlateView::Composite,
                                "Composite",
                            );
                            for (index, plate) in self.plate_previews.iter().enumerate() {
                                ui.horizontal(|ui| {
                                    let (rect, _) = ui.allocate_exact_size(
                                        Vec2::splat(10.0),
                                        egui::Sense::hover(),
                                    );
                                    ui.painter().rect_filled(rect, 1.0, plate.color);
                                    ui.label(&plate.name);
                                    ui.selectable_value(
                                        &mut self.plate_view,
                                        PlateView::Grayscale(index),
                                        "Mask",
                                    );
                                    ui.selectable_value(
                                        &mut self.plate_view,
                                        PlateView::Inked(index),
                                        "Ink",
                                    );
                                });
                            }
                        });
                        egui::CollapsingHeader::new("Image information").show(ui, |ui| {
                            if let Some(document) = &self.document {
                                let source = document.source();
                                ui.label(format!(
                                    "{} × {} pixels",
                                    source.width(),
                                    source.height()
                                ));
                                ui.label(format!(
                                    "{}-bit {}",
                                    source.info.bit_depth, source.info.format
                                ));
                                ui.label(format!(
                                    "ICC profile: {} bytes",
                                    source.info.color_profile.len()
                                ));
                                ui.label(format!(
                                    "EXIF {} · XMP {} · IPTC {} bytes",
                                    source.info.metadata.exif.len(),
                                    source.info.metadata.xmp.len(),
                                    source.info.metadata.iptc.len()
                                ));
                                ui.label(format!(
                                    "Print: {:.2} × {:.2} in at {:.0} DPI",
                                    source.width() as f32 / document.recipe.print.dpi,
                                    source.height() as f32 / document.recipe.print.dpi,
                                    document.recipe.print.dpi
                                ));
                                let mut histogram = [0_u32; 32];
                                for pixel in source
                                    .pixels()
                                    .iter()
                                    .step_by((source.pixels().len() / 100_000).max(1))
                                {
                                    let value =
                                        (pixel[0] * 0.2126 + pixel[1] * 0.7152 + pixel[2] * 0.0722)
                                            .clamp(0.0, 1.0);
                                    histogram[(value * 31.0).round() as usize] += 1;
                                }
                                let points: Vec<_> = histogram
                                    .iter()
                                    .enumerate()
                                    .map(|(x, y)| {
                                        egui::Pos2::new(
                                            x as f32 * 7.0,
                                            50.0 - *y as f32
                                                / *histogram.iter().max().unwrap_or(&1) as f32
                                                * 50.0,
                                        )
                                    })
                                    .collect();
                                ui.painter().add(egui::Shape::line(
                                    points,
                                    Stroke::new(1.0, Color32::LIGHT_GRAY),
                                ));
                                ui.allocate_space(Vec2::new(220.0, 55.0));
                            }
                        });
                    }
                    if self.inspector_tab == InspectorTab::Output {
                        ui.label(section_label("OUTPUT"));
                        egui::CollapsingHeader::new("Export manager")
                            .default_open(true)
                            .show(ui, |ui| {
                                control_row(ui, "Name", |ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(
                                            &mut self.export_settings.naming,
                                        )
                                        .desired_width(ui.available_width()),
                                    );
                                });
                                control_row(ui, "Format", |ui| {
                                    egui::ComboBox::from_id_salt("export-format")
                                        .width(ui.available_width())
                                        .selected_text(format!("{:?}", self.export_settings.format))
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.export_settings.format,
                                                SavedExportFormat::Png16,
                                                "PNG 16-bit",
                                            );
                                            ui.selectable_value(
                                                &mut self.export_settings.format,
                                                SavedExportFormat::Tiff16,
                                                "TIFF 16-bit",
                                            );
                                            ui.selectable_value(
                                                &mut self.export_settings.format,
                                                SavedExportFormat::OpenExr32,
                                                "OpenEXR 32-bit",
                                            );
                                        });
                                });
                                ui.checkbox(&mut self.export_settings.plates, "Composite + plates");
                                if ui.button("Choose output folder…").clicked() {
                                    self.export_settings.directory =
                                        FileDialog::new().pick_folder();
                                }
                                if let Some(document) = &self.document
                                    && let Some(destination) = self
                                        .export_settings
                                        .destination(&document.source().info.path)
                                {
                                    ui.label(format!("Will create: {}", destination.display()));
                                    for plate in &self.plate_previews {
                                        if self.export_settings.plates {
                                            ui.label(format!("  + plate {}.png", plate.name));
                                        }
                                    }
                                    if ui
                                        .add_enabled(
                                            !self.exporting,
                                            egui::Button::new("Export full-resolution")
                                                .fill(ACCENT),
                                        )
                                        .clicked()
                                    {
                                        self.run_configured_export();
                                    }
                                }
                                if self.exporting {
                                    ui.add(egui::ProgressBar::new(
                                        self.export_progress.load(Ordering::Relaxed) as f32 / 100.0,
                                    ));
                                    if ui.button("Cancel export").clicked() {
                                        self.export_cancel.store(true, Ordering::Relaxed);
                                    }
                                }
                                if ui.button("Export history").clicked() {
                                    self.export_history_open = !self.export_history_open;
                                }
                                if self.export_history_open {
                                    for record in &self.persistent.export_history {
                                        ui.horizontal(|ui| {
                                            ui.label(format!(
                                                "{} → {} file(s)",
                                                record.source.display(),
                                                record.outputs.len()
                                            ));
                                            if ui.small_button("Reveal").clicked()
                                                && let Some(path) = record.outputs.first()
                                            {
                                                reveal_path(path);
                                            }
                                        });
                                    }
                                }
                            });
                        egui::CollapsingHeader::new("Batch / watched folder").show(ui, |ui| {
                            if ui
                                .add_enabled(
                                    !self.batch_running,
                                    egui::Button::new("Export current folder"),
                                )
                                .clicked()
                            {
                                self.run_batch(&context);
                            }
                            for result in &self.batch_results {
                                ui.label(format!(
                                    "{} — {}",
                                    result
                                        .source
                                        .file_name()
                                        .unwrap_or_default()
                                        .to_string_lossy(),
                                    result.error.as_deref().map_or_else(
                                        || format!("{} file(s)", result.outputs.len()),
                                        str::to_owned,
                                    )
                                ));
                            }
                        });
                    }
                    if self.inspector_tab == InspectorTab::Adjust {
                        ui.label(section_label("RECIPE"));
                        ui.horizontal(|ui| {
                            if ui.button("Save recipe…").clicked()
                                && let Some(recipe) = self.document.as_ref().map(|d| &d.recipe)
                            {
                                match save_recipe_as(recipe) {
                                    Ok(Some(path)) => {
                                        self.status = format!("Saved recipe {}", path.display())
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        self.status = format!("Recipe save failed: {error}")
                                    }
                                }
                            }
                            if ui.button("Load recipe…").clicked() {
                                match load_recipe_as() {
                                    Ok(Some(recipe)) => {
                                        if let Some(document) = &mut self.document {
                                            self.undo.push(document.recipe.clone());
                                            document.recipe = recipe.clone();
                                            let errors = load_recipe_assets(document);
                                            self.status = if errors.is_empty() {
                                                "Loaded recipe".into()
                                            } else {
                                                format!(
                                                    "Loaded recipe; {} asset(s) unavailable",
                                                    errors.len()
                                                )
                                            };
                                        }
                                        self.recipe = recipe;
                                        self.redo.clear();
                                        self.schedule_preview(&context);
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        self.status = format!("Recipe load failed: {error}")
                                    }
                                }
                            }
                        });
                        ui.add_space(18.0);
                        let previous = self
                            .document
                            .as_ref()
                            .map(|document| document.recipe.clone());
                        let changed = self.controls(ui);
                        if let (true, Some(previous)) = (changed, previous) {
                            self.record_edit(previous);
                            self.schedule_preview(&context);
                        }
                    }
                });
            });
        self.finish_edit(&context);

        egui::CentralPanel::default_margins()
            .frame(egui::Frame::new().fill(CANVAS).inner_margin(28.0))
            .show(ui, |ui| {
                let texture = if self.show_comparison {
                    self.comparison.clone().or_else(|| self.preview.clone())
                } else if self.show_original {
                    self.original_preview.clone()
                } else {
                    match self.plate_view {
                        PlateView::Composite => self.preview.clone(),
                        PlateView::Grayscale(index) => self
                            .plate_previews
                            .get(index)
                            .map(|plate| plate.grayscale.clone()),
                        PlateView::Inked(index) => self
                            .plate_previews
                            .get(index)
                            .map(|plate| plate.inked.clone()),
                    }
                    .or_else(|| self.original_preview.clone())
                };
                if let Some(texture) = texture {
                    let size = texture.size_vec2();
                    egui::Scene::new()
                        .zoom_range(0.05..=16.0)
                        .max_inner_size(size)
                        .show(ui, &mut self.scene_rect, |ui| {
                            egui::Frame::new()
                                .shadow(egui::epaint::Shadow {
                                    offset: [0, 8],
                                    blur: 28,
                                    spread: 0,
                                    color: Color32::from_black_alpha(180),
                                })
                                .show(ui, |ui| {
                                    ui.add(egui::Image::new(&texture).fit_to_exact_size(size));
                                });
                        });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("⊕  A PHOTOCOPY DARKROOM")
                                    .size(28.0)
                                    .family(FontFamily::Name(Arc::from("plex-heading")))
                                    .color(PAPER),
                            );
                            ui.label(
                                RichText::new("Non-destructive · high bit depth · native export")
                                    .color(MUTED),
                            );
                            ui.add_space(16.0);
                            if ui.button("Choose an original").clicked() {
                                self.open(&context);
                            }
                        });
                    });
                }
            });

        if let Some(path) = self.pending_open.clone() {
            egui::Window::new("Unsaved project")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(&context, |ui| {
                    ui.label("The current document has unsaved changes.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            self.pending_open = None;
                        }
                        if ui.button("Save project").clicked() {
                            self.save_project_as();
                            if !self.dirty {
                                self.pending_open = None;
                                self.open_path(path.clone(), &context);
                            }
                        }
                        if ui.button("Discard and replace").clicked() {
                            self.dirty = false;
                            self.pending_open = None;
                            self.open_path(path.clone(), &context);
                        }
                    });
                });
        }

        if let Some(pending) = self.confirm.take() {
            let mut keep = true;
            let mut proceed = false;
            egui::Window::new("Confirm export")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(&context, |ui| {
                    ui.label(&pending.reason);
                    ui.label("Continuing grants permission for this warning only.");
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                        if ui.button("Export anyway").clicked() {
                            proceed = true;
                            keep = false;
                        }
                    });
                });
            if proceed {
                self.run_export(
                    pending.path,
                    pending.format,
                    pending.options,
                    pending.separated,
                );
            } else if keep {
                self.confirm = Some(pending);
            }
        }
    }

    fn on_exit(&mut self) {
        save_recipe(&self.recipe);
        self.persistent.default_export = self.export_settings.clone();
        self.save_persistent();
        if self.dirty {
            self.autosave_recovery();
        } else {
            let _ = fs::remove_file(recovery_path());
        }
    }
}

fn recipe_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    return std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Application Support/Dither/recipe.json"));

    #[cfg(target_os = "windows")]
    return std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .map(|directory| directory.join("Dither/recipe.json"));

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        })
        .map(|directory| directory.join("dither/recipe.json"))
}

fn load_recipe() -> Recipe {
    recipe_path()
        .and_then(|path| fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save_recipe(recipe: &Recipe) {
    let Some(path) = recipe_path() else {
        return;
    };
    let Some(parent) = path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_ok()
        && let Ok(bytes) = serde_json::to_vec(recipe)
    {
        let _ = fs::write(path, bytes);
    }
}

fn save_recipe_as(recipe: &Recipe) -> Result<Option<PathBuf>, String> {
    let Some(mut path) = FileDialog::new()
        .set_title("Save Dither recipe")
        .set_file_name("dither-recipe.json")
        .add_filter("Dither recipe", &["json"])
        .save_file()
    else {
        return Ok(None);
    };
    if path.extension().is_none() {
        path.set_extension("json");
    }
    let bytes = serde_json::to_vec_pretty(recipe).map_err(|error| error.to_string())?;
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(Some(path))
}

fn load_recipe_as() -> Result<Option<Recipe>, String> {
    let Some(path) = FileDialog::new()
        .set_title("Load Dither recipe")
        .add_filter("Dither recipe", &["json"])
        .pick_file()
    else {
        return Ok(None);
    };
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn load_recipe_assets(document: &mut Document) -> Vec<String> {
    let assets = [
        (
            AssetKind::PaperTexture,
            document.recipe.assets.paper_texture.clone(),
        ),
        (
            AssetKind::DisplacementMap,
            document.recipe.assets.displacement_map.clone(),
        ),
        (
            AssetKind::DistressMask,
            document.recipe.assets.distress_mask.clone(),
        ),
    ];
    let mut errors = Vec::new();
    for (kind, path) in assets {
        document.clear_asset(kind);
        if let Some(path) = path {
            match dither_io::open(&path) {
                Ok(image) => document.set_asset(kind, image),
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
    }
    errors
}

fn algorithm_selector(ui: &mut egui::Ui, algorithm: &mut DitherAlgorithm) -> bool {
    let previous = *algorithm;
    control_row(ui, "Algorithm", |ui| {
        egui::ComboBox::from_id_salt("algorithm")
            .width(ui.available_width())
            .selected_text(algorithm_name(*algorithm))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::Bayer { matrix_size: 2 },
                    "Bayer 2×2",
                );
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::Bayer { matrix_size: 4 },
                    "Bayer 4×4",
                );
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::Bayer { matrix_size: 8 },
                    "Bayer 8×8",
                );
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::FloydSteinberg,
                    "Floyd–Steinberg",
                );
                ui.selectable_value(algorithm, DitherAlgorithm::Atkinson, "Atkinson");
                ui.selectable_value(algorithm, DitherAlgorithm::SierraLite, "Sierra Lite");
                ui.selectable_value(algorithm, DitherAlgorithm::SierraTwoRow, "Sierra two-row");
                ui.selectable_value(algorithm, DitherAlgorithm::Sierra, "Sierra");
                ui.selectable_value(algorithm, DitherAlgorithm::Stucki, "Stucki");
                ui.selectable_value(algorithm, DitherAlgorithm::Burkes, "Burkes");
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::JarvisJudiceNinke,
                    "Jarvis–Judice–Ninke",
                );
                ui.selectable_value(algorithm, DitherAlgorithm::BlueNoise, "Blue noise");
                ui.selectable_value(algorithm, DitherAlgorithm::Modulation, "Modulation");
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::Halftone {
                        shape: HalftoneShape::Dot,
                    },
                    "Dot halftone",
                );
                ui.selectable_value(
                    algorithm,
                    DitherAlgorithm::Halftone {
                        shape: HalftoneShape::Line,
                    },
                    "Line halftone",
                );
                for (shape, name) in [
                    (HalftoneShape::Cross, "Cross halftone"),
                    (HalftoneShape::Diamond, "Diamond halftone"),
                    (HalftoneShape::ClusteredDot, "Clustered-dot halftone"),
                ] {
                    ui.selectable_value(algorithm, DitherAlgorithm::Halftone { shape }, name);
                }
            });
    });
    *algorithm != previous
}

fn algorithm_name(algorithm: DitherAlgorithm) -> &'static str {
    match algorithm {
        DitherAlgorithm::Bayer { matrix_size: 2 } => "Bayer 2×2",
        DitherAlgorithm::Bayer { matrix_size: 4 } => "Bayer 4×4",
        DitherAlgorithm::Bayer { .. } => "Bayer 8×8",
        DitherAlgorithm::FloydSteinberg => "Floyd–Steinberg",
        DitherAlgorithm::Atkinson => "Atkinson",
        DitherAlgorithm::SierraLite => "Sierra Lite",
        DitherAlgorithm::SierraTwoRow => "Sierra two-row",
        DitherAlgorithm::Sierra => "Sierra",
        DitherAlgorithm::Stucki => "Stucki",
        DitherAlgorithm::Burkes => "Burkes",
        DitherAlgorithm::JarvisJudiceNinke => "Jarvis–Judice–Ninke",
        DitherAlgorithm::BlueNoise => "Blue noise",
        DitherAlgorithm::Modulation => "Modulation",
        DitherAlgorithm::Halftone {
            shape: HalftoneShape::Dot,
        } => "Dot halftone",
        DitherAlgorithm::Halftone {
            shape: HalftoneShape::Line,
        } => "Line halftone",
        DitherAlgorithm::Halftone {
            shape: HalftoneShape::Cross,
        } => "Cross halftone",
        DitherAlgorithm::Halftone {
            shape: HalftoneShape::Diamond,
        } => "Diamond halftone",
        DitherAlgorithm::Halftone {
            shape: HalftoneShape::ClusteredDot,
        } => "Clustered-dot halftone",
    }
}

fn resampling_selector(ui: &mut egui::Ui, resampling: &mut Resampling) -> bool {
    let previous = *resampling;
    control_row(ui, "Resampling", |ui| {
        egui::ComboBox::from_id_salt("resampling")
            .width(ui.available_width())
            .selected_text(match resampling {
                Resampling::Nearest => "Crisp / nearest",
                Resampling::Bilinear => "Rounded / bilinear",
                Resampling::Supersample2x => "2× supersampled",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(resampling, Resampling::Nearest, "Crisp / nearest");
                ui.selectable_value(resampling, Resampling::Bilinear, "Rounded / bilinear");
                ui.selectable_value(resampling, Resampling::Supersample2x, "2× supersampled");
            });
    });
    *resampling != previous
}

fn map_pattern_selector(ui: &mut egui::Ui, pattern: &mut MapPattern) -> bool {
    let previous = *pattern;
    control_row(ui, "Map source", |ui| {
        egui::ComboBox::from_id_salt("map-source")
            .width(ui.available_width())
            .selected_text(match pattern {
                MapPattern::Imported => "Imported maps",
                MapPattern::Grain => "Grain",
                MapPattern::Halftone => "Halftone",
                MapPattern::Grunge => "Grunge",
                MapPattern::Splatter => "Splatter",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(pattern, MapPattern::Imported, "Imported maps");
                ui.selectable_value(pattern, MapPattern::Grain, "Grain");
                ui.selectable_value(pattern, MapPattern::Halftone, "Halftone");
                ui.selectable_value(pattern, MapPattern::Grunge, "Grunge");
                ui.selectable_value(pattern, MapPattern::Splatter, "Splatter");
            });
    });
    *pattern != previous
}

fn crt_phase_selector(ui: &mut egui::Ui, phase: &mut CrtPhase) -> bool {
    let previous = *phase;
    control_row(ui, "Phase mode", |ui| {
        egui::ComboBox::from_id_salt("phase-mode")
            .width(ui.available_width())
            .selected_text(match phase {
                CrtPhase::Waveform => "Waveform",
                CrtPhase::Linear => "Linear",
                CrtPhase::Flux => "Flux",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(phase, CrtPhase::Waveform, "Waveform");
                ui.selectable_value(phase, CrtPhase::Linear, "Linear");
                ui.selectable_value(phase, CrtPhase::Flux, "Flux");
            });
    });
    *phase != previous
}

fn mode_selector(ui: &mut egui::Ui, mode: &mut Separation) -> bool {
    let previous = std::mem::discriminant(mode);
    control_row(ui, "Mode", |ui| {
        egui::ComboBox::from_id_salt("separation-mode")
            .width(ui.available_width())
            .selected_text(mode_name(mode))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(matches!(mode, Separation::Monochrome(_)), "Monochrome")
                    .clicked()
                {
                    *mode = Separation::Monochrome(Monochrome::default());
                }
                if ui
                    .selectable_label(matches!(mode, Separation::Tonal(_)), "Tonal gradient")
                    .clicked()
                {
                    *mode = Separation::Tonal(PaletteSettings::default());
                }
                if ui
                    .selectable_label(matches!(mode, Separation::Indexed(_)), "Extracted indexed")
                    .clicked()
                {
                    *mode = Separation::Indexed(PaletteSettings {
                        colors: Vec::new(),
                        size: 8,
                        ..PaletteSettings::default()
                    });
                }
                if ui
                    .selectable_label(matches!(mode, Separation::Custom(_)), "Custom palette")
                    .clicked()
                {
                    *mode = Separation::Custom(PaletteSettings::default());
                }
                if ui
                    .selectable_label(matches!(mode, Separation::Rgb(_)), "RGB plates")
                    .clicked()
                {
                    let mut rgb = ThreeColor::default();
                    rgb.cyan.color = [0.9, 0.05, 0.05];
                    rgb.magenta.color = [0.05, 0.8, 0.1];
                    rgb.yellow.color = [0.05, 0.2, 0.95];
                    *mode = Separation::Rgb(rgb);
                }
                if ui
                    .selectable_label(matches!(mode, Separation::ThreeColor(_)), "CMY plates")
                    .clicked()
                {
                    *mode = Separation::ThreeColor(ThreeColor::default());
                }
                if ui
                    .selectable_label(matches!(mode, Separation::Cmyk(_)), "CMYK plates")
                    .clicked()
                {
                    *mode = Separation::Cmyk(FourColor::default());
                }
                if ui
                    .selectable_label(matches!(mode, Separation::TriTone(_)), "Tri-tone Xerox")
                    .clicked()
                {
                    *mode = Separation::TriTone(TriTone::default());
                }
            });
    });
    std::mem::discriminant(mode) != previous
}

fn mode_name(mode: &Separation) -> &'static str {
    match mode {
        Separation::Monochrome(_) => "Monochrome",
        Separation::ThreeColor(_) => "CMY plates",
        Separation::Tonal(_) => "Tonal gradient",
        Separation::Indexed(_) => "Extracted indexed",
        Separation::Custom(_) => "Custom palette",
        Separation::Rgb(_) => "RGB plates",
        Separation::Cmyk(_) => "CMYK plates",
        Separation::TriTone(_) => "Tri-tone Xerox",
    }
}

fn palette_controls(ui: &mut egui::Ui, settings: &mut PaletteSettings) -> bool {
    sync_palette_inks(settings);
    let mut changed = control_row(ui, "Extract colors", |ui| {
        ui.add_sized(
            [ui.available_width(), 18.0],
            egui::Slider::new(&mut settings.size, 2..=64),
        )
    })
    .changed();
    ui.horizontal(|ui| {
        ui.label("Palette presets");
        if ui.small_button("B/W").clicked() {
            settings.colors = vec![[0.0; 3], [1.0; 3]];
            settings.inks.clear();
            sync_palette_inks(settings);
            changed = true;
        }
        if ui.small_button("Warm").clicked() {
            settings.colors = vec![
                [0.05, 0.03, 0.03],
                [0.55, 0.08, 0.05],
                [0.95, 0.7, 0.2],
                [0.96, 0.92, 0.78],
            ];
            settings.inks.clear();
            sync_palette_inks(settings);
            changed = true;
        }
        if ui.small_button("RGB").clicked() {
            settings.colors = vec![
                [0.0; 3],
                [1.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [0.0, 0.0, 1.0],
                [1.0; 3],
            ];
            settings.inks.clear();
            sync_palette_inks(settings);
            changed = true;
        }
    });
    let mut remove = None;
    let can_remove = settings.colors.len() > 2;
    for (index, color) in settings.colors.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!("{:02}", index + 1));
            changed |= ui.color_edit_button_rgb(color).changed();
            if can_remove && ui.small_button("−").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        settings.colors.remove(index);
        settings.inks.remove(index);
        changed = true;
    }
    if settings.colors.len() < 64 && ui.button("Add palette color").clicked() {
        settings.colors.push([0.5; 3]);
        settings.inks.push(Ink::new(
            [0.5; 3],
            [0, 0],
            [45.0, 15.0, 75.0, 0.0][settings.inks.len() % 4],
        ));
        changed = true;
    }
    for (index, ink) in settings.inks.iter_mut().enumerate() {
        egui::CollapsingHeader::new(format!("Plate {:02} setup", index + 1))
            .default_open(false)
            .show(ui, |ui| changed |= plate_geometry(ui, ink));
    }
    changed
}

fn sync_palette_inks(settings: &mut PaletteSettings) {
    settings.inks.truncate(settings.colors.len());
    while settings.inks.len() < settings.colors.len() {
        let index = settings.inks.len();
        settings.inks.push(Ink::new(
            settings.colors[index],
            [0, 0],
            [45.0, 15.0, 75.0, 0.0][index % 4],
        ));
    }
}

fn tone_band(ui: &mut egui::Ui, label: &str, band: &mut ToneBand) -> bool {
    ui.label(RichText::new(label).small().color(Color32::from_gray(165)));
    let mut changed = control_row(ui, "Range", |ui| {
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(
                    egui::DragValue::new(&mut band.range[0])
                        .range(0.0..=1.0)
                        .prefix("from "),
                )
                .changed();
            changed |= ui
                .add(
                    egui::DragValue::new(&mut band.range[1])
                        .range(0.0..=1.0)
                        .prefix("to "),
                )
                .changed();
            changed
        })
        .inner
    });
    changed |= slider(ui, "Intensity", &mut band.intensity, 0.0..=2.0);
    changed |= slider(ui, "Band grain", &mut band.grain.amount, 0.0..=1.0);
    changed |= ink(ui, "Ink / registration", &mut band.ink);
    changed
}

fn asset_control(
    ui: &mut egui::Ui,
    document: &mut Document,
    kind: AssetKind,
    label: &str,
) -> (bool, Option<String>) {
    let current = match kind {
        AssetKind::PaperTexture => document.recipe.assets.paper_texture.clone(),
        AssetKind::DisplacementMap => document.recipe.assets.displacement_map.clone(),
        AssetKind::DistressMask => document.recipe.assets.distress_mask.clone(),
    };
    let mut changed = false;
    let mut error = None;
    ui.horizontal(|ui| {
        if ui.button(label).clicked() {
            let extensions: Vec<_> = dither_io::supported_extensions().collect();
            if let Some(path) = FileDialog::new()
                .set_title(label)
                .add_filter("Supported images", &extensions)
                .pick_file()
            {
                match dither_io::open(&path) {
                    Ok(image) => {
                        document.set_asset(kind, image);
                        match kind {
                            AssetKind::PaperTexture => {
                                document.recipe.assets.paper_texture = Some(path)
                            }
                            AssetKind::DisplacementMap => {
                                document.recipe.assets.displacement_map = Some(path)
                            }
                            AssetKind::DistressMask => {
                                document.recipe.assets.distress_mask = Some(path)
                            }
                        }
                        changed = true;
                    }
                    Err(import_error) => error = Some(import_error.to_string()),
                }
            }
        }
        if current.is_some() && ui.small_button("Clear").clicked() {
            document.clear_asset(kind);
            match kind {
                AssetKind::PaperTexture => document.recipe.assets.paper_texture = None,
                AssetKind::DisplacementMap => document.recipe.assets.displacement_map = None,
                AssetKind::DistressMask => document.recipe.assets.distress_mask = None,
            }
            changed = true;
        }
        if let Some(path) = &current
            && ui.small_button("Reveal").clicked()
        {
            reveal_path(path);
        }
    });
    if let Some(path) = current {
        ui.label(
            RichText::new(path.file_name().unwrap_or_default().to_string_lossy())
                .small()
                .color(Color32::from_gray(125)),
        );
    }
    (changed, error)
}

fn control_row<R>(
    ui: &mut egui::Ui,
    label: &str,
    add_control: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Grid::new(ui.next_auto_id())
        .num_columns(2)
        .min_col_width(96.0)
        .spacing(Vec2::new(10.0, 6.0))
        .show(ui, |ui| {
            ui.label(RichText::new(label).small().color(MUTED));
            let response = add_control(ui);
            ui.end_row();
            response
        })
        .inner
}

fn slider(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    control_row(ui, label, |ui| {
        ui.add_sized(
            [ui.available_width(), 18.0],
            egui::Slider::new(value, range).show_value(true),
        )
    })
    .changed()
}

fn ink(ui: &mut egui::Ui, label: &str, ink: &mut Ink) -> bool {
    let mut changed = control_row(ui, label, |ui| {
        ui.color_edit_button_rgb(&mut ink.color).changed()
    });
    changed |= plate_geometry(ui, ink);
    changed
}

fn plate_geometry(ui: &mut egui::Ui, ink: &mut Ink) -> bool {
    let mut changed = ui.checkbox(&mut ink.enabled, "Plate enabled").changed();
    changed |= offset_values(ui, &mut ink.offset);
    changed |= slider(ui, "Screen angle", &mut ink.angle_degrees, -180.0..=180.0);
    changed |= control_row(ui, "Plate bleed px", |ui| {
        ui.add_sized(
            [ui.available_width(), 18.0],
            egui::Slider::new(&mut ink.bleed_pixels, 0..=16),
        )
    })
    .changed();
    changed |= control_row(ui, "Plate trap px", |ui| {
        ui.add_sized(
            [ui.available_width(), 18.0],
            egui::Slider::new(&mut ink.trapping_pixels, 0..=16),
        )
    })
    .changed();
    changed
}

fn offset_values(ui: &mut egui::Ui, value: &mut [i32; 2]) -> bool {
    control_row(ui, "Offset", |ui| {
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(
                    egui::DragValue::new(&mut value[0])
                        .range(-128..=128)
                        .prefix("x "),
                )
                .changed();
            changed |= ui
                .add(
                    egui::DragValue::new(&mut value[1])
                        .range(-128..=128)
                        .prefix("y "),
                )
                .changed();
            changed
        })
        .inner
    })
}

fn variation(ui: &mut egui::Ui, label: &str, seed: &mut u64) -> bool {
    control_row(ui, label, |ui| {
        ui.add_sized(
            [ui.available_width(), 18.0],
            egui::DragValue::new(seed).range(0..=u64::MAX),
        )
        .changed()
    })
}

fn randomizable_variation(ui: &mut egui::Ui, label: &str, seed: &mut u64) -> bool {
    control_row(ui, label, |ui| {
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(egui::DragValue::new(seed).range(0..=u64::MAX))
                .changed();
            if ui.small_button("Randomize").clicked() {
                *seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|duration| duration.as_nanos() as u64)
                    .unwrap_or_else(|_| seed.wrapping_add(1));
                changed = true;
            }
            changed
        })
        .inner
    })
}

fn confirmed_options(error: &IoError, mut options: ExportOptions) -> Option<ExportOptions> {
    match error {
        IoError::DestinationExists => options.overwrite = true,
        IoError::MetadataLossRequiresConfirmation => options.allow_metadata_loss = true,
        IoError::BitDepthReduction { .. } => options.allow_bit_depth_reduction = true,
        _ => return None,
    }
    Some(options)
}

fn section_label(text: &str) -> RichText {
    RichText::new(text)
        .small()
        .family(FontFamily::Name(Arc::from("plex-heading")))
        .color(ACCENT)
}

fn rgb_color(color: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn preview_image(image: &RenderedImage) -> egui::ColorImage {
    let bytes: Vec<u8> = image
        .pixels()
        .iter()
        .flat_map(|pixel| {
            [
                display_channel(pixel[0]),
                display_channel(pixel[1]),
                display_channel(pixel[2]),
                (pixel[3].clamp(0.0, 1.0) * 255.0).round() as u8,
            ]
        })
        .collect();
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        &bytes,
    )
}

fn plate_image(coverage: &[f32], width: u32, height: u32, ink: [f32; 3]) -> egui::ColorImage {
    let bytes: Vec<u8> = coverage
        .iter()
        .flat_map(|coverage| {
            let coverage = coverage.clamp(0.0, 1.0);
            [
                display_channel(1.0 - coverage + coverage * ink[0]),
                display_channel(1.0 - coverage + coverage * ink[1]),
                display_channel(1.0 - coverage + coverage * ink[2]),
                255,
            ]
        })
        .collect();
    egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &bytes)
}

fn config_directory() -> PathBuf {
    recipe_path()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(".dither"))
}

fn state_path() -> PathBuf {
    config_directory().join("state.json")
}

fn recovery_path() -> PathBuf {
    config_directory().join("recovery.dither")
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn file_modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).ok()?.modified().ok()
}

fn is_supported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| dither_io::supported_extensions().any(|item| item == extension))
}

fn scan_folder(folder: &Path, sort: BrowserSort) -> Vec<BrowserEntry> {
    let mut entries: Vec<_> = fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_supported_file(path))
        .map(|path| BrowserEntry {
            modified: file_modified(&path),
            path,
            thumbnail: None,
            error: None,
        })
        .collect();
    entries.sort_by(|left, right| match sort {
        BrowserSort::Name => left.path.file_name().cmp(&right.path.file_name()),
        BrowserSort::Modified => right.modified.cmp(&left.modified),
        BrowserSort::FileType => left
            .path
            .extension()
            .cmp(&right.path.extension())
            .then_with(|| left.path.file_name().cmp(&right.path.file_name())),
    });
    entries
}

fn browser_matches(entry: &BrowserEntry, filter: &str) -> bool {
    filter.is_empty()
        || entry
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains(&filter.to_ascii_lowercase())
}

fn reveal_path(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(path)
        .spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if let Some(parent) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
    }
}

fn display_channel(value: f32) -> u8 {
    let value = if value <= 0.0031308 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_confirmation_grants_only_the_named_permission() {
        let overwrite =
            confirmed_options(&IoError::DestinationExists, ExportOptions::default()).unwrap();
        assert!(overwrite.overwrite);
        assert!(!overwrite.allow_metadata_loss);
        assert!(!overwrite.allow_bit_depth_reduction);

        let metadata =
            confirmed_options(&IoError::MetadataLossRequiresConfirmation, overwrite).unwrap();
        assert!(metadata.overwrite);
        assert!(metadata.allow_metadata_loss);
        assert!(!metadata.allow_bit_depth_reduction);
    }
}
