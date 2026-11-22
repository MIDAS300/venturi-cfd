use crate::error::VenturiError;
use crate::models::{
    CfdCase, CflInput, CflResult, FlowRegime, FluidConfig, OpenFoamFlavor, TelemetrySnapshot,
    TurbulenceIcInput, TurbulenceIcResult, TurbulenceModel, YPlusInput, YPlusResult,
};

const C_MU: f64 = 0.09;

/// Computes air properties using ideal gas law and Sutherland's viscosity law.
pub fn compute_air_sutherland(temperature_k: f64, pressure_pa: f64) -> Result<FluidConfig, VenturiError> {
    if temperature_k < 50.0 || temperature_k > 3000.0 {
        return Err(VenturiError::PhysicsBounds(
            "Temperature must be between 50 K and 3000 K".into(),
        ));
    }
    if pressure_pa <= 0.0 {
        return Err(VenturiError::PhysicsBounds(
            "Pressure must be positive".into(),
        ));
    }

    let r_air = 287.058;
    let gamma = 1.4;
    let density = pressure_pa / (r_air * temperature_k);

    // Sutherland's law for air: mu_ref = 1.716e-5 Pa.s at T_ref = 273.15 K, S = 110.4 K
    let t_ref = 273.15;
    let mu_ref = 1.716e-5;
    let s_const = 110.4;
    let mu = mu_ref * (temperature_k / t_ref).powf(1.5) * (t_ref + s_const) / (temperature_k + s_const);
    let nu = mu / density;

    Ok(FluidConfig {
        name: format!("Air ({:.1} °C, {:.1} kPa)", temperature_k - 273.15, pressure_pa / 1000.0),
        density,
        dynamic_viscosity: mu,
        kinematic_viscosity: nu,
        temperature_k,
        pressure_pa,
        gamma,
        gas_constant: r_air,
        is_gas: true,
    })
}

/// Computes Boundary Layer thickness, skin friction, friction velocity, first cell height for target y+,
/// and recommended inflation layers for snappyHexMesh.
pub fn compute_y_plus(input: &YPlusInput) -> Result<YPlusResult, VenturiError> {
    if input.velocity <= 0.0
        || input.density <= 0.0
        || input.dynamic_viscosity <= 0.0
        || input.reference_length <= 0.0
        || input.target_y_plus <= 0.0
        || input.expansion_ratio <= 1.0
    {
        return Err(VenturiError::PhysicsBounds(
            "All y+ parameters must be strictly positive, and expansion ratio > 1.0".into(),
        ));
    }

    let nu = input.dynamic_viscosity / input.density;
    let re = input.velocity * input.reference_length / nu;

    let (cf, delta_99, regime_label) = if re < 5.0e5 {
        // Blasius flat-plate laminar correlation
        let cf_lam = 0.664 / re.sqrt();
        let d99 = 4.91 * input.reference_length / re.sqrt();
        (cf_lam, d99, "Laminar / Transitional (Re < 5e5)".to_string())
    } else {
        // Schlichting turbulent flat-plate skin friction correlation
        let cf_turb = (2.0 * re.log10() - 0.65).powf(-2.3);
        let d99 = 0.37 * input.reference_length / re.powf(0.2);
        (cf_turb, d99, "Fully Turbulent (Schlichting)".to_string())
    };

    let tau_w = 0.5 * cf * input.density * input.velocity.powi(2);
    let u_tau = (tau_w / input.density).sqrt();

    // Cell-center wall distance y_c = y+ * nu / u_tau; total first-cell height dy1 = 2 * y_c
    let wall_distance_yc = input.target_y_plus * nu / u_tau;
    let first_cell_height = 2.0 * wall_distance_yc;

    // Solve geometric progression for layers covering delta_99:
    // H_total = dy1 * (r^N - 1) / (r - 1) >= delta_99
    let r = input.expansion_ratio;
    let rhs = 1.0 + (delta_99 / first_cell_height) * (r - 1.0);
    let raw_layers = if rhs > 1.0 {
        (rhs.ln() / r.ln()).ceil() as u32
    } else {
        3
    };
    let recommended_layers = raw_layers.clamp(3, 35);
    let total_prism_height = first_cell_height * (r.powi(recommended_layers as i32) - 1.0) / (r - 1.0);

    let (wall_treatment_note, buffer_layer_warning) = if input.target_y_plus <= 1.5 {
        (
            "Low-Re wall-resolved (viscous sublayer). Ideal for k-omega SST / LES heat transfer & separation."
                .to_string(),
            false,
        )
    } else if input.target_y_plus <= 5.0 {
        (
            "Near-viscous sublayer (y+ ≤ 5). Good for k-omega SST; avoid standard k-epsilon wall functions."
                .to_string(),
            false,
        )
    } else if input.target_y_plus < 30.0 {
        (
            "WARNING: Buffer layer (5 < y+ < 30). Wall functions are inaccurate here — move to y+ ≈ 1 or y+ ≥ 35."
                .to_string(),
            true,
        )
    } else if input.target_y_plus <= 300.0 {
        (
            "Log-law region (30 ≤ y+ ≤ 300). Optimal for high-Re wall functions (kqRWallFunction / omegaWallFunction)."
                .to_string(),
            false,
        )
    } else {
        (
            "WARNING: y+ > 300 exceeds log-law validity. Refine first cell height for accurate drag/shear."
                .to_string(),
            true,
        )
    };

    Ok(YPlusResult {
        reynolds: re,
        flow_regime_label: regime_label,
        skin_friction_cf: cf,
        wall_shear_tau_w: tau_w,
        friction_velocity_u_tau: u_tau,
        first_cell_height,
        wall_distance_yc,
        boundary_layer_delta99: delta_99,
        recommended_layers,
        total_prism_height,
        wall_treatment_note,
        buffer_layer_warning,
    })
}

