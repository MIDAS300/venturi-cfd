use crate::error::VenturiError;
use crate::mcp::registry::{execute_tool, tool_definitions};
use crate::models::{
    AutopilotEvent, AutopilotMode, BoundaryKind, LlmConfig, LlmProviderKind, OptimizationGoal,
    ParametricGeometrySpec, PendingProposal, PresetGeometryKind, TurbulenceModel,
};
use crate::physics::compute_telemetry;
use crate::session::{load_or_init_session, save_session_atomic};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::{oneshot, Mutex};

const AUTOPILOT_CHANNEL: &str = "autopilot-event";

/// Shared controller for cancelling active autopilot runs or approving Review-mode tool proposals.
#[derive(Default)]
pub struct AutopilotController {
    pub cancel_flag: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    pub pending_tx: Mutex<Option<oneshot::Sender<bool>>>,
}

struct RunningGuard {
    ctrl: Arc<AutopilotController>,
}

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.ctrl.running.store(false, Ordering::SeqCst);
        if let Ok(mut guard) = self.ctrl.pending_tx.try_lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(false);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ParsedToolCall {
    id: String,
    name: String,
    arguments: Value,
}

/// Runs the multi-provider LLM closed-loop CFD autopilot (Gemini, Anthropic, OpenAI, Ollama)
/// driving Venturi's MCP tool registry and streaming live events to the Tauri GUI.
pub async fn run_autopilot_loop(
    app: AppHandle,
    ctrl: Arc<AutopilotController>,
    session_path: PathBuf,
    llm_cfg: LlmConfig,
    goal: OptimizationGoal,
) -> Result<(), VenturiError> {
    if ctrl.running.swap(true, Ordering::SeqCst) {
        return Err(VenturiError::Execution(
            "An autopilot loop is already active. Stop it before launching a new one.".into(),
        ));
    }
    ctrl.cancel_flag.store(false, Ordering::SeqCst);
    let _guard = RunningGuard { ctrl: ctrl.clone() };

    if llm_cfg.provider != LlmProviderKind::Ollama && llm_cfg.api_key.trim().is_empty() {
        let msg = "API key is required for the selected LLM provider (or switch to Ollama / Built-In Loop).";
        emit_event(&app, "error", msg.into(), None, None);
        return Err(VenturiError::Execution(msg.into()));
    }

    // Persist optimization goal into session before starting
    let mut state = load_or_init_session(&session_path);
    state.goal = goal.clone();
    save_session_atomic(&session_path, &mut state, "in-app-autopilot")?;
    emit_event(
        &app,
        "state_sync",
        format!("Synchronized optimization goal (session v{})", state.version),
        None,
        None,
    );

    let system_prompt = build_cfd_system_prompt(&state.guardrails, &goal, &llm_cfg.backend);
    let initial_user_prompt = build_initial_user_message(&state, &goal, &llm_cfg.backend);

    let max_turns = ((state.guardrails.max_iterations as usize) * 5).clamp(14, 42);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| VenturiError::Execution(format!("Failed to build HTTP client: {}", e)))?;

    emit_event(
        &app,
        "thinking",
        format!(
            "Starting {:?} autopilot ({}) in {:?} mode...",
            llm_cfg.provider, llm_cfg.model, llm_cfg.mode
        ),
        None,
        None,
    );

    let loop_result = match llm_cfg.provider {
        LlmProviderKind::Gemini => {
            run_gemini_loop(
                &app,
                &ctrl,
                &client,
                &session_path,
                &llm_cfg,
                &system_prompt,
                &initial_user_prompt,
                max_turns,
            )
            .await
        }
        LlmProviderKind::Anthropic => {
            run_anthropic_loop(
                &app,
                &ctrl,
                &client,
                &session_path,
                &llm_cfg,
                &system_prompt,
                &initial_user_prompt,
                max_turns,
            )
            .await
        }
        LlmProviderKind::OpenAi | LlmProviderKind::Ollama => {
            run_openai_compatible_loop(
                &app,
                &ctrl,
                &client,
                &session_path,
                &llm_cfg,
                &system_prompt,
                &initial_user_prompt,
                max_turns,
            )
            .await
        }
    };

    if let Err(ref e) = loop_result {
        emit_event(&app, "error", format!("Autopilot error: {}", e), None, None);
    }

    loop_result
}

