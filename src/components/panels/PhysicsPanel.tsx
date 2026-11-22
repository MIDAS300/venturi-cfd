import React, { useState } from "react";
import { Wind, Thermometer, Flame } from "lucide-react";
import { CfdCase, FlowRegime, TurbulenceModel } from "../../types/cfd";
import { FLUID_LIBRARY } from "../../data/presets";
import { computeAirSutherland, computeTurbulenceIc } from "../../api/bridge";

interface PhysicsPanelProps {
  cfdCase: CfdCase;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
}

export const PhysicsPanel: React.FC<PhysicsPanelProps> = ({ cfdCase, onChange }) => {
  const p = cfdCase.physics;
  const [airTempC, setAirTempC] = useState(Number((p.fluid.temperatureK - 273.15).toFixed(1)));
  const [airPressKpa, setAirPressKpa] = useState(Number((p.fluid.pressurePa / 1000).toFixed(2)));
  const [suthNote, setSuthNote] = useState<string | null>(null);

  const applySutherlandAir = async () => {
    try {
      const fluid = await computeAirSutherland(airTempC + 273.15, airPressKpa * 1000);
      onChange((prev) => ({
        ...prev,
        physics: { ...prev.physics, fluid },
      }));
      setSuthNote(`Computed via Sutherland Law: ρ = ${fluid.density.toFixed(4)} kg/m³, ν = ${fluid.kinematicViscosity.toExponential(3)} m²/s`);
    } catch (e) {
      setSuthNote(String(e));
    }
  };

  const recalcTurbulenceIc = async (vel: number, refLen: number, nu: number) => {
    try {
      const res = await computeTurbulenceIc({
        velocity: Math.max(0.01, vel),
        intensityPercent: Math.max(0.1, p.turbulenceIc.intensity * 100),
        lengthScale: Math.max(1e-4, 0.07 * refLen),
        kinematicViscosity: Math.max(1e-10, nu),
      });
      onChange((prev) => ({
        ...prev,
        physics: {
          ...prev.physics,
          turbulenceIc: {
            ...prev.physics.turbulenceIc,
            lengthScale: Number((0.07 * refLen).toPrecision(4)),
            k: res.k,
            epsilon: res.epsilon,
            omega: res.omega,
            nut: res.nut,
            nuTilda: res.nuTilda,
          },
        },
      }));
    } catch {
      // ignore invalid intermediate input
    }
  };

  return (
    <>
      {/* Solver Regime & Turbulence */}
      <div className="section-block">
        <div className="section-title">
          <span>Flow Regime & Turbulence Modeling</span>
          <Wind size={12} />
        </div>

        <div className="field">
          <label>Flow Regime</label>
          <select
            className="select"
            value={p.regime}
            onChange={(e) => {
              const regime = e.target.value as FlowRegime;
              onChange((prev) => ({
                ...prev,
                physics: { ...prev.physics, regime },
                numerics:
                  regime === "transientIncompressible"
                    ? { ...prev.numerics, deltaT: 0.002, endTime: 4.0, writeInterval: 0.2 }
                    : { ...prev.numerics, deltaT: 1.0, endTime: 500, writeInterval: 100 },
              }));
            }}
          >
            <option value="steadyIncompressible">Steady Incompressible (SIMPLE / foamRun)</option>
            <option value="transientIncompressible">Transient Incompressible (PIMPLE / foamRun)</option>
            <option value="steadyCompressible">Steady Compressible (rhoSimpleFoam / fluid)</option>
          </select>
        </div>

        <div className="field">
          <label>Turbulence Closure</label>
          <select
            className="select"
            value={p.turbulence}
            onChange={(e) =>
              onChange((prev) => ({
                ...prev,
                physics: { ...prev.physics, turbulence: e.target.value as TurbulenceModel },
              }))
            }
          >
            <option value="kOmegaSST">RANS — k-ω SST (Menter, Aero & Separation)</option>
            <option value="kEpsilon">RANS — Standard k-ε (Industrial High-Re)</option>
            <option value="spalartAllmaras">RANS — Spalart-Allmaras (External Aero 1-Eq)</option>
            <option value="waleLes">LES — WALE Subgrid Scale (Unsteady Wake)</option>
            <option value="laminar">Laminar (No Turbulence Model)</option>
          </select>
        </div>

        <div className="field-grid-2">
          <div className="field">
            <label>
              Ref Velocity U∞ <span className="field-unit">m/s</span>
            </label>
            <input
              type="number"
              step="0.5"
              className="input input-mono"
              value={p.referenceVelocity}
              onChange={(e) => {
                const v = Math.max(0.001, parseFloat(e.target.value) || 0.001);
                onChange((prev) => ({
                  ...prev,
                  physics: { ...prev.physics, referenceVelocity: v },
                  boundaries: prev.boundaries.map((b) =>
                    b.kind === "velocityInlet" ? { ...b, velocity: [v, 0, 0] } : b
                  ),
                }));
              }}
            />
          </div>
          <div className="field">
            <label>
              Char. Length L <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.05"
              className="input input-mono"
              value={p.referenceLength}
              onChange={(e) => {
                const l = Math.max(0.001, parseFloat(e.target.value) || 0.001);
                onChange((prev) => ({
                  ...prev,
                  physics: { ...prev.physics, referenceLength: l },
                }));
              }}
            />
          </div>
        </div>

        <button
          type="button"
          className="btn btn-xs"
          onClick={() => recalcTurbulenceIc(p.referenceVelocity, p.referenceLength, p.fluid.kinematicViscosity)}
        >
          <Flame size={11} />
          Recompute Turbulence ICs (k, ω, ε, νt) from U∞ & L
        </button>
      </div>

      {/* Fluid Material Library */}
      <div className="section-block">
        <div className="section-title">
          <span>Fluid Thermophysical Material</span>
          <Thermometer size={12} />
        </div>

        <div className="field">
          <label>Fluid Preset Library</label>
          <select
            className="select"
            value={FLUID_LIBRARY.findIndex((f) => f.name === p.fluid.name)}
            onChange={(e) => {
              const idx = parseInt(e.target.value, 10);
              if (FLUID_LIBRARY[idx]) {
                onChange((prev) => ({
                  ...prev,
                  physics: { ...prev.physics, fluid: { ...FLUID_LIBRARY[idx] } },
                }));
                setAirTempC(Number((FLUID_LIBRARY[idx].temperatureK - 273.15).toFixed(1)));
              }
            }}
          >
            <option value={-1}>Custom / Sutherland Calculated</option>
            {FLUID_LIBRARY.map((f, idx) => (
              <option key={f.name} value={idx}>
                {f.name}
              </option>
            ))}
          </select>
        </div>

        <div className="readout-box">
          <div className="readout-row">
            <span>Density ρ</span>
            <span>{p.fluid.density.toFixed(4)} kg/m³</span>
          </div>
          <div className="readout-row">
            <span>Dynamic Viscosity μ</span>
            <span>{p.fluid.dynamicViscosity.toExponential(3)} Pa·s</span>
          </div>
          <div className="readout-row">
            <span>Kinematic Viscosity ν</span>
            <span style={{ color: "var(--cfd-inlet)" }}>
              {p.fluid.kinematicViscosity.toExponential(3)} m²/s
            </span>
          </div>
        </div>

        {/* Live Sutherland Air Calculator */}
        <div
          style={{
            padding: "9px 10px",
            background: "var(--bg-input)",
            border: "1px solid var(--border-subtle)",
            borderRadius: "var(--radius-md)",
            display: "flex",
            flexDirection: "column",
            gap: 8,
          }}
        >
          <span style={{ fontSize: 10.5, fontWeight: 600, color: "var(--text-secondary)", textTransform: "uppercase" }}>
            Sutherland Air Law (Custom T, P)
          </span>
          <div className="field-grid-2">
            <div className="field">
              <label>
                Temp <span className="field-unit">°C</span>
              </label>
              <input
                type="number"
                step="5"
                className="input input-mono"
                value={airTempC}
                onChange={(e) => setAirTempC(parseFloat(e.target.value) || 0)}
              />
            </div>
            <div className="field">
              <label>
                Pressure <span className="field-unit">kPa</span>
              </label>
              <input
                type="number"
                step="1"
                className="input input-mono"
                value={airPressKpa}
                onChange={(e) => setAirPressKpa(parseFloat(e.target.value) || 101.325)}
              />
            </div>
          </div>
          <button type="button" className="btn btn-xs" onClick={applySutherlandAir}>
            Apply Sutherland Air Properties
          </button>
          {suthNote && <div className="notice notice-pass">{suthNote}</div>}
        </div>
      </div>

      {/* Gravity / Body Force */}
      <div className="section-block">
        <label className="toggle-row">
          <span>Include Gravity Vector (constant/g)</span>
          <input
            type="checkbox"
            checked={p.gravityEnabled}
            onChange={(e) =>
              onChange((prev) => ({
                ...prev,
                physics: { ...prev.physics, gravityEnabled: e.target.checked },
              }))
            }
          />
        </label>
        {p.gravityEnabled && (
          <div className="field-grid-3">
            {(["gx", "gy", "gz"] as const).map((lbl, idx) => (
              <div key={lbl} className="field">
                <label>{lbl}</label>
                <input
                  type="number"
                  step="0.1"
                  className="input input-mono"
                  value={p.gravity[idx]}
                  onChange={(e) => {
                    const val = parseFloat(e.target.value) || 0;
                    onChange((prev) => {
                      const g: [number, number, number] = [...prev.physics.gravity];
                      g[idx] = val;
                      return { ...prev, physics: { ...prev.physics, gravity: g } };
                    });
                  }}
                />
              </div>
            ))}
          </div>
        )}
      </div>
    </>
  );
};
