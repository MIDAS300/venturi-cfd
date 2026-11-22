use crate::error::VenturiError;
use crate::models::{
    BoundaryKind, CfdCase, CheckMeshSummary, DomainBox, DomainFace, FlowRegime, GradientHotspot,
    GuardrailsConfig, OpenFoamResultSummary, OptimizationGoal, ParametricGeometrySpec, PatchYPlus,
    PresetGeometryKind, RefinementBox, TurbulenceModel, YPlusInput,
};
use crate::physics::{compute_telemetry, compute_y_plus};
use regex::Regex;
use std::collections::BTreeMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};

/// Ingests real OpenFOAM execution logs and `postProcessing/` tables from `case_dir`
/// into a structured `OpenFoamResultSummary`.
pub fn ingest_openfoam_results(
    case_dir: &Path,
    case: &CfdCase,
    backend_used: &str,
) -> Result<OpenFoamResultSummary, VenturiError> {
    if !case_dir.exists() {
        return Err(VenturiError::InvalidCase(format!(
            "Case directory does not exist: {}",
            case_dir.display()
        )));
    }

    let mut notes = Vec::new();
    let telemetry = compute_telemetry(case);

    // 1. Parse log.checkMesh
    let check_mesh = match std::fs::read_to_string(case_dir.join("log.checkMesh")) {
        Ok(txt) => {
            let cm = parse_check_mesh_log(&txt, telemetry.estimated_cells);
            notes.push(format!(
                "Parsed log.checkMesh: {} cells, maxNonOrtho={:.1}°, maxSkew={:.2}, ok={}",
                cm.cells, cm.max_non_orthogonality, cm.max_skewness, cm.mesh_ok
            ));
            cm
        }
        Err(_) => {
            notes.push("log.checkMesh not found; falling back to estimated mesh metrics.".into());
            CheckMeshSummary {
                cells: telemetry.estimated_cells,
                max_non_orthogonality: 28.0,
                avg_non_orthogonality: 6.5,
                max_skewness: 1.1,
                max_aspect_ratio: 6.0,
                failed_checks: 0,
                mesh_ok: true,
            }
        }
    };

    // 2. Parse log.solver
    let solver_stats = match std::fs::read_to_string(case_dir.join("log.solver")) {
        Ok(txt) => {
            let stats = parse_solver_log(&txt, case.numerics.convergence_tol);
            notes.push(format!(
                "Parsed log.solver: {} iters, resU={:.2e}, resP={:.2e}, converged={}, diverged={}",
                stats.iterations_completed,
                stats.final_residual_u,
                stats.final_residual_p,
                stats.converged,
                stats.diverged
            ));
            stats
        }
        Err(_) => {
            notes.push("log.solver not found; solver convergence fields unavailable.".into());
            SolverLogStats {
                converged: false,
                diverged: false,
                iterations_completed: 0,
                final_residual_u: 1.0,
                final_residual_p: 1.0,
                continuity_error: 0.0,
            }
        }
    };

    // 3. Parse postProcessing/yPlus/*/yPlus.dat
    let y_plus_patches =
        if let Some(yp_path) = find_latest_postprocessing_file(case_dir, &["yPlus"], &["yPlus.dat"]) {
            match std::fs::read_to_string(&yp_path) {
                Ok(txt) => {
                    let patches = parse_y_plus_dat(&txt);
                    notes.push(format!(
                        "Parsed {} patch y+ entries from {}",
                        patches.len(),
                        yp_path.display()
                    ));
                    patches
                }
                Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };

    // 4. Parse postProcessing/forceCoeffs1/*/coefficient.dat
    let (cd, cl, cm) = if let Some(fc_path) = find_latest_postprocessing_file(
        case_dir,
        &["forceCoeffs1", "forceCoeffs"],
        &["coefficient.dat", "forceCoeffs.dat"],
    ) {
        match std::fs::read_to_string(&fc_path) {
            Ok(txt) => {
                let coeffs = parse_force_coeffs_dat(&txt);
                notes.push(format!("Parsed force coefficients from {}", fc_path.display()));
                coeffs
            }
            Err(_) => (None, None, None),
        }
    } else {
        (None, None, None)
    };

    // 5. Parse postProcessing/inletPressure & outletPressure surfaceFieldValue.dat
    let p_inlet = find_latest_postprocessing_file(
        case_dir,
        &["inletPressure"],
        &["surfaceFieldValue.dat"],
    )
    .and_then(|p| std::fs::read_to_string(p).ok())
    .and_then(|txt| parse_surface_field_value_dat(&txt));

    let p_outlet = find_latest_postprocessing_file(
        case_dir,
        &["outletPressure"],
        &["surfaceFieldValue.dat"],
    )
    .and_then(|p| std::fs::read_to_string(p).ok())
    .and_then(|txt| parse_surface_field_value_dat(&txt));

    let delta_p_pa = match (p_inlet, p_outlet) {
        (Some(pi), Some(po)) => {
            let dp = (pi - po) * case.physics.fluid.density;
            notes.push(format!(
                "Computed Δp = {:.2} Pa from inlet/outlet surfaceFieldValue.",
                dp
            ));
            Some(dp)
        }
        _ => None,
    };

    // 6. Parse postProcessing/probes or flowProbes U and p for GradientHotspots
    let probes_u = find_latest_postprocessing_file(case_dir, &["probes", "flowProbes"], &["U"])
        .and_then(|p| std::fs::read_to_string(p).ok());
    let probes_p = find_latest_postprocessing_file(case_dir, &["probes", "flowProbes"], &["p"])
        .and_then(|p| std::fs::read_to_string(p).ok());

    let hotspots = extract_probe_hotspots(probes_u.as_deref(), probes_p.as_deref(), case);
    if !hotspots.is_empty() {
        notes.push(format!(
            "Detected {} high-gradient spatial hotspot(s) from probe samples.",
            hotspots.len()
        ));
    }

    Ok(OpenFoamResultSummary {
        backend_used: backend_used.to_string(),
        converged: solver_stats.converged,
        diverged: solver_stats.diverged,
        iterations_completed: solver_stats.iterations_completed,
        final_residual_u: solver_stats.final_residual_u,
        final_residual_p: solver_stats.final_residual_p,
        continuity_error: solver_stats.continuity_error,
        cd,
        cl,
        cm,
        delta_p_pa,
        check_mesh,
        y_plus_patches,
        hotspots,
        notes,
    })
}

/// Physics-informed surrogate CFD evaluator used when OpenFOAM/Docker is unavailable.
/// Produces responsive, deterministic aerodynamic, hydraulic, y+, mesh-quality, and
/// spatial gradient-hotspot diagnostics coupled directly to geometry, fluid, mesh, and numerics.
pub fn evaluate_surrogate_results(case: &CfdCase) -> OpenFoamResultSummary {
    let telemetry = compute_telemetry(case);
    let mut notes = vec![
        "Evaluated with Venturi physics-informed surrogate CFD engine (no live OpenFOAM solver required).".to_string(),
    ];

    let rho = case.physics.fluid.density.max(1e-4);
    let mu = case.physics.fluid.dynamic_viscosity.max(1e-9);
    let u_ref = case.physics.reference_velocity.abs().max(1e-3);
    let l_ref = case.physics.reference_length.abs().max(1e-4);
    let re = telemetry.reynolds.max(10.0);
    let q_dyn = 0.5 * rho * u_ref * u_ref;

    // 1. Compute actual achieved wall y+ from case.mesh.first_layer_thickness vs required dy1(y+=1)
    let dy1_for_yp1 = compute_y_plus(&YPlusInput {
        velocity: u_ref,
        density: rho,
        dynamic_viscosity: mu,
        reference_length: l_ref,
        target_y_plus: 1.0,
        expansion_ratio: case.mesh.expansion_ratio.max(1.05),
    })
    .map(|r| r.first_cell_height.max(1e-9))
    .unwrap_or(1e-4);

    let effective_first_cell = if case.mesh.snappy_enabled && case.mesh.inflation_layers > 0 {
        case.mesh.first_layer_thickness.max(1e-8)
    } else {
        let dy_bg = case.domain.ly() / (case.mesh.cells_y.max(1) as f64);
        (dy_bg / case.mesh.grading_y.max(1.0).sqrt()).max(1e-7)
    };
    let actual_y_plus = (effective_first_cell / dy1_for_yp1).clamp(0.05, 2500.0);

    // 2. Geometry-aware hotspots (evaluated early so coverage informs mesh/drag fidelity)
    let hotspots = synthesize_surrogate_hotspots(case, l_ref);
    let covered_hotspots = hotspots.iter().filter(|h| h.gradient_score < 2.5).count();
    let total_hotspots = hotspots.len().max(1);
    let hotspot_coverage_frac = covered_hotspots as f64 / total_hotspots as f64;

    // 3. Realistic CheckMeshSummary sensitive to aspect ratio, grading, refinement jumps & boxes
    let is_2d = case.mesh.cells_z <= 1
        || case
            .boundaries
            .iter()
            .any(|b| b.kind == BoundaryKind::Empty2D);

    let dx = case.domain.lx() / (case.mesh.cells_x.max(1) as f64);
    let dy = case.domain.ly() / (case.mesh.cells_y.max(1) as f64);
    let dz = case.domain.lz() / (case.mesh.cells_z.max(1) as f64);

    let bg_ar = if is_2d {
        (dx / dy).max(dy / dx)
    } else {
        let max_d = dx.max(dy).max(dz);
        let min_d = dx.min(dy).min(dz).max(1e-9);
        max_d / min_d
    };

    let ref_jump = case
        .mesh
        .refinement_max
        .saturating_sub(case.mesh.refinement_min);

    let domain_vol = (case.domain.lx() * case.domain.ly() * case.domain.lz()).max(1e-9);
    let base_cells =
        (case.mesh.cells_x as f64) * (case.mesh.cells_y as f64) * (case.mesh.cells_z as f64);
    let mut box_extra_cells = 0.0_f64;
    let mut valid_boxes = 0usize;
    for b in &case.mesh.refinement_boxes {
        let bx = (b.max[0] - b.min[0]).abs();
        let by = (b.max[1] - b.min[1]).abs();
        let bz = (b.max[2] - b.min[2]).abs();
        let bvol = bx * by * bz;
        if bvol > 1e-12 {
            valid_boxes += 1;
            let frac = (bvol / domain_vol).clamp(0.002, 1.0);
            let factor = if is_2d {
                4.0_f64.powi(b.level.min(5) as i32) - 1.0
            } else {
                8.0_f64.powi(b.level.min(5) as i32) - 1.0
            };
            box_extra_cells += base_cells * frac * factor * 0.14;
        }
    }
    let total_cells = telemetry.estimated_cells + (box_extra_cells.round() as u64);

    let grading_penalty = (case.mesh.grading_y - 1.0).abs().max(0.0);
    let er_penalty = if case.mesh.expansion_ratio > 1.35 {
        (case.mesh.expansion_ratio - 1.35) * 30.0
    } else if case.mesh.expansion_ratio < 1.05 {
        (1.05 - case.mesh.expansion_ratio) * 20.0
    } else {
        0.0
    };
    let box_smooth_bonus = (valid_boxes.min(3) as f64) * 2.8;

    let mut max_non_ortho = if case.mesh.snappy_enabled { 38.0 } else { 10.0 };
    max_non_ortho += (bg_ar - 2.5).max(0.0) * 3.8;
    max_non_ortho += (ref_jump.saturating_sub(2) as f64) * 7.0;
    max_non_ortho += (grading_penalty - 4.0).max(0.0) * 2.2;
    max_non_ortho += er_penalty;
    if case.mesh.snappy_enabled && case.mesh.inflation_layers >= 6 && case.mesh.inflation_layers <= 16 {
        max_non_ortho -= 2.5;
    }
    max_non_ortho -= box_smooth_bonus;
    max_non_ortho = max_non_ortho.clamp(4.0, 82.0);
    let avg_non_ortho = (max_non_ortho * 0.21).clamp(1.2, 24.0);

    let mut max_skewness = if case.mesh.snappy_enabled { 1.65 } else { 0.30 };
    max_skewness += (bg_ar - 3.0).max(0.0) * 0.35;
    max_skewness += (ref_jump.saturating_sub(2) as f64) * 0.45;
    max_skewness += (grading_penalty - 5.0).max(0.0) * 0.20;
    if case.mesh.snappy_enabled && case.mesh.first_layer_thickness > dx * 0.8 {
        max_skewness += 1.4;
    }
    if case.mesh.refinement_max > 4 {
        max_skewness += 0.65 * ((case.mesh.refinement_max - 4) as f64);
    }
    max_skewness -= (valid_boxes.min(3) as f64) * 0.18;
    max_skewness = max_skewness.clamp(0.08, 8.5);

    let surf_dx = dx / 2.0_f64.powi(if case.mesh.snappy_enabled {
        case.mesh.refinement_max.min(5) as i32
    } else {
        0
    });
    let prism_ar = if case.mesh.snappy_enabled && case.mesh.inflation_layers > 0 {
        (surf_dx / case.mesh.first_layer_thickness.max(1e-7)).clamp(1.0, 220.0)
    } else {
        bg_ar
    };
    let max_aspect_ratio = (bg_ar.max(prism_ar * 0.45) * (1.0 + 0.08 * grading_penalty))
        .clamp(1.0, 300.0);

    let domain_invalid = case.domain.x_max <= case.domain.x_min
        || case.domain.y_max <= case.domain.y_min
        || case.domain.z_max <= case.domain.z_min;

    let stl_outside = case.stl.as_ref().is_some_and(|stl| {
        stl.bounds_min[0] <= case.domain.x_min
            || stl.bounds_max[0] >= case.domain.x_max
            || stl.bounds_min[1] <= case.domain.y_min
            || stl.bounds_max[1] >= case.domain.y_max
            || stl.bounds_min[2] < case.domain.z_min
            || stl.bounds_max[2] > case.domain.z_max
    });

    let mut failed_checks = 0u32;
    if domain_invalid {
        failed_checks += 2;
        notes.push("Domain bounds invalid (max <= min).".into());
    }
    if stl_outside {
        failed_checks += 1;
        notes.push("STL geometry exceeds computational domain bounds.".into());
    }
    if max_non_ortho > 70.0 {
        failed_checks += 1;
        notes.push(format!("checkMesh: max non-orthogonality {:.1}° exceeds 70°.", max_non_ortho));
    }
    if max_skewness > 4.0 {
        failed_checks += 1;
        notes.push(format!("checkMesh: max skewness {:.2} exceeds 4.0.", max_skewness));
    }
    let mesh_ok = failed_checks == 0;

    let check_mesh = CheckMeshSummary {
        cells: total_cells,
        max_non_orthogonality: max_non_ortho,
        avg_non_orthogonality: avg_non_ortho,
        max_skewness,
        max_aspect_ratio,
        failed_checks,
        mesh_ok,
    };

    // 4. Patch y+ table
    let spread_tightness = if case.mesh.inflation_layers >= 8 && valid_boxes > 0 {
        0.12
    } else {
        0.0
    };
    let mut y_plus_patches = Vec::new();
    for b in &case.boundaries {
        if matches!(b.kind, BoundaryKind::NoSlipWall | BoundaryKind::MovingWall) {
            let scale = if b.face == DomainFace::Obstacle { 1.0 } else { 0.88 };
            let avg = actual_y_plus * scale;
            let min = (avg * (0.42 + spread_tightness)).max(0.02);
            let max = avg * (1.75 - spread_tightness);
            y_plus_patches.push(PatchYPlus {
                patch: b.name.clone(),
                min,
                max,
                average: avg,
            });
        }
    }
    if y_plus_patches.is_empty() && case.mesh.snappy_enabled {
        y_plus_patches.push(PatchYPlus {
            patch: "obstacle".into(),
            min: (actual_y_plus * 0.45).max(0.02),
            max: actual_y_plus * 1.7,
            average: actual_y_plus,
        });
    }

    // 5. Convergence & stability model
    let has_inlet = case
        .boundaries
        .iter()
        .any(|b| b.kind == BoundaryKind::VelocityInlet);
    let has_outlet = case
        .boundaries
        .iter()
        .any(|b| b.kind == BoundaryKind::PressureOutlet);
    let bad_turb_ic = !matches!(
        case.physics.turbulence,
        TurbulenceModel::Laminar | TurbulenceModel::WaleLes
    ) && (case.physics.turbulence_ic.k <= 0.0 || case.physics.turbulence_ic.omega <= 0.0);

    let diverged = domain_invalid
        || (has_inlet && !has_outlet)
        || bad_turb_ic
        || case.numerics.end_time <= case.numerics.start_time
        || case.numerics.relax_p > 0.78
        || case.numerics.relax_u > 0.98
        || case.numerics.relax_turb > 0.96
        || max_skewness > 5.5
        || (max_non_ortho > 75.0 && case.numerics.n_non_ortho_correctors < 2)
        || (matches!(case.physics.regime, FlowRegime::TransientIncompressible)
            && telemetry.estimated_courant > 8.0);

    let tol = case.numerics.convergence_tol.max(1e-7);
    let (converged, iterations_completed, final_residual_u, final_residual_p, continuity_error) =
        if diverged {
            notes.push("Solver diverged due to unstable mesh quality, boundary setup, or relaxation factors.".into());
            (false, 12, 1.5e2, 4.2e2, 8.5)
        } else {
            let max_iters = match case.physics.regime {
                FlowRegime::TransientIncompressible => {
                    ((case.numerics.end_time - case.numerics.start_time)
                        / case.numerics.delta_t.max(1e-6))
                    .round()
                    .clamp(20.0, 4000.0) as u32
                }
                _ => ((case.numerics.end_time - case.numerics.start_time)
                    / case.numerics.delta_t.max(1e-3))
                .round()
                .clamp(20.0, 2000.0) as u32,
            };

            let non_ortho_drag = if max_non_ortho > 50.0 && case.numerics.n_non_ortho_correctors == 0 {
                0.65
            } else {
                1.0
            };
            let relax_rate = (0.022 + 0.032 * case.numerics.relax_u.clamp(0.1, 0.95)
                + 0.025 * case.numerics.relax_p.clamp(0.05, 0.7))
                * non_ortho_drag;
            let raw_u = 0.9 * (-(max_iters as f64) * relax_rate * 1.15).exp();
            let raw_p = 1.1 * (-(max_iters as f64) * relax_rate * 0.92).exp();
            let res_u = raw_u.max(tol * 0.35);
            let res_p = raw_p.max(tol * 0.55);
            let conv = res_u <= tol * 1.2 && res_p <= tol * 1.2 && mesh_ok;
            let iters_used = if conv {
                let needed = ((1.1 / (tol * 0.8)).ln() / (relax_rate * 0.92)).ceil() as u32;
                needed.clamp(35, max_iters)
            } else {
                max_iters
            };
            let cont = (res_p * 1e-2).clamp(1e-11, 1e-1);
            (conv, iters_used, res_u, res_p, cont)
        };

    // 6. Physics-responsive aerodynamics & hydraulics (Cd, Cl, Cm, Δp)
    let cf_ref = 0.0031;
    let cf_actual = if re < 5.0e5 {
        0.664 / re.sqrt()
    } else {
        (2.0 * re.log10() - 0.65).powf(-2.3)
    };
    let cf_scale = (cf_actual / cf_ref).clamp(0.55, 2.8);

    // Numerical diffusion / wall-layer error penalty: refining y+ and hotspots eliminates artificial drag
    let in_buffer_layer = case.physics.turbulence != TurbulenceModel::Laminar
        && actual_y_plus > 5.0
        && actual_y_plus < 30.0;
    let yplus_err_factor = if in_buffer_layer {
        0.09
    } else if actual_y_plus > 150.0 {
        0.07
    } else {
        0.0
    };
    let hotspot_err_factor = 0.08 * (1.0 - hotspot_coverage_frac);
    let mesh_fidelity_mult = 1.0 + yplus_err_factor + hotspot_err_factor;

    let (cd, cl, cm, delta_p_pa) = if diverged {
        (None, None, None, None)
    } else {
        compute_surrogate_forces_and_dp(
            case,
            re,
            rho,
            u_ref,
            q_dyn,
            cf_scale,
            mesh_fidelity_mult,
            is_2d,
        )
    };

    notes.push(format!(
        "Surrogate Re={:.2e}, actual wall y+≈{:.2}, hotspots covered={}/{}.",
        re, actual_y_plus, covered_hotspots, hotspots.len()
    ));

    OpenFoamResultSummary {
        backend_used: "surrogate".to_string(),
        converged,
        diverged,
        iterations_completed,
        final_residual_u,
        final_residual_p,
        continuity_error,
        cd,
        cl,
        cm,
        delta_p_pa,
        check_mesh,
        y_plus_patches,
        hotspots,
        notes,
    }
}

/// Deterministic `0.0..=100.0` engineering quality + optimization objective score.
/// Heavily penalizes divergence, failed mesh checks, guardrail breaches, and buffer-layer y+;
/// rewards convergence, wall y+ within target window, hotspot resolution, and lower Cd / Δp or higher Cl/Cd.
pub fn compute_objective_score(
    case: &CfdCase,
    res: &OpenFoamResultSummary,
    goal: &OptimizationGoal,
    guardrails: &GuardrailsConfig,
) -> f64 {
    if res.diverged {
        return 2.0;
    }

    let telemetry = compute_telemetry(case);
    let tol = case.numerics.convergence_tol.max(1e-7);

    // 1. Convergence & Residuals (0..25 pts)
    let mut conv_score = if res.converged { 20.0 } else { 7.0 };
    if res.final_residual_u.is_finite() && res.final_residual_u <= tol * 1.5 {
        conv_score += 2.0;
    }
    if res.final_residual_p.is_finite() && res.final_residual_p <= tol * 1.5 {
        conv_score += 2.0;
    }
    if res.continuity_error.is_finite() && res.continuity_error < 1e-4 {
        conv_score += 1.0;
    }

    // 2. Mesh quality margin from checkMesh (0..20 pts)
    let non_ortho_margin =
        (1.0 - (res.check_mesh.max_non_orthogonality / guardrails.max_non_ortho.max(1.0)).clamp(0.0, 1.0))
            .powf(0.65);
    let skew_margin =
        (1.0 - (res.check_mesh.max_skewness / guardrails.max_skewness.max(0.5)).clamp(0.0, 1.0))
            .powf(0.65);
    let mesh_score = 10.0 * non_ortho_margin + 10.0 * skew_margin;

    // 3. Wall y+ accuracy & window compliance (0..20 pts)
    let avg_y_plus = representative_wall_y_plus(case, res);
    let target_yp = goal
        .target_y_plus
        .unwrap_or_else(|| 0.5 * (guardrails.target_y_plus_min + guardrails.target_y_plus_max))
        .max(0.1);

    let yplus_score = if case.physics.turbulence == TurbulenceModel::Laminar {
        18.0
    } else if let Some(yp) = avg_y_plus {
        let in_window = yp >= guardrails.target_y_plus_min && yp <= guardrails.target_y_plus_max;
        let log_dist = (yp.max(0.01) / target_yp).ln().abs();
        if in_window {
            14.0 + 6.0 * (-1.4 * log_dist).exp()
        } else {
            12.0 * (-1.15 * log_dist).exp()
        }
    } else {
        10.0
    };

    // 4. Gradient hotspot resolution (0..15 pts)
    let hotspot_score = if res.hotspots.is_empty() {
        15.0
    } else {
        let avg_sev: f64 =
            res.hotspots.iter().map(|h| h.gradient_score.max(0.0)).sum::<f64>()
                / (res.hotspots.len() as f64);
        15.0 / (1.0 + (avg_sev / 3.5).powi(2))
    };

    // 5. Aerodynamic / Hydraulic optimization objective (0..20 pts)
    let cd_ref = reference_cd_for_case(case);
    let perf_score = if goal.maximize_cl_over_cd {
        match (res.cl, res.cd) {
            (Some(cl), Some(cd)) if cd > 1e-5 => {
                let ld = cl / cd;
                20.0 * (1.0 - (-ld.max(0.0) / 32.0).exp())
            }
            _ => 8.0,
        }
    } else if goal.minimize_cd {
        let cd_term = match res.cd {
            Some(cd) if cd.is_finite() => 20.0 / (1.0 + (cd.max(0.0) / cd_ref)),
            _ => 10.0,
        };
        if let (Some(max_dp), Some(dp)) = (goal.max_delta_p_pa, res.delta_p_pa) {
            if max_dp > 0.0 && dp <= max_dp {
                let dp_term = 20.0 * (1.0 - 0.55 * (dp.max(0.0) / max_dp));
                0.55 * cd_term + 0.45 * dp_term
            } else {
                cd_term * 0.6
            }
        } else {
            cd_term
        }
    } else if let (Some(max_dp), Some(dp)) = (goal.max_delta_p_pa, res.delta_p_pa) {
        if max_dp > 0.0 {
            20.0 / (1.0 + (dp.max(0.0) / max_dp).powf(1.2))
        } else {
            10.0
        }
    } else if let Some(cd) = res.cd {
        20.0 / (1.0 + (cd.max(0.0) / cd_ref))
    } else {
        12.0
    };

    let mut score = conv_score + mesh_score + yplus_score + hotspot_score + perf_score;

    // --- Hard Guardrail & Engineering Violations ---
    if !res.check_mesh.mesh_ok || res.check_mesh.failed_checks > 0 {
        score -= 28.0 + 5.0 * (res.check_mesh.failed_checks as f64);
    }
    if res.check_mesh.max_skewness > guardrails.max_skewness {
        score -= 16.0 + 4.0 * (res.check_mesh.max_skewness - guardrails.max_skewness);
    }
    if res.check_mesh.max_non_orthogonality > guardrails.max_non_ortho {
        score -= 16.0 + 0.5 * (res.check_mesh.max_non_orthogonality - guardrails.max_non_ortho);
    }
    if res.check_mesh.cells > guardrails.max_cells && guardrails.max_cells > 0 {
        let excess = (res.check_mesh.cells as f64 / guardrails.max_cells as f64) - 1.0;
        score -= 14.0 + 16.0 * excess.min(2.0);
    }
    if telemetry.estimated_ram_mb > guardrails.max_ram_mb && guardrails.max_ram_mb > 0.0 {
        score -= 14.0;
    }
    if case
        .mesh
        .refinement_boxes
        .iter()
        .any(|b| b.level > guardrails.max_refinement_level)
    {
        score -= 12.0;
    }
    if case.physics.turbulence != TurbulenceModel::Laminar {
        if let Some(yp) = avg_y_plus {
            if yp > 5.0 && yp < 30.0 {
                // Penalize buffer layer unless user explicitly targeted buffer layer
                let target_in_buffer = target_yp > 5.0 && target_yp < 30.0;
                if !target_in_buffer {
                    score -= 12.0;
                }
            }
        }
    }
    if let (Some(max_dp), Some(dp)) = (goal.max_delta_p_pa, res.delta_p_pa) {
        if max_dp > 0.0 && dp > max_dp {
            let excess = (dp / max_dp) - 1.0;
            score -= 8.0 + 14.0 * excess.min(2.0);
        }
    }

    score.clamp(0.0, 100.0)
}

// ---------------------------------------------------------------------------
// Internal OpenFOAM Output Parsers
// ---------------------------------------------------------------------------

struct SolverLogStats {
    converged: bool,
    diverged: bool,
    iterations_completed: u32,
    final_residual_u: f64,
    final_residual_p: f64,
    continuity_error: f64,
}

fn parse_check_mesh_log(text: &str, fallback_cells: u64) -> CheckMeshSummary {
    let cells_re = Regex::new(r"(?m)(?:Number of cells:|cells:)\s+(\d+)").unwrap();
    let non_ortho_re = Regex::new(
        r"Mesh non-orthogonality Max:\s*([0-9eE.+\-]+)\s+average:\s*([0-9eE.+\-]+)",
    )
    .unwrap();
    let skew_re = Regex::new(r"Max skewness\s*=\s*([0-9eE.+\-]+)").unwrap();
    let ar_re = Regex::new(r"Max aspect ratio\s*=\s*([0-9eE.+\-]+)").unwrap();
    let failed_re = Regex::new(r"Failed\s+(\d+)\s+mesh checks?").unwrap();

    let cells = cells_re
        .captures(text)
        .and_then(|c| c[1].parse::<u64>().ok())
        .unwrap_or(fallback_cells);

    let (max_non_orthogonality, avg_non_orthogonality) = non_ortho_re
        .captures(text)
        .map(|c| {
            (
                c[1].parse::<f64>().unwrap_or(0.0),
                c[2].parse::<f64>().unwrap_or(0.0),
            )
        })
        .unwrap_or((0.0, 0.0));

    let max_skewness = skew_re
        .captures(text)
        .and_then(|c| c[1].parse::<f64>().ok())
        .unwrap_or(0.0);

    let max_aspect_ratio = ar_re
        .captures(text)
        .and_then(|c| c[1].parse::<f64>().ok())
        .unwrap_or(1.0);

    let failed_checks = failed_re
        .captures(text)
        .and_then(|c| c[1].parse::<u32>().ok())
        .unwrap_or_else(|| {
            if text.contains("Mesh OK.") {
                0
            } else {
                text.lines().filter(|l| l.trim_start().starts_with("***")).count() as u32
            }
        });

    let mesh_ok = text.contains("Mesh OK.") && failed_checks == 0;

    CheckMeshSummary {
        cells,
        max_non_orthogonality,
        avg_non_orthogonality,
        max_skewness,
        max_aspect_ratio,
        failed_checks,
        mesh_ok,
    }
}

fn parse_solver_log(text: &str, convergence_tol: f64) -> SolverLogStats {
    let time_re = Regex::new(r"(?m)^Time\s*=\s*([0-9eE.+\-]+)").unwrap();
    let res_re = Regex::new(
        r"Solving for ([A-Za-z0-9_]+),\s*Initial residual\s*=\s*([A-Za-z0-9eE.+\-]+)",
    )
    .unwrap();
    let cont_re = Regex::new(r"cumulative\s*=\s*([A-Za-z0-9eE.+\-]+)").unwrap();

    let nan_re = Regex::new(r"(?i)\b-?nan\b").unwrap();

    let mut diverged = text.contains("FOAM FATAL ERROR")
        || text.contains("FOAM FATAL IO ERROR")
        || text.contains("Floating point exception")
        || text.contains("SIGFPE")
        || nan_re.is_match(text);

    let iterations_completed = time_re.captures_iter(text).count() as u32;

    let mut latest_ux: Option<f64> = None;
    let mut latest_u_any: Option<f64> = None;
    let mut latest_p: Option<f64> = None;

    for cap in res_re.captures_iter(text) {
        let field = &cap[1];
        let raw = &cap[2];
        let lower = raw.to_ascii_lowercase();
        if lower.contains("nan") || lower.contains("inf") {
            diverged = true;
            continue;
        }
        if let Ok(val) = raw.parse::<f64>() {
            if !val.is_finite() || val.abs() > 1e3 {
                diverged = true;
            }
            match field {
                "Ux" => {
                    latest_ux = Some(val);
                    latest_u_any = Some(val);
                }
                "Uy" | "Uz" => {
                    latest_u_any = Some(val);
                }
                "p" | "p_rgh" => {
                    latest_p = Some(val);
                }
                _ => {}
            }
        }
    }

    let mut continuity_error = 0.0_f64;
    for cap in cont_re.captures_iter(text) {
        let raw = &cap[1];
        let lower = raw.to_ascii_lowercase();
        if lower.contains("nan") || lower.contains("inf") {
            diverged = true;
            continue;
        }
        if let Ok(val) = raw.parse::<f64>() {
            if !val.is_finite() {
                diverged = true;
            } else {
                continuity_error = val.abs();
            }
        }
    }

    let final_residual_u = latest_ux.or(latest_u_any).unwrap_or(if diverged { 1e3 } else { 1.0 });
    let final_residual_p = latest_p.unwrap_or(if diverged { 1e3 } else { 1.0 });

    let explicit_converged = text.contains("SIMPLE solution converged")
        || text.contains("solution converged in");
    let tol = convergence_tol.max(1e-9);
    let residual_converged = iterations_completed > 0
        && latest_ux.or(latest_u_any).is_some()
        && latest_p.is_some()
        && final_residual_u <= tol * 1.5
        && final_residual_p <= tol * 1.5;

    let converged = !diverged && (explicit_converged || residual_converged);

    SolverLogStats {
        converged,
        diverged,
        iterations_completed,
        final_residual_u,
        final_residual_p,
        continuity_error,
    }
}

fn find_latest_postprocessing_file(
    case_dir: &Path,
    fn_obj_names: &[&str],
    file_names: &[&str],
) -> Option<PathBuf> {
    let pp_root = case_dir.join("postProcessing");
    for obj in fn_obj_names {
        let obj_dir = pp_root.join(obj);
        if !obj_dir.is_dir() {
            continue;
        }

        // Collect candidate time subdirectories containing a target file
        let mut best: Option<(f64, String, PathBuf)> = None;
        if let Ok(entries) = std::fs::read_dir(&obj_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let dirname = entry.file_name().to_string_lossy().to_string();
                    let time_val = dirname.parse::<f64>().unwrap_or(f64::NEG_INFINITY);
                    for fname in file_names {
                        let candidate = p.join(fname);
                        if candidate.is_file() {
                            let replace = match &best {
                                None => true,
                                Some((bt, bd, _)) => {
                                    if time_val.is_finite() && bt.is_finite() {
                                        time_val > *bt
                                    } else {
                                        dirname > *bd
                                    }
                                }
                            };
                            if replace {
                                best = Some((time_val, dirname.clone(), candidate));
                            }
                            break;
                        }
                    }
                }
            }
        }
        if let Some((_, _, path)) = best {
            return Some(path);
        }

        // Direct fallback inside postProcessing/<obj>/<file>
        for fname in file_names {
            let direct = obj_dir.join(fname);
            if direct.is_file() {
                return Some(direct);
            }
        }
    }
    None
}

