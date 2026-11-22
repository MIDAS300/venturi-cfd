use crate::error::VenturiError;
use crate::models::{
    BoundaryKind, BoundaryPatch, CfdCase, DomainBox, DomainFace, FlowRegime, FluidConfig,
    GuardrailsConfig, McpToolLogEntry, MeshConfig, NumericsConfig, OpenFoamFlavor,
    OptimizationGoal, ParametricGeometrySpec, PhysicsConfig, PresetGeometryKind, SchemeAccuracy,
    SessionState, TurbulenceIC, TurbulenceModel,
};
use crate::physics::compute_telemetry;
use std::path::{Path, PathBuf};

const MAX_TOOL_LOGS: usize = 120;

/// Resolves the active session JSON file path shared between the Tauri GUI and `venturi-mcp`.
/// Checks `VENTURI_SESSION_FILE` first, then `$HOME/.venturi/active_session.json`,
/// falling back to the OS temporary directory.
pub fn default_session_path() -> PathBuf {
    if let Ok(env_path) = std::env::var("VENTURI_SESSION_FILE") {
        let trimmed = env_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed)
                .join(".venturi")
                .join("active_session.json");
        }
    }

    std::env::temp_dir().join("venturi_active_session.json")
}

fn air_isa_15c() -> FluidConfig {
    FluidConfig {
        name: "Air (ISA Sea Level, 15 °C)".into(),
        density: 1.225,
        dynamic_viscosity: 1.7894e-5,
        kinematic_viscosity: 1.4607e-5,
        temperature_k: 288.15,
        pressure_pa: 101_325.0,
        gamma: 1.4,
        gas_constant: 287.058,
        is_gas: true,
    }
}

fn air_20c() -> FluidConfig {
    FluidConfig {
        name: "Air (20 °C, 1 atm)".into(),
        density: 1.2041,
        dynamic_viscosity: 1.813e-5,
        kinematic_viscosity: 1.5057e-5,
        temperature_k: 293.15,
        pressure_pa: 101_325.0,
        gamma: 1.4,
        gas_constant: 287.058,
        is_gas: true,
    }
}

fn water_20c() -> FluidConfig {
    FluidConfig {
        name: "Water (Liquid, 20 °C)".into(),
        density: 998.2,
        dynamic_viscosity: 1.002e-3,
        kinematic_viscosity: 1.0038e-6,
        temperature_k: 293.15,
        pressure_pa: 101_325.0,
        gamma: 1.0,
        gas_constant: 461.5,
        is_gas: false,
    }
}

