pub mod commands;
pub mod error;
pub mod llm;
pub mod mcp;
pub mod models;
pub mod openfoam;
pub mod parametric;
pub mod physics;
pub mod session;
pub mod stl;

use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let runner_state = Arc::new(openfoam::runner::RunnerState::default());
    let autopilot_ctrl = Arc::new(llm::AutopilotController::default());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(runner_state)
        .manage(autopilot_ctrl)
        .invoke_handler(tauri::generate_handler![
            commands::cmd_compute_telemetry,
            commands::cmd_compute_y_plus,
            commands::cmd_compute_turbulence_ic,
            commands::cmd_compute_cfl,
            commands::cmd_compute_air_sutherland,
            commands::cmd_generate_case,
            commands::cmd_validate_case,
            commands::cmd_export_case,
            commands::cmd_import_case,
            commands::cmd_load_stl,
            commands::cmd_generate_parametric_geometry,
            commands::cmd_detect_environment,
            commands::cmd_start_solver,
            commands::cmd_stop_solver,
            commands::cmd_get_session,
            commands::cmd_sync_session_case,
            commands::cmd_execute_mcp_tool,
            commands::cmd_start_autopilot,
            commands::cmd_start_builtin_autopilot,
            commands::cmd_approve_proposal,
            commands::cmd_stop_autopilot,
            commands::cmd_rollback_iteration,
            commands::cmd_get_mcp_config_snippet,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Venturi CFD application");
}
