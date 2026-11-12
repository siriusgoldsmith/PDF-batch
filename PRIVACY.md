# Política de privacidad

PDF-batch procesa los documentos en el equipo del operador. No incluye telemetría, cuentas, publicidad ni una API remota propia para recibir documentos. No envía los PDFs a servidores del autor ni a servicios de inteligencia artificial.

## Datos locales

La cola SQLite, el historial y las copias internas de los PDFs se almacenan en `%LOCALAPPDATA%\PDF-batch` (en v1.x, `%LOCALAPPDATA%\PDFLote`; v2.0 la migra al abrir). Allí también se extrae y verifica el motor PDFium incluido en el ejecutable. Pueden contener información confidencial. Es responsabilidad del operador proteger su cuenta de Windows, sus permisos y sus respaldos. Los documentos originales no se modifican ni eliminan al vaciar el lote.

El informe JSON se genera sólo si el operador lo solicita. Incluye estados, rutas y metadatos históricos, no el contenido de los PDFs. El operador decide dónde guardarlo y si lo comparte. El registro de auditoría puede conservar rutas y nombres después de retirar un documento de la lista.

## Interacciones solicitadas por el operador

- La impresión se realiza mediante Windows y el controlador seleccionado. Si el operador elige una impresora o servidor de impresión de red, los datos necesarios se transfieren a ese destino.
- Al hacer clic en la firma de autor se abre GitHub en el navegador. La navegación queda sujeta a las políticas de GitHub y del navegador.
- Las descargas del repositorio y futuras firmas de código usan servicios externos de distribución y compilación. Esto no concede a esos servicios acceso a los documentos que se imprimen en el equipo del usuario.

## Diagnósticos

Las pruebas y el modo de diagnóstico crean PDFs temporales propios. No deben ejecutarse ni compartirse sobre carpetas con documentación sensible sin revisar el informe generado. Las pruebas automatizadas de transporte usan simulaciones y no prueban hardware físico.

## Contacto

Para informar problemas del programa utiliza los issues de https://github.com/siriusgoldsmith/PDF-batch sin adjuntar PDFs, bases de datos, credenciales ni rutas personales. Si necesitas compartir un ejemplo, utiliza un documento sintético sin datos reales.
