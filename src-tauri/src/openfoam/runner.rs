use crate::error::VenturiError;
use crate::models::{CfdCase, EnvironmentStatus, OpenFoamFlavor, ResidualPoint, SolverEvent};
use crate::openfoam::generator::export_case_to_disk;
use crate::physics::resolve_solver_name;
use regex::Regex;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

#[derive(Default)]
pub struct RunnerState {
    pub cancel_flag: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
}

/// Probes the host system for local OpenFOAM binaries, macOS OpenFOAM.app, and Docker daemon.
pub fn detect_environment() -> EnvironmentStatus {
    let candidates = ["foamRun", "simpleFoam", "blockMesh", "openfoam2512", "openfoam2412"];
    let mut local_binary = None;
    let mut local_openfoam = None;
    let locate_cmd = if cfg!(windows) { "where" } else { "which" };

    for bin in candidates {
        if let Ok(out) = std::process::Command::new(locate_cmd).arg(bin).output() {
            if out.status.success() {
                let p = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !p.is_empty() {
                    local_binary = Some(p);
                    local_openfoam = std::env::var("WM_PROJECT_VERSION")
                        .ok()
                        .or_else(|| Some("Local OpenFOAM in PATH".into()));
                    break;
                }
            }
        }
    }

    let macos_app_path = if cfg!(target_os = "macos") {
        [
            "/Applications/OpenFOAM.app",
            "/Applications/OpenFOAM-v2512.app",
            "/Applications/OpenFOAM-v2412.app",
            "/Applications/OpenFOAM-14.app",
            "/Applications/OpenFOAM-13.app",
        ]
        .iter()
        .find(|p| PathBuf::from(p).exists())
        .map(|s| s.to_string())
    } else {
        None
    };

    let docker_installed = std::process::Command::new(locate_cmd)
        .arg("docker")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    let docker_running = if docker_installed {
        std::process::Command::new("docker")
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        false
    };

    let recommended_backend = if local_binary.is_some() {
        "local".to_string()
    } else if docker_running {
        "docker".to_string()
    } else {
        "demo".to_string()
    };

    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);

    EnvironmentStatus {
        local_openfoam,
        local_binary,
        macos_app_path,
        docker_installed,
        docker_running,
        recommended_backend,
        platform: format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH),
        cpu_cores,
    }
}

/// Starts an asynchronous CFD run (local OpenFOAM, Docker container, or Physics Benchmark Demo)
/// and streams structured log & residual events back to the Tauri frontend.
pub async fn launch_solver(
    app: AppHandle,
    state: Arc<RunnerState>,
    case: CfdCase,
    backend: String,
    case_dir: Option<String>,
) -> Result<String, VenturiError> {
    if state.running.swap(true, Ordering::SeqCst) {
        return Err(VenturiError::Execution(
            "A solver run is already active. Stop it before starting a new run.".into(),
        ));
    }
    state.cancel_flag.store(false, Ordering::SeqCst);

    let run_id = format!("run-{}", chrono::Utc::now().timestamp_millis());
    let run_id_clone = run_id.clone();

    tokio::spawn(async move {
        let res = match backend.as_str() {
            "local" => run_external_openfoam(&app, &state, &run_id_clone, &case, false, case_dir).await,
            "docker" => run_external_openfoam(&app, &state, &run_id_clone, &case, true, case_dir).await,
            _ => run_benchmark_simulation(&app, &state, &run_id_clone, &case).await,
        };

        if let Err(e) = res {
            let _ = app.emit(
                "solver-event",
                SolverEvent {
                    run_id: run_id_clone.clone(),
                    kind: "log".into(),
                    message: Some(format!("[ERROR] {}", e)),
                    residual: None,
                    status: Some("error".into()),
                },
            );
            let _ = app.emit(
                "solver-event",
                SolverEvent {
                    run_id: run_id_clone,
                    kind: "status".into(),
                    message: None,
                    residual: None,
                    status: Some("error".into()),
                },
            );
        }
        state.running.store(false, Ordering::SeqCst);
    });

    Ok(run_id)
}