/// Zero-API-key deterministic engineering autopilot executing a genuine multi-step
/// closed CFD loop through `mcp::registry::execute_tool`.
pub async fn run_builtin_deterministic_loop(
    app: AppHandle,
    ctrl: Arc<AutopilotController>,
    session_path: PathBuf,
    goal: OptimizationGoal,
    backend: String,
) -> Result<(), VenturiError> {
    if ctrl.running.swap(true, Ordering::SeqCst) {
        return Err(VenturiError::Execution(
            "An autopilot loop is already active. Stop it before launching a new one.".into(),
        ));
    }
    ctrl.cancel_flag.store(false, Ordering::SeqCst);
    let _guard = RunningGuard { ctrl: ctrl.clone() };

    let backend_norm = match backend.trim().to_lowercase().as_str() {
        "local" => "local".to_string(),
        "docker" => "docker".to_string(),
        "auto" => "auto".to_string(),
        _ => "demo".to_string(),
    };

    // Persist goal to shared session
    let mut state = load_or_init_session(&session_path);
    state.goal = goal.clone();
    save_session_atomic(&session_path, &mut state, "in-app-autopilot")?;
    emit_event(
        &app,
        "state_sync",
        format!("Goal locked into session v{}. Launching Built-In Closed Loop.", state.version),
        None,
        None,
    );

    // Phase 1: Inspect workspace & ensure valid geometry + domain
    emit_event(
        &app,
        "thinking",
        "Phase 1: Auditing workspace state, geometry bounds, and pre-flight validation...".into(),
        None,
        None,
    );
    let _ = run_logged_tool(&app, &session_path, "venturi_get_state", json!({})).await?;

    if is_cancelled(&ctrl) {
        emit_event(&app, "done", "Built-in loop stopped by user.".into(), None, None);
        return Ok(());
    }

    let current = load_or_init_session(&session_path);
    let has_inlet = current
        .case
        .boundaries
        .iter()
        .any(|b| b.kind == BoundaryKind::VelocityInlet);

    if current.case.geometry_kind == PresetGeometryKind::None
        && current.case.stl.is_none()
        && has_inlet
    {
        emit_event(
            &app,
            "thinking",
            "Blank workspace detected — synthesizing parametric NACA 2412 airfoil STL & fitting domain...".into(),
            None,
            None,
        );
        let _ = run_logged_tool(
            &app,
            &session_path,
            "venturi_generate_parametric_geometry",
            json!({
                "spec": {
                    "family": "nacaAirfoil",
                    "chord": 1.0,
                    "camberPct": 2.0,
                    "camberPosPct": 40.0,
                    "thicknessPct": 12.0,
                    "aoaDeg": 5.0,
                    "span": 0.76
                },
                "autoFitDomain": true
            }),
        )
        .await?;
    } else if let Some(ref spec) = current.case.parametric {
        if current.case.stl.is_none() {
            emit_event(
                &app,
                "thinking",
                "Compiling active parametric geometry to watertight Binary STL...".into(),
                None,
                None,
            );
            let spec_val = serde_json::to_value(spec).unwrap_or_else(|_| json!({}));
            let _ = run_logged_tool(
                &app,
                &session_path,
                "venturi_generate_parametric_geometry",
                json!({ "spec": spec_val, "autoFitDomain": false }),
            )
            .await?;
        }
    }

    let val_res = run_logged_tool(&app, &session_path, "venturi_validate_case", json!({})).await?;
    if !val_res["validation"]["readyToRun"].as_bool().unwrap_or(true) {
        emit_event(
            &app,
            "thinking",
            "Pre-flight check flagged domain fit issues — running venturi_auto_fit_domain...".into(),
            None,
            None,
        );
        let _ = run_logged_tool(
            &app,
            &session_path,
            "venturi_auto_fit_domain",
            json!({ "upstreamFactor": 2.5, "wakeFactor": 5.5, "lateralFactor": 2.2 }),
        )
        .await?;
    }

    if is_cancelled(&ctrl) {
        emit_event(&app, "done", "Built-in loop stopped by user.".into(), None, None);
        return Ok(());
    }

    // Phase 2A: Baseline solver run
    emit_event(
        &app,
        "thinking",
        format!(
            "Phase 2 (Step 1/3): Running baseline CFD iteration on '{}' backend...",
            backend_norm
        ),
        None,
        None,
    );
    let baseline = run_logged_tool(
        &app,
        &session_path,
        "venturi_run_iteration",
        json!({
            "label": "Baseline Run",
            "changesSummary": "Initial geometry & mesh baseline",
            "backend": backend_norm
        }),
    )
    .await?;
    let baseline_score = baseline["objectiveScore"].as_f64().unwrap_or(0.0);

    tokio::time::sleep(Duration::from_millis(180)).await;
    if is_cancelled(&ctrl) {
        emit_event(&app, "done", "Built-in loop stopped by user.".into(), None, None);
        return Ok(());
    }

    // Phase 2B: Inspect OpenFOAM outputs & apply boundary-layer + hotspot mesh adaptation
    emit_event(
        &app,
        "thinking",
        "Inspecting checkMesh, per-patch y+, force coefficients, and gradient hotspots...".into(),
        None,
        None,
    );
    let inspect1 = run_logged_tool(
        &app,
        &session_path,
        "venturi_inspect_openfoam_results",
        json!({}),
    )
    .await?;

    let state_after_1 = load_or_init_session(&session_path);
    let mut adaptations: Vec<String> = Vec::new();

    // Auto-size y+ for turbulent cases
    if state_after_1.case.physics.turbulence != TurbulenceModel::Laminar {
        let target_yp = goal
            .target_y_plus
            .unwrap_or(0.5 * (state_after_1.guardrails.target_y_plus_min + state_after_1.guardrails.target_y_plus_max))
            .clamp(
                state_after_1.guardrails.target_y_plus_min,
                state_after_1.guardrails.target_y_plus_max,
            );
        emit_event(
            &app,
            "thinking",
            format!("Auto-sizing boundary-layer prism mesh for target y+ = {:.2}...", target_yp),
            None,
            None,
        );
        if run_logged_tool(
            &app,
            &session_path,
            "venturi_auto_size_yplus",
            json!({
                "targetYPlus": target_yp,
                "expansionRatio": state_after_1.case.mesh.expansion_ratio.clamp(1.15, 1.22)
            }),
        )
        .await
        .is_ok()
        {
            adaptations.push(format!("y+ auto-sized to {:.1}", target_yp));
        }
    }

    // Place a snappyHexMesh refinement box on the top hotspot
    if let Some(hotspots) = inspect1["results"]["hotspots"].as_array() {
        if let Some(top) = hotspots.first() {
            let h_id = top["id"].as_str().unwrap_or("wake_hotspot");
            let bmin = top["suggestedBoxMin"].clone();
            let bmax = top["suggestedBoxMax"].clone();
            let max_lvl = state_after_1.guardrails.max_refinement_level.max(1);
            let desired_lvl = top["suggestedLevel"]
                .as_u64()
                .unwrap_or(3)
                .clamp(1, max_lvl as u64) as u32;
            let reason = top["reason"]
                .as_str()
                .unwrap_or("High flow-gradient hotspot identified in OpenFOAM probes")
                .to_string();

            emit_event(
                &app,
                "thinking",
                format!("Inserting L{} refinementRegions box on top hotspot '{}'...", desired_lvl, h_id),
                None,
                None,
            );

            let box_name = format!("hotspot_{}", h_id);
            let box_args = json!({
                "name": box_name,
                "min": bmin,
                "max": bmax,
                "level": desired_lvl,
                "reason": reason
            });
            if run_logged_tool(&app, &session_path, "venturi_upsert_refinement_box", box_args)
                .await
                .is_ok()
            {
                adaptations.push(format!("added L{} box '{}'", desired_lvl, box_name));
            } else if desired_lvl > 1 {
                let fallback_args = json!({
                    "name": box_name,
                    "min": top["suggestedBoxMin"],
                    "max": top["suggestedBoxMax"],
                    "level": desired_lvl - 1,
                    "reason": reason
                });
                if run_logged_tool(
                    &app,
                    &session_path,
                    "venturi_upsert_refinement_box",
                    fallback_args,
                )
                .await
                .is_ok()
                {
                    adaptations.push(format!("added L{} box '{}'", desired_lvl - 1, box_name));
                }
            }
        }
    }

    // Check mesh non-orthogonality / convergence for numerics adjustment
    let max_non_ortho = inspect1["results"]["checkMesh"]["maxNonOrthogonality"]
        .as_f64()
        .unwrap_or(0.0);
    let converged = inspect1["results"]["converged"].as_bool().unwrap_or(true);
    if max_non_ortho > 50.0 || !converged {
        if run_logged_tool(
            &app,
            &session_path,
            "venturi_update_numerics",
            json!({ "nNonOrthoCorrectors": 2, "relaxP": 0.25, "relaxU": 0.65 }),
        )
        .await
        .is_ok()
        {
            adaptations.push("increased non-ortho correctors to 2".into());
        }
    }

    if is_cancelled(&ctrl) {
        emit_event(&app, "done", "Built-in loop stopped by user.".into(), None, None);
        return Ok(());
    }

    // Run Iteration #2 if budget permits
    let check_budget_2 = load_or_init_session(&session_path);
    if (check_budget_2.iterations.len() as u32) < check_budget_2.guardrails.max_iterations {
        let change_desc = if adaptations.is_empty() {
            "Mesh & boundary-layer verification pass".to_string()
        } else {
            adaptations.join("; ")
        };
        emit_event(
            &app,
            "thinking",
            format!("Phase 2 (Step 2/3): Running mesh-adapted iteration ({})...", change_desc),
            None,
            None,
        );
        let _ = run_logged_tool(
            &app,
            &session_path,
            "venturi_run_iteration",
            json!({
                "label": "Adapted Mesh & y+",
                "changesSummary": change_desc,
                "backend": backend_norm
            }),
        )
        .await?;
    }

    tokio::time::sleep(Duration::from_millis(180)).await;
    if is_cancelled(&ctrl) {
        emit_event(&app, "done", "Built-in loop stopped by user.".into(), None, None);
        return Ok(());
    }

    // Phase 2C: Parametric shape refinement (if parametric geometry is active and budget permits)
    let state_after_2 = load_or_init_session(&session_path);
    if (state_after_2.iterations.len() as u32) < state_after_2.guardrails.max_iterations {
        if let Some(ref spec) = state_after_2.case.parametric {
            if let Some((mutated_spec, geom_note, need_autofit)) =
                propose_parametric_refinement(spec, &goal)
            {
                emit_event(
                    &app,
                    "thinking",
                    format!("Phase 2 (Step 3/3): Parametric geometry optimization — {}...", geom_note),
                    None,
                    None,
                );
                let spec_val = serde_json::to_value(&mutated_spec).unwrap_or_else(|_| json!({}));
                let _ = run_logged_tool(
                    &app,
                    &session_path,
                    "venturi_generate_parametric_geometry",
                    json!({ "spec": spec_val, "autoFitDomain": need_autofit }),
                )
                .await?;

                // Ensure domain clearance still passes validation
                let val_check =
                    run_logged_tool(&app, &session_path, "venturi_validate_case", json!({})).await?;
                if !val_check["validation"]["readyToRun"].as_bool().unwrap_or(true) {
                    let _ = run_logged_tool(
                        &app,
                        &session_path,
                        "venturi_auto_fit_domain",
                        json!({ "upstreamFactor": 2.5, "wakeFactor": 5.5, "lateralFactor": 2.2 }),
                    )
                    .await?;
                }

                // Keep y+ calibrated after geometry change
                if state_after_2.case.physics.turbulence != TurbulenceModel::Laminar {
                    let target_yp = goal
                        .target_y_plus
                        .unwrap_or(state_after_2.case.mesh.target_y_plus)
                        .clamp(
                            state_after_2.guardrails.target_y_plus_min,
                            state_after_2.guardrails.target_y_plus_max,
                        );
                    let _ = run_logged_tool(
                        &app,
                        &session_path,
                        "venturi_auto_size_yplus",
                        json!({ "targetYPlus": target_yp }),
                    )
                    .await;
                }

                if !is_cancelled(&ctrl) {
                    let _ = run_logged_tool(
                        &app,
                        &session_path,
                        "venturi_run_iteration",
                        json!({
                            "label": "Parametric Shape Optimization",
                            "changesSummary": geom_note,
                            "backend": backend_norm
                        }),
                    )
                    .await?;
                }
            }
        }
    }

    // Final verification & summary
    let final_state = load_or_init_session(&session_path);
    let best_idx = final_state.best_iteration_index.unwrap_or(1);
    let _ = run_logged_tool(
        &app,
        &session_path,
        "venturi_inspect_openfoam_results",
        json!({ "iterationIndex": best_idx }),
    )
    .await;

    let best_rec = final_state
        .iterations
        .iter()
        .find(|r| r.index == best_idx);

    let done_msg = if let Some(best) = best_rec {
        let delta = best.objective_score - baseline_score;
        format!(
            "Closed-loop complete ({} iterations). Best: #{} '{}' — Score {:.1}/100 ({:+.1} vs baseline), Cd={:?}, Cl={:?}, Δp={:?} Pa, {} refinement zone(s).",
            final_state.iterations.len(),
            best.index,
            best.label,
            best.objective_score,
            delta,
            best.results.cd.map(|v| (v * 10000.0).round() / 10000.0),
            best.results.cl.map(|v| (v * 1000.0).round() / 1000.0),
            best.results.delta_p_pa.map(|v| v.round()),
            final_state.case.mesh.refinement_boxes.len()
        )
    } else {
        "Closed-loop finished.".into()
    };

    emit_event(&app, "done", done_msg, None, None);
    Ok(())
}

