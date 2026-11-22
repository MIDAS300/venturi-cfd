# Venturi CFD v0.2 — Carmelo Version (Updated Source Code + MCP Server + OpenFOAM Model)

**Venturi** is a native **Rust + Tauri v2** desktop CFD Engineering Studio & **MCP (Model Context Protocol)** Server compatible with **OpenFOAM®** (Foundation v14 `.org` and OpenCFD v2606 `.com`), built on a **100% commercial-friendly MIT / Apache-2.0 stack** (zero GPL linking).

---

## Novità nella Carmelo Version (`Venturi_CFD_v0.2_SourceCode.zip`)

1. **Modalità Guidata passo-passo (Aiuti ON / OFF)** — parallela alla modalità standard, attivabile e disattivabile in qualsiasi momento dal pulsante `Aiuti: ON / OFF` nella barra superiore. Guida l'utente lungo i 6 step logici della simulazione CFD: *Geometria → Fisica → Mesh → Condizioni al Contorno → Numerica → Audit & Lancio*.
2. **Granularità chiara per Mesh di Volume e Superficie** — controlli separati e immediati per la mesh di volume (`blockMesh` con preset e dimensione target in `mm`), la mesh di superficie (`snappyHexMesh` con preset e dimensione reale in `mm` per livello min/max) e le zone di infittimento volumetrico 3D (livello modificabile inline con dimensione in `mm`).
3. **Particellari del Flusso di Velocità ON / OFF** — toggle esplicito `Particellari ON / OFF` nella barra del viewport 3D, con tracciamento allineato al vettore velocità di ingresso (`velocityInlet`).
4. **Scaling CAD (STL) chiaro e immediato** — rilevamento automatico di scale anomale (es. STL esportati in `mm`), conversione unità 1-clic (`mm → m`, `cm → m`, `in → m`, `ft → m`, `1:1`), impostazione diretta della dimensione reale desiderata in metri (`X/Y/Z`), adattamento automatico di dominio e camera, ed export del file STL già scalato in `constant/triSurface/`.

> **Nota sui binari Windows (`.exe`)**: il codice sorgente aggiornato è contenuto in **`Venturi_CFD_v0.2_SourceCode.zip`**. Per rigenerare gli eseguibili Windows aggiornati (`Venturi CFD_0.2.0_x64-setup.exe` e portable), compila da sorgente con `npm install && npm run tauri build` oppure tramite il workflow GitHub Actions incluso (`.github/workflows/build-windows.yml`).

---

## Contents of this Drive Folder

1. **`Venturi_CFD_v0.2_SourceCode.zip`** *(Aggiornato — Carmelo Version)*:
   Complete Rust + TypeScript/React/Three.js source repository with Guided Mode, Volume/Surface Mesh Granularity, Velocity Flow Particles toggle, and CAD STL Scaling (includes `.github/workflows/build-windows.yml` for automated Windows CI builds).
2. **`venturi_gemini_mcp_extended_workflow.mp4`** (`1480×920` H.264, `37.1s`):
   Extended end-to-end screen recording of a real autonomous workflow piloted by **Gemini via `venturi-mcp`**.
3. **Windows x64 Executables (v0.2 baseline):**
   - `Venturi CFD_0.2.0_x64-setup.exe` — Windows 10/11 64-bit NSIS Installer (uses built-in Edge WebView2)
   - `venturi-mcp.exe` — Standalone Windows 64-bit MCP JSON-RPC 2.0 Server (`stdio`, 16 typed CFD tools)
   - `Venturi_CFD_v0.2_Windows_x64_Portable.zip` — Portable Windows 64-bit package
4. **`Venturi_Final_Model_Ahmed13deg_OpenFOAM.zip`** + **`ahmed_diffuser_13deg_optimized.stl`**:
   Complete exported OpenFOAM case (`0/*`, `constant/physicalProperties`, `constant/momentumTransport`, `constant/triSurface/ahmed_diffuser.stl`, `system/controlDict`, `system/snappyHexMeshDict`, `Allrun`, `Allclean`).
5. **`gemini_mcp_session_history.json`**:
   Complete recorded 5-iteration MCP session snapshot & 18-call tool audit trail.
6. **`venturi_mcp_windows_config.json`**:
   Ready-to-paste MCP configuration for Gemini / Antigravity / Claude Desktop / Cursor on Windows.

---

*Venturi CFD is an independent open-architecture engineering tool compatible with OpenFOAM®. OPENFOAM® is a registered trademark of OpenCFD Limited (Keysight Technologies).*
