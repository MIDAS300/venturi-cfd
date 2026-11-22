import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  AutopilotEvent,
  CfdCase,
  DomainFace,
  EnvironmentStatus,
  GeneratedCaseFiles,
  GuardrailsConfig,
  LlmConfig,
  OpenFoamFlavor,
  OptimizationGoal,
  ParametricGeometrySpec,
  PendingProposal,
  ResidualPoint,
  SessionState,
  StlMeshPayload,
  TelemetrySnapshot,
  ValidationReport,
  WorkflowTab,
} from "./types/cfd";
import { CASE_PRESETS } from "./data/presets";
import {
  approveProposal,
  computeTelemetry,
  detectEnvironment,
  exportCaseWithDialog,
  generateCase,
  generateParametricGeometry,
  getMcpConfigSnippet,
  getSession,
  importCaseWithDialog,
  importStlWithDialog,
  isTauriRuntime,
  listenAutopilotEvents,
  listenSolverEvents,
  rollbackIteration,
  startAutopilot,
  startBuiltinAutopilot,
  startSolver,
  stopAutopilot,
  stopSolver,
  syncSessionState,
  validateCase,
} from "./api/bridge";
import { TitleBar } from "./components/TitleBar";
import { WorkflowRail } from "./components/WorkflowRail";
import { Viewport3D } from "./components/Viewport3D";
import { DictionaryDrawer } from "./components/DictionaryDrawer";
import { GeometryPanel } from "./components/panels/GeometryPanel";
import { MeshPanel } from "./components/panels/MeshPanel";
import { PhysicsPanel } from "./components/panels/PhysicsPanel";
import { BoundariesPanel } from "./components/panels/BoundariesPanel";
import { NumericsPanel } from "./components/panels/NumericsPanel";
import { EngineeringLabPanel } from "./components/panels/EngineeringLabPanel";
import { RunMonitorPanel } from "./components/panels/RunMonitorPanel";
import { AutopilotPanel } from "./components/panels/AutopilotPanel";
import { GuidedAssistant } from "./components/GuidedAssistant";

const TAB_HEADERS: Record<WorkflowTab, { title: string; subtitle: string }> = {
  geometry: {
    title: "Domain & CAD Geometry",
    subtitle: "Parametric Rust STL generator, CAD import & blockMesh extents",
  },
  mesh: {
    title: "Mesh & Boundary Layers",
    subtitle: "blockMesh, snappyHexMesh, refinementRegions & y+ inflation",
  },
  physics: {
    title: "Fluid & Turbulence Physics",
    subtitle: "Thermophysical properties, Sutherland law (T, P) & RANS/LES",
  },
  boundaries: {
    title: "Boundary Conditions",
    subtitle: "Patch velocity, pressure & wall roughness (3D synced)",
  },
  numerics: {
    title: "Solver & Discretization",
    subtitle: "fvSchemes accuracy, SIMPLE/PIMPLE relaxation & MPI",
  },
  lab: {
    title: "CFD Engineering Lab",
    subtitle: "Rust calculators for y+, turbulence ICs & CFL stability",
  },
  run: {
    title: "Pre-Flight Audit & Solve",
    subtitle: "Static CFD verification, residuals & aerodynamic forces",
  },
  autopilot: {
    title: "AI Autopilot & MCP Studio",
    subtitle: "Closed-loop LLM optimization, guardrails, rollback & venturi-mcp",
  },
};

const DEFAULT_GUARDRAILS: GuardrailsConfig = {
  maxIterations: 8,
  maxCells: 2_000_000,
  maxRamMb: 4096,
  maxRefinementLevel: 4,
  targetYPlusMin: 0.5,
  targetYPlusMax: 5.0,
  maxSkewness: 4.0,
  maxNonOrtho: 65.0,
  autoRollback: true,
};

const DEFAULT_GOAL: OptimizationGoal = {
  prompt:
    "Analyze OpenFOAM results, auto-size y+ boundary layers, add wake refinement boxes on high-gradient hotspots, and optimize geometry to minimize drag / pressure drop.",
  targetYPlus: 1.5,
  maxDeltaPPa: null,
  minimizeCd: true,
  maximizeClOverCd: false,
};

