import React, { useEffect, useState } from "react";
import { Upload, Maximize2, Box, Layers, Cpu, Ruler, AlertTriangle, RotateCcw } from "lucide-react";
import { CfdCase, ParametricGeometrySpec, PresetGeometryKind } from "../../types/cfd";
import { generateParametricGeometry } from "../../api/bridge";

interface GeometryPanelProps {
  cfdCase: CfdCase;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
  onImportStl: () => void;
  onGenerateParametric?: (spec: ParametricGeometrySpec) => void;
  onScaleStl?: (scaleFactor: number, autoFitDomain: boolean) => void;
}

type ParametricFamily = ParametricGeometrySpec["family"];

const DEFAULT_PARAMETRIC_SPECS: Record<ParametricFamily, ParametricGeometrySpec> = {
  nacaAirfoil: {
    family: "nacaAirfoil",
    chord: 1.0,
    camberPct: 2.0,
    camberPosPct: 40.0,
    thicknessPct: 12.0,
    aoaDeg: 4.0,
    span: 0.76,
  },
  venturiTube: {
    family: "venturiTube",
    inletRadius: 0.52,
    throatRadius: 0.26,
    outletRadius: 0.52,
    convergentLen: 1.2,
    throatLen: 0.8,
    divergentLen: 2.8,
  },
  ahmedDiffuser: {
    family: "ahmedDiffuser",
    length: 1.044,
    width: 0.389,
    height: 0.288,
    rearSlantDeg: 25.0,
    groundClearance: 0.05,
  },
  pinFinArray: {
    family: "pinFinArray",
    baseLength: 0.24,
    baseWidth: 0.16,
    finCountX: 4,
    finCountZ: 3,
    finRadius: 0.008,
    finHeight: 0.04,
  },
  cylinder: {
    family: "cylinder",
    radius: 0.25,
    span: 0.58,
  },
};

const PRESET_SHAPES: { id: PresetGeometryKind; label: string; desc: string }[] = [
  { id: "airfoilNaca0012", label: "NACA 0012 Airfoil", desc: "Analytical 12% symmetric wing section" },
  { id: "ahmedBody", label: "Ahmed Body (25°)", desc: "ERCOFTAC automotive drag benchmark" },
  { id: "venturiNozzle", label: "Venturi Tube", desc: "Convergent-divergent internal throat" },
  { id: "cylinderCrossflow", label: "Circular Cylinder", desc: "Bluff body wake & shedding benchmark" },
  { id: "none", label: "Empty Domain", desc: "Pure blockMesh box (cavity / channel)" },
];

const CAD_UNIT_PRESETS: { id: string; label: string; factor: number; desc: string }[] = [
  { id: "mm", label: "mm → m", factor: 0.001, desc: "×0.001" },
  { id: "cm", label: "cm → m", factor: 0.01, desc: "×0.01" },
  { id: "in", label: "in → m", factor: 0.0254, desc: "×0.0254" },
  { id: "ft", label: "ft → m", factor: 0.3048, desc: "×0.3048" },
  { id: "m", label: "m (1:1)", factor: 1.0, desc: "×1" },
];