// ============================================================================
// Provider Loops (Gemini, Anthropic, OpenAI / Ollama)
// ============================================================================

#[allow(clippy::too_many_arguments)]
async fn run_gemini_loop(
    app: &AppHandle,
    ctrl: &Arc<AutopilotController>,
    client: &reqwest::Client,
    session_path: &Path,
    llm_cfg: &LlmConfig,
    system_prompt: &str,
    initial_user_prompt: &str,
    max_turns: usize,
) -> Result<(), VenturiError> {
    let base = if llm_cfg.base_url.trim().is_empty() {
        "https://generativelanguage.googleapis.com"
    } else {
        llm_cfg.base_url.trim().trim_end_matches('/')
    };
    let model = if llm_cfg.model.trim().is_empty() {
        "gemini-2.5-flash"
    } else {
        llm_cfg.model.trim()
    };
    let url = format!(
        "{}/v1beta/models/{}:generateContent?key={}",
        base,
        model,
        llm_cfg.api_key.trim()
    );

    let declarations: Vec<Value> = tool_definitions()
        .into_iter()
        .map(|t| {
            json!({
                "name": t["name"],
                "description": t["description"],
                "parameters": sanitize_schema_for_gemini(&t["inputSchema"])
            })
        })
        .collect();

    let mut contents: Vec<Value> = vec![json!({
        "role": "user",
        "parts": [{ "text": initial_user_prompt }]
    })];

    for turn in 1..=max_turns {
        if is_cancelled(ctrl) {
            emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
            return Ok(());
        }

        let body = json!({
            "systemInstruction": { "parts": [{ "text": system_prompt }] },
            "contents": contents,
            "tools": [{ "functionDeclarations": declarations }],
            "generationConfig": { "temperature": 0.2 }
        });

        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| VenturiError::Execution(format!("Gemini request failed: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| VenturiError::Execution(format!("Gemini response read failed: {}", e)))?;
        if !status.is_success() {
            return Err(VenturiError::Execution(format!(
                "Gemini HTTP {}: {}",
                status.as_u16(),
                truncate_str(&text, 400)
            )));
        }

        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| VenturiError::Execution(format!("Invalid Gemini JSON: {}", e)))?;

        let parts = parsed["candidates"][0]["content"]["parts"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if parts.is_empty() {
            emit_event(
                app,
                "done",
                "Gemini returned an empty response; autopilot finished.".into(),
                None,
                None,
            );
            return Ok(());
        }

        // Preserve exact model turn parts (including thought signatures if present)
        contents.push(json!({
            "role": "model",
            "parts": parts
        }));

        let mut thought_chunks = Vec::new();
        let mut tool_calls = Vec::new();

        for (i, part) in parts.iter().enumerate() {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                if !t.trim().is_empty() {
                    thought_chunks.push(t.trim().to_string());
                }
            }
            if let Some(fc) = part.get("functionCall") {
                if let Some(name) = fc.get("name").and_then(|v| v.as_str()) {
                    let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                    tool_calls.push(ParsedToolCall {
                        id: format!("gemini-{}-{}", turn, i),
                        name: name.to_string(),
                        arguments: args,
                    });
                }
            }
        }

        let thought_text = thought_chunks.join("\n");
        if !thought_text.is_empty() {
            emit_event(app, "thinking", thought_text.clone(), None, None);
        }

        if tool_calls.is_empty() {
            emit_event(
                app,
                "done",
                if thought_text.is_empty() {
                    "Autopilot optimization complete.".into()
                } else {
                    thought_text
                },
                None,
                None,
            );
            return Ok(());
        }

        let mut response_parts = Vec::new();
        for call in tool_calls {
            let (result_val, _is_err, cancelled) = execute_with_review_gate(
                app,
                ctrl,
                session_path,
                llm_cfg,
                &call,
                &thought_text,
            )
            .await;
            if cancelled {
                emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
                return Ok(());
            }
            response_parts.push(json!({
                "functionResponse": {
                    "name": call.name,
                    "response": { "result": result_val }
                }
            }));
        }

        contents.push(json!({
            "role": "user",
            "parts": response_parts
        }));
    }

    emit_event(
        app,
        "done",
        format!("Reached maximum agent turn limit ({}).", max_turns),
        None,
        None,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_anthropic_loop(
    app: &AppHandle,
    ctrl: &Arc<AutopilotController>,
    client: &reqwest::Client,
    session_path: &Path,
    llm_cfg: &LlmConfig,
    system_prompt: &str,
    initial_user_prompt: &str,
    max_turns: usize,
) -> Result<(), VenturiError> {
    let base = if llm_cfg.base_url.trim().is_empty() {
        "https://api.anthropic.com"
    } else {
        llm_cfg.base_url.trim().trim_end_matches('/')
    };
    let url = if base.ends_with("/v1/messages") {
        base.to_string()
    } else if base.ends_with("/v1") {
        format!("{}/messages", base)
    } else {
        format!("{}/v1/messages", base)
    };
    let model = if llm_cfg.model.trim().is_empty() {
        "claude-sonnet-4-20250514"
    } else {
        llm_cfg.model.trim()
    };

    let tools: Vec<Value> = tool_definitions()
        .into_iter()
        .map(|t| {
            json!({
                "name": t["name"],
                "description": t["description"],
                "input_schema": t["inputSchema"]
            })
        })
        .collect();

    let mut messages: Vec<Value> = vec![json!({
        "role": "user",
        "content": initial_user_prompt
    })];

    for _turn in 1..=max_turns {
        if is_cancelled(ctrl) {
            emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
            return Ok(());
        }

        let body = json!({
            "model": model,
            "max_tokens": 2048,
            "system": system_prompt,
            "tools": tools,
            "messages": messages
        });

        let resp = client
            .post(&url)
            .header("x-api-key", llm_cfg.api_key.trim())
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| VenturiError::Execution(format!("Anthropic request failed: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| VenturiError::Execution(format!("Anthropic response read failed: {}", e)))?;
        if !status.is_success() {
            return Err(VenturiError::Execution(format!(
                "Anthropic HTTP {}: {}",
                status.as_u16(),
                truncate_str(&text, 400)
            )));
        }

        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| VenturiError::Execution(format!("Invalid Anthropic JSON: {}", e)))?;

        let content_blocks = parsed["content"].as_array().cloned().unwrap_or_default();
        if content_blocks.is_empty() {
            emit_event(
                app,
                "done",
                "Anthropic returned empty content; autopilot finished.".into(),
                None,
                None,
            );
            return Ok(());
        }

        messages.push(json!({
            "role": "assistant",
            "content": content_blocks
        }));

        let mut thought_chunks = Vec::new();
        let mut tool_calls = Vec::new();

        for block in &content_blocks {
            match block.get("type").and_then(|v| v.as_str()) {
                Some("text") => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        if !t.trim().is_empty() {
                            thought_chunks.push(t.trim().to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    let id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("tool_call")
                        .to_string();
                    if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                        let input = block.get("input").cloned().unwrap_or_else(|| json!({}));
                        tool_calls.push(ParsedToolCall {
                            id,
                            name: name.to_string(),
                            arguments: input,
                        });
                    }
                }
                _ => {}
            }
        }

        let thought_text = thought_chunks.join("\n");
        if !thought_text.is_empty() {
            emit_event(app, "thinking", thought_text.clone(), None, None);
        }

        if tool_calls.is_empty() {
            emit_event(
                app,
                "done",
                if thought_text.is_empty() {
                    "Autopilot optimization complete.".into()
                } else {
                    thought_text
                },
                None,
                None,
            );
            return Ok(());
        }

        let mut tool_results_blocks = Vec::new();
        for call in tool_calls {
            let (result_val, is_err, cancelled) = execute_with_review_gate(
                app,
                ctrl,
                session_path,
                llm_cfg,
                &call,
                &thought_text,
            )
            .await;
            if cancelled {
                emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
                return Ok(());
            }
            tool_results_blocks.push(json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": serde_json::to_string(&result_val).unwrap_or_else(|_| result_val.to_string()),
                "is_error": is_err
            }));
        }

        messages.push(json!({
            "role": "user",
            "content": tool_results_blocks
        }));
    }

    emit_event(
        app,
        "done",
        format!("Reached maximum agent turn limit ({}).", max_turns),
        None,
        None,
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn run_openai_compatible_loop(
    app: &AppHandle,
    ctrl: &Arc<AutopilotController>,
    client: &reqwest::Client,
    session_path: &Path,
    llm_cfg: &LlmConfig,
    system_prompt: &str,
    initial_user_prompt: &str,
    max_turns: usize,
) -> Result<(), VenturiError> {
    let default_base = match llm_cfg.provider {
        LlmProviderKind::Ollama => "http://localhost:11434/v1",
        _ => "https://api.openai.com/v1",
    };
    let base = if llm_cfg.base_url.trim().is_empty() {
        default_base
    } else {
        llm_cfg.base_url.trim().trim_end_matches('/')
    };
    let url = if base.ends_with("/chat/completions") {
        base.to_string()
    } else {
        format!("{}/chat/completions", base)
    };
    let model = if llm_cfg.model.trim().is_empty() {
        match llm_cfg.provider {
            LlmProviderKind::Ollama => "qwen2.5:14b",
            _ => "gpt-4o",
        }
    } else {
        llm_cfg.model.trim()
    };

    let tools: Vec<Value> = tool_definitions()
        .into_iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["inputSchema"]
                }
            })
        })
        .collect();

    let mut messages: Vec<Value> = vec![
        json!({ "role": "system", "content": system_prompt }),
        json!({ "role": "user", "content": initial_user_prompt }),
    ];

    for turn in 1..=max_turns {
        if is_cancelled(ctrl) {
            emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
            return Ok(());
        }

        let body = json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "tool_choice": "auto",
            "temperature": 0.2
        });

        let mut req = client.post(&url).header("content-type", "application/json");
        if !llm_cfg.api_key.trim().is_empty() {
            req = req.header("Authorization", format!("Bearer {}", llm_cfg.api_key.trim()));
        }

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| VenturiError::Execution(format!("OpenAI/Ollama request failed: {}", e)))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| VenturiError::Execution(format!("Response read failed: {}", e)))?;
        if !status.is_success() {
            return Err(VenturiError::Execution(format!(
                "LLM HTTP {}: {}",
                status.as_u16(),
                truncate_str(&text, 400)
            )));
        }

        let parsed: Value = serde_json::from_str(&text)
            .map_err(|e| VenturiError::Execution(format!("Invalid OpenAI/Ollama JSON: {}", e)))?;

        let message = &parsed["choices"][0]["message"];
        if message.is_null() {
            emit_event(
                app,
                "done",
                "Provider returned no choices; autopilot finished.".into(),
                None,
                None,
            );
            return Ok(());
        }

        let thought_text = message
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !thought_text.is_empty() {
            emit_event(app, "thinking", thought_text.clone(), None, None);
        }

        let raw_tool_calls = message
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        messages.push(message.clone());

        if raw_tool_calls.is_empty() {
            emit_event(
                app,
                "done",
                if thought_text.is_empty() {
                    "Autopilot optimization complete.".into()
                } else {
                    thought_text
                },
                None,
                None,
            );
            return Ok(());
        }

        let mut calls = Vec::new();
        for (i, tc) in raw_tool_calls.iter().enumerate() {
            let id = tc
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("call_{}_{}", turn, i));
            let func = &tc["function"];
            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                let raw_args = &func["arguments"];
                let arguments = if let Some(s) = raw_args.as_str() {
                    serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({}))
                } else if raw_args.is_object() {
                    raw_args.clone()
                } else {
                    json!({})
                };
                calls.push(ParsedToolCall {
                    id,
                    name: name.to_string(),
                    arguments,
                });
            }
        }

        for call in calls {
            let (result_val, _is_err, cancelled) = execute_with_review_gate(
                app,
                ctrl,
                session_path,
                llm_cfg,
                &call,
                &thought_text,
            )
            .await;
            if cancelled {
                emit_event(app, "done", "Autopilot stopped by user.".into(), None, None);
                return Ok(());
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "content": serde_json::to_string(&result_val).unwrap_or_else(|_| result_val.to_string())
            }));
        }
    }

    emit_event(
        app,
        "done",
        format!("Reached maximum agent turn limit ({}).", max_turns),
        None,
        None,
    );
    Ok(())
}

