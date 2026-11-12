//! Locate the PDFium DLL to embed: `PDFIUM_DLL` env var, else `vendor/pdfium/bin/pdfium.dll`.
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=PDFIUM_DLL");
    let path = std::env::var_os("PDFIUM_DLL").map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../vendor/pdfium/bin/pdfium.dll")
    });
    let path = path.canonicalize().unwrap_or_else(|_| {
        panic!(
            "No se encontró pdfium.dll en {}. Descárgala de bblanchon/pdfium-binaries (pdfium-win-x64.tgz) \
             a vendor/pdfium/ o define PDFIUM_DLL.",
            path.display()
        )
    });
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:rustc-env=PDFIUM_EMBED={}", path.display());

    // Executable icon and version info shown by Windows Explorer.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/pdflote.ico")
            .set("ProductName", "PDF-batch")
            .set("FileDescription", "PDF-batch: impresión de PDFs por lotes")
            .set("LegalCopyright", "siriusgoldsmith · AGPL-3.0-or-later");
        res.compile().expect("no se pudo compilar el recurso de Windows");
    }
}

