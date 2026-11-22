use crate::error::VenturiError;
use crate::models::{
    BoundaryKind, BoundaryPatch, CfdCase, DomainBox, DomainFace, FlowRegime, FluidConfig,
    GuardrailsConfig, IterationRecord, OpenFoamFlavor, OpenFoamResultSummary, OptimizationGoal,
    ParametricGeometrySpec, PresetGeometryKind, RefinementBox, SchemeAccuracy, TurbulenceIC,
    TurbulenceIcInput, TurbulenceModel, YPlusInput,
};
use crate::openfoam::generator::export_case_to_disk;
use crate::openfoam::results::{
    compute_objective_score, evaluate_surrogate_results, ingest_openfoam_results,
};
use crate::openfoam::runner::detect_environment;
use crate::openfoam::validator::validate_case;
use crate::parametric::{generate_parametric_stl, reference_length, suggested_filename};
use crate::physics::{
    compute_air_sutherland, compute_telemetry, compute_turbulence_ic, compute_y_plus,
};
use crate::session::{
    append_tool_log, default_preset_case, enforce_mesh_guardrails, load_or_init_session,
    rollback_to_iteration, save_session_atomic,
};
use crate::stl::parse_stl_file;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "venturi-mcp";
const SERVER_VERSION: &str = "0.2.0";

/// Returns complete MCP tool schemas (`name`, `description`, `inputSchema`) for all Venturi CFD tools.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        json!({
            "name": "venturi_get_state",
            "description": "Read the full active Venturi CFD session state, including geometry, domain, mesh, fluid physics, boundaries, numerics, telemetry, validation report, guardrails, optimization goal, and best/latest solver iterations.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "venturi_load_preset",
            "description": "Load a built-in CFD benchmark preset or start from an empty wind-tunnel workspace without geometry.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "presetId": {
                        "type": "string",
                        "enum": ["naca0012", "ahmed_body", "venturi_nozzle", "cylinder_vortex", "lid_cavity", "empty"],
                        "description": "Preset identifier. Use 'empty' to build a case from scratch."
                    },
                    "clearHistory": {
                        "type": "boolean",
                        "description": "If true (default), resets prior solver iterations and best-iteration tracking."
                    }
                },
                "required": ["presetId"]
            }
        }),
        json!({
            "name": "venturi_set_fluid_conditions",
            "description": "Configure fluid properties (air Sutherland law from temperatureC/pressureKpa, standard liquids, or custom viscosity/density), freestream velocity, reference length, turbulence intensity, turbulence model, and flow regime. Automatically recalculates turbulence initial conditions (k, epsilon, omega, nut, nuTilda) and updates VelocityInlet patches.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "fluidName": {
                        "type": "string",
                        "description": "Fluid preset or custom label (e.g. 'air', 'water', 'seawater', 'oil', 'co2', 'methane')."
                    },
                    "temperatureC": {
                        "type": "number",
                        "description": "Fluid temperature in degrees Celsius (e.g. 20.0)."
                    },
                    "pressureKpa": {
                        "type": "number",
                        "description": "Operating static pressure in kPa (e.g. 101.325)."
                    },
                    "density": {
                        "type": "number",
                        "description": "Optional explicit fluid density in kg/m^3."
                    },
                    "dynamicViscosity": {
                        "type": "number",
                        "description": "Optional explicit dynamic viscosity in Pa*s."
                    },
                    "kinematicViscosity": {
                        "type": "number",
                        "description": "Optional explicit kinematic viscosity in m^2/s."
                    },
                    "velocity": {
                        "type": "number",
                        "description": "Reference freestream / inlet velocity magnitude in m/s."
                    },
                    "referenceLength": {
                        "type": "number",
                        "description": "Characteristic reference length L_ref in meters."
                    },
                    "turbulenceIntensityPct": {
                        "type": "number",
                        "description": "Freestream turbulence intensity in percent (0.05 to 50.0)."
                    },
                    "turbulenceModel": {
                        "type": "string",
                        "enum": ["laminar", "kOmegaSST", "kEpsilon", "spalartAllmaras", "waleLes"],
                        "description": "Turbulence modeling approach."
                    },
                    "regime": {
                        "type": "string",
                        "enum": ["steadyIncompressible", "transientIncompressible", "steadyCompressible"],
                        "description": "Flow solver regime."
                    }
                }
            }
        }),
        json!({
            "name": "venturi_set_guardrails",
            "description": "Configure hard engineering guardrails enforced on mesh sizing, refinement depth, RAM usage, y+ window, mesh quality, iteration budget, and automatic rollback on regression.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "maxIterations": { "type": "integer", "description": "Maximum solver iterations allowed in this session." },
                    "maxCells": { "type": "integer", "description": "Maximum allowed estimated mesh cell count." },
                    "maxRamMb": { "type": "number", "description": "Maximum estimated solver RAM in MB." },
                    "maxRefinementLevel": { "type": "integer", "description": "Maximum snappyHexMesh surface or region refinement level." },
                    "targetYPlusMin": { "type": "number", "description": "Lower bound of acceptable wall y+ window." },
                    "targetYPlusMax": { "type": "number", "description": "Upper bound of acceptable wall y+ window." },
                    "maxSkewness": { "type": "number", "description": "Maximum acceptable checkMesh skewness." },
                    "maxNonOrtho": { "type": "number", "description": "Maximum acceptable checkMesh non-orthogonality (degrees)." },
                    "autoRollback": { "type": "boolean", "description": "Automatically revert case state to best iteration on divergence, mesh failure, or severe score regression." }
                }
            }
        }),
        json!({
            "name": "venturi_generate_parametric_geometry",
            "description": "Generate a watertight 3D Binary STL from a parametric engineering specification (NACA 4-digit airfoil, Venturi tube, Ahmed diffuser, pin-fin heat sink array, or cylinder), attach it to the case, enable snappyHexMesh, ensure an obstacle wall patch exists, and optionally auto-fit the computational domain and reference length.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "spec": {
                        "type": "object",
                        "description": "Tagged ParametricGeometrySpec object. Choose 'family' and set its geometry fields: nacaAirfoil(chord, camberPct, camberPosPct, thicknessPct, aoaDeg, span); venturiTube(inletRadius, throatRadius, outletRadius, convergentLen, throatLen, divergentLen); ahmedDiffuser(length, width, height, rearSlantDeg, groundClearance); pinFinArray(baseLength, baseWidth, finCountX, finCountZ, finRadius, finHeight); cylinder(radius, span).",
                        "properties": {
                            "family": {
                                "type": "string",
                                "enum": ["nacaAirfoil", "venturiTube", "ahmedDiffuser", "pinFinArray", "cylinder"]
                            },
                            "chord": { "type": "number" },
                            "camberPct": { "type": "number" },
                            "camberPosPct": { "type": "number" },
                            "thicknessPct": { "type": "number" },
                            "aoaDeg": { "type": "number" },
                            "span": { "type": "number" },
                            "inletRadius": { "type": "number" },
                            "throatRadius": { "type": "number" },
                            "outletRadius": { "type": "number" },
                            "convergentLen": { "type": "number" },
                            "throatLen": { "type": "number" },
                            "divergentLen": { "type": "number" },
                            "length": { "type": "number" },
                            "width": { "type": "number" },
                            "height": { "type": "number" },
                            "rearSlantDeg": { "type": "number" },
                            "groundClearance": { "type": "number" },
                            "baseLength": { "type": "number" },
                            "baseWidth": { "type": "number" },
                            "finCountX": { "type": "integer" },
                            "finCountZ": { "type": "integer" },
                            "finRadius": { "type": "number" },
                            "finHeight": { "type": "number" },
                            "radius": { "type": "number" }
                        },
                        "required": ["family"]
                    },
                    "autoFitDomain": {
                        "type": "boolean",
                        "description": "Automatically resize the computational domain to wrap the generated geometry with upstream/wake clearance (default true)."
                    }
                },
                "required": ["spec"]
            }
        }),
        json!({
            "name": "venturi_import_stl",
            "description": "Import a custom ASCII or Binary STL file from disk, compute its bounding box, surface area, and volume, enable snappyHexMesh, and optionally auto-fit the domain.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the .stl file on disk." },
                    "autoFitDomain": { "type": "boolean", "description": "Auto-fit blockMesh domain around the STL bounds (default true)." }
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "venturi_auto_fit_domain",
            "description": "Resize the blockMesh computational domain around the active STL or parametric geometry using upstream, downstream wake, and lateral clearance multipliers.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "upstreamFactor": { "type": "number", "description": "Upstream clearance in multiples of body length (default 2.5)." },
                    "wakeFactor": { "type": "number", "description": "Downstream wake length in multiples of body length (default 5.5)." },
                    "lateralFactor": { "type": "number", "description": "Vertical/lateral clearance in multiples of characteristic body size (default 2.2)." }
                }
            }
        }),
        json!({
            "name": "venturi_update_mesh_config",
            "description": "Update blockMesh resolution and snappyHexMesh surface/layer controls. Strictly enforces mesh cell count, RAM, and refinement-level guardrails before committing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "cellsX": { "type": "integer", "description": "Background hex cells along X." },
                    "cellsY": { "type": "integer", "description": "Background hex cells along Y." },
                    "cellsZ": { "type": "integer", "description": "Background hex cells along Z." },
                    "gradingY": { "type": "number", "description": "SimpleGrading factor along Y." },
                    "refinementMin": { "type": "integer", "description": "Minimum snappyHexMesh surface refinement level." },
                    "refinementMax": { "type": "integer", "description": "Maximum snappyHexMesh surface refinement level." },
                    "inflationLayers": { "type": "integer", "description": "Number of boundary-layer prism layers." },
                    "firstLayerThickness": { "type": "number", "description": "Absolute first boundary-layer cell height in meters." },
                    "expansionRatio": { "type": "number", "description": "Prism layer geometric expansion ratio (> 1.0)." },
                    "targetYPlus": { "type": "number", "description": "Target dimensionless wall distance y+." },
                    "snappyEnabled": { "type": "boolean", "description": "Enable or disable snappyHexMesh." }
                }
            }
        }),
        json!({
            "name": "venturi_upsert_refinement_box",
            "description": "Create or update a 3D volumetric searchableBox refinement region in snappyHexMeshDict (e.g., wake, separation zone, throat, or hotspot). Enforces maxRefinementLevel and cell/RAM guardrails.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique OpenFOAM-safe identifier for the refinement region (e.g. 'near_wake_box')." },
                    "min": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Minimum [x, y, z] corner coordinates in meters."
                    },
                    "max": {
                        "type": "array",
                        "items": { "type": "number" },
                        "description": "Maximum [x, y, z] corner coordinates in meters."
                    },
                    "level": { "type": "integer", "description": "Volumetric octree refinement level (must be <= guardrails.maxRefinementLevel)." },
                    "reason": { "type": "string", "description": "Engineering justification for this refinement zone." }
                },
                "required": ["name", "min", "max", "level"]
            }
        }),
        json!({
            "name": "venturi_remove_refinement_box",
            "description": "Remove an existing volumetric refinement box by name from snappyHexMeshDict.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name of the refinement box to remove." }
                },
                "required": ["name"]
            }
        }),
        json!({
            "name": "venturi_auto_size_yplus",
            "description": "Compute exact boundary-layer first cell height (dy1) and recommended prism inflation layers from fluid properties, velocity, and reference length for a target y+, then apply them to the case mesh.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "targetYPlus": { "type": "number", "description": "Desired dimensionless wall y+ (e.g. 1.0 for wall-resolved k-omega SST, or 35.0 for wall functions)." },
                    "expansionRatio": { "type": "number", "description": "Optional prism layer growth ratio (default uses current mesh expansionRatio, typically 1.2)." }
                },
                "required": ["targetYPlus"]
            }
        }),
        json!({
            "name": "venturi_update_numerics",
            "description": "Tune OpenFOAM discretization schemes, under-relaxation factors, time/iteration limits, convergence tolerance, non-orthogonal correctors, and MPI core count.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "accuracy": {
                        "type": "string",
                        "enum": ["robustUpwind", "balancedSecondOrder", "highResolution"],
                        "description": "fvSchemes convection/gradient discretization accuracy."
                    },
                    "relaxP": { "type": "number", "description": "Pressure field relaxation factor (0.05..1.0)." },
                    "relaxU": { "type": "number", "description": "Velocity equation relaxation factor (0.05..1.0)." },
                    "relaxTurb": { "type": "number", "description": "Turbulence equation relaxation factor (0.05..1.0)." },
                    "endTime": { "type": "number", "description": "End time or steady iteration cap." },
                    "deltaT": { "type": "number", "description": "Time step size (or 1 for steady SIMPLE)." },
                    "writeInterval": { "type": "number", "description": "Field write interval." },
                    "convergenceTol": { "type": "number", "description": "SIMPLE residualControl target tolerance (e.g. 1e-4)." },
                    "maxCo": { "type": "number", "description": "Maximum Courant number for transient runs." },
                    "nNonOrthoCorrectors": { "type": "integer", "description": "Number of non-orthogonal pressure correctors (0..10)." },
                    "parallelCores": { "type": "integer", "description": "Number of MPI subdomains for decomposePar (1..128)." }
                }
            }
        }),
        json!({
            "name": "venturi_validate_case",
            "description": "Run the automated CFD pre-flight audit checking domain bounds, STL clearance, boundary condition well-posedness, Mach compressibility, turbulence IC, y+ wall treatment, Courant stability, and mesh guardrails.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "venturi_run_iteration",
            "description": "Export OpenFOAM dictionaries and parametric STL, execute a solver iteration (local OpenFOAM, Docker, or physics-informed surrogate), ingest checkMesh/yPlus/forces/pressure-drop/hotspots, compute objective score, and automatically roll back to best iteration if guardrails.autoRollback triggers on divergence, mesh failure, or >5 pt regression.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "label": { "type": "string", "description": "Short label for this solver iteration (e.g. 'Baseline' or 'Wake Refinement L3')." },
                    "changesSummary": { "type": "string", "description": "Concise description of geometry/mesh/numerics changes tested in this iteration." },
                    "backend": {
                        "type": "string",
                        "enum": ["auto", "local", "docker", "demo"],
                        "description": "Execution backend: 'auto' uses local OpenFOAM if available else surrogate; 'local'/'docker' run OpenFOAM Allrun; 'demo' forces fast physics surrogate."
                    },
                    "caseDir": { "type": "string", "description": "Optional directory path to export the OpenFOAM case into." }
                }
            }
        }),
        json!({
            "name": "venturi_inspect_openfoam_results",
            "description": "Inspect structured OpenFOAM post-processing outputs (checkMesh quality, per-patch y+, Cd/Cl/Cm, pressure drop, gradient hotspots), delta vs best iteration, guardrail compliance checklist, and concrete recommended next tool calls.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "iterationIndex": {
                        "type": "integer",
                        "description": "1-based iteration index to inspect (defaults to the most recent iteration)."
                    }
                }
            }
        }),
        json!({
            "name": "venturi_rollback_iteration",
            "description": "Restore the active CFD case configuration to a prior iteration snapshot (or to the highest-scoring best iteration when iterationIndex is omitted).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "iterationIndex": {
                        "type": "integer",
                        "description": "1-based iteration index to restore. Omit to restore the current best iteration."
                    }
                }
            }
        }),
    ]
}

