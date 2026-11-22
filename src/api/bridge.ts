import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AutopilotEvent,
  CfdCase,
  CflInput,
  CflResult,
  EnvironmentStatus,
  FluidConfig,
  GeneratedCaseFiles,
  GradientHotspot,
  GuardrailsConfig,
  IterationRecord,
  LlmConfig,
  OpenFoamResultSummary,
  OptimizationGoal,
  ParametricGeometrySpec,
  PendingProposal,
  RefinementBox,
  SessionState,
  SolverEvent,
  StlMeshPayload,
  TelemetrySnapshot,
  TurbulenceIcInput,
  TurbulenceIcResult,
  ValidationReport,
  YPlusInput,
  YPlusResult,
} from "../types/cfd";
import { CASE_PRESETS } from "../data/presets";

export function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export async function computeTelemetry(cfdCase: CfdCase): Promise<TelemetrySnapshot> {
  if (isTauriRuntime()) {
    return invoke<TelemetrySnapshot>("cmd_compute_telemetry", { case: cfdCase });
  }
  // Browser-only dev fallback mirroring Rust physics.rs
  const nu = Math.max(cfdCase.physics.fluid.kinematicViscosity, 1e-12);
  const u = Math.max(Math.abs(cfdCase.physics.referenceVelocity), 1e-6);
  const l = Math.max(Math.abs(cfdCase.physics.referenceLength), 1e-6);
  const reynolds = (u * l) / nu;
  const c = cfdCase.physics.fluid.isGas
    ? Math.sqrt(
        Math.max(1, cfdCase.physics.fluid.gamma * cfdCase.physics.fluid.gasConstant * cfdCase.physics.fluid.temperatureK)
      )
    : 1482;
  const mach = u / c;
  const base = cfdCase.mesh.cellsX * cfdCase.mesh.cellsY * cfdCase.mesh.cellsZ;
  const boxExtra = cfdCase.mesh.refinementBoxes.reduce((acc, b) => acc + Math.pow(2, Math.min(4, b.level)) * 1800, 0);
  const estimatedCells = cfdCase.mesh.snappyEnabled
    ? Math.round(base * (1 + 0.35 * Math.pow(2, Math.min(cfdCase.mesh.refinementMax, 5))) + boxExtra)
    : base;
  const lx = Math.max(1e-6, Math.abs(cfdCase.domain.xMax - cfdCase.domain.xMin));
  const ly = Math.max(1e-6, Math.abs(cfdCase.domain.yMax - cfdCase.domain.yMin));
  const lz = Math.max(1e-6, Math.abs(cfdCase.domain.zMax - cfdCase.domain.zMin));
  let minDx = Math.min(lx / Math.max(1, cfdCase.mesh.cellsX), ly / Math.max(1, cfdCase.mesh.cellsY), lz / Math.max(1, cfdCase.mesh.cellsZ));
  if (cfdCase.mesh.snappyEnabled) {
    const maxBoxLevel = cfdCase.mesh.refinementBoxes.reduce((m, b) => Math.max(m, b.level), 0);
    minDx /= Math.pow(2, Math.min(6, Math.max(cfdCase.mesh.refinementMax, maxBoxLevel)));
  }
  return {
    reynolds,
    mach,
    speedOfSound: c,
    estimatedCells,
    estimatedRamMb: 45 + estimatedCells * 0.00145,
    minDx,
    estimatedCourant: cfdCase.physics.regime === "transientIncompressible" ? (u * cfdCase.numerics.deltaT) / minDx : 0,
    firstCellHeightMm: cfdCase.mesh.firstLayerThickness * 1000,
    recommendedCores: Math.min(64, Math.max(1, Math.floor(estimatedCells / 50000))),
    compressibleWarning: cfdCase.physics.fluid.isGas && mach > 0.3 && cfdCase.physics.regime !== "steadyCompressible",
    solverName:
      cfdCase.flavor === "foundationV14"
        ? cfdCase.physics.regime === "steadyCompressible"
          ? "foamRun -solver fluid"
          : "foamRun -solver incompressibleFluid"
        : cfdCase.physics.regime === "transientIncompressible"
          ? "pimpleFoam"
          : cfdCase.physics.regime === "steadyCompressible"
            ? "rhoSimpleFoam"
            : "simpleFoam",
  };
}

export async function computeYPlus(input: YPlusInput): Promise<YPlusResult> {
  if (isTauriRuntime()) {
    return invoke<YPlusResult>("cmd_compute_y_plus", { input });
  }
  const nu = input.dynamicViscosity / input.density;
  const re = (input.velocity * input.referenceLength) / nu;
  const cf = re < 5e5 ? 0.664 / Math.sqrt(re) : Math.pow(2 * Math.log10(re) - 0.65, -2.3);
  const d99 = re < 5e5 ? (4.91 * input.referenceLength) / Math.sqrt(re) : (0.37 * input.referenceLength) / Math.pow(re, 0.2);
  const tauW = 0.5 * cf * input.density * input.velocity * input.velocity;
  const uTau = Math.sqrt(tauW / input.density);
  const yc = (input.targetYPlus * nu) / uTau;
  const dy1 = 2 * yc;
  const r = input.expansionRatio;
  const layers = Math.min(35, Math.max(3, Math.ceil(Math.log(1 + (d99 / dy1) * (r - 1)) / Math.log(r))));
  return {
    reynolds: re,
    flowRegimeLabel: re < 5e5 ? "Laminar (Blasius)" : "Fully Turbulent (Schlichting)",
    skinFrictionCf: cf,
    wallShearTauW: tauW,
    frictionVelocityUTau: uTau,
    firstCellHeight: dy1,
    wallDistanceYc: yc,
    boundaryLayerDelta99: d99,
    recommendedLayers: layers,
    totalPrismHeight: (dy1 * (Math.pow(r, layers) - 1)) / (r - 1),
    wallTreatmentNote:
      input.targetYPlus > 5 && input.targetYPlus < 30
        ? "WARNING: Buffer layer (5 < y+ < 30)."
        : input.targetYPlus <= 5
          ? "Low-Re wall-resolved (viscous sublayer)."
          : "Log-law region (wall functions).",
    bufferLayerWarning: input.targetYPlus > 5 && input.targetYPlus < 30,
  };
}

export async function computeTurbulenceIc(input: TurbulenceIcInput): Promise<TurbulenceIcResult> {
  if (isTauriRuntime()) {
    return invoke<TurbulenceIcResult>("cmd_compute_turbulence_ic", { input });
  }
  const cMu = 0.09;
  const i = input.intensityPercent / 100;
  const k = 1.5 * Math.pow(input.velocity * i, 2);
  const epsilon = (Math.pow(cMu, 0.75) * Math.pow(k, 1.5)) / input.lengthScale;
  const omega = Math.sqrt(k) / (Math.pow(cMu, 0.25) * input.lengthScale);
  const nut = (cMu * k * k) / epsilon;
  const nuTilda = Math.sqrt(1.5) * input.velocity * i * input.lengthScale;
  return { k, epsilon, omega, nut, nuTilda, viscosityRatio: nut / input.kinematicViscosity };
}

export async function computeCfl(input: CflInput): Promise<CflResult> {
  if (isTauriRuntime()) {
    return invoke<CflResult>("cmd_compute_cfl", { input });
  }
  const maxDeltaT = (input.targetCourant * input.minCellSize) / input.maxVelocity;
  const flowThroughTime = input.domainLength / input.maxVelocity;
  const recommendedEndTime = flowThroughTime * 5;
  return {
    maxDeltaT,
    flowThroughTime,
    recommendedEndTime,
    estimatedSteps: Math.ceil(recommendedEndTime / maxDeltaT),
    stabilityNote: input.targetCourant <= 1 ? "Time-accurate PISO/PIMPLE regime." : "Implicit PIMPLE regime.",
  };
}

export async function computeAirSutherland(temperatureK: number, pressurePa: number): Promise<FluidConfig> {
  if (isTauriRuntime()) {
    return invoke<FluidConfig>("cmd_compute_air_sutherland", { temperatureK, pressurePa });
  }
  const rAir = 287.058;
  const density = pressurePa / (rAir * temperatureK);
  const mu = 1.716e-5 * Math.pow(temperatureK / 273.15, 1.5) * ((273.15 + 110.4) / (temperatureK + 110.4));
  return {
    name: `Air (${(temperatureK - 273.15).toFixed(1)} °C, ${(pressurePa / 1000).toFixed(1)} kPa)`,
    density,
    dynamicViscosity: mu,
    kinematicViscosity: mu / density,
    temperatureK,
    pressurePa,
    gamma: 1.4,
    gasConstant: rAir,
    isGas: true,
  };
}

// Browser preview listeners for benchmark simulation when outside Tauri
const browserListeners = new Set<(e: SolverEvent) => void>();
let browserRunTimer: number | null = null;

function emitBrowserEvent(evt: SolverEvent) {
  browserListeners.forEach((fn) => fn(evt));
}