// ============================================================================
// Tool Execution, Review Gate, Prompting & Parametric Heuristics
// ============================================================================

async fn execute_with_review_gate(
    app: &AppHandle,
    ctrl: &Arc<AutopilotController>,
    session_path: &Path,
    llm_cfg: &LlmConfig,
    call: &ParsedToolCall,
    rationale_hint: &str,
) -> (Value, bool, bool) {
    let mut args = call.arguments.clone();

    // Inject configured backend on venturi_run_iteration if the LLM left it unset
    if call.name == "venturi_run_iteration" {
        if let Some(obj) = args.as_object_mut() {
            if !obj.contains_key("backend") && !llm_cfg.backend.trim().is_empty() {
                obj.insert("backend".into(), json!(llm_cfg.backend));
            }
        }
    }

    // In Review mode, require explicit human approval before any mutating or solver tool
    if llm_cfg.mode == AutopilotMode::Review && is_mutating_tool(&call.name) {
        let proposal = PendingProposal {
            id: format!("prop-{}", chrono::Utc::now().timestamp_millis()),
            tool_name: call.name.clone(),
            arguments: args.clone(),
            rationale: if rationale_hint.trim().is_empty() {
                format!("Autopilot proposes executing `{}`.", call.name)
            } else {
                truncate_str(rationale_hint.trim(), 320)
            },
        };

        let (tx, rx) = oneshot::channel::<bool>();
        {
            let mut slot = ctrl.pending_tx.lock().await;
            *slot = Some(tx);
        }

        emit_event(
            app,
            "proposal",
            format!("Awaiting engineer approval for `{}`", call.name),
            Some(call.name.clone()),
            Some(proposal),
        );

        let approved = await_proposal_decision(ctrl, rx).await;
        {
            let mut slot = ctrl.pending_tx.lock().await;
            *slot = None;
        }

        if is_cancelled(ctrl) {
            return (json!({ "cancelled": true }), true, true);
        }

        if !approved {
            emit_event(
                app,
                "tool_result",
                format!("Engineer rejected `{}` — asking agent for an alternative.", call.name),
                Some(call.name.clone()),
                None,
            );
            return (
                json!({
                    "rejectedByUser": true,
                    "tool": call.name,
                    "message": "The engineer rejected this proposed tool call. Adjust your plan, propose a safer/better alternative, or conclude."
                }),
                true,
                false,
            );
        }
    }

    emit_event(
        app,
        "tool_call",
        format!("Calling `{}` with {}", call.name, truncate_str(&args.to_string(), 160)),
        Some(call.name.clone()),
        None,
    );

    match execute_tool(session_path, &call.name, args, "in-app-autopilot").await {
        Ok(val) => {
            emit_event(
                app,
                "tool_result",
                format!("`{}` succeeded: {}", call.name, summarize_tool_result(&call.name, &val)),
                Some(call.name.clone()),
                None,
            );
            emit_event(
                app,
                "state_sync",
                format!("State synchronized after `{}`", call.name),
                Some(call.name.clone()),
                None,
            );
            (val, false, false)
        }
        Err(e) => {
            let err_str = e.to_string();
            emit_event(
                app,
                "tool_result",
                format!("`{}` blocked/failed: {}", call.name, err_str),
                Some(call.name.clone()),
                None,
            );
            emit_event(
                app,
                "state_sync",
                format!("Audit log updated after `{}` error", call.name),
                Some(call.name.clone()),
                None,
            );
            (json!({ "error": err_str, "tool": call.name }), true, false)
        }
    }
}