/// Executes a named Venturi MCP tool against the shared atomic session file.
pub async fn execute_tool(
    session_path: &Path,
    tool_name: &str,
    args: Value,
    source: &str,
) -> Result<Value, VenturiError> {
    let mut state = load_or_init_session(session_path);

    let outcome: Result<(Value, String, bool), VenturiError> = match tool_name {
        "venturi_get_state" => handle_get_state(&state),
        "venturi_load_preset" => handle_load_preset(&mut state, &args),
        "venturi_set_fluid_conditions" => handle_set_fluid_conditions(&mut state, &args),
        "venturi_set_guardrails" => handle_set_guardrails(&mut state, &args),
        "venturi_generate_parametric_geometry" => {
            handle_generate_parametric_geometry(&mut state, &args)
        }
        "venturi_import_stl" => handle_import_stl(&mut state, &args),
        "venturi_auto_fit_domain" => handle_auto_fit_domain(&mut state, &args),
        "venturi_update_mesh_config" => handle_update_mesh_config(&mut state, &args),
        "venturi_upsert_refinement_box" => handle_upsert_refinement_box(&mut state, &args),
        "venturi_remove_refinement_box" => handle_remove_refinement_box(&mut state, &args),
        "venturi_auto_size_yplus" => handle_auto_size_yplus(&mut state, &args),
        "venturi_update_numerics" => handle_update_numerics(&mut state, &args),
        "venturi_validate_case" => handle_validate_case(&state),
        "venturi_run_iteration" => handle_run_iteration(&mut state, &args).await,
        "venturi_inspect_openfoam_results" => handle_inspect_openfoam_results(&state, &args),
        "venturi_rollback_iteration" => handle_rollback_iteration(&mut state, &args),
        unknown => Err(VenturiError::InvalidCase(format!(
            "Unknown MCP tool '{}'. Call tools/list to discover valid Venturi tools.",
            unknown
        ))),
    };

    match outcome {
        Ok((result, summary, should_save)) => {
            if should_save || source != "internal-read" {
                append_tool_log(&mut state, source, tool_name, &args, true, summary);
                if should_save || session_path.exists() {
                    save_session_atomic(session_path, &mut state, source)?;
                }
            }
            Ok(result)
        }
        Err(err) => {
            append_tool_log(&mut state, source, tool_name, &args, false, err.to_string());
            if session_path.exists() {
                let _ = save_session_atomic(session_path, &mut state, source);
            }
            Err(err)
        }
    }
}

/// Handles a single MCP JSON-RPC 2.0 request/notification.
/// Returns `None` for notifications that do not expect a JSON-RPC response.
pub async fn handle_jsonrpc_request(session_path: &Path, req: Value) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned();

    // JSON-RPC notifications (no id, or MCP notifications/* methods) receive no response.
    if method == "notifications/initialized"
        || method.starts_with("notifications/")
        || (id.is_none() && method.is_empty())
    {
        return None;
    }

    let id = match id {
        Some(v) => v,
        None => return None,
    };

    match method {
        "initialize" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {
                    "tools": {}
                },
                "serverInfo": {
                    "name": SERVER_NAME,
                    "version": SERVER_VERSION
                }
            }
        })),
        "ping" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {}
        })),
        "tools/list" => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "tools": tool_definitions()
            }
        })),
        "tools/call" => {
            let params = req.get("params").cloned().unwrap_or_else(|| json!({}));
            let tool_name = match params.get("name").and_then(|n| n.as_str()) {
                Some(n) if !n.trim().is_empty() => n,
                _ => {
                    return Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32602,
                            "message": "Invalid params: missing required string 'name'."
                        }
                    }));
                }
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));

            match execute_tool(session_path, tool_name, arguments, "mcp-external").await {
                Ok(val) => {
                    let pretty = serde_json::to_string_pretty(&val)
                        .unwrap_or_else(|_| val.to_string());
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": pretty
                                }
                            ],
                            "isError": false
                        }
                    }))
                }
                Err(e) => {
                    let err_payload = json!({
                        "error": e.to_string(),
                        "tool": tool_name
                    });
                    let pretty = serde_json::to_string_pretty(&err_payload)
                        .unwrap_or_else(|_| e.to_string());
                    Some(json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [
                                {
                                    "type": "text",
                                    "text": pretty
                                }
                            ],
                            "isError": true
                        }
                    }))
                }
            }
        }
        other => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": -32601,
                "message": format!("Method not found: {}", other)
            }
        })),
    }
}

// ============================================================================
// Tool Handlers
// ============================================================================

fn handle_get_state(
    state: &crate::models::SessionState,
) -> Result<(Value, String, bool), VenturiError> {
    let telemetry = compute_telemetry(&state.case);
    let validation = validate_case(&state.case);
    let best_iter = state
        .best_iteration_index
        .and_then(|idx| state.iterations.iter().find(|r| r.index == idx))
        .map(compact_iteration_summary);
    let latest_iter = state.iterations.last().map(compact_iteration_summary);

    let out = json!({
        "version": state.version,
        "updatedAt": state.updated_at,
        "lastWriter": state.last_writer,
        "caseDir": state.case_dir,
        "case": state.case,
        "telemetry": telemetry,
        "validation": validation,
        "guardrails": state.guardrails,
        "goal": state.goal,
        "iterationCount": state.iterations.len(),
        "bestIterationIndex": state.best_iteration_index,
        "bestIteration": best_iter,
        "latestIteration": latest_iter,
    });

    let summary = format!(
        "Read state v{} ({}, {} cells, {} iters)",
        state.version,
        state.case.name,
        telemetry.estimated_cells,
        state.iterations.len()
    );
    Ok((out, summary, false))
}

