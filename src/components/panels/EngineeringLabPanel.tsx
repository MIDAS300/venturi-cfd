import React, { useEffect, useState } from "react";
import { Check, Calculator, Waves, Timer } from "lucide-react";
import {
  CfdCase,
  CflResult,
  TelemetrySnapshot,
  TurbulenceIcResult,
  YPlusResult,
} from "../../types/cfd";
import { computeCfl, computeTurbulenceIc, computeYPlus } from "../../api/bridge";

interface EngineeringLabPanelProps {
  cfdCase: CfdCase;
  telemetry: TelemetrySnapshot | null;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
}

export const EngineeringLabPanel: React.FC<EngineeringLabPanelProps> = ({
  cfdCase,
  telemetry,
  onChange,
}) => {
  const [subTab, setSubTab] = useState<"yplus" | "turbulence" | "cfl">("yplus");
  const [appliedBanner, setAppliedBanner] = useState<string | null>(null);

  // y+ calculator inputs
  const [ypVel, setYpVel] = useState(cfdCase.physics.referenceVelocity);
  const [ypLen, setYpLen] = useState(cfdCase.physics.referenceLength);
  const [ypTarget, setYpTarget] = useState(cfdCase.mesh.targetYPlus);
  const [ypRatio, setYpRatio] = useState(cfdCase.mesh.expansionRatio);
  const [ypResult, setYpResult] = useState<YPlusResult | null>(null);

  // Turbulence IC inputs
  const [turbVel, setTurbVel] = useState(cfdCase.physics.referenceVelocity);
  const [turbInt, setTurbInt] = useState(Number((cfdCase.physics.turbulenceIc.intensity * 100).toFixed(2)));
  const [turbLen, setTurbLen] = useState(cfdCase.physics.turbulenceIc.lengthScale);
  const [turbRes, setTurbRes] = useState<TurbulenceIcResult | null>(null);

  // CFL inputs
  const [cflDx, setCflDx] = useState(telemetry?.minDx ?? 0.01);
  const [cflVel, setCflVel] = useState(cfdCase.physics.referenceVelocity);
  const [cflTarget, setCflTarget] = useState(cfdCase.numerics.maxCo);
  const [cflDomainL, setCflDomainL] = useState(Math.abs(cfdCase.domain.xMax - cfdCase.domain.xMin));
  const [cflRes, setCflRes] = useState<CflResult | null>(null);

  // Sync lab inputs automatically when user switches case preset
  useEffect(() => {
    setYpVel(cfdCase.physics.referenceVelocity);
    setYpLen(cfdCase.physics.referenceLength);
    setYpTarget(cfdCase.mesh.targetYPlus);
    setYpRatio(cfdCase.mesh.expansionRatio);
    setTurbVel(cfdCase.physics.referenceVelocity);
    setTurbInt(Number((cfdCase.physics.turbulenceIc.intensity * 100).toFixed(2)));
    setTurbLen(cfdCase.physics.turbulenceIc.lengthScale);
    setCflVel(cfdCase.physics.referenceVelocity);
    setCflTarget(cfdCase.numerics.maxCo);
    setCflDomainL(Math.abs(cfdCase.domain.xMax - cfdCase.domain.xMin));
    if (telemetry?.minDx) setCflDx(Number(telemetry.minDx.toPrecision(3)));
  }, [cfdCase.name]);

  useEffect(() => {
    let active = true;
    computeYPlus({
      velocity: ypVel,
      density: cfdCase.physics.fluid.density,
      dynamicViscosity: cfdCase.physics.fluid.dynamicViscosity,
      referenceLength: ypLen,
      targetYPlus: ypTarget,
      expansionRatio: ypRatio,
    })
      .then((r) => active && setYpResult(r))
      .catch(() => active && setYpResult(null));
    return () => {
      active = false;
    };
  }, [ypVel, ypLen, ypTarget, ypRatio, cfdCase.physics.fluid.density, cfdCase.physics.fluid.dynamicViscosity]);

  useEffect(() => {
    let active = true;
    computeTurbulenceIc({
      velocity: turbVel,
      intensityPercent: turbInt,
      lengthScale: turbLen,
      kinematicViscosity: cfdCase.physics.fluid.kinematicViscosity,
    })
      .then((r) => active && setTurbRes(r))
      .catch(() => active && setTurbRes(null));
    return () => {
      active = false;
    };
  }, [turbVel, turbInt, turbLen, cfdCase.physics.fluid.kinematicViscosity]);

  useEffect(() => {
    let active = true;
    computeCfl({
      minCellSize: cflDx,
      maxVelocity: cflVel,
      targetCourant: cflTarget,
      domainLength: cflDomainL,
    })
      .then((r) => active && setCflRes(r))
      .catch(() => active && setCflRes(null));
    return () => {
      active = false;
    };
  }, [cflDx, cflVel, cflTarget, cflDomainL]);

  const flashBanner = (msg: string) => {
    setAppliedBanner(msg);
    setTimeout(() => setAppliedBanner(null), 2600);
  };

  return (
    <>
      <div className="segmented" style={{ width: "100%", display: "grid", gridTemplateColumns: "1fr 1fr 1fr" }}>
        <button type="button" className={subTab === "yplus" ? "active" : ""} onClick={() => setSubTab("yplus")}>
          y+ & Layers
        </button>
        <button
          type="button"
          className={subTab === "turbulence" ? "active" : ""}
          onClick={() => setSubTab("turbulence")}
        >
          Turb. IC
        </button>
        <button type="button" className={subTab === "cfl" ? "active" : ""} onClick={() => setSubTab("cfl")}>
          CFL / Δt
        </button>
      </div>

      {appliedBanner && (
        <div className="notice notice-pass" style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <Check size={13} />
          <span>{appliedBanner}</span>
        </div>
      )}

      {subTab === "yplus" && (
        <div className="section-block">
          <div className="section-title">
            <span>Boundary Layer & y+ Calculator</span>
            <Calculator size={12} />
          </div>

          <div className="field-grid-2">
            <div className="field">
              <label>
                Freestream U <span className="field-unit">m/s</span>
              </label>
              <input
                type="number"
                step="1"
                className="input input-mono"
                value={ypVel}
                onChange={(e) => setYpVel(Math.max(0.01, parseFloat(e.target.value) || 0.01))}
              />
            </div>
            <div className="field">
              <label>
                Length L <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.1"
                className="input input-mono"
                value={ypLen}
                onChange={(e) => setYpLen(Math.max(0.001, parseFloat(e.target.value) || 0.001))}
              />
            </div>
            <div className="field">
              <label>Target y+</label>
              <input
                type="number"
                step="0.5"
                className="input input-mono"
                value={ypTarget}
                onChange={(e) => setYpTarget(Math.max(0.1, parseFloat(e.target.value) || 1))}
              />
            </div>
            <div className="field">
              <label>Growth Ratio r</label>
              <input
                type="number"
                step="0.02"
                className="input input-mono"
                value={ypRatio}
                onChange={(e) => setYpRatio(Math.max(1.05, parseFloat(e.target.value) || 1.2))}
              />
            </div>
          </div>

          <div style={{ display: "flex", gap: 5 }}>
            {[1, 30, 50, 100].map((v) => (
              <button
                key={v}
                type="button"
                className="btn btn-xs btn-subtle"
                style={{ flex: 1 }}
                onClick={() => setYpTarget(v)}
              >
                y+ = {v}
              </button>
            ))}
          </div>

          {ypResult && (
            <>
              <div className="readout-box">
                <div className="readout-row">
                  <span>Reynolds Number Re</span>
                  <span style={{ color: "var(--cfd-inlet)" }}>{ypResult.reynolds.toExponential(3)}</span>
                </div>
                <div className="readout-row">
                  <span>First Cell Height Δy₁</span>
                  <span style={{ color: "var(--accent-amber-bright)" }}>
                    {(ypResult.firstCellHeight * 1000).toFixed(4)} mm
                  </span>
                </div>
                <div className="readout-row">
                  <span>Wall Center Distance yc</span>
                  <span>{(ypResult.wallDistanceYc * 1000).toFixed(4)} mm</span>
                </div>
                <div className="readout-row">
                  <span>Skin Friction Cf</span>
                  <span>{ypResult.skinFrictionCf.toExponential(3)}</span>
                </div>
                <div className="readout-row">
                  <span>Wall Shear τw / uτ</span>
                  <span>
                    {ypResult.wallShearTauW.toFixed(3)} Pa · {ypResult.frictionVelocityUTau.toFixed(3)} m/s
                  </span>
                </div>
                <div className="readout-row">
                  <span>Boundary Layer δ₉₉</span>
                  <span>{(ypResult.boundaryLayerDelta99 * 1000).toFixed(2)} mm</span>
                </div>
                <div className="readout-row">
                  <span>Recommended Prism Layers</span>
                  <span>
                    {ypResult.recommendedLayers} layers ({(ypResult.totalPrismHeight * 1000).toFixed(2)} mm)
                  </span>
                </div>
              </div>

              <div className={`notice ${ypResult.bufferLayerWarning ? "notice-warn" : "notice-pass"}`}>
                {ypResult.wallTreatmentNote}
              </div>

              <button
                type="button"
                className="btn btn-primary"
                style={{ width: "100%" }}
                onClick={() => {
                  onChange((prev) => ({
                    ...prev,
                    mesh: {
                      ...prev.mesh,
                      targetYPlus: ypTarget,
                      expansionRatio: ypRatio,
                      firstLayerThickness: Number(ypResult.firstCellHeight.toPrecision(4)),
                      inflationLayers: ypResult.recommendedLayers,
                    },
                  }));
                  flashBanner("Applied Δy₁, target y+, and prism layers to snappyHexMeshDict!");
                }}
              >
                Apply to snappyHexMeshDict
              </button>
            </>
          )}
        </div>
      )}

      {subTab === "turbulence" && (
        <div className="section-block">
          <div className="section-title">
            <span>RANS / SA Turbulence IC Estimator</span>
            <Waves size={12} />
          </div>

          <div className="field-grid-3">
            <div className="field">
              <label>
                U <span className="field-unit">m/s</span>
              </label>
              <input
                type="number"
                step="1"
                className="input input-mono"
                value={turbVel}
                onChange={(e) => setTurbVel(Math.max(0.01, parseFloat(e.target.value) || 0.01))}
              />
            </div>
            <div className="field">
              <label>
                Intensity I <span className="field-unit">%</span>
              </label>
              <input
                type="number"
                step="0.5"
                className="input input-mono"
                value={turbInt}
                onChange={(e) => setTurbInt(Math.max(0.05, parseFloat(e.target.value) || 0.5))}
              />
            </div>
            <div className="field">
              <label>
                Scale ℓ <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.01"
                className="input input-mono"
                value={turbLen}
                onChange={(e) => setTurbLen(Math.max(0.0005, parseFloat(e.target.value) || 0.01))}
              />
            </div>
          </div>

          <div style={{ display: "flex", gap: 5 }}>
            <button
              type="button"
              className="btn btn-xs btn-subtle"
              style={{ flex: 1 }}
              onClick={() => {
                setTurbInt(0.5);
                setTurbLen(Number((0.07 * cfdCase.physics.referenceLength).toPrecision(3)));
              }}
            >
              External Wind Tunnel (0.5%)
            </button>
            <button
              type="button"
              className="btn btn-xs btn-subtle"
              style={{ flex: 1 }}
              onClick={() => {
                setTurbInt(5.0);
                setTurbLen(Number((0.07 * cfdCase.physics.referenceLength).toPrecision(3)));
              }}
            >
              Internal Pipe (5%)
            </button>
          </div>

          {turbRes && (
            <>
              <div className="readout-box">
                <div className="readout-row">
                  <span>Turbulent Kinetic Energy k</span>
                  <span style={{ color: "var(--accent-amber-bright)" }}>{turbRes.k.toExponential(3)} m²/s²</span>
                </div>
                <div className="readout-row">
                  <span>Specific Dissipation ω</span>
                  <span style={{ color: "var(--cfd-inlet)" }}>{turbRes.omega.toExponential(3)} 1/s</span>
                </div>
                <div className="readout-row">
                  <span>Dissipation Rate ε</span>
                  <span>{turbRes.epsilon.toExponential(3)} m²/s³</span>
                </div>
                <div className="readout-row">
                  <span>Eddy Viscosity νt</span>
                  <span>{turbRes.nut.toExponential(3)} m²/s</span>
                </div>
                <div className="readout-row">
                  <span>Spalart-Allmaras ν̃</span>
                  <span>{turbRes.nuTilda.toExponential(3)} m²/s</span>
                </div>
                <div className="readout-row">
                  <span>Viscosity Ratio νt / ν</span>
                  <span>{turbRes.viscosityRatio.toFixed(1)}×</span>
                </div>
              </div>

              <button
                type="button"
                className="btn btn-primary"
                style={{ width: "100%" }}
                onClick={() => {
                  onChange((prev) => ({
                    ...prev,
                    physics: {
                      ...prev.physics,
                      turbulenceIc: {
                        intensity: turbInt / 100,
                        lengthScale: turbLen,
                        k: turbRes.k,
                        epsilon: turbRes.epsilon,
                        omega: turbRes.omega,
                        nut: turbRes.nut,
                        nuTilda: turbRes.nuTilda,
                      },
                    },
                  }));
                  flashBanner("Injected k, ω, ε, νt, ν̃ directly into 0/* OpenFOAM field files!");
                }}
              >
                Apply to 0/k, 0/omega, 0/epsilon, 0/nut
              </button>
            </>
          )}
        </div>
      )}

      {subTab === "cfl" && (
        <div className="section-block">
          <div className="section-title">
            <span>CFL / Courant Timestep Calculator</span>
            <Timer size={12} />
          </div>

          <div className="field-grid-2">
            <div className="field">
              <label>
                Min Cell Δx <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.001"
                className="input input-mono"
                value={cflDx}
                onChange={(e) => setCflDx(Math.max(1e-6, parseFloat(e.target.value) || 0.001))}
              />
            </div>
            <div className="field">
              <label>
                Max Velocity U <span className="field-unit">m/s</span>
              </label>
              <input
                type="number"
                step="1"
                className="input input-mono"
                value={cflVel}
                onChange={(e) => setCflVel(Math.max(0.01, parseFloat(e.target.value) || 1))}
              />
            </div>
            <div className="field">
              <label>Target Courant Co</label>
              <input
                type="number"
                step="0.1"
                className="input input-mono"
                value={cflTarget}
                onChange={(e) => setCflTarget(Math.max(0.05, parseFloat(e.target.value) || 0.8))}
              />
            </div>
            <div className="field">
              <label>
                Domain Length L <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.5"
                className="input input-mono"
                value={cflDomainL}
                onChange={(e) => setCflDomainL(Math.max(0.01, parseFloat(e.target.value) || 1))}
              />
            </div>
          </div>

          {cflRes && (
            <>
              <div className="readout-box">
                <div className="readout-row">
                  <span>Max Stable Δt</span>
                  <span style={{ color: "var(--accent-amber-bright)" }}>
                    {cflRes.maxDeltaT.toExponential(3)} s
                  </span>
                </div>
                <div className="readout-row">
                  <span>1× Flow-Through Time</span>
                  <span>{cflRes.flowThroughTime.toFixed(3)} s</span>
                </div>
                <div className="readout-row">
                  <span>Recommended endTime (5× FT)</span>
                  <span style={{ color: "var(--cfd-inlet)" }}>{cflRes.recommendedEndTime.toFixed(3)} s</span>
                </div>
                <div className="readout-row">
                  <span>Estimated Timesteps</span>
                  <span>{cflRes.estimatedSteps.toLocaleString()} steps</span>
                </div>
              </div>

              <div className="notice">{cflRes.stabilityNote}</div>

              <button
                type="button"
                className="btn btn-primary"
                style={{ width: "100%" }}
                onClick={() => {
                  onChange((prev) => ({
                    ...prev,
                    physics: { ...prev.physics, regime: "transientIncompressible" },
                    numerics: {
                      ...prev.numerics,
                      deltaT: Number(cflRes.maxDeltaT.toPrecision(3)),
                      endTime: Number(cflRes.recommendedEndTime.toFixed(3)),
                      writeInterval: Number((cflRes.recommendedEndTime / 15).toPrecision(2)),
                      maxCo: cflTarget,
                    },
                  }));
                  flashBanner("Applied transient Δt, endTime, and maxCo to controlDict!");
                }}
              >
                Apply Transient Δt to controlDict
              </button>
            </>
          )}
        </div>
      )}
    </>
  );
};
