use crate::error::VenturiError;
use crate::models::{
    BoundaryKind, BoundaryPatch, CfdCase, DomainBox, DomainFace, FlowRegime, FluidConfig,
    MeshConfig, NumericsConfig, OpenFoamFlavor, PhysicsConfig, PresetGeometryKind, SchemeAccuracy,
    TurbulenceIC, TurbulenceModel,
};
use regex::Regex;
use std::path::Path;

/// Reads an existing OpenFOAM case directory from disk and maps key dictionaries into a CfdCase.
pub fn import_case_from_disk(dir: &Path) -> Result<CfdCase, VenturiError> {
    let control_path = dir.join("system/controlDict");
    if !control_path.exists() {
        return Err(VenturiError::InvalidCase(
            "Directory does not contain system/controlDict".into(),
        ));
    }

    let control = std::fs::read_to_string(&control_path)?;
    let case_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("imported_case")
        .to_string();

    let flavor = if dir.join("constant/physicalProperties").exists()
        || control.contains("foamRun")
    {
        OpenFoamFlavor::FoundationV14
    } else {
        OpenFoamFlavor::OpenCfdV2606
    };

    let end_time = extract_scalar(&control, "endTime").unwrap_or(500.0);
    let delta_t = extract_scalar(&control, "deltaT").unwrap_or(1.0);
    let write_interval = extract_scalar(&control, "writeInterval").unwrap_or(100.0);
    let start_time = extract_scalar(&control, "startTime").unwrap_or(0.0);

    let regime = if control.contains("pimpleFoam")
        || control.contains("pisoFoam")
        || control.contains("icoFoam")
        || delta_t < 0.1
    {
        FlowRegime::TransientIncompressible
    } else if control.contains("rhoSimpleFoam") || control.contains("solver          fluid") {
        FlowRegime::SteadyCompressible
    } else {
        FlowRegime::SteadyIncompressible
    };

    // Fluid viscosity
    let props_text = std::fs::read_to_string(dir.join("constant/physicalProperties"))
        .or_else(|_| std::fs::read_to_string(dir.join("constant/transportProperties")))
        .unwrap_or_default();
    let nu = extract_nu(&props_text).unwrap_or(1.506e-5);
    let density = 1.204;

    // Turbulence model
    let turb_text = std::fs::read_to_string(dir.join("constant/momentumTransport"))
        .or_else(|_| std::fs::read_to_string(dir.join("constant/turbulenceProperties")))
        .unwrap_or_default();
    let turbulence = if turb_text.contains("kOmegaSST") {
        TurbulenceModel::KOmegaSST
    } else if turb_text.contains("kEpsilon") {
        TurbulenceModel::KEpsilon
    } else if turb_text.contains("SpalartAllmaras") {
        TurbulenceModel::SpalartAllmaras
    } else if turb_text.contains("WALE") || turb_text.contains("LES") {
        TurbulenceModel::WaleLes
    } else {
        TurbulenceModel::Laminar
    };

    // Parse domain & cells from blockMeshDict if present
    let (domain, cells) = std::fs::read_to_string(dir.join("system/blockMeshDict"))
        .ok()
        .map(|txt| parse_block_mesh_summary(&txt))
        .unwrap_or((
            DomainBox {
                x_min: -2.0,
                x_max: 6.0,
                y_min: -2.0,
                y_max: 2.0,
                z_min: -0.5,
                z_max: 0.5,
            },
            (80, 40, 8),
        ));

    // Parse inlet velocity & boundaries from 0/U if present
    let u_text = std::fs::read_to_string(dir.join("0/U")).unwrap_or_default();
    let (boundaries, ref_vel) = parse_boundaries_from_u(&u_text);
    let snappy_enabled = dir.join("system/snappyHexMeshDict").exists();

    Ok(CfdCase {
        name: case_name,
        description: format!("Imported OpenFOAM case from {}", dir.display()),
        flavor,
        geometry_kind: if snappy_enabled {
            PresetGeometryKind::CustomStl
        } else {
            PresetGeometryKind::None
        },
        parametric: None,
        stl: None,
        domain,
        mesh: MeshConfig {
            cells_x: cells.0,
            cells_y: cells.1,
            cells_z: cells.2,
            grading_y: 1.0,
            snappy_enabled,
            refinement_min: 2,
            refinement_max: 3,
            inflation_layers: 6,
            first_layer_thickness: 0.0005,
            expansion_ratio: 1.2,
            target_y_plus: 30.0,
            refinement_boxes: Vec::new(),
        },
        physics: PhysicsConfig {
            regime,
            turbulence,
            fluid: FluidConfig {
                name: format!("Imported Fluid (ν = {:.2e})", nu),
                density,
                dynamic_viscosity: nu * density,
                kinematic_viscosity: nu,
                temperature_k: 293.15,
                pressure_pa: 101_325.0,
                gamma: 1.4,
                gas_constant: 287.058,
                is_gas: nu > 5e-6,
            },
            reference_velocity: ref_vel.max(0.1),
            reference_length: 1.0,
            turbulence_ic: TurbulenceIC {
                intensity: 0.05,
                length_scale: 0.07,
                k: 1.5 * (ref_vel * 0.05).powi(2),
                epsilon: 0.1,
                omega: 10.0,
                nut: 1e-3,
                nu_tilda: 1e-3,
            },
            gravity_enabled: dir.join("constant/g").exists(),
            gravity: [0.0, -9.81, 0.0],
        },
        boundaries,
        numerics: NumericsConfig {
            accuracy: SchemeAccuracy::BalancedSecondOrder,
            start_time,
            end_time,
            delta_t,
            write_interval,
            relax_p: 0.3,
            relax_u: 0.7,
            relax_turb: 0.7,
            convergence_tol: 1e-4,
            max_co: 0.8,
            n_non_ortho_correctors: 1,
            parallel_cores: 1,
        },
    })
}