async fn await_proposal_decision(
    ctrl: &Arc<AutopilotController>,
    mut rx: oneshot::Receiver<bool>,
) -> bool {
    loop {
        if is_cancelled(ctrl) {
            return false;
        }
        tokio::select! {
            res = &mut rx => {
                return res.unwrap_or(false);
            }
            _ = tokio::time::sleep(Duration::from_millis(120)) => {
                if is_cancelled(ctrl) {
                    return false;
                }
            }
        }
    }
}

async fn run_logged_tool(
    app: &AppHandle,
    session_path: &Path,
    tool_name: &str,
    args: Value,
) -> Result<Value, VenturiError> {
    emit_event(
        app,
        "tool_call",
        format!("Executing `{}` {}", tool_name, truncate_str(&args.to_string(), 140)),
        Some(tool_name.to_string()),
        None,
    );

    match execute_tool(session_path, tool_name, args, "in-app-autopilot").await {
        Ok(val) => {
            emit_event(
                app,
                "tool_result",
                format!("`{}`: {}", tool_name, summarize_tool_result(tool_name, &val)),
                Some(tool_name.to_string()),
                None,
            );
            emit_event(
                app,
                "state_sync",
                format!("Updated workspace after `{}`", tool_name),
                Some(tool_name.to_string()),
                None,
            );
            Ok(val)
        }
        Err(e) => {
            emit_event(
                app,
                "tool_result",
                format!("`{}` failed: {}", tool_name, e),
                Some(tool_name.to_string()),
                None,
            );
            Err(e)
        }
    }
}

