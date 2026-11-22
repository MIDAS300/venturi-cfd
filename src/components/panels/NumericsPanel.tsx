import React from "react";
import { SlidersHorizontal, Cpu, Clock } from "lucide-react";
import { CfdCase, SchemeAccuracy } from "../../types/cfd";

interface NumericsPanelProps {
  cfdCase: CfdCase;
  maxCores: number;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
}

const SCHEME_CARDS: { id: SchemeAccuracy; title: string; detail: string }[] = [
  {
    id: "robustUpwind",
    title: "Robust First-Order (Upwind)",
    detail: "Highest stability for initial runs or skewed meshes; more numerical diffusion.",
  },
  {
    id: "balancedSecondOrder",
    title: "Balanced Second-Order (linearUpwind)",
    detail: "Industry standard for RANS drag/lift accuracy with cellLimited gradients.",
  },
  {
    id: "highResolution",
    title: "High-Resolution (linearUpwindV / backward)",
    detail: "Best for transient vortex shedding, wake dynamics, and LES accuracy.",
  },
];

export const NumericsPanel: React.FC<NumericsPanelProps> = ({
  cfdCase,
  maxCores,
  onChange,
}) => {
  const n = cfdCase.numerics;
  const isTransient = cfdCase.physics.regime === "transientIncompressible";

  const update = <K extends keyof typeof n>(key: K, value: (typeof n)[K]) => {
    onChange((prev) => ({
      ...prev,
      numerics: { ...prev.numerics, [key]: value },
    }));
  };

  return (
    <>
      {/* Discretization Schemes */}
      <div className="section-block">
        <div className="section-title">
          <span>Discretization Schemes (fvSchemes)</span>
          <SlidersHorizontal size={12} />
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          {SCHEME_CARDS.map((card) => {
            const active = n.accuracy === card.id;
            return (
              <button
                key={card.id}
                type="button"
                className={`patch-card ${active ? "selected" : ""}`}
                style={{ textAlign: "left" }}
                onClick={() => update("accuracy", card.id)}
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontWeight: 600, fontSize: 12, color: "var(--text-primary)" }}>
                    {card.title}
                  </span>
                  {active && <span className="pill pill-warn">SELECTED</span>}
                </div>
                <span style={{ fontSize: 11, color: "var(--text-muted)" }}>{card.detail}</span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Time / Iteration Controls */}
      <div className="section-block">
        <div className="section-title">
          <span>{isTransient ? "Time Integration (controlDict)" : "Iteration Control (SIMPLE)"}</span>
          <Clock size={12} />
        </div>

        <div className="field-grid-2">
          <div className="field">
            <label>
              endTime <span className="field-unit">{isTransient ? "s" : "iters"}</span>
            </label>
            <input
              type="number"
              step={isTransient ? "0.5" : "50"}
              className="input input-mono"
              value={n.endTime}
              onChange={(e) => update("endTime", Math.max(0.001, parseFloat(e.target.value) || 1))}
            />
          </div>
          <div className="field">
            <label>
              deltaT <span className="field-unit">{isTransient ? "s" : "step"}</span>
            </label>
            <input
              type="number"
              step={isTransient ? "0.0005" : "1"}
              className="input input-mono"
              value={n.deltaT}
              onChange={(e) => update("deltaT", Math.max(1e-7, parseFloat(e.target.value) || 1))}
            />
          </div>
          <div className="field">
            <label>
              writeInterval <span className="field-unit">{isTransient ? "s" : "iters"}</span>
            </label>
            <input
              type="number"
              step={isTransient ? "0.1" : "25"}
              className="input input-mono"
              value={n.writeInterval}
              onChange={(e) => update("writeInterval", Math.max(1e-5, parseFloat(e.target.value) || 10))}
            />
          </div>
          <div className="field">
            <label>
              Residual Target <span className="field-unit">tol</span>
            </label>
            <select
              className="select input-mono"
              value={n.convergenceTol}
              onChange={(e) => update("convergenceTol", parseFloat(e.target.value))}
            >
              <option value={1e-3}>1e-3 (Coarse)</option>
              <option value={1e-4}>1e-4 (Standard)</option>
              <option value={1e-5}>1e-5 (Tight)</option>
              <option value={1e-6}>1e-6 (Benchmark)</option>
            </select>
          </div>
        </div>

        {isTransient && (
          <div className="field">
            <label>
              Max Courant Number (maxCo) <span className="field-unit">{n.maxCo.toFixed(2)}</span>
            </label>
            <input
              type="range"
              min="0.2"
              max="3.0"
              step="0.05"
              value={n.maxCo}
              onChange={(e) => update("maxCo", parseFloat(e.target.value))}
            />
          </div>
        )}
      </div>

      {/* Relaxation & Correctors */}
      <div className="section-block">
        <div className="section-title">
          <span>Under-Relaxation & Non-Orthogonality</span>
        </div>

        <div className="field">
          <label>
            Pressure p Relaxation <span className="field-unit">{n.relaxP.toFixed(2)}</span>
          </label>
          <input
            type="range"
            min="0.1"
            max="0.7"
            step="0.05"
            value={n.relaxP}
            onChange={(e) => update("relaxP", parseFloat(e.target.value))}
          />
        </div>

        <div className="field">
          <label>
            Velocity U Relaxation <span className="field-unit">{n.relaxU.toFixed(2)}</span>
          </label>
          <input
            type="range"
            min="0.2"
            max="0.95"
            step="0.05"
            value={n.relaxU}
            onChange={(e) => update("relaxU", parseFloat(e.target.value))}
          />
        </div>

        <div className="field">
          <label>
            Turbulence (k, ω, ε) Relaxation <span className="field-unit">{n.relaxTurb.toFixed(2)}</span>
          </label>
          <input
            type="range"
            min="0.2"
            max="0.95"
            step="0.05"
            value={n.relaxTurb}
            onChange={(e) => update("relaxTurb", parseFloat(e.target.value))}
          />
        </div>

        <div className="field-grid-2">
          <div className="field">
            <label>Non-Ortho Correctors</label>
            <input
              type="number"
              min={0}
              max={5}
              className="input input-mono"
              value={n.nNonOrthoCorrectors}
              onChange={(e) =>
                update("nNonOrthoCorrectors", Math.max(0, Math.min(5, parseInt(e.target.value, 10) || 0)))
              }
            />
          </div>
          <div className="field">
            <label>
              MPI Cores (scotch) <span className="field-unit">max {maxCores}</span>
            </label>
            <input
              type="number"
              min={1}
              max={Math.max(1, maxCores)}
              className="input input-mono"
              value={n.parallelCores}
              onChange={(e) =>
                update(
                  "parallelCores",
                  Math.max(1, Math.min(Math.max(1, maxCores), parseInt(e.target.value, 10) || 1))
                )
              }
            />
          </div>
        </div>

        <div className="notice" style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <Cpu size={14} style={{ flexShrink: 0, color: "var(--cfd-inlet)" }} />
          <span>
            {n.parallelCores > 1
              ? `Generates system/decomposeParDict (method scotch, ${n.parallelCores} domains) and mpirun in Allrun.`
              : "Serial execution (1 core). Increase MPI Cores above 1 to auto-generate decomposeParDict."}
          </span>
        </div>
      </div>
    </>
  );
};