/// Computes OpenFOAM turbulence initial conditions (k, epsilon, omega, nut, nuTilda).
pub fn compute_turbulence_ic(input: &TurbulenceIcInput) -> Result<TurbulenceIcResult, VenturiError> {
    if input.velocity <= 0.0
        || input.intensity_percent <= 0.0
        || input.intensity_percent > 100.0
        || input.length_scale <= 0.0
        || input.kinematic_viscosity <= 0.0
    {
        return Err(VenturiError::PhysicsBounds(
            "Velocity, length scale, viscosity must be > 0 and intensity in (0, 100]%".into(),
        ));
    }

    let intensity = input.intensity_percent / 100.0;
    let k = 1.5 * (input.velocity * intensity).powi(2);
    let epsilon = C_MU.powf(0.75) * k.powf(1.5) / input.length_scale;
    let omega = k.sqrt() / (C_MU.powf(0.25) * input.length_scale);
    let nut = C_MU * k.powi(2) / epsilon;
    let nu_tilda = (1.5_f64).sqrt() * input.velocity * intensity * input.length_scale;
    let viscosity_ratio = nut / input.kinematic_viscosity;

    Ok(TurbulenceIcResult {
        k,
        epsilon,
        omega,
        nut,
        nu_tilda,
        viscosity_ratio,
    })
}

/// Computes CFL / Courant stability timestep and convective flow-through guidance.
pub fn compute_cfl(input: &CflInput) -> Result<CflResult, VenturiError> {
    if input.min_cell_size <= 0.0
        || input.max_velocity <= 0.0
        || input.target_courant <= 0.0
        || input.domain_length <= 0.0
    {
        return Err(VenturiError::PhysicsBounds(
            "All CFL inputs must be strictly positive".into(),
        ));
    }

    let max_delta_t = input.target_courant * input.min_cell_size / input.max_velocity;
    let flow_through_time = input.domain_length / input.max_velocity;
    let recommended_end_time = flow_through_time * 5.0;
    let estimated_steps = (recommended_end_time / max_delta_t).ceil() as u64;

    let stability_note = if input.target_courant <= 0.5 {
        "Conservative LES / high-accuracy acoustics regime (Co ≤ 0.5).".into()
    } else if input.target_courant <= 1.0 {
        "Standard PISO / PIMPLE time-accurate regime (Co ≤ 1.0).".into()
    } else if input.target_courant <= 5.0 {
        "Implicit PIMPLE regime (1 < Co ≤ 5). Requires nOuterCorrectors ≥ 2.".into()
    } else {
        "High Courant (Co > 5): risk of temporal smearing or divergence unless fully implicit.".into()
    };

    Ok(CflResult {
        max_delta_t,
        flow_through_time,
        recommended_end_time,
        estimated_steps,
        stability_note,
    })
}