fn is_mutating_tool(tool_name: &str) -> bool {
    !matches!(
        tool_name,
        "venturi_get_state" | "venturi_validate_case" | "venturi_inspect_openfoam_results"
    )
}

fn is_cancelled(ctrl: &Arc<AutopilotController>) -> bool {
    ctrl.cancel_flag.load(Ordering::SeqCst)
}

fn emit_event(
    app: &AppHandle,
    kind: &str,
    message: String,
    tool_name: Option<String>,
    proposal: Option<PendingProposal>,
) {
    let _ = app.emit(
        AUTOPILOT_CHANNEL,
        AutopilotEvent {
            kind: kind.to_string(),
            message,
            tool_name,
            proposal,
        },
    );
}

fn build_cfd_system_prompt(
    guardrails: &crate::models::GuardrailsConfig,
    goal: &OptimizationGoal,
    backend: &str,
) -> String {
    format!(
        r#"You are Venturi Autopilot, a senior CFD aerodynamic & OpenFOAM numerical engineer operating inside Venturi CFD Studio via structured MCP tools.

Follow this disciplined 2-Phase Engineering Workflow:
PHASE 1 — Case & Geometry Setup:
1. Call `venturi_get_state` to inspect active geometry, domain, mesh, fluid properties, and prior iterations.
2. If the goal requires different operating conditions (fluid, temperature, velocity, turbulence model), call `venturi_set_fluid_conditions`.
3. If the case has no geometry or needs a new/modified design, call `venturi_generate_parametric_geometry` (NACA airfoil, Venturi tube, Ahmed diffuser, pin-fin array, or cylinder) with `autoFitDomain: true` when creating new geometry, then call `venturi_validate_case`.

PHASE 2 — Closed-Loop OpenFOAM Execution & Mesh/Shape Adaptation:
4. Run a baseline solver iteration with `venturi_run_iteration` (use `backend: "{backend}"` unless otherwise required).
5. ALWAYS call `venturi_inspect_openfoam_results` after every run to examine `checkMesh` (non-orthogonality, skewness, cells), per-patch `yPlus`, `Cd`/`Cl`/`Cm`, `deltaPPa`, and `hotspots`.
6. Act on the physics evidence:
   - If wall `y+` is outside [{yp_min:.1}, {yp_max:.1}], call `venturi_auto_size_yplus`.
   - If gradient `hotspots` exist, call `venturi_upsert_refinement_box` to refine those exact 3D zones (never exceeding `maxRefinementLevel` = {max_lvl}).
   - If `maxNonOrthogonality > 55°` or convergence is weak, call `venturi_update_numerics`.
   - Optimize parametric geometry dimensions toward the goal (`minimize_cd={min_cd}`, `maximize_cl_over_cd={max_ld}`).
7. Re-run `venturi_run_iteration` and verify objective score improvement. Respect hard guardrails: maxIterations={max_iter}, maxCells={max_cells}, maxRamMb={max_ram:.0}, maxRefinementLevel={max_lvl}, autoRollback={auto_rb}.
8. Once converged, guardrail-compliant, and clearly improved (or iteration budget reached), stop calling tools and present a concise engineering summary with baseline vs best KPIs."#,
        backend = if backend.trim().is_empty() { "demo" } else { backend.trim() },
        yp_min = guardrails.target_y_plus_min,
        yp_max = guardrails.target_y_plus_max,
        max_lvl = guardrails.max_refinement_level,
        min_cd = goal.minimize_cd,
        max_ld = goal.maximize_cl_over_cd,
        max_iter = guardrails.max_iterations,
        max_cells = guardrails.max_cells,
        max_ram = guardrails.max_ram_mb,
        auto_rb = guardrails.auto_rollback,
    )
}