fn extract_scalar(text: &str, key: &str) -> Option<f64> {
    let pattern = format!(r"(?m)^\s*{}\s+([0-9eE.+\-]+)\s*;", regex::escape(key));
    let re = Regex::new(&pattern).ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

fn extract_nu(text: &str) -> Option<f64> {
    let re = Regex::new(r"(?m)^\s*nu\s+(?:\[[^\]]*\]\s+)?([0-9eE.+\-]+)\s*;").ok()?;
    re.captures(text)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse::<f64>().ok())
}

fn parse_block_mesh_summary(text: &str) -> (DomainBox, (u32, u32, u32)) {
    let vert_re = Regex::new(r"\(\s*([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s*\)").unwrap();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let mut zs = Vec::new();

    if let Some(start) = text.find("vertices") {
        let slice = &text[start..text.len().min(start + 1200)];
        for cap in vert_re.captures_iter(slice).take(8) {
            if let (Ok(x), Ok(y), Ok(z)) = (
                cap[1].parse::<f64>(),
                cap[2].parse::<f64>(),
                cap[3].parse::<f64>(),
            ) {
                xs.push(x);
                ys.push(y);
                zs.push(z);
            }
        }
    }

    let domain = if !xs.is_empty() {
        DomainBox {
            x_min: xs.iter().cloned().fold(f64::INFINITY, f64::min),
            x_max: xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            y_min: ys.iter().cloned().fold(f64::INFINITY, f64::min),
            y_max: ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            z_min: zs.iter().cloned().fold(f64::INFINITY, f64::min),
            z_max: zs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        }
    } else {
        DomainBox {
            x_min: -2.0,
            x_max: 6.0,
            y_min: -2.0,
            y_max: 2.0,
            z_min: -0.5,
            z_max: 0.5,
        }
    };

    let cell_re = Regex::new(r"hex\s*\([^)]+\)\s*\(\s*(\d+)\s+(\d+)\s+(\d+)\s*\)").unwrap();
    let cells = cell_re
        .captures(text)
        .and_then(|c| {
            Some((
                c[1].parse::<u32>().ok()?,
                c[2].parse::<u32>().ok()?,
                c[3].parse::<u32>().ok()?,
            ))
        })
        .unwrap_or((60, 30, 6));

    (domain, cells)
}

