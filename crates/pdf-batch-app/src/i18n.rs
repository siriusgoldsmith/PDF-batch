//! Interface languages. Same model as NxExtractor's `Lang.cs`: one key, one text per
//! language, columns in the order of [`LANGUAGES`] (en, es, fr, ja, de).
//!
//! Queue states and audit events are stored in the database with fixed keys and are
//! only translated when shown, so a queue created in one language opens in any other.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

pub const LANGUAGES: [(&str, &str); 5] =
    [("en", "English"), ("es", "Español"), ("fr", "Français"), ("ja", "日本語"), ("de", "Deutsch")];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn current() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

pub fn code() -> &'static str {
    LANGUAGES[current()].0
}

pub fn set(index: usize) {
    CURRENT.store(index.min(LANGUAGES.len() - 1), Ordering::Relaxed);
}

fn index_of(code: &str) -> Option<usize> {
    LANGUAGES.iter().position(|(c, _)| c.eq_ignore_ascii_case(code))
}

/// Windows display language, if it is one of ours.
fn windows_language() -> Option<usize> {
    #[cfg(windows)]
    {
        // Primary language id of the user's UI language.
        let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
        return match id {
            0x09 => Some(0),
            0x0a => Some(1),
            0x0c => Some(2),
            0x11 => Some(3),
            0x07 => Some(4),
            _ => None,
        };
    }
    #[allow(unreachable_code)]
    None
}

fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("settings.json")
}

/// Saved choice, else the Windows language, else English.
pub fn init(data_dir: &Path) {
    let saved = std::fs::read_to_string(settings_path(data_dir))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("language").and_then(|l| l.as_str()).and_then(index_of));
    set(saved.or_else(windows_language).unwrap_or(0));
}

/// Language before the data folder is known (startup errors).
pub fn init_from_windows() {
    set(windows_language().unwrap_or(0));
}

pub fn save(data_dir: &Path) -> std::io::Result<()> {
    let _ = std::fs::create_dir_all(data_dir);
    std::fs::write(settings_path(data_dir), serde_json::json!({ "language": code() }).to_string())
}

pub fn t(key: &str) -> &'static str {
    let i = current();
    STRINGS.iter().find(|(k, _)| *k == key).map_or("?", |(_, v)| v[i])
}

/// `t(key)` with `{0}`, `{1}`… replaced by `args`.
pub fn f(key: &str, args: &[String]) -> String {
    let mut s = t(key).to_string();
    for (n, a) in args.iter().enumerate() {
        s = s.replace(&format!("{{{n}}}"), a);
    }
    s
}

/// `tf!(key, a, b)` → `f(key, &[a.to_string(), b.to_string()])`.
#[macro_export]
macro_rules! tf {
    ($key:expr $(, $arg:expr)* $(,)?) => {
        $crate::i18n::f($key, &[$($arg.to_string()),*])
    };
}

/// Words accepted as "all documents" in the batch-size box, in any language.
pub fn is_all(s: &str) -> bool {
    let s = s.trim();
    STRINGS.iter().find(|(k, _)| *k == "all").map_or(false, |(_, v)| v.iter().any(|w| w.eq_ignore_ascii_case(s)))
}

/// Translate messages that come from the print engine and the queue (stored in the
/// database in Spanish, as in v1.x). Unknown texts are returned unchanged.
pub fn engine(s: &str) -> String {
    let job = |s: &str| s.strip_prefix("Trabajo ").and_then(|r| r.split(':').next()).unwrap_or("?").to_string();
    let between = |s: &str, a: &str, b: &str| -> String {
        s.split_once(a).map(|(_, r)| r.split(b).next().unwrap_or(r)).unwrap_or("").to_string()
    };
    if let Some(rest) = s.strip_prefix("Enviando #") {
        if let Some((pos, name)) = rest.split_once(": ") {
            return tf!("engine.sending", pos, name);
        }
    }
    if s.starts_with("Trabajo ") {
        if s.contains("tiempo de espera agotado") {
            return tf!("engine.jobTimeout", job(s));
        }
        if s.contains("error de impresión (estado ") {
            return tf!("engine.jobError", job(s), between(s, "(estado ", ")"));
        }
        if s.contains("no se pudo consultar la cola: ") {
            return tf!("engine.jobQuery", job(s), between(s, "no se pudo consultar la cola: ", "\u{0}"));
        }
        if s.contains("fallo al enviar el PDF: ") {
            return tf!("engine.jobSend", job(s), between(s, "fallo al enviar el PDF: ", ". Revise"));
        }
    }
    let exact = [
        ("Detenido por el usuario.", "engine.stopped"),
        ("La impresora informa un bloqueo o error", "engine.printerBlocked"),
        ("Interrupted while in flight", "engine.interrupted"),
        ("Snapshot SHA256 mismatch", "engine.snapshotMismatch"),
        ("No se puede eliminar mientras hay una impresión en curso.", "engine.deleteWhilePrinting"),
        ("Debe resolver los PDF con resultado incierto", "engine.resolveBeforeDelete"),
        ("El PDF seleccionado no existe en la cola.", "engine.notInQueue"),
        ("Impresión física incompleta.", "engine.physicalIncomplete"),
        ("Only pending items can be moved", "engine.onlyPendingMove"),
        ("Cannot edit queue while an item is in flight", "engine.editWhilePrinting"),
    ];
    for (prefix, key) in exact {
        if s.starts_with(prefix) {
            return t(key).to_string();
        }
    }
    s.to_string()
}

