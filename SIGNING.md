# Code signing policy

## Estado actual: preparación de solicitud, sin aprobación

PDFLote es mantenido por [siriusgoldsmith](https://github.com/siriusgoldsmith). Se prepara una solicitud al programa gratuito de SignPath Foundation para futuras versiones. **Todavía no existe un certificado de SignPath asignado al proyecto ni una versión firmada mediante ese programa.** No se debe presentar v1.4 como firmada ni anunciar una v1.5 firmada antes de verificarla.

Si SignPath aprueba el proyecto, el editor del certificado será **SignPath Foundation**, no siriusgoldsmith. La autoría del proyecto seguirá siendo siriusgoldsmith. La atribución que se incorporará una vez aprobado el servicio es: “Free code signing provided by SignPath.io, certificate by SignPath Foundation”. Esta frase describe la atribución futura, no un patrocinio ya concedido.

## Roles y condiciones para activar la firma

- Autor, mantenedor y revisor de contribuciones: [siriusgoldsmith](https://github.com/siriusgoldsmith).
- Aprobador de publicaciones y solicitudes de firma: siriusgoldsmith, propietario del repositorio.
- Los cambios de terceros deben revisarse antes de integrarse. No se afirma una revisión independiente cuando una sola persona ocupa varios roles.
- Antes de activar el servicio se debe confirmar MFA en GitHub y SignPath para todas las personas con acceso a estos roles. No se afirma que esa comprobación ya esté hecha.
- Las compilaciones que se soliciten firmar deben provenir de este repositorio y una ejecución verificable de CI, no de un EXE arbitrario subido desde un equipo local.
- Cada solicitud de firma requiere aprobación manual del mantenedor. Las claves deben permanecer en la infraestructura gestionada del proveedor, fuera del repositorio y de los ZIP.
- La integración debe seguir las restricciones de procedencia y metadatos aprobadas por SignPath. No se firmarán bibliotecas ajenas de forma independiente como si fueran originales del proyecto.

## Publicación y verificación

La versión sin firma v1.4 se publica con su SHA-256 y fuentes correspondientes. Para una futura versión firmada se comprobarán la firma Authenticode, la cadena de confianza, el editor y el sello de tiempo en Windows. Sólo después se calculará el hash y se empaquetará el artefacto final firmado. Una firma válida no garantiza la desaparición inmediata de todas las advertencias de SmartScreen.

El proyecto no intenta eludir SmartScreen, instalar certificados raíz en los equipos de usuarios ni presentar certificados autofirmados como confianza pública.

## Privacidad

[Política de privacidad](PRIVACY.md). PDFLote no transfiere información a otros sistemas salvo petición del operador, por ejemplo imprimir en una impresora de red elegida, exportar un informe o abrir el enlace a GitHub.

Condiciones del programa: https://signpath.org/terms
