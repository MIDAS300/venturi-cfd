import React, { useState } from "react";
import {
  Compass,
  ChevronLeft,
  ChevronRight,
  CheckCircle2,
  AlertTriangle,
  Upload,
  Sparkles,
  Play,
  Download,
  X,
} from "lucide-react";
import {
  CfdCase,
  DomainFace,
  FlowRegime,
  SchemeAccuracy,
  TelemetrySnapshot,
  TurbulenceModel,
  ValidationReport,
  WorkflowTab,
} from "../types/cfd";
import { FLUID_LIBRARY } from "../data/presets";
import { computeYPlus } from "../api/bridge";

interface GuidedAssistantProps {
  cfdCase: CfdCase;
  activeTab: WorkflowTab;
  telemetry: TelemetrySnapshot | null;
  validation: ValidationReport | null;
  isRunning: boolean;
  onSelectTab: (tab: WorkflowTab) => void;
  onSelectPreset: (presetId: string) => void;
  onImportStl: () => void;
  onChangeCase: (updater: (prev: CfdCase) => CfdCase) => void;
  onHighlightFace: (face: DomainFace | null) => void;
  onExportCase: () => void;
  onStartSolver: () => void;
  onDisableGuided: () => void;
}

interface GuidedStepDef {
  id: WorkflowTab;
  shortLabel: string;
  title: string;
  question: string;
  tip: string;
}

/** Strict logical CFD simulation setup sequence */
export const GUIDED_STEPS: GuidedStepDef[] = [
  {
    id: "geometry",
    shortLabel: "1. Geometria",
    title: "Tipo di Simulazione & Geometria",
    question: "Che tipo di simulazione CFD vuoi impostare?",
    tip: "Scegli uno scenario di partenza oppure importa un file CAD (.stl). Se importi un CAD, controlla subito lo scaling in metri qui sotto.",
  },
  {
    id: "physics",
    shortLabel: "2. Fisica",
    title: "Fluido, Regime & Turbolenza",
    question: "Quale fluido e regime fisico governano il problema?",
    tip: "Impostare fluido e velocità prima della mesh permette a Venturi di calcolare automaticamente lo spessore dello strato limite (y+).",
  },
  {
    id: "mesh",
    shortLabel: "3. Mesh",
    title: "Granularità Mesh di Volume & Superficie",
    question: "Che livello di dettaglio vuoi per volume e superficie?",
    tip: "Usa un profilo rapido o regola la dimensione in mm di volume (blockMesh) e superficie (snappyHexMesh), poi sincronizza y+.",
  },
  {
    id: "boundaries",
    shortLabel: "4. Contorno",
    title: "Condizioni al Contorno (Boundary)",
    question: "Le superfici di ingresso, uscita e pareti sono coerenti?",
    tip: "Passa il mouse sui patch per evidenziarli in 3D e assicurati che la velocità di ingresso corrisponda a U∞.",
  },
  {
    id: "numerics",
    shortLabel: "5. Numerica",
    title: "Schemi Numerici & Solutore",
    question: "Preferisci stabilità di avvio o massima accuratezza?",
    tip: "Il 2° ordine bilanciato (linearUpwind) è lo standard per Cd/Cl; usa 1° ordine per test preliminari su mesh complesse.",
  },
  {
    id: "run",
    shortLabel: "6. Lancia",
    title: "Audit Pre-Volo & Esecuzione",
    question: "Tutto pronto per validare, esportare o lanciare il caso?",
    tip: "Verifica che l'Audit Pre-Volo abbia 0 errori, poi avvia il solutore o esporta il caso completo per OpenFOAM.",
  },
];