fn parse_y_plus_dat(text: &str) -> Vec<PatchYPlus> {
    let mut rows: Vec<(f64, PatchYPlus)> = Vec::new();
    let mut max_time = f64::NEG_INFINITY;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() >= 5 {
            if let (Ok(t), Ok(min), Ok(max), Ok(avg)) = (
                cols[0].parse::<f64>(),
                cols[2].parse::<f64>(),
                cols[3].parse::<f64>(),
                cols[4].parse::<f64>(),
            ) {
                if t > max_time {
                    max_time = t;
                }
                rows.push((
                    t,
                    PatchYPlus {
                        patch: cols[1].to_string(),
                        min,
                        max,
                        average: avg,
                    },
                ));
            }
        }
    }

    if rows.is_empty() {
        return Vec::new();
    }

    let mut by_patch: BTreeMap<String, PatchYPlus> = BTreeMap::new();
    for (t, entry) in rows {
        if (t - max_time).abs() <= 1e-6 * max_time.abs().max(1.0) {
            by_patch.insert(entry.patch.clone(), entry);
        }
    }
    by_patch.into_values().collect()
}

fn parse_force_coeffs_dat(text: &str) -> (Option<f64>, Option<f64>, Option<f64>) {
    let mut idx_cd: Option<usize> = None;
    let mut idx_cl: Option<usize> = None;
    let mut idx_cm: Option<usize> = None;
    let mut last_data_cols: Option<Vec<f64>> = None;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('#') {
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.iter().any(|t| *t == "Cd" || *t == "Cl" || *t == "Cm" || *t == "CmPitch") {
                for (i, tok) in tokens.iter().enumerate() {
                    match *tok {
                        "Cd" => idx_cd = Some(i),
                        "Cl" => idx_cl = Some(i),
                        "Cm" | "CmPitch" => idx_cm = Some(i),
                        _ => {}
                    }
                }
            }
            continue;
        }
        let nums: Vec<f64> = line
            .split_whitespace()
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();
        if nums.len() >= 4 {
            last_data_cols = Some(nums);
        }
    }

    let Some(cols) = last_data_cols else {
        return (None, None, None);
    };

    // Standard OpenFOAM forceCoeffs fallback layout: Time(0), Cm(1), Cd(2), Cl(3)
    let cd = idx_cd.and_then(|i| cols.get(i).copied()).or_else(|| cols.get(2).copied());
    let cl = idx_cl.and_then(|i| cols.get(i).copied()).or_else(|| cols.get(3).copied());
    let cm = idx_cm.and_then(|i| cols.get(i).copied()).or_else(|| cols.get(1).copied());
    (cd, cl, cm)
}