fn build_initial_user_message(
    state: &crate::models::SessionState,
    goal: &OptimizationGoal,
    backend: &str,
) -> String {
    let tel = compute_telemetry(&state.case);
    format!(
        "Engineering Objective: {}\nTarget KPIs: target_y+={:?}, max_delta_p_pa={:?}, minimize_cd={}, maximize_cl_over_cd={}\nActive Case: '{}' (geometry={:?}, parametric={}, U={:.2} m/s, Re={:.2e}, est_cells={}, backend='{}', iterations_so_far={}/{})\nBegin Phase 1 and execute the closed optimization loop.",
        goal.prompt,
        goal.target_y_plus,
        goal.max_delta_p_pa,
        goal.minimize_cd,
        goal.maximize_cl_over_cd,
        state.case.name,
        state.case.geometry_kind,
        state.case.parametric.is_some(),
        state.case.physics.reference_velocity,
        tel.reynolds,
        tel.estimated_cells,
        if backend.trim().is_empty() { "demo" } else { backend.trim() },
        state.iterations.len(),
        state.guardrails.max_iterations,
    )
}

fn propose_parametric_refinement(
    spec: &ParametricGeometrySpec,
    goal: &OptimizationGoal,
) -> Option<(ParametricGeometrySpec, String, bool)> {
    match spec {
        ParametricGeometrySpec::NacaAirfoil {
            chord,
            camber_pct,
            camber_pos_pct,
            thickness_pct,
            aoa_deg,
            span,
        } => {
            let (new_aoa, new_camber, new_thick, note) = if goal.maximize_cl_over_cd {
                let aoa = 5.0_f64;
                let camber = (*camber_pct).max(2.4).min(4.0);
                let thick = thickness_pct.clamp(10.0, 12.5);
                (
                    aoa,
                    camber,
                    thick,
                    format!(
                        "Tuned airfoil for max Cl/Cd: camber={:.1}%, thickness={:.1}%, AoA={:.1}°",
                        camber, thick, aoa
                    ),
                )
            } else {
                let aoa = aoa_deg.clamp(-0.5, 1.0);
                let thick = (*thickness_pct - 2.0).clamp(9.0, 13.0);
                (
                    aoa,
                    *camber_pct,
                    thick,
                    format!(
                        "Streamlined airfoil for low Cd: thickness={:.1}%, AoA={:.1}°",
                        thick, aoa
                    ),
                )
            };
            Some((
                ParametricGeometrySpec::NacaAirfoil {
                    chord: *chord,
                    camber_pct: new_camber,
                    camber_pos_pct: *camber_pos_pct,
                    thickness_pct: new_thick,
                    aoa_deg: new_aoa,
                    span: *span,
                },
                note,
                false,
            ))
        }
        ParametricGeometrySpec::AhmedDiffuser {
            length,
            width,
            height,
            rear_slant_deg,
            ground_clearance,
        } => {
            let new_slant = if (*rear_slant_deg - 14.0).abs() > 1.5 {
                14.0
            } else {
                12.5
            };
            Some((
                ParametricGeometrySpec::AhmedDiffuser {
                    length: *length,
                    width: *width,
                    height: *height,
                    rear_slant_deg: new_slant,
                    ground_clearance: *ground_clearance,
                },
                format!(
                    "Adjusted Ahmed rear slant {:.1}° -> {:.1}° to suppress C-pillar vortex drag",
                    rear_slant_deg, new_slant
                ),
                false,
            ))
        }
        ParametricGeometrySpec::VenturiTube {
            inlet_radius,
            throat_radius,
            outlet_radius,
            convergent_len,
            throat_len,
            divergent_len,
        } => {
            let new_throat = (*throat_radius * 1.10).min(*inlet_radius * 0.78);
            let new_div = (*divergent_len * 1.15).min(*divergent_len + 0.6);
            Some((
                ParametricGeometrySpec::VenturiTube {
                    inlet_radius: *inlet_radius,
                    throat_radius: new_throat,
                    outlet_radius: *outlet_radius,
                    convergent_len: *convergent_len,
                    throat_len: *throat_len,
                    divergent_len: new_div,
                },
                format!(
                    "Opened Venturi throat {:.3}->{:.3} m & lengthened diffuser to {:.2} m for pressure recovery",
                    throat_radius, new_throat, new_div
                ),
                true,
            ))
        }
        ParametricGeometrySpec::PinFinArray {
            base_length,
            base_width,
            fin_count_x,
            fin_count_z,
            fin_radius,
            fin_height,
        } => {
            let new_r = (*fin_radius * 0.88).max(0.004);
            Some((
                ParametricGeometrySpec::PinFinArray {
                    base_length: *base_length,
                    base_width: *base_width,
                    fin_count_x: *fin_count_x,
                    fin_count_z: *fin_count_z,
                    fin_radius: new_r,
                    fin_height: *fin_height,
                },
                format!(
                    "Refined pin-fin radius {:.4}->{:.4} m to cut wake blockage & Δp",
                    fin_radius, new_r
                ),
                false,
            ))
        }
        ParametricGeometrySpec::Cylinder { radius, span } => {
            let new_r = (*radius * 0.90).max(0.05);
            Some((
                ParametricGeometrySpec::Cylinder {
                    radius: new_r,
                    span: *span,
                },
                format!("Reduced cylinder radius {:.3}->{:.3} m to cut pressure drag", radius, new_r),
                false,
            ))
        }
    }
}