fn parse_boundaries_from_u(text: &str) -> (Vec<BoundaryPatch>, f64) {
    let mut patches = Vec::new();
    let mut ref_vel = 10.0_f64;

    if let Some(bf_idx) = text.find("boundaryField") {
        let body = &text[bf_idx..];
        let patch_re = Regex::new(r"(?s)\n\s*([A-Za-z0-9_.\-]+)\s*\{([^{}]*)\}").unwrap();
        let faces = [
            DomainFace::XMin,
            DomainFace::XMax,
            DomainFace::YMin,
            DomainFace::YMax,
            DomainFace::ZMin,
            DomainFace::ZMax,
            DomainFace::Obstacle,
        ];
        for (idx, cap) in patch_re.captures_iter(body).enumerate() {
            let name = cap[1].to_string();
            let block = &cap[2];
            let kind = if block.contains("empty") {
                BoundaryKind::Empty2D
            } else if block.contains("symmetry") {
                BoundaryKind::Symmetry
            } else if block.contains("noSlip") {
                BoundaryKind::NoSlipWall
            } else if block.contains("zeroGradient") || block.contains("inletOutlet") {
                BoundaryKind::PressureOutlet
            } else {
                BoundaryKind::VelocityInlet
            };

            let vec_re = Regex::new(r"uniform\s*\(\s*([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s+([0-9eE.+\-]+)\s*\)").unwrap();
            let velocity = vec_re
                .captures(block)
                .and_then(|v| {
                    Some([
                        v[1].parse::<f64>().ok()?,
                        v[2].parse::<f64>().ok()?,
                        v[3].parse::<f64>().ok()?,
                    ])
                })
                .unwrap_or([0.0, 0.0, 0.0]);

            let speed = (velocity[0].powi(2) + velocity[1].powi(2) + velocity[2].powi(2)).sqrt();
            if kind == BoundaryKind::VelocityInlet && speed > 0.0 {
                ref_vel = speed;
            }

            let face = if name.to_lowercase().contains("inlet") {
                DomainFace::XMin
            } else if name.to_lowercase().contains("outlet") {
                DomainFace::XMax
            } else {
                faces[idx.min(faces.len() - 1)]
            };

            patches.push(BoundaryPatch {
                name,
                face,
                kind,
                velocity,
                pressure: 0.0,
                wall_roughness_ks: 0.0,
            });
        }
    }

    if patches.is_empty() {
        patches = vec![
            BoundaryPatch {
                name: "inlet".into(),
                face: DomainFace::XMin,
                kind: BoundaryKind::VelocityInlet,
                velocity: [10.0, 0.0, 0.0],
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
        ];
    }

    (patches, ref_vel)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openfoam::generator::{export_case_to_disk, tests::sample_case};

    #[test]
    fn roundtrip_export_and_import_case() {
        let case = sample_case();
        let tmp = std::env::temp_dir().join(format!(
            "venturi_roundtrip_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let written = export_case_to_disk(&case, &tmp).unwrap();
        assert!(written >= 12);

        let imported = import_case_from_disk(&tmp).unwrap();
        assert_eq!(imported.flavor, OpenFoamFlavor::FoundationV14);
        assert_eq!(imported.physics.turbulence, TurbulenceModel::KOmegaSST);
        assert_eq!(imported.mesh.cells_x, case.mesh.cells_x);
        assert_eq!(imported.mesh.cells_y, case.mesh.cells_y);
        assert!((imported.physics.reference_velocity - 25.0).abs() < 1e-3);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}