/// Builds clean Rust default `CfdCase` presets for external/internal CFD workflows
/// plus `"empty"` (starting without geometry so agents can synthesize geometry from scratch).
pub fn default_preset_case(preset_id: &str) -> CfdCase {
    match preset_id {
        "ahmed_body" => CfdCase {
            name: "ahmed_body_25deg".into(),
            description:
                "ERCOFTAC automotive drag benchmark (L = 1.044 m, U = 40 m/s, moving ground)."
                    .into(),
            flavor: OpenFoamFlavor::OpenCfdV2606,
            geometry_kind: PresetGeometryKind::AhmedBody,
            parametric: Some(ParametricGeometrySpec::AhmedDiffuser {
                length: 1.044,
                width: 0.389,
                height: 0.288,
                rear_slant_deg: 25.0,
                ground_clearance: 0.05,
            }),
            stl: None,
            domain: DomainBox {
                x_min: -2.2,
                x_max: 6.2,
                y_min: 0.0,
                y_max: 2.2,
                z_min: -1.4,
                z_max: 1.4,
            },
            mesh: MeshConfig {
                cells_x: 96,
                cells_y: 36,
                cells_z: 40,
                grading_y: 1.0,
                snappy_enabled: true,
                refinement_min: 2,
                refinement_max: 3,
                inflation_layers: 8,
                first_layer_thickness: 0.00048,
                expansion_ratio: 1.2,
                target_y_plus: 35.0,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::SteadyIncompressible,
                turbulence: TurbulenceModel::KOmegaSST,
                fluid: air_20c(),
                reference_velocity: 40.0,
                reference_length: 1.044,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.005,
                    length_scale: 0.05,
                    k: 0.06,
                    epsilon: 0.048,
                    omega: 8.94,
                    nut: 6.7e-3,
                    nu_tilda: 1.22e-2,
                },
                gravity_enabled: false,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "inlet".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::VelocityInlet,
                    velocity: [40.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "outlet".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::PressureOutlet,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "roadGround".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::MovingWall,
                    velocity: [40.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "sky".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "leftWall".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "rightWall".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "ahmedBody".into(),
                    face: DomainFace::Obstacle,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::BalancedSecondOrder,
                start_time: 0.0,
                end_time: 800.0,
                delta_t: 1.0,
                write_interval: 200.0,
                relax_p: 0.3,
                relax_u: 0.7,
                relax_turb: 0.7,
                convergence_tol: 1e-4,
                max_co: 0.9,
                n_non_ortho_correctors: 1,
                parallel_cores: 8,
            },
        },
        "venturi_nozzle" => CfdCase {
            name: "venturi_water_meter".into(),
            description:
                "Internal water flow through a Venturi throat (Δp measurement & cavitation screening)."
                    .into(),
            flavor: OpenFoamFlavor::FoundationV14,
            geometry_kind: PresetGeometryKind::VenturiNozzle,
            parametric: Some(ParametricGeometrySpec::VenturiTube {
                inlet_radius: 0.52,
                throat_radius: 0.26,
                outlet_radius: 0.52,
                convergent_len: 1.2,
                throat_len: 0.8,
                divergent_len: 2.8,
            }),
            stl: None,
            domain: DomainBox {
                x_min: -2.0,
                x_max: 4.0,
                y_min: -0.6,
                y_max: 0.6,
                z_min: -0.6,
                z_max: 0.6,
            },
            mesh: MeshConfig {
                cells_x: 120,
                cells_y: 32,
                cells_z: 32,
                grading_y: 1.0,
                snappy_enabled: true,
                refinement_min: 1,
                refinement_max: 2,
                inflation_layers: 6,
                first_layer_thickness: 0.0003,
                expansion_ratio: 1.2,
                target_y_plus: 40.0,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::SteadyIncompressible,
                turbulence: TurbulenceModel::KEpsilon,
                fluid: water_20c(),
                reference_velocity: 3.5,
                reference_length: 0.25,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.04,
                    length_scale: 0.0175,
                    k: 0.0294,
                    epsilon: 0.0472,
                    omega: 17.8,
                    nut: 1.65e-3,
                    nu_tilda: 3.0e-3,
                },
                gravity_enabled: true,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "inlet".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::VelocityInlet,
                    velocity: [3.5, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "outlet".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::PressureOutlet,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "bottomWall".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.000045,
                },
                BoundaryPatch {
                    name: "topWall".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.000045,
                },
                BoundaryPatch {
                    name: "sideMin".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.000045,
                },
                BoundaryPatch {
                    name: "sideMax".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.000045,
                },
                BoundaryPatch {
                    name: "venturiWall".into(),
                    face: DomainFace::Obstacle,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.000015,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::BalancedSecondOrder,
                start_time: 0.0,
                end_time: 500.0,
                delta_t: 1.0,
                write_interval: 100.0,
                relax_p: 0.3,
                relax_u: 0.7,
                relax_turb: 0.7,
                convergence_tol: 1e-5,
                max_co: 0.8,
                n_non_ortho_correctors: 1,
                parallel_cores: 4,
            },
        },
        "cylinder_vortex" => CfdCase {
            name: "cylinder_karman_street".into(),
            description:
                "Transient PIMPLE simulation of von Kármán vortex shedding behind a circular cylinder."
                    .into(),
            flavor: OpenFoamFlavor::FoundationV14,
            geometry_kind: PresetGeometryKind::CylinderCrossflow,
            parametric: Some(ParametricGeometrySpec::Cylinder {
                radius: 0.25,
                span: 0.58,
            }),
            stl: None,
            domain: DomainBox {
                x_min: -2.0,
                x_max: 8.0,
                y_min: -2.5,
                y_max: 2.5,
                z_min: -0.3,
                z_max: 0.3,
            },
            mesh: MeshConfig {
                cells_x: 120,
                cells_y: 60,
                cells_z: 4,
                grading_y: 1.0,
                snappy_enabled: true,
                refinement_min: 2,
                refinement_max: 3,
                inflation_layers: 10,
                first_layer_thickness: 0.00015,
                expansion_ratio: 1.18,
                target_y_plus: 1.0,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::TransientIncompressible,
                turbulence: TurbulenceModel::KOmegaSST,
                fluid: air_20c(),
                reference_velocity: 8.0,
                reference_length: 0.5,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.02,
                    length_scale: 0.035,
                    k: 0.0384,
                    epsilon: 0.0352,
                    omega: 10.2,
                    nut: 3.76e-3,
                    nu_tilda: 6.86e-3,
                },
                gravity_enabled: false,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "inlet".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::VelocityInlet,
                    velocity: [8.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "outlet".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::PressureOutlet,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "lower".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "upper".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "front".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "back".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "cylinder".into(),
                    face: DomainFace::Obstacle,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::HighResolution,
                start_time: 0.0,
                end_time: 6.0,
                delta_t: 0.001,
                write_interval: 0.25,
                relax_p: 0.3,
                relax_u: 0.9,
                relax_turb: 0.7,
                convergence_tol: 1e-5,
                max_co: 0.7,
                n_non_ortho_correctors: 1,
                parallel_cores: 4,
            },
        },
        "lid_cavity" => CfdCase {
            name: "lid_driven_cavity".into(),
            description:
                "Classic Ghia et al. laminar lid-driven cavity validation case (blockMesh only)."
                    .into(),
            flavor: OpenFoamFlavor::OpenCfdV2606,
            geometry_kind: PresetGeometryKind::None,
            parametric: None,
            stl: None,
            domain: DomainBox {
                x_min: 0.0,
                x_max: 1.0,
                y_min: 0.0,
                y_max: 1.0,
                z_min: -0.05,
                z_max: 0.05,
            },
            mesh: MeshConfig {
                cells_x: 60,
                cells_y: 60,
                cells_z: 1,
                grading_y: 1.0,
                snappy_enabled: false,
                refinement_min: 1,
                refinement_max: 1,
                inflation_layers: 0,
                first_layer_thickness: 0.005,
                expansion_ratio: 1.2,
                target_y_plus: 1.0,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::SteadyIncompressible,
                turbulence: TurbulenceModel::Laminar,
                fluid: FluidConfig {
                    name: "Reference Fluid (Re = 1000)".into(),
                    density: 1.0,
                    dynamic_viscosity: 0.001,
                    kinematic_viscosity: 0.001,
                    temperature_k: 293.15,
                    pressure_pa: 101_325.0,
                    gamma: 1.4,
                    gas_constant: 287.058,
                    is_gas: false,
                },
                reference_velocity: 1.0,
                reference_length: 1.0,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.01,
                    length_scale: 0.05,
                    k: 1e-4,
                    epsilon: 1e-4,
                    omega: 1.0,
                    nut: 0.0,
                    nu_tilda: 0.0,
                },
                gravity_enabled: false,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "leftWall".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "rightWall".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "bottomWall".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "movingLid".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::MovingWall,
                    velocity: [1.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "front".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "back".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::BalancedSecondOrder,
                start_time: 0.0,
                end_time: 400.0,
                delta_t: 1.0,
                write_interval: 50.0,
                relax_p: 0.3,
                relax_u: 0.7,
                relax_turb: 0.7,
                convergence_tol: 1e-6,
                max_co: 0.8,
                n_non_ortho_correctors: 0,
                parallel_cores: 1,
            },
        },
        "empty" => CfdCase {
            name: "empty_cfd_workspace".into(),
            description:
                "Blank CFD wind tunnel without geometry — ready for parametric synthesis or STL import."
                    .into(),
            flavor: OpenFoamFlavor::FoundationV14,
            geometry_kind: PresetGeometryKind::None,
            parametric: None,
            stl: None,
            domain: DomainBox {
                x_min: -2.0,
                x_max: 6.0,
                y_min: -2.0,
                y_max: 2.0,
                z_min: -0.5,
                z_max: 0.5,
            },
            mesh: MeshConfig {
                cells_x: 64,
                cells_y: 32,
                cells_z: 8,
                grading_y: 1.0,
                snappy_enabled: false,
                refinement_min: 2,
                refinement_max: 3,
                inflation_layers: 6,
                first_layer_thickness: 0.0005,
                expansion_ratio: 1.2,
                target_y_plus: 1.5,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::SteadyIncompressible,
                turbulence: TurbulenceModel::KOmegaSST,
                fluid: air_20c(),
                reference_velocity: 20.0,
                reference_length: 1.0,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.02,
                    length_scale: 0.07,
                    k: 0.24,
                    epsilon: 0.275,
                    omega: 12.76,
                    nut: 1.88e-2,
                    nu_tilda: 3.43e-2,
                },
                gravity_enabled: false,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "inlet".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::VelocityInlet,
                    velocity: [20.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "outlet".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::PressureOutlet,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "lowerWall".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "upperWall".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "front".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "back".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::BalancedSecondOrder,
                start_time: 0.0,
                end_time: 500.0,
                delta_t: 1.0,
                write_interval: 100.0,
                relax_p: 0.3,
                relax_u: 0.7,
                relax_turb: 0.7,
                convergence_tol: 1e-4,
                max_co: 0.8,
                n_non_ortho_correctors: 1,
                parallel_cores: 4,
            },
        },
        // Default / "naca0012"
        _ => CfdCase {
            name: "naca0012_wind_tunnel".into(),
            description:
                "Subsonic external aerodynamics around a NACA 0012 airfoil (c = 1.0 m, Re ≈ 2.0M)."
                    .into(),
            flavor: OpenFoamFlavor::FoundationV14,
            geometry_kind: PresetGeometryKind::AirfoilNaca0012,
            parametric: Some(ParametricGeometrySpec::NacaAirfoil {
                chord: 1.0,
                camber_pct: 0.0,
                camber_pos_pct: 40.0,
                thickness_pct: 12.0,
                aoa_deg: 0.0,
                span: 0.76,
            }),
            stl: None,
            domain: DomainBox {
                x_min: -2.5,
                x_max: 6.5,
                y_min: -2.2,
                y_max: 2.2,
                z_min: -0.4,
                z_max: 0.4,
            },
            mesh: MeshConfig {
                cells_x: 110,
                cells_y: 56,
                cells_z: 8,
                grading_y: 1.0,
                snappy_enabled: true,
                refinement_min: 2,
                refinement_max: 4,
                inflation_layers: 12,
                first_layer_thickness: 0.000045,
                expansion_ratio: 1.2,
                target_y_plus: 1.0,
                refinement_boxes: Vec::new(),
            },
            physics: PhysicsConfig {
                regime: FlowRegime::SteadyIncompressible,
                turbulence: TurbulenceModel::KOmegaSST,
                fluid: air_isa_15c(),
                reference_velocity: 30.0,
                reference_length: 1.0,
                turbulence_ic: TurbulenceIC {
                    intensity: 0.01,
                    length_scale: 0.07,
                    k: 0.135,
                    epsilon: 0.1165,
                    omega: 9.58,
                    nut: 1.41e-2,
                    nu_tilda: 2.57e-2,
                },
                gravity_enabled: false,
                gravity: [0.0, -9.81, 0.0],
            },
            boundaries: vec![
                BoundaryPatch {
                    name: "inlet".into(),
                    face: DomainFace::XMin,
                    kind: BoundaryKind::VelocityInlet,
                    velocity: [30.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "outlet".into(),
                    face: DomainFace::XMax,
                    kind: BoundaryKind::PressureOutlet,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "lowerWall".into(),
                    face: DomainFace::YMin,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "upperWall".into(),
                    face: DomainFace::YMax,
                    kind: BoundaryKind::Symmetry,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "front".into(),
                    face: DomainFace::ZMin,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "back".into(),
                    face: DomainFace::ZMax,
                    kind: BoundaryKind::Empty2D,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
                BoundaryPatch {
                    name: "airfoil".into(),
                    face: DomainFace::Obstacle,
                    kind: BoundaryKind::NoSlipWall,
                    velocity: [0.0, 0.0, 0.0],
                    pressure: 0.0,
                    wall_roughness_ks: 0.0,
                },
            ],
            numerics: NumericsConfig {
                accuracy: SchemeAccuracy::BalancedSecondOrder,
                start_time: 0.0,
                end_time: 600.0,
                delta_t: 1.0,
                write_interval: 100.0,
                relax_p: 0.3,
                relax_u: 0.7,
                relax_turb: 0.7,
                convergence_tol: 1e-4,
                max_co: 0.8,
                n_non_ortho_correctors: 1,
                parallel_cores: 4,
            },
        },
    }
}

/// Reads `SessionState` from `path`, or initializes a clean default `naca0012` session (version 1).
pub fn load_or_init_session(path: &Path) -> SessionState {
    if let Ok(raw) = std::fs::read_to_string(path) {
        if let Ok(state) = serde_json::from_str::<SessionState>(&raw) {
            return state;
        }
    }

    SessionState {
        version: 1,
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_writer: "init".into(),
        case: default_preset_case("naca0012"),
        guardrails: GuardrailsConfig::default(),
        goal: OptimizationGoal::default(),
        iterations: Vec::new(),
        best_iteration_index: None,
        tool_logs: Vec::new(),
        case_dir: None,
    }
}

/// Atomically persists `SessionState` to disk by writing to a unique sibling `.tmp` file
/// and renaming over `path`. Increments `version`, updates `updated_at` (RFC3339), and sets `last_writer`.
pub fn save_session_atomic(
    path: &Path,
    state: &mut SessionState,
    writer: &str,
) -> Result<(), VenturiError> {
    state.version = state.version.saturating_add(1);
    state.updated_at = chrono::Utc::now().to_rfc3339();
    state.last_writer = writer.to_string();

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let serialized = serde_json::to_string_pretty(state)
        .map_err(|e| VenturiError::Execution(format!("Failed to serialize session: {}", e)))?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("active_session.json");
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let tmp_name = format!(".{}.{}.{}.tmp", file_name, std::process::id(), nanos);
    let tmp_path = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(tmp_name),
        _ => PathBuf::from(tmp_name),
    };

    if let Err(e) = std::fs::write(&tmp_path, serialized) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(VenturiError::Io(e));
    }

    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(VenturiError::Io(e));
    }

    Ok(())
}