function previewHeader(cls: string, obj: string, loc: string): string {
  return `/*--------------------------------*- C++ -*----------------------------------*\\
  =========                 |
  \\\\      /  F ield         | Generated by Venturi CFD Studio (Rust)
   \\\\    /   O peration     | Compatible with OpenFOAM®
    \\\\  /    A nd           | Independent tool — not affiliated with OpenCFD Ltd
     \\\\/     M anipulation  |
\\*---------------------------------------------------------------------------*/
FoamFile
{
    format      ascii;
    class       ${cls};
    location    "${loc}";
    object      ${obj};
}
// * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * * //

`;
}

export async function generateCase(cfdCase: CfdCase): Promise<GeneratedCaseFiles> {
  if (isTauriRuntime()) {
    return invoke<GeneratedCaseFiles>("cmd_generate_case", { case: cfdCase });
  }
  const isFound = cfdCase.flavor === "foundationV14";
  const solverCommand = isFound
    ? cfdCase.physics.regime === "steadyCompressible"
      ? "foamRun -solver fluid"
      : "foamRun -solver incompressibleFluid"
    : cfdCase.physics.regime === "transientIncompressible"
      ? "pimpleFoam"
      : cfdCase.physics.regime === "steadyCompressible"
        ? "rhoSimpleFoam"
        : "simpleFoam";

  const app = isFound ? "foamRun" : solverCommand;
  const solverLine = isFound
    ? `solver          ${cfdCase.physics.regime === "steadyCompressible" ? "fluid" : "incompressibleFluid"};\n\n`
    : "";

  const files: Record<string, string> = {};
  files["system/controlDict"] =
    previewHeader("dictionary", "controlDict", "system") +
    `application     ${app};\n\n${solverLine}startFrom       startTime;\nstartTime       ${cfdCase.numerics.startTime};\nstopAt          endTime;\nendTime         ${cfdCase.numerics.endTime};\ndeltaT          ${cfdCase.numerics.deltaT};\nwriteControl    ${cfdCase.physics.regime === "transientIncompressible" ? "adjustableRunTime" : "timeStep"};\nwriteInterval   ${cfdCase.numerics.writeInterval};\npurgeWrite      5;\nwriteFormat     ascii;\nrunTimeModifiable true;\n\nfunctions\n{\n    yPlus\n    {\n        type            yPlus;\n        libs            ("libfieldFunctionObjects.so");\n        writeControl    writeTime;\n    }\n}\n`;

  files["system/fvSchemes"] =
    previewHeader("dictionary", "fvSchemes", "system") +
    `ddtSchemes      { default ${cfdCase.physics.regime === "transientIncompressible" ? "backward" : "steadyState"}; }\ngradSchemes     { default cellLimited Gauss linear 1; }\ndivSchemes\n{\n    default         none;\n    div(phi,U)      bounded Gauss linearUpwind grad(U);\n    div(phi,k)      bounded Gauss limitedLinear 1;\n    div(phi,omega)  bounded Gauss limitedLinear 1;\n    div((nuEff*dev2(T(grad(U))))) Gauss linear;\n}\nlaplacianSchemes { default Gauss linear corrected; }\n`;

  files["system/fvSolution"] =
    previewHeader("dictionary", "fvSolution", "system") +
    `solvers\n{\n    p { solver GAMG; smoother GaussSeidel; tolerance 1e-07; relTol 0.05; }\n    "(U|k|omega|epsilon)" { solver smoothSolver; smoother symGaussSeidel; tolerance 1e-07; relTol 0.1; }\n}\n\nSIMPLE\n{\n    nNonOrthogonalCorrectors ${cfdCase.numerics.nNonOrthoCorrectors};\n    consistent      yes;\n    residualControl { p ${cfdCase.numerics.convergenceTol.toExponential(1)}; U ${cfdCase.numerics.convergenceTol.toExponential(1)}; }\n}\n\nrelaxationFactors\n{\n    fields    { p ${cfdCase.numerics.relaxP.toFixed(2)}; }\n    equations { U ${cfdCase.numerics.relaxU.toFixed(2)}; "(k|omega|epsilon)" ${cfdCase.numerics.relaxTurb.toFixed(2)}; }\n}\n`;

  const d = cfdCase.domain;
  files["system/blockMeshDict"] =
    previewHeader("dictionary", "blockMeshDict", "system") +
    `scale 1;\nvertices\n(\n    (${d.xMin} ${d.yMin} ${d.zMin})\n    (${d.xMax} ${d.yMin} ${d.zMin})\n    (${d.xMax} ${d.yMax} ${d.zMin})\n    (${d.xMin} ${d.yMax} ${d.zMin})\n    (${d.xMin} ${d.yMin} ${d.zMax})\n    (${d.xMax} ${d.yMin} ${d.zMax})\n    (${d.xMax} ${d.yMax} ${d.zMax})\n    (${d.xMin} ${d.yMax} ${d.zMax})\n);\nblocks\n(\n    hex (0 1 2 3 4 5 6 7) (${cfdCase.mesh.cellsX} ${cfdCase.mesh.cellsY} ${cfdCase.mesh.cellsZ}) simpleGrading (1 ${cfdCase.mesh.gradingY} 1)\n);\n`;

  if (cfdCase.mesh.snappyEnabled) {
    const geomBoxes = cfdCase.mesh.refinementBoxes
      .map(
        (b) =>
          `    ${b.name}\n    {\n        type searchableBox;\n        min (${b.min[0]} ${b.min[1]} ${b.min[2]});\n        max (${b.max[0]} ${b.max[1]} ${b.max[2]});\n    }`
      )
      .join("\n");
    const refRegions = cfdCase.mesh.refinementBoxes
      .map((b) => `        ${b.name} { mode inside; levels ((1E15 ${b.level})); } // ${b.reason}`)
      .join("\n");
    files["system/snappyHexMeshDict"] =
      previewHeader("dictionary", "snappyHexMeshDict", "system") +
      `castellatedMesh true;\nsnap            true;\naddLayers       true;\n\ngeometry\n{\n${geomBoxes}\n}\n\ncastellatedMeshControls\n{\n    refinementRegions\n    {\n${refRegions}\n    }\n}\n\naddLayersControls\n{\n    relativeSizes       false;\n    expansionRatio      ${cfdCase.mesh.expansionRatio};\n    firstLayerThickness ${cfdCase.mesh.firstLayerThickness.toExponential(4)};\n    nSurfaceLayers      ${cfdCase.mesh.inflationLayers};\n}\n`;
  }
  if (cfdCase.numerics.parallelCores > 1) {
    files["system/decomposeParDict"] =
      previewHeader("dictionary", "decomposeParDict", "system") +
      `numberOfSubdomains ${cfdCase.numerics.parallelCores};\nmethod          scotch;\n`;
  }

  const propFile = isFound ? "constant/physicalProperties" : "constant/transportProperties";
  files[propFile] =
    previewHeader("dictionary", isFound ? "physicalProperties" : "transportProperties", "constant") +
    `// Fluid: ${cfdCase.physics.fluid.name}\nnu [0 2 -1 0 0 0 0] ${cfdCase.physics.fluid.kinematicViscosity.toExponential(4)};\n`;

  const turbFile = isFound ? "constant/momentumTransport" : "constant/turbulenceProperties";
  files[turbFile] =
    previewHeader("dictionary", isFound ? "momentumTransport" : "turbulenceProperties", "constant") +
    `simulationType ${cfdCase.physics.turbulence === "laminar" ? "laminar" : cfdCase.physics.turbulence === "waleLes" ? "LES" : "RAS"};\nRAS { ${isFound ? "model" : "RASModel"} ${cfdCase.physics.turbulence}; turbulence on; }\n`;

  files["0/U"] =
    previewHeader("volVectorField", "U", "0") +
    `dimensions      [0 1 -1 0 0 0 0];\ninternalField   uniform (${cfdCase.physics.referenceVelocity} 0 0);\nboundaryField\n{\n` +
    cfdCase.boundaries.map((b) => `    ${b.name} { type ${b.kind === "velocityInlet" ? "fixedValue" : b.kind === "pressureOutlet" ? "inletOutlet" : b.kind === "noSlipWall" ? "noSlip" : b.kind === "symmetry" ? "symmetryPlane" : b.kind === "empty2D" ? "empty" : "fixedValue"}; }\n`).join("") +
    `}\n`;

  files["0/p"] =
    previewHeader("volScalarField", "p", "0") +
    `dimensions      [0 2 -2 0 0 0 0];\ninternalField   uniform 0;\nboundaryField\n{\n` +
    cfdCase.boundaries.map((b) => `    ${b.name} { type ${b.kind === "pressureOutlet" ? "fixedValue" : b.kind === "symmetry" ? "symmetryPlane" : b.kind === "empty2D" ? "empty" : "zeroGradient"}; }\n`).join("") +
    `}\n`;

  if (cfdCase.physics.turbulence !== "laminar") {
    files["0/k"] = previewHeader("volScalarField", "k", "0") + `internalField uniform ${cfdCase.physics.turbulenceIc.k.toExponential(4)};\n`;
    files["0/omega"] = previewHeader("volScalarField", "omega", "0") + `internalField uniform ${cfdCase.physics.turbulenceIc.omega.toExponential(4)};\n`;
    files["0/nut"] = previewHeader("volScalarField", "nut", "0") + `internalField uniform ${cfdCase.physics.turbulenceIc.nut.toExponential(4)};\n`;
  }
  files["Allrun"] = `#!/bin/sh\nblockMesh\n${cfdCase.mesh.snappyEnabled ? "snappyHexMesh -overwrite\n" : ""}checkMesh\n${cfdCase.numerics.parallelCores > 1 ? `decomposePar -force\nmpirun -np ${cfdCase.numerics.parallelCores} ${solverCommand} -parallel\nreconstructPar\n` : `${solverCommand}\n`}touch case.foam\n`;

  return { files, solverCommand };
}