fn handle_load_preset(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let preset_id = args
        .get("presetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VenturiError::InvalidCase("Missing required parameter 'presetId'.".into()))?;

    let valid = [
        "naca0012",
        "ahmed_body",
        "venturi_nozzle",
        "cylinder_vortex",
        "lid_cavity",
        "empty",
    ];
    if !valid.contains(&preset_id) {
        return Err(VenturiError::InvalidCase(format!(
            "Unknown presetId '{}'. Valid presets: {}",
            preset_id,
            valid.join(", ")
        )));
    }

    let clear_history = args
        .get("clearHistory")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    state.case = default_preset_case(preset_id);
    if clear_history {
        state.iterations.clear();
        state.best_iteration_index = None;
    }

    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "presetId": preset_id,
        "caseName": state.case.name,
        "geometryKind": state.case.geometry_kind,
        "parametric": state.case.parametric,
        "domain": state.case.domain,
        "telemetry": telemetry,
    });
    let summary = format!("Loaded preset '{}' ({})", preset_id, state.case.name);
    Ok((out, summary, true))
}

fn handle_set_fluid_conditions(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let mut fluid = state.case.physics.fluid.clone();
    let old_u_ref = state.case.physics.reference_velocity;

    let mut fluid_name_provided = None;
    let mut preset_liquid_selected = false;

    if let Some(name) = args.get("fluidName").and_then(|v| v.as_str()) {
        let lower = name.trim().to_lowercase();
        if !lower.is_empty() {
            fluid_name_provided = Some(name.trim().to_string());
            if lower.contains("seawater") {
                fluid = FluidConfig {
                    name: "Seawater (15 °C, 35 g/kg)".into(),
                    density: 1026.0,
                    dynamic_viscosity: 1.22e-3,
                    kinematic_viscosity: 1.22e-3 / 1026.0,
                    temperature_k: 288.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.0,
                    gas_constant: 461.5,
                    is_gas: false,
                };
                preset_liquid_selected = true;
            } else if lower.contains("water") {
                fluid = FluidConfig {
                    name: "Water (Liquid, 20 °C)".into(),
                    density: 998.2,
                    dynamic_viscosity: 1.002e-3,
                    kinematic_viscosity: 1.002e-3 / 998.2,
                    temperature_k: 293.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.0,
                    gas_constant: 461.5,
                    is_gas: false,
                };
                preset_liquid_selected = true;
            } else if lower.contains("oil") {
                fluid = FluidConfig {
                    name: "Engine Oil SAE 30 (40 °C)".into(),
                    density: 876.0,
                    dynamic_viscosity: 0.105,
                    kinematic_viscosity: 0.105 / 876.0,
                    temperature_k: 313.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.0,
                    gas_constant: 100.0,
                    is_gas: false,
                };
                preset_liquid_selected = true;
            } else if lower.contains("co2") || lower.contains("carbon dioxide") {
                fluid = FluidConfig {
                    name: "Carbon Dioxide CO2 (25 °C)".into(),
                    density: 1.797,
                    dynamic_viscosity: 1.49e-5,
                    kinematic_viscosity: 1.49e-5 / 1.797,
                    temperature_k: 298.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.289,
                    gas_constant: 188.92,
                    is_gas: true,
                };
            } else if lower.contains("methane") || lower.contains("ch4") {
                fluid = FluidConfig {
                    name: "Methane CH4 (20 °C)".into(),
                    density: 0.668,
                    dynamic_viscosity: 1.10e-5,
                    kinematic_viscosity: 1.10e-5 / 0.668,
                    temperature_k: 293.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.31,
                    gas_constant: 518.28,
                    is_gas: true,
                };
            } else if lower.contains("air") {
                fluid.is_gas = true;
                fluid.gamma = 1.4;
                fluid.gas_constant = 287.058;
            } else {
                fluid.name = name.trim().to_string();
            }
        }
    }

    let temp_changed = args.get("temperatureC").and_then(|v| v.as_f64()).is_some();
    let press_changed = args.get("pressureKpa").and_then(|v| v.as_f64()).is_some();

    if let Some(temp_c) = args.get("temperatureC").and_then(|v| v.as_f64()) {
        fluid.temperature_k = temp_c + 273.15;
    }
    if let Some(p_kpa) = args.get("pressureKpa").and_then(|v| v.as_f64()) {
        fluid.pressure_pa = p_kpa * 1000.0;
    }

    let has_custom_props = args.get("density").is_some()
        || args.get("dynamicViscosity").is_some()
        || args.get("kinematicViscosity").is_some();

    let is_air = fluid.is_gas
        && !preset_liquid_selected
        && (fluid_name_provided
            .as_deref()
            .map(|n| n.to_lowercase().contains("air"))
            .unwrap_or_else(|| fluid.name.to_lowercase().contains("air")));

    if is_air && !has_custom_props && (temp_changed || press_changed || fluid_name_provided.is_some()) {
        fluid = compute_air_sutherland(fluid.temperature_k, fluid.pressure_pa)?;
    }

    // Apply explicit custom overrides if provided
    if let Some(rho) = args.get("density").and_then(|v| v.as_f64()) {
        if rho <= 0.0 || !rho.is_finite() {
            return Err(VenturiError::PhysicsBounds("density must be > 0.".into()));
        }
        fluid.density = rho;
        fluid.kinematic_viscosity = fluid.dynamic_viscosity / fluid.density;
    }
    if let Some(mu) = args.get("dynamicViscosity").and_then(|v| v.as_f64()) {
        if mu <= 0.0 || !mu.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "dynamicViscosity must be > 0.".into(),
            ));
        }
        fluid.dynamic_viscosity = mu;
        fluid.kinematic_viscosity = fluid.dynamic_viscosity / fluid.density;
    }
    if let Some(nu) = args.get("kinematicViscosity").and_then(|v| v.as_f64()) {
        if nu <= 0.0 || !nu.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "kinematicViscosity must be > 0.".into(),
            ));
        }
        fluid.kinematic_viscosity = nu;
        fluid.dynamic_viscosity = fluid.kinematic_viscosity * fluid.density;
    }

    let mut new_velocity = state.case.physics.reference_velocity;
    if let Some(vel) = args.get("velocity").and_then(|v| v.as_f64()) {
        if vel <= 0.0 || !vel.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "velocity must be strictly positive.".into(),
            ));
        }
        new_velocity = vel;
    }

    let mut new_l_ref = state.case.physics.reference_length;
    if let Some(l_ref) = args.get("referenceLength").and_then(|v| v.as_f64()) {
        if l_ref <= 0.0 || !l_ref.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "referenceLength must be strictly positive.".into(),
            ));
        }
        new_l_ref = l_ref;
    }

    let mut new_turb_model = state.case.physics.turbulence;
    if let Some(tm) = args.get("turbulenceModel").and_then(|v| v.as_str()) {
        new_turb_model = parse_turbulence_model(tm)?;
    }

    let mut new_regime = state.case.physics.regime;
    if let Some(rg) = args.get("regime").and_then(|v| v.as_str()) {
        new_regime = parse_flow_regime(rg)?;
    }

    let intensity_pct = if let Some(ti) = args
        .get("turbulenceIntensityPct")
        .and_then(|v| v.as_f64())
    {
        if ti <= 0.0 || ti > 100.0 || !ti.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "turbulenceIntensityPct must be in (0, 100].".into(),
            ));
        }
        ti
    } else {
        (state.case.physics.turbulence_ic.intensity * 100.0).clamp(0.1, 50.0)
    };

    let length_scale = (0.07 * new_l_ref).max(1e-4);
    let ic = compute_turbulence_ic(&TurbulenceIcInput {
        velocity: new_velocity,
        intensity_percent: intensity_pct,
        length_scale,
        kinematic_viscosity: fluid.kinematic_viscosity,
    })?;

    // Commit physics updates
    state.case.physics.fluid = fluid;
    state.case.physics.reference_velocity = new_velocity;
    state.case.physics.reference_length = new_l_ref;
    state.case.physics.turbulence = new_turb_model;
    state.case.physics.regime = new_regime;
    state.case.physics.turbulence_ic = TurbulenceIC {
        intensity: intensity_pct / 100.0,
        length_scale,
        k: ic.k,
        epsilon: ic.epsilon,
        omega: ic.omega,
        nut: ic.nut,
        nu_tilda: ic.nu_tilda,
    };

    // Update inlet velocity patches (preserving direction if non-zero) and matched moving walls
    if (new_velocity - old_u_ref).abs() > 1e-12 || args.get("velocity").is_some() {
        for patch in &mut state.case.boundaries {
            match patch.kind {
                BoundaryKind::VelocityInlet => {
                    let mag = (patch.velocity[0].powi(2)
                        + patch.velocity[1].powi(2)
                        + patch.velocity[2].powi(2))
                    .sqrt();
                    if mag > 1e-6 {
                        let scale = new_velocity / mag;
                        patch.velocity = [
                            patch.velocity[0] * scale,
                            patch.velocity[1] * scale,
                            patch.velocity[2] * scale,
                        ];
                    } else {
                        patch.velocity = [new_velocity, 0.0, 0.0];
                    }
                }
                BoundaryKind::MovingWall => {
                    let mag = (patch.velocity[0].powi(2)
                        + patch.velocity[1].powi(2)
                        + patch.velocity[2].powi(2))
                    .sqrt();
                    if mag > 1e-6 && (mag - old_u_ref).abs() <= 0.1 * old_u_ref.max(1.0) {
                        let scale = new_velocity / mag;
                        patch.velocity = [
                            patch.velocity[0] * scale,
                            patch.velocity[1] * scale,
                            patch.velocity[2] * scale,
                        ];
                    }
                }
                _ => {}
            }
        }
    }

    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "fluid": state.case.physics.fluid,
        "referenceVelocity": state.case.physics.reference_velocity,
        "referenceLength": state.case.physics.reference_length,
        "turbulence": state.case.physics.turbulence,
        "regime": state.case.physics.regime,
        "turbulenceIc": state.case.physics.turbulence_ic,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Set fluid={}, U={:.2} m/s, L={:.3} m, Re={:.2e}",
        state.case.physics.fluid.name, new_velocity, new_l_ref, telemetry.reynolds
    );
    Ok((out, summary, true))
}

