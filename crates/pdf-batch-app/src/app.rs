//! Main window. Every slow operation (add, delete, clear, print) runs on a worker
//! thread and reports through a channel, so the window never freezes.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText};
use egui_extras::{Column, TableBuilder};
use pdf_batch_core::{recovery_reference, Item, QueueStore, Resolution};
use pdf_batch_print::{self as printing, BatchEvent, BatchOutcome, Document};
use rfd::{MessageButtons, MessageDialog, MessageDialogResult, MessageLevel};

use crate::i18n::{self, engine, t};
use crate::tf;

const NAVY: Color32 = Color32::from_rgb(0x17, 0x3d, 0x60);
const MUTED: Color32 = Color32::from_rgb(0x63, 0x74, 0x8a);
const DANGER: Color32 = Color32::from_rgb(0xa4, 0x32, 0x32);
const WARN: Color32 = Color32::from_rgb(0x92, 0x40, 0x0e);

fn state_label(s: &str) -> &'static str {
    match s {
        "pending" => t("state.pending"),
        "in_flight" => t("state.in_flight"),
        "uncertain" => t("state.uncertain"),
        "sent" => t("state.sent"),
        "skipped" => t("state.skipped"),
        _ => t("state.error"),
    }
}

fn outcome_label(s: &str) -> &'static str {
    match s {
        "completed" => t("outcome.completed"),
        "review" => t("outcome.review"),
        _ => t("outcome.none"),
    }
}

fn row_colors(state: &str) -> Option<(Color32, Color32)> {
    Some(match state {
        "uncertain" => (Color32::from_rgb(0xff, 0xf2, 0xd7), Color32::from_rgb(0x87, 0x52, 0x0d)),
        "in_flight" => (Color32::from_rgb(0xe4, 0xf0, 0xfc), Color32::from_rgb(0x17, 0x4c, 0x7a)),
        "sent" => (Color32::from_rgb(0xed, 0xf5, 0xf0), Color32::from_rgb(0x2e, 0x60, 0x47)),
        "skipped" => (Color32::from_rgb(0xf5, 0xf6, 0xf8), Color32::from_rgb(0x73, 0x80, 0x93)),
        _ => return None,
    })
}

fn ask(title: &str, text: &str) -> bool {
    MessageDialog::new()
        .set_level(MessageLevel::Warning)
        .set_title(title)
        .set_description(text)
        .set_buttons(MessageButtons::YesNo)
        .show()
        == MessageDialogResult::Yes
}

fn info(title: &str, text: &str, level: MessageLevel) {
    MessageDialog::new().set_level(level).set_title(title).set_description(text).show();
}

/// egui's built-in fonts have no Japanese glyphs: use a Windows Japanese font.
/// Returns false if no suitable font was found.
fn install_cjk_font(ctx: &egui::Context) -> bool {
    let fonts_dir = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from).join("Fonts");
    for file in ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"] {
        if let Ok(bytes) = std::fs::read(fonts_dir.join(file)) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert("cjk".into(), egui::FontData::from_owned(bytes));
            // First, not fallback: mixing it with egui's Latin font misaligns the baselines.
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().insert(0, "cjk".into());
            }
            ctx.set_fonts(fonts);
            return true;
        }
    }
    false
}

enum Msg {
    Batch(BatchEvent),
    /// Operation finished; status text.
    Done(String),
    /// Operation stopped; shown in a dialog unless an interruption opens.
    Error(String),
}

struct Recovery {
    item: Item,
    page_text: String,
    caption: String,
    texture: Option<egui::TextureHandle>,
}

