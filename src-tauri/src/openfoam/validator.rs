use crate::models::{
    BoundaryKind, CfdCase, DiagnosticSeverity, FlowRegime, TurbulenceModel, ValidationItem,
    ValidationReport,
};
use crate::physics::compute_telemetry;

/// Performs an automated engineering pre-flight audit of an OpenFOAM case configuration.
pub fn validate_case(case: &CfdCase) -> ValidationReport {
    let mut items = Vec::new();
    let telemetry = compute_telemetry(case);

    // 1. Domain geometry sanity
    if case.domain.x_max <= case.domain.x_min
        || case.domain.y_max <= case.domain.y_min
        || case.domain.z_max <= case.domain.z_min
    {
        items.push(ValidationItem {
            id: "domain_bounds".into(),
            category: "Geometry".into(),
            severity: DiagnosticSeverity::Error,
            title: "Invalid domain extents".into(),
            detail: "Each domain axis must satisfy max > min.".into(),
            suggestion: Some("Adjust domain bounding box coordinates so Lx, Ly, Lz > 0.".into()),
        });
    } else {
        items.push(ValidationItem {
            id: "domain_bounds".into(),
            category: "Geometry".into(),
            severity: DiagnosticSeverity::Pass,
            title: format!(
                "Domain geometry valid ({:.2} × {:.2} × {:.2} m)",
                case.domain.lx(),
                case.domain.ly(),
                case.domain.lz()
            ),
            detail: "Domain bounding box has strictly positive volume.".into(),
            suggestion: None,
        });
    }

    // 2. STL clearance inside domain
    if let Some(stl) = &case.stl {
        let inside = stl.bounds_min[0] > case.domain.x_min
            && stl.bounds_max[0] < case.domain.x_max
            && stl.bounds_min[1] > case.domain.y_min
            && stl.bounds_max[1] < case.domain.y_max
            && stl.bounds_min[2] >= case.domain.z_min
            && stl.bounds_max[2] <= case.domain.z_max;
        if !inside {
            items.push(ValidationItem {
                id: "stl_domain_fit".into(),
                category: "Geometry".into(),
                severity: DiagnosticSeverity::Error,
                title: "STL geometry exceeds computational domain".into(),
                detail: format!(
                    "{} bounding box extends outside blockMesh domain.",
                    stl.filename
                ),
                suggestion: Some(
                    "Use 'Auto-Fit Domain' in Geometry to wrap the STL with wake clearance.".into(),
                ),
            });
        } else {
            let stl_len = (stl.bounds_max[0] - stl.bounds_min[0]).abs().max(1e-3);
            let wake_len = case.domain.x_max - stl.bounds_max[0];
            if wake_len < 3.0 * stl_len {
                items.push(ValidationItem {
                    id: "stl_wake".into(),
                    category: "Geometry".into(),
                    severity: DiagnosticSeverity::Warning,
                    title: "Short downstream wake length".into(),
                    detail: format!(
                        "Downstream distance is {:.1}× body length (recommended ≥ 5× for external aero).",
                        wake_len / stl_len
                    ),
                    suggestion: Some("Extend X Max to prevent outlet boundary reflections.".into()),
                });
            }
        }
    }

    // 3. Boundary condition completeness
    let inlets = case
        .boundaries
        .iter()
        .filter(|b| b.kind == BoundaryKind::VelocityInlet)
        .count();
    let outlets = case
        .boundaries
        .iter()
        .filter(|b| b.kind == BoundaryKind::PressureOutlet)
        .count();
    let moving = case
        .boundaries
        .iter()
        .filter(|b| b.kind == BoundaryKind::MovingWall)
        .count();

    if inlets > 0 && outlets == 0 {
        items.push(ValidationItem {
            id: "bc_no_outlet".into(),
            category: "Boundaries".into(),
            severity: DiagnosticSeverity::Error,
            title: "Inlet defined without pressure outlet".into(),
            detail: "Incompressible continuity cannot be satisfied when mass enters a closed box without an outlet.".into(),
            suggestion: Some("Set at least one patch (e.g. X Max) to Pressure Outlet.".into()),
        });
    } else if inlets == 0 && moving == 0 {
        items.push(ValidationItem {
            id: "bc_no_driver".into(),
            category: "Boundaries".into(),
            severity: DiagnosticSeverity::Warning,
            title: "No velocity inlet or moving wall defined".into(),
            detail: "Flow has no kinematic boundary driver (unless purely buoyancy-driven).".into(),
            suggestion: Some("Assign a Velocity Inlet or Moving Wall patch.".into()),
        });
    } else {
        items.push(ValidationItem {
            id: "bc_well_posed".into(),
            category: "Boundaries".into(),
            severity: DiagnosticSeverity::Pass,
            title: "Boundary conditions well-posed".into(),
            detail: format!(
                "{} inlet(s), {} outlet(s), {} total patches configured.",
                inlets,
                outlets,
                case.boundaries.len()
            ),
            suggestion: None,
        });
    }

    // 4. Compressibility / Mach number check
    if telemetry.compressible_warning {
        items.push(ValidationItem {
            id: "mach_compressibility".into(),
            category: "Physics".into(),
            severity: DiagnosticSeverity::Warning,
            title: format!("Compressibility effects significant (Ma = {:.2})", telemetry.mach),
            detail: "Gas flow exceeds Ma = 0.3 while an incompressible solver is selected (density variations > 5%).".into(),
            suggestion: Some("Switch flow regime to Steady Compressible (rhoSimpleFoam / fluid).".into()),
        });
    }

    // 5. Reynolds vs Turbulence model check
    if telemetry.reynolds > 4000.0 && case.physics.turbulence == TurbulenceModel::Laminar {
        items.push(ValidationItem {
            id: "re_laminar_mismatch".into(),
            category: "Physics".into(),
            severity: DiagnosticSeverity::Warning,
            title: format!("High Reynolds ({:.2e}) with Laminar model", telemetry.reynolds),
            detail: "Flow is strongly turbulent; laminar solver may fail to converge or mispredict separation.".into(),
            suggestion: Some("Switch turbulence model to k-omega SST or k-epsilon.".into()),
        });
    } else if telemetry.reynolds < 2300.0 && case.physics.turbulence != TurbulenceModel::Laminar {
        items.push(ValidationItem {
            id: "re_turbulent_low".into(),
            category: "Physics".into(),
            severity: DiagnosticSeverity::Info,
            title: format!("Low Reynolds ({:.0}) with RANS turbulence", telemetry.reynolds),
            detail: "Reynolds number suggests laminar flow; RANS models may add artificial eddy viscosity.".into(),
            suggestion: Some("Consider Laminar model if transition is not forced.".into()),
        });
    }

    // 6. Turbulence IC non-zero check
    if !matches!(
        case.physics.turbulence,
        TurbulenceModel::Laminar | TurbulenceModel::WaleLes
    ) && (case.physics.turbulence_ic.k <= 0.0 || case.physics.turbulence_ic.omega <= 0.0)
    {
        items.push(ValidationItem {
            id: "turb_ic_zero".into(),
            category: "Physics".into(),
            severity: DiagnosticSeverity::Error,
            title: "Zero turbulence initial conditions".into(),
            detail: "k and omega/epsilon must be strictly positive to avoid division by zero in RANS equations.".into(),
            suggestion: Some("Open Engineering Lab → Turbulence IC and click Apply to Case.".into()),
        });
    }

    // 7. y+ & Wall Function compatibility
    let yp = case.mesh.target_y_plus;
    if case.physics.turbulence != TurbulenceModel::Laminar {
        if yp > 5.0 && yp < 30.0 {
            items.push(ValidationItem {
                id: "yplus_buffer".into(),
                category: "Mesh / y+".into(),
                severity: DiagnosticSeverity::Warning,
                title: format!("Target y+ ({:.1}) falls in buffer layer (5–30)", yp),
                detail: "Both linear viscous sublayer and log-law wall assumptions lose accuracy in the buffer region.".into(),
                suggestion: Some("Target y+ ≈ 1 (wall-resolved) or y+ = 35–100 (wall functions).".into()),
            });
        } else if case.physics.turbulence == TurbulenceModel::KEpsilon && yp < 20.0 {
            items.push(ValidationItem {
                id: "keps_low_yplus".into(),
                category: "Mesh / y+".into(),
                severity: DiagnosticSeverity::Warning,
                title: "Standard k-epsilon used with low y+".into(),
                detail: "Standard k-epsilon expects wall functions (y+ ≥ 30) and over-predicts shear at y+ ≈ 1.".into(),
                suggestion: Some("Switch to k-omega SST for wall-resolved meshes or raise y+ to 35+.".into()),
            });
        } else {
            items.push(ValidationItem {
                id: "yplus_ok".into(),
                category: "Mesh / y+".into(),
                severity: DiagnosticSeverity::Pass,
                title: format!("y+ target ({:.1}) compatible with {:?}", yp, case.physics.turbulence),
                detail: format!(
                    "Estimated first-cell height Δy₁ ≈ {:.4} mm.",
                    telemetry.first_cell_height_mm
                ),
                suggestion: None,
            });
        }
    }

    // 8. Transient Courant check
    if matches!(case.physics.regime, FlowRegime::TransientIncompressible) {
        if telemetry.estimated_courant > 5.0 {
            items.push(ValidationItem {
                id: "cfl_high".into(),
                category: "Numerics".into(),
                severity: DiagnosticSeverity::Warning,
                title: format!("Estimated Courant Co ≈ {:.1} is high", telemetry.estimated_courant),
                detail: "Large timesteps relative to Δx_min can destabilize PIMPLE or smear vortices.".into(),
                suggestion: Some(format!(
                    "Reduce deltaT below {:.2e} s (Co ≤ 1.0) in Solver & Numerics.",
                    telemetry.min_dx / case.physics.reference_velocity.max(1e-6)
                )),
            });
        } else {
            items.push(ValidationItem {
                id: "cfl_ok".into(),
                category: "Numerics".into(),
                severity: DiagnosticSeverity::Pass,
                title: format!("Transient Courant Co ≈ {:.2} within safe bounds", telemetry.estimated_courant),
                detail: "Timestep is well matched to minimum cell size and reference velocity.".into(),
                suggestion: None,
            });
        }
    }

    // 9. Parallelism / RAM guidance
    if telemetry.estimated_cells > 500_000 && case.numerics.parallel_cores == 1 {
        items.push(ValidationItem {
            id: "parallel_tip".into(),
            category: "Performance".into(),
            severity: DiagnosticSeverity::Info,
            title: format!("Large mesh (~{}k cells) on 1 CPU core", telemetry.estimated_cells / 1000),
            detail: format!("Estimated RAM: {:.0} MB. Parallel MPI execution will significantly reduce solve time.", telemetry.estimated_ram_mb),
            suggestion: Some(format!("Increase parallel cores to {} (decomposePar).", telemetry.recommended_cores)),
        });
    }

    let error_count = items
        .iter()
        .filter(|i| i.severity == DiagnosticSeverity::Error)
        .count();
    let warning_count = items
        .iter()
        .filter(|i| i.severity == DiagnosticSeverity::Warning)
        .count();

    let penalty = (error_count * 30 + warning_count * 12).min(100) as u32;
    let score = 100u32.saturating_sub(penalty);

    ValidationReport {
        score,
        ready_to_run: error_count == 0,
        error_count,
        warning_count,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openfoam::generator::tests::sample_case;

    #[test]
    fn sample_case_is_ready_to_run() {
        let case = sample_case();
        let report = validate_case(&case);
        assert!(report.ready_to_run);
        assert_eq!(report.error_count, 0);
        assert!(report.score >= 85);
    }

    #[test]
    fn flags_missing_outlet_error() {
        let mut case = sample_case();
        case.boundaries.retain(|b| b.kind != BoundaryKind::PressureOutlet);
        let report = validate_case(&case);
        assert!(!report.ready_to_run);
        assert!(report.error_count >= 1);
    }
}