export async function validateCase(cfdCase: CfdCase): Promise<ValidationReport> {
  if (isTauriRuntime()) {
    return invoke<ValidationReport>("cmd_validate_case", { case: cfdCase });
  }
  const tel = await computeTelemetry(cfdCase);
  const items: ValidationReport["items"] = [];
  const lx = cfdCase.domain.xMax - cfdCase.domain.xMin;
  const ly = cfdCase.domain.yMax - cfdCase.domain.yMin;
  const lz = cfdCase.domain.zMax - cfdCase.domain.zMin;
  items.push({
    id: "domain_bounds",
    category: "Geometry",
    severity: lx > 0 && ly > 0 && lz > 0 ? "pass" : "error",
    title: `Domain geometry valid (${lx.toFixed(2)} × ${ly.toFixed(2)} × ${lz.toFixed(2)} m)`,
    detail: "Domain bounding box has strictly positive volume.",
    suggestion: null,
  });
  const inlets = cfdCase.boundaries.filter((b) => b.kind === "velocityInlet").length;
  const outlets = cfdCase.boundaries.filter((b) => b.kind === "pressureOutlet").length;
  items.push({
    id: "bc_well_posed",
    category: "Boundaries",
    severity: inlets > 0 && outlets === 0 ? "error" : "pass",
    title: inlets > 0 && outlets === 0 ? "Inlet defined without pressure outlet" : "Boundary conditions well-posed",
    detail: `${inlets} inlet(s), ${outlets} outlet(s), ${cfdCase.boundaries.length} total patches configured.`,
    suggestion: inlets > 0 && outlets === 0 ? "Assign at least one Pressure Outlet patch." : null,
  });
  const yp = cfdCase.mesh.targetYPlus;
  if (cfdCase.physics.turbulence !== "laminar") {
    const buf = yp > 5 && yp < 30;
    items.push({
      id: "yplus_check",
      category: "Mesh / y+",
      severity: buf ? "warning" : "pass",
      title: buf
        ? `Target y+ (${yp.toFixed(1)}) falls in buffer layer (5–30)`
        : `y+ target (${yp.toFixed(1)}) compatible with ${cfdCase.physics.turbulence}`,
      detail: `Estimated first-cell height Δy₁ ≈ ${tel.firstCellHeightMm.toFixed(4)} mm.`,
      suggestion: buf ? "Target y+ ≈ 1 (wall-resolved) or y+ = 35–100 (wall functions)." : null,
    });
  }
  if (tel.compressibleWarning) {
    items.push({
      id: "mach_warn",
      category: "Physics",
      severity: "warning",
      title: `Compressibility effects significant (Ma = ${tel.mach.toFixed(2)})`,
      detail: "Gas flow exceeds Ma = 0.3 while incompressible solver is selected.",
      suggestion: "Switch flow regime to Steady Compressible.",
    });
  }
  const errorCount = items.filter((i) => i.severity === "error").length;
  const warningCount = items.filter((i) => i.severity === "warning").length;
  const score = Math.max(0, 100 - errorCount * 30 - warningCount * 12);
  return { score, readyToRun: errorCount === 0, errorCount, warningCount, items };
}

export async function detectEnvironment(): Promise<EnvironmentStatus> {
  if (isTauriRuntime()) {
    return invoke<EnvironmentStatus>("cmd_detect_environment");
  }
  return {
    localOpenfoam: null,
    localBinary: null,
    macosAppPath: null,
    dockerInstalled: false,
    dockerRunning: false,
    recommendedBackend: "demo",
    platform: "web/preview",
    cpuCores: 8,
  };
}

export async function exportCaseWithDialog(cfdCase: CfdCase): Promise<{ dir: string; filesWritten: number } | null> {
  if (!isTauriRuntime()) {
    throw new Error("Native directory export requires the Venturi Rust desktop window.");
  }
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Select target folder for OpenFOAM case export",
  });
  if (!selected || typeof selected !== "string") return null;
  const targetDir = `${selected}/${cfdCase.name.replace(/\s+/g, "_")}`;
  const filesWritten = await invoke<number>("cmd_export_case", { case: cfdCase, targetDir });
  return { dir: targetDir, filesWritten };
}

export async function importCaseWithDialog(): Promise<CfdCase | null> {
  if (!isTauriRuntime()) {
    throw new Error("Native OpenFOAM case import requires the Venturi Rust desktop window.");
  }
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Select an existing OpenFOAM case directory (containing system/controlDict)",
  });
  if (!selected || typeof selected !== "string") return null;
  return invoke<CfdCase>("cmd_import_case", { sourceDir: selected });
}

function parseBrowserStlBuffer(buffer: ArrayBuffer, filename: string): StlMeshPayload {
  const bytes = new Uint8Array(buffer);
  if (bytes.byteLength < 15) {
    throw new Error("File is too small to be a valid STL.");
  }

  let isBinary = false;
  if (bytes.byteLength >= 84) {
    const view = new DataView(buffer);
    const triCount = view.getUint32(80, true);
    if (84 + triCount * 50 === bytes.byteLength) {
      isBinary = true;
    }
  }
  if (!isBinary) {
    const head = new TextDecoder("utf-8", { fatal: false }).decode(bytes.subarray(0, Math.min(512, bytes.byteLength)));
    isBinary = !(head.trimStart().startsWith("solid") && head.includes("facet"));
  }

  const tris: { v: [[number, number, number], [number, number, number], [number, number, number]]; n: [number, number, number] }[] = [];

  if (isBinary) {
    const view = new DataView(buffer);
    const count = view.getUint32(80, true);
    if (bytes.byteLength < 84 + count * 50) {
      throw new Error(`Binary STL truncated: expected ${84 + count * 50} bytes.`);
    }
    let off = 84;
    for (let i = 0; i < count; i++) {
      const n: [number, number, number] = [view.getFloat32(off, true), view.getFloat32(off + 4, true), view.getFloat32(off + 8, true)];
      const v0: [number, number, number] = [view.getFloat32(off + 12, true), view.getFloat32(off + 16, true), view.getFloat32(off + 20, true)];
      const v1: [number, number, number] = [view.getFloat32(off + 24, true), view.getFloat32(off + 28, true), view.getFloat32(off + 32, true)];
      const v2: [number, number, number] = [view.getFloat32(off + 36, true), view.getFloat32(off + 40, true), view.getFloat32(off + 44, true)];
      tris.push({ v: [v0, v1, v2], n });
      off += 50;
    }
  } else {
    const text = new TextDecoder("utf-8").decode(bytes);
    let curN: [number, number, number] = [0, 0, 1];
    let verts: [number, number, number][] = [];
    for (const raw of text.split(/\r?\n/)) {
      const line = raw.trim();
      if (line.startsWith("facet normal")) {
        const parts = line.slice(12).trim().split(/\s+/).map(Number);
        if (parts.length >= 3 && parts.every(Number.isFinite)) {
          curN = [parts[0], parts[1], parts[2]];
        }
        verts = [];
      } else if (line.startsWith("vertex")) {
        const parts = line.slice(6).trim().split(/\s+/).map(Number);
        if (parts.length >= 3 && parts.every(Number.isFinite)) {
          verts.push([parts[0], parts[1], parts[2]]);
        }
      } else if (line.startsWith("endfacet") && verts.length === 3) {
        tris.push({ v: [verts[0], verts[1], verts[2]], n: curN });
        verts = [];
      }
    }
  }

  if (tris.length === 0) {
    throw new Error("STL contains zero triangles.");
  }

  const boundsMin: [number, number, number] = [Infinity, Infinity, Infinity];
  const boundsMax: [number, number, number] = [-Infinity, -Infinity, -Infinity];
  let surfaceArea = 0;
  let signedVol6 = 0;

  for (const t of tris) {
    for (const pt of t.v) {
      for (let a = 0; a < 3; a++) {
        if (pt[a] < boundsMin[a]) boundsMin[a] = pt[a];
        if (pt[a] > boundsMax[a]) boundsMax[a] = pt[a];
      }
    }
    const [a, b, c] = t.v;
    const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    const ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    const cx = ab[1] * ac[2] - ab[2] * ac[1];
    const cy = ab[2] * ac[0] - ab[0] * ac[2];
    const cz = ab[0] * ac[1] - ab[1] * ac[0];
    surfaceArea += 0.5 * Math.hypot(cx, cy, cz);
    const bxc0 = b[1] * c[2] - b[2] * c[1];
    const bxc1 = b[2] * c[0] - b[0] * c[2];
    const bxc2 = b[0] * c[1] - b[1] * c[0];
    signedVol6 += a[0] * bxc0 + a[1] * bxc1 + a[2] * bxc2;
  }

  const stride = Math.max(1, Math.floor(tris.length / 80000));
  const positions: number[] = [];
  const normals: number[] = [];
  for (let i = 0; i < tris.length; i += stride) {
    const t = tris[i];
    const [a, b, c] = t.v;
    let [nx, ny, nz] = t.n;
    const nLen = Math.hypot(nx, ny, nz);
    if (nLen > 1e-6 && Number.isFinite(nLen)) {
      nx /= nLen;
      ny /= nLen;
      nz /= nLen;
    } else {
      const ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
      const ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
      const cx = ab[1] * ac[2] - ab[2] * ac[1];
      const cy = ab[2] * ac[0] - ab[0] * ac[2];
      const cz = ab[0] * ac[1] - ab[1] * ac[0];
      const cl = Math.max(1e-8, Math.hypot(cx, cy, cz));
      nx = cx / cl;
      ny = cy / cl;
      nz = cz / cl;
    }
    for (const pt of t.v) {
      positions.push(pt[0], pt[1], pt[2]);
      normals.push(nx, ny, nz);
    }
  }

  return {
    summary: {
      filename,
      triangleCount: tris.length,
      isBinary,
      boundsMin,
      boundsMax,
      surfaceArea,
      volumeEst: Math.abs(signedVol6 / 6),
      scaleFactor: 1.0,
      sourcePath: null,
    },
    positions,
    normals,
  };
}

