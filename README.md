# PDF-batch

**English** · [Español](README.es.md)

**Batch PDF printing for Windows 10/11 x64.** Created by [siriusgoldsmith](https://github.com/siriusgoldsmith). Formerly *PDF Lote*.

An ordered queue, local copies of the documents and a persistent log, so you can continue after an interruption without blindly reprinting. A single `pdf-batch.exe`: nothing to install, no Adobe Reader and no Internet needed on the printing PC.

Interface in **English, Spanish, French, Japanese and German**. It starts in the Windows display language; change it with the **Language** selector (top right).

## Download

[**Downloads and releases**](https://github.com/siriusgoldsmith/PDF-batch/releases)

The published executables **are not signed with a trusted certificate**. Windows may show "Unknown publisher". Do not disable your antivirus or change security policies to run them. An application to **SignPath Foundation** for code signing is being prepared; it does not mean approval, nor that the current files are signed.

1. Close PDF Lote v1.x if it is open, then open `pdf-batch.exe`.
2. Add PDFs, check the order and select a printer.
3. Choose 20, 50, 100, All, or type a positive whole number.
4. Start the batch. The counter tracks documents finished according to Windows; it does not certify physical output.

Progress is automatic: you don't have to confirm each PDF. After an interruption, a preview of the last recorded page opens so you can choose where to continue. Remove PDF and Clear batch folder never delete the original files.

See [README.txt](README.txt) for how it works, its limits and recovery.

## What's new in v2.0

- Rewritten in Rust: a single ~16 MB executable that starts instantly.
- **PDFium** engine embedded in the executable; extracted to `%LOCALAPPDATA%\PDF-batch\runtime\` and its SHA-256 is verified before every load.
- **Fixed:** "Clear batch folder" froze the window. All long operations now run in the background.
- Automatic migration from `%LOCALAPPDATA%\PDFLote` (v1.x) to `%LOCALAPPDATA%\PDF-batch`, with queue, copies and history.

## Data and limits

- Log and copies: `%LOCALAPPDATA%\PDF-batch`. Do not delete that folder when updating.
- No telemetry and no PDFs sent to servers. The printer chosen by the operator is used, including a network printer if configured.
- Exported reports may contain sensitive paths and metadata.
- Pages are rasterized at 150 dpi and fitted to the printable area; 100 % scale is not guaranteed.
- Windows cannot generically certify which sheet was physically printed last after an interruption.
- Test first with a few documents and your real printer.

[Privacy policy](PRIVACY.md) · [Code signing policy](SIGNING.md) · [Signing application](docs/SIGNPATH_APPLICATION.md)

## Building from source

Windows x64 with [Rust](https://rustup.rs) (toolchain `stable-x86_64-pc-windows-msvc`) and Visual Studio Build Tools with "Desktop development with C++".

1. Download `pdfium-win-x64.tgz` from [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases) and extract it to `vendor/pdfium/` (you should get `vendor/pdfium/bin/pdfium.dll`). Other path: `PDFIUM_DLL` environment variable.
2. In PowerShell (not Git Bash, whose `link.exe` shadows MSVC's):

```powershell
cargo test --workspace
cargo build --release -p pdf-batch-app
```

The executable is `target/release/pdf-batch.exe`, with PDFium and the C runtime included. Building it **does not sign it**.

| Crate | Contents |
|---|---|
| `crates/pdf-batch-core` | Durable, audited SQLite queue (compatible with the v1.x database) |
| `crates/pdf-batch-print` | PDFium rendering, GDI printing, spooler wait and batch worker |
| `crates/pdf-batch-app` | Desktop interface (egui), translations, single instance and data migration |

`cargo run -p pdf-batch-core --example seed -- <folder>` creates a demo queue to review the interface without printing.

### Translations

All interface texts are in [`crates/pdf-batch-app/src/i18n.rs`](crates/pdf-batch-app/src/i18n.rs): one key per text and one column per language (en, es, fr, ja, de). Tests check that every key used exists and that every language has the same placeholders. Corrections from native speakers are welcome.

## Authorship and license

Author and maintainer: **siriusgoldsmith**. GNU Affero General Public License v3 or later; text in [COPYING](COPYING). Dependencies and notices in [THIRD_PARTY.txt](THIRD_PARTY.txt) and `licenses/`. No warranty.