fn sanitize_schema_for_gemini(schema: &Value) -> Value {
    match schema {
        Value::Object(map) => {
            let mut clean = serde_json::Map::new();
            for (k, v) in map {
                if matches!(k.as_str(), "additionalProperties" | "default" | "$schema") {
                    continue;
                }
                clean.insert(k.clone(), sanitize_schema_for_gemini(v));
            }
            Value::Object(clean)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sanitize_schema_for_gemini).collect()),
        other => other.clone(),
    }
}

fn summarize_tool_result(tool_name: &str, val: &Value) -> String {
    match tool_name {
        "venturi_run_iteration" => format!(
            "Iter #{} score={:.1}/100 (best={}, rolledBack={}, Cd={}, Δp={})",
            val["iterationIndex"].as_u64().unwrap_or(0),
            val["objectiveScore"].as_f64().unwrap_or(0.0),
            val["isBest"].as_bool().unwrap_or(false),
            val["rolledBack"].as_bool().unwrap_or(false),
            val["results"]["cd"]
                .as_f64()
                .map(|x| format!("{:.4}", x))
                .unwrap_or_else(|| "n/a".into()),
            val["results"]["deltaPPa"]
                .as_f64()
                .map(|x| format!("{:.1} Pa", x))
                .unwrap_or_else(|| "n/a".into()),
        ),
        "venturi_inspect_openfoam_results" => format!(
            "Iter #{}: {} hotspots, {} recommendations, allGuardrailsOk={}",
            val["iteration"]["index"].as_u64().unwrap_or(0),
            val["results"]["hotspots"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0),
            val["recommendations"]
                .as_array()
                .map(|a| a.len())
                .unwrap_or(0),
            val["guardrailCompliance"]["allPassed"]
                .as_bool()
                .unwrap_or(false)
        ),
        "venturi_auto_size_yplus" => format!(
            "dy1={:.4} mm, {} layers (target y+={:.1})",
            val["yPlusResult"]["firstCellHeight"].as_f64().unwrap_or(0.0) * 1000.0,
            val["yPlusResult"]["recommendedLayers"].as_u64().unwrap_or(0),
            val["mesh"]["targetYPlus"].as_f64().unwrap_or(1.0)
        ),
        "venturi_upsert_refinement_box" => format!(
            "box '{}' L{} (total {} zones)",
            val["box"]["name"].as_str().unwrap_or("box"),
            val["box"]["level"].as_u64().unwrap_or(0),
            val["totalBoxes"].as_u64().unwrap_or(0)
        ),
        "venturi_generate_parametric_geometry" => format!(
            "{} ({} tris, L_ref={:.3} m)",
            val["filename"].as_str().unwrap_or("parametric.stl"),
            val["stlSummary"]["triangleCount"].as_u64().unwrap_or(0),
            val["referenceLength"].as_f64().unwrap_or(1.0)
        ),
        "venturi_validate_case" => format!(
            "score={}/100, ready={}",
            val["validation"]["score"].as_u64().unwrap_or(0),
            val["validation"]["readyToRun"].as_bool().unwrap_or(false)
        ),
        _ => truncate_str(&val.to_string(), 140),
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_schema_sanitizer_strips_unsupported_keys() {
        let raw = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "x": { "type": "number", "default": 1.0 }
            }
        });
        let cleaned = sanitize_schema_for_gemini(&raw);
        assert!(cleaned.get("additionalProperties").is_none());
        assert!(cleaned["properties"]["x"].get("default").is_none());
        assert_eq!(cleaned["properties"]["x"]["type"], "number");
    }

    #[test]
    fn parametric_refinement_covers_all_five_families() {
        let goal = OptimizationGoal::default();
        let specs = vec![
            ParametricGeometrySpec::NacaAirfoil {
                chord: 1.0,
                camber_pct: 2.0,
                camber_pos_pct: 40.0,
                thickness_pct: 12.0,
                aoa_deg: 6.0,
                span: 0.76,
            },
            ParametricGeometrySpec::AhmedDiffuser {
                length: 1.044,
                width: 0.389,
                height: 0.288,
                rear_slant_deg: 25.0,
                ground_clearance: 0.05,
            },
            ParametricGeometrySpec::VenturiTube {
                inlet_radius: 0.52,
                throat_radius: 0.26,
                outlet_radius: 0.52,
                convergent_len: 1.2,
                throat_len: 0.8,
                divergent_len: 2.8,
            },
            ParametricGeometrySpec::PinFinArray {
                base_length: 0.2,
                base_width: 0.14,
                fin_count_x: 4,
                fin_count_z: 3,
                fin_radius: 0.008,
                fin_height: 0.045,
            },
            ParametricGeometrySpec::Cylinder {
                radius: 0.25,
                span: 0.6,
            },
        ];
        for s in &specs {
            let res = propose_parametric_refinement(s, &goal);
            assert!(res.is_some());
        }
    }
}