export async function importStlWithDialog(): Promise<StlMeshPayload | null> {
  if (!isTauriRuntime()) {
    return new Promise<StlMeshPayload | null>((resolve, reject) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = ".stl,.STL";
      input.onchange = async () => {
        const file = input.files?.[0];
        if (!file) {
          resolve(null);
          return;
        }
        try {
          const buf = await file.arrayBuffer();
          resolve(parseBrowserStlBuffer(buf, file.name));
        } catch (err) {
          reject(err);
        }
      };
      input.click();
    });
  }
  const selected = await open({
    directory: false,
    multiple: false,
    title: "Import CAD Geometry (Binary or ASCII STL)",
    filters: [{ name: "STL 3D Mesh", extensions: ["stl", "STL"] }],
  });
  if (!selected || typeof selected !== "string") return null;
  return invoke<StlMeshPayload>("cmd_load_stl", { path: selected });
}

export async function startSolver(
  cfdCase: CfdCase,
  backend: string,
  caseDir?: string
): Promise<string> {
  if (isTauriRuntime()) {
    return invoke<string>("cmd_start_solver", {
      case: cfdCase,
      backend,
      caseDir: caseDir ?? null,
    });
  }
  // Browser-preview benchmark stream mirroring Rust runner.rs
  if (browserRunTimer !== null) window.clearInterval(browserRunTimer);
  const runId = `preview-${Date.now()}`;
  const tol = Math.max(1e-7, cfdCase.numerics.convergenceTol);
  const cdTarget =
    cfdCase.geometryKind === "airfoilNaca0012"
      ? 0.0185
      : cfdCase.geometryKind === "ahmedBody"
        ? 0.285
        : cfdCase.geometryKind === "cylinderCrossflow"
          ? 1.14
          : 0.064;
  const clTarget = cfdCase.geometryKind === "airfoilNaca0012" ? 0.412 : -0.042;
  let step = 0;

  emitBrowserEvent({
    runId,
    kind: "log",
    message: `[Venturi] Launching convergence stream (${cfdCase.name}, ${cfdCase.mesh.cellsX}×${cfdCase.mesh.cellsY}×${cfdCase.mesh.cellsZ} base hexes)...`,
    residual: null,
    status: "running",
  });

  const emitStep = () => {
    step += 1;
    const t = step;
    const rate = 0.048 + 0.032 * cfdCase.numerics.relaxU;
    const ripple = 1 + 0.14 * Math.sin(t * 0.45) + 0.07 * Math.cos(t * 1.13);
    const ux = Math.max(tol * 0.25, 0.85 * Math.exp(-t * rate * 1.12) * ripple);
    const uy = Math.max(tol * 0.3, 0.72 * Math.exp(-t * rate * 1.05) * (1 + 0.16 * Math.cos(t * 0.6)));
    const p = Math.max(tol * 0.45, 0.95 * Math.exp(-t * rate * 0.88) * (1 + 0.18 * Math.sin(t * 0.35)));
    const k = Math.max(tol * 0.35, 0.65 * Math.exp(-t * rate * 0.96) * ripple);
    const omega = Math.max(tol * 0.35, 0.78 * Math.exp(-t * rate * 0.92) * (1 + 0.12 * Math.sin(t * 0.7)));
    const decay = Math.exp(-t * 0.055);
    const cd = cdTarget * (1 + 0.65 * decay * Math.cos(t * 0.35));
    const cl = clTarget * (1 - 0.55 * decay) + 0.012 * decay * Math.sin(t * 0.5);
    const continuity = Math.max(1e-11, 1e-3 * Math.exp(-t * 0.06));

    if (step === 1 || step % 10 === 0) {
      emitBrowserEvent({
        runId,
        kind: "log",
        message: `Time = ${step} | smoothSolver: Ux=${ux.toExponential(2)}, Uy=${uy.toExponential(2)} | GAMG: p=${p.toExponential(2)} | Cd=${cd.toFixed(4)}, Cl=${cl.toFixed(4)}`,
        residual: null,
        status: null,
      });
    }
    emitBrowserEvent({
      runId,
      kind: "residual",
      message: null,
      residual: { iteration: t, ux, uy, uz: uy * 0.4, p, k, omega, epsilon: k * 0.8, cd, cl, continuity },
      status: null,
    });

    if (ux < tol && uy < tol && p < tol && step > 25) {
      if (browserRunTimer !== null) window.clearInterval(browserRunTimer);
      browserRunTimer = null;
      emitBrowserEvent({
        runId,
        kind: "log",
        message: `[Venturi] SIMPLE solution converged in ${step} iterations (residuals < ${tol.toExponential(1)}).`,
        residual: null,
        status: null,
      });
      emitBrowserEvent({ runId, kind: "status", message: null, residual: null, status: "converged" });
    }
  };

  browserRunTimer = window.setInterval(emitStep, 20);
  return runId;
}

export async function stopSolver(): Promise<void> {
  if (isTauriRuntime()) {
    await invoke("cmd_stop_solver");
    return;
  }
  if (browserRunTimer !== null) {
    window.clearInterval(browserRunTimer);
    browserRunTimer = null;
  }
}

export async function listenSolverEvents(
  handler: (event: SolverEvent) => void
): Promise<UnlistenFn> {
  if (isTauriRuntime()) {
    return listen<SolverEvent>("solver-event", (e) => handler(e.payload));
  }
  browserListeners.add(handler);
  return () => {
    browserListeners.delete(handler);
  };
}

// ---------------------------------------------------------------------------
// Parametric CAD, Shared Session, MCP Tooling & Autonomous Loop Bridge
// ---------------------------------------------------------------------------

const browserAutopilotListeners = new Set<(evt: AutopilotEvent) => void>();
let browserAutopilotCancel = false;
let browserPendingResolver: ((approved: boolean) => void) | null = null;

function emitBrowserAutopilotEvent(evt: AutopilotEvent) {
  browserAutopilotListeners.forEach((fn) => fn(evt));
}

