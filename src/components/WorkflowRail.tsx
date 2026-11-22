import React from "react";
import {
  Box,
  Grid3x3,
  Wind,
  Layers,
  SlidersHorizontal,
  Calculator,
  Activity,
  Sparkles,
} from "lucide-react";
import { ValidationReport, WorkflowTab } from "../types/cfd";

interface WorkflowRailProps {
  activeTab: WorkflowTab;
  onSelect: (tab: WorkflowTab) => void;
  validation: ValidationReport | null;
  isRunning: boolean;
  iterationCount?: number;
  autopilotRunning?: boolean;
}

const ITEMS: { id: WorkflowTab; label: string; icon: React.FC<{ size?: number }> }[] = [
  { id: "geometry", label: "Domain", icon: Box },
  { id: "mesh", label: "Mesh", icon: Grid3x3 },
  { id: "physics", label: "Physics", icon: Wind },
  { id: "boundaries", label: "Bounds", icon: Layers },
  { id: "numerics", label: "Numerics", icon: SlidersHorizontal },
  { id: "lab", label: "Eng Lab", icon: Calculator },
  { id: "run", label: "Solve", icon: Activity },
  { id: "autopilot", label: "AI MCP", icon: Sparkles },
];

export const WorkflowRail: React.FC<WorkflowRailProps> = ({
  activeTab,
  onSelect,
  validation,
  isRunning,
  iterationCount = 0,
  autopilotRunning = false,
}) => {
  return (
    <nav className="workflow-rail" aria-label="CFD Workflow Steps">
      {ITEMS.map((item) => {
        const Icon = item.icon;
        const active = activeTab === item.id;
        const showError = item.id === "run" && validation && validation.errorCount > 0;
        const showWarn =
          item.id === "run" && validation && validation.errorCount === 0 && validation.warningCount > 0;
        const showAutopilotRun = item.id === "autopilot" && autopilotRunning;
        const showAutopilotCount = item.id === "autopilot" && !autopilotRunning && iterationCount > 0;

        return (
          <button
            key={item.id}
            type="button"
            className={`rail-btn ${active ? "active" : ""}`}
            onClick={() => onSelect(item.id)}
            title={item.label}
          >
            <Icon size={18} />
            <span className="rail-label">{item.label}</span>

            {showError && (
              <span
                className="rail-badge"
                style={{ background: "var(--cfd-wall)", color: "#fff" }}
              >
                {validation.errorCount}
              </span>
            )}
            {showWarn && (
              <span
                className="rail-badge"
                style={{ background: "var(--accent-amber)", color: "#0b0e14" }}
              >
                {validation.warningCount}
              </span>
            )}
            {item.id === "run" && isRunning && (
              <span
                className="rail-badge"
                style={{ background: "var(--cfd-outlet)", color: "#0b0e14" }}
              >
                ●
              </span>
            )}
            {showAutopilotRun && (
              <span
                className="rail-badge"
                style={{ background: "var(--accent-amber)", color: "#0b0e14" }}
                title="AI Autopilot Running"
              >
                ●
              </span>
            )}
            {showAutopilotCount && (
              <span
                className="rail-badge"
                style={{ background: "var(--cfd-inlet)", color: "#090c11" }}
                title={`${iterationCount} Optimization Iteration(s)`}
              >
                {iterationCount}
              </span>
            )}
          </button>
        );
      })}
    </nav>
  );
};