fn handle_set_guardrails(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let mut g = state.guardrails.clone();

    if let Some(v) = args.get("maxIterations").and_then(|v| v.as_u64()) {
        if v == 0 || v > 100 {
            return Err(VenturiError::PhysicsBounds(
                "maxIterations must be between 1 and 100.".into(),
            ));
        }
        g.max_iterations = v as u32;
    }
    if let Some(v) = args.get("maxCells").and_then(|v| v.as_u64()) {
        if v < 1_000 || v > 100_000_000 {
            return Err(VenturiError::PhysicsBounds(
                "maxCells must be between 1,000 and 100,000,000.".into(),
            ));
        }
        g.max_cells = v;
    }
    if let Some(v) = args.get("maxRamMb").and_then(|v| v.as_f64()) {
        if v < 128.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "maxRamMb must be >= 128 MB.".into(),
            ));
        }
        g.max_ram_mb = v;
    }
    if let Some(v) = args.get("maxRefinementLevel").and_then(|v| v.as_u64()) {
        if v > 8 {
            return Err(VenturiError::PhysicsBounds(
                "maxRefinementLevel must be in 0..=8.".into(),
            ));
        }
        g.max_refinement_level = v as u32;
    }
    if let Some(v) = args.get("targetYPlusMin").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "targetYPlusMin must be > 0.".into(),
            ));
        }
        g.target_y_plus_min = v;
    }
    if let Some(v) = args.get("targetYPlusMax").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "targetYPlusMax must be > 0.".into(),
            ));
        }
        g.target_y_plus_max = v;
    }
    if g.target_y_plus_min >= g.target_y_plus_max {
        return Err(VenturiError::PhysicsBounds(
            "targetYPlusMin must be strictly less than targetYPlusMax.".into(),
        ));
    }
    if let Some(v) = args.get("maxSkewness").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "maxSkewness must be > 0.".into(),
            ));
        }
        g.max_skewness = v;
    }
    if let Some(v) = args.get("maxNonOrtho").and_then(|v| v.as_f64()) {
        if !(10.0..=89.0).contains(&v) {
            return Err(VenturiError::PhysicsBounds(
                "maxNonOrtho must be in [10, 89] degrees.".into(),
            ));
        }
        g.max_non_ortho = v;
    }
    if let Some(v) = args.get("autoRollback").and_then(|v| v.as_bool()) {
        g.auto_rollback = v;
    }

    state.guardrails = g.clone();
    let summary = format!(
        "Updated guardrails (maxIter={}, maxCells={}, maxLvl={}, y+=[{:.1},{:.1}], autoRollback={})",
        g.max_iterations,
        g.max_cells,
        g.max_refinement_level,
        g.target_y_plus_min,
        g.target_y_plus_max,
        g.auto_rollback
    );
    Ok((json!({ "guardrails": g }), summary, true))
}

fn handle_generate_parametric_geometry(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let raw_spec = args
        .get("spec")
        .or_else(|| if args.get("family").is_some() { Some(args) } else { None })
        .ok_or_else(|| {
            VenturiError::InvalidCase(
                "Missing required 'spec' object containing 'family' and geometry parameters.".into(),
            )
        })?;

    let spec = parse_parametric_spec_with_defaults(raw_spec)?;
    let auto_fit = args
        .get("autoFitDomain")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let (payload, _stl_bytes) = generate_parametric_stl(&spec)?;
    let l_ref = reference_length(&spec).max(1e-4);
    let fname = suggested_filename(&spec);

    let mut candidate = state.case.clone();
    candidate.geometry_kind = PresetGeometryKind::Parametric;
    candidate.parametric = Some(spec.clone());
    candidate.stl = Some(payload.summary.clone());
    candidate.mesh.snappy_enabled = true;
    candidate.physics.reference_length = l_ref;

    ensure_obstacle_patch(&mut candidate, default_obstacle_name_for_spec(&spec));

    if auto_fit {
        let upstream = args
            .get("upstreamFactor")
            .and_then(|v| v.as_f64())
            .unwrap_or(2.5);
        let wake = args
            .get("wakeFactor")
            .and_then(|v| v.as_f64())
            .unwrap_or(5.5);
        let lateral = args
            .get("lateralFactor")
            .and_then(|v| v.as_f64())
            .unwrap_or(2.2);
        candidate.domain = fit_domain_to_bounds(
            payload.summary.bounds_min,
            payload.summary.bounds_max,
            &candidate,
            upstream,
            wake,
            lateral,
        );
    }

    // Recompute turbulence IC for updated reference length
    let intensity_pct = (candidate.physics.turbulence_ic.intensity * 100.0).clamp(0.1, 50.0);
    let length_scale = (0.07 * l_ref).max(1e-4);
    if let Ok(ic) = compute_turbulence_ic(&TurbulenceIcInput {
        velocity: candidate.physics.reference_velocity.max(1e-4),
        intensity_percent: intensity_pct,
        length_scale,
        kinematic_viscosity: candidate.physics.fluid.kinematic_viscosity.max(1e-12),
    }) {
        candidate.physics.turbulence_ic = TurbulenceIC {
            intensity: intensity_pct / 100.0,
            length_scale,
            k: ic.k,
            epsilon: ic.epsilon,
            omega: ic.omega,
            nut: ic.nut,
            nu_tilda: ic.nu_tilda,
        };
    }

    enforce_mesh_guardrails(&candidate, &state.guardrails)?;
    state.case = candidate;

    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "filename": fname,
        "spec": spec,
        "stlSummary": payload.summary,
        "domain": state.case.domain,
        "referenceLength": state.case.physics.reference_length,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Generated parametric STL {} ({} tris, L_ref={:.3} m)",
        fname, payload.summary.triangle_count, l_ref
    );
    Ok((out, summary, true))
}

fn handle_import_stl(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let path_str = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VenturiError::InvalidCase("Missing required parameter 'path'.".into()))?;
    let auto_fit = args
        .get("autoFitDomain")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let payload = parse_stl_file(Path::new(path_str))?;
    let l_ref = (payload.summary.bounds_max[0] - payload.summary.bounds_min[0])
        .abs()
        .max(1e-3);

    let mut candidate = state.case.clone();
    candidate.geometry_kind = PresetGeometryKind::CustomStl;
    candidate.parametric = None;
    candidate.stl = Some(payload.summary.clone());
    candidate.mesh.snappy_enabled = true;
    candidate.physics.reference_length = l_ref;
    ensure_obstacle_patch(&mut candidate, "obstacle");

    if auto_fit {
        candidate.domain = fit_domain_to_bounds(
            payload.summary.bounds_min,
            payload.summary.bounds_max,
            &candidate,
            2.5,
            5.5,
            2.2,
        );
    }

    enforce_mesh_guardrails(&candidate, &state.guardrails)?;
    state.case = candidate;

    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "stlSummary": payload.summary,
        "domain": state.case.domain,
        "referenceLength": state.case.physics.reference_length,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Imported STL {} ({} tris)",
        payload.summary.filename, payload.summary.triangle_count
    );
    Ok((out, summary, true))
}

fn handle_auto_fit_domain(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let upstream = args
        .get("upstreamFactor")
        .and_then(|v| v.as_f64())
        .unwrap_or(2.5);
    let wake = args
        .get("wakeFactor")
        .and_then(|v| v.as_f64())
        .unwrap_or(5.5);
    let lateral = args
        .get("lateralFactor")
        .and_then(|v| v.as_f64())
        .unwrap_or(2.2);

    if upstream <= 0.0 || wake <= 0.0 || lateral <= 0.0 {
        return Err(VenturiError::PhysicsBounds(
            "upstreamFactor, wakeFactor, and lateralFactor must all be > 0.".into(),
        ));
    }

    let (bmin, bmax) = if let Some(ref stl) = state.case.stl {
        (stl.bounds_min, stl.bounds_max)
    } else if let Some(ref spec) = state.case.parametric {
        let (payload, _) = generate_parametric_stl(spec)?;
        state.case.stl = Some(payload.summary.clone());
        (payload.summary.bounds_min, payload.summary.bounds_max)
    } else {
        let half = 0.5 * state.case.physics.reference_length.max(0.1);
        ([-half, -0.25 * half, -0.25 * half], [half, 0.25 * half, 0.25 * half])
    };

    state.case.domain = fit_domain_to_bounds(bmin, bmax, &state.case, upstream, wake, lateral);
    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "domain": state.case.domain,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Auto-fit domain [{:.2},{:.2}]×[{:.2},{:.2}]×[{:.2},{:.2}] m",
        state.case.domain.x_min,
        state.case.domain.x_max,
        state.case.domain.y_min,
        state.case.domain.y_max,
        state.case.domain.z_min,
        state.case.domain.z_max
    );
    Ok((out, summary, true))
}

fn handle_update_mesh_config(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let mut candidate = state.case.clone();

    if let Some(v) = args.get("cellsX").and_then(|v| v.as_u64()) {
        candidate.mesh.cells_x = v as u32;
    }
    if let Some(v) = args.get("cellsY").and_then(|v| v.as_u64()) {
        candidate.mesh.cells_y = v as u32;
    }
    if let Some(v) = args.get("cellsZ").and_then(|v| v.as_u64()) {
        candidate.mesh.cells_z = v as u32;
    }
    if let Some(v) = args.get("gradingY").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds("gradingY must be > 0.".into()));
        }
        candidate.mesh.grading_y = v;
    }
    if let Some(v) = args.get("refinementMin").and_then(|v| v.as_u64()) {
        candidate.mesh.refinement_min = v as u32;
    }
    if let Some(v) = args.get("refinementMax").and_then(|v| v.as_u64()) {
        candidate.mesh.refinement_max = v as u32;
    }
    if let Some(v) = args.get("inflationLayers").and_then(|v| v.as_u64()) {
        if v > 40 {
            return Err(VenturiError::PhysicsBounds(
                "inflationLayers must be <= 40.".into(),
            ));
        }
        candidate.mesh.inflation_layers = v as u32;
    }
    if let Some(v) = args.get("firstLayerThickness").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "firstLayerThickness must be > 0.".into(),
            ));
        }
        candidate.mesh.first_layer_thickness = v;
    }
    if let Some(v) = args.get("expansionRatio").and_then(|v| v.as_f64()) {
        if v <= 1.0 || v > 3.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "expansionRatio must be in (1.0, 3.0].".into(),
            ));
        }
        candidate.mesh.expansion_ratio = v;
    }
    if let Some(v) = args.get("targetYPlus").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds(
                "targetYPlus must be > 0.".into(),
            ));
        }
        candidate.mesh.target_y_plus = v;
    }
    if let Some(v) = args.get("snappyEnabled").and_then(|v| v.as_bool()) {
        candidate.mesh.snappy_enabled = v;
    }

    // Strictly enforce guardrails BEFORE committing
    enforce_mesh_guardrails(&candidate, &state.guardrails)?;

    state.case = candidate;
    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "mesh": state.case.mesh,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Updated mesh {}×{}×{} (est {} cells, {:.0} MB, L{}-L{})",
        state.case.mesh.cells_x,
        state.case.mesh.cells_y,
        state.case.mesh.cells_z,
        telemetry.estimated_cells,
        telemetry.estimated_ram_mb,
        state.case.mesh.refinement_min,
        state.case.mesh.refinement_max
    );
    Ok((out, summary, true))
}