function defaultGuardrails(): GuardrailsConfig {
  return {
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
}

function defaultGoal(): OptimizationGoal {
  return {
    prompt: "Achieve wall-resolved y+ ≈ 1, refine high-gradient wake region, and minimize drag coefficient Cd.",
    targetYPlus: 1.0,
    maxDeltaPPa: null,
    minimizeCd: true,
    maximizeClOverCd: false,
  };
}

let mockSession: SessionState | null = null;

function ensureMockSession(): SessionState {
  if (!mockSession) {
    mockSession = {
      version: 1,
      updatedAt: new Date().toISOString(),
      lastWriter: "gui",
      case: CASE_PRESETS[0].build(),
      guardrails: defaultGuardrails(),
      goal: defaultGoal(),
      iterations: [],
      bestIterationIndex: null,
      toolLogs: [],
      caseDir: null,
    };
  }
  return mockSession;
}

function cloneSession(s: SessionState): SessionState {
  return JSON.parse(JSON.stringify(s)) as SessionState;
}

function pushMockToolLog(
  s: SessionState,
  source: string,
  toolName: string,
  args: Record<string, unknown>,
  ok: boolean,
  summary: string
) {
  const argsStr = JSON.stringify(args);
  s.toolLogs.push({
    id: `log-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
    timestamp: new Date().toISOString(),
    source,
    toolName,
    argsPreview: argsStr.length > 110 ? `${argsStr.slice(0, 107)}...` : argsStr,
    ok,
    summary,
  });
  if (s.toolLogs.length > 120) {
    s.toolLogs = s.toolLogs.slice(-120);
  }
}

type Vec3 = [number, number, number];

function buildStlPayloadFromTriangles(filename: string, tris: [Vec3, Vec3, Vec3][]): StlMeshPayload {
  const positions: number[] = [];
  const normals: number[] = [];
  const boundsMin: Vec3 = [Infinity, Infinity, Infinity];
  const boundsMax: Vec3 = [-Infinity, -Infinity, -Infinity];
  let surfaceArea = 0;
  let signedVol = 0;

  for (const [a, b, c] of tris) {
    const ux = b[0] - a[0];
    const uy = b[1] - a[1];
    const uz = b[2] - a[2];
    const vx = c[0] - a[0];
    const vy = c[1] - a[1];
    const vz = c[2] - a[2];
    const cx = uy * vz - uz * vy;
    const cy = uz * vx - ux * vz;
    const cz = ux * vy - uy * vx;
    const len = Math.hypot(cx, cy, cz);
    const nx = len > 1e-12 ? cx / len : 0;
    const ny = len > 1e-12 ? cy / len : 1;
    const nz = len > 1e-12 ? cz / len : 0;
    surfaceArea += 0.5 * len;
    signedVol +=
      (a[0] * (b[1] * c[2] - b[2] * c[1]) +
        b[0] * (c[1] * a[2] - c[2] * a[1]) +
        c[0] * (a[1] * b[2] - a[2] * b[1])) /
      6.0;

    for (const p of [a, b, c]) {
      positions.push(p[0], p[1], p[2]);
      normals.push(nx, ny, nz);
      if (p[0] < boundsMin[0]) boundsMin[0] = p[0];
      if (p[1] < boundsMin[1]) boundsMin[1] = p[1];
      if (p[2] < boundsMin[2]) boundsMin[2] = p[2];
      if (p[0] > boundsMax[0]) boundsMax[0] = p[0];
      if (p[1] > boundsMax[1]) boundsMax[1] = p[1];
      if (p[2] > boundsMax[2]) boundsMax[2] = p[2];
    }
  }

  if (tris.length === 0) {
    boundsMin[0] = boundsMin[1] = boundsMin[2] = 0;
    boundsMax[0] = boundsMax[1] = boundsMax[2] = 0;
  }

  return {
    summary: {
      filename,
      triangleCount: tris.length,
      isBinary: true,
      boundsMin,
      boundsMax,
      surfaceArea,
      volumeEst: Math.abs(signedVol),
    },
    positions,
    normals,
  };
}

function extrudePolygonZ(profile: [number, number][], z0: number, z1: number, tris: [Vec3, Vec3, Vec3][]) {
  const n = profile.length;
  if (n < 3) return;
  let cx = 0;
  let cy = 0;
  for (const [x, y] of profile) {
    cx += x;
    cy += y;
  }
  cx /= n;
  cy /= n;

  for (let i = 0; i < n; i++) {
    const [x0, y0] = profile[i];
    const [x1, y1] = profile[(i + 1) % n];
    // Side quad
    tris.push(
      [[x0, y0, z0], [x1, y1, z0], [x1, y1, z1]],
      [[x0, y0, z0], [x1, y1, z1], [x0, y0, z1]]
    );
    // Front & back cap fans
    tris.push([[cx, cy, z1], [x0, y0, z1], [x1, y1, z1]]);
    tris.push([[cx, cy, z0], [x1, y1, z0], [x0, y0, z0]]);
  }
}

function appendBoxTris(
  x0: number,
  y0: number,
  z0: number,
  x1: number,
  y1: number,
  z1: number,
  tris: [Vec3, Vec3, Vec3][]
) {
  const v: Vec3[] = [
    [x0, y0, z0],
    [x1, y0, z0],
    [x1, y1, z0],
    [x0, y1, z0],
    [x0, y0, z1],
    [x1, y0, z1],
    [x1, y1, z1],
    [x0, y1, z1],
  ];
  const faces: [number, number, number, number][] = [
    [0, 3, 2, 1],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [2, 3, 7, 6],
    [1, 2, 6, 5],
    [0, 4, 7, 3],
  ];
  for (const [a, b, c, d] of faces) {
    tris.push([v[a], v[b], v[c]], [v[a], v[c], v[d]]);
  }
}

function appendCylinderYTris(
  cx: number,
  y0: number,
  y1: number,
  cz: number,
  r: number,
  segs: number,
  tris: [Vec3, Vec3, Vec3][]
) {
  for (let i = 0; i < segs; i++) {
    const a0 = (i / segs) * Math.PI * 2;
    const a1 = ((i + 1) / segs) * Math.PI * 2;
    const x0 = cx + r * Math.cos(a0);
    const z0 = cz + r * Math.sin(a0);
    const x1 = cx + r * Math.cos(a1);
    const z1 = cz + r * Math.sin(a1);
    tris.push(
      [[x0, y0, z0], [x0, y1, z0], [x1, y1, z1]],
      [[x0, y0, z0], [x1, y1, z1], [x1, y0, z1]]
    );
    tris.push([[cx, y1, cz], [x0, y1, z0], [x1, y1, z1]]);
    tris.push([[cx, y0, cz], [x1, y0, z1], [x0, y0, z0]]);
  }
}

function buildPreviewParametricStl(spec: ParametricGeometrySpec): StlMeshPayload {
  const tris: [Vec3, Vec3, Vec3][] = [];

  if (spec.family === "nacaAirfoil") {
    const c = Math.max(0.05, spec.chord);
    const m = Math.max(0, Math.min(0.12, spec.camberPct / 100));
    const p = Math.max(0.1, Math.min(0.9, spec.camberPosPct / 100));
    const t = Math.max(0.03, Math.min(0.4, spec.thicknessPct / 100));
    const aoa = (-spec.aoaDeg * Math.PI) / 180;
    const cosA = Math.cos(aoa);
    const sinA = Math.sin(aoa);
    const n = 36;
    const upper: [number, number][] = [];
    const lower: [number, number][] = [];

    for (let i = 0; i <= n; i++) {
      const beta = (i / n) * Math.PI;
      const x = 0.5 * (1 - Math.cos(beta));
      const yt =
        5 *
        t *
        (0.2969 * Math.sqrt(x) - 0.126 * x - 0.3516 * x * x + 0.2843 * x * x * x - 0.1015 * x * x * x * x);
      let yc = 0;
      let dyc = 0;
      if (m > 1e-6) {
        if (x < p) {
          yc = (m / (p * p)) * (2 * p * x - x * x);
          dyc = ((2 * m) / (p * p)) * (p - x);
        } else {
          yc = (m / ((1 - p) * (1 - p))) * (1 - 2 * p + 2 * p * x - x * x);
          dyc = ((2 * m) / ((1 - p) * (1 - p))) * (p - x);
        }
      }
      const theta = Math.atan(dyc);
      const xu = (x - yt * Math.sin(theta) - 0.25) * c;
      const yu = (yc + yt * Math.cos(theta)) * c;
      const xl = (x + yt * Math.sin(theta) - 0.25) * c;
      const yl = (yc - yt * Math.cos(theta)) * c;
      upper.push([xu * cosA - yu * sinA, xu * sinA + yu * cosA]);
      lower.push([xl * cosA - yl * sinA, xl * sinA + yl * cosA]);
    }
    const profile: [number, number][] = [...upper, ...lower.slice(1, -1).reverse()];
    const halfSpan = Math.max(0.02, spec.span) / 2;
    extrudePolygonZ(profile, -halfSpan, halfSpan, tris);
    return buildStlPayloadFromTriangles("naca_airfoil.stl", tris);
  }

  if (spec.family === "venturiTube") {
    const lConv = Math.max(0.05, spec.convergentLen);
    const lThroat = Math.max(0.05, spec.throatLen);
    const lDiv = Math.max(0.05, spec.divergentLen);
    const total = lConv + lThroat + lDiv;
    const xStart = -total * 0.5;
    const stations: [number, number][] = [];
    const segAxial = 32;
    for (let i = 0; i <= segAxial; i++) {
      const s = (i / segAxial) * total;
      let r = spec.throatRadius;
      if (s < lConv) {
        const u = s / lConv;
        const smooth = 0.5 * (1 - Math.cos(Math.PI * u));
        r = spec.inletRadius + (spec.throatRadius - spec.inletRadius) * smooth;
      } else if (s > lConv + lThroat) {
        const u = (s - lConv - lThroat) / lDiv;
        const smooth = 0.5 * (1 - Math.cos(Math.PI * u));
        r = spec.throatRadius + (spec.outletRadius - spec.throatRadius) * smooth;
      }
      stations.push([xStart + s, Math.max(0.02, r)]);
    }
    const segCirc = 28;
    for (let i = 0; i < stations.length - 1; i++) {
      const [x0, r0] = stations[i];
      const [x1, r1] = stations[i + 1];
      for (let j = 0; j < segCirc; j++) {
        const a0 = (j / segCirc) * Math.PI * 2;
        const a1 = ((j + 1) / segCirc) * Math.PI * 2;
        const p00: Vec3 = [x0, r0 * Math.cos(a0), r0 * Math.sin(a0)];
        const p01: Vec3 = [x0, r0 * Math.cos(a1), r0 * Math.sin(a1)];
        const p10: Vec3 = [x1, r1 * Math.cos(a0), r1 * Math.sin(a0)];
        const p11: Vec3 = [x1, r1 * Math.cos(a1), r1 * Math.sin(a1)];
        tris.push([p00, p10, p11], [p00, p11, p01]);
      }
    }
    return buildStlPayloadFromTriangles("venturi_tube.stl", tris);
  }

  if (spec.family === "ahmedDiffuser") {
    const L = Math.max(0.2, spec.length);
    const H = Math.max(0.08, spec.height);
    const W = Math.max(0.08, spec.width);
    const gc = Math.max(0.01, spec.groundClearance);
    const slantRad = (Math.max(2, Math.min(60, spec.rearSlantDeg)) * Math.PI) / 180;
    const slantLen = Math.min(L * 0.35, 0.222 * (L / 1.044));
    const slantDrop = Math.min(H * 0.72, slantLen * Math.tan(slantRad));
    const noseR = Math.min(H * 0.32, L * 0.12);
    const profile: [number, number][] = [
      [noseR, gc],
      [L, gc],
      [L, gc + H - slantDrop],
      [L - slantLen, gc + H],
      [noseR, gc + H],
      [0.25 * noseR, gc + H - 0.25 * noseR],
      [0, gc + H - noseR],
      [0, gc + noseR],
      [0.25 * noseR, gc + 0.25 * noseR],
    ].map(([x, y]) => [x - 0.2 * L, y]);
    extrudePolygonZ(profile, -W / 2, W / 2, tris);
    return buildStlPayloadFromTriangles("ahmed_diffuser.stl", tris);
  }

  if (spec.family === "pinFinArray") {
    const bl = Math.max(0.02, spec.baseLength);
    const bw = Math.max(0.02, spec.baseWidth);
    const baseThick = Math.max(0.004, spec.finHeight * 0.18);
    appendBoxTris(-bl / 2, 0, -bw / 2, bl / 2, baseThick, bw / 2, tris);
    const nx = Math.max(1, Math.min(16, Math.round(spec.finCountX)));
    const nz = Math.max(1, Math.min(16, Math.round(spec.finCountZ)));
    const r = Math.max(0.001, spec.finRadius);
    const h = Math.max(0.005, spec.finHeight);
    for (let ix = 0; ix < nx; ix++) {
      const fx = nx === 1 ? 0 : -bl * 0.38 + (ix / (nx - 1)) * (bl * 0.76);
      for (let iz = 0; iz < nz; iz++) {
        const fz = nz === 1 ? 0 : -bw * 0.38 + (iz / (nz - 1)) * (bw * 0.76);
        appendCylinderYTris(fx, baseThick, baseThick + h, fz, r, 14, tris);
      }
    }
    return buildStlPayloadFromTriangles("pin_fin_array.stl", tris);
  }

  // cylinder
  const r = Math.max(0.01, spec.radius);
  const span = Math.max(0.02, spec.span);
  const segs = 36;
  const profile: [number, number][] = [];
  for (let i = 0; i < segs; i++) {
    const a = (i / segs) * Math.PI * 2;
    profile.push([r * Math.cos(a), r * Math.sin(a)]);
  }
  extrudePolygonZ(profile, -span / 2, span / 2, tris);
  return buildStlPayloadFromTriangles("cylinder.stl", tris);
}

async function evaluateMockSurrogate(cfdCase: CfdCase, goal: OptimizationGoal, guardrails: GuardrailsConfig): Promise<{
  results: OpenFoamResultSummary;
  score: number;
}> {
  const tel = await computeTelemetry(cfdCase);
  const ypTarget = goal.targetYPlus ?? cfdCase.mesh.targetYPlus ?? 1.0;
  const ypCalc = await computeYPlus({
    velocity: Math.max(0.05, cfdCase.physics.referenceVelocity),
    density: cfdCase.physics.fluid.density,
    dynamicViscosity: cfdCase.physics.fluid.dynamicViscosity,
    referenceLength: Math.max(0.01, cfdCase.physics.referenceLength),
    targetYPlus: ypTarget,
    expansionRatio: cfdCase.mesh.expansionRatio,
  });

  const achievedYPlus = Math.max(
    0.15,
    (cfdCase.mesh.firstLayerThickness / Math.max(1e-9, ypCalc.firstCellHeight)) * ypTarget
  );
  const boxBonus = Math.min(3, cfdCase.mesh.refinementBoxes.length);
  const maxSkewness = Math.max(0.8, 3.1 - boxBonus * 0.55 - (cfdCase.mesh.inflationLayers >= 8 ? 0.35 : 0));
  const maxNonOrtho = Math.max(24, 54 - boxBonus * 7.5);

  let cd = 0.045;
  let cl = 0.12;
  let deltaPPa = 0.5 * cfdCase.physics.fluid.density * cfdCase.physics.referenceVelocity ** 2 * 0.28;
  const p = cfdCase.parametric;
  if (p?.family === "nacaAirfoil" || cfdCase.geometryKind === "airfoilNaca0012") {
    const aoa = p?.family === "nacaAirfoil" ? p.aoaDeg : 0;
    const thick = p?.family === "nacaAirfoil" ? p.thicknessPct / 100 : 0.12;
    const camb = p?.family === "nacaAirfoil" ? p.camberPct / 100 : 0;
    cd = 0.012 + 0.055 * thick + 0.00055 * aoa * aoa - boxBonus * 0.0014;
    cl = 0.105 * aoa + 9.5 * camb;
    deltaPPa = 0.5 * cfdCase.physics.fluid.density * cfdCase.physics.referenceVelocity ** 2 * cd * 1.4;
  } else if (p?.family === "ahmedDiffuser" || cfdCase.geometryKind === "ahmedBody") {
    const slant = p?.family === "ahmedDiffuser" ? p.rearSlantDeg : 25;
    cd = 0.245 + 0.00022 * (slant - 14) ** 2 - boxBonus * 0.012;
    cl = -0.06;
    deltaPPa = 0.5 * cfdCase.physics.fluid.density * cfdCase.physics.referenceVelocity ** 2 * cd;
  } else if (p?.family === "venturiTube" || cfdCase.geometryKind === "venturiNozzle") {
    const ratio = p?.family === "venturiTube" ? p.inletRadius / Math.max(0.04, p.throatRadius) : 2.0;
    cd = 0.075 + 0.025 * (ratio - 1.4) ** 2 - boxBonus * 0.008;
    cl = 0;
    deltaPPa = 0.5 * cfdCase.physics.fluid.density * cfdCase.physics.referenceVelocity ** 2 * (ratio ** 2 - 1) * 0.35;
  } else if (p?.family === "pinFinArray") {
    cd = 0.32 + 0.008 * p.finCountX * p.finCountZ - boxBonus * 0.02;
    cl = 0;
    deltaPPa = 0.5 * cfdCase.physics.fluid.density * cfdCase.physics.referenceVelocity ** 2 * cd * 0.9;
  }

  const d = cfdCase.domain;
  const lx = Math.max(0.2, d.xMax - d.xMin);
  const ly = Math.max(0.2, d.yMax - d.yMin);
  const lz = Math.max(0.1, d.zMax - d.zMin);
  const refL = Math.max(0.1, cfdCase.physics.referenceLength);
  const hotspots: GradientHotspot[] =
    boxBonus === 0
      ? [
          {
            id: "hs_wake_1",
            center: [Math.min(d.xMax - 0.1 * lx, refL * 0.85), 0.5 * (d.yMin + d.yMax), 0.5 * (d.zMin + d.zMax)],
            suggestedBoxMin: [
              Number(Math.max(d.xMin + 0.05 * lx, -0.15 * refL).toFixed(3)),
              Number(Math.max(d.yMin + 0.05 * ly, 0.5 * (d.yMin + d.yMax) - 0.24 * ly).toFixed(3)),
              Number(Math.max(d.zMin + 0.05 * lz, d.zMin + 0.08 * lz).toFixed(3)),
            ],
            suggestedBoxMax: [
              Number(Math.min(d.xMax - 0.08 * lx, refL * 2.2).toFixed(3)),
              Number(Math.min(d.yMax - 0.05 * ly, 0.5 * (d.yMin + d.yMax) + 0.24 * ly).toFixed(3)),
              Number(Math.min(d.zMax - 0.05 * lz, d.zMax - 0.08 * lz).toFixed(3)),
            ],
            gradientScore: 0.86,
            suggestedLevel: Math.min(guardrails.maxRefinementLevel, 3),
            reason: "High wake velocity-gradient & shear-layer vorticity downstream of obstacle.",
          },
        ]
      : [];

  const wallPatchName = cfdCase.boundaries.find((b) => b.face === "obstacle")?.name ?? "walls";
  const results: OpenFoamResultSummary = {
    backendUsed: "surrogate-preview",
    converged: true,
    diverged: false,
    iterationsCompleted: 140 + boxBonus * 18,
    finalResidualU: 4.2e-5,
    finalResidualP: 7.8e-5,
    continuityError: 1.9e-9,
    cd: Number(cd.toFixed(4)),
    cl: Number(cl.toFixed(4)),
    cm: Number((-0.025 * cl).toFixed(4)),
    deltaPPa: Number(deltaPPa.toFixed(1)),
    checkMesh: {
      cells: tel.estimatedCells,
      maxNonOrthogonality: Number(maxNonOrtho.toFixed(1)),
      avgNonOrthogonality: Number((maxNonOrtho * 0.24).toFixed(1)),
      maxSkewness: Number(maxSkewness.toFixed(2)),
      maxAspectRatio: Number((18 - boxBonus * 2.5).toFixed(1)),
      failedChecks: 0,
      meshOk: maxSkewness <= guardrails.maxSkewness && maxNonOrtho <= guardrails.maxNonOrtho,
    },
    yPlusPatches: [
      {
        patch: wallPatchName,
        min: Number((achievedYPlus * 0.55).toFixed(2)),
        max: Number((achievedYPlus * 1.45).toFixed(2)),
        average: Number(achievedYPlus.toFixed(2)),
      },
    ],
    hotspots,
    notes: [
      `Achieved mean y+ = ${achievedYPlus.toFixed(2)} (target ${ypTarget.toFixed(1)}).`,
      boxBonus > 0
        ? `Active refinement regions (${boxBonus}) reduced wake discretization error.`
        : "Unrefined wake detected — add a refinement box on hs_wake_1.",
    ],
  };

  let score = 74;
  const ypErr = Math.abs(achievedYPlus - ypTarget) / Math.max(0.5, ypTarget);
  score -= Math.min(22, ypErr * 14);
  if (results.checkMesh.meshOk) score += 8;
  score += Math.min(10, boxBonus * 6);
  if (goal.minimizeCd) score += Math.max(0, 8 - cd * 18);
  if (goal.maximizeClOverCd && cd > 1e-4) score += Math.min(8, Math.abs(cl / cd) * 0.25);
  if (goal.maxDeltaPPa !== null && deltaPPa > goal.maxDeltaPPa) score -= 12;
  score = Math.max(5, Math.min(99.4, Number(score.toFixed(1))));

  return { results, score };
}

export async function generateParametricGeometry(spec: ParametricGeometrySpec): Promise<StlMeshPayload> {
  if (isTauriRuntime()) {
    return invoke<StlMeshPayload>("cmd_generate_parametric_geometry", { spec });
  }
  return buildPreviewParametricStl(spec);
}

export async function getSession(): Promise<SessionState> {
  if (isTauriRuntime()) {
    return invoke<SessionState>("cmd_get_session");
  }
  return cloneSession(ensureMockSession());
}

export async function syncSessionState(
  cfdCase: CfdCase,
  guardrails: GuardrailsConfig,
  goal: OptimizationGoal
): Promise<SessionState> {
  if (isTauriRuntime()) {
    return invoke<SessionState>("cmd_sync_session_case", {
      case: cfdCase,
      guardrails,
      goal,
    });
  }
  const s = ensureMockSession();
  s.case = JSON.parse(JSON.stringify(cfdCase)) as CfdCase;
  s.guardrails = { ...guardrails };
  s.goal = { ...goal };
  s.version += 1;
  s.updatedAt = new Date().toISOString();
  s.lastWriter = "gui";
  return cloneSession(s);
}

export async function executeMcpTool(
  toolName: string,
  args: Record<string, unknown>
): Promise<Record<string, unknown>> {
  if (isTauriRuntime()) {
    return invoke<Record<string, unknown>>("cmd_execute_mcp_tool", {
      toolName,
      arguments: args,
    });
  }
  const s = ensureMockSession();
  const source = "in-app-autopilot";

  if (toolName === "venturi_get_state") {
    pushMockToolLog(s, source, toolName, args, true, `Returned session v${s.version}`);
    return { session: cloneSession(s) };
  }

  if (toolName === "venturi_auto_size_yplus") {
    const targetYPlus = typeof args.targetYPlus === "number" ? args.targetYPlus : s.goal.targetYPlus ?? 1.0;
    const res = await computeYPlus({
      velocity: Math.max(0.05, s.case.physics.referenceVelocity),
      density: s.case.physics.fluid.density,
      dynamicViscosity: s.case.physics.fluid.dynamicViscosity,
      referenceLength: Math.max(0.01, s.case.physics.referenceLength),
      targetYPlus,
      expansionRatio: s.case.mesh.expansionRatio,
    });
    s.case.mesh.targetYPlus = targetYPlus;
    s.case.mesh.firstLayerThickness = Number(res.firstCellHeight.toPrecision(4));
    s.case.mesh.inflationLayers = res.recommendedLayers;
    s.case.mesh.snappyEnabled = true;
    s.version += 1;
    s.updatedAt = new Date().toISOString();
    s.lastWriter = source;
    const summary = `Sized Δy₁ = ${(res.firstCellHeight * 1000).toFixed(4)} mm, ${res.recommendedLayers} prism layers for y+=${targetYPlus}`;
    pushMockToolLog(s, source, toolName, args, true, summary);
    return { ok: true, summary, firstLayerThickness: s.case.mesh.firstLayerThickness, layers: res.recommendedLayers };
  }

  if (toolName === "venturi_upsert_refinement_box") {
    const box: RefinementBox = {
      name: String(args.name ?? `wake_zone_${s.case.mesh.refinementBoxes.length + 1}`),
      min: (args.min as [number, number, number]) ?? [-0.2, -0.5, -0.3],
      max: (args.max as [number, number, number]) ?? [2.2, 0.5, 0.3],
      level: Math.min(s.guardrails.maxRefinementLevel, Math.max(1, Number(args.level ?? 3))),
      reason: String(args.reason ?? "Gradient hotspot refinement"),
    };
    const idx = s.case.mesh.refinementBoxes.findIndex((b) => b.name === box.name);
    if (idx >= 0) s.case.mesh.refinementBoxes[idx] = box;
    else s.case.mesh.refinementBoxes.push(box);
    s.case.mesh.snappyEnabled = true;
    s.version += 1;
    s.updatedAt = new Date().toISOString();
    s.lastWriter = source;
    const summary = `Upserted refinement box "${box.name}" at L${box.level}`;
    pushMockToolLog(s, source, toolName, args, true, summary);
    return { ok: true, box, summary };
  }

  if (toolName === "venturi_run_iteration") {
    const label = String(args.label ?? `Iteration #${s.iterations.length}`);
    const changesSummary = String(args.changesSummary ?? "Evaluated current CFD configuration");
    const { results, score } = await evaluateMockSurrogate(s.case, s.goal, s.guardrails);
    const index = s.iterations.length;
    const prevBest = s.bestIterationIndex !== null ? s.iterations[s.bestIterationIndex] : null;
    const isBest = !prevBest || score >= prevBest.objectiveScore;
    if (isBest) {
      s.iterations.forEach((it) => {
        it.isBest = false;
      });
      s.bestIterationIndex = index;
    }
    const rec: IterationRecord = {
      index,
      timestamp: new Date().toISOString(),
      label,
      changesSummary,
      caseSnapshot: JSON.parse(JSON.stringify(s.case)) as CfdCase,
      results,
      objectiveScore: score,
      isBest,
      rolledBack: false,
    };
    s.iterations.push(rec);
    s.version += 1;
    s.updatedAt = new Date().toISOString();
    s.lastWriter = source;
    const summary = `Iteration #${index} complete: Score ${score.toFixed(1)}/100, Cd=${results.cd ?? "—"}`;
    pushMockToolLog(s, source, toolName, args, true, summary);
    return { ok: true, iteration: rec, summary };
  }

  pushMockToolLog(s, source, toolName, args, true, `Executed ${toolName} in browser preview`);
  return { ok: true, toolName };
}

async function sleepPreview(ms: number): Promise<void> {
  await new Promise((resolve) => window.setTimeout(resolve, ms));
}

async function waitProposalPreview(proposal: PendingProposal): Promise<boolean> {
  emitBrowserAutopilotEvent({
    kind: "proposal",
    message: `Awaiting engineer approval for ${proposal.toolName}`,
    toolName: proposal.toolName,
    proposal,
  });
  return new Promise<boolean>((resolve) => {
    browserPendingResolver = resolve;
  });
}

async function runPreviewLoop(goal: OptimizationGoal, mode: "autopilot" | "review", headerLabel: string) {
  browserAutopilotCancel = false;
  const s = ensureMockSession();
  s.goal = { ...goal };

  emitBrowserAutopilotEvent({
    kind: "thinking",
    message: `[${headerLabel}] Analyzing current case "${s.case.name}" against goal: ${goal.prompt || "Optimize CFD setup"}`,
    toolName: null,
    proposal: null,
  });
  await sleepPreview(350);
  if (browserAutopilotCancel) return;

  // Step 1: Baseline run
  emitBrowserAutopilotEvent({
    kind: "tool_call",
    message: "Executing baseline simulation to measure wall y+, checkMesh quality, and wake gradients.",
    toolName: "venturi_run_iteration",
    proposal: null,
  });
  const baseOut = await executeMcpTool("venturi_run_iteration", {
    label: "Baseline",
    changesSummary: "Initial baseline evaluation",
  });
  const baseRec = baseOut.iteration as IterationRecord;
  emitBrowserAutopilotEvent({
    kind: "tool_result",
    message: String(baseOut.summary ?? "Baseline finished"),
    toolName: "venturi_run_iteration",
    proposal: null,
  });
  emitBrowserAutopilotEvent({
    kind: "state_sync",
    message: `Session synced (v${s.version})`,
    toolName: null,
    proposal: null,
  });
  await sleepPreview(450);
  if (browserAutopilotCancel) return;

  // Step 2: Auto-size y+
  const ypArgs = { targetYPlus: goal.targetYPlus ?? s.case.mesh.targetYPlus ?? 1.0 };
  if (mode === "review") {
    const ok = await waitProposalPreview({
      id: `prop-${Date.now()}-1`,
      toolName: "venturi_auto_size_yplus",
      arguments: ypArgs,
      rationale: `Size first-layer Δy₁ and prism layers to align wall y+ with target ${ypArgs.targetYPlus}.`,
    });
    if (!ok || browserAutopilotCancel) {
      emitBrowserAutopilotEvent({
        kind: "done",
        message: "Proposal rejected or stopped by engineer.",
        toolName: null,
        proposal: null,
      });
      return;
    }
  }
  emitBrowserAutopilotEvent({
    kind: "tool_call",
    message: `Applying Rust boundary-layer sizing for target y+ = ${ypArgs.targetYPlus}`,
    toolName: "venturi_auto_size_yplus",
    proposal: null,
  });
  const ypOut = await executeMcpTool("venturi_auto_size_yplus", ypArgs);
  emitBrowserAutopilotEvent({
    kind: "tool_result",
    message: String(ypOut.summary ?? "Updated boundary layer"),
    toolName: "venturi_auto_size_yplus",
    proposal: null,
  });
  emitBrowserAutopilotEvent({
    kind: "state_sync",
    message: `Session synced (v${s.version})`,
    toolName: null,
    proposal: null,
  });
  await sleepPreview(420);
  if (browserAutopilotCancel) return;

  // Step 3: Place wake refinement box on top gradient hotspot
  const topHs = baseRec?.results?.hotspots?.[0];
  if (topHs) {
    const boxArgs = {
      name: "wake_hotspot_1",
      min: topHs.suggestedBoxMin,
      max: topHs.suggestedBoxMax,
      level: topHs.suggestedLevel,
      reason: topHs.reason,
    };
    if (mode === "review") {
      const ok = await waitProposalPreview({
        id: `prop-${Date.now()}-2`,
        toolName: "venturi_upsert_refinement_box",
        arguments: boxArgs,
        rationale: `Place L${boxArgs.level} snappyHexMesh refinement region over detected wake gradient hotspot.`,
      });
      if (!ok || browserAutopilotCancel) {
        emitBrowserAutopilotEvent({
          kind: "done",
          message: "Refinement box proposal skipped by engineer.",
          toolName: null,
          proposal: null,
        });
        return;
      }
    }
    emitBrowserAutopilotEvent({
      kind: "tool_call",
      message: `Inserting L${boxArgs.level} refinement box "wake_hotspot_1" into snappyHexMeshDict`,
      toolName: "venturi_upsert_refinement_box",
      proposal: null,
    });
    const boxOut = await executeMcpTool("venturi_upsert_refinement_box", boxArgs);
    emitBrowserAutopilotEvent({
      kind: "tool_result",
      message: String(boxOut.summary ?? "Added refinement box"),
      toolName: "venturi_upsert_refinement_box",
      proposal: null,
    });
    emitBrowserAutopilotEvent({
      kind: "state_sync",
      message: `Session synced (v${s.version})`,
      toolName: null,
      proposal: null,
    });
    await sleepPreview(420);
    if (browserAutopilotCancel) return;
  }

  // Step 4: Verification run
  emitBrowserAutopilotEvent({
    kind: "tool_call",
    message: "Re-running simulation to verify improved y+, skewness, and aerodynamic coefficients.",
    toolName: "venturi_run_iteration",
    proposal: null,
  });
  const refOut = await executeMcpTool("venturi_run_iteration", {
    label: "Refined y+ & Wake",
    changesSummary: "Auto-sized wall prism layers + L3 wake hotspot refinement box",
  });
  emitBrowserAutopilotEvent({
    kind: "tool_result",
    message: String(refOut.summary ?? "Verification iteration complete"),
    toolName: "venturi_run_iteration",
    proposal: null,
  });
  emitBrowserAutopilotEvent({
    kind: "state_sync",
    message: `Session synced (v${s.version})`,
    toolName: null,
    proposal: null,
  });
  emitBrowserAutopilotEvent({
    kind: "done",
    message: "Closed-loop optimization complete — best configuration verified within guardrails.",
    toolName: null,
    proposal: null,
  });
}

export async function startAutopilot(llmConfig: LlmConfig, goal: OptimizationGoal): Promise<void> {
  if (isTauriRuntime()) {
    await invoke("cmd_start_autopilot", { llmConfig, goal });
    return;
  }
  void runPreviewLoop(goal, llmConfig.mode, `${llmConfig.provider}/${llmConfig.model}`);
}

export async function startBuiltinAutopilot(goal: OptimizationGoal, backend: string): Promise<void> {
  if (isTauriRuntime()) {
    await invoke("cmd_start_builtin_autopilot", { goal, backend });
    return;
  }
  void runPreviewLoop(goal, "autopilot", `Built-In Loop (${backend})`);
}

export async function approveProposal(approved: boolean): Promise<void> {
  if (isTauriRuntime()) {
    await invoke("cmd_approve_proposal", { approved });
    return;
  }
  if (browserPendingResolver) {
    const resolve = browserPendingResolver;
    browserPendingResolver = null;
    resolve(approved);
  }
}

export async function stopAutopilot(): Promise<void> {
  if (isTauriRuntime()) {
    await invoke("cmd_stop_autopilot");
    return;
  }
  browserAutopilotCancel = true;
  if (browserPendingResolver) {
    const resolve = browserPendingResolver;
    browserPendingResolver = null;
    resolve(false);
  }
  emitBrowserAutopilotEvent({
    kind: "done",
    message: "Autopilot stopped by engineer.",
    toolName: null,
    proposal: null,
  });
}

export async function rollbackIteration(iterationIndex?: number): Promise<SessionState> {
  if (isTauriRuntime()) {
    return invoke<SessionState>("cmd_rollback_iteration", {
      iterationIndex: iterationIndex ?? null,
    });
  }
  const s = ensureMockSession();
  const targetIdx = iterationIndex ?? s.bestIterationIndex;
  const found = targetIdx !== null && targetIdx !== undefined ? s.iterations.find((i) => i.index === targetIdx) : null;
  if (found) {
    s.case = JSON.parse(JSON.stringify(found.caseSnapshot)) as CfdCase;
    s.version += 1;
    s.updatedAt = new Date().toISOString();
    s.lastWriter = "gui";
    pushMockToolLog(s, "gui", "venturi_rollback_iteration", { iterationIndex: found.index }, true, `Rolled back to iteration #${found.index}`);
    emitBrowserAutopilotEvent({
      kind: "state_sync",
      message: `Rolled back to iteration #${found.index}`,
      toolName: "venturi_rollback_iteration",
      proposal: null,
    });
  }
  return cloneSession(s);
}

export async function getMcpConfigSnippet(): Promise<string> {
  if (isTauriRuntime()) {
    return invoke<string>("cmd_get_mcp_config_snippet");
  }
  return JSON.stringify(
    {
      mcpServers: {
        "venturi-cfd": {
          command: "/path/to/venturi_mcp",
          args: ["--session", "~/.venturi/active_session.json"],
        },
      },
    },
    null,
    2
  );
}

export async function listenAutopilotEvents(
  handler: (evt: AutopilotEvent) => void
): Promise<UnlistenFn> {
  if (isTauriRuntime()) {
    return listen<AutopilotEvent>("autopilot-event", (e) => handler(e.payload));
  }
  browserAutopilotListeners.add(handler);
  return () => {
    browserAutopilotListeners.delete(handler);
  };
}