export const GeometryPanel: React.FC<GeometryPanelProps> = ({
  cfdCase,
  onChange,
  onImportStl,
  onGenerateParametric,
  onScaleStl,
}) => {
  const [draftSpec, setDraftSpec] = useState<ParametricGeometrySpec>(
    () => cfdCase.parametric ?? DEFAULT_PARAMETRIC_SPECS.nacaAirfoil
  );
  const [genBusy, setGenBusy] = useState(false);
  const [genMessage, setGenMessage] = useState<string | null>(null);

  // CAD STL Scaling controls
  const activeScale = cfdCase.stl?.scaleFactor ?? 1.0;
  const [customScaleInput, setCustomScaleInput] = useState<string>(String(activeScale));
  const [targetAxis, setTargetAxis] = useState<0 | 1 | 2>(0);
  const [targetRealMeters, setTargetRealMeters] = useState<string>("");
  const [autoFitOnScale, setAutoFitOnScale] = useState<boolean>(true);

  const parametricKey = cfdCase.parametric ? JSON.stringify(cfdCase.parametric) : "";
  useEffect(() => {
    if (cfdCase.parametric) {
      setDraftSpec(cfdCase.parametric);
    }
  }, [parametricKey]);

  const stlKey = cfdCase.stl
    ? `${cfdCase.stl.filename}:${cfdCase.stl.scaleFactor ?? 1}:${cfdCase.stl.boundsMin.join(",")}:${cfdCase.stl.boundsMax.join(",")}`
    : "";
  useEffect(() => {
    if (!cfdCase.stl) return;
    const s = cfdCase.stl.scaleFactor ?? 1.0;
    setCustomScaleInput(Number(s.toPrecision(6)).toString());
    const curAxisLen = Math.abs(cfdCase.stl.boundsMax[targetAxis] - cfdCase.stl.boundsMin[targetAxis]);
    setTargetRealMeters(curAxisLen > 0 ? Number(curAxisLen.toPrecision(4)).toString() : "");
  }, [stlKey, targetAxis]);

  const d = cfdCase.domain;
  const lx = Math.max(0, d.xMax - d.xMin);
  const ly = Math.max(0, d.yMax - d.yMin);
  const lz = Math.max(0, d.zMax - d.zMin);

  const updateDomain = (key: keyof typeof d, value: number) => {
    if (!Number.isFinite(value)) return;
    onChange((prev) => ({
      ...prev,
      domain: { ...prev.domain, [key]: value },
    }));
  };

  const handleFamilySelect = (family: ParametricFamily) => {
    if (cfdCase.parametric?.family === family) {
      setDraftSpec(cfdCase.parametric);
    } else {
      setDraftSpec({ ...DEFAULT_PARAMETRIC_SPECS[family] });
    }
    setGenMessage(null);
  };

  const handleGenerateStl = async () => {
    if (onGenerateParametric) {
      setGenMessage(null);
      onGenerateParametric(draftSpec);
      return;
    }
    setGenBusy(true);
    setGenMessage(null);
    try {
      const payload = await generateParametricGeometry(draftSpec);
      onChange((prev) => ({
        ...prev,
        geometryKind: "parametric",
        parametric: draftSpec,
        stl: payload.summary,
        mesh: { ...prev.mesh, snappyEnabled: true },
      }));
      setGenMessage(
        `Generated ${payload.summary.filename} (${payload.summary.triangleCount.toLocaleString()} tris)`
      );
    } catch (e) {
      setGenMessage(`Generation failed: ${e}`);
    } finally {
      setGenBusy(false);
    }
  };

  const autoFitStlDomain = () => {
    if (!cfdCase.stl) return;
    const [minX, minY, minZ] = cfdCase.stl.boundsMin;
    const [maxX, maxY, maxZ] = cfdCase.stl.boundsMax;
    const sx = Math.max(0.1, maxX - minX);
    const sy = Math.max(0.1, maxY - minY);
    const sz = Math.max(0.1, maxZ - minZ);
    // Standard aerodynamic wind-tunnel margins: 3L upstream, 6L wake, 2.5H top/bottom
    onChange((prev) => ({
      ...prev,
      domain: {
        xMin: Number((minX - 3.0 * sx).toFixed(3)),
        xMax: Number((maxX + 6.0 * sx).toFixed(3)),
        yMin: Number((minY - 2.5 * sy).toFixed(3)),
        yMax: Number((maxY + 2.5 * sy).toFixed(3)),
        zMin: Number((minZ - 2.0 * sz).toFixed(3)),
        zMax: Number((maxZ + 2.0 * sz).toFixed(3)),
      },
      physics: {
        ...prev.physics,
        referenceLength: Number(sx.toFixed(4)),
      },
    }));
  };

  const nacaCode =
    draftSpec.family === "nacaAirfoil"
      ? `NACA ${Math.round(draftSpec.camberPct)}${Math.round(draftSpec.camberPosPct / 10)}${String(
          Math.round(draftSpec.thicknessPct)
        ).padStart(2, "0")}`
      : null;

  return (
    <>
      {/* Native Rust Parametric Generator */}
      <div className="section-block">
        <div className="section-title">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
            <Cpu size={12} />
            Native Rust Parametric Generator
          </span>
          {cfdCase.geometryKind === "parametric" ? (
            <span className="pill pill-cyan" style={{ fontSize: 9.5 }}>
              PARAMETRIC
            </span>
          ) : (
            <span className="mono" style={{ color: "var(--accent-amber)", fontSize: 10 }}>
              WATERTIGHT STL
            </span>
          )}
        </div>

        <div className="field">
          <label>
            Geometry Family
            {nacaCode && (
              <span className="mono" style={{ color: "var(--cfd-inlet)", fontSize: 10 }}>
                {nacaCode}
              </span>
            )}
          </label>
          <select
            className="select"
            value={draftSpec.family}
            onChange={(e) => handleFamilySelect(e.target.value as ParametricFamily)}
          >
            <option value="nacaAirfoil">NACA Airfoil (4-Digit Cambered)</option>
            <option value="venturiTube">Venturi Tube (Convergent-Divergent)</option>
            <option value="ahmedDiffuser">Ahmed Diffuser (Automotive Slant)</option>
            <option value="pinFinArray">Pin-Fin Array (Thermal Heatsink)</option>
            <option value="cylinder">Circular Cylinder (Crossflow)</option>
          </select>
        </div>

        {draftSpec.family === "nacaAirfoil" && (
          <>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  Camber <span className="field-unit">%</span>
                </label>
                <input
                  type="number"
                  step="0.5"
                  min={0}
                  max={9}
                  className="input input-mono"
                  value={draftSpec.camberPct}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, camberPct: Math.max(0, parseFloat(e.target.value) || 0) })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Thick <span className="field-unit">%</span>
                </label>
                <input
                  type="number"
                  step="1"
                  min={4}
                  max={35}
                  className="input input-mono"
                  value={draftSpec.thicknessPct}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      thicknessPct: Math.max(4, parseFloat(e.target.value) || 12),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  AoA <span className="field-unit">°</span>
                </label>
                <input
                  type="number"
                  step="0.5"
                  min={-25}
                  max={25}
                  className="input input-mono"
                  value={draftSpec.aoaDeg}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, aoaDeg: Number.isFinite(parseFloat(e.target.value)) ? parseFloat(e.target.value) : 0 })
                  }
                />
              </div>
            </div>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  Chord <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.1"
                  min={0.05}
                  className="input input-mono"
                  value={draftSpec.chord}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, chord: Math.max(0.05, parseFloat(e.target.value) || 1) })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Pos <span className="field-unit">%</span>
                </label>
                <input
                  type="number"
                  step="5"
                  min={10}
                  max={80}
                  className="input input-mono"
                  value={draftSpec.camberPosPct}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      camberPosPct: Math.max(10, Math.min(80, parseFloat(e.target.value) || 40)),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Span <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.05"
                  min={0.02}
                  className="input input-mono"
                  value={draftSpec.span}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, span: Math.max(0.02, parseFloat(e.target.value) || 0.5) })
                  }
                />
              </div>
            </div>
          </>
        )}

        {draftSpec.family === "venturiTube" && (
          <>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  R in <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.05}
                  className="input input-mono"
                  value={draftSpec.inletRadius}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      inletRadius: Math.max(0.05, parseFloat(e.target.value) || 0.5),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  R throat <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.03}
                  className="input input-mono"
                  value={draftSpec.throatRadius}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      throatRadius: Math.max(0.03, parseFloat(e.target.value) || 0.25),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  R out <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.05}
                  className="input input-mono"
                  value={draftSpec.outletRadius}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      outletRadius: Math.max(0.05, parseFloat(e.target.value) || 0.5),
                    })
                  }
                />
              </div>
            </div>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  L conv <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.1"
                  min={0.1}
                  className="input input-mono"
                  value={draftSpec.convergentLen}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      convergentLen: Math.max(0.1, parseFloat(e.target.value) || 1.0),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  L throat <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.1"
                  min={0.05}
                  className="input input-mono"
                  value={draftSpec.throatLen}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      throatLen: Math.max(0.05, parseFloat(e.target.value) || 0.6),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  L div <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.1"
                  min={0.1}
                  className="input input-mono"
                  value={draftSpec.divergentLen}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      divergentLen: Math.max(0.1, parseFloat(e.target.value) || 2.5),
                    })
                  }
                />
              </div>
            </div>
          </>
        )}

        {draftSpec.family === "ahmedDiffuser" && (
          <>
            <div className="field-grid-2">
              <div className="field">
                <label>
                  Rear Slant <span className="field-unit">°</span>
                </label>
                <input
                  type="number"
                  step="1"
                  min={2}
                  max={50}
                  className="input input-mono"
                  value={draftSpec.rearSlantDeg}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      rearSlantDeg: Math.max(2, Math.min(50, parseFloat(e.target.value) || 25)),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Ground Clear. <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.01"
                  min={0.01}
                  className="input input-mono"
                  value={draftSpec.groundClearance}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      groundClearance: Math.max(0.01, parseFloat(e.target.value) || 0.05),
                    })
                  }
                />
              </div>
            </div>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  Length <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.05"
                  min={0.2}
                  className="input input-mono"
                  value={draftSpec.length}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, length: Math.max(0.2, parseFloat(e.target.value) || 1.044) })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Width <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.1}
                  className="input input-mono"
                  value={draftSpec.width}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, width: Math.max(0.1, parseFloat(e.target.value) || 0.389) })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Height <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.08}
                  className="input input-mono"
                  value={draftSpec.height}
                  onChange={(e) =>
                    setDraftSpec({ ...draftSpec, height: Math.max(0.08, parseFloat(e.target.value) || 0.288) })
                  }
                />
              </div>
            </div>
          </>
        )}

        {draftSpec.family === "pinFinArray" && (
          <>
            <div className="field-grid-3">
              <div className="field">
                <label>Fins X</label>
                <input
                  type="number"
                  min={1}
                  max={16}
                  className="input input-mono"
                  value={draftSpec.finCountX}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      finCountX: Math.max(1, Math.min(16, parseInt(e.target.value, 10) || 4)),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>Fins Z</label>
                <input
                  type="number"
                  min={1}
                  max={16}
                  className="input input-mono"
                  value={draftSpec.finCountZ}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      finCountZ: Math.max(1, Math.min(16, parseInt(e.target.value, 10) || 3)),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Fin H <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.005"
                  min={0.005}
                  className="input input-mono"
                  value={draftSpec.finHeight}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      finHeight: Math.max(0.005, parseFloat(e.target.value) || 0.04),
                    })
                  }
                />
              </div>
            </div>
            <div className="field-grid-3">
              <div className="field">
                <label>
                  Base L <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.02}
                  className="input input-mono"
                  value={draftSpec.baseLength}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      baseLength: Math.max(0.02, parseFloat(e.target.value) || 0.24),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Base W <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min={0.02}
                  className="input input-mono"
                  value={draftSpec.baseWidth}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      baseWidth: Math.max(0.02, parseFloat(e.target.value) || 0.16),
                    })
                  }
                />
              </div>
              <div className="field">
                <label>
                  Fin R <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.001"
                  min={0.001}
                  className="input input-mono"
                  value={draftSpec.finRadius}
                  onChange={(e) =>
                    setDraftSpec({
                      ...draftSpec,
                      finRadius: Math.max(0.001, parseFloat(e.target.value) || 0.008),
                    })
                  }
                />
              </div>
            </div>
          </>
        )}

        {draftSpec.family === "cylinder" && (
          <div className="field-grid-2">
            <div className="field">
              <label>
                Radius <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.02"
                min={0.01}
                className="input input-mono"
                value={draftSpec.radius}
                onChange={(e) =>
                  setDraftSpec({ ...draftSpec, radius: Math.max(0.01, parseFloat(e.target.value) || 0.25) })
                }
              />
            </div>
            <div className="field">
              <label>
                Span <span className="field-unit">m</span>
              </label>
              <input
                type="number"
                step="0.05"
                min={0.02}
                className="input input-mono"
                value={draftSpec.span}
                onChange={(e) =>
                  setDraftSpec({ ...draftSpec, span: Math.max(0.02, parseFloat(e.target.value) || 0.58) })
                }
              />
            </div>
          </div>
        )}

        <button
          type="button"
          className="btn btn-primary"
          style={{ width: "100%" }}
          onClick={handleGenerateStl}
          disabled={genBusy}
        >
          <Cpu size={13} />
          {genBusy ? "Generating Binary STL…" : "Generate Watertight STL"}
        </button>

        {genMessage && <div className="notice notice-pass">{genMessage}</div>}
      </div>

      {/* CAD / STL Import & Scaling */}
      <div className="section-block">
        <div className="section-title">
          <span>CAD / STL Surface Geometry</span>
          <span className="mono" style={{ color: "var(--accent-amber)" }}>RUST PARSER</span>
        </div>

        <button type="button" className="btn" style={{ width: "100%" }} onClick={onImportStl}>
          <Upload size={13} />
          Importa Geometria CAD (STL Binary / ASCII)…
        </button>

        {cfdCase.stl ? (
          (() => {
            const stl = cfdCase.stl;
            const scale = stl.scaleFactor && stl.scaleFactor > 0 ? stl.scaleFactor : 1.0;
            const curX = Math.abs(stl.boundsMax[0] - stl.boundsMin[0]);
            const curY = Math.abs(stl.boundsMax[1] - stl.boundsMin[1]);
            const curZ = Math.abs(stl.boundsMax[2] - stl.boundsMin[2]);
            const rawX = curX / scale;
            const rawY = curY / scale;
            const rawZ = curZ / scale;
            const maxDim = Math.max(curX, curY, curZ);
            const looksLikeMm = maxDim > 50;
            const looksTiny = maxDim > 0 && maxDim < 0.002;

            const fmtDim = (v: number) => {
              if (v >= 100) return v.toFixed(1);
              if (v >= 1) return v.toFixed(3);
              return v.toFixed(4);
            };

            const applyScale = (nextFactor: number) => {
              if (!Number.isFinite(nextFactor) || nextFactor <= 0) return;
              if (onScaleStl) {
                onScaleStl(nextFactor, autoFitOnScale);
              } else {
                const ratio = nextFactor / scale;
                onChange((prev) => {
                  if (!prev.stl) return prev;
                  const bMin: [number, number, number] = [
                    prev.stl.boundsMin[0] * ratio,
                    prev.stl.boundsMin[1] * ratio,
                    prev.stl.boundsMin[2] * ratio,
                  ];
                  const bMax: [number, number, number] = [
                    prev.stl.boundsMax[0] * ratio,
                    prev.stl.boundsMax[1] * ratio,
                    prev.stl.boundsMax[2] * ratio,
                  ];
                  return {
                    ...prev,
                    stl: {
                      ...prev.stl,
                      scaleFactor: nextFactor,
                      boundsMin: bMin,
                      boundsMax: bMax,
                      surfaceArea: prev.stl.surfaceArea * ratio * ratio,
                      volumeEst: prev.stl.volumeEst * ratio * ratio * ratio,
                    },
                  };
                });
              }
            };

            const applyTargetRealSize = () => {
              const desiredM = parseFloat(targetRealMeters);
              const rawAxis = targetAxis === 0 ? rawX : targetAxis === 1 ? rawY : rawZ;
              if (!Number.isFinite(desiredM) || desiredM <= 0 || rawAxis <= 1e-9) return;
              applyScale(desiredM / rawAxis);
            };

            return (
              <>
                {/* Scale Anomaly Detection Banner */}
                {looksLikeMm && (
                  <div className="notice notice-warn" style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                    <div style={{ display: "flex", alignItems: "center", gap: 6, fontWeight: 600 }}>
                      <AlertTriangle size={13} />
                      Scala CAD sospetta: dimensione max = {maxDim.toFixed(1)} m
                    </div>
                    <div style={{ fontSize: 11 }}>
                      Molti CAD esportano file STL in <strong>millimetri (mm)</strong> mentre OpenFOAM lavora in{" "}
                      <strong>metri (m)</strong>.
                    </div>
                    <button
                      type="button"
                      className="btn btn-primary btn-xs"
                      onClick={() => applyScale(0.001)}
                    >
                      <Ruler size={11} />
                      Converti subito mm → m (×0.001)
                    </button>
                  </div>
                )}

                {looksTiny && (
                  <div className="notice notice-warn" style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <AlertTriangle size={13} />
                    <span>
                      Oggetto molto piccolo ({(maxDim * 1000).toFixed(3)} mm). Verifica il fattore di scala qui sotto.
                    </span>
                  </div>
                )}

                {/* STL Telemetry & Dimensions Readout */}
                <div className="readout-box">
                  <div className="readout-row">
                    <span>File</span>
                    <span style={{ color: "var(--accent-amber-bright)" }}>{stl.filename}</span>
                  </div>
                  <div className="readout-row">
                    <span>Triangoli / Formato</span>
                    <span>
                      {stl.triangleCount.toLocaleString()} ({stl.isBinary ? "Binary" : "ASCII"})
                    </span>
                  </div>
                  <div className="readout-row">
                    <span>Dimensioni CAD originali</span>
                    <span style={{ color: "var(--text-muted)" }}>
                      {fmtDim(rawX)} × {fmtDim(rawY)} × {fmtDim(rawZ)} u
                    </span>
                  </div>
                  <div className="readout-row">
                    <span>Dimensioni Reali CFD (m)</span>
                    <span style={{ color: "var(--cfd-inlet)", fontWeight: 600 }}>
                      {fmtDim(curX)} × {fmtDim(curY)} × {fmtDim(curZ)} m
                    </span>
                  </div>
                  <div className="readout-row">
                    <span>Area Superficie / Volume</span>
                    <span>
                      {stl.surfaceArea.toFixed(4)} m² · {stl.volumeEst.toFixed(5)} m³
                    </span>
                  </div>
                </div>

                {/* Dedicated CAD Scaling Card */}
                <div className="cad-scale-card">
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: 6,
                    }}
                  >
                    <span
                      style={{
                        display: "inline-flex",
                        alignItems: "center",
                        gap: 5,
                        fontSize: 10.5,
                        fontWeight: 600,
                        textTransform: "uppercase",
                        letterSpacing: "0.06em",
                        color: "var(--accent-amber-bright)",
                      }}
                    >
                      <Ruler size={12} />
                      Scaling CAD → Metri (SI)
                    </span>
                    <span className="pill pill-cyan" style={{ fontSize: 9.5 }}>
                      ×{Number(scale.toPrecision(5))}
                    </span>
                  </div>

                  {/* 1-click CAD unit conversion presets */}
                  <div className="unit-preset-grid">
                    {CAD_UNIT_PRESETS.map((u) => {
                      const active = Math.abs(scale - u.factor) / Math.max(1e-9, u.factor) < 0.005;
                      return (
                        <button
                          key={u.id}
                          type="button"
                          className={`granularity-btn ${active ? "active" : ""}`}
                          onClick={() => applyScale(u.factor)}
                          title={`Scala da ${u.label} (${u.desc})`}
                        >
                          <span className="granularity-name">{u.label}</span>
                          <span className="granularity-sub mono">{u.desc}</span>
                        </button>
                      );
                    })}
                  </div>

                  {/* Set exact real-world target dimension */}
                  <div className="field">
                    <label>
                      <span>Imposta Dimensione Reale Oggetto</span>
                      <span className="field-unit">metri (m)</span>
                    </label>
                    <div style={{ display: "flex", gap: 5 }}>
                      <select
                        className="select"
                        style={{ width: 92, height: 27, fontSize: 11 }}
                        value={targetAxis}
                        onChange={(e) => setTargetAxis(Number(e.target.value) as 0 | 1 | 2)}
                        aria-label="Asse di riferimento per dimensione reale"
                      >
                        <option value={0}>Lung. X</option>
                        <option value={1}>Alt. Y</option>
                        <option value={2}>Largh. Z</option>
                      </select>
                      <input
                        type="number"
                        step="0.01"
                        min="0.0001"
                        className="input input-mono"
                        style={{ flex: 1, height: 27, fontSize: 11.5 }}
                        value={targetRealMeters}
                        onChange={(e) => setTargetRealMeters(e.target.value)}
                        placeholder="es. 1.044"
                      />
                      <button
                        type="button"
                        className="btn btn-primary btn-xs"
                        style={{ height: 27 }}
                        onClick={applyTargetRealSize}
                      >
                        Applica
                      </button>
                    </div>
                  </div>

                  {/* Custom Scale Factor */}
                  <div className="field">
                    <label>
                      <span>Fattore di Scala Personalizzato</span>
                      {Math.abs(scale - 1) > 1e-6 && (
                        <button
                          type="button"
                          className="btn btn-xs btn-subtle"
                          style={{ height: 18, padding: "0 5px", fontSize: 9.5 }}
                          onClick={() => applyScale(1.0)}
                          title="Ripristina scala originale 1:1"
                        >
                          <RotateCcw size={9} /> Reset 1:1
                        </button>
                      )}
                    </label>
                    <div style={{ display: "flex", gap: 5 }}>
                      <input
                        type="number"
                        step="0.001"
                        min="0.000001"
                        className="input input-mono"
                        style={{ flex: 1, height: 27, fontSize: 11.5 }}
                        value={customScaleInput}
                        onChange={(e) => setCustomScaleInput(e.target.value)}
                      />
                      <button
                        type="button"
                        className="btn btn-xs"
                        style={{ height: 27 }}
                        onClick={() => applyScale(parseFloat(customScaleInput))}
                      >
                        Scala ×
                      </button>
                    </div>
                  </div>

                  <label className="toggle-row" style={{ fontSize: 11 }}>
                    <span>Adatta automaticamente dominio e camera</span>
                    <input
                      type="checkbox"
                      checked={autoFitOnScale}
                      onChange={(e) => setAutoFitOnScale(e.target.checked)}
                    />
                  </label>

                  <button
                    type="button"
                    className="btn btn-xs"
                    style={{ width: "100%" }}
                    onClick={autoFitStlDomain}
                  >
                    <Maximize2 size={11} />
                    Adatta Dominio Galleria del Vento (3L / 6L Scia)
                  </button>
                </div>
              </>
            );
          })()
        ) : (
          <div className="notice">
            Genera un corpo parametrico sopra, scegli un preset di riferimento, oppure importa un file CAD{" "}
            <code className="mono">.stl</code> per verificare e correggere la scala in metri e configurare{" "}
            <code className="mono">snappyHexMeshDict</code>.
          </div>
        )}
      </div>

      {/* Built-in Reference Geometries */}
      <div className="section-block">
        <div className="section-title">
          <span>Parametric Reference Geometry</span>
          <Layers size={12} />
        </div>
        <div style={{ display: "flex", flexDirection: "column", gap: 5 }}>
          {PRESET_SHAPES.map((g) => {
            const active = cfdCase.geometryKind === g.id;
            return (
              <button
                key={g.id}
                type="button"
                className={`patch-card ${active ? "selected" : ""}`}
                style={{ textAlign: "left" }}
                onClick={() =>
                  onChange((prev) => ({
                    ...prev,
                    geometryKind: g.id,
                    mesh: {
                      ...prev.mesh,
                      snappyEnabled: g.id !== "none",
                    },
                  }))
                }
              >
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <span style={{ fontWeight: 600, fontSize: 12, color: "var(--text-primary)" }}>
                    {g.label}
                  </span>
                  {active && <span className="pill pill-warn">ACTIVE</span>}
                </div>
                <span style={{ fontSize: 11, color: "var(--text-muted)" }}>{g.desc}</span>
              </button>
            );
          })}
        </div>
      </div>

      {/* Computational Domain Box */}
      <div className="section-block">
        <div className="section-title">
          <span>Computational Domain (blockMesh)</span>
          <Box size={12} />
        </div>

        <div className="field-grid-2">
          <div className="field">
            <label>
              X Min <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.25"
              className="input input-mono"
              value={d.xMin}
              onChange={(e) => updateDomain("xMin", parseFloat(e.target.value))}
            />
          </div>
          <div className="field">
            <label>
              X Max <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.25"
              className="input input-mono"
              value={d.xMax}
              onChange={(e) => updateDomain("xMax", parseFloat(e.target.value))}
            />
          </div>
          <div className="field">
            <label>
              Y Min <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.25"
              className="input input-mono"
              value={d.yMin}
              onChange={(e) => updateDomain("yMin", parseFloat(e.target.value))}
            />
          </div>
          <div className="field">
            <label>
              Y Max <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.25"
              className="input input-mono"
              value={d.yMax}
              onChange={(e) => updateDomain("yMax", parseFloat(e.target.value))}
            />
          </div>
          <div className="field">
            <label>
              Z Min <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.1"
              className="input input-mono"
              value={d.zMin}
              onChange={(e) => updateDomain("zMin", parseFloat(e.target.value))}
            />
          </div>
          <div className="field">
            <label>
              Z Max <span className="field-unit">m</span>
            </label>
            <input
              type="number"
              step="0.1"
              className="input input-mono"
              value={d.zMax}
              onChange={(e) => updateDomain("zMax", parseFloat(e.target.value))}
            />
          </div>
        </div>

        <div className="readout-box">
          <div className="readout-row">
            <span>Domain Dimensions (Lx × Ly × Lz)</span>
            <span>
              {lx.toFixed(2)} × {ly.toFixed(2)} × {lz.toFixed(2)} m
            </span>
          </div>
          <div className="readout-row">
            <span>Domain Volume</span>
            <span>{(lx * ly * lz).toFixed(3)} m³</span>
          </div>
        </div>
      </div>
    </>
  );
};