/// Resolves the canonical OpenFOAM solver executable name for the selected flavor & physics.
pub fn resolve_solver_name(flavor: OpenFoamFlavor, regime: FlowRegime) -> String {
    match (flavor, regime) {
        (OpenFoamFlavor::FoundationV14, FlowRegime::SteadyIncompressible) => {
            "foamRun -solver incompressibleFluid".into()
        }
        (OpenFoamFlavor::FoundationV14, FlowRegime::TransientIncompressible) => {
            "foamRun -solver incompressibleFluid".into()
        }
        (OpenFoamFlavor::FoundationV14, FlowRegime::SteadyCompressible) => {
            "foamRun -solver fluid".into()
        }
        (OpenFoamFlavor::OpenCfdV2606, FlowRegime::SteadyIncompressible) => "simpleFoam".into(),
        (OpenFoamFlavor::OpenCfdV2606, FlowRegime::TransientIncompressible) => "pimpleFoam".into(),
        (OpenFoamFlavor::OpenCfdV2606, FlowRegime::SteadyCompressible) => "rhoSimpleFoam".into(),
    }
}

/// Computes real-time viewport telemetry directly from the full CFD case configuration.
pub fn compute_telemetry(case: &CfdCase) -> TelemetrySnapshot {
    let nu = case.physics.fluid.kinematic_viscosity.max(1e-12);
    let u_ref = case.physics.reference_velocity.abs().max(1e-6);
    let l_ref = case.physics.reference_length.abs().max(1e-6);

    let reynolds = u_ref * l_ref / nu;

    let speed_of_sound = if case.physics.fluid.is_gas {
        (case.physics.fluid.gamma * case.physics.fluid.gas_constant * case.physics.fluid.temperature_k)
            .max(1.0)
            .sqrt()
    } else {
        1482.0 // typical water/liquid acoustic speed (m/s)
    };
    let mach = u_ref / speed_of_sound;

    // Cell count estimate
    let base_cells =
        (case.mesh.cells_x as u64) * (case.mesh.cells_y as u64) * (case.mesh.cells_z as u64);
    let estimated_cells = if case.mesh.snappy_enabled {
        let refine_factor = 1.0 + 0.35 * (2.0_f64.powi(case.mesh.refinement_max.min(5) as i32));
        let layer_bonus = (case.mesh.inflation_layers as f64) * (base_cells as f64).powf(0.66) * 0.45;
        ((base_cells as f64) * refine_factor + layer_bonus).round() as u64
    } else {
        base_cells
    };

    // Typical OpenFOAM memory footprint: ~1.45 KB/cell for RANS + baseline overhead ~45 MB
    let estimated_ram_mb = 45.0 + (estimated_cells as f64) * 0.00145;

    let dx = case.domain.lx() / (case.mesh.cells_x.max(1) as f64);
    let dy = case.domain.ly() / (case.mesh.cells_y.max(1) as f64);
    let dz = case.domain.lz() / (case.mesh.cells_z.max(1) as f64);
    let mut min_dx = dx.min(dy).min(dz);
    if case.mesh.snappy_enabled && case.mesh.refinement_max > 0 {
        min_dx /= 2.0_f64.powi(case.mesh.refinement_max.min(6) as i32);
    }

    let estimated_courant = match case.physics.regime {
        FlowRegime::SteadyIncompressible | FlowRegime::SteadyCompressible => 0.0,
        FlowRegime::TransientIncompressible => u_ref * case.numerics.delta_t / min_dx.max(1e-9),
    };

    let y_plus_calc = compute_y_plus(&YPlusInput {
        velocity: u_ref,
        density: case.physics.fluid.density.max(1e-6),
        dynamic_viscosity: case.physics.fluid.dynamic_viscosity.max(1e-12),
        reference_length: l_ref,
        target_y_plus: case.mesh.target_y_plus.max(0.1),
        expansion_ratio: case.mesh.expansion_ratio.max(1.05),
    });

    let first_cell_height_mm = y_plus_calc
        .map(|r| r.first_cell_height * 1000.0)
        .unwrap_or(case.mesh.first_layer_thickness * 1000.0);

    // ~50,000 cells per MPI core heuristic
    let recommended_cores = ((estimated_cells / 50_000).max(1)).min(64) as u32;
    let compressible_warning = case.physics.fluid.is_gas
        && mach > 0.3
        && !matches!(case.physics.regime, FlowRegime::SteadyCompressible);

    let solver_name = resolve_solver_name(case.flavor, case.physics.regime);

    TelemetrySnapshot {
        reynolds,
        mach,
        speed_of_sound,
        estimated_cells,
        estimated_ram_mb,
        min_dx,
        estimated_courant,
        first_cell_height_mm,
        recommended_cores,
        compressible_warning,
        solver_name,
    }
}

