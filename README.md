# PDFLote

**Impresión de PDFs por lotes para Windows 10/11 x64.** Creado por [siriusgoldsmith](https://github.com/siriusgoldsmith).

Una cola ordenada, copias locales de los documentos y un registro persistente para continuar tras interrupciones sin reimprimir a ciegas. No requiere Adobe Reader, Python ni Internet en el equipo que imprime.

## Descargar

[**Descargas y versiones**](https://github.com/siriusgoldsmith/PDFLote/releases)

La versión publicada **v1.4 no tiene firma digital de confianza**. Windows puede mostrar «Editor desconocido». No desactives el antivirus ni cambies las políticas de seguridad para instalarla. La firma para una futura v1.5 se está preparando para solicitar a **SignPath Foundation**; la solicitud no implica aprobación ni que los archivos actuales estén firmados.

1. Extrae el ZIP en una carpeta y abre `PDFLote.exe`.
2. Añade PDFs, revisa el orden y selecciona impresora.
3. Elige 20, 50, 100, Todos o escribe una cantidad entera positiva.
4. Inicia el grupo. El contador registra documentos finalizados según Windows, no certifica salida física.

El avance normal es automático: no se exige confirmar cada PDF. Ante una interrupción se abre una vista previa de la última página registrada para elegir desde dónde continuar. Eliminar PDF y Vaciar carpeta del lote conservan siempre los archivos originales.

Consulta [LEEME.txt](LEEME.txt) para funcionamiento, límites y recuperación.

## Datos y límites

- Registro y copias: `%LOCALAPPDATA%\PDFLote`. No borres esa carpeta al actualizar.
- Sin telemetría ni envío de PDFs a servidores. Se utiliza la impresora elegida por el operador, incluida una impresora de red si así se configura.
- Los informes exportados pueden contener rutas y metadatos sensibles.
- Las páginas se rasterizan a 150 ppp y se ajustan al área imprimible; no se garantiza escala 100 %.
- Windows no puede certificar genéricamente cuál fue la última hoja física tras un corte.
- Prueba primero con pocos documentos y tu impresora real. Las pruebas de interfaz y las simulaciones de transporte no sustituyen la validación de hardware.

[Política de privacidad](PRIVACY.md) · [Code signing policy](SIGNING.md) · [Preparación de la solicitud](docs/SIGNPATH_APPLICATION.md)

## Construir desde las fuentes

Windows x64 con Python 3.11 x64:

```bat
rem Antes: prepara runtime/ como se explica debajo.
py -3.11 -m venv .venv
call .venv\Scripts\activate.bat
python -m pip install -r requirements.txt
python -m unittest discover -v
python -m PyInstaller --noconfirm --clean PDFLote.spec
```

El ejecutable queda en `dist/PDFLote.exe`. Construirlo **no lo firma**. Las dependencias están fijadas en `requirements.txt`.

### Runtime de Microsoft

No se publican DLLs de Microsoft por separado en el repositorio. El spec necesita una carpeta `runtime/` preparada desde una instalación autorizada del runtime oficial x64 de Visual C++ en Windows, no desde Wine. [Descarga oficial de Microsoft](https://aka.ms/vs/17/release/vc_redist.x64.exe). Respeta sus condiciones de uso y distribución; se conserva el aviso original en `licenses/`.

Con el runtime ya instalado, abre PowerShell x64 en la carpeta del proyecto:

```powershell
New-Item -ItemType Directory -Force runtime | Out-Null
Copy-Item "$env:WINDIR\System32\msvcp140*.dll" runtime\
Copy-Item "$env:WINDIR\System32\vcruntime140*.dll" runtime\
```

Las DLLs adicionales de otras familias del runtime sólo son necesarias si las requiere el binario construido; no deben sustituirse por archivos descargados de sitios de terceros. La futura integración de CI debe comprobar las dependencias reales del EXE y ejercitarlo antes de solicitar la firma. La compilación verificable exige poder vincular repositorio, cambios, ejecución de CI y artefacto; no se afirma reproducibilidad bit a bit.

El proyecto incluye pruebas de persistencia, orden, borrado seguro, recuperación, límites de lote y GUI. Los diagnósticos trabajan con PDFs temporales y transporte simulado; no deben confundirse con una prueba de la impresora física.

## Autoría y licencia

Autor y mantenedor: **siriusgoldsmith**. GNU Affero General Public License v3 o posterior; texto en [COPYING](COPYING). Dependencias y avisos en [THIRD_PARTY.txt](THIRD_PARTY.txt) y `licenses/`. Sin garantía.