fn handle_upsert_refinement_box(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let raw_name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VenturiError::InvalidCase("Missing required parameter 'name'.".into()))?;
    let name = sanitize_ident(raw_name);
    if name.is_empty() {
        return Err(VenturiError::InvalidCase(
            "Refinement box 'name' must contain alphanumeric characters.".into(),
        ));
    }

    let min = parse_vec3(args.get("min"), "min")?;
    let max = parse_vec3(args.get("max"), "max")?;
    let level = args
        .get("level")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| VenturiError::InvalidCase("Missing required integer 'level'.".into()))?
        as u32;
    if level == 0 {
        return Err(VenturiError::PhysicsBounds(
            "Refinement box level must be >= 1.".into(),
        ));
    }

    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("Targeted volumetric gradient refinement")
        .to_string();

    let new_box = RefinementBox {
        name: name.clone(),
        min,
        max,
        level,
        reason,
    };

    let mut candidate = state.case.clone();
    if let Some(existing) = candidate
        .mesh
        .refinement_boxes
        .iter_mut()
        .find(|b| b.name == name)
    {
        *existing = new_box.clone();
    } else {
        candidate.mesh.refinement_boxes.push(new_box.clone());
    }

    // Enforce guardrails BEFORE committing
    enforce_mesh_guardrails(&candidate, &state.guardrails)?;

    state.case = candidate;
    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "box": new_box,
        "totalBoxes": state.case.mesh.refinement_boxes.len(),
        "refinementBoxes": state.case.mesh.refinement_boxes,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Upserted refinement box '{}' at L{} (total: {})",
        name,
        level,
        state.case.mesh.refinement_boxes.len()
    );
    Ok((out, summary, true))
}

fn handle_remove_refinement_box(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let raw_name = args
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| VenturiError::InvalidCase("Missing required parameter 'name'.".into()))?;
    let sanitized = sanitize_ident(raw_name);
    let before = state.case.mesh.refinement_boxes.len();
    state
        .case
        .mesh
        .refinement_boxes
        .retain(|b| b.name != raw_name && b.name != sanitized);

    if state.case.mesh.refinement_boxes.len() == before {
        return Err(VenturiError::InvalidCase(format!(
            "Refinement box '{}' not found.",
            raw_name
        )));
    }

    let out = json!({
        "removed": raw_name,
        "remainingBoxes": state.case.mesh.refinement_boxes,
    });
    let summary = format!("Removed refinement box '{}'", raw_name);
    Ok((out, summary, true))
}

fn handle_auto_size_yplus(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let target_y_plus = args
        .get("targetYPlus")
        .and_then(|v| v.as_f64())
        .or(state.goal.target_y_plus)
        .unwrap_or(state.case.mesh.target_y_plus);

    let expansion_ratio = args
        .get("expansionRatio")
        .and_then(|v| v.as_f64())
        .unwrap_or(state.case.mesh.expansion_ratio.max(1.05));

    let yp = compute_y_plus(&YPlusInput {
        velocity: state.case.physics.reference_velocity,
        density: state.case.physics.fluid.density,
        dynamic_viscosity: state.case.physics.fluid.dynamic_viscosity,
        reference_length: state.case.physics.reference_length,
        target_y_plus,
        expansion_ratio,
    })?;

    let mut candidate = state.case.clone();
    candidate.mesh.target_y_plus = target_y_plus;
    candidate.mesh.expansion_ratio = expansion_ratio;
    candidate.mesh.first_layer_thickness = yp.first_cell_height;
    candidate.mesh.inflation_layers = yp.recommended_layers;

    enforce_mesh_guardrails(&candidate, &state.guardrails)?;
    state.case = candidate;

    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "yPlusResult": yp,
        "mesh": state.case.mesh,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Auto-sized y+={:.2} -> dy1={:.4} mm, {} layers (r={:.2})",
        target_y_plus,
        yp.first_cell_height * 1000.0,
        yp.recommended_layers,
        expansion_ratio
    );
    Ok((out, summary, true))
}

fn handle_update_numerics(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let mut n = state.case.numerics.clone();

    if let Some(acc) = args.get("accuracy").and_then(|v| v.as_str()) {
        n.accuracy = match acc.trim().to_lowercase().as_str() {
            "robustupwind" | "robust_upwind" | "upwind" => SchemeAccuracy::RobustUpwind,
            "balancedsecondorder" | "balanced_second_order" | "secondorder" => {
                SchemeAccuracy::BalancedSecondOrder
            }
            "highresolution" | "high_resolution" | "highres" => SchemeAccuracy::HighResolution,
            _ => {
                return Err(VenturiError::InvalidCase(format!(
                    "Invalid accuracy '{}'. Use robustUpwind, balancedSecondOrder, or highResolution.",
                    acc
                )));
            }
        };
    }

    if let Some(v) = args.get("relaxP").and_then(|v| v.as_f64()) {
        if !(0.05..=1.0).contains(&v) {
            return Err(VenturiError::PhysicsBounds("relaxP must be in [0.05, 1.0].".into()));
        }
        n.relax_p = v;
    }
    if let Some(v) = args.get("relaxU").and_then(|v| v.as_f64()) {
        if !(0.05..=1.0).contains(&v) {
            return Err(VenturiError::PhysicsBounds("relaxU must be in [0.05, 1.0].".into()));
        }
        n.relax_u = v;
    }
    if let Some(v) = args.get("relaxTurb").and_then(|v| v.as_f64()) {
        if !(0.05..=1.0).contains(&v) {
            return Err(VenturiError::PhysicsBounds("relaxTurb must be in [0.05, 1.0].".into()));
        }
        n.relax_turb = v;
    }
    if let Some(v) = args.get("endTime").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds("endTime must be > 0.".into()));
        }
        n.end_time = v;
    }
    if let Some(v) = args.get("deltaT").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds("deltaT must be > 0.".into()));
        }
        n.delta_t = v;
    }
    if let Some(v) = args.get("writeInterval").and_then(|v| v.as_f64()) {
        if v <= 0.0 || !v.is_finite() {
            return Err(VenturiError::PhysicsBounds("writeInterval must be > 0.".into()));
        }
        n.write_interval = v;
    }
    if let Some(v) = args.get("convergenceTol").and_then(|v| v.as_f64()) {
        if !(1e-9..=1e-1).contains(&v) {
            return Err(VenturiError::PhysicsBounds(
                "convergenceTol must be in [1e-9, 1e-1].".into(),
            ));
        }
        n.convergence_tol = v;
    }
    if let Some(v) = args.get("maxCo").and_then(|v| v.as_f64()) {
        if !(0.05..=20.0).contains(&v) {
            return Err(VenturiError::PhysicsBounds("maxCo must be in [0.05, 20.0].".into()));
        }
        n.max_co = v;
    }
    if let Some(v) = args.get("nNonOrthoCorrectors").and_then(|v| v.as_u64()) {
        if v > 10 {
            return Err(VenturiError::PhysicsBounds(
                "nNonOrthoCorrectors must be in 0..=10.".into(),
            ));
        }
        n.n_non_ortho_correctors = v as u32;
    }
    if let Some(v) = args.get("parallelCores").and_then(|v| v.as_u64()) {
        if !(1..=128).contains(&v) {
            return Err(VenturiError::PhysicsBounds(
                "parallelCores must be in 1..=128.".into(),
            ));
        }
        n.parallel_cores = v as u32;
    }

    state.case.numerics = n.clone();
    let telemetry = compute_telemetry(&state.case);
    let out = json!({
        "numerics": n,
        "telemetry": telemetry,
    });
    let summary = format!(
        "Updated numerics ({:?}, relaxP={:.2}, relaxU={:.2}, nonOrtho={}, cores={})",
        n.accuracy, n.relax_p, n.relax_u, n.n_non_ortho_correctors, n.parallel_cores
    );
    Ok((out, summary, true))
}

fn handle_validate_case(
    state: &crate::models::SessionState,
) -> Result<(Value, String, bool), VenturiError> {
    let validation = validate_case(&state.case);
    let guardrail_res = enforce_mesh_guardrails(&state.case, &state.guardrails);
    let telemetry = compute_telemetry(&state.case);

    let out = json!({
        "validation": validation,
        "guardrailsOk": guardrail_res.is_ok(),
        "guardrailError": guardrail_res.as_ref().err().map(|e| e.to_string()),
        "telemetry": telemetry,
    });
    let summary = format!(
        "Validation score {}/100 ({} errors, {} warnings, guardrailsOk={})",
        validation.score,
        validation.error_count,
        validation.warning_count,
        guardrail_res.is_ok()
    );
    Ok((out, summary, false))
}

