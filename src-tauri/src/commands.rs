use crate::llm::{self, AutopilotController};
use crate::mcp::registry;
use crate::models::{
    CfdCase, CflInput, CflResult, EnvironmentStatus, FluidConfig, GeneratedCaseFiles,
    GuardrailsConfig, LlmConfig, OptimizationGoal, ParametricGeometrySpec, SessionState,
    StlMeshPayload, TelemetrySnapshot, TurbulenceIcInput, TurbulenceIcResult, ValidationReport,
    YPlusInput, YPlusResult,
};
use crate::openfoam::{generator, parser, runner, validator};
use crate::parametric;
use crate::physics;
use crate::session;
use crate::stl;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
pub fn cmd_compute_telemetry(case: CfdCase) -> TelemetrySnapshot {
    physics::compute_telemetry(&case)
}

#[tauri::command]
pub fn cmd_compute_y_plus(input: YPlusInput) -> Result<YPlusResult, String> {
    physics::compute_y_plus(&input).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_compute_turbulence_ic(input: TurbulenceIcInput) -> Result<TurbulenceIcResult, String> {
    physics::compute_turbulence_ic(&input).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_compute_cfl(input: CflInput) -> Result<CflResult, String> {
    physics::compute_cfl(&input).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_compute_air_sutherland(
    temperature_k: f64,
    pressure_pa: f64,
) -> Result<FluidConfig, String> {
    physics::compute_air_sutherland(temperature_k, pressure_pa).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_generate_case(case: CfdCase) -> GeneratedCaseFiles {
    generator::generate_case_files(&case)
}

#[tauri::command]
pub fn cmd_validate_case(case: CfdCase) -> ValidationReport {
    validator::validate_case(&case)
}

#[tauri::command]
pub fn cmd_export_case(case: CfdCase, target_dir: String) -> Result<usize, String> {
    let path = PathBuf::from(target_dir);
    generator::export_case_to_disk(&case, &path).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_import_case(source_dir: String) -> Result<CfdCase, String> {
    let path = PathBuf::from(source_dir);
    parser::import_case_from_disk(&path).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_load_stl(path: String) -> Result<StlMeshPayload, String> {
    let pb = PathBuf::from(path);
    stl::parse_stl_file(&pb).map_err(Into::into)
}

#[tauri::command]
pub fn cmd_generate_parametric_geometry(
    spec: ParametricGeometrySpec,
) -> Result<StlMeshPayload, String> {
    parametric::generate_parametric_stl(&spec)
        .map(|(payload, _)| payload)
        .map_err(Into::into)
}

#[tauri::command]
pub fn cmd_detect_environment() -> EnvironmentStatus {
    runner::detect_environment()
}

#[tauri::command]
pub async fn cmd_start_solver(
    app: AppHandle,
    state: State<'_, Arc<runner::RunnerState>>,
    case: CfdCase,
    backend: String,
    case_dir: Option<String>,
) -> Result<String, String> {
    runner::launch_solver(app, state.inner().clone(), case, backend, case_dir)
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub fn cmd_stop_solver(state: State<'_, Arc<runner::RunnerState>>) -> Result<(), String> {
    state.cancel_flag.store(true, Ordering::SeqCst);
    Ok(())
}

// --- Shared Session, MCP & LLM Autopilot Commands ---

#[tauri::command]
pub fn cmd_get_session() -> SessionState {
    let path = session::default_session_path();
    session::load_or_init_session(&path)
}

#[tauri::command]
pub fn cmd_sync_session_case(
    case: CfdCase,
    guardrails: GuardrailsConfig,
    goal: OptimizationGoal,
) -> Result<SessionState, String> {
    let path = session::default_session_path();
    let mut state = session::load_or_init_session(&path);
    state.case = case;
    state.guardrails = guardrails;
    state.goal = goal;
    session::save_session_atomic(&path, &mut state, "user")?;
    Ok(state)
}

#[tauri::command]
pub async fn cmd_execute_mcp_tool(
    tool_name: String,
    arguments: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let path = session::default_session_path();
    registry::execute_tool(&path, &tool_name, arguments, "user")
        .await
        .map_err(Into::into)
}

#[tauri::command]
pub async fn cmd_start_autopilot(
    app: AppHandle,
    ctrl: State<'_, Arc<AutopilotController>>,
    llm_config: LlmConfig,
    goal: OptimizationGoal,
) -> Result<(), String> {
    let session_path = session::default_session_path();
    let ctrl_arc = ctrl.inner().clone();
    tokio::spawn(async move {
        let _ = llm::run_autopilot_loop(app, ctrl_arc, session_path, llm_config, goal).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn cmd_start_builtin_autopilot(
    app: AppHandle,
    ctrl: State<'_, Arc<AutopilotController>>,
    goal: OptimizationGoal,
    backend: String,
) -> Result<(), String> {
    let session_path = session::default_session_path();
    let ctrl_arc = ctrl.inner().clone();
    tokio::spawn(async move {
        let _ =
            llm::run_builtin_deterministic_loop(app, ctrl_arc, session_path, goal, backend).await;
    });
    Ok(())
}

#[tauri::command]
pub async fn cmd_approve_proposal(
    ctrl: State<'_, Arc<AutopilotController>>,
    approved: bool,
) -> Result<(), String> {
    let mut guard = ctrl.pending_tx.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(approved);
    }
    Ok(())
}

#[tauri::command]
pub async fn cmd_stop_autopilot(
    ctrl: State<'_, Arc<AutopilotController>>,
) -> Result<(), String> {
    ctrl.cancel_flag.store(true, Ordering::SeqCst);
    let mut guard = ctrl.pending_tx.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(false);
    }
    Ok(())
}

#[tauri::command]
pub fn cmd_rollback_iteration(iteration_index: Option<u32>) -> Result<SessionState, String> {
    let path = session::default_session_path();
    let mut state = session::load_or_init_session(&path);
    session::rollback_to_iteration(&mut state, iteration_index)?;
    session::save_session_atomic(&path, &mut state, "user")?;
    Ok(state)
}

#[tauri::command]
pub fn cmd_get_mcp_config_snippet() -> String {
    let bin_name = if cfg!(windows) {
        "venturi-mcp.exe"
    } else {
        "venturi-mcp"
    };
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let bin_path = exe_dir
        .map(|d| d.join(bin_name))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(bin_name));
    let session_path = session::default_session_path();
    serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "venturi": {
                "command": bin_path.display().to_string(),
                "args": ["--session", session_path.display().to_string()]
            }
        }
    }))
    .unwrap_or_default()
}
