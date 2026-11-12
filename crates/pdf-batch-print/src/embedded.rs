//! Single-file distribution: `pdfium.dll` travels inside the executable and is
//! written once to `<data_dir>\runtime\pdfium-<sha12>.dll`. Its SHA-256 is checked
//! against the embedded bytes every time before it is loaded, so a replaced or
//! corrupted file on disk is rewritten, never loaded.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

pub(crate) struct Embedded {
    pub bytes: &'static [u8],
    pub dir: PathBuf,
}

pub(crate) static EMBEDDED: OnceLock<Embedded> = OnceLock::new();

/// Register the DLL bytes compiled into the executable and where to extract them.
/// Call before the first [`crate::pdfium`] use.
pub fn set_embedded_pdfium(bytes: &'static [u8], runtime_dir: impl Into<PathBuf>) {
    let _ = EMBEDDED.set(Embedded { bytes, dir: runtime_dir.into() });
}

fn hex(d: impl AsRef<[u8]>) -> String {
    d.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn file_sha(path: &Path) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(hex(h.finalize()))
}

/// Ensure a verified copy exists and return its path.
pub fn extract(bytes: &[u8], dir: &Path) -> Result<PathBuf, String> {
    if bytes.is_empty() {
        return Err("la DLL incrustada está vacía".into());
    }
    let want = hex(Sha256::digest(bytes));
    let path = dir.join(format!("pdfium-{}.dll", &want[..12]));
    if file_sha(&path).as_deref() == Some(want.as_str()) {
        return Ok(path);
    }
    fs::create_dir_all(dir).map_err(|e| format!("No se pudo crear {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("pdfium-{}.{}.tmp", &want[..12], std::process::id()));
    let write = || -> std::io::Result<()> {
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()
    };
    write().map_err(|e| format!("No se pudo extraer PDFium: {e}"))?;
    let _ = fs::remove_file(&path); // stale or tampered copy
    if let Err(e) = fs::rename(&tmp, &path) {
        let _ = fs::remove_file(&tmp);
        // Another instance may have won the race with an identical file.
        if file_sha(&path).as_deref() != Some(want.as_str()) {
            return Err(format!("No se pudo extraer PDFium: {e}"));
        }
    }
    if file_sha(&path).as_deref() != Some(want.as_str()) {
        return Err("La copia extraída de PDFium no coincide con la incrustada.".into());
    }
    Ok(path)
}