pub fn turbulence_model_name(model: TurbulenceModel) -> &'static str {
    match model {
        TurbulenceModel::Laminar => "laminar",
        TurbulenceModel::KOmegaSST => "kOmegaSST",
        TurbulenceModel::KEpsilon => "kEpsilon",
        TurbulenceModel::SpalartAllmaras => "SpalartAllmaras",
        TurbulenceModel::WaleLes => "WALE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sutherland_standard_air_at_20c() {
        let air = compute_air_sutherland(293.15, 101_325.0).unwrap();
        assert!((air.density - 1.204).abs() < 0.01);
        assert!((air.dynamic_viscosity - 1.813e-5).abs() < 2e-7);
        assert!((air.kinematic_viscosity - 1.506e-5).abs() < 2e-7);
    }

    #[test]
    fn y_plus_turbulent_airfoil_order_of_magnitude() {
        let res = compute_y_plus(&YPlusInput {
            velocity: 30.0,
            density: 1.225,
            dynamic_viscosity: 1.789e-5,
            reference_length: 1.0,
            target_y_plus: 1.0,
            expansion_ratio: 1.2,
        })
        .unwrap();
        // Re ~ 2.05e6, first cell height ~ 2.5e-5 m (~0.025 mm)
        assert!(res.reynolds > 2.0e6 && res.reynolds < 2.1e6);
        assert!(res.first_cell_height > 1.5e-5 && res.first_cell_height < 4.0e-5);
        assert!(!res.buffer_layer_warning);
    }

    #[test]
    fn y_plus_flags_buffer_layer() {
        let res = compute_y_plus(&YPlusInput {
            velocity: 20.0,
            density: 1.225,
            dynamic_viscosity: 1.8e-5,
            reference_length: 1.0,
            target_y_plus: 12.0,
            expansion_ratio: 1.2,
        })
        .unwrap();
        assert!(res.buffer_layer_warning);
    }

    #[test]
    fn turbulence_ic_formulas_match_theory() {
        let res = compute_turbulence_ic(&TurbulenceIcInput {
            velocity: 10.0,
            intensity_percent: 5.0,
            length_scale: 0.1,
            kinematic_viscosity: 1.5e-5,
        })
        .unwrap();
        // k = 1.5 * (10 * 0.05)^2 = 0.375
        assert!((res.k - 0.375).abs() < 1e-9);
        assert!(res.omega > 10.0);
        assert!(res.epsilon > 0.3);
        assert!(res.viscosity_ratio > 10.0);
    }

    #[test]
    fn cfl_computes_timestep() {
        let res = compute_cfl(&CflInput {
            min_cell_size: 0.01,
            max_velocity: 20.0,
            target_courant: 0.8,
            domain_length: 4.0,
        })
        .unwrap();
        assert!((res.max_delta_t - 0.0004).abs() < 1e-9);
        assert!((res.flow_through_time - 0.2).abs() < 1e-9);
    }
}
