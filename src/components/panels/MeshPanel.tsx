import React, { useState } from "react";
import { Sparkles, Grid3x3, Layers, Plus, Trash2, Box } from "lucide-react";
import { CfdCase, RefinementBox, TelemetrySnapshot } from "../../types/cfd";
import { computeYPlus } from "../../api/bridge";

interface MeshPanelProps {
  cfdCase: CfdCase;
  telemetry: TelemetrySnapshot | null;
  onChange: (updater: (prev: CfdCase) => CfdCase) => void;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

function round3(v: number): number {
  return Number(v.toFixed(3));
}

export const MeshPanel: React.FC<MeshPanelProps> = ({ cfdCase, telemetry, onChange }) => {
  const m = cfdCase.mesh;
  const [syncMessage, setSyncMessage] = useState<string | null>(null);

  const updateMesh = <K extends keyof typeof m>(key: K, val: (typeof m)[K]) => {
    onChange((prev) => ({
      ...prev,
      mesh: { ...prev.mesh, [key]: val },
    }));
  };

  const addWakeRefinementBox = () => {
    const d = cfdCase.domain;
    const lx = Math.max(0.1, d.xMax - d.xMin);
    const ly = Math.max(0.1, d.yMax - d.yMin);
    const lz = Math.max(0.05, d.zMax - d.zMin);
    const refL = Math.max(0.05, cfdCase.physics.referenceLength);
    const existing = m.refinementBoxes;

    let idx = existing.length + 1;
    let name = `wake_zone_${idx}`;
    while (existing.some((b) => b.name === name)) {
      idx += 1;
      name = `wake_zone_${idx}`;
    }

    const isSecondary = existing.length > 0;
    const bodyXMin = cfdCase.stl ? cfdCase.stl.boundsMin[0] : -0.25 * refL;
    const bodyXMax = cfdCase.stl ? cfdCase.stl.boundsMax[0] : 0.75 * refL;
    const bodyYMid = cfdCase.stl
      ? 0.5 * (cfdCase.stl.boundsMin[1] + cfdCase.stl.boundsMax[1])
      : 0.5 * (d.yMin + d.yMax);

    const x0 = clamp(
      isSecondary ? bodyXMax + 0.5 * refL : bodyXMin - 0.15 * refL,
      d.xMin + 0.04 * lx,
      d.xMax - 0.12 * lx
    );
    const x1 = clamp(
      isSecondary ? bodyXMax + 3.2 * refL : bodyXMax + 2.0 * refL,
      x0 + 0.08 * lx,
      d.xMax - 0.05 * lx
    );
    const halfY = isSecondary ? 0.3 * ly : 0.22 * ly;
    const y0 = clamp(bodyYMid - halfY, d.yMin + 0.04 * ly, d.yMax - 0.12 * ly);
    const y1 = clamp(bodyYMid + halfY, y0 + 0.08 * ly, d.yMax - 0.04 * ly);
    const z0 = clamp(d.zMin + 0.08 * lz, d.zMin, d.zMax);
    const z1 = clamp(d.zMax - 0.08 * lz, z0 + 0.01, d.zMax);

    const newBox: RefinementBox = {
      name,
      min: [round3(x0), round3(y0), round3(z0)],
      max: [round3(x1), round3(y1), round3(z1)],
      level: isSecondary ? 2 : 3,
      reason: isSecondary
        ? "Far-wake vortex & pressure recovery refinement"
        : "Near-wake shear-layer & separation gradient refinement",
    };

    onChange((prev) => ({
      ...prev,
      mesh: {
        ...prev.mesh,
        snappyEnabled: true,
        refinementBoxes: [...prev.mesh.refinementBoxes, newBox],
      },
    }));
  };

  const removeRefinementBox = (name: string) => {
    onChange((prev) => ({
      ...prev,
      mesh: {
        ...prev.mesh,
        refinementBoxes: prev.mesh.refinementBoxes.filter((b) => b.name !== name),
      },
    }));
  };

  const updateRefinementBoxLevel = (name: string, level: number) => {
    const clamped = clamp(Math.round(level), 1, 5);
    onChange((prev) => ({
      ...prev,
      mesh: {
        ...prev.mesh,
        refinementBoxes: prev.mesh.refinementBoxes.map((b) =>
          b.name === name ? { ...b, level: clamped } : b
        ),
      },
    }));
  };

  const autoSyncBoundaryLayer = async () => {
    try {
      const res = await computeYPlus({
        velocity: Math.max(0.01, cfdCase.physics.referenceVelocity),
        density: cfdCase.physics.fluid.density,
        dynamicViscosity: cfdCase.physics.fluid.dynamicViscosity,
        referenceLength: cfdCase.physics.referenceLength,
        targetYPlus: m.targetYPlus,
        expansionRatio: m.expansionRatio,
      });
      onChange((prev) => ({
        ...prev,
        mesh: {
          ...prev.mesh,
          firstLayerThickness: Number(res.firstCellHeight.toPrecision(4)),
          inflationLayers: res.recommendedLayers,
        },
      }));
      setSyncMessage(
        `Sincronizzato: Δy₁ = ${(res.firstCellHeight * 1000).toFixed(4)} mm, ${res.recommendedLayers} strati (δ₉₉ = ${(res.boundaryLayerDelta99 * 1000).toFixed(1)} mm)`
      );
    } catch (e) {
      setSyncMessage(String(e));
    }
  };

  const lx = Math.max(1e-4, Math.abs(cfdCase.domain.xMax - cfdCase.domain.xMin));
  const ly = Math.max(1e-4, Math.abs(cfdCase.domain.yMax - cfdCase.domain.yMin));
  const lz = Math.max(1e-4, Math.abs(cfdCase.domain.zMax - cfdCase.domain.zMin));
  const is2DCase = cfdCase.boundaries.some((b) => b.kind === "empty2D");

  const baseCells = m.cellsX * m.cellsY * m.cellsZ;
  const dx = lx / Math.max(1, m.cellsX);
  const dy = ly / Math.max(1, m.cellsY);
  const dz = lz / Math.max(1, m.cellsZ);
  const baseCellM = is2DCase ? Math.sqrt(dx * dy) : Math.cbrt(dx * dy * dz);
  const baseCellMm = baseCellM * 1000;
  const aspectRatio = Math.max(dx, dy, dz) / Math.max(1e-6, Math.min(dx, dy, dz));

  const maxDom = Math.max(lx, ly, lz);
  const minSliderMm = Math.max(0.5, Number(((maxDom / 260) * 1000).toFixed(1)));
  const maxSliderMm = Math.max(minSliderMm + 1, Number(((maxDom / 18) * 1000).toFixed(1)));

  const applyVolumeCellSizeMm = (targetMm: number) => {
    const h = Math.max(1e-4, targetMm / 1000);
    const nx = clamp(Math.round(lx / h), 4, 600);
    const ny = clamp(Math.round(ly / h), 4, 600);
    const nz = is2DCase ? 1 : clamp(Math.round(lz / h), 2, 400);
    onChange((prev) => ({
      ...prev,
      mesh: { ...prev.mesh, cellsX: nx, cellsY: ny, cellsZ: nz },
    }));
  };

  const applyVolumePreset = (divisionsAlongMax: number) => {
    const targetH = maxDom / divisionsAlongMax;
    applyVolumeCellSizeMm(targetH * 1000);
  };

  const applySurfacePreset = (rMin: number, rMax: number) => {
    onChange((prev) => ({
      ...prev,
      mesh: {
        ...prev.mesh,
        snappyEnabled: true,
        refinementMin: rMin,
        refinementMax: rMax,
      },
    }));
  };

  const formatMm = (mm: number): string => {
    if (!Number.isFinite(mm)) return "—";
    if (mm >= 100) return `${mm.toFixed(0)} mm`;
    if (mm >= 10) return `${mm.toFixed(1)} mm`;
    if (mm >= 1) return `${mm.toFixed(2)} mm`;
    return `${mm.toFixed(3)} mm`;
  };

  const surfMinMm = baseCellMm / Math.pow(2, m.refinementMin);
  const surfMaxMm = baseCellMm / Math.pow(2, m.refinementMax);

  const volumePresets = [
    { id: "coarse", label: "Grossolana", divs: 40, desc: "Bozza rapida" },
    { id: "medium", label: "Media", divs: 80, desc: "Standard" },
    { id: "fine", label: "Fine", divs: 130, desc: "Produzione" },
    { id: "ultra", label: "Ultra", divs: 190, desc: "Alta risol." },
  ];

  const surfacePresets = [
    { id: "draft", label: "Bozza", min: 1, max: 2 },
    { id: "balanced", label: "Bilanciata", min: 2, max: 3 },
    { id: "fine", label: "Fine", min: 3, max: 4 },
    { id: "sharp", label: "Dettaglio", min: 4, max: 5 },
  ];

  const currentDivs = Math.round(maxDom / Math.max(1e-5, baseCellM));

  return (
    <>
      {/* 1. VOLUME MESH GRANULARITY (blockMesh) */}
      <div className="section-block">
        <div className="section-title">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
            <Grid3x3 size={12} />
            Granularità Mesh di Volume (blockMesh)
          </span>
          <span className="pill pill-cyan" style={{ fontSize: 9.5 }}>
            Δ ≈ {formatMm(baseCellMm)}
          </span>
        </div>

        {/* Volume Granularity Quick Presets */}
        <div className="granularity-grid">
          {volumePresets.map((vp) => {
            const isNear = Math.abs(currentDivs - vp.divs) <= Math.max(12, vp.divs * 0.22);
            return (
              <button
                key={vp.id}
                type="button"
                className={`granularity-btn ${isNear ? "active" : ""}`}
                onClick={() => applyVolumePreset(vp.divs)}
                title={`Imposta cella di volume base ≈ ${formatMm((maxDom / vp.divs) * 1000)}`}
              >
                <span className="granularity-name">{vp.label}</span>
                <span className="granularity-sub mono">{formatMm((maxDom / vp.divs) * 1000)}</span>
              </button>
            );
          })}
        </div>

        {/* Target Volumetric Cell Size Slider + Input */}
        <div className="field">
          <label>
            <span>Dimensione Target Cella di Volume (Δ_vol)</span>
            <span className="mono" style={{ color: "var(--accent-amber-bright)", fontSize: 11 }}>
              {formatMm(baseCellMm)}
            </span>
          </label>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <input
              type="range"
              style={{ flex: 1 }}
              min={minSliderMm}
              max={maxSliderMm}
              step={Math.max(0.5, Number(((maxSliderMm - minSliderMm) / 80).toFixed(1)))}
              value={clamp(Number(baseCellMm.toFixed(1)), minSliderMm, maxSliderMm)}
              onChange={(e) => applyVolumeCellSizeMm(parseFloat(e.target.value))}
            />
            <input
              type="number"
              min={0.1}
              step={1}
              className="input input-mono"
              style={{ width: 74, height: 26, fontSize: 11 }}
              value={Number(baseCellMm.toFixed(1))}
              onChange={(e) => {
                const v = parseFloat(e.target.value);
                if (Number.isFinite(v) && v >= 0.1) applyVolumeCellSizeMm(v);
              }}
              aria-label="Target volume cell size in mm"
            />
            <span className="field-unit">mm</span>
          </div>
        </div>

        <div className="field-grid-3">
          <div className="field">
            <label>
              Nx <span className="field-unit">{formatMm(dx * 1000)}</span>
            </label>
            <input
              type="number"
              min={2}
              max={600}
              className="input input-mono"
              value={m.cellsX}
              onChange={(e) => updateMesh("cellsX", Math.max(2, parseInt(e.target.value, 10) || 2))}
            />
          </div>
          <div className="field">
            <label>
              Ny <span className="field-unit">{formatMm(dy * 1000)}</span>
            </label>
            <input
              type="number"
              min={2}
              max={600}
              className="input input-mono"
              value={m.cellsY}
              onChange={(e) => updateMesh("cellsY", Math.max(2, parseInt(e.target.value, 10) || 2))}
            />
          </div>
          <div className="field">
            <label>
              Nz <span className="field-unit">{formatMm(dz * 1000)}</span>
            </label>
            <input
              type="number"
              min={1}
              max={400}
              className="input input-mono"
              value={m.cellsZ}
              onChange={(e) => updateMesh("cellsZ", Math.max(1, parseInt(e.target.value, 10) || 1))}
            />
          </div>
        </div>

        <div className="field">
          <label>
            Grading Y (infittimento verticale) <span className="field-unit">{m.gradingY.toFixed(2)}</span>
          </label>
          <input
            type="range"
            min="0.2"
            max="5.0"
            step="0.05"
            value={m.gradingY}
            onChange={(e) => updateMesh("gradingY", parseFloat(e.target.value))}
          />
        </div>

        <div className="readout-box">
          <div className="readout-row">
            <span>Celle di Volume Base (blockMesh)</span>
            <span>{baseCells.toLocaleString()}</span>
          </div>
          <div className="readout-row">
            <span>Passo Base (Δx × Δy × Δz)</span>
            <span>
              {formatMm(dx * 1000)} × {formatMm(dy * 1000)} × {formatMm(dz * 1000)}
            </span>
          </div>
          <div className="readout-row">
            <span>Aspect Ratio Base</span>
            <span
              style={{
                color:
                  aspectRatio > 6
                    ? "var(--cfd-wall)"
                    : aspectRatio > 3
                      ? "var(--accent-amber-bright)"
                      : "var(--cfd-outlet)",
              }}
            >
              {aspectRatio.toFixed(2)} {aspectRatio > 6 ? "(Alto)" : aspectRatio > 3 ? "(Moderato)" : "(Ottimale)"}
            </span>
          </div>
        </div>
      </div>

      {/* 2. SURFACE MESH GRANULARITY & BOUNDARY LAYER (snappyHexMesh) */}
      <div className="section-block">
        <div className="section-title">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
            <Layers size={12} />
            Granularità Mesh di Superficie (snappyHexMesh)
          </span>
          {m.snappyEnabled && (
            <span className="pill pill-warn" style={{ fontSize: 9.5 }}>
              {formatMm(surfMaxMm)}
            </span>
          )}
        </div>

        <label className="toggle-row">
          <span>Attiva snappyHexMesh + Strato Limite</span>
          <input
            type="checkbox"
            checked={m.snappyEnabled}
            onChange={(e) => updateMesh("snappyEnabled", e.target.checked)}
          />
        </label>

        {m.snappyEnabled && (
          <>
            {/* Surface Granularity Quick Presets */}
            <div className="granularity-grid">
              {surfacePresets.map((sp) => {
                const active = m.refinementMin === sp.min && m.refinementMax === sp.max;
                const spMaxMm = baseCellMm / Math.pow(2, sp.max);
                return (
                  <button
                    key={sp.id}
                    type="button"
                    className={`granularity-btn ${active ? "active" : ""}`}
                    onClick={() => applySurfacePreset(sp.min, sp.max)}
                    title={`Superficie L${sp.min}–L${sp.max} (min cella ≈ ${formatMm(spMaxMm)})`}
                  >
                    <span className="granularity-name">
                      {sp.label} <span className="mono">L{sp.min}-{sp.max}</span>
                    </span>
                    <span className="granularity-sub mono">{formatMm(spMaxMm)}</span>
                  </button>
                );
              })}
            </div>

            {/* Min / Max Surface Refinement Level with Live Physical Cell Size */}
            <div className="field">
              <label>
                <span>Livello Min Superficie (zone piane)</span>
                <span className="mono" style={{ color: "var(--cfd-inlet)", fontSize: 10.5 }}>
                  L{m.refinementMin} → {formatMm(surfMinMm)} (1/{Math.pow(2, m.refinementMin)}×)
                </span>
              </label>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <input
                  type="range"
                  style={{ flex: 1 }}
                  min={0}
                  max={6}
                  step={1}
                  value={m.refinementMin}
                  onChange={(e) => {
                    const nextMin = parseInt(e.target.value, 10);
                    onChange((prev) => ({
                      ...prev,
                      mesh: {
                        ...prev.mesh,
                        refinementMin: nextMin,
                        refinementMax: Math.max(prev.mesh.refinementMax, nextMin),
                      },
                    }));
                  }}
                />
                <input
                  type="number"
                  min={0}
                  max={6}
                  className="input input-mono"
                  style={{ width: 56, height: 26, fontSize: 11 }}
                  value={m.refinementMin}
                  onChange={(e) => {
                    const nextMin = clamp(parseInt(e.target.value, 10) || 0, 0, 6);
                    onChange((prev) => ({
                      ...prev,
                      mesh: {
                        ...prev.mesh,
                        refinementMin: nextMin,
                        refinementMax: Math.max(prev.mesh.refinementMax, nextMin),
                      },
                    }));
                  }}
                />
              </div>
            </div>

            <div className="field">
              <label>
                <span>Livello Max Superficie (spigoli & curvature)</span>
                <span className="mono" style={{ color: "var(--accent-amber-bright)", fontSize: 10.5 }}>
                  L{m.refinementMax} → {formatMm(surfMaxMm)} (1/{Math.pow(2, m.refinementMax)}×)
                </span>
              </label>
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <input
                  type="range"
                  style={{ flex: 1 }}
                  min={0}
                  max={6}
                  step={1}
                  value={m.refinementMax}
                  onChange={(e) => {
                    const nextMax = parseInt(e.target.value, 10);
                    onChange((prev) => ({
                      ...prev,
                      mesh: {
                        ...prev.mesh,
                        refinementMax: nextMax,
                        refinementMin: Math.min(prev.mesh.refinementMin, nextMax),
                      },
                    }));
                  }}
                />
                <input
                  type="number"
                  min={0}
                  max={6}
                  className="input input-mono"
                  style={{ width: 56, height: 26, fontSize: 11 }}
                  value={m.refinementMax}
                  onChange={(e) => {
                    const nextMax = clamp(parseInt(e.target.value, 10) || 0, 0, 6);
                    onChange((prev) => ({
                      ...prev,
                      mesh: {
                        ...prev.mesh,
                        refinementMax: nextMax,
                        refinementMin: Math.min(prev.mesh.refinementMin, nextMax),
                      },
                    }));
                  }}
                />
              </div>
            </div>

            {/* Volume vs Surface Granularity Comparison Summary */}
            <div className="readout-box">
              <div className="readout-row">
                <span>Volume Base → Superficie (Min/Max)</span>
                <span>
                  {formatMm(baseCellMm)} → {formatMm(surfMinMm)} /{" "}
                  <strong style={{ color: "var(--accent-amber-bright)" }}>{formatMm(surfMaxMm)}</strong>
                </span>
              </div>
              <div className="readout-row">
                <span>Prima Cella Parete (Δy₁)</span>
                <span>
                  {formatMm(m.firstLayerThickness * 1000)} ({m.inflationLayers} strati)
                </span>
              </div>
            </div>

            <div className="field-grid-2">
              <div className="field">
                <label>
                  Target y+ <span className="field-unit">adim.</span>
                </label>
                <input
                  type="number"
                  step="0.5"
                  min="0.2"
                  max="400"
                  className="input input-mono"
                  value={m.targetYPlus}
                  onChange={(e) => updateMesh("targetYPlus", Math.max(0.1, parseFloat(e.target.value) || 1))}
                />
              </div>
              <div className="field">
                <label>
                  Growth Ratio <span className="field-unit">r</span>
                </label>
                <input
                  type="number"
                  step="0.02"
                  min="1.05"
                  max="1.5"
                  className="input input-mono"
                  value={m.expansionRatio}
                  onChange={(e) =>
                    updateMesh("expansionRatio", Math.max(1.05, parseFloat(e.target.value) || 1.2))
                  }
                />
              </div>
            </div>

            <div className="field-grid-2">
              <div className="field">
                <label>
                  First Layer Δy₁ <span className="field-unit">m</span>
                </label>
                <input
                  type="number"
                  step="0.00005"
                  className="input input-mono"
                  value={m.firstLayerThickness}
                  onChange={(e) =>
                    updateMesh("firstLayerThickness", Math.max(1e-7, parseFloat(e.target.value) || 1e-4))
                  }
                />
              </div>
              <div className="field">
                <label>Prism Layers N</label>
                <input
                  type="number"
                  min={0}
                  max={40}
                  className="input input-mono"
                  value={m.inflationLayers}
                  onChange={(e) =>
                    updateMesh("inflationLayers", Math.max(0, parseInt(e.target.value, 10) || 0))
                  }
                />
              </div>
            </div>

            <button
              type="button"
              className="btn btn-primary"
              style={{ width: "100%" }}
              onClick={autoSyncBoundaryLayer}
            >
              <Sparkles size={13} />
              Auto-Size Δy₁ & Layers from Physics (Rust y+)
            </button>

            {syncMessage && <div className="notice notice-pass">{syncMessage}</div>}
          </>
        )}
      </div>

      {/* 3. VOLUMETRIC 3D REFINEMENT REGIONS (snappyHexMeshDict) */}
      <div className="section-block">
        <div className="section-title">
          <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
            <Box size={12} />
            Zone di Infittimento Volumetrico 3D
          </span>
          <span className="mono" style={{ fontSize: 10, color: "var(--cfd-inlet)" }}>
            {m.refinementBoxes.length} ZON{m.refinementBoxes.length === 1 ? "A" : "E"}
          </span>
        </div>

        {m.refinementBoxes.length === 0 ? (
          <div className="notice">
            Nessuna zona volumetrica configurata. Aggiungi una box di scia qui sotto per infittire il volume dietro
            l&apos;ostacolo senza appesantire l&apos;intero dominio.
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {m.refinementBoxes.map((box) => {
              const boxCellMm = baseCellMm / Math.pow(2, box.level);
              return (
                <div
                  key={box.name}
                  className="patch-card"
                  style={{ cursor: "default", gap: 5 }}
                >
                  <div
                    style={{
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "space-between",
                      gap: 6,
                    }}
                  >
                    <div style={{ display: "flex", alignItems: "center", gap: 6, minWidth: 0 }}>
                      <span
                        className="mono"
                        style={{
                          fontWeight: 600,
                          fontSize: 11.5,
                          color: "var(--text-primary)",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {box.name}
                      </span>
                      <select
                        className="select mono"
                        style={{ width: 66, height: 22, fontSize: 10, padding: "0 4px" }}
                        value={box.level}
                        onChange={(e) => updateRefinementBoxLevel(box.name, parseInt(e.target.value, 10))}
                        aria-label={`Refinement level for ${box.name}`}
                      >
                        {[1, 2, 3, 4, 5].map((lv) => (
                          <option key={lv} value={lv}>
                            L{lv}
                          </option>
                        ))}
                      </select>
                      <span className="mono" style={{ fontSize: 10, color: "var(--accent-amber-bright)", flexShrink: 0 }}>
                        ≈ {formatMm(boxCellMm)}
                      </span>
                    </div>
                    <button
                      type="button"
                      className="btn btn-xs btn-danger"
                      style={{ padding: "0 6px" }}
                      onClick={() => removeRefinementBox(box.name)}
                      title={`Elimina zona ${box.name}`}
                    >
                      <Trash2 size={11} />
                    </button>
                  </div>

                  <div
                    className="mono"
                    style={{
                      fontSize: 10,
                      color: "var(--text-secondary)",
                      background: "var(--bg-canvas)",
                      border: "1px solid var(--border-subtle)",
                      borderRadius: "var(--radius-xs)",
                      padding: "4px 6px",
                    }}
                  >
                    [{box.min[0].toFixed(2)}, {box.min[1].toFixed(2)}, {box.min[2].toFixed(2)}] → [
                    {box.max[0].toFixed(2)}, {box.max[1].toFixed(2)}, {box.max[2].toFixed(2)}] m
                  </div>

                  {box.reason && (
                    <div style={{ fontSize: 10.5, color: "var(--text-muted)", lineHeight: 1.35 }}>
                      {box.reason}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        )}

        <button
          type="button"
          className="btn"
          style={{ width: "100%" }}
          onClick={addWakeRefinementBox}
        >
          <Plus size={13} />
          Aggiungi Zona di Infittimento Scia (Wake Box)
        </button>
      </div>

      {/* Resource Forecast */}
      {telemetry && (
        <div className="section-block">
          <div className="section-title">
            <span>Mesh Budget & Hardware Forecast</span>
          </div>
          <div className="readout-box">
            <div className="readout-row">
              <span>Est. Final Cell Count</span>
              <span style={{ color: "var(--accent-amber-bright)" }}>
                {telemetry.estimatedCells.toLocaleString()}
              </span>
            </div>
            <div className="readout-row">
              <span>Min Refined Δx</span>
              <span>{(telemetry.minDx * 1000).toFixed(3)} mm</span>
            </div>
            <div className="readout-row">
              <span>Estimated RAM Footprint</span>
              <span>
                {telemetry.estimatedRamMb >= 1024
                  ? `${(telemetry.estimatedRamMb / 1024).toFixed(2)} GB`
                  : `${telemetry.estimatedRamMb.toFixed(0)} MB`}
              </span>
            </div>
            <div className="readout-row">
              <span>Recommended MPI Ranks</span>
              <span>{telemetry.recommendedCores} cores (~50k cells/rank)</span>
            </div>
          </div>
        </div>
      )}
    </>
  );
};