pub struct App {
    store: QueueStore,
    data_dir: PathBuf,
    items: Vec<Item>,
    selected: BTreeSet<i64>,
    printers: Vec<String>,
    printer: String,
    batch_size: String,
    busy: bool,
    pause: Arc<AtomicBool>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    status: String,
    /// `(completed, target)` of the current batch, if one was started.
    batch: Option<(usize, usize)>,
    recovery: Option<Recovery>,
    checked_interruption: bool,
    cjk_font: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, store: QueueStore, data_dir: PathBuf) -> Self {
        let mut visuals = egui::Visuals::light();
        visuals.panel_fill = Color32::from_rgb(0xf3, 0xf5, 0xf8);
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.style_mut(|s| {
            s.spacing.button_padding = egui::vec2(10.0, 6.0);
            s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        });
        let (tx, rx) = channel();
        let mut app = App {
            store,
            data_dir,
            items: Vec::new(),
            selected: BTreeSet::new(),
            printers: Vec::new(),
            printer: String::new(),
            batch_size: t("all").into(),
            busy: false,
            pause: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            status: t("ready").into(),
            batch: None,
            recovery: None,
            checked_interruption: false,
            cjk_font: false,
        };
        if i18n::code() == "ja" {
            app.cjk_font = install_cjk_font(&cc.egui_ctx);
        }
        app.refresh_printers();
        app.refresh();
        app
    }

    fn set_language(&mut self, ctx: &egui::Context, index: usize) {
        if index == i18n::current() {
            return;
        }
        let all_selected = i18n::is_all(&self.batch_size);
        i18n::set(index);
        if i18n::code() == "ja" && !self.cjk_font {
            self.cjk_font = install_cjk_font(ctx);
        } else if i18n::code() != "ja" && self.cjk_font {
            ctx.set_fonts(egui::FontDefinitions::default());
            self.cjk_font = false;
        }
        let _ = i18n::save(&self.data_dir);
        if all_selected {
            self.batch_size = t("all").into();
        }
        if !self.busy {
            self.status = t("ready").into();
        }
    }

    fn refresh(&mut self) {
        match self.store.list_items() {
            Ok(items) => {
                self.selected.retain(|id| items.iter().any(|i| i.id == *id));
                self.items = items;
            }
            Err(e) => self.status = tf!("readQueueFailed", e),
        }
    }

    fn refresh_printers(&mut self) {
        self.printers = printing::list_printers();
        if !self.printers.contains(&self.printer) {
            let def = printing::default_printer().filter(|d| self.printers.contains(d));
            self.printer = def.or_else(|| self.printers.first().cloned()).unwrap_or_default();
        }
        if self.printers.is_empty() {
            self.status = t("noPrinters").into();
        }
    }

    fn summary(&self) -> String {
        let c = |s: &str| self.items.iter().filter(|i| i.state == s).count();
        tf!("summary", self.items.len(), c("pending"), c("sent"), c("in_flight"), c("uncertain"))
    }

    fn batch_label(&self) -> String {
        match self.batch {
            Some((done, target)) => tf!("batchProgress", done, target),
            None => t("batchNotStarted").into(),
        }
    }

    /// Run `job` on a worker thread; the window stays responsive meanwhile.
    fn spawn(&mut self, ctx: &egui::Context, status: &str, job: impl FnOnce(QueueStore, &Sender<Msg>) -> Msg + Send + 'static) {
        self.busy = true;
        self.status = status.into();
        let (store, tx, ctx) = (self.store.clone(), self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            let msg = job(store, &tx);
            let _ = tx.send(msg);
            ctx.request_repaint();
        });
    }

    // ------------------------------------------------------------ actions

    fn add_files(&mut self, ctx: &egui::Context) {
        if let Some(paths) = rfd::FileDialog::new().set_title(t("dlgAddFiles")).add_filter("PDF", &["pdf", "PDF"]).pick_files() {
            self.add_paths(ctx, paths);
        }
    }

    fn add_folder(&mut self, ctx: &egui::Context) {
        let Some(folder) = rfd::FileDialog::new().set_title(t("dlgAddFolder")).pick_folder() else {
            return;
        };
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&folder)
            .map(|rd| {
                rd.filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.is_file() && p.extension().map_or(false, |e| e.eq_ignore_ascii_case("pdf")))
                    .collect()
            })
            .unwrap_or_default();
        paths.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
        if paths.is_empty() {
            info(t("emptyFolderTitle"), t("emptyFolderText"), MessageLevel::Info);
        } else {
            self.add_paths(ctx, paths);
        }
    }

    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.spawn(ctx, t("validating"), move |store, _| {
            let check = || -> Result<(), String> {
                let pdfium = printing::pdfium().map_err(|e| e.to_string())?;
                for p in &paths {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let doc = Document::open(pdfium, p).map_err(|_| tf!("invalidPdf", name))?;
                    if doc.page_count() < 1 {
                        return Err(tf!("emptyPdf", name));
                    }
                    if !(0..doc.page_count()).all(|i| doc.page_ok(i)) {
                        return Err(tf!("damagedPdf", name));
                    }
                }
                store.add(&paths).map_err(|e| e.to_string())?;
                Ok(())
            };
            match check() {
                Ok(()) => Msg::Done(tf!("added", paths.len())),
                Err(e) => Msg::Error(e),
            }
        });
    }

    fn single_selected(&self) -> Result<i64, String> {
        if self.selected.len() != 1 {
            return Err(t("selectOne").into());
        }
        Ok(*self.selected.iter().next().unwrap())
    }

    fn move_selected(&mut self, delta: i64) {
        let r = self.single_selected().and_then(|id| self.store.move_item(id, delta).map_err(|e| e.to_string()));
        if let Err(e) = r {
            info(t("moveFailed"), &engine(&e), MessageLevel::Error);
        }
        self.refresh();
    }

    fn remove(&mut self, ctx: &egui::Context) {
        if self.selected.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.selected.iter().copied().collect();
        if !ask(t("removeTitle"), &tf!("removeText", ids.len())) {
            return;
        }
        self.spawn(ctx, t("removing"), move |store, _| match store.delete_items(&ids) {
            Ok(n) => Msg::Done(tf!("removed", n)),
            Err(e) => Msg::Error(e.to_string()),
        });
    }

    /// Runs on a worker thread: this is the fix for the v1.4 freeze.
    fn clear_folder(&mut self, ctx: &egui::Context) {
        if self.items.is_empty() {
            return;
        }
        if !ask(t("clearTitle"), t("clearText")) {
            return;
        }
        self.batch = None;
        self.spawn(ctx, t("clearing"), |store, _| match store.clear_items() {
            Ok(n) => Msg::Done(tf!("cleared", n)),
            Err(e) => Msg::Error(e.to_string()),
        });
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.printer.is_empty() {
            return info(t("noPrinterTitle"), t("noPrinterText"), MessageLevel::Error);
        }
        if !matches!(self.store.pending(), Ok(Some(_))) {
            return info(t("noPendingTitle"), t("noPendingText"), MessageLevel::Info);
        }
        let choice = self.batch_size.trim();
        let limit = if i18n::is_all(choice) {
            None
        } else {
            match choice.parse::<usize>() {
                Ok(n) if n > 0 && choice.bytes().all(|b| b.is_ascii_digit()) => Some(n),
                _ => return info(t("badCountTitle"), t("badCountText"), MessageLevel::Error),
            }
        };
        let available = self
            .items
            .iter()
            .filter(|i| i.state != "sent" && i.state != "skipped")
            .take_while(|i| i.state == "pending")
            .count();
        let target = limit.map_or(available, |l| l.min(available));
        if !ask(t("confirmPrintTitle"), &tf!("confirmPrintText", target, self.printer)) {
            return;
        }
        self.pause.store(false, Ordering::SeqCst);
        self.batch = Some((0, target));
        let (printer, pause, ctx2) = (self.printer.clone(), self.pause.clone(), ctx.clone());
        self.spawn(ctx, t("starting"), move |store, tx| {
            let (etx, erx) = channel::<BatchEvent>();
            let fwd = {
                let (tx, ctx) = (tx.clone(), ctx2.clone());
                std::thread::spawn(move || {
                    for e in erx {
                        let _ = tx.send(Msg::Batch(e));
                        ctx.request_repaint();
                    }
                })
            };
            let mut transport = printing::windows_printer();
            let out = printing::run_batch(&store, &printer, target, &pause, &etx, &mut transport);
            drop(etx);
            let _ = fwd.join();
            match out {
                BatchOutcome::Finished { completed, target } => Msg::Done(tf!("batchFinished", completed, target)),
                BatchOutcome::Paused { completed, target } => Msg::Done(tf!("batchPaused", completed, target)),
                BatchOutcome::Error(e) => Msg::Error(e),
            }
        });
    }

    fn request_pause(&mut self) {
        self.pause.store(true, Ordering::SeqCst);
        self.status = t("pauseRequested").into();
    }

    fn export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title(t("exportTitle"))
            .set_file_name(t("exportFileName"))
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        let r = serde_json::to_string_pretty(&self.items)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        match r {
            Ok(()) => self.status = tf!("exported", path.display()),
            Err(e) => info(t("exportTitle"), &e, MessageLevel::Error),
        }
    }

    fn open_shell(target: &str) {
        let _ = std::process::Command::new("explorer").arg(target).spawn();
    }

    // ------------------------------------------------------------ recovery

    fn open_recovery(&mut self, ctx: &egui::Context, id: i64) {
        let Some(item) = self.items.iter().find(|i| i.id == id).cloned() else { return };
        if item.state != "uncertain" {
            return info(t("selectInterruption"), t("notInterrupted"), MessageLevel::Error);
        }
        let mut caption = t("noLastPage").to_string();
        let mut texture = None;
        if let Some((ref_item, page)) = recovery_reference(&self.items, id) {
            let render = || -> Result<printing::Raster, String> {
                let doc = Document::open(printing::pdfium().map_err(|e| e.to_string())?, Path::new(&ref_item.snapshot))
                    .map_err(|e| e.to_string())?;
                if page as u32 > doc.page_count() {
                    return Err(t("pageOutsideCopy").into());
                }
                doc.render_fit(page as u32 - 1, (460, 480))
            };
            match render() {
                Ok(r) => {
                    let mut rgba = r.bgra;
                    for px in rgba.chunks_exact_mut(4) {
                        px.swap(0, 2);
                        px[3] = 255;
                    }
                    let img = egui::ColorImage::from_rgba_unmultiplied([r.width as usize, r.height as usize], &rgba);
                    texture = Some(ctx.load_texture("recovery", img, Default::default()));
                    caption = tf!("recoveryCaption", ref_item.name, page);
                }
                Err(e) => caption = tf!("previewFailed", e),
            }
        }
        self.recovery = Some(Recovery { item, page_text: "1".into(), caption, texture });
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(rec) = self.recovery.as_mut() else { return };
        let mut open = true;
        let mut action: Option<(Resolution, bool)> = None;
        egui::Window::new(t("recoveryTitle"))
            .id(egui::Id::new("recovery"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([960.0, 600.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.columns(2, |cols| {
                    let ui = &mut cols[0];
                    ui.label(RichText::new(t("expectedPage")).size(18.0).strong());
                    ui.label(&rec.caption);
                    if let Some(tex) = &rec.texture {
                        ui.add(egui::Image::new(tex).max_size(egui::vec2(460.0, 480.0)));
                    }
                    ui.colored_label(WARN, t("compareHint"));

                    let ui = &mut cols[1];
                    let it = &rec.item;
                    ui.label(RichText::new(&it.name).size(16.0).strong());
                    ui.label(tf!(
                        "recoveryDetails",
                        it.printer.as_deref().unwrap_or(t("notRecorded")),
                        it.job_id.as_deref().unwrap_or(t("notRecorded")),
                        it.last_page,
                        it.total_pages.map_or("?".to_string(), |n| n.to_string()),
                    ));
                    if let Some(e) = &it.error {
                        ui.colored_label(DANGER, engine(e));
                    }
                    ui.horizontal(|ui| {
                        ui.label(t("firstMissing"));
                        ui.add(egui::TextEdit::singleline(&mut rec.page_text).desired_width(60.0));
                    });
                    let w = ui.available_width();
                    let b = |ui: &mut egui::Ui, text: &str| ui.add_sized([w, 30.0], egui::Button::new(text)).clicked();
                    if b(ui, t("resumeFirst")) {
                        action = Some((Resolution::Retry, true));
                    }
                    if b(ui, t("resumeFrom")) {
                        action = Some((Resolution::Retry, false));
                    }
                    if b(ui, t("continueNext")) {
                        action = Some((Resolution::Sent, false));
                    }
                    if b(ui, t("skipPdf")) {
                        action = Some((Resolution::Skip, false));
                    }
                    ui.colored_label(MUTED, t("afterResolve"));
                });
            });
        if let Some((res, from_first)) = action {
            let first = if res == Resolution::Retry && !from_first { rec.page_text.trim().parse::<i64>().unwrap_or(0) } else { 1 };
            let (id, total) = (rec.item.id, rec.item.total_pages);
            if first < 1 || total.map_or(false, |n| first > n) {
                info(t("resolveFailed"), t("pageOutsideDoc"), MessageLevel::Error);
            } else if ask(t("confirmReviewTitle"), t("confirmReviewText")) {
                match self.store.resolve(id, res, first) {
                    Ok(()) => {
                        self.recovery = None;
                        self.refresh();
                    }
                    Err(e) => info(t("resolveFailed"), &engine(&e.to_string()), MessageLevel::Error),
                }
            }
        }
        if !open {
            self.recovery = None;
        }
    }

    /// Open the first interruption, if any. Returns whether one was found.
    fn show_interruption(&mut self, ctx: &egui::Context) -> bool {
        if self.busy {
            return false;
        }
        if let Some(id) = self.items.iter().find(|i| i.state == "uncertain").map(|i| i.id) {
            self.selected = [id].into();
            self.open_recovery(ctx, id);
            return true;
        }
        false
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Batch(BatchEvent::Status(s)) => self.status = engine(&s),
                Msg::Batch(BatchEvent::Refresh) => self.refresh(),
                Msg::Batch(BatchEvent::Batch(done, target)) => self.batch = Some((done, target)),
                Msg::Done(s) => {
                    self.busy = false;
                    self.status = s;
                    self.refresh();
                }
                Msg::Error(s) => {
                    let s = engine(&s);
                    self.busy = false;
                    self.status = s.clone();
                    self.refresh();
                    if !self.show_interruption(ctx) {
                        info(t("stoppedTitle"), &tf!("stoppedText", s), MessageLevel::Error);
                    }
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        if !self.checked_interruption {
            self.checked_interruption = true;
            self.show_interruption(ctx);
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.busy {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_pause();
            info(t("pauseTitle"), t("closeBusyText"), MessageLevel::Info);
        }
        let idle = !self.busy && self.recovery.is_none();

        egui::TopBottomPanel::top("header").frame(egui::Frame::none().inner_margin(16.0).fill(ctx.style().visuals.panel_fill)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("PDF-batch").size(26.0).strong().color(NAVY));
                ui.label(RichText::new(tf!("subtitle", crate::VERSION)).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut lang = i18n::current();
                    egui::ComboBox::from_id_salt("language")
                        .selected_text(i18n::LANGUAGES[lang].1)
                        .show_ui(ui, |ui| {
                            for (i, (_, name)) in i18n::LANGUAGES.iter().enumerate() {
                                // Japanese needs the CJK fallback font to show its own name.
                                let name = if i18n::LANGUAGES[i].0 == "ja" && !self.cjk_font { "Japanese" } else { name };
                                ui.selectable_value(&mut lang, i, name);
                            }
                        });
                    ui.label(RichText::new(t("language")).color(MUTED));
                    self.set_language(ctx, lang);
                    ui.add_space(12.0);
                    if ui.link(t("createdBy")).clicked() {
                        Self::open_shell("https://github.com/siriusgoldsmith/PDF-batch");
                    }
                });
            });
            ui.add_space(8.0);
            ui.add_enabled_ui(idle, |ui| {
                ui.horizontal(|ui| {
                    if ui.button(t("addPdfs")).clicked() {
                        self.add_files(ctx);
                    }
                    if ui.button(t("addFolder")).clicked() {
                        self.add_folder(ctx);
                    }
                    if ui.button(t("moveUp")).clicked() {
                        self.move_selected(-1);
                    }
                    if ui.button(t("moveDown")).clicked() {
                        self.move_selected(1);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new(t("clearFolder")).color(DANGER)).clicked() {
                            self.clear_folder(ctx);
                        }
                        if ui.button(t("removePdf")).clicked() {
                            self.remove(ctx);
                        }
                    });
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.summary()).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(MUTED, t("printOrder"));
                });
            });
        });

        egui::TopBottomPanel::bottom("footer").frame(egui::Frame::none().inner_margin(16.0).fill(ctx.style().visuals.panel_fill)).show(ctx, |ui| {
            let details = self
                .selected
                .iter()
                .next()
                .and_then(|id| self.items.iter().find(|i| i.id == *id))
                .map(|r| match &r.error {
                    Some(e) => format!("{} | {}", r.source, engine(e)),
                    None => r.source.clone(),
                })
                .unwrap_or_else(|| t("footerHint").into());
            ui.colored_label(MUTED, details);
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
                ui.label(RichText::new(t("printing")).strong());
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(t("printer"));
                        egui::ComboBox::from_id_salt("printer")
                            .width(ui.available_width() - 300.0)
                            .selected_text(&self.printer)
                            .show_ui(ui, |ui| {
                                for p in self.printers.clone() {
                                    ui.selectable_value(&mut self.printer, p.clone(), p);
                                }
                            });
                        if ui.button(t("refresh")).clicked() {
                            self.refresh_printers();
                        }
                        if ui.button(t("windowsSettings")).clicked() {
                            Self::open_shell("ms-settings:printers");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(t("perBatch"));
                    ui.add_enabled_ui(idle, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.batch_size).desired_width(70.0));
                        for v in ["20", "50", "100", t("all")] {
                            if ui.small_button(v).clicked() {
                                self.batch_size = v.into();
                            }
                        }
                    });
                    ui.colored_label(MUTED, t("countHint"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let start = egui::Button::new(RichText::new(t("start")).strong().color(Color32::WHITE)).fill(NAVY);
                        if ui.add_enabled(idle, start).clicked() {
                            self.start(ctx);
                        }
                        let pausing = self.pause.load(Ordering::SeqCst);
                        if ui.add_enabled(self.busy && !pausing, egui::Button::new(t("pauseAfter"))).clicked() {
                            self.request_pause();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(self.batch_label());
                    let (d, n) = self.batch.unwrap_or((0, 1));
                    ui.add(egui::ProgressBar::new(d as f32 / n.max(1) as f32));
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(idle, egui::Button::new(t("resolve"))).clicked() {
                    match self.single_selected() {
                        Ok(id) => self.open_recovery(ctx, id),
                        Err(e) => info(t("selectInterruption"), &e, MessageLevel::Error),
                    }
                }
                ui.colored_label(MUTED, t("resolveHint"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("exportReport")).clicked() {
                        self.export();
                    }
                    if ui.button(t("openLog")).clicked() {
                        Self::open_shell(&self.data_dir.to_string_lossy());
                    }
                });
            });
            ui.horizontal(|ui| {
                if self.busy {
                    ui.spinner();
                }
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().frame(egui::Frame::none().inner_margin(egui::Margin::symmetric(16.0, 0.0)).fill(Color32::WHITE)).show(ctx, |ui| {
            let ctrl = ui.input(|i| i.modifiers.command || i.modifiers.shift);
            let mut clicked: Option<i64> = None;
            TableBuilder::new(ui)
                .striped(false)
                .sense(egui::Sense::click())
                .column(Column::exact(90.0))
                .column(Column::remainder().at_least(200.0).clip(true))
                .column(Column::exact(170.0))
                .column(Column::exact(210.0))
                .column(Column::exact(130.0))
                .header(30.0, |mut h| {
                    for key in ["col.order", "col.document", "col.state", "col.outcome", "col.pages"] {
                        h.col(|ui| {
                            ui.strong(t(key));
                        });
                    }
                })
                .body(|body| {
                    body.rows(28.0, self.items.len(), |mut row| {
                        let it = &self.items[row.index()];
                        let selected = self.selected.contains(&it.id);
                        row.set_selected(selected);
                        let color = row_colors(&it.state);
                        let cell = |ui: &mut egui::Ui, text: String| {
                            if let Some((bg, fg)) = color {
                                if !selected {
                                    ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(4.0, 2.0)), 0.0, bg);
                                }
                                ui.colored_label(fg, text);
                            } else {
                                ui.label(text);
                            }
                        };
                        let total = it.total_pages.map_or("?".into(), |n| n.to_string());
                        let cols = [
                            it.position.to_string(),
                            it.name.clone(),
                            state_label(&it.state).to_string(),
                            outcome_label(&it.queue_outcome).to_string(),
                            format!("{} / {total}", it.last_page),
                        ];
                        for c in cols {
                            row.col(|ui| cell(ui, c));
                        }
                        if row.response().clicked() {
                            clicked = Some(it.id);
                        }
                    });
                });
            if let Some(id) = clicked {
                if ctrl {
                    if !self.selected.remove(&id) {
                        self.selected.insert(id);
                    }
                } else {
                    self.selected = [id].into();
                }
            }
        });

        self.recovery_window(ctx);
    }
}