/// Enforces hard engineering mesh & memory guardrails before committing any mesh/refinement change.
pub fn enforce_mesh_guardrails(
    case: &CfdCase,
    guardrails: &GuardrailsConfig,
) -> Result<(), VenturiError> {
    if case.mesh.cells_x == 0 || case.mesh.cells_y == 0 || case.mesh.cells_z == 0 {
        return Err(VenturiError::PhysicsBounds(
            "Mesh cell counts (cellsX, cellsY, cellsZ) must all be >= 1.".into(),
        ));
    }

    if case.mesh.refinement_min > case.mesh.refinement_max {
        return Err(VenturiError::PhysicsBounds(format!(
            "Invalid refinement levels: refinementMin ({}) cannot exceed refinementMax ({}).",
            case.mesh.refinement_min, case.mesh.refinement_max
        )));
    }

    if case.mesh.snappy_enabled
        && (case.mesh.refinement_max > guardrails.max_refinement_level
            || case.mesh.refinement_min > guardrails.max_refinement_level)
    {
        return Err(VenturiError::PhysicsBounds(format!(
            "Mesh guardrail violated: surface refinement level ({}, {}) exceeds maxRefinementLevel ({}).",
            case.mesh.refinement_min, case.mesh.refinement_max, guardrails.max_refinement_level
        )));
    }

    for rbox in &case.mesh.refinement_boxes {
        if rbox.min[0] >= rbox.max[0] || rbox.min[1] >= rbox.max[1] || rbox.min[2] >= rbox.max[2] {
            return Err(VenturiError::PhysicsBounds(format!(
                "Invalid refinement box '{}': every axis must satisfy min < max (min={:?}, max={:?}).",
                rbox.name, rbox.min, rbox.max
            )));
        }
        if rbox.level > guardrails.max_refinement_level {
            return Err(VenturiError::PhysicsBounds(format!(
                "Mesh guardrail violated: refinement box '{}' level {} exceeds maxRefinementLevel ({}).",
                rbox.name, rbox.level, guardrails.max_refinement_level
            )));
        }
    }

    let telemetry = compute_telemetry(case);

    if telemetry.estimated_cells > guardrails.max_cells {
        return Err(VenturiError::PhysicsBounds(format!(
            "Mesh guardrail violated: estimated cell count {} exceeds maxCells limit {} (reduce cellsX/cellsY/cellsZ, refinementMax, or raise guardrails.maxCells).",
            telemetry.estimated_cells, guardrails.max_cells
        )));
    }

    if telemetry.estimated_ram_mb > guardrails.max_ram_mb {
        return Err(VenturiError::PhysicsBounds(format!(
            "Memory guardrail violated: estimated RAM {:.1} MB exceeds maxRamMb limit {:.1} MB.",
            telemetry.estimated_ram_mb, guardrails.max_ram_mb
        )));
    }

    Ok(())
}

