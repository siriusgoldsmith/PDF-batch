# PDF-batch

**Impresión de PDFs por lotes para Windows 10/11 x64.** Creado por [siriusgoldsmith](https://github.com/siriusgoldsmith). Antes llamado *PDF Lote*.

Una cola ordenada, copias locales de los documentos y un registro persistente para continuar tras interrupciones sin reimprimir a ciegas. Un solo `pdf-batch.exe`: no requiere instalar nada, ni Adobe Reader ni Internet en el equipo que imprime.

> **v2.0 está en fase alfa.** Es una reescritura completa en Rust. La impresión con impresoras reales todavía no se ha validado: prueba primero con 2 o 3 PDFs y conserva la v1.4 a mano.

## Descargar

[**Descargas y versiones**](https://github.com/siriusgoldsmith/PDF-batch/releases)

Los ejecutables publicados **no tienen firma digital de confianza**. Windows puede mostrar «Editor desconocido». No desactives el antivirus ni cambies las políticas de seguridad para usarlos. Se está preparando la solicitud de firma a **SignPath Foundation**; la solicitud no implica aprobación ni que los archivos actuales estén firmados.

1. Cierra PDF Lote v1.x si lo tienes abierto y abre `pdf-batch.exe`.
2. Añade PDFs, revisa el orden y selecciona impresora.
3. Elige 20, 50, 100, Todos o escribe una cantidad entera positiva.
4. Inicia el grupo. El contador registra documentos finalizados según Windows, no certifica salida física.

El avance normal es automático: no se exige confirmar cada PDF. Ante una interrupción se abre una vista previa de la última página registrada para elegir desde dónde continuar. Eliminar PDF y Vaciar carpeta del lote conservan siempre los archivos originales.

Consulta [LEEME.txt](LEEME.txt) para funcionamiento, límites y recuperación.

## Novedades de v2.0

- Reescrito en Rust: un solo ejecutable de unos 15 MB que arranca al instante.
- Motor PDF **PDFium** incrustado en el ejecutable; se extrae a `%LOCALAPPDATA%\PDF-batch\runtime\` y se verifica su SHA-256 antes de cada carga.
- **Corregido:** «Vaciar carpeta del lote» dejaba la ventana congelada. Todas las operaciones largas trabajan ahora en segundo plano.
- Migración automática de `%LOCALAPPDATA%\PDFLote` (v1.x) a `%LOCALAPPDATA%\PDF-batch`, con cola, copias e historial.

## Datos y límites

- Registro y copias: `%LOCALAPPDATA%\PDF-batch`. No borres esa carpeta al actualizar.
- Sin telemetría ni envío de PDFs a servidores. Se utiliza la impresora elegida por el operador, incluida una impresora de red si así se configura.
- Los informes exportados pueden contener rutas y metadatos sensibles.
- Las páginas se rasterizan a 150 ppp y se ajustan al área imprimible; no se garantiza escala 100 %.
- Windows no puede certificar genéricamente cuál fue la última hoja física tras un corte.
- Prueba primero con pocos documentos y tu impresora real. Las pruebas automáticas y las simulaciones de transporte no sustituyen la validación de hardware.

[Política de privacidad](PRIVACY.md) · [Code signing policy](SIGNING.md) · [Preparación de la solicitud](docs/SIGNPATH_APPLICATION.md)

## Construir desde las fuentes

Windows x64 con [Rust](https://rustup.rs) (toolchain `stable-x86_64-pc-windows-msvc`) y Visual Studio Build Tools con «Desarrollo para el escritorio con C++».

1. Descarga `pdfium-win-x64.tgz` de [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries/releases) y extráelo en `vendor/pdfium/` (debe quedar `vendor/pdfium/bin/pdfium.dll`). Otra ruta: variable `PDFIUM_DLL`.
2. En PowerShell (no en Git Bash, cuyo `link.exe` tapa al de MSVC):

```powershell
cargo test --workspace
cargo build --release -p pdf-batch-app
```

El ejecutable queda en `target/release/pdf-batch.exe`, con PDFium y el runtime de C incluidos. Construirlo **no lo firma**.

| Crate | Contenido |
|---|---|
| `crates/pdf-batch-core` | Cola SQLite duradera y auditada (compatible con la base de datos de v1.x) |
| `crates/pdf-batch-print` | Renderizado PDFium, impresión GDI, espera del spooler y worker del lote |
| `crates/pdf-batch-app` | Interfaz de escritorio (egui), instancia única y migración de datos |

`cargo run -p pdf-batch-core --example seed -- <carpeta>` crea una cola de demostración para revisar la interfaz sin imprimir.

## Autoría y licencia

Autor y mantenedor: **siriusgoldsmith**. GNU Affero General Public License v3 o posterior; texto en [COPYING](COPYING). Dependencias y avisos en [THIRD_PARTY.txt](THIRD_PARTY.txt) y `licenses/`. Sin garantía.
