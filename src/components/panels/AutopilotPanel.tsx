import React, { useEffect, useRef, useState } from "react";
import {
  Sparkles,
  Play,
  Square,
  RotateCcw,
  Check,
  X,
  Copy,
  Shield,
  Terminal,
  Trophy,
  Cpu,
  Radio,
  Bot,
  Eye,
  EyeOff,
  Flame,
  CheckCircle2,
  XCircle,
} from "lucide-react";
import {
  AutopilotEvent,
  AutopilotMode,
  EnvironmentStatus,
  GuardrailsConfig,
  IterationRecord,
  LlmConfig,
  LlmProviderKind,
  OptimizationGoal,
  PendingProposal,
  SessionState,
} from "../../types/cfd";

export interface AutopilotPanelProps {
  session: SessionState | null;
  env: EnvironmentStatus | null;
  autopilotRunning: boolean;
  autopilotLog: AutopilotEvent[];
  pendingProposal: PendingProposal | null;
  mcpSnippet: string;
  onUpdateGuardrails: (g: GuardrailsConfig) => void;
  onUpdateGoal: (goal: OptimizationGoal) => void;
  onStartLlmAutopilot: (cfg: LlmConfig, goal: OptimizationGoal) => void;
  onStartBuiltinLoop: (goal: OptimizationGoal, backend: string) => void;
  onApproveProposal: (approved: boolean) => void;
  onStopAutopilot: () => void;
  onRollback: (iterationIndex?: number) => void;
}

type SubTab = "loop" | "history" | "config";

const PROVIDER_DEFAULTS: Record<
  LlmProviderKind,
  { label: string; model: string; baseUrl: string; needsKey: boolean; keyPlaceholder: string }
> = {
  gemini: {
    label: "Gemini",
    model: "gemini-2.5-flash",
    baseUrl: "https://generativelanguage.googleapis.com",
    needsKey: true,
    keyPlaceholder: "AIza...",
  },
  anthropic: {
    label: "Anthropic Claude",
    model: "claude-sonnet-4-20250514",
    baseUrl: "https://api.anthropic.com",
    needsKey: true,
    keyPlaceholder: "sk-ant-...",
  },
  openAi: {
    label: "OpenAI-Compatible",
    model: "gpt-4o",
    baseUrl: "https://api.openai.com/v1",
    needsKey: true,
    keyPlaceholder: "sk-...",
  },
  ollama: {
    label: "Ollama Local",
    model: "qwen2.5:14b",
    baseUrl: "http://localhost:11434/v1",
    needsKey: false,
    keyPlaceholder: "",
  },
};

const FALLBACK_GUARDRAILS: GuardrailsConfig = {
  maxIterations: 8,
  maxCells: 2_500_000,
  maxRamMb: 8192,
  maxRefinementLevel: 4,
  targetYPlusMin: 0.5,
  targetYPlusMax: 5.0,
  maxSkewness: 4.0,
  maxNonOrtho: 65.0,
  autoRollback: true,
};

const FALLBACK_GOAL: OptimizationGoal = {
  prompt:
    "Achieve wall-resolved y+ within target bounds, refine wake velocity gradients, and minimize drag Cd.",
  targetYPlus: 1.0,
  maxDeltaPPa: null,
  minimizeCd: true,
  maximizeClOverCd: false,
};

function formatCompactCells(n: number): string {
  if (!Number.isFinite(n)) return "—";
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return n.toFixed(0);
}

function formatShortTime(iso: string): string {
  if (!iso) return "";
  const t = iso.indexOf("T");
  if (t >= 0) return iso.slice(t + 1, t + 9);
  return iso.slice(0, 8);
}

function meanYPlus(rec: IterationRecord): number | null {
  const patches = rec.results.yPlusPatches;
  if (!patches || patches.length === 0) return null;
  const sum = patches.reduce((acc, p) => acc + p.average, 0);
  return sum / patches.length;
}

function eventBadgeStyle(kind: AutopilotEvent["kind"]): { label: string; color: string; bg: string } {
  switch (kind) {
    case "thinking":
      return { label: "THINK", color: "var(--text-secondary)", bg: "rgba(154, 167, 189, 0.12)" };
    case "tool_call":
      return { label: "CALL", color: "var(--cfd-inlet)", bg: "var(--cfd-inlet-dim)" };
    case "tool_result":
      return { label: "OK", color: "var(--cfd-outlet)", bg: "var(--cfd-outlet-dim)" };
    case "proposal":
      return { label: "REVIEW", color: "var(--accent-amber-bright)", bg: "var(--accent-amber-dim)" };
    case "state_sync":
      return { label: "SYNC", color: "#67e8f9", bg: "var(--cfd-inlet-dim)" };
    case "done":
      return { label: "DONE", color: "#34d399", bg: "var(--cfd-outlet-dim)" };
    case "error":
      return { label: "ERR", color: "#fda4af", bg: "var(--cfd-wall-dim)" };
  }
}

