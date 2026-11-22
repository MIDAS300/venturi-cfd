import React from "react";
import { BoundaryKind, BoundaryPatch, CfdCase, DomainFace } from "../../types/cfd";

interface BoundariesPanelProps {
  cfdCase: CfdCase;
  highlightedFace: DomainFace | null;
  onHighlightFace: (face: DomainFace | null) => void;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
}

const KIND_META: Record<BoundaryKind, { label: string; color: string }> = {
  velocityInlet: { label: "Velocity Inlet", color: "var(--cfd-inlet)" },
  pressureOutlet: { label: "Pressure Outlet", color: "var(--cfd-outlet)" },
  noSlipWall: { label: "No-Slip Wall", color: "var(--cfd-wall)" },
  movingWall: { label: "Moving Wall", color: "var(--accent-amber)" },
  symmetry: { label: "Symmetry Plane", color: "var(--cfd-symmetry)" },
  empty2D: { label: "Empty (2D)", color: "var(--text-muted)" },
};

const FACE_LABELS: Record<DomainFace, string> = {
  xMin: "X− Face",
  xMax: "X+ Face",
  yMin: "Y− Face",
  yMax: "Y+ Face",
  zMin: "Z− Face",
  zMax: "Z+ Face",
  obstacle: "3D Body / STL",
};

export const BoundariesPanel: React.FC<BoundariesPanelProps> = ({
  cfdCase,
  highlightedFace,
  onHighlightFace,
  onChange,
}) => {
  const updatePatch = (idx: number, patch: Partial<BoundaryPatch>) => {
    onChange((prev) => ({
      ...prev,
      boundaries: prev.boundaries.map((b, i) => (i === idx ? { ...b, ...patch } : b)),
    }));
  };

  return (
    <div className="section-block">
      <div className="section-title">
        <span>Boundary Condition Matrix</span>
        <span className="mono" style={{ color: "var(--cfd-inlet)" }}>
          3D SYNC
        </span>
      </div>
      <div className="notice">
        Hover or select any boundary patch below to highlight its exact surface and flow direction inside the 3D viewport.
      </div>

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        {cfdCase.boundaries.map((b, idx) => {
          const meta = KIND_META[b.kind];
          const isSelected = highlightedFace === b.face;

          return (
            <div
              key={`${b.face}-${idx}`}
              className={`patch-card ${isSelected ? "selected" : ""}`}
              onMouseEnter={() => onHighlightFace(b.face)}
              onMouseLeave={() => onHighlightFace(null)}
              onClick={() => onHighlightFace(isSelected ? null : b.face)}
            >
              <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 7, minWidth: 0 }}>
                  <span
                    style={{
                      width: 8,
                      height: 8,
                      borderRadius: "50%",
                      background: meta.color,
                      boxShadow: `0 0 8px ${meta.color}`,
                      flexShrink: 0,
                    }}
                  />
                  <input
                    className="input input-mono"
                    style={{ height: 24, padding: "0 6px", width: 120, fontSize: 11.5 }}
                    value={b.name}
                    onClick={(e) => e.stopPropagation()}
                    onChange={(e) => updatePatch(idx, { name: e.target.value })}
                  />
                </div>
                <span className="mono" style={{ fontSize: 10, color: "var(--text-muted)" }}>
                  {FACE_LABELS[b.face]}
                </span>
              </div>

              <select
                className="select"
                style={{ height: 27, fontSize: 11.5 }}
                value={b.kind}
                onClick={(e) => e.stopPropagation()}
                onChange={(e) => updatePatch(idx, { kind: e.target.value as BoundaryKind })}
              >
                {Object.entries(KIND_META).map(([k, m]) => (
                  <option key={k} value={k}>
                    {m.label}
                  </option>
                ))}
              </select>

              {(b.kind === "velocityInlet" || b.kind === "movingWall") && (
                <div className="field-grid-3" onClick={(e) => e.stopPropagation()}>
                  {(["Ux", "Uy", "Uz"] as const).map((axis, aIdx) => (
                    <div key={axis} className="field">
                      <label>
                        {axis} <span className="field-unit">m/s</span>
                      </label>
                      <input
                        type="number"
                        step="0.5"
                        className="input input-mono"
                        style={{ height: 25, fontSize: 11 }}
                        value={b.velocity[aIdx]}
                        onChange={(e) => {
                          const v: [number, number, number] = [...b.velocity];
                          v[aIdx] = parseFloat(e.target.value) || 0;
                          updatePatch(idx, { velocity: v });
                        }}
                      />
                    </div>
                  ))}
                </div>
              )}

              {b.kind === "pressureOutlet" && (
                <div className="field" onClick={(e) => e.stopPropagation()}>
                  <label>
                    Kinematic Gauge Pressure p/ρ <span className="field-unit">m²/s²</span>
                  </label>
                  <input
                    type="number"
                    step="1"
                    className="input input-mono"
                    style={{ height: 25, fontSize: 11 }}
                    value={b.pressure}
                    onChange={(e) => updatePatch(idx, { pressure: parseFloat(e.target.value) || 0 })}
                  />
                </div>
              )}

              {b.kind === "noSlipWall" && (
                <div className="field" onClick={(e) => e.stopPropagation()}>
                  <label>
                    Sand-Grain Roughness Ks (0 = smooth) <span className="field-unit">m</span>
                  </label>
                  <input
                    type="number"
                    step="0.00001"
                    min="0"
                    className="input input-mono"
                    style={{ height: 25, fontSize: 11 }}
                    value={b.wallRoughnessKs}
                    onChange={(e) =>
                      updatePatch(idx, { wallRoughnessKs: Math.max(0, parseFloat(e.target.value) || 0) })
                    }
                  />
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
};