const SIM_TYPES: {
  id: string;
  presetId: string;
  label: string;
  desc: string;
}[] = [
  {
    id: "aero",
    presetId: "naca0012",
    label: "Aerodinamica Esterna",
    desc: "Profilo NACA 0012 · Portanza/Resistenza",
  },
  {
    id: "internal",
    presetId: "venturi_nozzle",
    label: "Flusso Interno / Ugello",
    desc: "Tubo di Venturi · Caduta di pressione",
  },
  {
    id: "auto",
    presetId: "ahmed_body",
    label: "Veicolo / Scia",
    desc: "Ahmed Body 25° · Scia e drag",
  },
  {
    id: "wake",
    presetId: "cylinder_vortex",
    label: "Vortex Shedding",
    desc: "Cilindro transitorio · Scia instazionaria",
  },
];

export const GuidedAssistant: React.FC<GuidedAssistantProps> = ({
  cfdCase,
  activeTab,
  telemetry,
  validation,
  isRunning,
  onSelectTab,
  onSelectPreset,
  onImportStl,
  onChangeCase,
  onHighlightFace,
  onExportCase,
  onStartSolver,
  onDisableGuided,
}) => {
  const [yPlusNote, setYPlusNote] = useState<string | null>(null);
  const stepIdx = GUIDED_STEPS.findIndex((s) => s.id === activeTab);
  const currentStep = stepIdx >= 0 ? GUIDED_STEPS[stepIdx] : null;

  const is2D = cfdCase.boundaries.some((b) => b.kind === "empty2D");
  const lx = Math.max(1e-4, Math.abs(cfdCase.domain.xMax - cfdCase.domain.xMin));
  const ly = Math.max(1e-4, Math.abs(cfdCase.domain.yMax - cfdCase.domain.yMin));
  const lz = Math.max(1e-4, Math.abs(cfdCase.domain.zMax - cfdCase.domain.zMin));
  const maxDom = Math.max(lx, ly, lz);

  const applyGuidedMeshProfile = (divs: number, rMin: number, rMax: number) => {
    const h = Math.max(1e-4, maxDom / divs);
    const nx = Math.max(4, Math.min(600, Math.round(lx / h)));
    const ny = Math.max(4, Math.min(600, Math.round(ly / h)));
    const nz = is2D ? 1 : Math.max(2, Math.min(400, Math.round(lz / h)));
    onChangeCase((prev) => ({
      ...prev,
      mesh: {
        ...prev.mesh,
        cellsX: nx,
        cellsY: ny,
        cellsZ: nz,
        snappyEnabled: prev.geometryKind !== "none" || prev.mesh.snappyEnabled,
        refinementMin: rMin,
        refinementMax: rMax,
      },
    }));
  };

  const handleGuidedAutoYPlus = async () => {
    try {
      const res = await computeYPlus({
        velocity: Math.max(0.01, cfdCase.physics.referenceVelocity),
        density: cfdCase.physics.fluid.density,
        dynamicViscosity: cfdCase.physics.fluid.dynamicViscosity,
        referenceLength: cfdCase.physics.referenceLength,
        targetYPlus: cfdCase.mesh.targetYPlus,
        expansionRatio: cfdCase.mesh.expansionRatio,
      });
      onChangeCase((prev) => ({
        ...prev,
        mesh: {
          ...prev.mesh,
          firstLayerThickness: Number(res.firstCellHeight.toPrecision(4)),
          inflationLayers: res.recommendedLayers,
        },
      }));
      setYPlusNote(`Δy₁ = ${(res.firstCellHeight * 1000).toFixed(3)} mm · ${res.recommendedLayers} strati`);
    } catch (e) {
      setYPlusNote(String(e));
    }
  };

  const inletPatch = cfdCase.boundaries.find((b) => b.kind === "velocityInlet");
  const inletSpeed = inletPatch
    ? Math.hypot(inletPatch.velocity[0], inletPatch.velocity[1], inletPatch.velocity[2])
    : null;
  const inletMismatch =
    inletSpeed !== null && Math.abs(inletSpeed - cfdCase.physics.referenceVelocity) > 0.05;

  const syncInletVelocity = () => {
    onChangeCase((prev) => ({
      ...prev,
      boundaries: prev.boundaries.map((b) =>
        b.kind === "velocityInlet" ? { ...b, velocity: [prev.physics.referenceVelocity, 0, 0] } : b
      ),
    }));
  };

  const stlMaxDim = cfdCase.stl
    ? Math.max(
        Math.abs(cfdCase.stl.boundsMax[0] - cfdCase.stl.boundsMin[0]),
        Math.abs(cfdCase.stl.boundsMax[1] - cfdCase.stl.boundsMin[1]),
        Math.abs(cfdCase.stl.boundsMax[2] - cfdCase.stl.boundsMin[2])
      )
    : 0;

  return (
    <div className="guided-card" aria-label="Modalità Guidata CFD">
      {/* Header */}
      <div className="guided-header">
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <Compass size={14} style={{ color: "var(--cfd-inlet)" }} />
          <span className="display" style={{ fontSize: 12, fontWeight: 700, color: "var(--text-primary)" }}>
            MODALITÀ GUIDATA
          </span>
          {stepIdx >= 0 && (
            <span className="pill pill-cyan" style={{ fontSize: 9.5, padding: "1px 6px" }}>
              STEP {stepIdx + 1}/{GUIDED_STEPS.length}
            </span>
          )}
        </div>
        <button
          type="button"
          className="btn btn-xs btn-subtle"
          style={{ height: 22, padding: "0 6px", fontSize: 10 }}
          onClick={onDisableGuided}
          title="Disattiva gli aiuti guidati e torna alla modalità standard"
        >
          <X size={11} />
          Aiuti OFF
        </button>
      </div>

      {/* 6-step logical workflow stepper */}
      <div className="guided-stepper">
        {GUIDED_STEPS.map((s, idx) => {
          const isCurrent = s.id === activeTab;
          const isDone = stepIdx > idx;
          return (
            <button
              key={s.id}
              type="button"
              className={`guided-step-pill ${isCurrent ? "current" : isDone ? "done" : ""}`}
              onClick={() => onSelectTab(s.id)}
              title={`${s.shortLabel}: ${s.title}`}
            >
              {idx + 1}
            </button>
          );
        })}
      </div>

      {currentStep ? (
        <>
          <div className="guided-question">{currentStep.question}</div>
          <div className="guided-tip">{currentStep.tip}</div>

          {/* STEP 1: Simulation Type & Geometry */}
          {currentStep.id === "geometry" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div className="guided-choice-grid">
                {SIM_TYPES.map((st) => (
                  <button
                    key={st.id}
                    type="button"
                    className="guided-choice-btn"
                    onClick={() => onSelectPreset(st.presetId)}
                  >
                    <span className="guided-choice-title">{st.label}</span>
                    <span className="guided-choice-desc">{st.desc}</span>
                  </button>
                ))}
              </div>
              <button type="button" className="btn btn-xs" style={{ width: "100%" }} onClick={onImportStl}>
                <Upload size={11} />
                Ho un file CAD (.stl) — Importa e verifica scala
              </button>
              {stlMaxDim > 50 && (
                <div className="notice notice-warn" style={{ fontSize: 10.5, padding: "6px 8px" }}>
                  <AlertTriangle size={11} style={{ verticalAlign: "middle", marginRight: 4 }} />
                  Scala STL sospetta ({stlMaxDim.toFixed(1)} m): usa il pannello <strong>Scaling CAD</strong> qui sotto!
                </div>
              )}
            </div>
          )}

          {/* STEP 2: Physics & Fluid */}
          {currentStep.id === "physics" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {[0, 1, 2, 5].map((fIdx) => {
                  const fl = FLUID_LIBRARY[fIdx];
                  const active = cfdCase.physics.fluid.name === fl.name;
                  return (
                    <button
                      key={fl.name}
                      type="button"
                      className={`btn btn-xs ${active ? "btn-primary" : ""}`}
                      onClick={() =>
                        onChangeCase((prev) => ({
                          ...prev,
                          physics: { ...prev.physics, fluid: { ...fl } },
                        }))
                      }
                    >
                      {fl.name.split(" (")[0]}
                    </button>
                  );
                })}
              </div>

              <div className="field-grid-3">
                {(
                  [
                    { id: "steadyIncompressible", label: "Stazionario" },
                    { id: "transientIncompressible", label: "Transitorio" },
                    { id: "steadyCompressible", label: "Comprimibile" },
                  ] as { id: FlowRegime; label: string }[]
                ).map((r) => (
                  <button
                    key={r.id}
                    type="button"
                    className={`btn btn-xs ${cfdCase.physics.regime === r.id ? "btn-primary" : ""}`}
                    onClick={() =>
                      onChangeCase((prev) => ({
                        ...prev,
                        physics: { ...prev.physics, regime: r.id },
                        numerics:
                          r.id === "transientIncompressible"
                            ? { ...prev.numerics, deltaT: 0.002, endTime: 4.0, writeInterval: 0.2 }
                            : { ...prev.numerics, deltaT: 1.0, endTime: 500, writeInterval: 100 },
                      }))
                    }
                  >
                    {r.label}
                  </button>
                ))}
              </div>

              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {(
                  [
                    { id: "kOmegaSST", label: "k-ω SST" },
                    { id: "kEpsilon", label: "k-ε" },
                    { id: "spalartAllmaras", label: "Spalart-Allm." },
                    { id: "laminar", label: "Laminare" },
                  ] as { id: TurbulenceModel; label: string }[]
                ).map((t) => (
                  <button
                    key={t.id}
                    type="button"
                    className={`btn btn-xs ${cfdCase.physics.turbulence === t.id ? "btn-primary" : ""}`}
                    onClick={() =>
                      onChangeCase((prev) => ({
                        ...prev,
                        physics: { ...prev.physics, turbulence: t.id },
                      }))
                    }
                  >
                    {t.label}
                  </button>
                ))}
              </div>

              {telemetry?.compressibleWarning && (
                <div className="notice notice-warn" style={{ fontSize: 10.5, padding: "6px 8px" }}>
                  Mach = {telemetry.mach.toFixed(2)} &gt; 0.30: consigliato il regime <strong>Comprimibile</strong>.
                </div>
              )}
            </div>
          )}

          {/* STEP 3: Mesh Volume & Surface Granularity */}
          {currentStep.id === "mesh" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div className="field-grid-3">
                <button
                  type="button"
                  className="granularity-btn"
                  onClick={() => applyGuidedMeshProfile(45, 1, 2)}
                >
                  <span className="granularity-name">Bozza</span>
                  <span className="granularity-sub">Vol. Gross. · L1-2</span>
                </button>
                <button
                  type="button"
                  className="granularity-btn"
                  onClick={() => applyGuidedMeshProfile(85, 2, 3)}
                >
                  <span className="granularity-name">Bilanciata</span>
                  <span className="granularity-sub">Vol. Media · L2-3</span>
                </button>
                <button
                  type="button"
                  className="granularity-btn"
                  onClick={() => applyGuidedMeshProfile(135, 3, 4)}
                >
                  <span className="granularity-name">Alta Prec.</span>
                  <span className="granularity-sub">Vol. Fine · L3-4</span>
                </button>
              </div>
              <button
                type="button"
                className="btn btn-xs"
                style={{ width: "100%" }}
                onClick={handleGuidedAutoYPlus}
              >
                <Sparkles size={11} />
                Auto-calcola Δy₁ e Strato Limite dalla Fisica (y+ = {cfdCase.mesh.targetYPlus})
              </button>
              {yPlusNote && (
                <div className="mono" style={{ fontSize: 10, color: "var(--cfd-outlet)" }}>
                  ✓ {yPlusNote}
                </div>
              )}
            </div>
          )}

          {/* STEP 4: Boundary Conditions */}
          {currentStep.id === "boundaries" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
                {cfdCase.boundaries.map((b, i) => (
                  <button
                    key={`${b.face}-${i}`}
                    type="button"
                    className="btn btn-xs"
                    onMouseEnter={() => onHighlightFace(b.face)}
                    onMouseLeave={() => onHighlightFace(null)}
                    onClick={() => onHighlightFace(b.face)}
                  >
                    {b.name} ({b.face})
                  </button>
                ))}
              </div>
              {inletMismatch && (
                <button
                  type="button"
                  className="btn btn-primary btn-xs"
                  style={{ width: "100%" }}
                  onClick={syncInletVelocity}
                >
                  Sincronizza U_inlet ({inletSpeed?.toFixed(1)} m/s) → U∞ ({cfdCase.physics.referenceVelocity} m/s)
                </button>
              )}
            </div>
          )}

          {/* STEP 5: Numerics */}
          {currentStep.id === "numerics" && (
            <div className="field-grid-3">
              {(
                [
                  { id: "robustUpwind", label: "1° Ordine", sub: "Stabile" },
                  { id: "balancedSecondOrder", label: "2° Ordine", sub: "Standard" },
                  { id: "highResolution", label: "Alta Ris.", sub: "Trans./LES" },
                ] as { id: SchemeAccuracy; label: string; sub: string }[]
              ).map((s) => (
                <button
                  key={s.id}
                  type="button"
                  className={`granularity-btn ${cfdCase.numerics.accuracy === s.id ? "active" : ""}`}
                  onClick={() =>
                    onChangeCase((prev) => ({
                      ...prev,
                      numerics: { ...prev.numerics, accuracy: s.id },
                    }))
                  }
                >
                  <span className="granularity-name">{s.label}</span>
                  <span className="granularity-sub">{s.sub}</span>
                </button>
              ))}
            </div>
          )}

          {/* STEP 6: Audit & Solve */}
          {currentStep.id === "run" && (
            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div
                className={`notice ${
                  !validation
                    ? ""
                    : validation.errorCount > 0
                      ? "notice-error"
                      : validation.warningCount > 0
                        ? "notice-warn"
                        : "notice-pass"
                }`}
                style={{ fontSize: 11, padding: "6px 8px", display: "flex", alignItems: "center", gap: 6 }}
              >
                <CheckCircle2 size={13} />
                <span>
                  Audit Pre-Volo: <strong>{validation ? `${validation.score}/100` : "—"}</strong>
                  {validation ? ` (${validation.errorCount} errori, ${validation.warningCount} avvisi)` : ""}
                </span>
              </div>
              <div className="field-grid-2">
                <button type="button" className="btn btn-xs" onClick={onExportCase}>
                  <Download size={11} />
                  Esporta OpenFOAM
                </button>
                <button
                  type="button"
                  className="btn btn-primary btn-xs"
                  onClick={onStartSolver}
                  disabled={isRunning}
                >
                  <Play size={11} fill="currentColor" />
                  {isRunning ? "In esecuzione…" : "Avvia Solutore"}
                </button>
              </div>
            </div>
          )}

          {/* Step Navigation Footer */}
          <div className="guided-footer">
            <button
              type="button"
              className="btn btn-xs"
              disabled={stepIdx <= 0}
              onClick={() => {
                if (stepIdx > 0) onSelectTab(GUIDED_STEPS[stepIdx - 1].id);
              }}
            >
              <ChevronLeft size={12} />
              Indietro
            </button>
            <span className="mono" style={{ fontSize: 10, color: "var(--text-muted)" }}>
              {currentStep.shortLabel}
            </span>
            <button
              type="button"
              className="btn btn-primary btn-xs"
              disabled={stepIdx >= GUIDED_STEPS.length - 1}
              onClick={() => {
                if (stepIdx < GUIDED_STEPS.length - 1) onSelectTab(GUIDED_STEPS[stepIdx + 1].id);
              }}
            >
              Avanti
              <ChevronRight size={12} />
            </button>
          </div>
        </>
      ) : (
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div className="guided-tip">
            Stai usando uno strumento avanzato ({activeTab.toUpperCase()}). Puoi tornare in qualsiasi momento al flusso
            guidato passo-passo.
          </div>
          <button
            type="button"
            className="btn btn-primary btn-xs"
            onClick={() => onSelectTab("geometry")}
          >
            Torna al flusso guidato (Step 1)
          </button>
        </div>
      )}
    </div>
  );
};