fn parse_surface_field_value_dat(text: &str) -> Option<f64> {
    let mut last_val = None;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cleaned = line.replace(['(', ')'], " ");
        let nums: Vec<f64> = cleaned
            .split_whitespace()
            .filter_map(|s| s.parse::<f64>().ok())
            .collect();
        if nums.len() >= 2 {
            last_val = Some(nums[1]);
        }
    }
    last_val
}

fn extract_probe_hotspots(
    u_text: Option<&str>,
    p_text: Option<&str>,
    case: &CfdCase,
) -> Vec<GradientHotspot> {
    let coords = u_text
        .map(parse_probe_coords)
        .filter(|v| !v.is_empty())
        .or_else(|| p_text.map(parse_probe_coords))
        .unwrap_or_default();

    if coords.len() < 2 {
        return Vec::new();
    }

    let u_vals = u_text.map(parse_latest_probe_vectors).unwrap_or_default();
    let p_vals = p_text.map(parse_latest_probe_scalars).unwrap_or_default();

    let n = coords.len();
    let u_ref = case.physics.reference_velocity.abs().max(1e-3);
    let l_ref = case.physics.reference_length.abs().max(1e-4);
    let grad_u_norm = (u_ref / l_ref).max(1e-6);
    let grad_p_norm = (0.5 * u_ref * u_ref / l_ref).max(1e-6);

    // Find characteristic nearest-neighbor spacing across probes
    let mut min_dists = Vec::with_capacity(n);
    for i in 0..n {
        let mut best = f64::INFINITY;
        for j in 0..n {
            if i == j {
                continue;
            }
            let d = dist3(coords[i], coords[j]);
            if d > 1e-6 && d < best {
                best = d;
            }
        }
        if best.is_finite() {
            min_dists.push(best);
        }
    }
    if min_dists.is_empty() {
        return Vec::new();
    }
    min_dists.sort_by(|a, b| a.total_cmp(b));
    let d_char = min_dists[min_dists.len() / 2];
    let neighbor_cutoff = d_char * 1.85;

    struct Candidate {
        center: [f64; 3],
        score: f64,
        grad_u_raw: f64,
        grad_p_raw: f64,
        spacing: f64,
    }

    let mut candidates = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let d = dist3(coords[i], coords[j]);
            if d < 1e-6 || d > neighbor_cutoff {
                continue;
            }
            let du_ds = if i < u_vals.len() && j < u_vals.len() {
                dist3(u_vals[i], u_vals[j]) / d
            } else {
                0.0
            };
            let dp_ds = if i < p_vals.len() && j < p_vals.len() {
                (p_vals[i] - p_vals[j]).abs() / d
            } else {
                0.0
            };
            let score = ((du_ds / grad_u_norm) * 2.2 + (dp_ds / grad_p_norm) * 1.6).clamp(0.0, 20.0);
            if score > 0.25 {
                let center = [
                    0.5 * (coords[i][0] + coords[j][0]),
                    0.5 * (coords[i][1] + coords[j][1]),
                    0.5 * (coords[i][2] + coords[j][2]),
                ];
                candidates.push(Candidate {
                    center,
                    score,
                    grad_u_raw: du_ds,
                    grad_p_raw: dp_ds,
                    spacing: d,
                });
            }
        }
    }

    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut hotspots = Vec::new();
    for cand in candidates {
        if hotspots.len() >= 3 {
            break;
        }
        let too_close = hotspots
            .iter()
            .any(|h: &GradientHotspot| dist3(h.center, cand.center) < 0.65 * d_char);
        if too_close {
            continue;
        }

        let hx = (0.45 * cand.spacing).max(0.12 * l_ref);
        let hy = (0.45 * cand.spacing).max(0.10 * l_ref);
        let hz = (0.40 * case.domain.lz()).max(0.05 * l_ref);
        let (bmin, bmax) = clamp_box_to_domain(
            [cand.center[0] - hx, cand.center[1] - hy, cand.center[2] - hz],
            [cand.center[0] + hx, cand.center[1] + hy, cand.center[2] + hz],
            &case.domain,
        );

        let suggested_level = if cand.score > 6.0 {
            4
        } else if cand.score > 2.8 {
            3
        } else {
            2
        }
        .min(case.mesh.refinement_max.saturating_add(1).max(2));

        let idx = hotspots.len() + 1;
        hotspots.push(GradientHotspot {
            id: format!("probe_hotspot_{}", idx),
            center: cand.center,
            suggested_box_min: bmin,
            suggested_box_max: bmax,
            gradient_score: cand.score,
            suggested_level,
            reason: format!(
                "High spatial flow gradient detected between probes (|∇U|≈{:.1} s⁻¹, |∇p/ρ|≈{:.1} m/s²). Add L{} refinement box to resolve shear/pressure core.",
                cand.grad_u_raw, cand.grad_p_raw, suggested_level
            ),
        });
    }

    hotspots
}

