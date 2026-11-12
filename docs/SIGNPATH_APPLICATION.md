# Solicitud de SignPath: borrador público

**Estado: no enviada.** El repositorio y la descarga pública existen; faltan datos privados de contacto, confirmación de MFA y activar/verificar el build de CI. Esta página no constituye una solicitud aprobada ni una certificación.

- Project Name: PDF-batch
- Repository URL: https://github.com/siriusgoldsmith/PDF-batch
- Homepage URL: https://github.com/siriusgoldsmith/PDF-batch
- Download URL: https://github.com/siriusgoldsmith/PDF-batch/releases
- Privacy Policy URL: https://github.com/siriusgoldsmith/PDF-batch/blob/main/PRIVACY.md
- Wikipedia URL: no existe artículo conocido; dejar vacío.
- Tagline: Ordered PDF batch printing for Windows with persistent queues and interruption recovery.
- Description: PDF-batch (formerly PDFLote) is a local-first Windows desktop application for printing ordered groups of PDF documents. It keeps a persistent queue and local document copies, records progress automatically, and helps operators recover from interruptions using a preview of the last recorded page. It does not require per-document paper confirmation or upload documents to a remote service.
- Reputation: Newly published open-source project, maintained by siriusgoldsmith. There are no independently verified public adoption or reputation metrics yet. v1.x (Python) was exercised with a 93-test suite. v2.0 is a Rust rewrite with 22 automated tests (queue durability, safe cleanup, PDFium rendering, simulated print transport) built natively on Windows with MSVC. Physical printer validation remains the operator's responsibility. These tests demonstrate maintenance work, not established public reputation.
- Maintainer: independent developer / siriusgoldsmith; select the matching option actually offered by the form.
- Build System: currently Rust (cargo, MSVC toolchain) on the maintainer's Windows host. A GitHub Actions Windows build template is prepared but not active or verified. Do not select GitHub Actions as an active build system until it has run successfully.
- Discovery: research by the maintainer's AI assistant using official SignPath documentation; select the appropriate channel offered by the form. Do not claim a recommendation from a person or project that did not occur.

The required First Name, Last Name and Email must be provided privately by the owner. Do not invent identity data or put private contact details in this repository. Company Name may remain empty if inapplicable. The reputation paragraph is deliberately candid: SignPath may decline a new project that does not yet have sufficient verifiable reputation.

## CI activation

[Active workflow](../.github/workflows/windows-build.yml). It is enabled in GitHub Actions and its first run passed. Do not store GitHub tokens in the repository or ask the owner to paste them into an issue.

The workflow uses immutable commit pins for the official checkout and artifact-upload actions, read-only repository permissions, no signing credentials, and a single Windows job. It runs the Rust test suite, downloads PDFium from bblanchon/pdfium-binaries and records its hash, builds an unsigned EXE, and preserves the resulting artifact and evidence. Syntax checks do not establish a working CI run.

After approval of GitHub workflow access: move the candidate to `.github/workflows/windows-build.yml`, run it, fix any native Windows failures, and retain the successful run/commit/artifact links. Only then update the application with truthful CI provenance. SignPath integration and signed releases require separate project approval and configuration by SignPath; do not invent organization IDs, certificate IDs, signing policies or API keys.

## Form and approvals

Official application: https://signpath.org/apply

The owner must approve the program terms and processing of the supplied contact information. Marketing communications are optional and should not be selected by default. Do not report the application as submitted without a success response or confirmation. Do not report v1.5 as signed without an actual verified signed EXE.
