import React from "react";
import {
  FolderOpen,
  Download,
  Play,
  Square,
  Cpu,
  CheckCircle2,
  Container,
  Sparkles,
  Compass,
} from "lucide-react";
import { CfdCase, EnvironmentStatus, OpenFoamFlavor } from "../types/cfd";
import { CASE_PRESETS } from "../data/presets";

interface TitleBarProps {
  cfdCase: CfdCase;
  env: EnvironmentStatus | null;
  isRunning: boolean;
  guidedMode: boolean;
  onToggleGuidedMode: () => void;
  onCaseNameChange: (name: string) => void;
  onFlavorChange: (flavor: OpenFoamFlavor) => void;
  onSelectPreset: (presetId: string) => void;
  onImportCase: () => void;
  onExportCase: () => void;
  onQuickRun: () => void;
  onStopRun: () => void;
}

export const TitleBar: React.FC<TitleBarProps> = ({
  cfdCase,
  env,
  isRunning,
  guidedMode,
  onToggleGuidedMode,
  onCaseNameChange,
  onFlavorChange,
  onSelectPreset,
  onImportCase,
  onExportCase,
  onQuickRun,
  onStopRun,
}) => {
  const envBadge = (() => {
    if (!env) return { label: "Probing...", cls: "pill", icon: Cpu };
    if (env.localBinary) return { label: `Native OpenFOAM`, cls: "pill pill-pass", icon: CheckCircle2 };
    if (env.dockerRunning) return { label: "Docker Engine Ready", cls: "pill pill-cyan", icon: Container };
    return { label: "Benchmark / Export Mode", cls: "pill pill-warn", icon: Sparkles };
  })();
  const EnvIcon = envBadge.icon;

  return (
    <header className="titlebar">
      {/* Brand + Case Identity */}
      <div style={{ display: "flex", alignItems: "center", gap: 10, minWidth: 0 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <svg width="26" height="26" viewBox="0 0 32 32" fill="none" aria-hidden="true">
            <rect width="32" height="32" rx="7" fill="#131A26" stroke="#2D3B52" />
            <path
              d="M5 9C10 9 12 13 16 13C20 13 22 9 27 9"
              stroke="#F59E0B"
              strokeWidth="2.2"
              strokeLinecap="round"
            />
            <path
              d="M5 23C10 23 12 19 16 19C20 19 22 23 27 23"
              stroke="#F59E0B"
              strokeWidth="2.2"
              strokeLinecap="round"
            />
            <path
              d="M6 16H26"
              stroke="#22D3EE"
              strokeWidth="1.6"
              strokeDasharray="3 2.5"
              strokeLinecap="round"
            />
            <circle cx="16" cy="16" r="2.1" fill="#22D3EE" />
          </svg>
          <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.1 }}>
            <span
              className="display"
              style={{ fontSize: 13.5, fontWeight: 700, color: "var(--text-primary)" }}
            >
              VENTURI
            </span>
            <span
              className="mono"
              style={{ fontSize: 9, color: "var(--text-muted)", letterSpacing: "0.08em" }}
            >
              RUST CFD STUDIO
            </span>
          </div>
        </div>

        <div style={{ width: 1, height: 22, background: "var(--border-subtle)" }} />

        <input
          className="input input-mono"
          style={{ width: 185, height: 28, fontSize: 11.5 }}
          value={cfdCase.name}
          onChange={(e) => onCaseNameChange(e.target.value)}
          aria-label="Case name"
          title="OpenFOAM case directory name"
        />

        {/* Preset Loader */}
        <select
          className="select"
          style={{ width: 205, height: 28, fontSize: 11.5 }}
          defaultValue=""
          onChange={(e) => {
            if (e.target.value) {
              onSelectPreset(e.target.value);
              e.target.value = "";
            }
          }}
          aria-label="Load engineering preset"
        >
          <option value="" disabled>
            Load Template Preset…
          </option>
          {CASE_PRESETS.map((p) => (
            <option key={p.id} value={p.id}>
              [{p.category}] {p.label}
            </option>
          ))}
        </select>
      </div>

      {/* OpenFOAM Dialect Switch */}
      <div className="segmented" title="Target OpenFOAM dictionary dialect">
        <button
          type="button"
          className={cfdCase.flavor === "foundationV14" ? "active" : ""}
          onClick={() => onFlavorChange("foundationV14")}
        >
          Foundation v14 (.org)
        </button>
        <button
          type="button"
          className={cfdCase.flavor === "openCfdV2606" ? "active" : ""}
          onClick={() => onFlavorChange("openCfdV2606")}
        >
          OpenCFD v2606 (.com)
        </button>
      </div>

      {/* Actions */}
      <div style={{ display: "flex", alignItems: "center", gap: 7 }}>
        <button
          type="button"
          className={`btn ${guidedMode ? "btn-guided-on" : ""}`}
          onClick={onToggleGuidedMode}
          aria-pressed={guidedMode}
          title="Attiva o disattiva la Modalità Guidata passo-passo (Aiuti ON / OFF)"
        >
          <Compass size={13} />
          Aiuti: {guidedMode ? "ON" : "OFF"}
        </button>

        <span className={envBadge.cls} title="Detected execution environment">
          <EnvIcon size={12} />
          {envBadge.label}
        </span>

        <button type="button" className="btn" onClick={onImportCase} title="Open an existing OpenFOAM case folder">
          <FolderOpen size={13} />
          Open
        </button>

        <button type="button" className="btn" onClick={onExportCase} title="Export complete OpenFOAM case to disk">
          <Download size={13} />
          Export
        </button>

        {isRunning ? (
          <button key="stop-btn" type="button" className="btn btn-danger" onClick={onStopRun}>
            <Square size={12} fill="currentColor" />
            Stop
          </button>
        ) : (
          <button key="run-btn" type="button" className="btn btn-primary" onClick={onQuickRun}>
            <Play size={12} fill="currentColor" />
            Run Case
          </button>
        )}
      </div>
    </header>
  );
};