async fn handle_run_iteration(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    // 1. Enforce iteration budget & mesh guardrails
    if state.iterations.len() as u32 >= state.guardrails.max_iterations {
        return Err(VenturiError::PhysicsBounds(format!(
            "Iteration budget exhausted: {}/{} iterations completed (guardrails.maxIterations). Raise guardrails or finalize.",
            state.iterations.len(),
            state.guardrails.max_iterations
        )));
    }
    enforce_mesh_guardrails(&state.case, &state.guardrails)?;

    // 2. Enforce pre-flight validation (hard errors block solve)
    let validation = validate_case(&state.case);
    if !validation.ready_to_run {
        let errs: Vec<String> = validation
            .items
            .iter()
            .filter(|i| i.severity == crate::models::DiagnosticSeverity::Error)
            .map(|i| format!("{}: {}", i.title, i.detail))
            .collect();
        return Err(VenturiError::InvalidCase(format!(
            "Case failed pre-flight validation and cannot run: {}",
            errs.join(" | ")
        )));
    }

    let next_index = (state.iterations.len() as u32) + 1;
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| format!("Iteration #{}", next_index));
    let changes_summary = args
        .get("changesSummary")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "Solver evaluation".to_string());
    let backend_req = args
        .get("backend")
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .trim()
        .to_lowercase();

    // 3. Determine case_dir and export OpenFOAM dictionaries + parametric STL
    let case_dir: PathBuf = match args.get("caseDir").and_then(|v| v.as_str()) {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p.trim()),
        _ => {
            if let Some(ref existing) = state.case_dir {
                let base = PathBuf::from(existing);
                // Put each iteration in its own sub-folder if base is a root workspace dir
                if base
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("iter_"))
                    .unwrap_or(false)
                {
                    base.parent()
                        .unwrap_or_else(|| Path::new("."))
                        .join(format!("iter_{:03}", next_index))
                } else {
                    base.join(format!("iter_{:03}", next_index))
                }
            } else {
                std::env::temp_dir()
                    .join(format!("venturi_case_{}", sanitize_ident(&state.case.name)))
                    .join(format!("iter_{:03}", next_index))
            }
        }
    };

    let exported_files = export_case_to_disk(&state.case, &case_dir)?;
    state.case_dir = Some(case_dir.to_string_lossy().to_string());

    // 4. Execute solver backend (local / docker when explicitly chosen & available, else surrogate)
    let env = detect_environment();
    let results: OpenFoamResultSummary = match backend_req.as_str() {
        "demo" => evaluate_surrogate_results(&state.case),
        "local" => {
            if env.local_binary.is_none() {
                return Err(VenturiError::Execution(
                    "Local OpenFOAM binaries not found in PATH. Choose backend='docker' or 'demo'."
                        .into(),
                ));
            }
            run_openfoam_allrun(&case_dir, &state.case, false).await?;
            ingest_openfoam_results(&case_dir, &state.case, "local")?
        }
        "docker" => {
            if !env.docker_running {
                return Err(VenturiError::Execution(
                    "Docker daemon is not running. Start Docker or choose backend='demo'.".into(),
                ));
            }
            run_openfoam_allrun(&case_dir, &state.case, true).await?;
            ingest_openfoam_results(&case_dir, &state.case, "docker")?
        }
        _ => {
            // "auto": run local OpenFOAM if natively installed in PATH; otherwise fast surrogate
            if env.local_binary.is_some() {
                if run_openfoam_allrun(&case_dir, &state.case, false).await.is_ok() {
                    ingest_openfoam_results(&case_dir, &state.case, "local")
                        .unwrap_or_else(|_| evaluate_surrogate_results(&state.case))
                } else {
                    evaluate_surrogate_results(&state.case)
                }
            } else {
                evaluate_surrogate_results(&state.case)
            }
        }
    };

    // 5. Score iteration, track best, and apply auto-rollback guardrails
    let objective_score =
        compute_objective_score(&state.case, &results, &state.goal, &state.guardrails);
    let case_snapshot = state.case.clone();
    let prev_best = state
        .best_iteration_index
        .and_then(|idx| state.iterations.iter().find(|r| r.index == idx).cloned());

    let mut is_best = false;
    let mut rolled_back = false;

    match &prev_best {
        None => {
            is_best = true;
            state.best_iteration_index = Some(next_index);
        }
        Some(best) => {
            let severe_regression = results.diverged
                || !results.check_mesh.mesh_ok
                || objective_score < best.objective_score - 5.0;

            if !results.diverged
                && results.check_mesh.mesh_ok
                && objective_score > best.objective_score
            {
                for rec in &mut state.iterations {
                    rec.is_best = false;
                }
                is_best = true;
                state.best_iteration_index = Some(next_index);
            } else if state.guardrails.auto_rollback && severe_regression {
                state.case = best.case_snapshot.clone();
                rolled_back = true;
            }
        }
    }

    let delta_vs_best = compute_delta_vs_record(&results, objective_score, prev_best.as_ref());
    let checklist = build_guardrail_checklist(&results, &state.guardrails, &state.goal);

    let record = IterationRecord {
        index: next_index,
        timestamp: chrono::Utc::now().to_rfc3339(),
        label: label.clone(),
        changes_summary: changes_summary.clone(),
        case_snapshot,
        results: results.clone(),
        objective_score,
        is_best,
        rolled_back,
    };
    state.iterations.push(record);

    let out = json!({
        "iterationIndex": next_index,
        "label": label,
        "changesSummary": changes_summary,
        "caseDir": case_dir.to_string_lossy(),
        "exportedFiles": exported_files,
        "objectiveScore": objective_score,
        "isBest": is_best,
        "rolledBack": rolled_back,
        "bestIterationIndex": state.best_iteration_index,
        "deltaVsBest": delta_vs_best,
        "guardrailCompliance": checklist,
        "results": results,
    });

    let summary = format!(
        "Iter #{} '{}' score={:.1}/100 ({}{}{})",
        next_index,
        label,
        objective_score,
        results.backend_used,
        if is_best { ", BEST" } else { "" },
        if rolled_back { ", AUTO-ROLLED BACK" } else { "" }
    );
    Ok((out, summary, true))
}

fn handle_inspect_openfoam_results(
    state: &crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    if state.iterations.is_empty() {
        return Err(VenturiError::InvalidCase(
            "No solver iterations exist yet. Call venturi_run_iteration first.".into(),
        ));
    }

    let rec = if let Some(idx) = args.get("iterationIndex").and_then(|v| v.as_u64()) {
        let target = idx as u32;
        state
            .iterations
            .iter()
            .find(|r| r.index == target)
            .ok_or_else(|| {
                VenturiError::InvalidCase(format!("Iteration #{} not found.", target))
            })?
    } else {
        state.iterations.last().unwrap()
    };

    let best_rec = state
        .best_iteration_index
        .and_then(|bidx| state.iterations.iter().find(|r| r.index == bidx));
    let delta_vs_best = compute_delta_vs_record(&rec.results, rec.objective_score, best_rec);
    let checklist = build_guardrail_checklist(&rec.results, &state.guardrails, &state.goal);
    let recommendations =
        build_engineering_recommendations(&state.case, &rec.results, &state.guardrails, &state.goal);

    let out = json!({
        "iteration": {
            "index": rec.index,
            "timestamp": rec.timestamp,
            "label": rec.label,
            "changesSummary": rec.changes_summary,
            "objectiveScore": rec.objective_score,
            "isBest": rec.is_best,
            "rolledBack": rec.rolled_back,
        },
        "bestIterationIndex": state.best_iteration_index,
        "deltaVsBest": delta_vs_best,
        "guardrailCompliance": checklist,
        "results": rec.results,
        "recommendations": recommendations,
    });

    let summary = format!(
        "Inspected iter #{} (score={:.1}, {} hotspots, {} recommendations)",
        rec.index,
        rec.objective_score,
        rec.results.hotspots.len(),
        recommendations.len()
    );
    Ok((out, summary, false))
}

fn handle_rollback_iteration(
    state: &mut crate::models::SessionState,
    args: &Value,
) -> Result<(Value, String, bool), VenturiError> {
    let target = args
        .get("iterationIndex")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);
    let restored_idx = rollback_to_iteration(state, target)?;
    let telemetry = compute_telemetry(&state.case);

    let out = json!({
        "rolledBackToIndex": restored_idx,
        "caseName": state.case.name,
        "geometryKind": state.case.geometry_kind,
        "parametric": state.case.parametric,
        "mesh": state.case.mesh,
        "telemetry": telemetry,
    });
    let summary = format!("Rolled back active case to iteration #{}", restored_idx);
    Ok((out, summary, true))
}

// ============================================================================
// Helpers
// ============================================================================

async fn run_openfoam_allrun(
    case_dir: &Path,
    case: &CfdCase,
    use_docker: bool,
) -> Result<(), VenturiError> {
    let mut cmd = if use_docker {
        let image = match case.flavor {
            OpenFoamFlavor::FoundationV14 => "openfoam/openfoam14-paraview512",
            OpenFoamFlavor::OpenCfdV2606 => "opencfd/openfoam-default:2512",
        };
        let mut c = Command::new("docker");
        c.args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/case", case_dir.display()),
            "-w",
            "/case",
            image,
            "sh",
            "-c",
            "./Allrun",
        ]);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("./Allrun").current_dir(case_dir);
        c
    };

    let output = tokio::time::timeout(Duration::from_secs(300), cmd.output())
        .await
        .map_err(|_| VenturiError::Execution("OpenFOAM Allrun timed out after 300s.".into()))?
        .map_err(|e| VenturiError::Execution(format!("Failed to invoke OpenFOAM Allrun: {}", e)))?;

    // Save combined log for post-mortem inspection if needed
    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::write(case_dir.join("log.allrun"), combined);

    Ok(())
}

fn compact_iteration_summary(rec: &IterationRecord) -> Value {
    let avg_yplus = average_y_plus(&rec.results);
    json!({
        "index": rec.index,
        "label": rec.label,
        "objectiveScore": rec.objective_score,
        "isBest": rec.is_best,
        "rolledBack": rec.rolled_back,
        "converged": rec.results.converged,
        "diverged": rec.results.diverged,
        "cd": rec.results.cd,
        "cl": rec.results.cl,
        "deltaPPa": rec.results.delta_p_pa,
        "avgYPlus": avg_yplus,
        "cells": rec.results.check_mesh.cells,
        "hotspotCount": rec.results.hotspots.len(),
    })
}

fn average_y_plus(res: &OpenFoamResultSummary) -> Option<f64> {
    if res.y_plus_patches.is_empty() {
        return None;
    }
    let sum: f64 = res.y_plus_patches.iter().map(|p| p.average).sum();
    Some(sum / (res.y_plus_patches.len() as f64))
}

fn compute_delta_vs_record(
    current: &OpenFoamResultSummary,
    current_score: f64,
    baseline: Option<&IterationRecord>,
) -> Value {
    let Some(base) = baseline else {
        return json!(null);
    };
    json!({
        "referenceIndex": base.index,
        "scoreDelta": current_score - base.objective_score,
        "cdDelta": match (current.cd, base.results.cd) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
        "clDelta": match (current.cl, base.results.cl) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
        "deltaPDeltaPa": match (current.delta_p_pa, base.results.delta_p_pa) {
            (Some(a), Some(b)) => Some(a - b),
            _ => None,
        },
    })
}

fn build_guardrail_checklist(
    res: &OpenFoamResultSummary,
    guardrails: &GuardrailsConfig,
    goal: &OptimizationGoal,
) -> Value {
    let converged = res.converged && !res.diverged;
    let mesh_ok = res.check_mesh.mesh_ok && res.check_mesh.failed_checks == 0;
    let cells_ok = res.check_mesh.cells <= guardrails.max_cells;
    let skewness_ok = res.check_mesh.max_skewness <= guardrails.max_skewness;
    let non_ortho_ok = res.check_mesh.max_non_orthogonality <= guardrails.max_non_ortho;
    let y_plus_ok = res.y_plus_patches.is_empty()
        || res.y_plus_patches.iter().all(|p| {
            p.average >= guardrails.target_y_plus_min && p.average <= guardrails.target_y_plus_max
        });
    let delta_p_ok = match (goal.max_delta_p_pa, res.delta_p_pa) {
        (Some(limit), Some(dp)) => dp <= limit,
        _ => true,
    };

    let all_passed =
        converged && mesh_ok && cells_ok && skewness_ok && non_ortho_ok && y_plus_ok && delta_p_ok;

    json!({
        "converged": converged,
        "meshOk": mesh_ok,
        "cellsOk": cells_ok,
        "skewnessOk": skewness_ok,
        "nonOrthoOk": non_ortho_ok,
        "yPlusOk": y_plus_ok,
        "deltaPOk": delta_p_ok,
        "allPassed": all_passed,
    })
}