async fn run_external_openfoam(
    app: &AppHandle,
    state: &Arc<RunnerState>,
    run_id: &str,
    case: &CfdCase,
    use_docker: bool,
    case_dir: Option<String>,
) -> Result<(), VenturiError> {
    let dir = match case_dir {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => std::env::temp_dir().join(format!("venturi_{}", case.name.replace(' ', "_"))),
    };

    let count = export_case_to_disk(case, &dir)?;
    emit_log(
        app,
        run_id,
        format!("[Venturi] Exported {} OpenFOAM files to {}", count, dir.display()),
    );
    emit_status(app, run_id, "running");

    let mut cmd = if use_docker {
        let image = match case.flavor {
            OpenFoamFlavor::FoundationV14 => "openfoam/openfoam14-paraview512",
            OpenFoamFlavor::OpenCfdV2606 => "opencfd/openfoam-default:2512",
        };
        emit_log(
            app,
            run_id,
            format!("[Venturi] Launching Docker container ({})...", image),
        );
        let mut c = Command::new("docker");
        c.args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/case", dir.display()),
            "-w",
            "/case",
            image,
            "sh",
            "-c",
            "cd /case && ./Allrun",
        ]);
        c
    } else {
        emit_log(
            app,
            run_id,
            format!(
                "[Venturi] Executing native ./Allrun ({})...",
                resolve_solver_name(case.flavor, case.physics.regime)
            ),
        );
        let mut c = Command::new("sh");
        c.arg("./Allrun").current_dir(&dir);
        c
    };

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| VenturiError::Execution(format!("Failed to launch process: {}", e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| VenturiError::Execution("Unable to capture stdout".into()))?;

    let mut reader = BufReader::new(stdout).lines();

    let time_re = Regex::new(r"^Time\s*=\s*([0-9eE.+\-]+)").unwrap();
    let res_re = Regex::new(
        r"Solving for ([A-Za-z0-9_]+),\s*Initial residual\s*=\s*([0-9eE.+\-]+)",
    )
    .unwrap();
    let cont_re = Regex::new(r"cumulative\s*=\s*([0-9eE.+\-]+)").unwrap();

    let mut current = ResidualPoint {
        iteration: 0.0,
        ux: None,
        uy: None,
        uz: None,
        p: None,
        k: None,
        omega: None,
        epsilon: None,
        cd: None,
        cl: None,
        continuity: None,
    };
    let mut has_data = false;

    while let Ok(Some(line)) = reader.next_line().await {
        if state.cancel_flag.load(Ordering::SeqCst) {
            let _ = child.kill().await;
            emit_log(app, run_id, "[Venturi] Solver terminated by user.".into());
            emit_status(app, run_id, "stopped");
            return Ok(());
        }

        emit_log(app, run_id, line.clone());

        if let Some(cap) = time_re.captures(&line) {
            if has_data {
                emit_residual(app, run_id, current.clone());
            }
            let t = cap[1].parse::<f64>().unwrap_or(current.iteration + 1.0);
            current = ResidualPoint {
                iteration: t,
                ux: None,
                uy: None,
                uz: None,
                p: None,
                k: None,
                omega: None,
                epsilon: None,
                cd: None,
                cl: None,
                continuity: None,
            };
            has_data = false;
        } else if let Some(cap) = res_re.captures(&line) {
            let field = &cap[1];
            if let Ok(val) = cap[2].parse::<f64>() {
                has_data = true;
                match field {
                    "Ux" => current.ux = Some(val),
                    "Uy" => current.uy = Some(val),
                    "Uz" => current.uz = Some(val),
                    "p" | "p_rgh" => current.p = Some(val),
                    "k" => current.k = Some(val),
                    "omega" => current.omega = Some(val),
                    "epsilon" => current.epsilon = Some(val),
                    _ => {}
                }
            }
        } else if let Some(cap) = cont_re.captures(&line) {
            if let Ok(val) = cap[1].parse::<f64>() {
                current.continuity = Some(val.abs());
            }
        }
    }

    if has_data {
        emit_residual(app, run_id, current);
    }

    let status = child
        .wait()
        .await
        .map_err(|e| VenturiError::Execution(e.to_string()))?;

    if status.success() {
        emit_log(app, run_id, "[Venturi] OpenFOAM execution completed successfully.".into());
        emit_status(app, run_id, "completed");
    } else {
        emit_log(
            app,
            run_id,
            format!("[Venturi] Process exited with status {}.", status),
        );
        emit_status(app, run_id, "error");
    }
    Ok(())
}

