export type OpenFoamFlavor = "foundationV14" | "openCfdV2606";

export type FlowRegime =
  | "steadyIncompressible"
  | "transientIncompressible"
  | "steadyCompressible";

export type TurbulenceModel =
  | "laminar"
  | "kOmegaSST"
  | "kEpsilon"
  | "spalartAllmaras"
  | "waleLes";

export type SchemeAccuracy =
  | "robustUpwind"
  | "balancedSecondOrder"
  | "highResolution";

export type BoundaryKind =
  | "velocityInlet"
  | "pressureOutlet"
  | "noSlipWall"
  | "movingWall"
  | "symmetry"
  | "empty2D";

export type DomainFace =
  | "xMin"
  | "xMax"
  | "yMin"
  | "yMax"
  | "zMin"
  | "zMax"
  | "obstacle";

export type PresetGeometryKind =
  | "none"
  | "airfoilNaca0012"
  | "ahmedBody"
  | "venturiNozzle"
  | "cylinderCrossflow"
  | "customStl"
  | "parametric";

export type ParametricGeometrySpec =
  | {
      family: "nacaAirfoil";
      chord: number;
      camberPct: number;
      camberPosPct: number;
      thicknessPct: number;
      aoaDeg: number;
      span: number;
    }
  | {
      family: "venturiTube";
      inletRadius: number;
      throatRadius: number;
      outletRadius: number;
      convergentLen: number;
      throatLen: number;
      divergentLen: number;
    }
  | {
      family: "ahmedDiffuser";
      length: number;
      width: number;
      height: number;
      rearSlantDeg: number;
      groundClearance: number;
    }
  | {
      family: "pinFinArray";
      baseLength: number;
      baseWidth: number;
      finCountX: number;
      finCountZ: number;
      finRadius: number;
      finHeight: number;
    }
  | {
      family: "cylinder";
      radius: number;
      span: number;
    };

export interface RefinementBox {
  name: string;
  min: [number, number, number];
  max: [number, number, number];
  level: number;
  reason: string;
}

export interface DomainBox {
  xMin: number;
  xMax: number;
  yMin: number;
  yMax: number;
  zMin: number;
  zMax: number;
}

export interface MeshConfig {
  cellsX: number;
  cellsY: number;
  cellsZ: number;
  gradingY: number;
  snappyEnabled: boolean;
  refinementMin: number;
  refinementMax: number;
  inflationLayers: number;
  firstLayerThickness: number;
  expansionRatio: number;
  targetYPlus: number;
  refinementBoxes: RefinementBox[];
}

export interface FluidConfig {
  name: string;
  density: number;
  dynamicViscosity: number;
  kinematicViscosity: number;
  temperatureK: number;
  pressurePa: number;
  gamma: number;
  gasConstant: number;
  isGas: boolean;
}

export interface TurbulenceIC {
  intensity: number;
  lengthScale: number;
  k: number;
  epsilon: number;
  omega: number;
  nut: number;
  nuTilda: number;
}

export interface PhysicsConfig {
  regime: FlowRegime;
  turbulence: TurbulenceModel;
  fluid: FluidConfig;
  referenceVelocity: number;
  referenceLength: number;
  turbulenceIc: TurbulenceIC;
  gravityEnabled: boolean;
  gravity: [number, number, number];
}

export interface BoundaryPatch {
  name: string;
  face: DomainFace;
  kind: BoundaryKind;
  velocity: [number, number, number];
  pressure: number;
  wallRoughnessKs: number;
}

export interface NumericsConfig {
  accuracy: SchemeAccuracy;
  startTime: number;
  endTime: number;
  deltaT: number;
  writeInterval: number;
  relaxP: number;
  relaxU: number;
  relaxTurb: number;
  convergenceTol: number;
  maxCo: number;
  nNonOrthoCorrectors: number;
  parallelCores: number;
}

export interface StlSummary {
  filename: string;
  triangleCount: number;
  isBinary: boolean;
  boundsMin: [number, number, number];
  boundsMax: [number, number, number];
  surfaceArea: number;
  volumeEst: number;
  scaleFactor?: number;
  sourcePath?: string | null;
}

export interface StlMeshPayload {
  summary: StlSummary;
  positions: number[];
  normals: number[];
}

export interface CfdCase {
  name: string;
  description: string;
  flavor: OpenFoamFlavor;
  geometryKind: PresetGeometryKind;
  parametric: ParametricGeometrySpec | null;
  stl: StlSummary | null;
  domain: DomainBox;
  mesh: MeshConfig;
  physics: PhysicsConfig;
  boundaries: BoundaryPatch[];
  numerics: NumericsConfig;
}

export interface YPlusInput {
  velocity: number;
  density: number;
  dynamicViscosity: number;
  referenceLength: number;
  targetYPlus: number;
  expansionRatio: number;
}

export interface YPlusResult {
  reynolds: number;
  flowRegimeLabel: string;
  skinFrictionCf: number;
  wallShearTauW: number;
  frictionVelocityUTau: number;
  firstCellHeight: number;
  wallDistanceYc: number;
  boundaryLayerDelta99: number;
  recommendedLayers: number;
  totalPrismHeight: number;
  wallTreatmentNote: string;
  bufferLayerWarning: boolean;
}

export interface TurbulenceIcInput {
  velocity: number;
  intensityPercent: number;
  lengthScale: number;
  kinematicViscosity: number;
}