export const App: React.FC = () => {
  const initialParams =
    typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
  const initialPreset = initialParams?.get("preset");
  const initialTab = (initialParams?.get("tab") as WorkflowTab | null) ?? "geometry";
  const initialHighlight = (initialParams?.get("highlight") as DomainFace | null) ?? null;
  const liveSessionFile = initialParams?.get("sessionFile");

  const [cfdCase, setCfdCase] = useState<CfdCase>(() => {
    const matched = initialPreset ? CASE_PRESETS.find((p) => p.id === initialPreset) : null;
    return (matched ?? CASE_PRESETS[0]).build();
  });
  const [activeTab, setActiveTab] = useState<WorkflowTab>(initialTab);
  const [guidedMode, setGuidedMode] = useState<boolean>(() => {
    const param = initialParams?.get("guided");
    if (param === "0" || param === "false") return false;
    if (param === "1" || param === "true") return true;
    try {
      const saved = window.localStorage.getItem("venturi.guidedMode");
      if (saved === "false") return false;
      if (saved === "true") return true;
    } catch {
      // ignore storage errors
    }
    return true;
  });
  const [stlMesh, setStlMesh] = useState<StlMeshPayload | null>(null);
  const rawStlMeshRef = useRef<StlMeshPayload | null>(null);
  const [highlightedFace, setHighlightedFace] = useState<DomainFace | null>(initialHighlight);

  const handleToggleGuidedMode = useCallback(() => {
    setGuidedMode((prev) => {
      const next = !prev;
      try {
        window.localStorage.setItem("venturi.guidedMode", String(next));
      } catch {
        // ignore
      }
      return next;
    });
  }, []);

  const [telemetry, setTelemetry] = useState<TelemetrySnapshot | null>(null);
  const [validation, setValidation] = useState<ValidationReport | null>(null);
  const [generated, setGenerated] = useState<GeneratedCaseFiles | null>(null);
  const [env, setEnv] = useState<EnvironmentStatus | null>(null);

  // Shared MCP session & Autopilot state
  const [session, setSession] = useState<SessionState | null>(null);
  const [guardrails, setGuardrails] = useState<GuardrailsConfig>(DEFAULT_GUARDRAILS);
  const [goal, setGoal] = useState<OptimizationGoal>(DEFAULT_GOAL);
  const [mcpSnippet, setMcpSnippet] = useState<string>("");
  const [autopilotRunning, setAutopilotRunning] = useState(false);
  const [autopilotLog, setAutopilotLog] = useState<AutopilotEvent[]>([]);
  const [pendingProposal, setPendingProposal] = useState<PendingProposal | null>(null);
  const lastVersionRef = useRef<number>(0);
  const skipNextLocalSyncRef = useRef<boolean>(false);

  const [backend, setBackend] = useState<string>("demo");
  const [isRunning, setIsRunning] = useState(false);
  const [runStatus, setRunStatus] = useState("idle");
  const [residuals, setResiduals] = useState<ResidualPoint[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [statusMessage, setStatusMessage] = useState<string>(
    "Ready — Rust CFD & MCP Engine active"
  );

  // Apply an externally updated SessionState (from venturi-mcp or in-app autopilot)
  const applyRemoteSession = useCallback((remote: SessionState) => {
    lastVersionRef.current = remote.version;
    skipNextLocalSyncRef.current = true;
    setSession(remote);
    setCfdCase(remote.case);
    setGuardrails(remote.guardrails);
    setGoal(remote.goal);

    // If the remote session has parametric geometry, regenerate its 3D triangle mesh for viewport
    if (remote.case.geometryKind === "parametric" && remote.case.parametric) {
      generateParametricGeometry(remote.case.parametric)
        .then((payload) => setStlMesh(payload))
        .catch(() => {});
    } else if (remote.case.geometryKind !== "customStl") {
      setStlMesh(null);
    }

    // Mirror latest iteration logs into console & status bar when driven externally
    if (remote.toolLogs.length > 0) {
      const latest = remote.toolLogs[remote.toolLogs.length - 1];
      setStatusMessage(
        `Live Sync v${remote.version} [${latest.source}] ${latest.toolName}: ${latest.summary}`
      );
      setLogs(
        remote.toolLogs.slice(-40).map(
          (t) =>
            `[${t.source}] ${t.toolName}(${t.argsPreview}) → ${t.ok ? "OK" : "ERR"}: ${t.summary}`
        )
      );
    }
  }, []);

  // Probe host environment & MCP config snippet once
  useEffect(() => {
    detectEnvironment()
      .then((status) => {
        setEnv(status);
        setBackend(status.recommendedBackend);
      })
      .catch(() => {});

    getMcpConfigSnippet()
      .then((s) => setMcpSnippet(s))
      .catch(() => {});

    if (isTauriRuntime() || liveSessionFile) {
      const fetchInitial = async () => {
        try {
          if (!isTauriRuntime() && liveSessionFile) {
            const resp = await fetch(
              `/__venturi_session?file=${encodeURIComponent(liveSessionFile)}`
            );
            if (resp.ok) {
              const remote: SessionState = await resp.json();
              applyRemoteSession(remote);
            }
          } else {
            const remote = await getSession();
            applyRemoteSession(remote);
          }
        } catch {
          // ignore if session not yet created
        }
      };
      fetchInitial();
    }
  }, [applyRemoteSession, liveSessionFile]);

  // Live 400ms Shared Session Watcher (syncs venturi-mcp external agent mutations live into 3D GUI!)
  useEffect(() => {
    if (!isTauriRuntime() && !liveSessionFile) return;
    const id = window.setInterval(async () => {
      try {
        let remote: SessionState | null = null;
        if (isTauriRuntime()) {
          remote = await getSession();
        } else if (liveSessionFile) {
          const resp = await fetch(
            `/__venturi_session?file=${encodeURIComponent(liveSessionFile)}`
          );
          if (resp.ok) {
            remote = await resp.json();
          }
        }
        if (remote && remote.version > lastVersionRef.current) {
          applyRemoteSession(remote);
        }
      } catch {
        // transient read while atomic rename completes
      }
    }, 400);
    return () => window.clearInterval(id);
  }, [applyRemoteSession, liveSessionFile]);

  // Persist user edits back to shared session (debounced, skipped when triggered by remote sync)
  useEffect(() => {
    if (!isTauriRuntime()) return;
    if (skipNextLocalSyncRef.current) {
      skipNextLocalSyncRef.current = false;
      return;
    }
    const id = window.setTimeout(() => {
      syncSessionState(cfdCase, guardrails, goal)
        .then((saved) => {
          lastVersionRef.current = saved.version;
          setSession(saved);
        })
        .catch(() => {});
    }, 320);
    return () => window.clearTimeout(id);
  }, [cfdCase, guardrails, goal]);

  // Recompute Rust telemetry, pre-flight audit & OpenFOAM dictionaries whenever case mutates
  useEffect(() => {
    let active = true;
    Promise.all([
      computeTelemetry(cfdCase),
      validateCase(cfdCase),
      generateCase(cfdCase),
    ])
      .then(([tel, val, gen]) => {
        if (!active) return;
        setTelemetry(tel);
        setValidation(val);
        setGenerated(gen);
      })
      .catch((err) => {
        if (active) setStatusMessage(`Rust engine error: ${err}`);
      });
    return () => {
      active = false;
    };
  }, [cfdCase]);

  // Listen to streaming solver telemetry from Rust Tokio runner
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listenSolverEvents((evt) => {
      if (disposed) return;
      if (evt.kind === "log" && evt.message) {
        setLogs((prev) => [...prev.slice(-399), evt.message!]);
      } else if (evt.kind === "residual" && evt.residual) {
        setResiduals((prev) => [...prev.slice(-399), evt.residual!]);
      } else if (evt.kind === "status" && evt.status) {
        setRunStatus(evt.status);
        if (evt.status !== "running") {
          setIsRunning(false);
          setStatusMessage(`Solver ${evt.status}`);
        }
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Listen to streaming AI Autopilot events from Rust LLM orchestrator
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listenAutopilotEvents((evt) => {
      if (disposed) return;
      setAutopilotLog((prev) => [...prev.slice(-199), evt]);
      if (evt.kind === "proposal" && evt.proposal) {
        setPendingProposal(evt.proposal);
      } else if (evt.kind === "tool_result" || evt.kind === "state_sync") {
        setPendingProposal(null);
        getSession()
          .then((s) => {
            if (!disposed) applyRemoteSession(s);
          })
          .catch(() => {});
      } else if (evt.kind === "done" || evt.kind === "error") {
        setAutopilotRunning(false);
        setPendingProposal(null);
        setStatusMessage(evt.message);
        getSession()
          .then((s) => {
            if (!disposed) applyRemoteSession(s);
          })
          .catch(() => {});
      }
    }).then((fn) => {
      if (disposed) fn();
      else unlisten = fn;
    });
    return () => {
      disposed = true;
      if (unlisten) unlisten();
    };
  }, [applyRemoteSession]);

  const handleSelectPreset = useCallback((presetId: string) => {
    const found = CASE_PRESETS.find((p) => p.id === presetId);
    if (!found) return;
    setCfdCase(found.build());
    rawStlMeshRef.current = null;
    setStlMesh(null);
    setResiduals([]);
    setRunStatus("idle");
    setStatusMessage(`Loaded template: ${found.label}`);
  }, []);

  const handleFlavorChange = useCallback((flavor: OpenFoamFlavor) => {
    setCfdCase((prev) => ({ ...prev, flavor }));
    setStatusMessage(
      flavor === "foundationV14"
        ? "Switched dictionary format to OpenFOAM Foundation v14 (foamRun)"
        : "Switched dictionary format to OpenCFD v2606 (simpleFoam/pimpleFoam)"
    );
  }, []);

  const handleImportStl = useCallback(async () => {
    try {
      const payload = await importStlWithDialog();
      if (!payload) return;
      const normalizedSummary = {
        ...payload.summary,
        scaleFactor: payload.summary.scaleFactor ?? 1.0,
      };
      const normalizedPayload: StlMeshPayload = {
        ...payload,
        summary: normalizedSummary,
      };
      rawStlMeshRef.current = normalizedPayload;
      setStlMesh(normalizedPayload);
      setCfdCase((prev) => ({
        ...prev,
        geometryKind: "customStl",
        parametric: null,
        stl: normalizedSummary,
        mesh: { ...prev.mesh, snappyEnabled: true },
        boundaries: prev.boundaries.some((b) => b.face === "obstacle")
          ? prev.boundaries
          : [
              ...prev.boundaries,
              {
                name: normalizedSummary.filename
                  .replace(/\.stl$/i, "")
                  .replace(/[^A-Za-z0-9_]/g, "_"),
                face: "obstacle",
                kind: "noSlipWall",
                velocity: [0, 0, 0],
                pressure: 0,
                wallRoughnessKs: 0,
              },
            ],
      }));
      const [minX, minY, minZ] = normalizedSummary.boundsMin;
      const [maxX, maxY, maxZ] = normalizedSummary.boundsMax;
      const maxDim = Math.max(Math.abs(maxX - minX), Math.abs(maxY - minY), Math.abs(maxZ - minZ));
      setStatusMessage(
        maxDim > 50
          ? `Loaded STL ${normalizedSummary.filename} (max ${maxDim.toFixed(1)} m — verifica scala mm → m nel pannello Domain!)`
          : `Loaded STL ${normalizedSummary.filename} (${normalizedSummary.triangleCount.toLocaleString()} triangles)`
      );
    } catch (e) {
      setStatusMessage(`STL import failed: ${e}`);
    }
  }, []);

  const handleScaleStl = useCallback((scaleFactor: number, autoFitDomain: boolean) => {
    if (!Number.isFinite(scaleFactor) || scaleFactor <= 0) return;

    const raw = rawStlMeshRef.current;
    if (raw) {
      const scaledPositions = raw.positions.map((v) => v * scaleFactor);
      const bMin: [number, number, number] = [
        raw.summary.boundsMin[0] * scaleFactor,
        raw.summary.boundsMin[1] * scaleFactor,
        raw.summary.boundsMin[2] * scaleFactor,
      ];
      const bMax: [number, number, number] = [
        raw.summary.boundsMax[0] * scaleFactor,
        raw.summary.boundsMax[1] * scaleFactor,
        raw.summary.boundsMax[2] * scaleFactor,
      ];
      const scaledSummary = {
        ...raw.summary,
        scaleFactor,
        boundsMin: bMin,
        boundsMax: bMax,
        surfaceArea: raw.summary.surfaceArea * scaleFactor * scaleFactor,
        volumeEst: raw.summary.volumeEst * scaleFactor * scaleFactor * scaleFactor,
      };
      setStlMesh({
        summary: scaledSummary,
        positions: scaledPositions,
        normals: raw.normals,
      });

      const sx = Math.max(0.01, Math.abs(bMax[0] - bMin[0]));
      const sy = Math.max(0.01, Math.abs(bMax[1] - bMin[1]));
      const sz = Math.max(0.01, Math.abs(bMax[2] - bMin[2]));

      setCfdCase((prev) => ({
        ...prev,
        stl: scaledSummary,
        domain: autoFitDomain
          ? {
              xMin: Number((bMin[0] - 3.0 * sx).toFixed(4)),
              xMax: Number((bMax[0] + 6.0 * sx).toFixed(4)),
              yMin: Number((bMin[1] - 2.5 * sy).toFixed(4)),
              yMax: Number((bMax[1] + 2.5 * sy).toFixed(4)),
              zMin: Number((bMin[2] - 2.0 * sz).toFixed(4)),
              zMax: Number((bMax[2] + 2.0 * sz).toFixed(4)),
            }
          : prev.domain,
        physics: autoFitDomain
          ? { ...prev.physics, referenceLength: Number(sx.toPrecision(4)) }
          : prev.physics,
      }));
      setStatusMessage(
        `Scala CAD applicata (×${Number(scaleFactor.toPrecision(5))}): ${sx.toFixed(3)} × ${sy.toFixed(3)} × ${sz.toFixed(3)} m`
      );
      return;
    }

    // Fallback when STL metadata exists in session without raw vertex buffers
    setCfdCase((prev) => {
      if (!prev.stl) return prev;
      const oldScale = prev.stl.scaleFactor && prev.stl.scaleFactor > 0 ? prev.stl.scaleFactor : 1.0;
      const ratio = scaleFactor / oldScale;
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
      const sx = Math.max(0.01, Math.abs(bMax[0] - bMin[0]));
      const sy = Math.max(0.01, Math.abs(bMax[1] - bMin[1]));
      const sz = Math.max(0.01, Math.abs(bMax[2] - bMin[2]));
      return {
        ...prev,
        stl: {
          ...prev.stl,
          scaleFactor,
          boundsMin: bMin,
          boundsMax: bMax,
          surfaceArea: prev.stl.surfaceArea * ratio * ratio,
          volumeEst: prev.stl.volumeEst * ratio * ratio * ratio,
        },
        domain: autoFitDomain
          ? {
              xMin: Number((bMin[0] - 3.0 * sx).toFixed(4)),
              xMax: Number((bMax[0] + 6.0 * sx).toFixed(4)),
              yMin: Number((bMin[1] - 2.5 * sy).toFixed(4)),
              yMax: Number((bMax[1] + 2.5 * sy).toFixed(4)),
              zMin: Number((bMin[2] - 2.0 * sz).toFixed(4)),
              zMax: Number((bMax[2] + 2.0 * sz).toFixed(4)),
            }
          : prev.domain,
        physics: autoFitDomain
          ? { ...prev.physics, referenceLength: Number(sx.toPrecision(4)) }
          : prev.physics,
      };
    });
    setStatusMessage(`Scala CAD applicata (×${Number(scaleFactor.toPrecision(5))})`);
  }, []);

  const handleGenerateParametric = useCallback(async (spec: ParametricGeometrySpec) => {
    try {
      const payload = await generateParametricGeometry(spec);
      rawStlMeshRef.current = payload;
      setStlMesh(payload);
      const [minX, minY, minZ] = payload.summary.boundsMin;
      const [maxX, maxY, maxZ] = payload.summary.boundsMax;
      const sx = Math.max(0.1, maxX - minX);
      const sy = Math.max(0.1, maxY - minY);
      const sz = Math.max(0.1, maxZ - minZ);
      setCfdCase((prev) => ({
        ...prev,
        geometryKind: "parametric",
        parametric: spec,
        stl: payload.summary,
        domain:
          spec.family === "venturiTube"
            ? {
                xMin: Number(minX.toFixed(3)),
                xMax: Number(maxX.toFixed(3)),
                yMin: Number((minY - 0.1 * sy).toFixed(3)),
                yMax: Number((maxY + 0.1 * sy).toFixed(3)),
                zMin: Number((minZ - 0.1 * sz).toFixed(3)),
                zMax: Number((maxZ + 0.1 * sz).toFixed(3)),
              }
            : {
                xMin: Number((minX - 2.5 * sx).toFixed(3)),
                xMax: Number((maxX + 5.5 * sx).toFixed(3)),
                yMin: Number((minY - 2.0 * Math.max(sy, sx * 0.5)).toFixed(3)),
                yMax: Number((maxY + 2.0 * Math.max(sy, sx * 0.5)).toFixed(3)),
                zMin: Number((minZ - 0.4 * sz).toFixed(3)),
                zMax: Number((maxZ + 0.4 * sz).toFixed(3)),
              },
        physics: {
          ...prev.physics,
          referenceLength: Number(sx.toFixed(4)),
        },
        mesh: { ...prev.mesh, snappyEnabled: true },
        boundaries: prev.boundaries.some((b) => b.face === "obstacle")
          ? prev.boundaries
          : [
              ...prev.boundaries,
              {
                name: "parametricBody",
                face: "obstacle",
                kind: "noSlipWall",
                velocity: [0, 0, 0],
                pressure: 0,
                wallRoughnessKs: 0,
              },
            ],
      }));
      setStatusMessage(
        `Generated watertight parametric STL (${payload.summary.triangleCount.toLocaleString()} tris, area ${payload.summary.surfaceArea.toFixed(3)} m²)`
      );
    } catch (e) {
      setStatusMessage(`Parametric generation failed: ${e}`);
    }
  }, []);

  const handleImportCase = useCallback(async () => {
    try {
      const imported = await importCaseWithDialog();
      if (!imported) return;
      setCfdCase(imported);
      setStlMesh(null);
      setStatusMessage(`Imported OpenFOAM case: ${imported.name}`);
    } catch (e) {
      setStatusMessage(`Case import failed: ${e}`);
    }
  }, []);

  const handleExportCase = useCallback(async () => {
    try {
      const res = await exportCaseWithDialog(cfdCase);
      if (!res) return;
      setStatusMessage(`Exported ${res.filesWritten} OpenFOAM files to ${res.dir}`);
    } catch (e) {
      setStatusMessage(`Export failed: ${e}`);
    }
  }, [cfdCase]);

  const handleStartSolver = useCallback(async () => {
    setActiveTab("run");
    setResiduals([]);
    setLogs([]);
    setIsRunning(true);
    setRunStatus("running");
    setStatusMessage(`Running solver via ${backend} backend…`);
    try {
      await startSolver(cfdCase, backend);
    } catch (e) {
      setIsRunning(false);
      setRunStatus("error");
      setLogs((prev) => [...prev, `[ERROR] ${e}`]);
      setStatusMessage(`Run failed: ${e}`);
    }
  }, [cfdCase, backend]);

  const handleStopSolver = useCallback(async () => {
    await stopSolver();
    setStatusMessage("Stop signal sent to solver");
  }, []);

  // Autopilot handlers
  const handleStartLlmAutopilot = useCallback(
    async (cfg: LlmConfig, g: OptimizationGoal) => {
      setAutopilotLog([]);
      setPendingProposal(null);
      setAutopilotRunning(true);
      setStatusMessage(`Launching ${cfg.provider} LLM autopilot (${cfg.mode} mode)…`);
      try {
        await syncSessionState(cfdCase, guardrails, g);
        await startAutopilot(cfg, g);
      } catch (e) {
        setAutopilotRunning(false);
        setStatusMessage(`Autopilot error: ${e}`);
      }
    },
    [cfdCase, guardrails]
  );

  const handleStartBuiltinLoop = useCallback(
    async (g: OptimizationGoal, runBackend: string) => {
      setAutopilotLog([]);
      setPendingProposal(null);
      setAutopilotRunning(true);
      setStatusMessage("Running built-in closed-loop MCP optimization…");
      try {
        await syncSessionState(cfdCase, guardrails, g);
        await startBuiltinAutopilot(g, runBackend);
      } catch (e) {
        setAutopilotRunning(false);
        setStatusMessage(`Built-in loop error: ${e}`);
      }
    },
    [cfdCase, guardrails]
  );

  const handleApproveProposal = useCallback(async (approved: boolean) => {
    await approveProposal(approved);
    setPendingProposal(null);
  }, []);

  const handleStopAutopilot = useCallback(async () => {
    await stopAutopilot();
    setAutopilotRunning(false);
    setPendingProposal(null);
    setStatusMessage("Autopilot stopped");
  }, []);

  const handleRollback = useCallback(
    async (iterationIndex?: number) => {
      try {
        const updated = await rollbackIteration(iterationIndex);
        applyRemoteSession(updated);
        setStatusMessage(
          `Rolled back to iteration #${iterationIndex ?? updated.bestIterationIndex ?? 0}`
        );
      } catch (e) {
        setStatusMessage(`Rollback failed: ${e}`);
      }
    },
    [applyRemoteSession]
  );

  const autorunStartedRef = useRef(false);
  useEffect(() => {
    if (initialParams?.get("autorun") === "1" && !autorunStartedRef.current) {
      const id = window.setTimeout(() => {
        if (autorunStartedRef.current) return;
        autorunStartedRef.current = true;
        handleStartSolver();
      }, 220);
      return () => window.clearTimeout(id);
    }
  }, [handleStartSolver]);

  const header = TAB_HEADERS[activeTab];

  return (
    <div className="app-shell">
      <TitleBar
        cfdCase={cfdCase}
        env={env}
        isRunning={isRunning}
        guidedMode={guidedMode}
        onToggleGuidedMode={handleToggleGuidedMode}
        onCaseNameChange={(name) => setCfdCase((prev) => ({ ...prev, name }))}
        onFlavorChange={handleFlavorChange}
        onSelectPreset={handleSelectPreset}
        onImportCase={handleImportCase}
        onExportCase={handleExportCase}
        onQuickRun={handleStartSolver}
        onStopRun={handleStopSolver}
      />

      <main className="studio-body">
        <WorkflowRail
          activeTab={activeTab}
          onSelect={setActiveTab}
          validation={validation}
          isRunning={isRunning}
          iterationCount={session?.iterations.length}
          autopilotRunning={autopilotRunning}
        />

        <aside className="inspector-pane" aria-label={header.title}>
          <div className="inspector-header">
            <div>
              <h2>{header.title}</h2>
              <div className="inspector-subtitle">{header.subtitle}</div>
            </div>
            <span className="pill" style={{ fontSize: 9.5 }}>
              {activeTab.toUpperCase()}
            </span>
          </div>

          <div className="inspector-scroll">
            {guidedMode && (
              <GuidedAssistant
                cfdCase={cfdCase}
                activeTab={activeTab}
                telemetry={telemetry}
                validation={validation}
                isRunning={isRunning}
                onSelectTab={setActiveTab}
                onSelectPreset={handleSelectPreset}
                onImportStl={handleImportStl}
                onChangeCase={setCfdCase}
                onHighlightFace={setHighlightedFace}
                onExportCase={handleExportCase}
                onStartSolver={handleStartSolver}
                onDisableGuided={handleToggleGuidedMode}
              />
            )}
            {activeTab === "geometry" && (
              <GeometryPanel
                cfdCase={cfdCase}
                onChange={setCfdCase}
                onImportStl={handleImportStl}
                onGenerateParametric={handleGenerateParametric}
                onScaleStl={handleScaleStl}
              />
            )}
            {activeTab === "mesh" && (
              <MeshPanel cfdCase={cfdCase} telemetry={telemetry} onChange={setCfdCase} />
            )}
            {activeTab === "physics" && (
              <PhysicsPanel cfdCase={cfdCase} onChange={setCfdCase} />
            )}
            {activeTab === "boundaries" && (
              <BoundariesPanel
                cfdCase={cfdCase}
                highlightedFace={highlightedFace}
                onHighlightFace={setHighlightedFace}
                onChange={setCfdCase}
              />
            )}
            {activeTab === "numerics" && (
              <NumericsPanel
                cfdCase={cfdCase}
                maxCores={env?.cpuCores ?? 16}
                onChange={setCfdCase}
              />
            )}
            {activeTab === "lab" && (
              <EngineeringLabPanel
                cfdCase={cfdCase}
                telemetry={telemetry}
                onChange={setCfdCase}
              />
            )}
            {activeTab === "run" && (
              <RunMonitorPanel
                cfdCase={cfdCase}
                env={env}
                validation={validation}
                backend={backend}
                onBackendChange={setBackend}
                isRunning={isRunning}
                runStatus={runStatus}
                residuals={residuals}
                onStart={handleStartSolver}
                onStop={handleStopSolver}
              />
            )}
            {activeTab === "autopilot" && (
              <AutopilotPanel
                session={session}
                env={env}
                autopilotRunning={autopilotRunning}
                autopilotLog={autopilotLog}
                pendingProposal={pendingProposal}
                mcpSnippet={mcpSnippet}
                onUpdateGuardrails={setGuardrails}
                onUpdateGoal={setGoal}
                onStartLlmAutopilot={handleStartLlmAutopilot}
                onStartBuiltinLoop={handleStartBuiltinLoop}
                onApproveProposal={handleApproveProposal}
                onStopAutopilot={handleStopAutopilot}
                onRollback={handleRollback}
              />
            )}
          </div>
        </aside>

        <section className="stage-column">
          <Viewport3D
            cfdCase={cfdCase}
            stlMesh={stlMesh}
            telemetry={telemetry}
            validation={validation}
            highlightedFace={highlightedFace}
            onJumpToAudit={() => setActiveTab("run")}
          />
          <DictionaryDrawer
            generated={generated}
            logs={logs}
            onExportCase={handleExportCase}
          />
        </section>
      </main>

      <footer className="statusbar">
        <div style={{ display: "flex", alignItems: "center", gap: 12, minWidth: 0 }}>
          <span style={{ color: "var(--accent-amber)", flexShrink: 0 }}>
            ● {isTauriRuntime() ? "NATIVE RUST CORE" : liveSessionFile ? "LIVE MCP SYNC" : "WEB PREVIEW"}
          </span>
          <span style={{ flexShrink: 0 }}>Solver: {telemetry?.solverName ?? "—"}</span>
          <span
            style={{
              color: "var(--text-secondary)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {statusMessage}
          </span>
        </div>
        <div style={{ flexShrink: 0 }}>
          Venturi CFD v0.3 · MIT Stack · MCP Server Ready · Compatible with OpenFOAM®
        </div>
      </footer>
    </div>
  );
};

export default App;