/// Interactive Rust physics-informed convergence simulator so engineers can test the live monitor
/// even before starting Docker or installing OpenFOAM on macOS.
async fn run_benchmark_simulation(
    app: &AppHandle,
    state: &Arc<RunnerState>,
    run_id: &str,
    case: &CfdCase,
) -> Result<(), VenturiError> {
    let solver = resolve_solver_name(case.flavor, case.physics.regime);
    emit_status(app, run_id, "running");
    emit_log(
        app,
        run_id,
        format!(
            "[Venturi Benchmark Mode] OpenFOAM/Docker inactive — running Rust live convergence preview for `{}`",
            solver
        ),
    );
    emit_log(
        app,
        run_id,
        format!(
            "/* blockMesh: {}×{}×{} hex cells | snappyHexMesh: {} | target y+ = {:.1} */",
            case.mesh.cells_x,
            case.mesh.cells_y,
            case.mesh.cells_z,
            if case.mesh.snappy_enabled { "ON" } else { "OFF" },
            case.mesh.target_y_plus
        ),
    );

    let max_iters = (case.numerics.end_time as usize).clamp(40, 220);
    let tol = case.numerics.convergence_tol.max(1e-7);

    // Base drag/lift targets depending on geometry preset
    let (cd_target, cl_target) = match case.geometry_kind {
        crate::models::PresetGeometryKind::AirfoilNaca0012 => (0.0185, 0.412),
        crate::models::PresetGeometryKind::AhmedBody => (0.285, -0.042),
        crate::models::PresetGeometryKind::CylinderCrossflow => (1.14, 0.02),
        crate::models::PresetGeometryKind::VenturiNozzle => (0.064, 0.001),
        _ => (0.045, 0.0),
    };

    for step in 1..=max_iters {
        if state.cancel_flag.load(Ordering::SeqCst) {
            emit_log(app, run_id, "[Venturi] Simulation stopped by user.".into());
            emit_status(app, run_id, "stopped");
            return Ok(());
        }

        let t = step as f64;
        let relax_speed = 0.045 + 0.035 * case.numerics.relax_u;
        let ripple = 1.0 + 0.14 * (t * 0.45).sin() + 0.07 * (t * 1.13).cos();

        let ux = (0.85 * (-t * relax_speed * 1.12).exp() * ripple).max(tol * 0.25);
        let uy = (0.72 * (-t * relax_speed * 1.05).exp() * (1.0 + 0.16 * (t * 0.6).cos())).max(tol * 0.3);
        let p = (0.95 * (-t * relax_speed * 0.88).exp() * (1.0 + 0.18 * (t * 0.35).sin())).max(tol * 0.45);
        let k = (0.65 * (-t * relax_speed * 0.96).exp() * ripple).max(tol * 0.35);
        let omega = (0.78 * (-t * relax_speed * 0.92).exp() * (1.0 + 0.12 * (t * 0.7).sin())).max(tol * 0.35);

        let transient_decay = (-t * 0.055).exp();
        let cd = cd_target * (1.0 + 0.65 * transient_decay * (t * 0.35).cos());
        let cl = cl_target * (1.0 - 0.55 * transient_decay) + 0.012 * transient_decay * (t * 0.5).sin();
        let continuity = (1e-3 * (-t * 0.06).exp()).max(1e-11);

        if step == 1 || step % 5 == 0 || step == max_iters {
            emit_log(
                app,
                run_id,
                format!(
                    "Time = {} | smoothSolver: Ux={:.2e}, Uy={:.2e} | GAMG: p={:.2e} | Cd={:.4}, Cl={:.4}",
                    step, ux, uy, p, cd, cl
                ),
            );
        }

        emit_residual(
            app,
            run_id,
            ResidualPoint {
                iteration: t,
                ux: Some(ux),
                uy: Some(uy),
                uz: Some(uy * 0.4),
                p: Some(p),
                k: Some(k),
                omega: Some(omega),
                epsilon: Some(k * 0.8),
                cd: Some(cd),
                cl: Some(cl),
                continuity: Some(continuity),
            },
        );

        if ux < tol && uy < tol && p < tol && step > 25 {
            emit_log(
                app,
                run_id,
                format!(
                    "[Venturi] SIMPLE solution converged in {} iterations (residuals < {:.1e}).",
                    step, tol
                ),
            );
            emit_status(app, run_id, "converged");
            return Ok(());
        }

        tokio::time::sleep(std::time::Duration::from_millis(65)).await;
    }

    emit_log(
        app,
        run_id,
        format!("[Venturi] Reached endTime ({} iterations).", max_iters),
    );
    emit_status(app, run_id, "completed");
    Ok(())
}

fn emit_log(app: &AppHandle, run_id: &str, message: String) {
    let _ = app.emit(
        "solver-event",
        SolverEvent {
            run_id: run_id.to_string(),
            kind: "log".into(),
            message: Some(message),
            residual: None,
            status: None,
        },
    );
}

fn emit_residual(app: &AppHandle, run_id: &str, residual: ResidualPoint) {
    let _ = app.emit(
        "solver-event",
        SolverEvent {
            run_id: run_id.to_string(),
            kind: "residual".into(),
            message: None,
            residual: Some(residual),
            status: None,
        },
    );
}

fn emit_status(app: &AppHandle, run_id: &str, status: &str) {
    let _ = app.emit(
        "solver-event",
        SolverEvent {
            run_id: run_id.to_string(),
            kind: "status".into(),
            message: None,
            residual: None,
            status: Some(status.to_string()),
        },
    );
}