fn parse_probe_coords(text: &str) -> Vec<[f64; 3]> {
    let re = Regex::new(
        r"#\s*Probe\s+\d+\s*\(\s*([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s*\)",
    )
    .unwrap();
    let mut coords = Vec::new();
    for cap in re.captures_iter(text) {
        if let (Ok(x), Ok(y), Ok(z)) = (
            cap[1].parse::<f64>(),
            cap[2].parse::<f64>(),
            cap[3].parse::<f64>(),
        ) {
            coords.push([x, y, z]);
        }
    }
    coords
}

fn parse_latest_probe_vectors(text: &str) -> Vec<[f64; 3]> {
    let vec_re = Regex::new(
        r"\(\s*([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s*\)",
    )
    .unwrap();
    let Some(last_line) = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
    else {
        return Vec::new();
    };
    vec_re
        .captures_iter(last_line)
        .filter_map(|c| {
            Some([
                c[1].parse::<f64>().ok()?,
                c[2].parse::<f64>().ok()?,
                c[3].parse::<f64>().ok()?,
            ])
        })
        .collect()
}

fn parse_latest_probe_scalars(text: &str) -> Vec<f64> {
    let Some(last_line) = text
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
    else {
        return Vec::new();
    };
    let nums: Vec<f64> = last_line
        .split_whitespace()
        .filter_map(|s| s.parse::<f64>().ok())
        .collect();
    if nums.len() > 1 {
        nums[1..].to_vec()
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Internal Surrogate Physics Helpers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn compute_surrogate_forces_and_dp(
    case: &CfdCase,
    re: f64,
    rho: f64,
    u_ref: f64,
    q_dyn: f64,
    cf_scale: f64,
    mesh_fidelity_mult: f64,
    is_2d: bool,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    if let Some(spec) = &case.parametric {
        return match spec {
            ParametricGeometrySpec::NacaAirfoil {
                chord,
                camber_pct,
                camber_pos_pct,
                thickness_pct,
                aoa_deg,
                span,
            } => {
                let alpha0_deg =
                    -1.15 * camber_pct * (0.4 + 0.6 * (camber_pos_pct / 40.0).clamp(0.5, 1.5));
                let alpha_eff = (aoa_deg - alpha0_deg).to_radians();
                let stall_deg = 13.0 + 0.28 * (thickness_pct - 12.0).clamp(-5.0, 8.0);
                let quasi_2d = is_2d || (span.abs() / case.domain.lz().max(1e-4)) >= 0.70;
                let finite_wing = if quasi_2d {
                    1.0
                } else {
                    let ar = (span / chord.max(1e-4)).max(0.5);
                    ar / (ar + 2.0)
                };

                let cl_linear =
                    2.0 * PI * alpha_eff.sin() * finite_wing * (1.0 - 0.008 * (thickness_pct - 12.0).abs());
                let excess_stall = (aoa_deg.abs() - stall_deg).max(0.0);
                let stall_blend = (-0.22 * excess_stall).exp();
                let cl = cl_linear * (0.35 + 0.65 * stall_blend);

                let cd0 = (0.0058 + 0.00028 * thickness_pct + 0.00016 * camber_pct.abs())
                    * cf_scale.clamp(0.6, 2.4);
                let k_induced = if quasi_2d {
                    0.011
                } else {
                    let ar = (span / chord.max(1e-4)).max(0.5);
                    1.0 / (PI * 0.85 * ar)
                };
                let cd_stall = 0.018 * excess_stall.powf(1.55);
                let cd = (cd0 + k_induced * cl * cl + cd_stall) * mesh_fidelity_mult;
                let cm = -0.024 * camber_pct - 0.003 * aoa_deg;
                let blockage = (chord * (thickness_pct / 100.0) / case.domain.ly().max(1e-4))
                    .clamp(0.015, 0.45);
                let dp = q_dyn * cd * blockage;
                (Some(cd), Some(cl), Some(cm), Some(dp))
            }
            ParametricGeometrySpec::VenturiTube {
                inlet_radius,
                throat_radius,
                outlet_radius,
                convergent_len,
                throat_len,
                divergent_len,
            } => {
                let r_in = inlet_radius.abs().max(1e-4);
                let r_th = throat_radius.abs().max(1e-4);
                let r_out = outlet_radius.abs().max(1e-4);
                let beta = (r_th / r_in).clamp(0.15, 0.99);
                let area_ratio = beta * beta;

                let theta_div_deg = ((r_out - r_th).abs() / divergent_len.max(1e-4))
                    .atan()
                    .to_degrees();
                // Gibson/Idelchik conical diffuser loss: minimum near 4–5 deg, separation jump above 6 deg
                let k_div = 0.06
                    + 0.014 * (theta_div_deg - 4.2).abs()
                    + if theta_div_deg > 6.0 {
                        0.032 * (theta_div_deg - 6.0).powf(1.55)
                    } else {
                        0.0
                    };
                let k_conv = 0.045
                    * (1.0 - area_ratio).powi(2)
                    * (1.0 + 0.4 / (convergent_len / r_in).max(0.25));

                let f_darcy = if re < 2300.0 {
                    64.0 / re.max(10.0)
                } else {
                    0.3164 / re.powf(0.25)
                };
                let k_fric = f_darcy
                    * (convergent_len / (2.0 * r_in)
                        + (throat_len / (2.0 * r_th)) / area_ratio
                        + divergent_len / (2.0 * r_out));

                let expansion_factor = (1.0 / area_ratio - 1.0).powi(2).max(0.20);
                let k_total = (k_conv + k_fric + k_div * expansion_factor) * mesh_fidelity_mult;
                let dp = q_dyn * k_total;
                let cd = (k_total * 0.08).clamp(0.008, 4.0);
                (Some(cd), Some(0.0), Some(0.0), Some(dp))
            }
            ParametricGeometrySpec::AhmedDiffuser {
                length,
                width,
                height,
                rear_slant_deg,
                ground_clearance,
            } => {
                // ERCOFTAC Ahmed et al. (1984) drag curve vs rear slant angle:
                // minimum near 12.5 deg (~0.230), C-pillar vortex jump near 28-30 deg (~0.375),
                // breakdown above 30 deg (~0.265).
                let theta = rear_slant_deg.clamp(0.0, 60.0);
                let cd_slant = if theta <= 12.5 {
                    0.230 + 0.028 * ((12.5 - theta) / 12.5).powi(2)
                } else if theta <= 30.0 {
                    0.230 + 0.148 * ((theta - 12.5) / 17.5).powf(1.85)
                } else if theta <= 35.0 {
                    // Sudden vortex breakdown to separated wake
                    let u = (theta - 30.0) / 5.0;
                    0.378 - 0.112 * u
                } else {
                    0.266 + 0.018 * ((theta - 35.0) / 25.0)
                };

                let clr_ratio = (ground_clearance / height.max(1e-4)).clamp(0.01, 1.0);
                let clr_penalty = 0.016 * (clr_ratio - 0.17).abs();
                let fineness = (length / height.max(1e-4)).clamp(1.5, 8.0);
                let fineness_adj = 1.0 - 0.015 * (fineness - 3.6).clamp(-1.5, 2.5);

                let cd = (cd_slant + clr_penalty)
                    * fineness_adj
                    * (0.82 + 0.18 * cf_scale.clamp(0.6, 2.0))
                    * mesh_fidelity_mult;
                let cl = -0.055 + 0.012 * (theta - 12.0).clamp(-6.0, 18.0);
                let cm = -0.015 - 0.0018 * theta;
                let frontal_ratio = ((width * height)
                    / (case.domain.ly() * case.domain.lz()).max(1e-4))
                .clamp(0.03, 0.55);
                let dp = q_dyn * cd * frontal_ratio;
                (Some(cd), Some(cl), Some(cm), Some(dp))
            }
            ParametricGeometrySpec::PinFinArray {
                base_width,
                fin_count_x,
                fin_count_z,
                fin_radius,
                fin_height,
                ..
            } => {
                let nx = (*fin_count_x).max(1) as f64;
                let nz = (*fin_count_z).max(1) as f64;
                let sigma = (nz * 2.0 * fin_radius / base_width.max(1e-4)).clamp(0.05, 0.90);
                let u_max = u_ref / (1.0 - sigma).max(0.10);
                let h_ratio = (fin_height / case.domain.ly().max(1e-4)).clamp(0.08, 1.0);
                let dp = 0.5
                    * rho
                    * u_max
                    * u_max
                    * nx
                    * (0.24 + 1.35 * sigma.powf(1.5))
                    * h_ratio
                    * cf_scale.clamp(0.65, 2.2)
                    * mesh_fidelity_mult;
                let cd = (dp / q_dyn.max(1e-6)).clamp(0.05, 15.0);
                (Some(cd), Some(0.0), Some(0.0), Some(dp))
            }
            ParametricGeometrySpec::Cylinder { radius, .. } => {
                let cd_base = cylinder_drag_vs_re(re);
                let cd = cd_base * mesh_fidelity_mult;
                let blockage = (2.0 * radius / case.domain.ly().max(1e-4)).clamp(0.04, 0.65);
                let dp = q_dyn * cd * blockage;
                (Some(cd), Some(0.015), Some(0.0), Some(dp))
            }
        };
    }

    // Fallback when `case.parametric` is None (uses PresetGeometryKind)
    match case.geometry_kind {
        PresetGeometryKind::AirfoilNaca0012 => {
            let cd = 0.0125 * cf_scale.clamp(0.6, 2.2) * mesh_fidelity_mult;
            let cl = 0.02;
            (Some(cd), Some(cl), Some(0.0), Some(q_dyn * cd * 0.06))
        }
        PresetGeometryKind::AhmedBody => {
            let cd = 0.295 * mesh_fidelity_mult;
            (Some(cd), Some(-0.04), Some(-0.05), Some(q_dyn * cd * 0.09))
        }
        PresetGeometryKind::VenturiNozzle => {
            let dp = q_dyn * 0.42 * cf_scale.clamp(0.7, 2.0) * mesh_fidelity_mult;
            (Some(0.048 * mesh_fidelity_mult), Some(0.0), Some(0.0), Some(dp))
        }
        PresetGeometryKind::CylinderCrossflow => {
            let cd = cylinder_drag_vs_re(re) * mesh_fidelity_mult;
            (Some(cd), Some(0.015), Some(0.0), Some(q_dyn * cd * 0.12))
        }
        _ => {
            let f_darcy = if re < 2300.0 {
                64.0 / re.max(10.0)
            } else {
                0.3164 / re.powf(0.25)
            };
            let dp = q_dyn * f_darcy * (case.domain.lx() / case.domain.ly().max(1e-4)).clamp(0.5, 20.0);
            let cd = (dp / q_dyn.max(1e-6) * 0.15).clamp(0.01, 2.0) * mesh_fidelity_mult;
            (Some(cd), Some(0.0), Some(0.0), Some(dp))
        }
    }
}

fn cylinder_drag_vs_re(re: f64) -> f64 {
    if re < 200.0 {
        1.15 + 10.0 / re.max(5.0).sqrt()
    } else if re < 2.0e5 {
        1.16
    } else if re <= 6.0e5 {
        // Drag crisis transition
        let u = (re - 2.0e5) / 4.0e5;
        1.16 - 0.78 * u
    } else {
        0.38 + 0.18 * ((re / 6.0e5).ln().clamp(0.0, 2.0) / 2.0)
    }
}

fn synthesize_surrogate_hotspots(case: &CfdCase, l_ref: f64) -> Vec<GradientHotspot> {
    let d = &case.domain;
    let z_mid = 0.5 * (d.z_min + d.z_max);
    let hz = (0.45 * d.lz()).max(0.05 * l_ref);

    // Raw candidate descriptors: (id, center, half_extents, base_score, suggested_level, reason)
    let raw_specs: Vec<(&str, [f64; 3], [f64; 3], f64, u32, String)> = match &case.parametric {
        Some(ParametricGeometrySpec::NacaAirfoil { chord, aoa_deg, .. }) => {
            let c = chord.abs().max(0.05);
            let aoa_factor = 1.0 + (aoa_deg.abs() / 12.0).min(1.2);
            vec![
                (
                    "naca_trailing_edge_wake",
                    [1.35 * c, -0.05 * c * aoa_deg.to_radians().sin(), z_mid],
                    [0.95 * c, 0.35 * c, hz],
                    7.4 * aoa_factor.min(1.5),
                    3,
                    "Trailing-edge shear layer and wake vortex deficit require streamwise refinement.".into(),
                ),
                (
                    "naca_leading_edge_suction",
                    [-0.22 * c, 0.08 * c, z_mid],
                    [0.28 * c, 0.25 * c, hz],
                    6.8 * aoa_factor.min(1.6),
                    4,
                    "Leading-edge stagnation and suction peak exhibit steep pressure & velocity gradients.".into(),
                ),
                (
                    "naca_suction_boundary_layer",
                    [0.25 * c, 0.16 * c, z_mid],
                    [0.45 * c, 0.20 * c, hz],
                    5.2 * aoa_factor.min(1.4),
                    3,
                    "Adverse pressure gradient along upper surface risks separation if under-resolved.".into(),
                ),
            ]
        }
        Some(ParametricGeometrySpec::VenturiTube {
            inlet_radius,
            throat_radius,
            outlet_radius,
            convergent_len,
            throat_len,
            divergent_len,
        }) => {
            let total = convergent_len + throat_len + divergent_len;
            let x_start = -0.5 * total;
            let x_throat = x_start + convergent_len + 0.5 * throat_len;
            let x_diff = x_start + convergent_len + throat_len + 0.42 * divergent_len;
            let theta_deg = ((outlet_radius - throat_radius).abs() / divergent_len.max(1e-4))
                .atan()
                .to_degrees();
            let sep_boost = if theta_deg > 7.0 { 1.35 } else { 1.0 };
            let r_max = inlet_radius.max(*outlet_radius).max(*throat_radius);
            vec![
                (
                    "venturi_throat_core",
                    [x_throat, 0.0, 0.0],
                    [0.65 * throat_len.max(0.2 * total), 1.15 * throat_radius, 1.15 * throat_radius],
                    8.2,
                    3,
                    "Peak Venturi throat acceleration and minimum static pressure core.".into(),
                ),
                (
                    "venturi_diffuser_separation",
                    [x_diff, 0.0, 0.0],
                    [0.38 * divergent_len, 1.05 * r_max, 1.05 * r_max],
                    6.9 * sep_boost,
                    3,
                    format!(
                        "Adverse pressure gradient in {:.1}° divergent diffuser risks wall separation.",
                        theta_deg
                    ),
                ),
            ]
        }
        Some(ParametricGeometrySpec::AhmedDiffuser {
            length,
            width,
            height,
            rear_slant_deg,
            ground_clearance,
        }) => {
            let vortex_boost = if *rear_slant_deg > 20.0 && *rear_slant_deg <= 32.0 {
                1.35
            } else {
                1.0
            };
            vec![
                (
                    "ahmed_slant_c_pillar_vortex",
                    [0.92 * length, ground_clearance + 0.82 * height, 0.0],
                    [0.24 * length, 0.42 * height, 0.72 * width],
                    7.8 * vortex_boost,
                    3,
                    format!(
                        "Longitudinal C-pillar vortex roll-up over {:.1}° rear slant drives pressure drag.",
                        rear_slant_deg
                    ),
                ),
                (
                    "ahmed_near_wake_torus",
                    [1.45 * length, ground_clearance + 0.50 * height, 0.0],
                    [0.52 * length, 0.65 * height, 0.78 * width],
                    7.1,
                    3,
                    "Counter-rotating near-wake recirculation bubble downstream of vertical base.".into(),
                ),
                (
                    "ahmed_underbody_shear",
                    [0.50 * length, (0.5 * ground_clearance).max(0.02), 0.0],
                    [0.48 * length, (0.8 * ground_clearance).max(0.04), 0.65 * width],
                    5.0,
                    2,
                    "High velocity shear between moving road ground and underbody floor.".into(),
                ),
            ]
        }
        Some(ParametricGeometrySpec::PinFinArray {
            base_length,
            base_width,
            fin_height,
            ..
        }) => {
            let base_t = (0.18 * fin_height).min(0.05).max(1e-4);
            let y_mid = base_t + 0.5 * fin_height;
            vec![
                (
                    "pinfin_interfin_shear",
                    [0.0, y_mid, 0.0],
                    [0.55 * base_length, 0.62 * fin_height, 0.55 * base_width],
                    8.1,
                    3,
                    "Inter-fin jet acceleration and horseshoe vortex junctions dominate array Δp.".into(),
                ),
                (
                    "pinfin_downstream_wake",
                    [0.95 * base_length, y_mid, 0.0],
                    [0.48 * base_length, 0.68 * fin_height, 0.58 * base_width],
                    6.4,
                    2,
                    "Trailing pin-fin wake mixing and pressure recovery zone.".into(),
                ),
            ]
        }
        Some(ParametricGeometrySpec::Cylinder { radius, .. }) => {
            let diam = (2.0 * radius).max(0.05);
            vec![
                (
                    "cylinder_karman_wake",
                    [1.6 * diam, 0.0, z_mid],
                    [1.3 * diam, 0.95 * diam, hz],
                    8.0,
                    3,
                    "Near-wake von Kármán vortex formation zone with intense velocity shear.".into(),
                ),
                (
                    "cylinder_separation_shoulders",
                    [0.10 * diam, 0.0, z_mid],
                    [0.72 * diam, 0.85 * diam, hz],
                    7.2,
                    3,
                    "Boundary-layer separation shoulders around θ ≈ ±80°–110° require high resolution.".into(),
                ),
            ]
        }
        None => {
            // Fallback based on PresetGeometryKind or domain geometry
            let (cx, cy) = match &case.stl {
                Some(stl) => (
                    0.5 * (stl.bounds_min[0] + stl.bounds_max[0]),
                    0.5 * (stl.bounds_min[1] + stl.bounds_max[1]),
                ),
                None => (d.x_min + 0.35 * d.lx(), 0.5 * (d.y_min + d.y_max)),
            };
            let wake_x = (cx + 1.2 * l_ref).min(d.x_max - 0.12 * d.lx());
            vec![
                (
                    "near_wake_shear_core",
                    [wake_x, cy, z_mid],
                    [0.85 * l_ref, 0.45 * l_ref, hz],
                    7.2,
                    3,
                    "Primary downstream wake shear layer requires localized snappyHexMesh refinement.".into(),
                ),
                (
                    "body_stagnation_zone",
                    [cx, cy, z_mid],
                    [0.55 * l_ref, 0.40 * l_ref, hz],
                    6.1,
                    3,
                    "High pressure and velocity curvature around primary obstacle surface.".into(),
                ),
            ]
        }
    };

    raw_specs
        .into_iter()
        .map(|(id, center, half, base_score, suggested_level, mut reason)| {
            let (bmin, bmax) = clamp_box_to_domain(
                [center[0] - half[0], center[1] - half[1], center[2] - half[2]],
                [center[0] + half[0], center[1] + half[1], center[2] + half[2]],
                d,
            );
            let clamped_center = [
                0.5 * (bmin[0] + bmax[0]),
                0.5 * (bmin[1] + bmax[1]),
                0.5 * (bmin[2] + bmax[2]),
            ];

            let score = if let Some((lvl, box_name)) =
                refinement_box_coverage(clamped_center, bmin, bmax, &case.mesh.refinement_boxes)
            {
                let ratio = (lvl as f64 / suggested_level.max(1) as f64).min(1.35);
                let atten = 1.0 / (1.0 + 3.8 * ratio * ratio);
                reason.push_str(&format!(
                    " [Refined by '{}' at L{}]",
                    box_name, lvl
                ));
                base_score * atten
            } else {
                base_score
            };

            GradientHotspot {
                id: id.to_string(),
                center: clamped_center,
                suggested_box_min: bmin,
                suggested_box_max: bmax,
                gradient_score: score,
                suggested_level,
                reason,
            }
        })
        .collect()
}

fn refinement_box_coverage(
    center: [f64; 3],
    hmin: [f64; 3],
    hmax: [f64; 3],
    boxes: &[RefinementBox],
) -> Option<(u32, String)> {
    let mut best: Option<(u32, String)> = None;
    let h_area_xy = ((hmax[0] - hmin[0]).abs() * (hmax[1] - hmin[1]).abs()).max(1e-12);

    for b in boxes {
        let bmin = [b.min[0].min(b.max[0]), b.min[1].min(b.max[1]), b.min[2].min(b.max[2])];
        let bmax = [b.min[0].max(b.max[0]), b.min[1].max(b.max[1]), b.min[2].max(b.max[2])];

        let contains_center = center[0] >= bmin[0] - 1e-4
            && center[0] <= bmax[0] + 1e-4
            && center[1] >= bmin[1] - 1e-4
            && center[1] <= bmax[1] + 1e-4
            && center[2] >= bmin[2] - 1e-4
            && center[2] <= bmax[2] + 1e-4;

        let ox = (hmax[0].min(bmax[0]) - hmin[0].max(bmin[0])).max(0.0);
        let oy = (hmax[1].min(bmax[1]) - hmin[1].max(bmin[1])).max(0.0);
        let oz = (hmax[2].min(bmax[2]) - hmin[2].max(bmin[2])).max(0.0);
        let xy_overlap = (ox * oy) / h_area_xy;
        let z_touches = oz > 0.0 || (bmin[2] <= center[2] + 1e-4 && bmax[2] >= center[2] - 1e-4);

        if contains_center || (xy_overlap >= 0.30 && z_touches) {
            let replace = match &best {
                None => true,
                Some((lvl, _)) => b.level > *lvl,
            };
            if replace {
                best = Some((b.level, b.name.clone()));
            }
        }
    }

    best
}

fn clamp_box_to_domain(min: [f64; 3], max: [f64; 3], d: &DomainBox) -> ([f64; 3], [f64; 3]) {
    let eps_x = 0.01 * d.lx();
    let eps_y = 0.01 * d.ly();
    let eps_z = 0.01 * d.lz();

    let x0 = min[0].clamp(d.x_min + eps_x, d.x_max - 2.0 * eps_x);
    let x1 = max[0].clamp(x0 + eps_x, d.x_max - eps_x);
    let y0 = min[1].clamp(d.y_min + eps_y, d.y_max - 2.0 * eps_y);
    let y1 = max[1].clamp(y0 + eps_y, d.y_max - eps_y);
    let z0 = min[2].clamp(d.z_min + eps_z, d.z_max - 2.0 * eps_z);
    let z1 = max[2].clamp(z0 + eps_z, d.z_max - eps_z);

    ([x0, y0, z0], [x1, y1, z1])
}

fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn representative_wall_y_plus(case: &CfdCase, res: &OpenFoamResultSummary) -> Option<f64> {
    if res.y_plus_patches.is_empty() {
        return None;
    }
    let obstacle_names: Vec<&str> = case
        .boundaries
        .iter()
        .filter(|b| b.face == DomainFace::Obstacle)
        .map(|b| b.name.as_str())
        .collect();

    let obs_vals: Vec<f64> = res
        .y_plus_patches
        .iter()
        .filter(|p| obstacle_names.contains(&p.patch.as_str()))
        .map(|p| p.average)
        .collect();

    if !obs_vals.is_empty() {
        Some(obs_vals.iter().sum::<f64>() / obs_vals.len() as f64)
    } else {
        Some(
            res.y_plus_patches.iter().map(|p| p.average).sum::<f64>()
                / res.y_plus_patches.len() as f64,
        )
    }
}

fn reference_cd_for_case(case: &CfdCase) -> f64 {
    match &case.parametric {
        Some(ParametricGeometrySpec::NacaAirfoil { .. }) => 0.025,
        Some(ParametricGeometrySpec::VenturiTube { .. }) => 0.08,
        Some(ParametricGeometrySpec::AhmedDiffuser { .. }) => 0.30,
        Some(ParametricGeometrySpec::PinFinArray { .. }) => 1.50,
        Some(ParametricGeometrySpec::Cylinder { .. }) => 1.15,
        None => match case.geometry_kind {
            PresetGeometryKind::AirfoilNaca0012 => 0.025,
            PresetGeometryKind::AhmedBody => 0.30,
            PresetGeometryKind::VenturiNozzle => 0.08,
            PresetGeometryKind::CylinderCrossflow => 1.15,
            _ => 0.25,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openfoam::generator::tests::sample_case;

    #[test]
    fn parses_mock_openfoam_logs_and_postprocessing() {
        let case = sample_case();
        let tmp = std::env::temp_dir().join(format!(
            "venturi_results_parse_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("postProcessing/yPlus/500")).unwrap();
        std::fs::create_dir_all(tmp.join("postProcessing/forceCoeffs1/0")).unwrap();
        std::fs::create_dir_all(tmp.join("postProcessing/inletPressure/0")).unwrap();
        std::fs::create_dir_all(tmp.join("postProcessing/outletPressure/0")).unwrap();
        std::fs::create_dir_all(tmp.join("postProcessing/probes/0")).unwrap();

        std::fs::write(
            tmp.join("log.checkMesh"),
            r#"
Mesh stats
    points:           182400
    faces:            512000
    cells:            148200
Checking geometry...
    Max aspect ratio = 12.4 OK.
    Mesh non-orthogonality Max: 44.2 average: 7.85
    Max skewness = 1.62 OK.
Mesh OK.
"#,
        )
        .unwrap();

        std::fs::write(
            tmp.join("log.solver"),
            r#"
Time = 1
smoothSolver:  Solving for Ux, Initial residual = 0.42, Final residual = 0.03, No Iterations 3
GAMG:  Solving for p, Initial residual = 0.85, Final residual = 0.04, No Iterations 6
time step continuity errors : sum local = 1e-4, global = 2e-5, cumulative = 2e-5
Time = 240
smoothSolver:  Solving for Ux, Initial residual = 4.2e-05, Final residual = 1.1e-06, No Iterations 2
GAMG:  Solving for p, Initial residual = 7.8e-05, Final residual = 2.5e-06, No Iterations 4
time step continuity errors : sum local = 1.2e-08, global = -3.4e-09, cumulative = 1.5e-07
SIMPLE solution converged in 240 iterations
"#,
        )
        .unwrap();

        std::fs::write(
            tmp.join("postProcessing/yPlus/500/yPlus.dat"),
            r#"
# Time          patch           min             max             average
100             airfoil         1.10            5.20            2.80
500             airfoil         0.45            2.65            1.35
500             lowerWall       0.60            2.90            1.55
"#,
        )
        .unwrap();

        std::fs::write(
            tmp.join("postProcessing/forceCoeffs1/0/coefficient.dat"),
            r#"
# Force coefficients
# Time          Cm              Cd              Cl              Cl(f)           Cl(r)
100             -0.015          0.0240          0.350           0.16            0.19
240             -0.022          0.0168          0.428           0.19            0.23
"#,
        )
        .unwrap();

        std::fs::write(
            tmp.join("postProcessing/inletPressure/0/surfaceFieldValue.dat"),
            "# Time          areaAverage(p)\n240             18.50\n",
        )
        .unwrap();
        std::fs::write(
            tmp.join("postProcessing/outletPressure/0/surfaceFieldValue.dat"),
            "# Time          areaAverage(p)\n240             0.50\n",
        )
        .unwrap();

        std::fs::write(
            tmp.join("postProcessing/probes/0/U"),
            r#"# Probe 0 (0.0 0.0 0.0)
# Probe 1 (1.0 0.0 0.0)
# Probe 2 (2.0 0.0 0.0)
# Probe 3 (0.0 0.5 0.0)
# Time
240 (25.0 0.0 0.0) (6.0 2.5 0.0) (19.0 -0.5 0.0) (26.2 0.4 0.0)
"#,
        )
        .unwrap();
        std::fs::write(
            tmp.join("postProcessing/probes/0/p"),
            r#"# Probe 0 (0.0 0.0 0.0)
# Probe 1 (1.0 0.0 0.0)
# Probe 2 (2.0 0.0 0.0)
# Probe 3 (0.0 0.5 0.0)
# Time
240 24.0 -140.0 -12.0 18.0
"#,
        )
        .unwrap();

        let summary = ingest_openfoam_results(&tmp, &case, "local").unwrap();
        assert_eq!(summary.backend_used, "local");
        assert!(summary.converged);
        assert!(!summary.diverged);
        assert_eq!(summary.iterations_completed, 2);
        assert!((summary.final_residual_u - 4.2e-5).abs() < 1e-8);
        assert!((summary.final_residual_p - 7.8e-5).abs() < 1e-8);
        assert!((summary.continuity_error - 1.5e-7).abs() < 1e-10);

        assert_eq!(summary.check_mesh.cells, 148_200);
        assert!(summary.check_mesh.mesh_ok);
        assert_eq!(summary.check_mesh.failed_checks, 0);
        assert!((summary.check_mesh.max_non_orthogonality - 44.2).abs() < 1e-6);
        assert!((summary.check_mesh.avg_non_orthogonality - 7.85).abs() < 1e-6);
        assert!((summary.check_mesh.max_skewness - 1.62).abs() < 1e-6);
        assert!((summary.check_mesh.max_aspect_ratio - 12.4).abs() < 1e-6);

        assert_eq!(summary.y_plus_patches.len(), 2);
        let airfoil_yp = summary
            .y_plus_patches
            .iter()
            .find(|p| p.patch == "airfoil")
            .unwrap();
        assert!((airfoil_yp.average - 1.35).abs() < 1e-6);

        assert!((summary.cd.unwrap() - 0.0168).abs() < 1e-6);
        assert!((summary.cl.unwrap() - 0.428).abs() < 1e-6);
        assert!((summary.cm.unwrap() + 0.022).abs() < 1e-6);

        // delta_p_pa = (18.5 - 0.5) * 1.204 = 21.672 Pa
        assert!((summary.delta_p_pa.unwrap() - 18.0 * case.physics.fluid.density).abs() < 1e-4);
        assert!(!summary.hotspots.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn surrogate_responds_to_yplus_refinement_and_geometry_optimization() {
        let mut case = sample_case();
        case.geometry_kind = PresetGeometryKind::Parametric;
        case.parametric = Some(ParametricGeometrySpec::AhmedDiffuser {
            length: 1.044,
            width: 0.389,
            height: 0.288,
            rear_slant_deg: 29.0, // high-drag vortex peak
            ground_clearance: 0.05,
        });
        case.domain = DomainBox {
            x_min: -2.2,
            x_max: 6.2,
            y_min: 0.0,
            y_max: 2.2,
            z_min: -1.4,
            z_max: 1.4,
        };
        // Intentionally coarse first layer -> buffer layer y+
        case.mesh.first_layer_thickness = 0.00055;
        case.mesh.target_y_plus = 1.5;

        let guardrails = GuardrailsConfig::default();
        let goal = OptimizationGoal {
            prompt: "Minimize Ahmed drag and reach y+ ~ 1.5".into(),
            target_y_plus: Some(1.5),
            max_delta_p_pa: None,
            minimize_cd: true,
            maximize_cl_over_cd: false,
        };

        let base_res = evaluate_surrogate_results(&case);
        let base_score = compute_objective_score(&case, &base_res, &goal, &guardrails);
        assert!(base_res.converged);
        assert!(base_res.cd.unwrap() > 0.33);
        assert!(!base_res.hotspots.is_empty());

        // Step 1: Auto-size y+ first layer thickness to target y+ = 1.5
        let yp_calc = compute_y_plus(&YPlusInput {
            velocity: case.physics.reference_velocity,
            density: case.physics.fluid.density,
            dynamic_viscosity: case.physics.fluid.dynamic_viscosity,
            reference_length: case.physics.reference_length,
            target_y_plus: 1.5,
            expansion_ratio: case.mesh.expansion_ratio,
        })
        .unwrap();
        case.mesh.first_layer_thickness = yp_calc.first_cell_height;

        // Step 2: Add refinement box covering top hotspot
        let top_h = base_res.hotspots[0].clone();
        case.mesh.refinement_boxes.push(RefinementBox {
            name: "slant_vortex_box".into(),
            min: top_h.suggested_box_min,
            max: top_h.suggested_box_max,
            level: top_h.suggested_level,
            reason: "Resolve C-pillar vortex".into(),
        });

        // Step 3: Optimize Ahmed rear slant angle from 29° toward low-drag 13°
        case.parametric = Some(ParametricGeometrySpec::AhmedDiffuser {
            length: 1.044,
            width: 0.389,
            height: 0.288,
            rear_slant_deg: 13.0,
            ground_clearance: 0.05,
        });

        let opt_res = evaluate_surrogate_results(&case);
        let opt_score = compute_objective_score(&case, &opt_res, &goal, &guardrails);

        assert!(opt_res.cd.unwrap() < base_res.cd.unwrap() - 0.08);
        assert!(opt_res.hotspots[0].gradient_score < base_res.hotspots[0].gradient_score * 0.5);
        assert!(
            opt_score > base_score + 15.0,
            "Expected optimized score ({}) >> baseline ({})",
            opt_score,
            base_score
        );
    }

    #[test]
    fn objective_score_heavily_penalizes_divergence_and_guardrail_breaches() {
        let case = sample_case();
        let goal = OptimizationGoal::default();
        let guardrails = GuardrailsConfig::default();

        let mut res = evaluate_surrogate_results(&case);
        let healthy_score = compute_objective_score(&case, &res, &goal, &guardrails);

        res.check_mesh.max_skewness = 6.5;
        res.check_mesh.mesh_ok = false;
        res.check_mesh.failed_checks = 2;
        let bad_mesh_score = compute_objective_score(&case, &res, &goal, &guardrails);
        assert!(bad_mesh_score < healthy_score - 30.0);

        res.diverged = true;
        res.converged = false;
        let diverged_score = compute_objective_score(&case, &res, &goal, &guardrails);
        assert!(diverged_score <= 5.0);
    }

    #[test]
    fn parses_diverged_solver_log_and_foundation_force_coeffs() {
        let stats = parse_solver_log(
            "Time = 1\nsmoothSolver: Solving for Ux, Initial residual = -nan, Final residual = nan\nFOAM FATAL ERROR: Floating point exception\n",
            1e-4,
        );
        assert!(stats.diverged);
        assert!(!stats.converged);

        let foundation_dat = "# Time  Cd  Cs  Cl  CmRoll  CmPitch  CmYaw\n100 0.021 0.0 0.51 0.0 -0.045 0.0\n";
        let (cd, cl, cm) = parse_force_coeffs_dat(foundation_dat);
        assert!((cd.unwrap() - 0.021).abs() < 1e-9);
        assert!((cl.unwrap() - 0.51).abs() < 1e-9);
        assert!((cm.unwrap() + 0.045).abs() < 1e-9);
    }

    #[test]
    fn surrogate_physics_sensitivity_across_parametric_families() {
        let mut case = sample_case();
        case.geometry_kind = PresetGeometryKind::Parametric;

        // 1. NACA airfoil: camber + AoA increases Cl and L/D
        case.parametric = Some(ParametricGeometrySpec::NacaAirfoil {
            chord: 1.0,
            camber_pct: 0.0,
            camber_pos_pct: 40.0,
            thickness_pct: 12.0,
            aoa_deg: 0.0,
            span: 0.8,
        });
        let naca0 = evaluate_surrogate_results(&case);
        case.parametric = Some(ParametricGeometrySpec::NacaAirfoil {
            chord: 1.0,
            camber_pct: 2.0,
            camber_pos_pct: 40.0,
            thickness_pct: 12.0,
            aoa_deg: 5.0,
            span: 0.8,
        });
        let naca5 = evaluate_surrogate_results(&case);
        assert!(naca5.cl.unwrap() > naca0.cl.unwrap() + 0.5);

        // 2. Venturi: wider throat & longer diffuser reduce pressure drop
        case.parametric = Some(ParametricGeometrySpec::VenturiTube {
            inlet_radius: 0.5,
            throat_radius: 0.22,
            outlet_radius: 0.5,
            convergent_len: 1.0,
            throat_len: 0.6,
            divergent_len: 1.0, // steep diffuser
        });
        let v_steep = evaluate_surrogate_results(&case);
        case.parametric = Some(ParametricGeometrySpec::VenturiTube {
            inlet_radius: 0.5,
            throat_radius: 0.30,
            outlet_radius: 0.5,
            convergent_len: 1.0,
            throat_len: 0.6,
            divergent_len: 2.8, // gentle diffuser
        });
        let v_gentle = evaluate_surrogate_results(&case);
        assert!(v_gentle.delta_p_pa.unwrap() < v_steep.delta_p_pa.unwrap() * 0.5);

        // 3. Pin-fin: more fins / thicker pins increase pressure drop
        case.parametric = Some(ParametricGeometrySpec::PinFinArray {
            base_length: 0.2,
            base_width: 0.15,
            fin_count_x: 3,
            fin_count_z: 3,
            fin_radius: 0.006,
            fin_height: 0.05,
        });
        let pf_sparse = evaluate_surrogate_results(&case);
        case.parametric = Some(ParametricGeometrySpec::PinFinArray {
            base_length: 0.2,
            base_width: 0.15,
            fin_count_x: 6,
            fin_count_z: 5,
            fin_radius: 0.009,
            fin_height: 0.05,
        });
        let pf_dense = evaluate_surrogate_results(&case);
        assert!(pf_dense.delta_p_pa.unwrap() > pf_sparse.delta_p_pa.unwrap() * 2.0);
    }
}