export const AutopilotPanel: React.FC<AutopilotPanelProps> = ({
  session,
  env,
  autopilotRunning,
  autopilotLog,
  pendingProposal,
  mcpSnippet,
  onUpdateGuardrails,
  onUpdateGoal,
  onStartLlmAutopilot,
  onStartBuiltinLoop,
  onApproveProposal,
  onStopAutopilot,
  onRollback,
}) => {
  const [subTab, setSubTab] = useState<SubTab>("loop");
  const [provider, setProvider] = useState<LlmProviderKind>("gemini");
  const [model, setModel] = useState<string>(PROVIDER_DEFAULTS.gemini.model);
  const [baseUrl, setBaseUrl] = useState<string>(PROVIDER_DEFAULTS.gemini.baseUrl);
  const [apiKey, setApiKey] = useState<string>("");
  const [showKey, setShowKey] = useState<boolean>(false);
  const [mode, setMode] = useState<AutopilotMode>("autopilot");
  const [backend, setBackend] = useState<string>(env?.recommendedBackend ?? "demo");
  const [backendTouched, setBackendTouched] = useState<boolean>(false);
  const [copiedSnippet, setCopiedSnippet] = useState<boolean>(false);

  const streamEndRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!backendTouched && env?.recommendedBackend) {
      setBackend(env.recommendedBackend);
    }
  }, [env?.recommendedBackend, backendTouched]);

  useEffect(() => {
    if (subTab === "loop") {
      streamEndRef.current?.scrollIntoView({ behavior: "smooth", block: "nearest" });
    }
  }, [autopilotLog.length, subTab]);

  const guardrails = session?.guardrails ?? FALLBACK_GUARDRAILS;
  const goal = session?.goal ?? FALLBACK_GOAL;
  const iterations = session?.iterations ?? [];
  const toolLogs = session?.toolLogs ?? [];

  const isExternalMcp = session?.lastWriter === "mcp-external";
  const isInAppAutopilot = session?.lastWriter === "in-app-autopilot";

  const handleProviderChange = (next: LlmProviderKind) => {
    setProvider(next);
    setModel(PROVIDER_DEFAULTS[next].model);
    setBaseUrl(PROVIDER_DEFAULTS[next].baseUrl);
  };

  const updateGuardrail = <K extends keyof GuardrailsConfig>(key: K, val: GuardrailsConfig[K]) => {
    onUpdateGuardrails({ ...guardrails, [key]: val });
  };

  const updateGoalField = <K extends keyof OptimizationGoal>(key: K, val: OptimizationGoal[K]) => {
    onUpdateGoal({ ...goal, [key]: val });
  };

  const handleLaunchLlm = () => {
    const cfg: LlmConfig = {
      provider,
      apiKey: provider === "ollama" ? "" : apiKey.trim(),
      model: model.trim() || PROVIDER_DEFAULTS[provider].model,
      baseUrl: baseUrl.trim() || PROVIDER_DEFAULTS[provider].baseUrl,
      mode,
      backend,
    };
    onStartLlmAutopilot(cfg, goal);
  };

  const handleCopySnippet = async () => {
    try {
      await navigator.clipboard.writeText(mcpSnippet);
      setCopiedSnippet(true);
      window.setTimeout(() => setCopiedSnippet(false), 1800);
    } catch {
      setCopiedSnippet(false);
    }
  };

  return (
    <>
      {/* 3 Sub-Tabs inside 392px Inspector */}
      <div
        className="segmented"
        style={{ width: "100%", display: "grid", gridTemplateColumns: "1fr 1fr 1fr" }}
      >
        <button
          type="button"
          className={subTab === "loop" ? "active" : ""}
          onClick={() => setSubTab("loop")}
        >
          Loop{pendingProposal ? " ●" : ""}
        </button>
        <button
          type="button"
          className={subTab === "history" ? "active" : ""}
          onClick={() => setSubTab("history")}
        >
          History{iterations.length > 0 ? ` (${iterations.length})` : ""}
        </button>
        <button
          type="button"
          className={subTab === "config" ? "active" : ""}
          onClick={() => setSubTab("config")}
        >
          MCP / LLM
        </button>
      </div>

      {/* ========================== TAB 1: LOOP ========================== */}
      {subTab === "loop" && (
        <>
          {/* Live Sync Status Banner */}
          <div
            className={`notice ${isExternalMcp ? "notice-pass" : ""}`}
            style={{
              display: "flex",
              alignItems: "center",
              justifyContent: "space-between",
              gap: 8,
              borderLeftColor: isExternalMcp
                ? "var(--cfd-outlet)"
                : isInAppAutopilot
                  ? "var(--accent-amber)"
                  : "var(--cfd-inlet)",
            }}
          >
            <div style={{ display: "flex", alignItems: "center", gap: 7, minWidth: 0 }}>
              <svg width="10" height="10" viewBox="0 0 10 10" style={{ flexShrink: 0 }}>
                {isExternalMcp && (
                  <circle cx="5" cy="5" r="4.5" fill="none" stroke="#10b981" strokeWidth="1.2">
                    <animate attributeName="r" values="2.5;4.8;2.5" dur="1.6s" repeatCount="indefinite" />
                    <animate attributeName="opacity" values="0.9;0.15;0.9" dur="1.6s" repeatCount="indefinite" />
                  </circle>
                )}
                <circle
                  cx="5"
                  cy="5"
                  r="2.8"
                  fill={isExternalMcp ? "#10b981" : isInAppAutopilot ? "#f59e0b" : "#22d3ee"}
                />
              </svg>
              <span
                className="mono"
                style={{
                  fontSize: 10.5,
                  color: "var(--text-primary)",
                  overflow: "hidden",
                  textOverflow: "ellipsis",
                  whiteSpace: "nowrap",
                }}
              >
                Session v{session?.version ?? 1} · Last writer:{" "}
                <strong
                  style={{
                    color: isExternalMcp
                      ? "#34d399"
                      : isInAppAutopilot
                        ? "var(--accent-amber-bright)"
                        : "var(--cfd-inlet)",
                  }}
                >
                  {session?.lastWriter ?? "gui"}
                </strong>
              </span>
            </div>
            <span className="mono" style={{ fontSize: 9.5, color: "var(--text-muted)", flexShrink: 0 }}>
              {session?.updatedAt ? formatShortTime(session.updatedAt) : "LIVE"}
            </span>
          </div>

          {/* Goal & Target KPIs */}
          <div className="section-block">
            <div className="section-title">
              <span>Engineering Optimization Goal</span>
              <Sparkles size={12} />
            </div>

            <div className="field">
              <label>Objective Prompt</label>
              <textarea
                rows={3}
                className="input"
                style={{
                  height: "auto",
                  padding: "7px 9px",
                  resize: "vertical",
                  lineHeight: 1.4,
                  fontFamily: "var(--font-sans)",
                  fontSize: 11.5,
                }}
                value={goal.prompt}
                onChange={(e) => updateGoalField("prompt", e.target.value)}
                placeholder="Describe target physics, wall y+, pressure drop ceiling, or aerodynamic goal..."
              />
            </div>

            <div className="field-grid-2">
              <div className="field">
                <label>
                  Target y+ <span className="field-unit">dimless</span>
                </label>
                <input
                  type="number"
                  step="0.5"
                  min="0.1"
                  max="300"
                  placeholder="auto"
                  className="input input-mono"
                  value={goal.targetYPlus ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    updateGoalField("targetYPlus", v === "" ? null : Math.max(0.1, parseFloat(v) || 1));
                  }}
                />
              </div>
              <div className="field">
                <label>
                  Max Δp <span className="field-unit">Pa</span>
                </label>
                <input
                  type="number"
                  step="10"
                  min="0"
                  placeholder="none"
                  className="input input-mono"
                  value={goal.maxDeltaPPa ?? ""}
                  onChange={(e) => {
                    const v = e.target.value.trim();
                    updateGoalField("maxDeltaPPa", v === "" ? null : Math.max(0, parseFloat(v) || 0));
                  }}
                />
              </div>
            </div>

            <div className="field-grid-2">
              <label
                className="patch-card"
                style={{
                  flexDirection: "row",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "6px 8px",
                }}
              >
                <span style={{ fontSize: 11, color: "var(--text-primary)" }}>Minimize Cd</span>
                <input
                  type="checkbox"
                  checked={goal.minimizeCd}
                  onChange={(e) => updateGoalField("minimizeCd", e.target.checked)}
                />
              </label>
              <label
                className="patch-card"
                style={{
                  flexDirection: "row",
                  alignItems: "center",
                  justifyContent: "space-between",
                  padding: "6px 8px",
                }}
              >
                <span style={{ fontSize: 11, color: "var(--text-primary)" }}>Max Cl/Cd</span>
                <input
                  type="checkbox"
                  checked={goal.maximizeClOverCd}
                  onChange={(e) => updateGoalField("maximizeClOverCd", e.target.checked)}
                />
              </label>
            </div>
          </div>

          {/* Mode & Execution Controls */}
          <div className="section-block">
            <div className="section-title">
              <span>Closed-Loop Autopilot Control</span>
              <span className={`pill ${autopilotRunning ? "pill-warn" : "pill-cyan"}`}>
                {autopilotRunning ? "RUNNING" : mode.toUpperCase()}
              </span>
            </div>

            <div className="field-grid-2">
              <div className="field">
                <label>Governance Mode</label>
                <div
                  className="segmented"
                  style={{ width: "100%", display: "grid", gridTemplateColumns: "1fr 1fr", height: 30 }}
                >
                  <button
                    type="button"
                    className={mode === "autopilot" ? "active" : ""}
                    onClick={() => setMode("autopilot")}
                    disabled={autopilotRunning}
                  >
                    Autopilot
                  </button>
                  <button
                    type="button"
                    className={mode === "review" ? "active" : ""}
                    onClick={() => setMode("review")}
                    disabled={autopilotRunning}
                  >
                    Review
                  </button>
                </div>
              </div>

              <div className="field">
                <label>Solver Backend</label>
                <select
                  className="select"
                  value={backend}
                  onChange={(e) => {
                    setBackend(e.target.value);
                    setBackendTouched(true);
                  }}
                  disabled={autopilotRunning}
                >
                  <option value="demo">Surrogate / Preview</option>
                  <option value="local" disabled={!env?.localBinary}>
                    Local OpenFOAM {env?.localBinary ? "✓" : "(N/A)"}
                  </option>
                  <option value="docker" disabled={!env?.dockerRunning}>
                    Docker {env?.dockerRunning ? "✓" : "(Off)"}
                  </option>
                </select>
              </div>
            </div>

            <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              {autopilotRunning ? (
                <button
                  type="button"
                  className="btn btn-danger"
                  style={{ width: "100%" }}
                  onClick={onStopAutopilot}
                >
                  <Square size={12} fill="currentColor" />
                  Stop Autopilot Loop
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    className="btn btn-primary"
                    style={{ width: "100%" }}
                    onClick={handleLaunchLlm}
                  >
                    <Bot size={13} />
                    Launch LLM Autopilot ({PROVIDER_DEFAULTS[provider].label})
                  </button>
                  <button
                    type="button"
                    className="btn"
                    style={{
                      width: "100%",
                      borderColor: "rgba(34, 211, 238, 0.38)",
                      color: "#67e8f9",
                    }}
                    onClick={() => onStartBuiltinLoop(goal, backend)}
                  >
                    <Play size={12} fill="currentColor" />
                    Run Built-In Closed Loop (Zero API Key)
                  </button>
                </>
              )}
            </div>

            <div
              className="mono"
              style={{
                fontSize: 10,
                color: "var(--text-muted)",
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
              }}
            >
              <span>
                LLM: {PROVIDER_DEFAULTS[provider].label} · {model}
              </span>
              <button
                type="button"
                className="btn btn-xs btn-subtle"
                style={{ height: 20, padding: "0 6px", fontSize: 10 }}
                onClick={() => setSubTab("config")}
              >
                Configure
              </button>
            </div>
          </div>

          {/* Pending Proposal Card (Review Mode Gate) */}
          {pendingProposal && (
            <div
              className="section-block"
              style={{
                background: "rgba(245, 158, 11, 0.06)",
                border: "1px solid var(--accent-amber-border)",
                borderRadius: "var(--radius-md)",
                padding: "10px 11px",
              }}
            >
              <div className="section-title">
                <span style={{ color: "var(--accent-amber-bright)" }}>Pending Tool Proposal</span>
                <span className="pill pill-warn">REVIEW GATE</span>
              </div>

              <div className="readout-box" style={{ gap: 4 }}>
                <div className="readout-row">
                  <span>MCP Tool</span>
                  <span style={{ color: "var(--cfd-inlet)" }}>{pendingProposal.toolName}</span>
                </div>
                <div style={{ fontSize: 11, color: "var(--text-secondary)", lineHeight: 1.4 }}>
                  {pendingProposal.rationale}
                </div>
              </div>

              <pre
                className="mono"
                style={{
                  margin: 0,
                  padding: "7px 9px",
                  background: "var(--bg-canvas)",
                  border: "1px solid var(--border-subtle)",
                  borderRadius: "var(--radius-sm)",
                  fontSize: 10.5,
                  color: "#d6e0f0",
                  maxHeight: 130,
                  overflow: "auto",
                  userSelect: "text",
                }}
              >
                {JSON.stringify(pendingProposal.arguments, null, 2)}
              </pre>

              <div style={{ display: "flex", gap: 6 }}>
                <button
                  type="button"
                  className="btn btn-primary"
                  style={{ flex: 1 }}
                  onClick={() => onApproveProposal(true)}
                >
                  <Check size={13} />
                  Approve & Execute
                </button>
                <button
                  type="button"
                  className="btn btn-danger"
                  onClick={() => onApproveProposal(false)}
                >
                  <X size={13} />
                  Reject
                </button>
              </div>
            </div>
          )}

          {/* Live Autopilot Thought & Tool Stream */}
          <div className="section-block">
            <div className="section-title">
              <span>Live Thought & Tool Stream</span>
              <span className="mono" style={{ fontSize: 10, color: "var(--text-muted)" }}>
                {autopilotLog.length} events
              </span>
            </div>

            <div
              style={{
                background: "var(--bg-input)",
                border: "1px solid var(--border-subtle)",
                borderRadius: "var(--radius-md)",
                padding: "8px 9px",
                minHeight: 110,
                maxHeight: 230,
                overflowY: "auto",
                display: "flex",
                flexDirection: "column",
                gap: 6,
                userSelect: "text",
              }}
            >
              {autopilotLog.length === 0 ? (
                <div style={{ fontSize: 11, color: "var(--text-muted)", lineHeight: 1.45 }}>
                  Ready. Launch the <strong style={{ color: "var(--text-secondary)" }}>Built-In Closed Loop</strong>{" "}
                  for immediate baseline → y+ sizing → wake refinement → verification, or connect an external MCP agent
                  via the <strong style={{ color: "var(--text-secondary)" }}>MCP / LLM</strong> tab.
                </div>
              ) : (
                autopilotLog.map((evt, idx) => {
                  const badge = eventBadgeStyle(evt.kind);
                  return (
                    <div
                      key={`${evt.kind}-${idx}`}
                      style={{
                        display: "flex",
                        alignItems: "flex-start",
                        gap: 6,
                        fontSize: 11,
                        lineHeight: 1.38,
                        borderBottom:
                          idx < autopilotLog.length - 1 ? "1px solid rgba(30, 39, 54, 0.7)" : "none",
                        paddingBottom: idx < autopilotLog.length - 1 ? 5 : 0,
                      }}
                    >
                      <span
                        className="mono"
                        style={{
                          fontSize: 9,
                          fontWeight: 600,
                          padding: "1px 5px",
                          borderRadius: 3,
                          background: badge.bg,
                          color: badge.color,
                          flexShrink: 0,
                          marginTop: 1,
                        }}
                      >
                        {badge.label}
                      </span>
                      <div style={{ minWidth: 0, flex: 1, wordBreak: "break-word" }}>
                        {evt.toolName && (
                          <span
                            className="mono"
                            style={{
                              fontSize: 10.5,
                              color: "var(--cfd-inlet)",
                              marginRight: 5,
                            }}
                          >
                            {evt.toolName}
                          </span>
                        )}
                        <span style={{ color: "var(--text-primary)" }}>{evt.message}</span>
                      </div>
                    </div>
                  );
                })
              )}
              <div ref={streamEndRef} />
            </div>
          </div>
        </>
      )}

      {/* ========================== TAB 2: HISTORY ========================== */}
      {subTab === "history" && (
        <>
          {/* Iteration Leaderboard */}
          <div className="section-block">
            <div className="section-title">
              <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                <Trophy size={12} />
                Iteration Leaderboard
              </span>
              {session?.bestIterationIndex !== null && session?.bestIterationIndex !== undefined && (
                <button
                  type="button"
                  className="btn btn-xs"
                  onClick={() => onRollback(session.bestIterationIndex ?? undefined)}
                  title="Restore best scoring case snapshot"
                >
                  <RotateCcw size={10} />
                  Best #{session.bestIterationIndex}
                </button>
              )}
            </div>

            {iterations.length === 0 ? (
              <div className="notice">
                No iterations recorded yet. Run the Built-In Closed Loop, LLM Autopilot, or call{" "}
                <code className="mono">venturi_run_iteration</code> from an external MCP agent.
              </div>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
                {iterations.map((rec) => {
                  const avgYp = meanYPlus(rec);
                  const skewBad = rec.results.checkMesh.maxSkewness > guardrails.maxSkewness;
                  const hotspotsCount = rec.results.hotspots.length;
                  return (
                    <div
                      key={rec.index}
                      className={`patch-card ${rec.isBest ? "selected" : ""}`}
                      style={{
                        cursor: "default",
                        borderColor: rec.isBest
                          ? "rgba(16, 185, 129, 0.55)"
                          : rec.rolledBack
                            ? "rgba(244, 63, 94, 0.45)"
                            : undefined,
                        background: rec.isBest ? "rgba(16, 185, 129, 0.05)" : undefined,
                      }}
                    >
                      {/* Top header */}
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          gap: 6,
                        }}
                      >
                        <div style={{ display: "flex", alignItems: "center", gap: 5, minWidth: 0, flexWrap: "wrap" }}>
                          <span
                            className="mono"
                            style={{ fontWeight: 700, fontSize: 11.5, color: "var(--accent-amber-bright)" }}
                          >
                            #{rec.index}
                          </span>
                          <span
                            style={{
                              fontWeight: 600,
                              fontSize: 11.5,
                              color: "var(--text-primary)",
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                            }}
                          >
                            {rec.label}
                          </span>
                          {rec.isBest && (
                            <span className="pill pill-pass" style={{ fontSize: 9, padding: "1px 6px" }}>
                              BEST
                            </span>
                          )}
                          {rec.rolledBack && (
                            <span className="pill pill-error" style={{ fontSize: 9, padding: "1px 6px" }}>
                              ROLLED BACK
                            </span>
                          )}
                        </div>
                        <span
                          className="mono"
                          style={{
                            fontSize: 11.5,
                            fontWeight: 700,
                            color: rec.isBest ? "#34d399" : "var(--text-primary)",
                            flexShrink: 0,
                          }}
                        >
                          {rec.objectiveScore.toFixed(1)}/100
                        </span>
                      </div>

                      {/* 3x2 KPI Grid */}
                      <div
                        className="field-grid-3"
                        style={{
                          background: "var(--bg-canvas)",
                          border: "1px solid var(--border-subtle)",
                          borderRadius: "var(--radius-sm)",
                          padding: "6px 7px",
                          gap: "5px 6px",
                        }}
                      >
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">Cd</span>
                          <span className="mono" style={{ fontSize: 11, color: "var(--accent-amber-bright)" }}>
                            {rec.results.cd !== null ? rec.results.cd.toFixed(4) : "—"}
                          </span>
                        </div>
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">Cl</span>
                          <span className="mono" style={{ fontSize: 11, color: "var(--cfd-inlet)" }}>
                            {rec.results.cl !== null ? rec.results.cl.toFixed(4) : "—"}
                          </span>
                        </div>
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">Δp</span>
                          <span className="mono" style={{ fontSize: 11 }}>
                            {rec.results.deltaPPa !== null ? `${rec.results.deltaPPa.toFixed(1)} Pa` : "—"}
                          </span>
                        </div>
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">y+ avg</span>
                          <span className="mono" style={{ fontSize: 11 }}>
                            {avgYp !== null ? avgYp.toFixed(2) : "—"}
                          </span>
                        </div>
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">Cells</span>
                          <span className="mono" style={{ fontSize: 11 }}>
                            {formatCompactCells(rec.results.checkMesh.cells)}
                          </span>
                        </div>
                        <div style={{ display: "flex", flexDirection: "column" }}>
                          <span className="telemetry-label">Skewness</span>
                          <span
                            className="mono"
                            style={{
                              fontSize: 11,
                              color: skewBad ? "#fda4af" : "var(--text-primary)",
                            }}
                          >
                            {rec.results.checkMesh.maxSkewness.toFixed(2)}
                          </span>
                        </div>
                      </div>

                      {/* Summary */}
                      <div style={{ fontSize: 11, color: "var(--text-secondary)", lineHeight: 1.35 }}>
                        {rec.changesSummary}
                      </div>

                      {/* Footer: hotspots & rollback */}
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          gap: 6,
                          marginTop: 1,
                        }}
                      >
                        <div style={{ display: "flex", alignItems: "center", gap: 5 }}>
                          <span
                            className={`pill ${hotspotsCount > 0 ? "pill-warn" : "pill-pass"}`}
                            style={{ fontSize: 9.5, padding: "1px 6px" }}
                          >
                            <Flame size={9} />
                            {hotspotsCount} hotspot{hotspotsCount === 1 ? "" : "s"}
                          </span>
                          <span className="mono" style={{ fontSize: 9.5, color: "var(--text-muted)" }}>
                            {rec.results.backendUsed}
                          </span>
                        </div>
                        <button
                          type="button"
                          className="btn btn-xs"
                          onClick={() => onRollback(rec.index)}
                        >
                          <RotateCcw size={10} />
                          Rollback to #{rec.index}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* MCP Tool Audit Log */}
          <div className="section-block">
            <div className="section-title">
              <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                <Terminal size={12} />
                MCP Tool Audit Log
              </span>
              <span className="mono" style={{ fontSize: 10, color: "var(--text-muted)" }}>
                {toolLogs.length} calls
              </span>
            </div>

            {toolLogs.length === 0 ? (
              <div className="notice">No MCP tool calls recorded in this session yet.</div>
            ) : (
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 5,
                  maxHeight: 240,
                  overflowY: "auto",
                }}
              >
                {[...toolLogs].reverse().map((entry) => {
                  const ext = entry.source === "mcp-external";
                  const inApp = entry.source === "in-app-autopilot";
                  return (
                    <div
                      key={entry.id}
                      className="readout-box"
                      style={{ padding: "7px 9px", gap: 4 }}
                    >
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          justifyContent: "space-between",
                          gap: 6,
                        }}
                      >
                        <div style={{ display: "flex", alignItems: "center", gap: 5, minWidth: 0 }}>
                          <span
                            className={`pill ${ext ? "pill-cyan" : inApp ? "pill-warn" : ""}`}
                            style={{ fontSize: 9, padding: "1px 5px", flexShrink: 0 }}
                          >
                            {entry.source}
                          </span>
                          <span
                            className="mono"
                            style={{
                              fontSize: 10.5,
                              color: "var(--text-primary)",
                              overflow: "hidden",
                              textOverflow: "ellipsis",
                              whiteSpace: "nowrap",
                            }}
                            title={entry.toolName}
                          >
                            {entry.toolName}
                          </span>
                        </div>
                        <span
                          className="mono"
                          style={{
                            display: "inline-flex",
                            alignItems: "center",
                            gap: 3,
                            fontSize: 9.5,
                            color: entry.ok ? "#34d399" : "#fda4af",
                            flexShrink: 0,
                          }}
                        >
                          {entry.ok ? <CheckCircle2 size={10} /> : <XCircle size={10} />}
                          {entry.ok ? "OK" : "ERR"}
                        </span>
                      </div>
                      <div
                        style={{
                          fontSize: 10.5,
                          color: "var(--text-secondary)",
                          lineHeight: 1.35,
                          wordBreak: "break-word",
                        }}
                      >
                        {entry.summary}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </>
      )}

      {/* ========================== TAB 3: MCP / LLM ========================== */}
      {subTab === "config" && (
        <>
          {/* LLM Provider Configuration */}
          <div className="section-block">
            <div className="section-title">
              <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                <Cpu size={12} />
                In-App LLM Provider
              </span>
              <span className="mono" style={{ fontSize: 10, color: "var(--accent-amber)" }}>
                TOOL USE
              </span>
            </div>

            <div className="field">
              <label>Provider</label>
              <select
                className="select"
                value={provider}
                onChange={(e) => handleProviderChange(e.target.value as LlmProviderKind)}
              >
                <option value="gemini">Gemini (Google AI)</option>
                <option value="anthropic">Anthropic Claude</option>
                <option value="openAi">OpenAI-Compatible</option>
                <option value="ollama">Ollama Local (Zero Key)</option>
              </select>
            </div>

            <div className="field">
              <label>Model ID</label>
              <input
                type="text"
                className="input input-mono"
                value={model}
                onChange={(e) => setModel(e.target.value)}
                placeholder={PROVIDER_DEFAULTS[provider].model}
              />
            </div>

            <div className="field">
              <label>API Base URL</label>
              <input
                type="text"
                className="input input-mono"
                value={baseUrl}
                onChange={(e) => setBaseUrl(e.target.value)}
                placeholder={PROVIDER_DEFAULTS[provider].baseUrl}
              />
            </div>

            {PROVIDER_DEFAULTS[provider].needsKey ? (
              <div className="field">
                <label>API Key (held in memory only)</label>
                <div style={{ display: "flex", gap: 5 }}>
                  <input
                    type={showKey ? "text" : "password"}
                    className="input input-mono"
                    style={{ flex: 1 }}
                    value={apiKey}
                    onChange={(e) => setApiKey(e.target.value)}
                    placeholder={PROVIDER_DEFAULTS[provider].keyPlaceholder}
                    autoComplete="off"
                  />
                  <button
                    type="button"
                    className="btn"
                    style={{ padding: "0 8px" }}
                    onClick={() => setShowKey((v) => !v)}
                    title={showKey ? "Hide API key" : "Show API key"}
                  >
                    {showKey ? <EyeOff size={13} /> : <Eye size={13} />}
                  </button>
                </div>
              </div>
            ) : (
              <div className="notice notice-pass">
                Ollama runs locally at <code className="mono">{baseUrl}</code> — no API key required.
              </div>
            )}
          </div>

          {/* Hard Guardrails Configuration */}
          <div className="section-block">
            <div className="section-title">
              <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                <Shield size={12} />
                Hard Engineering Guardrails
              </span>
              <span className="pill pill-warn" style={{ fontSize: 9.5 }}>
                ENFORCED
              </span>
            </div>

            <div className="field-grid-2">
              <div className="field">
                <label>Max Iterations</label>
                <input
                  type="number"
                  min={1}
                  max={50}
                  className="input input-mono"
                  value={guardrails.maxIterations}
                  onChange={(e) =>
                    updateGuardrail("maxIterations", Math.max(1, parseInt(e.target.value, 10) || 1))
                  }
                />
              </div>
              <div className="field">
                <label>Max Cells</label>
                <input
                  type="number"
                  step={100000}
                  min={10000}
                  className="input input-mono"
                  value={guardrails.maxCells}
                  onChange={(e) =>
                    updateGuardrail("maxCells", Math.max(10000, parseInt(e.target.value, 10) || 100000))
                  }
                />
              </div>
              <div className="field">
                <label>
                  Max RAM <span className="field-unit">MB</span>
                </label>
                <input
                  type="number"
                  step={512}
                  min={512}
                  className="input input-mono"
                  value={guardrails.maxRamMb}
                  onChange={(e) =>
                    updateGuardrail("maxRamMb", Math.max(512, parseFloat(e.target.value) || 2048))
                  }
                />
              </div>
              <div className="field">
                <label>Max Ref. Level</label>
                <input
                  type="number"
                  min={1}
                  max={6}
                  className="input input-mono"
                  value={guardrails.maxRefinementLevel}
                  onChange={(e) =>
                    updateGuardrail(
                      "maxRefinementLevel",
                      Math.min(6, Math.max(1, parseInt(e.target.value, 10) || 3))
                    )
                  }
                />
              </div>
              <div className="field">
                <label>y+ Min</label>
                <input
                  type="number"
                  step="0.1"
                  min={0.05}
                  className="input input-mono"
                  value={guardrails.targetYPlusMin}
                  onChange={(e) =>
                    updateGuardrail("targetYPlusMin", Math.max(0.05, parseFloat(e.target.value) || 0.1))
                  }
                />
              </div>
              <div className="field">
                <label>y+ Max</label>
                <input
                  type="number"
                  step="0.5"
                  min={0.5}
                  className="input input-mono"
                  value={guardrails.targetYPlusMax}
                  onChange={(e) =>
                    updateGuardrail("targetYPlusMax", Math.max(0.5, parseFloat(e.target.value) || 5))
                  }
                />
              </div>
              <div className="field">
                <label>Max Skewness</label>
                <input
                  type="number"
                  step="0.2"
                  min={1.0}
                  max={10.0}
                  className="input input-mono"
                  value={guardrails.maxSkewness}
                  onChange={(e) =>
                    updateGuardrail("maxSkewness", Math.max(1.0, parseFloat(e.target.value) || 4.0))
                  }
                />
              </div>
              <div className="field">
                <label>
                  Max Non-Ortho <span className="field-unit">deg</span>
                </label>
                <input
                  type="number"
                  step="5"
                  min={30}
                  max={85}
                  className="input input-mono"
                  value={guardrails.maxNonOrtho}
                  onChange={(e) =>
                    updateGuardrail("maxNonOrtho", Math.max(30, parseFloat(e.target.value) || 65))
                  }
                />
              </div>
            </div>

            <label className="toggle-row">
              <span>Auto-Rollback on Regression</span>
              <input
                type="checkbox"
                checked={guardrails.autoRollback}
                onChange={(e) => updateGuardrail("autoRollback", e.target.checked)}
              />
            </label>
          </div>

          {/* External Agent MCP Server Card */}
          <div className="section-block">
            <div className="section-title">
              <span style={{ display: "inline-flex", alignItems: "center", gap: 5 }}>
                <Radio size={12} />
                External Agent MCP Server
              </span>
              <span className="pill pill-cyan" style={{ fontSize: 9.5 }}>
                venturi-mcp
              </span>
            </div>

            <div className="notice">
              Connect <strong style={{ color: "var(--text-primary)" }}>Antigravity (agy)</strong>,{" "}
              <strong style={{ color: "var(--text-primary)" }}>Claude Desktop</strong>, or{" "}
              <strong style={{ color: "var(--text-primary)" }}>Cursor</strong> to drive Venturi over stdio JSON-RPC.
              Every MCP mutation live-syncs the 3D viewport and OpenFOAM dictionaries.
            </div>

            <div style={{ position: "relative" }}>
              <pre
                className="mono"
                style={{
                  margin: 0,
                  padding: "9px 10px",
                  background: "var(--bg-canvas)",
                  border: "1px solid var(--border-default)",
                  borderRadius: "var(--radius-md)",
                  fontSize: 10.5,
                  lineHeight: 1.45,
                  color: "#d6e0f0",
                  overflowX: "auto",
                  userSelect: "text",
                }}
              >
                {mcpSnippet ||
                  JSON.stringify(
                    {
                      mcpServers: {
                        "venturi-cfd": {
                          command: "venturi_mcp",
                          args: ["--session", "~/.venturi/active_session.json"],
                        },
                      },
                    },
                    null,
                    2
                  )}
              </pre>
            </div>

            <button
              type="button"
              className="btn"
              style={{ width: "100%" }}
              onClick={handleCopySnippet}
            >
              {copiedSnippet ? <Check size={13} style={{ color: "#34d399" }} /> : <Copy size={13} />}
              {copiedSnippet ? "Copied MCP Config JSON!" : "Copy MCP Config for agy / Claude / Cursor"}
            </button>
          </div>
        </>
      )}
    </>
  );
};