/// Appends an entry to the session MCP tool audit log, retaining the latest 120 entries.
pub fn append_tool_log(
    state: &mut SessionState,
    source: &str,
    tool_name: &str,
    args: &serde_json::Value,
    ok: bool,
    summary: String,
) {
    let raw_args = args.to_string();
    let args_preview = if raw_args.chars().count() > 180 {
        let mut s: String = raw_args.chars().take(177).collect();
        s.push_str("...");
        s
    } else {
        raw_args
    };

    let entry = McpToolLogEntry {
        id: format!(
            "log-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            state.tool_logs.len() + 1
        ),
        timestamp: chrono::Utc::now().to_rfc3339(),
        source: source.to_string(),
        tool_name: tool_name.to_string(),
        args_preview,
        ok,
        summary,
    };

    state.tool_logs.push(entry);
    if state.tool_logs.len() > MAX_TOOL_LOGS {
        let excess = state.tool_logs.len() - MAX_TOOL_LOGS;
        state.tool_logs.drain(0..excess);
    }
}

/// Restores `state.case` from the specified `target_index` or from `state.best_iteration_index`.
pub fn rollback_to_iteration(
    state: &mut SessionState,
    target_index: Option<u32>,
) -> Result<u32, VenturiError> {
    let idx = target_index.or(state.best_iteration_index).ok_or_else(|| {
        VenturiError::InvalidCase(
            "No target or best iteration available to roll back to.".into(),
        )
    })?;

    let snapshot = state
        .iterations
        .iter()
        .find(|rec| rec.index == idx)
        .map(|rec| rec.case_snapshot.clone())
        .ok_or_else(|| {
            VenturiError::InvalidCase(format!(
                "Iteration #{} not found in session history.",
                idx
            ))
        })?;

    state.case = snapshot;
    Ok(idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CheckMeshSummary, IterationRecord, OpenFoamResultSummary, RefinementBox};

    #[test]
    fn all_six_presets_initialize_cleanly_within_guardrails() {
        let guardrails = GuardrailsConfig::default();
        for id in [
            "naca0012",
            "ahmed_body",
            "venturi_nozzle",
            "cylinder_vortex",
            "lid_cavity",
            "empty",
        ] {
            let case = default_preset_case(id);
            enforce_mesh_guardrails(&case, &guardrails)
                .unwrap_or_else(|e| panic!("preset '{}' failed guardrails: {}", id, e));
        }

        let empty = default_preset_case("empty");
        assert_eq!(empty.geometry_kind, PresetGeometryKind::None);
        assert!(empty.parametric.is_none());
        assert!(empty.stl.is_none());
    }

    #[test]
    fn guardrails_reject_excessive_cells_and_refinement_level() {
        let guardrails = GuardrailsConfig::default();
        let mut case = default_preset_case("naca0012");
        case.mesh.cells_x = 400;
        case.mesh.cells_y = 200;
        case.mesh.cells_z = 50; // 4M base cells > 2M max_cells
        assert!(enforce_mesh_guardrails(&case, &guardrails).is_err());

        let mut box_case = default_preset_case("naca0012");
        box_case.mesh.refinement_boxes.push(RefinementBox {
            name: "too_fine".into(),
            min: [0.0, -0.5, -0.3],
            max: [2.0, 0.5, 0.3],
            level: 6, // > default max_refinement_level 4
            reason: "test".into(),
        });
        assert!(enforce_mesh_guardrails(&box_case, &guardrails).is_err());
    }

    #[test]
    fn atomic_save_and_load_roundtrip_increments_version() {
        let path = std::env::temp_dir().join(format!(
            "venturi_session_test_{}.json",
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let mut state = load_or_init_session(&path);
        assert_eq!(state.version, 1);

        save_session_atomic(&path, &mut state, "unit-test").unwrap();
        assert_eq!(state.version, 2);
        assert_eq!(state.last_writer, "unit-test");

        let reloaded = load_or_init_session(&path);
        assert_eq!(reloaded.version, 2);
        assert_eq!(reloaded.last_writer, "unit-test");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tool_log_caps_at_120_and_rollback_restores_case() {
        let path = std::env::temp_dir().join("venturi_nonexistent_test_session.json");
        let mut state = load_or_init_session(&path);
        for i in 0..145 {
            append_tool_log(
                &mut state,
                "mcp-external",
                "venturi_get_state",
                &serde_json::json!({ "i": i }),
                true,
                format!("call {}", i),
            );
        }
        assert_eq!(state.tool_logs.len(), 120);
        assert_eq!(state.tool_logs.last().unwrap().summary, "call 144");

        let mut snap = default_preset_case("naca0012");
        snap.name = "best_snapshot_case".into();
        state.iterations.push(IterationRecord {
            index: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            label: "iter 1".into(),
            changes_summary: "baseline".into(),
            case_snapshot: snap,
            results: OpenFoamResultSummary {
                backend_used: "demo".into(),
                converged: true,
                diverged: false,
                iterations_completed: 100,
                final_residual_u: 1e-5,
                final_residual_p: 2e-5,
                continuity_error: 1e-8,
                cd: Some(0.02),
                cl: Some(0.4),
                cm: Some(-0.01),
                delta_p_pa: Some(120.0),
                check_mesh: CheckMeshSummary {
                    cells: 150_000,
                    max_non_orthogonality: 22.0,
                    avg_non_orthogonality: 6.0,
                    max_skewness: 0.7,
                    max_aspect_ratio: 12.0,
                    failed_checks: 0,
                    mesh_ok: true,
                },
                y_plus_patches: Vec::new(),
                hotspots: Vec::new(),
                notes: Vec::new(),
            },
            objective_score: 88.5,
            is_best: true,
            rolled_back: false,
        });
        state.best_iteration_index = Some(1);
        state.case.name = "mutated_bad_case".into();

        let idx = rollback_to_iteration(&mut state, None).unwrap();
        assert_eq!(idx, 1);
        assert_eq!(state.case.name, "best_snapshot_case");
    }
}