export interface TurbulenceIcResult {
  k: number;
  epsilon: number;
  omega: number;
  nut: number;
  nuTilda: number;
  viscosityRatio: number;
}

export interface CflInput {
  minCellSize: number;
  maxVelocity: number;
  targetCourant: number;
  domainLength: number;
}

export interface CflResult {
  maxDeltaT: number;
  flowThroughTime: number;
  recommendedEndTime: number;
  estimatedSteps: number;
  stabilityNote: string;
}

export interface TelemetrySnapshot {
  reynolds: number;
  mach: number;
  speedOfSound: number;
  estimatedCells: number;
  estimatedRamMb: number;
  minDx: number;
  estimatedCourant: number;
  firstCellHeightMm: number;
  recommendedCores: number;
  compressibleWarning: boolean;
  solverName: string;
}

export type DiagnosticSeverity = "pass" | "info" | "warning" | "error";

export interface ValidationItem {
  id: string;
  category: string;
  severity: DiagnosticSeverity;
  title: string;
  detail: string;
  suggestion: string | null;
}

export interface ValidationReport {
  score: number;
  readyToRun: boolean;
  errorCount: number;
  warningCount: number;
  items: ValidationItem[];
}

export interface GeneratedCaseFiles {
  files: Record<string, string>;
  solverCommand: string;
}

export interface EnvironmentStatus {
  localOpenfoam: string | null;
  localBinary: string | null;
  macosAppPath: string | null;
  dockerInstalled: boolean;
  dockerRunning: boolean;
  recommendedBackend: string;
  platform: string;
  cpuCores: number;
}

export interface ResidualPoint {
  iteration: number;
  ux: number | null;
  uy: number | null;
  uz: number | null;
  p: number | null;
  k: number | null;
  omega: number | null;
  epsilon: number | null;
  cd: number | null;
  cl: number | null;
  continuity: number | null;
}

export interface SolverEvent {
  runId: string;
  kind: "log" | "residual" | "status";
  message: string | null;
  residual: ResidualPoint | null;
  status: "running" | "converged" | "completed" | "stopped" | "error" | null;
}

// --- Autonomous Loop, Output Ingestion & MCP Models ---

export interface CheckMeshSummary {
  cells: number;
  maxNonOrthogonality: number;
  avgNonOrthogonality: number;
  maxSkewness: number;
  maxAspectRatio: number;
  failedChecks: number;
  meshOk: boolean;
}

export interface PatchYPlus {
  patch: string;
  min: number;
  max: number;
  average: number;
}

export interface GradientHotspot {
  id: string;
  center: [number, number, number];
  suggestedBoxMin: [number, number, number];
  suggestedBoxMax: [number, number, number];
  gradientScore: number;
  suggestedLevel: number;
  reason: string;
}

export interface OpenFoamResultSummary {
  backendUsed: string;
  converged: boolean;
  diverged: boolean;
  iterationsCompleted: number;
  finalResidualU: number;
  finalResidualP: number;
  continuityError: number;
  cd: number | null;
  cl: number | null;
  cm: number | null;
  deltaPPa: number | null;
  checkMesh: CheckMeshSummary;
  yPlusPatches: PatchYPlus[];
  hotspots: GradientHotspot[];
  notes: string[];
}

export interface GuardrailsConfig {
  maxIterations: number;
  maxCells: number;
  maxRamMb: number;
  maxRefinementLevel: number;
  targetYPlusMin: number;
  targetYPlusMax: number;
  maxSkewness: number;
  maxNonOrtho: number;
  autoRollback: boolean;
}

export interface OptimizationGoal {
  prompt: string;
  targetYPlus: number | null;
  maxDeltaPPa: number | null;
  minimizeCd: boolean;
  maximizeClOverCd: boolean;
}

export interface IterationRecord {
  index: number;
  timestamp: string;
  label: string;
  changesSummary: string;
  caseSnapshot: CfdCase;
  results: OpenFoamResultSummary;
  objectiveScore: number;
  isBest: boolean;
  rolledBack: boolean;
}

export interface McpToolLogEntry {
  id: string;
  timestamp: string;
  source: string;
  toolName: string;
  argsPreview: string;
  ok: boolean;
  summary: string;
}

export interface SessionState {
  version: number;
  updatedAt: string;
  lastWriter: string;
  case: CfdCase;
  guardrails: GuardrailsConfig;
  goal: OptimizationGoal;
  iterations: IterationRecord[];
  bestIterationIndex: number | null;
  toolLogs: McpToolLogEntry[];
  caseDir: string | null;
}

export type LlmProviderKind = "gemini" | "anthropic" | "openAi" | "ollama";
export type AutopilotMode = "autopilot" | "review";

export interface LlmConfig {
  provider: LlmProviderKind;
  apiKey: string;
  model: string;
  baseUrl: string;
  mode: AutopilotMode;
  backend: string;
}

export interface PendingProposal {
  id: string;
  toolName: string;
  arguments: Record<string, unknown>;
  rationale: string;
}

export interface AutopilotEvent {
  kind: "thinking" | "tool_call" | "tool_result" | "proposal" | "state_sync" | "done" | "error";
  message: string;
  toolName: string | null;
  proposal: PendingProposal | null;
}

export type WorkflowTab =
  | "geometry"
  | "mesh"
  | "physics"
  | "boundaries"
  | "numerics"
  | "lab"
  | "run"
  | "autopilot";