fn build_engineering_recommendations(
    case: &CfdCase,
    res: &OpenFoamResultSummary,
    guardrails: &GuardrailsConfig,
    goal: &OptimizationGoal,
) -> Vec<Value> {
    let mut recs = Vec::new();

    // 1. Wall y+ sizing recommendation
    let target_yp = goal
        .target_y_plus
        .unwrap_or(0.5 * (guardrails.target_y_plus_min + guardrails.target_y_plus_max))
        .clamp(guardrails.target_y_plus_min, guardrails.target_y_plus_max);

    let yplus_out_of_band = res.y_plus_patches.iter().any(|p| {
        p.average < guardrails.target_y_plus_min || p.average > guardrails.target_y_plus_max
    });
    if yplus_out_of_band {
        let worst = res
            .y_plus_patches
            .iter()
            .max_by(|a, b| a.average.total_cmp(&b.average))
            .map(|p| format!("{} (y+ avg={:.2})", p.patch, p.average))
            .unwrap_or_default();
        recs.push(json!({
            "priority": "high",
            "category": "boundaryLayer",
            "summary": format!(
                "Wall y+ on {} is outside [{:.1}, {:.1}]. Auto-size firstLayerThickness & inflationLayers.",
                worst, guardrails.target_y_plus_min, guardrails.target_y_plus_max
            ),
            "suggestedTool": "venturi_auto_size_yplus",
            "suggestedArgs": {
                "targetYPlus": target_yp,
                "expansionRatio": case.mesh.expansion_ratio.clamp(1.15, 1.25)
            }
        }));
    }

    // 2. Volumetric refinement box suggestions for uncovered hotspots
    for hotspot in &res.hotspots {
        let clamped_level = hotspot
            .suggested_level
            .clamp(1, guardrails.max_refinement_level.max(1));
        let already_covered = case.mesh.refinement_boxes.iter().any(|b| {
            b.level >= clamped_level
                && hotspot.center[0] >= b.min[0]
                && hotspot.center[0] <= b.max[0]
                && hotspot.center[1] >= b.min[1]
                && hotspot.center[1] <= b.max[1]
                && hotspot.center[2] >= b.min[2]
                && hotspot.center[2] <= b.max[2]
        });
        if !already_covered {
            recs.push(json!({
                "priority": if hotspot.gradient_score >= 0.6 { "high" } else { "medium" },
                "category": "meshRefinement",
                "summary": format!(
                    "Unresolved gradient hotspot '{}' (score {:.2}): {}",
                    hotspot.id, hotspot.gradient_score, hotspot.reason
                ),
                "suggestedTool": "venturi_upsert_refinement_box",
                "suggestedArgs": {
                    "name": sanitize_ident(&format!("ref_{}", hotspot.id)),
                    "min": hotspot.suggested_box_min,
                    "max": hotspot.suggested_box_max,
                    "level": clamped_level,
                    "reason": hotspot.reason
                }
            }));
        }
    }

    // 3. Numerics stability recommendation
    if res.diverged || !res.converged || res.check_mesh.max_non_orthogonality > 55.0 {
        recs.push(json!({
            "priority": "high",
            "category": "numerics",
            "summary": format!(
                "Solver stability/mesh orthogonality attention (converged={}, maxNonOrtho={:.1}°): add non-orthogonal correctors and conservative relaxation.",
                res.converged, res.check_mesh.max_non_orthogonality
            ),
            "suggestedTool": "venturi_update_numerics",
            "suggestedArgs": {
                "nNonOrthoCorrectors": 2,
                "relaxP": 0.25,
                "relaxU": 0.6,
                "accuracy": if res.diverged { "robustUpwind" } else { "balancedSecondOrder" }
            }
        }));
    }

    // 4. Parametric geometry optimization hint
    if let Some(ref spec) = case.parametric {
        let geom_hint = match spec {
            ParametricGeometrySpec::NacaAirfoil {
                chord,
                camber_pct,
                camber_pos_pct,
                thickness_pct,
                aoa_deg,
                span,
            } => {
                let (next_aoa, next_camber, next_thick, desc) = if goal.maximize_cl_over_cd {
                    (
                        5.0_f64,
                        (*camber_pct).max(2.0).min(4.0),
                        thickness_pct.clamp(10.0, 13.0),
                        "Increase lift-to-drag ratio by tuning camber and moderate AoA (~5°).",
                    )
                } else {
                    (
                        aoa_deg.clamp(-1.0, 1.5),
                        *camber_pct,
                        (thickness_pct - 1.5).clamp(9.0, 14.0),
                        "Reduce profile & pressure drag by moderating AoA and thickness.",
                    )
                };
                Some((
                    desc.to_string(),
                    json!({
                        "spec": {
                            "family": "nacaAirfoil",
                            "chord": chord,
                            "camberPct": next_camber,
                            "camberPosPct": camber_pos_pct,
                            "thicknessPct": next_thick,
                            "aoaDeg": next_aoa,
                            "span": span
                        },
                        "autoFitDomain": false
                    }),
                ))
            }
            ParametricGeometrySpec::AhmedDiffuser {
                length,
                width,
                height,
                rear_slant_deg,
                ground_clearance,
            } => {
                if (*rear_slant_deg - 14.0).abs() > 2.5 {
                    Some((
                        "Shift Ahmed rear slant angle toward 14° low-drag attached-flow regime (avoiding 25-30° high-drag vortex burst).".into(),
                        json!({
                            "spec": {
                                "family": "ahmedDiffuser",
                                "length": length,
                                "width": width,
                                "height": height,
                                "rearSlantDeg": 14.0,
                                "groundClearance": ground_clearance
                            },
                            "autoFitDomain": false
                        }),
                    ))
                } else {
                    None
                }
            }
            ParametricGeometrySpec::VenturiTube {
                inlet_radius,
                throat_radius,
                outlet_radius,
                convergent_len,
                throat_len,
                divergent_len,
            } => Some((
                "Improve diffuser pressure recovery and cut head loss by extending divergent length and easing throat contraction.".into(),
                json!({
                    "spec": {
                        "family": "venturiTube",
                        "inletRadius": inlet_radius,
                        "throatRadius": (throat_radius * 1.1).min(inlet_radius * 0.78),
                        "outletRadius": outlet_radius,
                        "convergentLen": convergent_len,
                        "throatLen": throat_len,
                        "divergentLen": divergent_len * 1.15
                    },
                    "autoFitDomain": true
                }),
            )),
            ParametricGeometrySpec::PinFinArray {
                base_length,
                base_width,
                fin_count_x,
                fin_count_z,
                fin_radius,
                fin_height,
            } => Some((
                "Reduce pin-fin wake blockage and pressure drop by refining fin radius.".into(),
                json!({
                    "spec": {
                        "family": "pinFinArray",
                        "baseLength": base_length,
                        "baseWidth": base_width,
                        "finCountX": fin_count_x,
                        "finCountZ": fin_count_z,
                        "finRadius": (fin_radius * 0.88).max(0.004),
                        "finHeight": fin_height
                    },
                    "autoFitDomain": false
                }),
            )),
            ParametricGeometrySpec::Cylinder { radius, span } => Some((
                "Reduce frontal blockage by refining cylinder radius.".into(),
                json!({
                    "spec": {
                        "family": "cylinder",
                        "radius": (radius * 0.9).max(0.05),
                        "span": span
                    },
                    "autoFitDomain": false
                }),
            )),
        };

        if let Some((desc, suggested_args)) = geom_hint {
            recs.push(json!({
                "priority": "medium",
                "category": "geometryOptimization",
                "summary": desc,
                "suggestedTool": "venturi_generate_parametric_geometry",
                "suggestedArgs": suggested_args
            }));
        }
    }

    recs
}

fn parse_turbulence_model(s: &str) -> Result<TurbulenceModel, VenturiError> {
    match s.trim().to_lowercase().replace(['-', '_', ' '], "").as_str() {
        "laminar" => Ok(TurbulenceModel::Laminar),
        "komegasst" | "sst" | "komega" => Ok(TurbulenceModel::KOmegaSST),
        "kepsilon" | "keps" => Ok(TurbulenceModel::KEpsilon),
        "spalartallmaras" | "sa" => Ok(TurbulenceModel::SpalartAllmaras),
        "waleles" | "wale" | "les" => Ok(TurbulenceModel::WaleLes),
        _ => Err(VenturiError::InvalidCase(format!(
            "Unknown turbulenceModel '{}'. Use laminar, kOmegaSST, kEpsilon, spalartAllmaras, or waleLes.",
            s
        ))),
    }
}

fn parse_flow_regime(s: &str) -> Result<FlowRegime, VenturiError> {
    match s.trim().to_lowercase().replace(['-', '_', ' '], "").as_str() {
        "steadyincompressible" | "steady" => Ok(FlowRegime::SteadyIncompressible),
        "transientincompressible" | "transient" | "unsteady" => {
            Ok(FlowRegime::TransientIncompressible)
        }
        "steadycompressible" | "compressible" => Ok(FlowRegime::SteadyCompressible),
        _ => Err(VenturiError::InvalidCase(format!(
            "Unknown regime '{}'. Use steadyIncompressible, transientIncompressible, or steadyCompressible.",
            s
        ))),
    }
}

fn parse_vec3(val: Option<&Value>, field: &str) -> Result<[f64; 3], VenturiError> {
    let arr = val
        .and_then(|v| v.as_array())
        .ok_or_else(|| VenturiError::InvalidCase(format!("Missing [x,y,z] array '{}'.", field)))?;
    if arr.len() != 3 {
        return Err(VenturiError::InvalidCase(format!(
            "Parameter '{}' must have exactly 3 numbers.",
            field
        )));
    }
    let mut out = [0.0; 3];
    for (i, item) in arr.iter().enumerate() {
        let num = item.as_f64().ok_or_else(|| {
            VenturiError::InvalidCase(format!("Parameter '{}'[{}] must be a finite number.", field, i))
        })?;
        if !num.is_finite() {
            return Err(VenturiError::PhysicsBounds(format!(
                "Parameter '{}'[{}] must be finite.",
                field, i
            )));
        }
        out[i] = num;
    }
    Ok(out)
}

