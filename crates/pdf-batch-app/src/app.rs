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

const NAVY: Color32 = Color32::from_rgb(0x17, 0x3d, 0x60);
const MUTED: Color32 = Color32::from_rgb(0x63, 0x74, 0x8a);
const DANGER: Color32 = Color32::from_rgb(0xa4, 0x32, 0x32);
const WARN: Color32 = Color32::from_rgb(0x92, 0x40, 0x0e);

fn state_label(s: &str) -> &'static str {
    match s {
        "pending" => "Pendiente",
        "in_flight" => "En proceso",
        "uncertain" => "Revisar interrupción",
        "sent" => "Finalizado",
        "skipped" => "Omitido",
        _ => "Error",
    }
}

fn outcome_label(s: &str) -> &'static str {
    match s {
        "completed" => "Finalizado según Windows",
        "review" => "Error / por revisar",
        _ => "Sin resultado registrado",
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
    batch_count: String,
    progress: (usize, usize),
    recovery: Option<Recovery>,
    checked_interruption: bool,
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
            batch_size: "Todos".into(),
            busy: false,
            pause: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            status: "Listo. El avance se guarda automáticamente; no necesitas confirmar cada PDF.".into(),
            batch_count: "Lote: sin iniciar".into(),
            progress: (0, 1),
            recovery: None,
            checked_interruption: false,
        };
        app.refresh_printers();
        app.refresh();
        app
    }

    fn refresh(&mut self) {
        match self.store.list_items() {
            Ok(items) => {
                self.selected.retain(|id| items.iter().any(|i| i.id == *id));
                self.items = items;
            }
            Err(e) => self.status = format!("No se pudo leer la cola: {e}"),
        }
    }

    fn refresh_printers(&mut self) {
        self.printers = printing::list_printers();
        if !self.printers.contains(&self.printer) {
            let def = printing::default_printer().filter(|d| self.printers.contains(d));
            self.printer = def.or_else(|| self.printers.first().cloned()).unwrap_or_default();
        }
        if self.printers.is_empty() {
            self.status = "No hay impresoras disponibles. Instala una en Windows y pulsa Actualizar.".into();
        }
    }

    fn summary(&self) -> String {
        let c = |s: &str| self.items.iter().filter(|i| i.state == s).count();
        format!(
            "{} PDFs · {} pendientes · {} finalizados · {} en proceso · {} por revisar",
            self.items.len(),
            c("pending"),
            c("sent"),
            c("in_flight"),
            c("uncertain")
        )
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
        if let Some(paths) = rfd::FileDialog::new()
            .set_title("Añadir PDFs: revisa luego el orden en la lista")
            .add_filter("PDF", &["pdf", "PDF"])
            .pick_files()
        {
            self.add_paths(ctx, paths);
        }
    }

    fn add_folder(&mut self, ctx: &egui::Context) {
        let Some(folder) = rfd::FileDialog::new()
            .set_title("Añadir PDFs de una carpeta (orden por nombre, sin subcarpetas)")
            .pick_folder()
        else {
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
            info("Carpeta vacía", "No hay archivos PDF en esta carpeta.", MessageLevel::Info);
        } else {
            self.add_paths(ctx, paths);
        }
    }

    fn add_paths(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.spawn(ctx, "Validando PDFs y guardando copias del lote…", move |store, _| {
            let check = || -> Result<(), String> {
                let pdfium = printing::pdfium().map_err(|e| e.to_string())?;
                for p in &paths {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let doc = Document::open(pdfium, p)
                        .map_err(|_| format!("PDF inválido o protegido con contraseña: {name}"))?;
                    if doc.page_count() < 1 {
                        return Err(format!("PDF inválido o vacío: {name}"));
                    }
                    if !(0..doc.page_count()).all(|i| doc.page_ok(i)) {
                        return Err(format!("PDF dañado: {name}"));
                    }
                }
                store.add(&paths).map_err(|e| e.to_string())?;
                Ok(())
            };
            match check() {
                Ok(()) => Msg::Done(format!("Se añadieron {} PDFs al final de la lista.", paths.len())),
                Err(e) => Msg::Error(e),
            }
        });
    }

    fn single_selected(&self) -> Result<i64, String> {
        if self.selected.len() != 1 {
            return Err("Selecciona un solo documento.".into());
        }
        Ok(*self.selected.iter().next().unwrap())
    }

    fn move_selected(&mut self, delta: i64) {
        let r = self.single_selected().and_then(|id| self.store.move_item(id, delta).map_err(|e| e.to_string()));
        if let Err(e) = r {
            info("No se pudo mover", &e, MessageLevel::Error);
        }
        self.refresh();
    }

    fn remove(&mut self, ctx: &egui::Context) {
        if self.selected.is_empty() {
            return;
        }
        let ids: Vec<i64> = self.selected.iter().copied().collect();
        if !ask(
            "Eliminar PDF del lote",
            &format!(
                "¿Eliminar {} PDF(s) seleccionados de la lista y sus copias locales?\n\nLos archivos originales \
                 y el historial se conservan. No se pueden eliminar interrupciones sin resolver.",
                ids.len()
            ),
        ) {
            return;
        }
        self.spawn(ctx, "Eliminando PDFs del lote…", move |store, _| match store.delete_items(&ids) {
            Ok(n) => Msg::Done(format!("{n} PDF(s) eliminados del lote. Originales conservados.")),
            Err(e) => Msg::Error(e.to_string()),
        });
    }

    /// Runs on a worker thread: this is the fix for the v1.4 freeze.
    fn clear_folder(&mut self, ctx: &egui::Context) {
        if self.items.is_empty() {
            return;
        }
        if !ask(
            "Vaciar carpeta del lote",
            "¿Vaciar la lista y eliminar sus copias locales de PDF?\n\nNO se borrará la carpeta de origen ni sus \
             archivos. El historial se conserva. Resuelve primero cualquier interrupción.",
        ) {
            return;
        }
        self.batch_count = "Lote: sin iniciar".into();
        self.progress = (0, 1);
        self.spawn(ctx, "Vaciando la carpeta del lote…", |store, _| match store.clear_items() {
            Ok(n) => Msg::Done(format!("Carpeta del lote vaciada: {n} PDF(s). Originales conservados.")),
            Err(e) => Msg::Error(e.to_string()),
        });
    }

    fn start(&mut self, ctx: &egui::Context) {
        if self.printer.is_empty() {
            return info("Sin impresora", "Selecciona una impresora.", MessageLevel::Error);
        }
        if !matches!(self.store.pending(), Ok(Some(_))) {
            return info(
                "Sin pendientes disponibles",
                "El lote terminó o hay una interrupción que debes resolver antes de continuar.",
                MessageLevel::Info,
            );
        }
        let choice = self.batch_size.trim();
        let limit = if choice.eq_ignore_ascii_case("todos") {
            None
        } else {
            match choice.parse::<usize>() {
                Ok(n) if n > 0 && choice.bytes().all(|b| b.is_ascii_digit()) => Some(n),
                _ => {
                    return info(
                        "Cantidad no válida",
                        "Escribe una cantidad entera mayor que cero (por ejemplo, 35) o selecciona Todos.",
                        MessageLevel::Error,
                    )
                }
            }
        };
        let available = self
            .items
            .iter()
            .filter(|i| i.state != "sent" && i.state != "skipped")
            .take_while(|i| i.state == "pending")
            .count();
        let target = limit.map_or(available, |l| l.min(available));
        if !ask(
            "Confirmar impresión",
            &format!(
                "¿Imprimir los próximos {target} PDFs en «{}»?\n\nUna copia por documento, ajustada al área \
                 imprimible. Papel, bandeja, color y dúplex: valores predeterminados del controlador.\n\nRevisa los \
                 ajustes y el orden antes de aceptar. No envíes el mismo lote desde otra aplicación.",
                self.printer
            ),
        ) {
            return;
        }
        self.pause.store(false, Ordering::SeqCst);
        self.batch_count = format!("Lote: 0 / {target} PDFs finalizados según Windows");
        self.progress = (0, target.max(1));
        let (printer, pause, ctx2) = (self.printer.clone(), self.pause.clone(), ctx.clone());
        self.spawn(ctx, "Iniciando lote…", move |store, tx| {
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
                BatchOutcome::Finished { completed, target } => Msg::Done(format!(
                    "Lote terminado: {completed} / {target} PDFs finalizados según Windows. Pulsa Iniciar / continuar para el siguiente lote."
                )),
                BatchOutcome::Paused { completed, target } => Msg::Done(format!(
                    "Lote pausado: {completed} / {target} PDFs finalizados según Windows. Pulsa Iniciar / continuar para el siguiente lote."
                )),
                BatchOutcome::Error(e) => Msg::Error(e),
            }
        });
    }

    fn request_pause(&mut self) {
        self.pause.store(true, Ordering::SeqCst);
        self.status = "Pausa solicitada. Se terminará el PDF actual antes de detener el lote.".into();
    }

    fn export(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Exportar registro")
            .set_file_name("informe-pdf-batch.json")
            .add_filter("JSON", &["json"])
            .save_file()
        else {
            return;
        };
        let r = serde_json::to_string_pretty(&self.items)
            .map_err(|e| e.to_string())
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        match r {
            Ok(()) => self.status = format!("Informe exportado: {}", path.display()),
            Err(e) => info("Exportar", &e, MessageLevel::Error),
        }
    }

    fn open_shell(target: &str) {
        let _ = std::process::Command::new("explorer").arg(target).spawn();
    }

    // ------------------------------------------------------------ recovery

    fn open_recovery(&mut self, ctx: &egui::Context, id: i64) {
        let Some(item) = self.items.iter().find(|i| i.id == id).cloned() else { return };
        if item.state != "uncertain" {
            return info("Seleccionar interrupción", "Este documento no tiene una interrupción pendiente de resolver.", MessageLevel::Error);
        }
        let mut caption = "No hay una última página registrada para mostrar.\nRevisa físicamente el papel y la cola de Windows.".to_string();
        let mut texture = None;
        if let Some((ref_item, page)) = recovery_reference(&self.items, id) {
            let render = || -> Result<printing::Raster, String> {
                let doc = Document::open(printing::pdfium().map_err(|e| e.to_string())?, Path::new(&ref_item.snapshot))
                    .map_err(|e| e.to_string())?;
                if page as u32 > doc.page_count() {
                    return Err("La página está fuera de la copia".into());
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
                    caption = format!(
                        "{} · Página {page}\nReferencia enviada al controlador; NO confirma salida física.",
                        ref_item.name
                    );
                }
                Err(e) => caption = format!("No se pudo abrir la vista previa: {e}"),
            }
        }
        self.recovery = Some(Recovery { item, page_text: "1".into(), caption, texture });
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(rec) = self.recovery.as_mut() else { return };
        let mut open = true;
        let mut action: Option<(Resolution, bool)> = None;
        egui::Window::new("Revisar interrupción · elegir dónde continuar")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_size([960.0, 600.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.columns(2, |cols| {
                    let ui = &mut cols[0];
                    ui.label(RichText::new("Última página esperada").size(18.0).strong());
                    ui.label(&rec.caption);
                    if let Some(t) = &rec.texture {
                        ui.add(egui::Image::new(t).max_size(egui::vec2(460.0, 480.0)));
                    }
                    ui.colored_label(
                        WARN,
                        "Compara esta página con la última hoja de papel que salió.\nSi imprimes a doble cara, comprueba ambos lados.",
                    );

                    let ui = &mut cols[1];
                    let it = &rec.item;
                    ui.label(RichText::new(&it.name).size(16.0).strong());
                    ui.label(format!(
                        "Impresora: {}\nTrabajo Windows: {}\nPágina registrada: {} / {}\n\nCompara el preview con el \
                         papel. Antes de reintentar, espera o cancela el trabajo anterior en Windows: podría seguir \
                         imprimiéndose.",
                        it.printer.as_deref().unwrap_or("sin registrar"),
                        it.job_id.as_deref().unwrap_or("no registrado"),
                        it.last_page,
                        it.total_pages.map_or("?".to_string(), |t| t.to_string()),
                    ));
                    if let Some(e) = &it.error {
                        ui.colored_label(DANGER, e);
                    }
                    ui.horizontal(|ui| {
                        ui.label("Primera página que falta:");
                        ui.add(egui::TextEdit::singleline(&mut rec.page_text).desired_width(60.0));
                    });
                    let w = ui.available_width();
                    let b = |ui: &mut egui::Ui, text: &str| ui.add_sized([w, 30.0], egui::Button::new(text)).clicked();
                    if b(ui, "Reanudar desde este PDF (página 1)") {
                        action = Some((Resolution::Retry, true));
                    }
                    if b(ui, "Reanudar desde la página indicada") {
                        action = Some((Resolution::Retry, false));
                    }
                    if b(ui, "Continuar con el siguiente PDF (este salió completo)") {
                        action = Some((Resolution::Sent, false));
                    }
                    if b(ui, "Omitir este PDF sin confirmar impresión") {
                        action = Some((Resolution::Skip, false));
                    }
                    ui.colored_label(MUTED, "Después pulsa Iniciar / continuar. Cerrar esta ventana conserva el bloqueo.");
                });
            });
        if let Some((res, from_first)) = action {
            let first = if res == Resolution::Retry && !from_first { rec.page_text.trim().parse::<i64>().unwrap_or(0) } else { 1 };
            let (id, total) = (rec.item.id, rec.item.total_pages);
            if first < 1 || total.map_or(false, |t| first > t) {
                info("No se pudo resolver", "Página fuera del documento.", MessageLevel::Error);
            } else if ask(
                "Confirmar revisión manual",
                "¿Confirmas que revisaste las hojas y la cola de Windows?\nEl programa no cancela trabajos anteriores automáticamente.",
            ) {
                match self.store.resolve(id, res, first) {
                    Ok(()) => {
                        self.recovery = None;
                        self.refresh();
                    }
                    Err(e) => info("No se pudo resolver", &e.to_string(), MessageLevel::Error),
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
                Msg::Batch(BatchEvent::Status(s)) => self.status = s,
                Msg::Batch(BatchEvent::Refresh) => self.refresh(),
                Msg::Batch(BatchEvent::Batch(done, target)) => {
                    self.batch_count = format!("Lote: {done} / {target} PDFs finalizados según Windows");
                    self.progress = (done, target.max(1));
                }
                Msg::Done(s) => {
                    self.busy = false;
                    self.status = s;
                    self.refresh();
                }
                Msg::Error(s) => {
                    self.busy = false;
                    self.status = s.clone();
                    self.refresh();
                    if !self.show_interruption(ctx) {
                        info(
                            "Proceso detenido de forma segura",
                            &format!("{s}\n\nNo se reintentará automáticamente."),
                            MessageLevel::Error,
                        );
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
            info(
                "Pausa solicitada",
                "Espera a que termine el documento o la operación actual. Luego podrás cerrar. Si Windows fuerza un \
                 cierre, el documento activo quedará por revisar al volver a abrir.",
                MessageLevel::Info,
            );
        }
        let idle = !self.busy && self.recovery.is_none();

        egui::TopBottomPanel::top("header").frame(egui::Frame::none().inner_margin(16.0).fill(ctx.style().visuals.panel_fill)).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("PDF-batch").size(26.0).strong().color(NAVY));
                ui.label(RichText::new(format!("v{}  /  Impresión ordenada", crate::VERSION)).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.link("Creado por siriusgoldsmith · GitHub").clicked() {
                        Self::open_shell("https://github.com/siriusgoldsmith/PDF-batch");
                    }
                });
            });
            ui.add_space(8.0);
            ui.add_enabled_ui(idle, |ui| {
                ui.horizontal(|ui| {
                    if ui.button("Añadir PDFs").clicked() {
                        self.add_files(ctx);
                    }
                    if ui.button("Añadir carpeta").clicked() {
                        self.add_folder(ctx);
                    }
                    if ui.button("Subir").clicked() {
                        self.move_selected(-1);
                    }
                    if ui.button("Bajar").clicked() {
                        self.move_selected(1);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Vaciar carpeta del lote").color(DANGER)).clicked() {
                            self.clear_folder(ctx);
                        }
                        if ui.button("Eliminar PDF").clicked() {
                            self.remove(ctx);
                        }
                    });
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(self.summary()).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.colored_label(MUTED, "Orden de impresión: de arriba hacia abajo");
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
                    Some(e) => format!("{} | {e}", r.source),
                    None => r.source.clone(),
                })
                .unwrap_or_else(|| "Añade PDFs y revisa el orden. Los originales se conservan.".into());
            ui.colored_label(MUTED, details);
            ui.add_space(6.0);
            egui::Frame::group(ui.style()).inner_margin(12.0).show(ui, |ui| {
                ui.label(RichText::new("Impresión").strong());
                ui.add_enabled_ui(idle, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Impresora");
                        egui::ComboBox::from_id_salt("printer")
                            .width(ui.available_width() - 260.0)
                            .selected_text(&self.printer)
                            .show_ui(ui, |ui| {
                                for p in self.printers.clone() {
                                    ui.selectable_value(&mut self.printer, p.clone(), p);
                                }
                            });
                        if ui.button("Actualizar").clicked() {
                            self.refresh_printers();
                        }
                        if ui.button("Ajustes Windows").clicked() {
                            Self::open_shell("ms-settings:printers");
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label("PDFs por lote");
                    ui.add_enabled_ui(idle, |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.batch_size).desired_width(70.0));
                        for v in ["20", "50", "100", "Todos"] {
                            if ui.small_button(v).clicked() {
                                self.batch_size = v.into();
                            }
                        }
                    });
                    ui.colored_label(MUTED, "Escribe una cantidad o elige Todos");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let start = egui::Button::new(RichText::new("Iniciar / continuar").strong().color(Color32::WHITE)).fill(NAVY);
                        if ui.add_enabled(idle, start).clicked() {
                            self.start(ctx);
                        }
                        let pausing = self.pause.load(Ordering::SeqCst);
                        if ui.add_enabled(self.busy && !pausing, egui::Button::new("Pausar tras documento")).clicked() {
                            self.request_pause();
                        }
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(&self.batch_count);
                    let (d, t) = self.progress;
                    ui.add(egui::ProgressBar::new(d as f32 / t as f32));
                });
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(idle, egui::Button::new("Resolver interrupción")).clicked() {
                    match self.single_selected() {
                        Ok(id) => self.open_recovery(ctx, id),
                        Err(e) => info("Seleccionar interrupción", &e, MessageLevel::Error),
                    }
                }
                ui.colored_label(MUTED, "Solo si hay un error o corte");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Exportar informe").clicked() {
                        self.export();
                    }
                    if ui.button("Abrir registro local").clicked() {
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
                .column(Column::exact(55.0))
                .column(Column::remainder().at_least(200.0).clip(true))
                .column(Column::exact(160.0))
                .column(Column::exact(200.0))
                .column(Column::exact(110.0))
                .header(30.0, |mut h| {
                    for t in ["Orden", "Documento", "Estado", "Resultado Windows", "Pág. enviadas"] {
                        h.col(|ui| {
                            ui.strong(t);
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
                        let total = it.total_pages.map_or("?".into(), |t| t.to_string());
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
