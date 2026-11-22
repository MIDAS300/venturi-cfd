import React, { useMemo, useState } from "react";
import {
  FileCode2,
  Terminal,
  Copy,
  Check,
  ChevronDown,
  ChevronUp,
  Download,
} from "lucide-react";
import { GeneratedCaseFiles } from "../types/cfd";

interface DictionaryDrawerProps {
  generated: GeneratedCaseFiles | null;
  logs: string[];
  onExportCase: () => void;
}

function highlightOpenFoamLine(line: string, idx: number): React.ReactNode {
  const trimmed = line.trim();
  if (
    trimmed.startsWith("//") ||
    trimmed.startsWith("/*") ||
    trimmed.startsWith("*") ||
    trimmed.startsWith("\\") ||
    trimmed.startsWith("=") ||
    trimmed.startsWith("#")
  ) {
    return (
      <div key={idx} className="tok-comment">
        {line || " "}
      </div>
    );
  }

  // Highlight key + values on dictionary lines
  const kvMatch = line.match(/^(\s*)([A-Za-z0-9_.\-"]+)(.*)$/);
  if (kvMatch) {
    const [, indent, key, rest] = kvMatch;
    const isBlockKeyword = [
      "FoamFile",
      "boundaryField",
      "solvers",
      "SIMPLE",
      "PIMPLE",
      "relaxationFactors",
      "functions",
      "ddtSchemes",
      "gradSchemes",
      "divSchemes",
      "laplacianSchemes",
      "vertices",
      "blocks",
      "boundary",
      "geometry",
      "RAS",
      "LES",
    ].includes(key);

    return (
      <div key={idx}>
        {indent}
        <span className={isBlockKeyword ? "tok-keyword" : "tok-key"}>{key}</span>
        <span>{rest}</span>
      </div>
    );
  }

  return <div key={idx}>{line || " "}</div>;
}

export const DictionaryDrawer: React.FC<DictionaryDrawerProps> = ({
  generated,
  logs,
  onExportCase,
}) => {
  const [open, setOpen] = useState(true);
  const [drawerTab, setDrawerTab] = useState<"files" | "console">(() => {
    const p = typeof window !== "undefined" ? new URLSearchParams(window.location.search) : null;
    return p?.get("drawer") === "console" ? "console" : "files";
  });
  const [selectedFile, setSelectedFile] = useState<string>("system/controlDict");
  const [copied, setCopied] = useState(false);

  const fileList = useMemo(() => {
    if (!generated) return [];
    return Object.keys(generated.files);
  }, [generated]);

  const activeFile =
    generated && generated.files[selectedFile]
      ? selectedFile
      : fileList[0] ?? "system/controlDict";

  const content = generated?.files[activeFile] ?? "// Generating OpenFOAM dictionaries via Rust…";

  const handleCopy = async () => {
    await navigator.clipboard.writeText(content);
    setCopied(true);
    setTimeout(() => setCopied(false), 1400);
  };

  return (
    <section
      className="bottom-drawer"
      style={{ height: open ? 236 : 34 }}
      aria-label="Live OpenFOAM Dictionary & Console Drawer"
    >
      <div className="drawer-header">
        <div className="drawer-tabs">
          <button
            type="button"
            className={`drawer-tab ${drawerTab === "files" ? "active" : ""}`}
            onClick={() => {
              setDrawerTab("files");
              setOpen(true);
            }}
          >
            <FileCode2 size={13} />
            Live OpenFOAM Dictionaries
            <span className="mono" style={{ fontSize: 10, color: "var(--text-muted)" }}>
              ({fileList.length})
            </span>
          </button>
          <button
            type="button"
            className={`drawer-tab ${drawerTab === "console" ? "active" : ""}`}
            onClick={() => {
              setDrawerTab("console");
              setOpen(true);
            }}
          >
            <Terminal size={13} />
            Solver Console
            {logs.length > 0 && (
              <span className="mono" style={{ fontSize: 10, color: "var(--cfd-inlet)" }}>
                ({logs.length})
              </span>
            )}
          </button>
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          {drawerTab === "files" && open && (
            <>
              <button type="button" className="btn btn-xs btn-subtle" onClick={handleCopy}>
                {copied ? <Check size={11} /> : <Copy size={11} />}
                {copied ? "Copied" : "Copy File"}
              </button>
              <button type="button" className="btn btn-xs" onClick={onExportCase}>
                <Download size={11} />
                Export All
              </button>
            </>
          )}
          <button
            type="button"
            className="btn btn-xs btn-subtle"
            onClick={() => setOpen((o) => !o)}
            title={open ? "Collapse drawer" : "Expand drawer"}
          >
            {open ? <ChevronDown size={13} /> : <ChevronUp size={13} />}
          </button>
        </div>
      </div>

      {open && drawerTab === "files" && (
        <div className="drawer-body">
          <div className="file-tree">
            {fileList.map((fname) => (
              <button
                key={fname}
                type="button"
                className={`file-item ${fname === activeFile ? "active" : ""}`}
                onClick={() => setSelectedFile(fname)}
              >
                <FileCode2 size={11} style={{ flexShrink: 0, opacity: 0.7 }} />
                <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {fname}
                </span>
              </button>
            ))}
          </div>
          <pre className="code-viewer">
            {content.split("\n").map((line, i) => highlightOpenFoamLine(line, i))}
          </pre>
        </div>
      )}

      {open && drawerTab === "console" && (
        <div className="drawer-body">
          <pre className="code-viewer" style={{ width: "100%" }}>
            {logs.length === 0
              ? "// Ready. Press 'Run Case' to launch OpenFOAM (Local / Docker) or Benchmark Convergence Preview."
              : logs.map((l, i) => (
                  <div
                    key={i}
                    style={{
                      color: l.includes("[ERROR]")
                        ? "#fda4af"
                        : l.includes("converged") || l.includes("completed")
                          ? "#34d399"
                          : l.startsWith("[Venturi]")
                            ? "#fbbf24"
                            : "#cbd5e1",
                    }}
                  >
                    {l}
                  </div>
                ))}
          </pre>
        </div>
      )}
    </section>
  );
};