/// Columns: en, es, fr, ja, de.
pub static STRINGS: &[(&str, [&str; 5])] = &[
    // ---------- General ----------
    ("language", ["Language", "Idioma", "Langue", "言語", "Sprache"]),
    ("subtitle", ["v{0}  /  Ordered printing", "v{0}  /  Impresión ordenada", "v{0}  /  Impression ordonnée", "v{0}  /  順序どおりの印刷", "v{0}  /  Geordneter Druck"]),
    ("createdBy", ["Created by siriusgoldsmith · GitHub", "Creado por siriusgoldsmith · GitHub", "Créé par siriusgoldsmith · GitHub", "作成: siriusgoldsmith · GitHub", "Erstellt von siriusgoldsmith · GitHub"]),
    ("ready", [
        "Ready. Progress is saved automatically; you don't need to confirm each PDF.",
        "Listo. El avance se guarda automáticamente; no necesitas confirmar cada PDF.",
        "Prêt. La progression est enregistrée automatiquement ; inutile de confirmer chaque PDF.",
        "準備完了。進行状況は自動で保存されます。PDF ごとの確認は不要です。",
        "Bereit. Der Fortschritt wird automatisch gespeichert; Sie müssen nicht jedes PDF bestätigen.",
    ]),
    ("readQueueFailed", ["Could not read the queue: {0}", "No se pudo leer la cola: {0}", "Impossible de lire la file : {0}", "キューを読み込めませんでした: {0}", "Die Warteschlange konnte nicht gelesen werden: {0}"]),
    ("noPrinters", [
        "No printers available. Install one in Windows and click Refresh.",
        "No hay impresoras disponibles. Instala una en Windows y pulsa Actualizar.",
        "Aucune imprimante disponible. Installez-en une dans Windows puis cliquez sur Actualiser.",
        "使用できるプリンターがありません。Windows でインストールしてから「更新」を押してください。",
        "Keine Drucker verfügbar. Installieren Sie einen in Windows und klicken Sie auf Aktualisieren.",
    ]),
    ("summary", [
        "{0} PDFs · {1} pending · {2} finished · {3} in progress · {4} to review",
        "{0} PDFs · {1} pendientes · {2} finalizados · {3} en proceso · {4} por revisar",
        "{0} PDF · {1} en attente · {2} terminés · {3} en cours · {4} à vérifier",
        "{0} 件の PDF · 未処理 {1} · 完了 {2} · 処理中 {3} · 要確認 {4}",
        "{0} PDFs · {1} ausstehend · {2} fertig · {3} in Bearbeitung · {4} zu prüfen",
    ]),
    // ---------- States ----------
    ("state.pending", ["Pending", "Pendiente", "En attente", "未処理", "Ausstehend"]),
    ("state.in_flight", ["In progress", "En proceso", "En cours", "処理中", "In Bearbeitung"]),
    ("state.uncertain", ["Review interruption", "Revisar interrupción", "Vérifier l’interruption", "中断を確認", "Unterbrechung prüfen"]),
    ("state.sent", ["Finished", "Finalizado", "Terminé", "完了", "Fertig"]),
    ("state.skipped", ["Skipped", "Omitido", "Ignoré", "スキップ", "Übersprungen"]),
    ("state.error", ["Error", "Error", "Erreur", "エラー", "Fehler"]),
    ("outcome.completed", ["Finished according to Windows", "Finalizado según Windows", "Terminé selon Windows", "Windows 上で完了", "Laut Windows fertig"]),
    ("outcome.review", ["Error / to review", "Error / por revisar", "Erreur / à vérifier", "エラー / 要確認", "Fehler / zu prüfen"]),
    ("outcome.none", ["No result recorded", "Sin resultado registrado", "Aucun résultat enregistré", "結果の記録なし", "Kein Ergebnis erfasst"]),
    // ---------- Table ----------
    ("col.order", ["Order", "Orden", "Ordre", "順番", "Reihenfolge"]),
    ("col.document", ["Document", "Documento", "Document", "ドキュメント", "Dokument"]),
    ("col.state", ["Status", "Estado", "État", "状態", "Status"]),
    ("col.outcome", ["Windows result", "Resultado Windows", "Résultat Windows", "Windows の結果", "Windows-Ergebnis"]),
    ("col.pages", ["Pages sent", "Pág. enviadas", "Pages envoyées", "送信ページ", "Gesendete Seiten"]),
    // ---------- Toolbar ----------
    ("addPdfs", ["Add PDFs", "Añadir PDFs", "Ajouter des PDF", "PDF を追加", "PDFs hinzufügen"]),
    ("addFolder", ["Add folder", "Añadir carpeta", "Ajouter un dossier", "フォルダーを追加", "Ordner hinzufügen"]),
    ("moveUp", ["Move up", "Subir", "Monter", "上へ", "Nach oben"]),
    ("moveDown", ["Move down", "Bajar", "Descendre", "下へ", "Nach unten"]),
    ("clearFolder", ["Clear batch folder", "Vaciar carpeta del lote", "Vider le dossier du lot", "バッチフォルダーを空にする", "Stapelordner leeren"]),
    ("removePdf", ["Remove PDF", "Eliminar PDF", "Retirer le PDF", "PDF を削除", "PDF entfernen"]),
    ("printOrder", ["Print order: top to bottom", "Orden de impresión: de arriba hacia abajo", "Ordre d’impression : de haut en bas", "印刷順: 上から下", "Druckreihenfolge: von oben nach unten"]),
    // ---------- Footer ----------
    ("footerHint", [
        "Add PDFs and check the order. The originals are kept.",
        "Añade PDFs y revisa el orden. Los originales se conservan.",
        "Ajoutez des PDF et vérifiez l’ordre. Les originaux sont conservés.",
        "PDF を追加して順番を確認してください。元のファイルは保持されます。",
        "Fügen Sie PDFs hinzu und prüfen Sie die Reihenfolge. Die Originale bleiben erhalten.",
    ]),
    ("printing", ["Printing", "Impresión", "Impression", "印刷", "Drucken"]),
    ("printer", ["Printer", "Impresora", "Imprimante", "プリンター", "Drucker"]),
    ("refresh", ["Refresh", "Actualizar", "Actualiser", "更新", "Aktualisieren"]),
    ("windowsSettings", ["Windows settings", "Ajustes Windows", "Paramètres Windows", "Windows の設定", "Windows-Einstellungen"]),
    ("perBatch", ["PDFs per batch", "PDFs por lote", "PDF par lot", "1 回の PDF 数", "PDFs pro Stapel"]),
    ("all", ["All", "Todos", "Tous", "すべて", "Alle"]),
    ("countHint", ["Type a number or choose All", "Escribe una cantidad o elige Todos", "Saisissez un nombre ou choisissez Tous", "数を入力するか「すべて」を選択", "Anzahl eingeben oder Alle wählen"]),
    ("start", ["Start / continue", "Iniciar / continuar", "Démarrer / continuer", "開始 / 続行", "Starten / fortsetzen"]),
    ("pauseAfter", ["Pause after document", "Pausar tras documento", "Pause après le document", "このドキュメントの後で一時停止", "Nach Dokument pausieren"]),
    ("batchNotStarted", ["Batch: not started", "Lote: sin iniciar", "Lot : non démarré", "バッチ: 未開始", "Stapel: nicht gestartet"]),
    ("batchProgress", [
        "Batch: {0} / {1} PDFs finished according to Windows",
        "Lote: {0} / {1} PDFs finalizados según Windows",
        "Lot : {0} / {1} PDF terminés selon Windows",
        "バッチ: Windows 上で {0} / {1} 件の PDF が完了",
        "Stapel: {0} / {1} PDFs laut Windows fertig",
    ]),
    ("resolve", ["Resolve interruption", "Resolver interrupción", "Résoudre l’interruption", "中断を解決", "Unterbrechung beheben"]),
    ("resolveHint", ["Only after an error or power cut", "Solo si hay un error o corte", "Uniquement en cas d’erreur ou de coupure", "エラーや中断があった場合のみ", "Nur nach einem Fehler oder Abbruch"]),
    ("exportReport", ["Export report", "Exportar informe", "Exporter le rapport", "レポートをエクスポート", "Bericht exportieren"]),
    ("openLog", ["Open local log", "Abrir registro local", "Ouvrir le registre local", "ローカル記録を開く", "Lokales Protokoll öffnen"]),
    // ---------- Adding ----------
    ("dlgAddFiles", ["Add PDFs: check the order in the list afterwards", "Añadir PDFs: revisa luego el orden en la lista", "Ajouter des PDF : vérifiez ensuite l’ordre dans la liste", "PDF を追加: 追加後に一覧で順番を確認してください", "PDFs hinzufügen: danach die Reihenfolge in der Liste prüfen"]),
    ("dlgAddFolder", ["Add PDFs from a folder (sorted by name, no subfolders)", "Añadir PDFs de una carpeta (orden por nombre, sin subcarpetas)", "Ajouter les PDF d’un dossier (tri par nom, sans sous-dossiers)", "フォルダーから PDF を追加 (名前順、サブフォルダーは除く)", "PDFs aus einem Ordner hinzufügen (nach Name, ohne Unterordner)"]),
    ("emptyFolderTitle", ["Empty folder", "Carpeta vacía", "Dossier vide", "空のフォルダー", "Leerer Ordner"]),
    ("emptyFolderText", ["There are no PDF files in this folder.", "No hay archivos PDF en esta carpeta.", "Ce dossier ne contient aucun fichier PDF.", "このフォルダーには PDF ファイルがありません。", "Dieser Ordner enthält keine PDF-Dateien."]),
    ("validating", ["Checking PDFs and saving batch copies…", "Validando PDFs y guardando copias del lote…", "Vérification des PDF et enregistrement des copies du lot…", "PDF を確認し、バッチ用のコピーを保存しています…", "PDFs werden geprüft und Stapelkopien gespeichert…"]),
    ("invalidPdf", ["Invalid or password-protected PDF: {0}", "PDF inválido o protegido con contraseña: {0}", "PDF non valide ou protégé par mot de passe : {0}", "無効またはパスワード保護された PDF: {0}", "Ungültiges oder passwortgeschütztes PDF: {0}"]),
    ("emptyPdf", ["Invalid or empty PDF: {0}", "PDF inválido o vacío: {0}", "PDF non valide ou vide : {0}", "無効または空の PDF: {0}", "Ungültiges oder leeres PDF: {0}"]),
    ("damagedPdf", ["Damaged PDF: {0}", "PDF dañado: {0}", "PDF endommagé : {0}", "破損した PDF: {0}", "Beschädigtes PDF: {0}"]),
    ("added", ["{0} PDFs added to the end of the list.", "Se añadieron {0} PDFs al final de la lista.", "{0} PDF ajoutés à la fin de la liste.", "{0} 件の PDF を一覧の末尾に追加しました。", "{0} PDFs am Ende der Liste hinzugefügt."]),
    // ---------- Moving / removing ----------
    ("selectOne", ["Select a single document.", "Selecciona un solo documento.", "Sélectionnez un seul document.", "ドキュメントを 1 つだけ選択してください。", "Wählen Sie genau ein Dokument aus."]),
    ("moveFailed", ["Could not move", "No se pudo mover", "Déplacement impossible", "移動できませんでした", "Verschieben nicht möglich"]),
    ("removeTitle", ["Remove PDF from batch", "Eliminar PDF del lote", "Retirer le PDF du lot", "バッチから PDF を削除", "PDF aus dem Stapel entfernen"]),
    ("removeText", [
        "Remove {0} selected PDF(s) from the list along with their local copies?\n\nThe original files and the history are kept. Unresolved interruptions cannot be removed.",
        "¿Eliminar {0} PDF(s) seleccionados de la lista y sus copias locales?\n\nLos archivos originales y el historial se conservan. No se pueden eliminar interrupciones sin resolver.",
        "Retirer de la liste les {0} PDF sélectionnés et leurs copies locales ?\n\nLes fichiers originaux et l’historique sont conservés. Les interruptions non résolues ne peuvent pas être retirées.",
        "選択した {0} 件の PDF とそのローカルコピーを一覧から削除しますか?\n\n元のファイルと履歴は保持されます。未解決の中断は削除できません。",
        "{0} ausgewählte PDF(s) samt lokalen Kopien aus der Liste entfernen?\n\nDie Originaldateien und der Verlauf bleiben erhalten. Ungelöste Unterbrechungen können nicht entfernt werden.",
    ]),
    ("removing", ["Removing PDFs from the batch…", "Eliminando PDFs del lote…", "Retrait des PDF du lot…", "バッチから PDF を削除しています…", "PDFs werden aus dem Stapel entfernt…"]),
    ("removed", ["{0} PDF(s) removed from the batch. Originals kept.", "{0} PDF(s) eliminados del lote. Originales conservados.", "{0} PDF retirés du lot. Originaux conservés.", "{0} 件の PDF をバッチから削除しました。元のファイルは保持されています。", "{0} PDF(s) aus dem Stapel entfernt. Originale erhalten."]),
    ("clearTitle", ["Clear batch folder", "Vaciar carpeta del lote", "Vider le dossier du lot", "バッチフォルダーを空にする", "Stapelordner leeren"]),
    ("clearText", [
        "Empty the list and delete its local PDF copies?\n\nThe source folder and its files will NOT be deleted. The history is kept. Resolve any interruption first.",
        "¿Vaciar la lista y eliminar sus copias locales de PDF?\n\nNO se borrará la carpeta de origen ni sus archivos. El historial se conserva. Resuelve primero cualquier interrupción.",
        "Vider la liste et supprimer ses copies locales de PDF ?\n\nLe dossier source et ses fichiers ne seront PAS supprimés. L’historique est conservé. Résolvez d’abord toute interruption.",
        "一覧を空にし、PDF のローカルコピーを削除しますか?\n\n元のフォルダーとファイルは削除されません。履歴は保持されます。先に中断をすべて解決してください。",
        "Liste leeren und die lokalen PDF-Kopien löschen?\n\nDer Quellordner und seine Dateien werden NICHT gelöscht. Der Verlauf bleibt erhalten. Beheben Sie zuerst alle Unterbrechungen.",
    ]),
    ("clearing", ["Clearing the batch folder…", "Vaciando la carpeta del lote…", "Vidage du dossier du lot…", "バッチフォルダーを空にしています…", "Stapelordner wird geleert…"]),
    ("cleared", ["Batch folder cleared: {0} PDF(s). Originals kept.", "Carpeta del lote vaciada: {0} PDF(s). Originales conservados.", "Dossier du lot vidé : {0} PDF. Originaux conservés.", "バッチフォルダーを空にしました: {0} 件の PDF。元のファイルは保持されています。", "Stapelordner geleert: {0} PDF(s). Originale erhalten."]),
    // ---------- Printing ----------
    ("noPrinterTitle", ["No printer", "Sin impresora", "Aucune imprimante", "プリンターなし", "Kein Drucker"]),
    ("noPrinterText", ["Select a printer.", "Selecciona una impresora.", "Sélectionnez une imprimante.", "プリンターを選択してください。", "Wählen Sie einen Drucker aus."]),
    ("noPendingTitle", ["Nothing pending", "Sin pendientes disponibles", "Rien en attente", "未処理のものがありません", "Nichts ausstehend"]),
    ("noPendingText", [
        "The batch is finished, or there is an interruption you must resolve before continuing.",
        "El lote terminó o hay una interrupción que debes resolver antes de continuar.",
        "Le lot est terminé, ou une interruption doit être résolue avant de continuer.",
        "バッチが完了したか、続行する前に解決が必要な中断があります。",
        "Der Stapel ist fertig, oder eine Unterbrechung muss vor dem Fortsetzen behoben werden.",
    ]),
    ("badCountTitle", ["Invalid number", "Cantidad no válida", "Nombre non valide", "無効な数", "Ungültige Anzahl"]),
    ("badCountText", [
        "Type a whole number greater than zero (for example, 35) or choose All.",
        "Escribe una cantidad entera mayor que cero (por ejemplo, 35) o selecciona Todos.",
        "Saisissez un nombre entier supérieur à zéro (par exemple 35) ou choisissez Tous.",
        "0 より大きい整数 (例: 35) を入力するか、「すべて」を選択してください。",
        "Geben Sie eine ganze Zahl größer als null ein (zum Beispiel 35) oder wählen Sie Alle.",
    ]),
    ("confirmPrintTitle", ["Confirm printing", "Confirmar impresión", "Confirmer l’impression", "印刷の確認", "Druck bestätigen"]),
    ("confirmPrintText", [
        "Print the next {0} PDFs on “{1}”?\n\nOne copy per document, fitted to the printable area. Paper, tray, colour and duplex: driver defaults.\n\nCheck the settings and the order before accepting. Do not send the same batch from another application.",
        "¿Imprimir los próximos {0} PDFs en «{1}»?\n\nUna copia por documento, ajustada al área imprimible. Papel, bandeja, color y dúplex: valores predeterminados del controlador.\n\nRevisa los ajustes y el orden antes de aceptar. No envíes el mismo lote desde otra aplicación.",
        "Imprimer les {0} prochains PDF sur « {1} » ?\n\nUne copie par document, ajustée à la zone imprimable. Papier, bac, couleur et recto verso : valeurs par défaut du pilote.\n\nVérifiez les réglages et l’ordre avant d’accepter. N’envoyez pas le même lot depuis une autre application.",
        "次の {0} 件の PDF を「{1}」で印刷しますか?\n\n各ドキュメント 1 部、印刷可能領域に合わせて縮小します。用紙、トレイ、カラー、両面はドライバーの既定値を使用します。\n\n承認する前に設定と順番を確認してください。同じバッチを他のアプリから送信しないでください。",
        "Die nächsten {0} PDFs auf „{1}“ drucken?\n\nEine Kopie pro Dokument, an den druckbaren Bereich angepasst. Papier, Fach, Farbe und Duplex: Standardwerte des Treibers.\n\nPrüfen Sie Einstellungen und Reihenfolge, bevor Sie bestätigen. Senden Sie denselben Stapel nicht aus einer anderen Anwendung.",
    ]),
    ("starting", ["Starting batch…", "Iniciando lote…", "Démarrage du lot…", "バッチを開始しています…", "Stapel wird gestartet…"]),
    ("batchFinished", [
        "Batch finished: {0} / {1} PDFs finished according to Windows. Click Start / continue for the next batch.",
        "Lote terminado: {0} / {1} PDFs finalizados según Windows. Pulsa Iniciar / continuar para el siguiente lote.",
        "Lot terminé : {0} / {1} PDF terminés selon Windows. Cliquez sur Démarrer / continuer pour le lot suivant.",
        "バッチ完了: Windows 上で {0} / {1} 件の PDF が完了しました。次のバッチは「開始 / 続行」を押してください。",
        "Stapel fertig: {0} / {1} PDFs laut Windows fertig. Klicken Sie auf Starten / fortsetzen für den nächsten Stapel.",
    ]),
    ("batchPaused", [
        "Batch paused: {0} / {1} PDFs finished according to Windows. Click Start / continue for the next batch.",
        "Lote pausado: {0} / {1} PDFs finalizados según Windows. Pulsa Iniciar / continuar para el siguiente lote.",
        "Lot en pause : {0} / {1} PDF terminés selon Windows. Cliquez sur Démarrer / continuer pour le lot suivant.",
        "バッチを一時停止: Windows 上で {0} / {1} 件の PDF が完了しました。次のバッチは「開始 / 続行」を押してください。",
        "Stapel pausiert: {0} / {1} PDFs laut Windows fertig. Klicken Sie auf Starten / fortsetzen für den nächsten Stapel.",
    ]),
    ("pauseRequested", [
        "Pause requested. The current PDF will finish before the batch stops.",
        "Pausa solicitada. Se terminará el PDF actual antes de detener el lote.",
        "Pause demandée. Le PDF en cours sera terminé avant l’arrêt du lot.",
        "一時停止を要求しました。現在の PDF が終わってからバッチを停止します。",
        "Pause angefordert. Das aktuelle PDF wird fertiggestellt, bevor der Stapel anhält.",
    ]),
    ("pauseTitle", ["Pause requested", "Pausa solicitada", "Pause demandée", "一時停止を要求しました", "Pause angefordert"]),
    ("closeBusyText", [
        "Wait for the current document or operation to finish. Then you can close. If Windows forces the app to close, the active document will need review when you reopen it.",
        "Espera a que termine el documento o la operación actual. Luego podrás cerrar. Si Windows fuerza un cierre, el documento activo quedará por revisar al volver a abrir.",
        "Attendez la fin du document ou de l’opération en cours. Vous pourrez ensuite fermer. Si Windows force la fermeture, le document actif devra être vérifié à la réouverture.",
        "現在のドキュメントまたは操作が終わるまでお待ちください。その後で閉じられます。Windows が強制終了した場合、次回起動時に処理中だったドキュメントの確認が必要になります。",
        "Warten Sie, bis das aktuelle Dokument oder der aktuelle Vorgang abgeschlossen ist. Danach können Sie schließen. Erzwingt Windows das Schließen, muss das aktive Dokument beim nächsten Öffnen geprüft werden.",
    ]),
    ("stoppedTitle", ["Process stopped safely", "Proceso detenido de forma segura", "Processus arrêté en toute sécurité", "処理を安全に停止しました", "Vorgang sicher angehalten"]),
    ("stoppedText", ["{0}\n\nIt will not be retried automatically.", "{0}\n\nNo se reintentará automáticamente.", "{0}\n\nAucune nouvelle tentative automatique.", "{0}\n\n自動での再試行は行いません。", "{0}\n\nEs wird nicht automatisch erneut versucht."]),
    // ---------- Export ----------
    ("exportTitle", ["Export log", "Exportar registro", "Exporter le registre", "記録をエクスポート", "Protokoll exportieren"]),
    ("exportFileName", ["pdf-batch-report.json", "informe-pdf-batch.json", "rapport-pdf-batch.json", "pdf-batch-report.json", "pdf-batch-bericht.json"]),
    ("exported", ["Report exported: {0}", "Informe exportado: {0}", "Rapport exporté : {0}", "レポートをエクスポートしました: {0}", "Bericht exportiert: {0}"]),
    // ---------- Recovery ----------
    ("selectInterruption", ["Select interruption", "Seleccionar interrupción", "Sélectionner l’interruption", "中断を選択", "Unterbrechung auswählen"]),
    ("notInterrupted", ["This document has no interruption waiting to be resolved.", "Este documento no tiene una interrupción pendiente de resolver.", "Ce document n’a aucune interruption à résoudre.", "このドキュメントには解決待ちの中断がありません。", "Dieses Dokument hat keine offene Unterbrechung."]),
    ("noLastPage", [
        "There is no recorded last page to show.\nCheck the paper and the Windows queue yourself.",
        "No hay una última página registrada para mostrar.\nRevisa físicamente el papel y la cola de Windows.",
        "Aucune dernière page enregistrée à afficher.\nVérifiez vous-même le papier et la file Windows.",
        "表示できる最後のページの記録がありません。\n用紙と Windows の印刷キューを直接確認してください。",
        "Es ist keine letzte Seite erfasst.\nPrüfen Sie selbst das Papier und die Windows-Warteschlange.",
    ]),
    ("pageOutsideCopy", ["The page is outside the copy", "La página está fuera de la copia", "La page est hors de la copie", "ページがコピーの範囲外です", "Die Seite liegt außerhalb der Kopie"]),
    ("recoveryCaption", [
        "{0} · Page {1}\nReference sent to the driver; it does NOT confirm physical output.",
        "{0} · Página {1}\nReferencia enviada al controlador; NO confirma salida física.",
        "{0} · Page {1}\nRéférence envoyée au pilote ; elle ne confirme PAS la sortie papier.",
        "{0} · {1} ページ\nドライバーに送信された参照です。実際に印刷されたことは確認できません。",
        "{0} · Seite {1}\nAn den Treiber gesendete Referenz; bestätigt KEINE physische Ausgabe.",
    ]),
    ("previewFailed", ["Could not open the preview: {0}", "No se pudo abrir la vista previa: {0}", "Impossible d’ouvrir l’aperçu : {0}", "プレビューを開けませんでした: {0}", "Vorschau konnte nicht geöffnet werden: {0}"]),
    ("recoveryTitle", ["Review interruption · choose where to continue", "Revisar interrupción · elegir dónde continuar", "Vérifier l’interruption · choisir où reprendre", "中断を確認 · 再開位置を選択", "Unterbrechung prüfen · Fortsetzungspunkt wählen"]),
    ("expectedPage", ["Expected last page", "Última página esperada", "Dernière page attendue", "最後に印刷されたはずのページ", "Erwartete letzte Seite"]),
    ("compareHint", [
        "Compare this page with the last sheet of paper that came out.\nIf you print double-sided, check both sides.",
        "Compara esta página con la última hoja de papel que salió.\nSi imprimes a doble cara, comprueba ambos lados.",
        "Comparez cette page avec la dernière feuille sortie.\nEn recto verso, vérifiez les deux faces.",
        "このページを最後に出てきた用紙と比べてください。\n両面印刷の場合は両面を確認してください。",
        "Vergleichen Sie diese Seite mit dem zuletzt ausgegebenen Blatt.\nBei Duplexdruck prüfen Sie beide Seiten.",
    ]),
    ("recoveryDetails", [
        "Printer: {0}\nWindows job: {1}\nRecorded page: {2} / {3}\n\nCompare the preview with the paper. Before retrying, wait for or cancel the previous job in Windows: it might still be printing.",
        "Impresora: {0}\nTrabajo Windows: {1}\nPágina registrada: {2} / {3}\n\nCompara el preview con el papel. Antes de reintentar, espera o cancela el trabajo anterior en Windows: podría seguir imprimiéndose.",
        "Imprimante : {0}\nTâche Windows : {1}\nPage enregistrée : {2} / {3}\n\nComparez l’aperçu avec le papier. Avant de réessayer, attendez ou annulez la tâche précédente dans Windows : elle pourrait encore s’imprimer.",
        "プリンター: {0}\nWindows のジョブ: {1}\n記録されたページ: {2} / {3}\n\nプレビューと用紙を比べてください。再試行する前に、Windows で前のジョブが終わるのを待つかキャンセルしてください。まだ印刷中の可能性があります。",
        "Drucker: {0}\nWindows-Auftrag: {1}\nErfasste Seite: {2} / {3}\n\nVergleichen Sie die Vorschau mit dem Papier. Warten Sie vor einem neuen Versuch auf den vorherigen Auftrag in Windows oder brechen Sie ihn ab: Er könnte noch gedruckt werden.",
    ]),
    ("notRecorded", ["not recorded", "sin registrar", "non enregistré", "記録なし", "nicht erfasst"]),
    ("firstMissing", ["First missing page:", "Primera página que falta:", "Première page manquante :", "最初の未印刷ページ:", "Erste fehlende Seite:"]),
    ("resumeFirst", ["Resume from this PDF (page 1)", "Reanudar desde este PDF (página 1)", "Reprendre à ce PDF (page 1)", "この PDF から再開 (1 ページ目)", "Ab diesem PDF fortsetzen (Seite 1)"]),
    ("resumeFrom", ["Resume from the page entered", "Reanudar desde la página indicada", "Reprendre à la page indiquée", "入力したページから再開", "Ab der angegebenen Seite fortsetzen"]),
    ("continueNext", ["Continue with the next PDF (this one printed completely)", "Continuar con el siguiente PDF (este salió completo)", "Passer au PDF suivant (celui-ci est sorti en entier)", "次の PDF に進む (この PDF は全部印刷済み)", "Mit dem nächsten PDF fortfahren (dieses wurde vollständig gedruckt)"]),
    ("skipPdf", ["Skip this PDF without confirming it printed", "Omitir este PDF sin confirmar impresión", "Ignorer ce PDF sans confirmer l’impression", "印刷を確認せずにこの PDF をスキップ", "Dieses PDF überspringen, ohne den Druck zu bestätigen"]),
    ("afterResolve", [
        "Then click Start / continue. Closing this window keeps the batch blocked.",
        "Después pulsa Iniciar / continuar. Cerrar esta ventana conserva el bloqueo.",
        "Cliquez ensuite sur Démarrer / continuer. Fermer cette fenêtre laisse le lot bloqué.",
        "その後「開始 / 続行」を押してください。このウィンドウを閉じてもブロックは解除されません。",
        "Klicken Sie danach auf Starten / fortsetzen. Wenn Sie dieses Fenster schließen, bleibt der Stapel blockiert.",
    ]),
    ("resolveFailed", ["Could not resolve", "No se pudo resolver", "Résolution impossible", "解決できませんでした", "Konnte nicht behoben werden"]),
    ("pageOutsideDoc", ["Page outside the document.", "Página fuera del documento.", "Page hors du document.", "ページがドキュメントの範囲外です。", "Seite außerhalb des Dokuments."]),
    ("confirmReviewTitle", ["Confirm manual review", "Confirmar revisión manual", "Confirmer la vérification manuelle", "手動確認の確認", "Manuelle Prüfung bestätigen"]),
    ("confirmReviewText", [
        "Do you confirm that you checked the sheets and the Windows queue?\nThe program does not cancel previous jobs automatically.",
        "¿Confirmas que revisaste las hojas y la cola de Windows?\nEl programa no cancela trabajos anteriores automáticamente.",
        "Confirmez-vous avoir vérifié les feuilles et la file Windows ?\nLe programme n’annule pas automatiquement les tâches précédentes.",
        "用紙と Windows の印刷キューを確認しましたか?\nこのプログラムは前のジョブを自動ではキャンセルしません。",
        "Bestätigen Sie, dass Sie die Blätter und die Windows-Warteschlange geprüft haben?\nDas Programm bricht frühere Aufträge nicht automatisch ab.",
    ]),
    // ---------- Startup ----------
    ("alreadyOpen", ["This batch is already open in another window or Windows session.", "Este lote ya está abierto en otra ventana o sesión de Windows.", "Ce lot est déjà ouvert dans une autre fenêtre ou session Windows.", "このバッチは別のウィンドウまたは Windows セッションで既に開かれています。", "Dieser Stapel ist bereits in einem anderen Fenster oder einer anderen Windows-Sitzung geöffnet."]),
    ("v1Open", ["PDFLote v1.x is open. Close it so PDF-batch can migrate the queue.", "PDFLote v1.x está abierto. Ciérralo para que PDF-batch pueda migrar la cola.", "PDFLote v1.x est ouvert. Fermez-le pour que PDF-batch puisse migrer la file.", "PDFLote v1.x が開いています。PDF-batch がキューを移行できるよう閉じてください。", "PDFLote v1.x ist geöffnet. Schließen Sie es, damit PDF-batch die Warteschlange übernehmen kann."]),
    ("startFailed", ["PDF-batch could not start", "PDF-batch no pudo iniciar", "PDF-batch n’a pas pu démarrer", "PDF-batch を起動できませんでした", "PDF-batch konnte nicht starten"]),
    // ---------- Engine messages (see `engine`) ----------
    ("engine.sending", ["Sending #{0}: {1}", "Enviando #{0}: {1}", "Envoi n° {0} : {1}", "送信中 #{0}: {1}", "Sende #{0}: {1}"]),
    ("engine.jobTimeout", [
        "Job {0}: timed out. Check the queue and the printer before retrying; the job was not cancelled.",
        "Trabajo {0}: tiempo de espera agotado. Revise la cola y la impresora antes de reintentar; no se canceló el trabajo.",
        "Tâche {0} : délai dépassé. Vérifiez la file et l’imprimante avant de réessayer ; la tâche n’a pas été annulée.",
        "ジョブ {0}: タイムアウトしました。再試行する前にキューとプリンターを確認してください。ジョブはキャンセルされていません。",
        "Auftrag {0}: Zeitüberschreitung. Prüfen Sie Warteschlange und Drucker vor einem neuen Versuch; der Auftrag wurde nicht abgebrochen.",
    ]),
    ("engine.jobError", [
        "Job {0}: printing error (status {1}). Check the queue and the printer before retrying; some pages may have printed.",
        "Trabajo {0}: error de impresión (estado {1}). Revise la cola y la impresora antes de reintentar; puede haber páginas impresas.",
        "Tâche {0} : erreur d’impression (état {1}). Vérifiez la file et l’imprimante avant de réessayer ; des pages ont pu être imprimées.",
        "ジョブ {0}: 印刷エラー (状態 {1})。再試行する前にキューとプリンターを確認してください。一部のページが印刷されている可能性があります。",
        "Auftrag {0}: Druckfehler (Status {1}). Prüfen Sie Warteschlange und Drucker vor einem neuen Versuch; einige Seiten könnten gedruckt sein.",
    ]),
    ("engine.jobQuery", ["Job {0}: could not query the queue: {1}", "Trabajo {0}: no se pudo consultar la cola: {1}", "Tâche {0} : impossible d’interroger la file : {1}", "ジョブ {0}: キューを照会できませんでした: {1}", "Auftrag {0}: Warteschlange konnte nicht abgefragt werden: {1}"]),
    ("engine.jobSend", [
        "Job {0}: failed to send the PDF: {1}. Check the queue and the printed pages before retrying; some pages may have printed.",
        "Trabajo {0}: fallo al enviar el PDF: {1}. Revise la cola y las páginas físicas antes de reintentar; puede haber páginas impresas.",
        "Tâche {0} : échec de l’envoi du PDF : {1}. Vérifiez la file et les pages imprimées avant de réessayer ; des pages ont pu être imprimées.",
        "ジョブ {0}: PDF を送信できませんでした: {1}。再試行する前にキューと印刷済みの用紙を確認してください。一部のページが印刷されている可能性があります。",
        "Auftrag {0}: PDF konnte nicht gesendet werden: {1}. Prüfen Sie Warteschlange und gedruckte Seiten vor einem neuen Versuch; einige Seiten könnten gedruckt sein.",
    ]),
    ("engine.stopped", [
        "Stopped by the user. Check the queue before retrying; the job may still be printing. No queued job was cancelled.",
        "Detenido por el usuario. Revise la cola antes de reintentar; el trabajo puede seguir imprimiéndose. No se canceló ningún trabajo de la cola.",
        "Arrêté par l’utilisateur. Vérifiez la file avant de réessayer ; la tâche peut encore s’imprimer. Aucune tâche n’a été annulée.",
        "ユーザーが停止しました。再試行する前にキューを確認してください。ジョブはまだ印刷中の可能性があります。キュー内のジョブはキャンセルされていません。",
        "Vom Benutzer angehalten. Prüfen Sie die Warteschlange vor einem neuen Versuch; der Auftrag wird eventuell noch gedruckt. Es wurde kein Auftrag abgebrochen.",
    ]),
    ("engine.printerBlocked", [
        "The printer reports a jam or error; check the printer and the queue before continuing.",
        "La impresora informa un bloqueo o error; revise la impresora y la cola antes de continuar.",
        "L’imprimante signale un blocage ou une erreur ; vérifiez l’imprimante et la file avant de continuer.",
        "プリンターが紙詰まりまたはエラーを報告しています。続行する前にプリンターとキューを確認してください。",
        "Der Drucker meldet einen Stau oder Fehler; prüfen Sie Drucker und Warteschlange, bevor Sie fortfahren.",
    ]),
    ("engine.interrupted", [
        "Interrupted while printing; operator review required.",
        "Interrumpido durante la impresión; requiere revisión del operador.",
        "Interrompu pendant l’impression ; vérification par l’opérateur requise.",
        "印刷中に中断されました。オペレーターによる確認が必要です。",
        "Während des Drucks unterbrochen; Prüfung durch den Bediener erforderlich.",
    ]),
    ("engine.snapshotMismatch", [
        "The local copy of this PDF has changed or is damaged (SHA-256 mismatch).",
        "La copia local de este PDF cambió o está dañada (SHA-256 no coincide).",
        "La copie locale de ce PDF a changé ou est endommagée (SHA-256 différent).",
        "この PDF のローカルコピーが変更されたか破損しています (SHA-256 不一致)。",
        "Die lokale Kopie dieses PDFs wurde geändert oder ist beschädigt (SHA-256 stimmt nicht überein).",
    ]),
    ("engine.deleteWhilePrinting", ["Cannot remove while printing is in progress.", "No se puede eliminar mientras hay una impresión en curso.", "Impossible de retirer pendant une impression.", "印刷中は削除できません。", "Während eines Druckvorgangs kann nichts entfernt werden."]),
    ("engine.resolveBeforeDelete", [
        "Resolve the PDFs with an uncertain result before removing them.",
        "Debe resolver los PDF con resultado incierto antes de eliminarlos.",
        "Résolvez les PDF au résultat incertain avant de les retirer.",
        "結果が不確かな PDF は、削除する前に解決してください。",
        "Beheben Sie PDFs mit unsicherem Ergebnis, bevor Sie sie entfernen.",
    ]),
    ("engine.notInQueue", ["The selected PDF is no longer in the queue.", "El PDF seleccionado no existe en la cola.", "Le PDF sélectionné n’est plus dans la file.", "選択した PDF はキューにありません。", "Das ausgewählte PDF ist nicht mehr in der Warteschlange."]),
    ("engine.physicalIncomplete", [
        "Physical printing incomplete. Check the sheets and resolve before continuing.",
        "Impresión física incompleta. Revise las hojas y resuelva antes de continuar.",
        "Impression papier incomplète. Vérifiez les feuilles et résolvez avant de continuer.",
        "実際の印刷が不完全です。用紙を確認し、解決してから続行してください。",
        "Physischer Druck unvollständig. Prüfen Sie die Blätter und beheben Sie das Problem, bevor Sie fortfahren.",
    ]),
    ("engine.onlyPendingMove", ["Only pending documents can be moved.", "Solo se pueden mover documentos pendientes.", "Seuls les documents en attente peuvent être déplacés.", "移動できるのは未処理のドキュメントだけです。", "Nur ausstehende Dokumente können verschoben werden."]),
    ("engine.editWhilePrinting", ["The queue cannot be edited while a document is printing.", "No se puede editar la cola mientras se imprime un documento.", "La file ne peut pas être modifiée pendant l’impression d’un document.", "ドキュメントの印刷中はキューを編集できません。", "Die Warteschlange kann während des Drucks nicht bearbeitet werden."]),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<usize> {
        (0..10).filter(|n| s.contains(&format!("{{{n}}}"))).collect()
    }

    #[test]
    fn table_is_complete_and_consistent() {
        let mut seen = std::collections::HashSet::new();
        for (key, texts) in STRINGS {
            assert!(seen.insert(*key), "duplicate key {key}");
            let want = placeholders(texts[0]);
            for (i, t) in texts.iter().enumerate() {
                assert!(!t.trim().is_empty(), "{key}: empty text for {}", LANGUAGES[i].0);
                assert_eq!(placeholders(t), want, "{key}: placeholders differ in {}", LANGUAGES[i].0);
            }
        }
    }

    /// Every key used in the sources exists in the table.
    #[test]
    fn every_used_key_exists() {
        let sources = [include_str!("app.rs"), include_str!("main.rs"), include_str!("i18n.rs")];
        let mut missing = Vec::new();
        for src in sources {
            for marker in ["t(\"", "tf!(\""] {
                let parts: Vec<&str> = src.split(marker).collect();
                for (before, part) in parts.iter().zip(parts.iter().skip(1)) {
                    // Only a call to `t`/`tf!`, not `insert("…")` or similar.
                    if before.chars().last().map_or(false, |c| c.is_alphanumeric() || c == '_') && marker == "t(\"" {
                        continue;
                    }
                    let key = part.split('"').next().unwrap();
                    if !key.is_empty() && !STRINGS.iter().any(|(k, _)| *k == key) {
                        missing.push(key.to_string());
                    }
                }
            }
        }
        assert!(missing.is_empty(), "missing keys: {missing:?}");
    }

    #[test]
    fn engine_messages_are_translated() {
        set(0);
        assert_eq!(engine("Enviando #3: a.pdf"), "Sending #3: a.pdf");
        assert!(engine("Trabajo 12: error de impresión (estado 2). Revise…").starts_with("Job 12: printing error (status 2)"));
        assert!(engine("Trabajo 5: tiempo de espera agotado. Revise").starts_with("Job 5: timed out"));
        assert!(engine("Trabajo 9: fallo al enviar el PDF: StartPage: x. Revise la cola").contains("StartPage: x."));
        assert_eq!(engine("algo desconocido"), "algo desconocido");
        set(1);
        assert_eq!(engine("Enviando #3: a.pdf"), "Enviando #3: a.pdf");
        set(0);
    }

    #[test]
    fn all_is_accepted_in_any_language() {
        for w in ["Todos", "all", "ALLE", "Tous", "すべて"] {
            assert!(is_all(w));
        }
        assert!(!is_all("20"));
    }
}