fn parse_parametric_spec_with_defaults(raw: &Value) -> Result<ParametricGeometrySpec, VenturiError> {
    if let Ok(spec) = serde_json::from_value::<ParametricGeometrySpec>(raw.clone()) {
        return Ok(spec);
    }

    let family = raw
        .get("family")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            VenturiError::InvalidCase(
                "Parametric spec requires 'family' (nacaAirfoil|venturiTube|ahmedDiffuser|pinFinArray|cylinder)."
                    .into(),
            )
        })?;

    let get_f64 = |k: &str, default: f64| -> f64 {
        raw.get(k).and_then(|v| v.as_f64()).unwrap_or(default)
    };
    let get_u32 = |k: &str, default: u32| -> u32 {
        raw.get(k)
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(default)
    };

    match family {
        "nacaAirfoil" => Ok(ParametricGeometrySpec::NacaAirfoil {
            chord: get_f64("chord", 1.0),
            camber_pct: get_f64("camberPct", 2.0),
            camber_pos_pct: get_f64("camberPosPct", 40.0),
            thickness_pct: get_f64("thicknessPct", 12.0),
            aoa_deg: get_f64("aoaDeg", 4.0),
            span: get_f64("span", 0.76),
        }),
        "venturiTube" => Ok(ParametricGeometrySpec::VenturiTube {
            inlet_radius: get_f64("inletRadius", 0.5),
            throat_radius: get_f64("throatRadius", 0.25),
            outlet_radius: get_f64("outletRadius", 0.5),
            convergent_len: get_f64("convergentLen", 1.2),
            throat_len: get_f64("throatLen", 0.8),
            divergent_len: get_f64("divergentLen", 2.8),
        }),
        "ahmedDiffuser" => Ok(ParametricGeometrySpec::AhmedDiffuser {
            length: get_f64("length", 1.044),
            width: get_f64("width", 0.389),
            height: get_f64("height", 0.288),
            rear_slant_deg: get_f64("rearSlantDeg", 25.0),
            ground_clearance: get_f64("groundClearance", 0.05),
        }),
        "pinFinArray" => Ok(ParametricGeometrySpec::PinFinArray {
            base_length: get_f64("baseLength", 0.2),
            base_width: get_f64("baseWidth", 0.14),
            fin_count_x: get_u32("finCountX", 4),
            fin_count_z: get_u32("finCountZ", 3),
            fin_radius: get_f64("finRadius", 0.008),
            fin_height: get_f64("finHeight", 0.045),
        }),
        "cylinder" => Ok(ParametricGeometrySpec::Cylinder {
            radius: get_f64("radius", 0.25),
            span: get_f64("span", 0.6),
        }),
        other => Err(VenturiError::InvalidCase(format!(
            "Unsupported parametric family '{}'. Use nacaAirfoil, venturiTube, ahmedDiffuser, pinFinArray, or cylinder.",
            other
        ))),
    }
}

fn default_obstacle_name_for_spec(spec: &ParametricGeometrySpec) -> &'static str {
    match spec {
        ParametricGeometrySpec::NacaAirfoil { .. } => "airfoil",
        ParametricGeometrySpec::VenturiTube { .. } => "venturiWall",
        ParametricGeometrySpec::AhmedDiffuser { .. } => "ahmedBody",
        ParametricGeometrySpec::PinFinArray { .. } => "heatSink",
        ParametricGeometrySpec::Cylinder { .. } => "cylinder",
    }
}

fn ensure_obstacle_patch(case: &mut CfdCase, default_name: &str) {
    if !case
        .boundaries
        .iter()
        .any(|b| b.face == DomainFace::Obstacle)
    {
        case.boundaries.push(BoundaryPatch {
            name: default_name.to_string(),
            face: DomainFace::Obstacle,
            kind: BoundaryKind::NoSlipWall,
            velocity: [0.0, 0.0, 0.0],
            pressure: 0.0,
            wall_roughness_ks: 0.0,
        });
    }
}

fn fit_domain_to_bounds(
    bmin: [f64; 3],
    bmax: [f64; 3],
    case: &CfdCase,
    upstream_factor: f64,
    wake_factor: f64,
    lateral_factor: f64,
) -> DomainBox {
    let lx = (bmax[0] - bmin[0]).abs().max(0.05);
    let ly = (bmax[1] - bmin[1]).abs().max(0.05);
    let lz = (bmax[2] - bmin[2]).abs().max(0.05);
    let char_l = lx.max(ly);

    let x_min = bmin[0] - upstream_factor * lx;
    let x_max = bmax[0] + wake_factor * lx;

    let y_min = match &case.parametric {
        Some(ParametricGeometrySpec::AhmedDiffuser { .. }) if bmin[1] > 1e-4 => 0.0,
        Some(ParametricGeometrySpec::PinFinArray { .. }) => bmin[1] - (0.15 * ly).max(0.01),
        _ => bmin[1] - lateral_factor * char_l,
    };
    let y_max = bmax[1] + lateral_factor * char_l;

    let is_2d_z = case
        .boundaries
        .iter()
        .any(|b| matches!(b.face, DomainFace::ZMin | DomainFace::ZMax) && b.kind == BoundaryKind::Empty2D);

    let z_pad = if is_2d_z {
        (0.06 * lz).max(0.02)
    } else {
        (lateral_factor * 0.55 * lz.max(0.5 * char_l)).max(0.05)
    };

    DomainBox {
        x_min,
        x_max,
        y_min,
        y_max,
        z_min: bmin[2] - z_pad,
        z_max: bmax[2] + z_pad,
    }
}

fn sanitize_ident(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else if ch == '-' || ch == ' ' || ch == '.' {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    if trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        format!("z_{}", trimmed)
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_session_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "venturi_mcp_test_{}_{}.json",
            tag,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ))
    }

    #[tokio::test]
    async fn jsonrpc_initialize_ping_and_tools_list() {
        let session = temp_session_path("init");

        let init_resp = handle_jsonrpc_request(
            &session,
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": { "protocolVersion": "2024-11-05", "clientInfo": { "name": "test" } }
            }),
        )
        .await
        .expect("initialize must return response");
        assert_eq!(init_resp["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(init_resp["result"]["serverInfo"]["name"], "venturi-mcp");
        assert!(init_resp["result"]["capabilities"]["tools"].is_object());

        let notif_resp = handle_jsonrpc_request(
            &session,
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        )
        .await;
        assert!(notif_resp.is_none());

        let ping_resp = handle_jsonrpc_request(
            &session,
            json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
        )
        .await
        .expect("ping must respond");
        assert!(ping_resp["result"].is_object());

        let list_resp = handle_jsonrpc_request(
            &session,
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
        )
        .await
        .expect("tools/list must respond");
        let tools = list_resp["result"]["tools"]
            .as_array()
            .expect("tools must be an array");
        assert_eq!(tools.len(), 16);
        assert!(tools.iter().any(|t| t["name"] == "venturi_run_iteration"));
        assert!(tools
            .iter()
            .any(|t| t["name"] == "venturi_generate_parametric_geometry"));

        let _ = std::fs::remove_file(&session);
    }

    #[tokio::test]
    async fn guardrail_rejects_ten_million_cells() {
        let session = temp_session_path("guardrail");
        // Initialize session file first
        let _ = execute_tool(
            &session,
            "venturi_load_preset",
            json!({ "presetId": "naca0012" }),
            "mcp-external",
        )
        .await
        .unwrap();

        let err = execute_tool(
            &session,
            "venturi_update_mesh_config",
            json!({ "cellsX": 500, "cellsY": 200, "cellsZ": 100 }),
            "mcp-external",
        )
        .await;
        assert!(err.is_err(), "10M cells must be rejected by mesh guardrails");

        // Also check MCP JSON-RPC surfaces isError = true
        let rpc = handle_jsonrpc_request(
            &session,
            json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "tools/call",
                "params": {
                    "name": "venturi_update_mesh_config",
                    "arguments": { "cellsX": 500, "cellsY": 200, "cellsZ": 100 }
                }
            }),
        )
        .await
        .unwrap();
        assert_eq!(rpc["result"]["isError"], true);

        // Verify original session mesh was not mutated
        let reloaded = load_or_init_session(&session);
        assert_ne!(reloaded.case.mesh.cells_x, 500);

        let _ = std::fs::remove_file(&session);
    }

    #[tokio::test]
    async fn full_closed_loop_mcp_sequence_from_empty_case() {
        let session = temp_session_path("closed_loop");

        // 1. Start from empty workspace (no geometry)
        let loaded = execute_tool(
            &session,
            "venturi_load_preset",
            json!({ "presetId": "empty" }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert_eq!(loaded["presetId"], "empty");

        // 2. Set fluid & freestream conditions
        let fluid = execute_tool(
            &session,
            "venturi_set_fluid_conditions",
            json!({
                "fluidName": "air",
                "temperatureC": 20.0,
                "pressureKpa": 101.325,
                "velocity": 32.0,
                "turbulenceIntensityPct": 1.5,
                "turbulenceModel": "kOmegaSST",
                "regime": "steadyIncompressible"
            }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert!((fluid["referenceVelocity"].as_f64().unwrap() - 32.0).abs() < 1e-6);

        // 3. Synthesize parametric NACA 2412 airfoil STL & auto-fit domain
        let geom = execute_tool(
            &session,
            "venturi_generate_parametric_geometry",
            json!({
                "spec": {
                    "family": "nacaAirfoil",
                    "chord": 1.0,
                    "camberPct": 2.0,
                    "camberPosPct": 40.0,
                    "thicknessPct": 12.0,
                    "aoaDeg": 4.0,
                    "span": 0.76
                },
                "autoFitDomain": true
            }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert!(geom["stlSummary"]["triangleCount"].as_u64().unwrap() > 100);

        // 4. Run baseline iteration (surrogate demo backend)
        let iter1 = execute_tool(
            &session,
            "venturi_run_iteration",
            json!({
                "label": "Baseline NACA 2412",
                "changesSummary": "Initial generated parametric airfoil",
                "backend": "demo"
            }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert_eq!(iter1["iterationIndex"], 1);
        assert_eq!(iter1["isBest"], true);

        // 5. Inspect OpenFOAM results & hotspots
        let inspect = execute_tool(
            &session,
            "venturi_inspect_openfoam_results",
            json!({}),
            "mcp-external",
        )
        .await
        .unwrap();
        assert_eq!(inspect["iteration"]["index"], 1);
        assert!(inspect["recommendations"].is_array());

        // 6. Auto-size y+ boundary layer prism mesh
        let yp = execute_tool(
            &session,
            "venturi_auto_size_yplus",
            json!({ "targetYPlus": 1.2, "expansionRatio": 1.2 }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert!(yp["mesh"]["firstLayerThickness"].as_f64().unwrap() < 0.0002);

        // 7. Insert wake refinement box
        let rbox = execute_tool(
            &session,
            "venturi_upsert_refinement_box",
            json!({
                "name": "near_wake_zone",
                "min": [0.2, -0.35, -0.4],
                "max": [2.2, 0.35, 0.4],
                "level": 3,
                "reason": "Resolve trailing-edge shear layer and wake vortex"
            }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert_eq!(rbox["totalBoxes"], 1);

        // 8. Run adapted iteration #2
        let iter2 = execute_tool(
            &session,
            "venturi_run_iteration",
            json!({
                "label": "Adapted y+ & Wake Box",
                "changesSummary": "Applied target y+=1.2 and L3 wake refinement box",
                "backend": "demo"
            }),
            "mcp-external",
        )
        .await
        .unwrap();
        assert_eq!(iter2["iterationIndex"], 2);

        let final_state = load_or_init_session(&session);
        assert_eq!(final_state.iterations.len(), 2);
        assert_eq!(final_state.case.mesh.refinement_boxes.len(), 1);
        assert!(final_state.tool_logs.len() >= 8);

        let _ = std::fs::remove_file(&session);
    }
}
