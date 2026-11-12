PDF-batch v2.1.0 — Windows 10/11 64-bit
=======================================
Created by siriusgoldsmith — https://github.com/siriusgoldsmith/PDF-batch
Formerly PDF Lote (PDFLote). Spanish version: LEEME.txt

FEATURES
- Written in Rust. A single file, pdf-batch.exe (about 16 MB), with no extra
  folders or DLLs next to it. Starts instantly.
- PDF engine: PDFium (the one in Chrome), embedded in the executable. The first
  time, it is extracted to %LOCALAPPDATA%\PDF-batch\runtime\ and its SHA-256 is
  verified every time before use.
- Clearing, removing, adding and printing run in the background and the
  window stays responsive, even with large batches or disconnected network/USB
  drives.
- Automatic migration: on first start, the %LOCALAPPDATA%\PDFLote folder is
  moved to %LOCALAPPDATA%\PDF-batch with its queue, copies and history. Close
  PDF Lote v1.x first; if it is open, you will be told.
- Translated into English, Spanish, French, Japanese and German. The app starts in
  the Windows display language; change it with the Language selector (top
  right). The choice is saved in %LOCALAPPDATA%\PDF-batch\settings.json.
- Validated with a real printer. Still, test first with 2 or 3 PDFs on your PC.

USE
1. Close the previous version and open pdf-batch.exe. Nothing to install, no
   Adobe Reader and no Internet needed to print. Do not delete
   %LOCALAPPDATA%\PDF-batch (or PDFLote if not migrated yet): your batch is kept
   there.
2. Add PDFs. They are appended in the visible order. Add folder sorts by name,
   without subfolders. Windows may return a multiple selection sorted by name
   rather than by click order. Check the order; use Move up/Move down on
   pending documents, or add them one by one if you need another order.
   Ctrl+click selects several documents.
3. Select the printer. Check paper, tray, colour and duplex in Windows settings
   BEFORE starting. Those settings apply to every PDF in the batch.
4. Select or type how many PDFs to send per batch. The unit is DOCUMENTS, not
   pages. All processes the available pending documents until a blocker.
5. Click Start / continue and accept the confirmation with the actual number.
   When the batch ends, it stops. Start / continue processes the next batch
   without repeating finished documents. There is no confirmation per PDF.
6. Pause after document waits for the current PDF to finish; it does not cut
   it in half. The Pause requested message explains the wait. It does not force
   cancellation of Windows jobs. Wait for the operation to end before closing.

COUNTER AND STATES
- The counter and the bar show PDFs finished according to Windows in the
  current batch. They reset on each start; the summary keeps overall progress.
- Pending: not started. In progress: active attempt. Finished: the process
  ended, or the operator resolved an interruption by continuing with the next.
- Windows result keeps the automatic recorded result separately. Finished
  according to Windows does NOT certify physical sheets: a job removed from the
  queue can also disappear without an observable error.
- Review interruption: there is a doubtful result; it blocks progress until
  resolved.
- Skipped: it was decided not to print the document. Not counted as finished.
- You do not have to watch or confirm each document for the batch to advance.

INTERRUPTIONS AND RECOVERY
1. After an error, or when opening after an unexpected shutdown, a popup
   appears. You can also select the document and click Resolve interruption.
2. The view shows the last page recorded as sent to the driver. If the
   document has no progress, it shows the previous eligible reference. With no
   prior record, it warns without inventing a page. The preview is generated
   locally and is a reference, not a certification of printed paper.
3. Only in this case, compare with the last sheet and check the Windows queue.
   For duplex, check both sides. A job may continue after closing the app: wait
   or cancel it manually before retrying.
4. Choose: resume this PDF from page 1, from the missing page, continue with
   the next because this one printed completely, or skip it without confirming
   paper. The decision requires confirmation to avoid blind repeats.
5. Then click Start / continue. Closing the popup does not change the state or
   print.

REMOVE PDF / CLEAR BATCH FOLDER
- Remove PDF removes the selected documents and their local copies from the
  batch.
- Clear batch folder empties the list and deletes its identified copies.
- Both actions ask for confirmation; they NEVER delete the source originals.
  The audit history is kept, not a recoverable copy of the PDFs.
- You cannot delete during an operation or remove unresolved interruptions.
- Locked files or files without verifiable origin may be kept for safety. Do
  not manually delete log files while the app is in use.
- To resume later a batch you are about to clear, back up its whole folder
  first.

LIMITATIONS AND FIRST TEST
- First, test 2 or 3 PDFs with your printer.
- No guarantee against jams, out-of-paper, driver failures or the physical
  order between trays or other people's jobs. Use one printer and avoid other
  jobs while the batch prints.
- Rasterized at 150 dpi, fitted to the printable area keeping proportions. It
  is not vector printing and does not guarantee 100% scale; do not use it for
  scale drawings.
- No support for passwords or per-PDF settings other than paper/tray.
- A damaged PDF may fail to render; the point is kept for review.

DATA AND PRIVACY
The originals do not change. Copies, history and log are kept in:
%LOCALAPPDATA%\PDF-batch\
You need space for a copy of all the PDFs. It may contain sensitive data;
protect your account and do not share that folder or reports with personal
paths. No PDFs are sent to servers and there is no telemetry. The JSON report
contains states, paths and historical metadata; it does not include the PDFs.
The signature link opens GitHub only if you click it. One instance per data
folder, even across Windows sessions. Do not delete queue.lock to open another
instance.

AUTHOR AND LICENSES
Visual signature: siriusgoldsmith. It is not a Windows digital certificate.
SmartScreen may show unknown publisher. Do not disable your antivirus.
Source code: https://github.com/siriusgoldsmith/PDF-batch. PDF engine: PDFium
(BSD/Apache), included in the executable. Distributed under GNU AGPL v3 or
later. See COPYING, THIRD_PARTY.txt and licenses/ for licenses and dependency
notices.
