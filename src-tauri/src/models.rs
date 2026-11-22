use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OpenFoamFlavor {
    /// OpenFOAM Foundation (openfoam.org) v12 / v13 / v14 — modular foamRun
    FoundationV14,
    /// OpenCFD / Keysight (openfoam.com) v2412 / v2512 / v2606
    OpenCfdV2606,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlowRegime {
    SteadyIncompressible,
    TransientIncompressible,
    SteadyCompressible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TurbulenceModel {
    Laminar,
    KOmegaSST,
    KEpsilon,
    SpalartAllmaras,
    WaleLes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SchemeAccuracy {
    RobustUpwind,
    BalancedSecondOrder,
    HighResolution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BoundaryKind {
    VelocityInlet,
    PressureOutlet,
    NoSlipWall,
    MovingWall,
    Symmetry,
    Empty2D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DomainFace {
    XMin,
    XMax,
    YMin,
    YMax,
    ZMin,
    ZMax,
    Obstacle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresetGeometryKind {
    None,
    AirfoilNaca0012,
    AhmedBody,
    VenturiNozzle,
    CylinderCrossflow,
    CustomStl,
    Parametric,
}

/// Native Rust parametric geometry specifications (compiled to watertight Binary STL).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "family", rename_all = "camelCase")]
pub enum ParametricGeometrySpec {
    #[serde(rename_all = "camelCase")]
    NacaAirfoil {
        chord: f64,
        camber_pct: f64,     // 0.0..6.0 (e.g. 2.0 for NACA 2412)
        camber_pos_pct: f64, // 20.0..60.0 (e.g. 40.0 for NACA 2412)
        thickness_pct: f64,  // 6.0..24.0 (e.g. 12.0)
        aoa_deg: f64,        // -12.0..18.0
        span: f64,
    },
    #[serde(rename_all = "camelCase")]
    VenturiTube {
        inlet_radius: f64,
        throat_radius: f64,
        outlet_radius: f64,
        convergent_len: f64,
        throat_len: f64,
        divergent_len: f64,
    },
    #[serde(rename_all = "camelCase")]
    AhmedDiffuser {
        length: f64,
        width: f64,
        height: f64,
        rear_slant_deg: f64,
        ground_clearance: f64,
    },
    #[serde(rename_all = "camelCase")]
    PinFinArray {
        base_length: f64,
        base_width: f64,
        fin_count_x: u32,
        fin_count_z: u32,
        fin_radius: f64,
        fin_height: f64,
    },
    #[serde(rename_all = "camelCase")]
    Cylinder {
        radius: f64,
        span: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefinementBox {
    pub name: String,
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub level: u32,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainBox {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub z_min: f64,
    pub z_max: f64,
}

impl DomainBox {
    pub fn lx(&self) -> f64 {
        (self.x_max - self.x_min).abs().max(1e-6)
    }
    pub fn ly(&self) -> f64 {
        (self.y_max - self.y_min).abs().max(1e-6)
    }
    pub fn lz(&self) -> f64 {
        (self.z_max - self.z_min).abs().max(1e-6)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshConfig {
    pub cells_x: u32,
    pub cells_y: u32,
    pub cells_z: u32,
    pub grading_y: f64,
    pub snappy_enabled: bool,
    pub refinement_min: u32,
    pub refinement_max: u32,
    pub inflation_layers: u32,
    pub first_layer_thickness: f64,
    pub expansion_ratio: f64,
    pub target_y_plus: f64,
    #[serde(default)]
    pub refinement_boxes: Vec<RefinementBox>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FluidConfig {
    pub name: String,
    pub density: f64,             // kg/m^3
    pub dynamic_viscosity: f64,   // Pa*s
    pub kinematic_viscosity: f64, // m^2/s
    pub temperature_k: f64,       // K
    pub pressure_pa: f64,         // Pa
    pub gamma: f64,               // Cp/Cv
    pub gas_constant: f64,        // J/(kg*K)
    pub is_gas: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurbulenceIC {
    pub intensity: f64,    // fraction e.g. 0.05 = 5%
    pub length_scale: f64, // m
    pub k: f64,            // m^2/s^2
    pub epsilon: f64,      // m^2/s^3
    pub omega: f64,        // 1/s
    pub nut: f64,          // m^2/s
    pub nu_tilda: f64,     // m^2/s
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhysicsConfig {
    pub regime: FlowRegime,
    pub turbulence: TurbulenceModel,
    pub fluid: FluidConfig,
    pub reference_velocity: f64,
    pub reference_length: f64,
    pub turbulence_ic: TurbulenceIC,
    pub gravity_enabled: bool,
    pub gravity: [f64; 3],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoundaryPatch {
    pub name: String,
    pub face: DomainFace,
    pub kind: BoundaryKind,
    pub velocity: [f64; 3],
    pub pressure: f64,
    pub wall_roughness_ks: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NumericsConfig {
    pub accuracy: SchemeAccuracy,
    pub start_time: f64,
    pub end_time: f64,
    pub delta_t: f64,
    pub write_interval: f64,
    pub relax_p: f64,
    pub relax_u: f64,
    pub relax_turb: f64,
    pub convergence_tol: f64,
    pub max_co: f64,
    pub n_non_ortho_correctors: u32,
    pub parallel_cores: u32,
}

fn default_scale_factor() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StlSummary {
    pub filename: String,
    pub triangle_count: usize,
    pub is_binary: bool,
    pub bounds_min: [f64; 3],
    pub bounds_max: [f64; 3],
    pub surface_area: f64,
    pub volume_est: f64,
    #[serde(default = "default_scale_factor")]
    pub scale_factor: f64,
    #[serde(default)]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CfdCase {
    pub name: String,
    pub description: String,
    pub flavor: OpenFoamFlavor,
    pub geometry_kind: PresetGeometryKind,
    #[serde(default)]
    pub parametric: Option<ParametricGeometrySpec>,
    pub stl: Option<StlSummary>,
    pub domain: DomainBox,
    pub mesh: MeshConfig,
    pub physics: PhysicsConfig,
    pub boundaries: Vec<BoundaryPatch>,
    pub numerics: NumericsConfig,
}

// --- Engineering Calculator Inputs & Outputs ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YPlusInput {
    pub velocity: f64,
    pub density: f64,
    pub dynamic_viscosity: f64,
    pub reference_length: f64,
    pub target_y_plus: f64,
    pub expansion_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YPlusResult {
    pub reynolds: f64,
    pub flow_regime_label: String,
    pub skin_friction_cf: f64,
    pub wall_shear_tau_w: f64,
    pub friction_velocity_u_tau: f64,
    pub first_cell_height: f64,
    pub wall_distance_yc: f64,
    pub boundary_layer_delta99: f64,
    pub recommended_layers: u32,
    pub total_prism_height: f64,
    pub wall_treatment_note: String,
    pub buffer_layer_warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurbulenceIcInput {
    pub velocity: f64,
    pub intensity_percent: f64,
    pub length_scale: f64,
    pub kinematic_viscosity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurbulenceIcResult {
    pub k: f64,
    pub epsilon: f64,
    pub omega: f64,
    pub nut: f64,
    pub nu_tilda: f64,
    pub viscosity_ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CflInput {
    pub min_cell_size: f64,
    pub max_velocity: f64,
    pub target_courant: f64,
    pub domain_length: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CflResult {
    pub max_delta_t: f64,
    pub flow_through_time: f64,
    pub recommended_end_time: f64,
    pub estimated_steps: u64,
    pub stability_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    pub reynolds: f64,
    pub mach: f64,
    pub speed_of_sound: f64,
    pub estimated_cells: u64,
    pub estimated_ram_mb: f64,
    pub min_dx: f64,
    pub estimated_courant: f64,
    pub first_cell_height_mm: f64,
    pub recommended_cores: u32,
    pub compressible_warning: bool,
    pub solver_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiagnosticSeverity {
    Pass,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationItem {
    pub id: String,
    pub category: String,
    pub severity: DiagnosticSeverity,
    pub title: String,
    pub detail: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub score: u32,
    pub ready_to_run: bool,
    pub error_count: usize,
    pub warning_count: usize,
    pub items: Vec<ValidationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedCaseFiles {
    pub files: BTreeMap<String, String>,
    pub solver_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentStatus {
    pub local_openfoam: Option<String>,
    pub local_binary: Option<String>,
    pub macos_app_path: Option<String>,
    pub docker_installed: bool,
    pub docker_running: bool,
    pub recommended_backend: String,
    pub platform: String,
    pub cpu_cores: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StlMeshPayload {
    pub summary: StlSummary,
    /// Flat xyz vertex array for Three.js BufferAttribute (downsampled if huge)
    pub positions: Vec<f32>,
    /// Flat xyz normal array
    pub normals: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResidualPoint {
    pub iteration: f64,
    pub ux: Option<f64>,
    pub uy: Option<f64>,
    pub uz: Option<f64>,
    pub p: Option<f64>,
    pub k: Option<f64>,
    pub omega: Option<f64>,
    pub epsilon: Option<f64>,
    pub cd: Option<f64>,
    pub cl: Option<f64>,
    pub continuity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolverEvent {
    pub run_id: String,
    pub kind: String, // "log" | "residual" | "status"
    pub message: Option<String>,
    pub residual: Option<ResidualPoint>,
    pub status: Option<String>, // "running" | "converged" | "completed" | "stopped" | "error"
}

// --- OpenFOAM Output Ingestion & Autonomous Loop Models ---

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckMeshSummary {
    pub cells: u64,
    pub max_non_orthogonality: f64,
    pub avg_non_orthogonality: f64,
    pub max_skewness: f64,
    pub max_aspect_ratio: f64,
    pub failed_checks: u32,
    pub mesh_ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatchYPlus {
    pub patch: String,
    pub min: f64,
    pub max: f64,
    pub average: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradientHotspot {
    pub id: String,
    pub center: [f64; 3],
    pub suggested_box_min: [f64; 3],
    pub suggested_box_max: [f64; 3],
    pub gradient_score: f64,
    pub suggested_level: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenFoamResultSummary {
    pub backend_used: String,
    pub converged: bool,
    pub diverged: bool,
    pub iterations_completed: u32,
    pub final_residual_u: f64,
    pub final_residual_p: f64,
    pub continuity_error: f64,
    pub cd: Option<f64>,
    pub cl: Option<f64>,
    pub cm: Option<f64>,
    pub delta_p_pa: Option<f64>,
    pub check_mesh: CheckMeshSummary,
    pub y_plus_patches: Vec<PatchYPlus>,
    pub hotspots: Vec<GradientHotspot>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardrailsConfig {
    pub max_iterations: u32,
    pub max_cells: u64,
    pub max_ram_mb: f64,
    pub max_refinement_level: u32,
    pub target_y_plus_min: f64,
    pub target_y_plus_max: f64,
    pub max_skewness: f64,
    pub max_non_ortho: f64,
    pub auto_rollback: bool,
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            max_iterations: 8,
            max_cells: 2_000_000,
            max_ram_mb: 4096.0,
            max_refinement_level: 4,
            target_y_plus_min: 0.5,
            target_y_plus_max: 5.0,
            max_skewness: 4.0,
            max_non_ortho: 65.0,
            auto_rollback: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizationGoal {
    pub prompt: String,
    pub target_y_plus: Option<f64>,
    pub max_delta_p_pa: Option<f64>,
    pub minimize_cd: bool,
    pub maximize_cl_over_cd: bool,
}

impl Default for OptimizationGoal {
    fn default() -> Self {
        Self {
            prompt: "Optimize geometry and adapt mesh refinement boxes + boundary layers to reach target y+ and minimize drag / pressure drop.".into(),
            target_y_plus: Some(1.5),
            max_delta_p_pa: None,
            minimize_cd: true,
            maximize_cl_over_cd: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IterationRecord {
    pub index: u32,
    pub timestamp: String,
    pub label: String,
    pub changes_summary: String,
    pub case_snapshot: CfdCase,
    pub results: OpenFoamResultSummary,
    pub objective_score: f64,
    pub is_best: bool,
    pub rolled_back: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolLogEntry {
    pub id: String,
    pub timestamp: String,
    pub source: String, // "mcp-external" | "in-app-autopilot" | "user"
    pub tool_name: String,
    pub args_preview: String,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub version: u64,
    pub updated_at: String,
    pub last_writer: String,
    pub case: CfdCase,
    pub guardrails: GuardrailsConfig,
    pub goal: OptimizationGoal,
    pub iterations: Vec<IterationRecord>,
    pub best_iteration_index: Option<u32>,
    pub tool_logs: Vec<McpToolLogEntry>,
    pub case_dir: Option<String>,
}

// --- Multi-Provider In-App LLM Orchestrator Models ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LlmProviderKind {
    Gemini,
    Anthropic,
    OpenAi,
    Ollama,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutopilotMode {
    Autopilot,
    Review,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmConfig {
    pub provider: LlmProviderKind,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub mode: AutopilotMode,
    pub backend: String, // "demo" | "local" | "docker"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingProposal {
    pub id: String,
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutopilotEvent {
    pub kind: String, // "thinking" | "tool_call" | "tool_result" | "proposal" | "state_sync" | "done" | "error"
    pub message: String,
    pub tool_name: Option<String>,
    pub proposal: Option<PendingProposal>,
}
