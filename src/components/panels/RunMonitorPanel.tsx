import React from "react";
import {
  Play,
  Square,
  CheckCircle2,
  AlertTriangle,
  XCircle,
  Info,
  Activity,
} from "lucide-react";
import {
  CfdCase,
  EnvironmentStatus,
  ResidualPoint,
  ValidationReport,
} from "../../types/cfd";

interface RunMonitorPanelProps {
  cfdCase: CfdCase;
  env: EnvironmentStatus | null;
  validation: ValidationReport | null;
  backend: string;
  onBackendChange: (b: string) => void;
  isRunning: boolean;
  runStatus: string;
  residuals: ResidualPoint[];
  onStart: () => void;
  onStop: () => void;
}

const SERIES_META: { key: keyof ResidualPoint; label: string; color: string }[] = [
  { key: "ux", label: "Ux", color: "#22d3ee" },
  { key: "uy", label: "Uy", color: "#60a5fa" },
  { key: "p", label: "p", color: "#f59e0b" },
  { key: "k", label: "k", color: "#10b981" },
  { key: "omega", label: "ω", color: "#f43f5e" },
];

export const RunMonitorPanel: React.FC<RunMonitorPanelProps> = ({
  cfdCase,
  env,
  validation,
  backend,
  onBackendChange,
  isRunning,
  runStatus,
  residuals,
  onStart,
  onStop,
}) => {
  const last = residuals.length > 0 ? residuals[residuals.length - 1] : null;

  // Logarithmic SVG Residual Plot bounds: 10^0 down to 10^-7
  const W = 340;
  const H = 165;
  const padL = 34;
  const padR = 10;
  const padT = 10;
  const padB = 22;
  const plotW = W - padL - padR;
  const plotH = H - padT - padB;
  const minExp = -7;
  const maxExp = 0;

  const maxIter = Math.max(20, residuals.length > 0 ? residuals[residuals.length - 1].iteration : 20);
  const xFor = (iter: number) => padL + (iter / maxIter) * plotW;
  const yFor = (val: number) => {
    const exp = Math.max(minExp, Math.min(maxExp, Math.log10(Math.max(1e-12, val))));
    const norm = (maxExp - exp) / (maxExp - minExp);
    return padT + norm * plotH;
  };

  const tolY = yFor(cfdCase.numerics.convergenceTol);

  return (
    <>
      {/* Execution Control */}
      <div className="section-block">
        <div className="section-title">
          <span>Solver Execution Engine</span>
          <span
            className={`pill ${
              runStatus === "converged" || runStatus === "completed"
                ? "pill-pass"
                : runStatus === "running"
                  ? "pill-cyan"
                  : runStatus === "error"
                    ? "pill-error"
                    : ""
            }`}
          >
            {runStatus.toUpperCase()}
          </span>
        </div>

        <div className="field">
          <label>Execution Backend</label>
          <select
            className="select"
            value={backend}
            onChange={(e) => onBackendChange(e.target.value)}
            disabled={isRunning}
          >
            <option value="demo">Rust Benchmark Convergence Preview (No Docker Needed)</option>
            <option value="local" disabled={!env?.localBinary}>
              Native Local OpenFOAM {env?.localBinary ? `(${env.localOpenfoam})` : "(Not in PATH)"}
            </option>
            <option value="docker" disabled={!env?.dockerRunning}>
              Docker Container {env?.dockerRunning ? "(Daemon Ready)" : "(Start Docker Desktop)"}
            </option>
          </select>
        </div>

        <div style={{ display: "flex", gap: 8 }}>
          {isRunning ? (
            <button key="term-btn" type="button" className="btn btn-danger" style={{ flex: 1 }} onClick={onStop}>
              <Square size={12} fill="currentColor" />
              Terminate Solver
            </button>
          ) : (
            <button
              key="launch-btn"
              type="button"
              className="btn btn-primary"
              style={{ flex: 1 }}
              onClick={onStart}
              disabled={validation ? !validation.readyToRun : false}
            >
              <Play size={12} fill="currentColor" />
              Launch Simulation
            </button>
          )}
        </div>
      </div>

      {/* Live Logarithmic Residuals Plot */}
      <div className="section-block">
        <div className="section-title">
          <span>Live Residuals (Log₁₀)</span>
          <Activity size={12} />
        </div>

        <div
          style={{
            background: "var(--bg-input)",
            border: "1px solid var(--border-subtle)",
            borderRadius: "var(--radius-md)",
            padding: "8px 6px 6px",
          }}
        >
          <svg viewBox={`0 0 ${W} ${H}`} width="100%" style={{ display: "block" }}>
            {/* Log gridlines */}
            {[0, -1, -2, -3, -4, -5, -6, -7].map((exp) => {
              const y = yFor(Math.pow(10, exp));
              return (
                <g key={exp}>
                  <line
                    x1={padL}
                    y1={y}
                    x2={W - padR}
                    y2={y}
                    stroke="#1c2536"
                    strokeWidth="1"
                  />
                  {exp % 2 === 0 || exp === -7 ? (
                    <text
                      x={padL - 5}
                      y={y + 3}
                      textAnchor="end"
                      fill="#65748e"
                      fontSize="8.5"
                      fontFamily="JetBrains Mono, monospace"
                    >
                      1e{exp}
                    </text>
                  ) : null}
                </g>
              );
            })}

            {/* Convergence Tolerance Target Line */}
            <line
              x1={padL}
              y1={tolY}
              x2={W - padR}
              y2={tolY}
              stroke="#10b981"
              strokeWidth="1"
              strokeDasharray="4 3"
              opacity="0.7"
            />
            <text
              x={W - padR - 2}
              y={tolY - 3}
              textAnchor="end"
              fill="#34d399"
              fontSize="8"
              fontFamily="JetBrains Mono, monospace"
            >
              tol {cfdCase.numerics.convergenceTol.toExponential(0)}
            </text>

            {/* Residual curves */}
            {SERIES_META.map((s) => {
              const pts = residuals
                .map((r) => {
                  const v = r[s.key];
                  if (typeof v !== "number" || !Number.isFinite(v)) return null;
                  return `${xFor(r.iteration).toFixed(1)},${yFor(v).toFixed(1)}`;
                })
                .filter(Boolean);
              if (pts.length < 2) return null;
              return (
                <polyline
                  key={s.key}
                  fill="none"
                  stroke={s.color}
                  strokeWidth="1.6"
                  strokeLinejoin="round"
                  points={pts.join(" ")}
                />
              );
            })}

            {/* X-axis label */}
            <text
              x={padL + plotW / 2}
              y={H - 4}
              textAnchor="middle"
              fill="#65748e"
              fontSize="8.5"
              fontFamily="JetBrains Mono, monospace"
            >
              Iteration / Time (max: {maxIter.toFixed(0)})
            </text>
          </svg>

          {/* Legend */}
          <div style={{ display: "flex", justifyContent: "center", gap: 12, marginTop: 4 }}>
            {SERIES_META.map((s) => (
              <span
                key={s.key}
                className="mono"
                style={{ fontSize: 10, color: s.color, display: "inline-flex", alignItems: "center", gap: 4 }}
              >
                <span style={{ width: 8, height: 2, background: s.color, display: "inline-block" }} />
                {s.label}
              </span>
            ))}
          </div>
        </div>

        {/* Live Force Coefficients & Continuity */}
        {last && (
          <div className="readout-box">
            <div className="readout-row">
              <span>Current Iteration</span>
              <span>{last.iteration.toFixed(0)}</span>
            </div>
            {last.cd !== null && (
              <div className="readout-row">
                <span>Drag Coefficient Cd</span>
                <span style={{ color: "var(--accent-amber-bright)" }}>{last.cd.toFixed(4)}</span>
              </div>
            )}
            {last.cl !== null && (
              <div className="readout-row">
                <span>Lift Coefficient Cl</span>
                <span style={{ color: "var(--cfd-inlet)" }}>{last.cl.toFixed(4)}</span>
              </div>
            )}
            {last.continuity !== null && (
              <div className="readout-row">
                <span>Cumulative Continuity</span>
                <span>{last.continuity.toExponential(2)}</span>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Pre-Flight Engineering Audit */}
      {validation && (
        <div className="section-block">
          <div className="section-title">
            <span>Pre-Flight Engineering Audit</span>
            <span className="mono" style={{ color: validation.readyToRun ? "#34d399" : "#fda4af" }}>
              SCORE {validation.score}/100
            </span>
          </div>

          <div style={{ display: "flex", flexDirection: "column", gap: 7 }}>
            {validation.items.map((item) => {
              const Icon =
                item.severity === "pass"
                  ? CheckCircle2
                  : item.severity === "warning"
                    ? AlertTriangle
                    : item.severity === "error"
                      ? XCircle
                      : Info;
              const cls =
                item.severity === "pass"
                  ? "notice-pass"
                  : item.severity === "warning"
                    ? "notice-warn"
                    : item.severity === "error"
                      ? "notice-error"
                      : "";
              return (
                <div key={item.id} className={`notice ${cls}`}>
                  <div style={{ display: "flex", alignItems: "center", gap: 6, fontWeight: 600 }}>
                    <Icon size={13} style={{ flexShrink: 0 }} />
                    <span>{item.title}</span>
                  </div>
                  <div style={{ marginTop: 3, opacity: 0.9, fontSize: 11 }}>{item.detail}</div>
                  {item.suggestion && (
                    <div
                      className="mono"
                      style={{ marginTop: 4, fontSize: 10.5, color: "var(--text-primary)", opacity: 0.95 }}
                    >
                      → {item.suggestion}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
    </>
  );
};
