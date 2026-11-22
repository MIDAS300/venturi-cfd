use crate::error::VenturiError;
use crate::models::{ParametricGeometrySpec, StlMeshPayload};
use crate::stl::parse_stl_bytes;
use std::f64::consts::PI;

type Vec3 = [f64; 3];
type Tri = [Vec3; 3];

/// Returns a clean OpenFOAM-safe STL filename for the given parametric geometry family.
pub fn suggested_filename(spec: &ParametricGeometrySpec) -> String {
    match spec {
        ParametricGeometrySpec::NacaAirfoil { .. } => "naca_airfoil.stl".to_string(),
        ParametricGeometrySpec::VenturiTube { .. } => "venturi_tube.stl".to_string(),
        ParametricGeometrySpec::AhmedDiffuser { .. } => "ahmed_diffuser.stl".to_string(),
        ParametricGeometrySpec::PinFinArray { .. } => "pin_fin_array.stl".to_string(),
        ParametricGeometrySpec::Cylinder { .. } => "cylinder.stl".to_string(),
    }
}

/// Returns the canonical aerodynamic / hydraulic reference length (m) for Reynolds & y+ sizing.
pub fn reference_length(spec: &ParametricGeometrySpec) -> f64 {
    match spec {
        ParametricGeometrySpec::NacaAirfoil { chord, .. } => chord.abs().max(1e-4),
        ParametricGeometrySpec::VenturiTube { throat_radius, .. } => {
            (2.0 * throat_radius.abs()).max(1e-4)
        }
        ParametricGeometrySpec::AhmedDiffuser { length, .. } => length.abs().max(1e-4),
        ParametricGeometrySpec::PinFinArray { base_length, .. } => base_length.abs().max(1e-4),
        ParametricGeometrySpec::Cylinder { radius, .. } => (2.0 * radius.abs()).max(1e-4),
    }
}

/// Generates a watertight, manifold 3D triangle mesh for the parametric specification,
/// encodes it into Binary STL bytes, and parses it into a viewport-ready `StlMeshPayload`.
pub fn generate_parametric_stl(
    spec: &ParametricGeometrySpec,
) -> Result<(StlMeshPayload, Vec<u8>), VenturiError> {
    let filename = suggested_filename(spec);
    let mut tris = Vec::new();

    match spec {
        ParametricGeometrySpec::NacaAirfoil {
            chord,
            camber_pct,
            camber_pos_pct,
            thickness_pct,
            aoa_deg,
            span,
        } => {
            build_naca_airfoil(
                *chord,
                *camber_pct,
                *camber_pos_pct,
                *thickness_pct,
                *aoa_deg,
                *span,
                &mut tris,
            )?;
        }
        ParametricGeometrySpec::VenturiTube {
            inlet_radius,
            throat_radius,
            outlet_radius,
            convergent_len,
            throat_len,
            divergent_len,
        } => {
            build_venturi_tube(
                *inlet_radius,
                *throat_radius,
                *outlet_radius,
                *convergent_len,
                *throat_len,
                *divergent_len,
                &mut tris,
            )?;
        }
        ParametricGeometrySpec::AhmedDiffuser {
            length,
            width,
            height,
            rear_slant_deg,
            ground_clearance,
        } => {
            build_ahmed_diffuser(
                *length,
                *width,
                *height,
                *rear_slant_deg,
                *ground_clearance,
                &mut tris,
            )?;
        }
        ParametricGeometrySpec::PinFinArray {
            base_length,
            base_width,
            fin_count_x,
            fin_count_z,
            fin_radius,
            fin_height,
        } => {
            build_pin_fin_array(
                *base_length,
                *base_width,
                *fin_count_x,
                *fin_count_z,
                *fin_radius,
                *fin_height,
                &mut tris,
            )?;
        }
        ParametricGeometrySpec::Cylinder { radius, span } => {
            build_cylinder(*radius, *span, &mut tris)?;
        }
    }

    if tris.is_empty() {
        return Err(VenturiError::InvalidStl(
            "Parametric generator produced zero triangles".into(),
        ));
    }

    let bytes = encode_binary_stl(&tris, &filename);
    let payload = parse_stl_bytes(&bytes, &filename)?;
    Ok((payload, bytes))
}

fn validate_positive(name: &str, v: f64) -> Result<(), VenturiError> {
    if !v.is_finite() || v <= 0.0 {
        return Err(VenturiError::PhysicsBounds(format!(
            "Parametric parameter '{}' must be finite and strictly positive (got {})",
            name, v
        )));
    }
    Ok(())
}

fn validate_range(name: &str, v: f64, min: f64, max: f64) -> Result<(), VenturiError> {
    if !v.is_finite() || v < min || v > max {
        return Err(VenturiError::PhysicsBounds(format!(
            "Parametric parameter '{}' must be in [{}, {}] (got {})",
            name, min, max, v
        )));
    }
    Ok(())
}

fn push_tri(tris: &mut Vec<Tri>, a: Vec3, b: Vec3, c: Vec3) {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cross = [
        ab[1] * ac[2] - ab[2] * ac[1],
        ab[2] * ac[0] - ab[0] * ac[2],
        ab[0] * ac[1] - ab[1] * ac[0],
    ];
    let mag2 = cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2];
    if mag2 > 1e-24 && mag2.is_finite() {
        tris.push([a, b, c]);
    }
}

fn compute_normal_f32(a: Vec3, b: Vec3, c: Vec3) -> [f32; 3] {
    let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let nx = ab[1] * ac[2] - ab[2] * ac[1];
    let ny = ab[2] * ac[0] - ab[0] * ac[2];
    let nz = ab[0] * ac[1] - ab[1] * ac[0];
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-12);
    [(nx / len) as f32, (ny / len) as f32, (nz / len) as f32]
}

fn encode_binary_stl(tris: &[Tri], label: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(84 + tris.len() * 50);

    // 80-byte binary STL header (must never begin with "solid")
    let mut header = [0u8; 80];
    let text = format!("Venturi CFD Parametric STL ({})", label);
    let raw = text.as_bytes();
    let copy_len = raw.len().min(80);
    header[..copy_len].copy_from_slice(&raw[..copy_len]);
    buf.extend_from_slice(&header);

    let count = tris.len() as u32;
    buf.extend_from_slice(&count.to_le_bytes());

    for tri in tris {
        let n = compute_normal_f32(tri[0], tri[1], tri[2]);
        for c in n {
            buf.extend_from_slice(&c.to_le_bytes());
        }
        for v in tri {
            for coord in v {
                buf.extend_from_slice(&(*coord as f32).to_le_bytes());
            }
        }
        buf.extend_from_slice(&0u16.to_le_bytes());
    }

    buf
}

/// Extrudes a strictly convex 2D counter-clockwise polygon in XY along Z in `[z0, z1]`,
/// generating outward-oriented side walls and watertight end caps.
fn extrude_convex_polygon_z(poly_ccw: &[[f64; 2]], z0: f64, z1: f64, tris: &mut Vec<Tri>) {
    let n = poly_ccw.len();
    if n < 3 {
        return;
    }

    let mut cx = 0.0;
    let mut cy = 0.0;
    for p in poly_ccw {
        cx += p[0];
        cy += p[1];
    }
    cx /= n as f64;
    cy /= n as f64;

    let c0 = [cx, cy, z0];
    let c1 = [cx, cy, z1];

    for i in 0..n {
        let p0 = poly_ccw[i];
        let p1 = poly_ccw[(i + 1) % n];

        let a = [p0[0], p0[1], z0];
        let b = [p1[0], p1[1], z0];
        let c = [p1[0], p1[1], z1];
        let d = [p0[0], p0[1], z1];

        // Side wall (outward normal = (p1 - p0) x +Z)
        push_tri(tris, a, b, c);
        push_tri(tris, a, c, d);

        // +Z cap (outward normal = +Z)
        push_tri(tris, c1, d, c);

        // -Z cap (outward normal = -Z)
        push_tri(tris, c0, b, a);
    }
}

/// True NACA 4-digit cambered airfoil with closed trailing edge, rotated by `-aoa_deg`
/// around quarter-chord `(0, 0)` and extruded along Z in `[-span/2, +span/2]`.
fn build_naca_airfoil(
    chord: f64,
    camber_pct: f64,
    camber_pos_pct: f64,
    thickness_pct: f64,
    aoa_deg: f64,
    span: f64,
    tris: &mut Vec<Tri>,
) -> Result<(), VenturiError> {
    validate_positive("chord", chord)?;
    validate_positive("span", span)?;
    validate_range("thickness_pct", thickness_pct, 1.0, 45.0)?;
    validate_range("camber_pct", camber_pct, -15.0, 15.0)?;
    validate_range("camber_pos_pct", camber_pos_pct, 5.0, 95.0)?;
    validate_range("aoa_deg", aoa_deg, -45.0, 45.0)?;

    let n: usize = 64;
    let m = camber_pct / 100.0;
    let p = (camber_pos_pct / 100.0).clamp(0.05, 0.95);
    let t = thickness_pct / 100.0;

    // Positive AoA tilts leading edge (+Y) up and trailing edge (-Y) down -> rotation by -aoa_deg
    let alpha = -aoa_deg.to_radians();
    let cos_a = alpha.cos();
    let sin_a = alpha.sin();

    let transform = |x_norm: f64, y_norm: f64| -> [f64; 2] {
        let dx = (x_norm - 0.25) * chord;
        let dy = y_norm * chord;
        [dx * cos_a - dy * sin_a, dx * sin_a + dy * cos_a]
    };

    let mut upper = Vec::with_capacity(n + 1);
    let mut lower = Vec::with_capacity(n + 1);

    for i in 0..=n {
        if i == 0 {
            let le = transform(0.0, 0.0);
            upper.push(le);
            lower.push(le);
            continue;
        }
        if i == n {
            let te = transform(1.0, 0.0);
            upper.push(te);
            lower.push(te);
            continue;
        }

        // Cosine clustering concentrates resolution at leading and trailing edges
        let beta = PI * (i as f64) / (n as f64);
        let xc = 0.5 * (1.0 - beta.cos());

        // Closed-trailing-edge NACA 4-digit half-thickness (-0.1036 ensures yt(1) = 0)
        let yt = 5.0
            * t
            * (0.2969 * xc.sqrt()
                - 0.1260 * xc
                - 0.3516 * xc.powi(2)
                + 0.2843 * xc.powi(3)
                - 0.1036 * xc.powi(4));

        let (yc, dyc_dx) = if m.abs() < 1e-9 {
            (0.0, 0.0)
        } else if xc < p {
            let yc = (m / (p * p)) * (2.0 * p * xc - xc * xc);
            let dyc = (2.0 * m / (p * p)) * (p - xc);
            (yc, dyc)
        } else {
            let om = 1.0 - p;
            let yc = (m / (om * om)) * ((1.0 - 2.0 * p) + 2.0 * p * xc - xc * xc);
            let dyc = (2.0 * m / (om * om)) * (p - xc);
            (yc, dyc)
        };

        let theta = dyc_dx.atan();
        let sin_t = theta.sin();
        let cos_t = theta.cos();

        upper.push(transform(xc - yt * sin_t, yc + yt * cos_t));
        lower.push(transform(xc + yt * sin_t, yc - yt * cos_t));
    }

    let z0 = -0.5 * span;
    let z1 = 0.5 * span;

    // 1. Watertight ribbon triangulation for both end caps (handles arbitrary camber cleanly)
    let p_z0 = |pt: [f64; 2]| [pt[0], pt[1], z0];
    let p_z1 = |pt: [f64; 2]| [pt[0], pt[1], z1];

    // Leading-edge cap triangle
    push_tri(tris, p_z1(upper[0]), p_z1(lower[1]), p_z1(upper[1]));
    push_tri(tris, p_z0(upper[0]), p_z0(upper[1]), p_z0(lower[1]));

    // Interior upper-lower station quads
    for i in 1..(n - 1) {
        let l0 = lower[i];
        let l1 = lower[i + 1];
        let u0 = upper[i];
        let u1 = upper[i + 1];

        // +Z cap (CCW -> +Z)
        push_tri(tris, p_z1(l0), p_z1(l1), p_z1(u1));
        push_tri(tris, p_z1(l0), p_z1(u1), p_z1(u0));

        // -Z cap (CW -> -Z)
        push_tri(tris, p_z0(l0), p_z0(u1), p_z0(l1));
        push_tri(tris, p_z0(l0), p_z0(u0), p_z0(u1));
    }

    // Trailing-edge cap triangle
    push_tri(tris, p_z1(lower[n - 1]), p_z1(upper[n]), p_z1(upper[n - 1]));
    push_tri(tris, p_z0(lower[n - 1]), p_z0(upper[n - 1]), p_z0(upper[n]));

    // 2. CCW perimeter ring for side skin matching cap boundary edges exactly
    let mut ring = Vec::with_capacity(2 * n);
    ring.push(upper[0]);
    for pt in &lower[1..n] {
        ring.push(*pt);
    }
    ring.push(upper[n]);
    for pt in upper[1..n].iter().rev() {
        ring.push(*pt);
    }

    let m_ring = ring.len();
    for k in 0..m_ring {
        let q0 = ring[k];
        let q1 = ring[(k + 1) % m_ring];
        let a = [q0[0], q0[1], z0];
        let b = [q1[0], q1[1], z0];
        let c = [q1[0], q1[1], z1];
        let d = [q0[0], q0[1], z1];
        push_tri(tris, a, b, c);
        push_tri(tris, a, c, d);
    }

    Ok(())
}

/// Watertight annular thin-wall convergent-divergent Venturi tube revolved around the X axis,
/// centered at `X = 0` with smooth cosine transitions at the throat.
fn build_venturi_tube(
    inlet_radius: f64,
    throat_radius: f64,
    outlet_radius: f64,
    convergent_len: f64,
    throat_len: f64,
    divergent_len: f64,
    tris: &mut Vec<Tri>,
) -> Result<(), VenturiError> {
    validate_positive("inlet_radius", inlet_radius)?;
    validate_positive("throat_radius", throat_radius)?;
    validate_positive("outlet_radius", outlet_radius)?;
    validate_positive("convergent_len", convergent_len)?;
    validate_positive("throat_len", throat_len)?;
    validate_positive("divergent_len", divergent_len)?;

    let total_len = convergent_len + throat_len + divergent_len;
    let x_start = -0.5 * total_len;
    let x1 = x_start + convergent_len;
    let x2 = x1 + throat_len;

    let max_r = inlet_radius.max(outlet_radius).max(throat_radius);
    let wall_t = (0.06 * max_r).clamp(0.004, 0.05);

    let n_conv: usize = 16;
    let n_throat: usize = 8;
    let n_div: usize = 24;
    let n_theta: usize = 48;

    let smoothstep = |u: f64| 0.5 * (1.0 - (PI * u).cos());

    let mut axial: Vec<(f64, f64)> = Vec::with_capacity(n_conv + n_throat + n_div + 1);
    for i in 0..=n_conv {
        let u = (i as f64) / (n_conv as f64);
        let x = x_start + u * convergent_len;
        let r = inlet_radius + (throat_radius - inlet_radius) * smoothstep(u);
        axial.push((x, r));
    }
    for i in 1..=n_throat {
        let u = (i as f64) / (n_throat as f64);
        let x = x1 + u * throat_len;
        axial.push((x, throat_radius));
    }
    for i in 1..=n_div {
        let u = (i as f64) / (n_div as f64);
        let x = x2 + u * divergent_len;
        let r = throat_radius + (outlet_radius - throat_radius) * smoothstep(u);
        axial.push((x, r));
    }

    // Closed (x, r) meridional cross-section loop:
    // outer wall (inlet -> outlet), outlet annular end cap, inner wall (outlet -> inlet), inlet cap.
    let mut profile: Vec<(f64, f64)> = Vec::with_capacity(axial.len() * 2);
    for &(x, r_in) in &axial {
        profile.push((x, r_in + wall_t));
    }
    for &(x, r_in) in axial.iter().rev() {
        profile.push((x, r_in));
    }

    let n_prof = profile.len();
    let cos_sin: Vec<(f64, f64)> = (0..n_theta)
        .map(|j| {
            let phi = 2.0 * PI * (j as f64) / (n_theta as f64);
            (phi.cos(), phi.sin())
        })
        .collect();

    let pt3 = |p: (f64, f64), j: usize| -> Vec3 {
        let (c, s) = cos_sin[j];
        [p.0, p.1 * c, p.1 * s]
    };

    for i in 0..n_prof {
        let p0 = profile[i];
        let p1 = profile[(i + 1) % n_prof];
        for j in 0..n_theta {
            let j_next = (j + 1) % n_theta;
            let v00 = pt3(p0, j);
            let v10 = pt3(p1, j);
            let v11 = pt3(p1, j_next);
            let v01 = pt3(p0, j_next);
            // Outward orientation verified for outer wall (+r), caps (+X/-X), and inner bore (-r)
            push_tri(tris, v00, v01, v11);
            push_tri(tris, v00, v11, v10);
        }
    }

    Ok(())
}

/// ERCOFTAC-style Ahmed body with rounded nose, exact `rear_slant_deg` fastback bevel,
/// extruded symmetrically in Z (`[-width/2, +width/2]`) and raised by `ground_clearance`.
fn build_ahmed_diffuser(
    length: f64,
    width: f64,
    height: f64,
    rear_slant_deg: f64,
    ground_clearance: f64,
    tris: &mut Vec<Tri>,
) -> Result<(), VenturiError> {
    validate_positive("length", length)?;
    validate_positive("width", width)?;
    validate_positive("height", height)?;
    validate_range("rear_slant_deg", rear_slant_deg, 0.0, 60.0)?;
    validate_range("ground_clearance", ground_clearance, 0.0, 100.0)?;

    let y_bot = ground_clearance;
    let y_top = ground_clearance + height;
    let r_front = (0.30 * height).min(0.20 * length);

    // Exact rear slant geometry: slant horizontal run ~ 21.3% of length (ERCOFTAC = 0.222 / 1.044)
    let tan_s = rear_slant_deg.to_radians().tan();
    let max_drop = 0.78 * height;
    let max_run = (length - r_front * 1.25).max(0.05 * length);
    let (slant_run, slant_drop) = if tan_s > 1e-6 {
        let nom_run = (0.213 * length).min(max_run);
        let drop = (nom_run * tan_s).min(max_drop);
        let run = drop / tan_s;
        (run, drop)
    } else {
        (0.0, 0.0)
    };

    let arc_segs: usize = 8;
    let mut poly: Vec<[f64; 2]> = Vec::new();
    let push_unique = |poly: &mut Vec<[f64; 2]>, pt: [f64; 2]| {
        if let Some(last) = poly.last() {
            if (last[0] - pt[0]).hypot(last[1] - pt[1]) < 1e-9 {
                return;
            }
        }
        poly.push(pt);
    };

    // CCW side profile in XY plane
    push_unique(&mut poly, [r_front, y_bot]);
    push_unique(&mut poly, [length, y_bot]);
    push_unique(&mut poly, [length, y_top - slant_drop]);
    if slant_run > 1e-7 {
        push_unique(&mut poly, [length - slant_run, y_top]);
    }
    push_unique(&mut poly, [r_front, y_top]);

    // Front upper quarter-circle arc (PI/2 -> PI)
    let c_up = [r_front, y_top - r_front];
    for k in 1..=arc_segs {
        let theta = 0.5 * PI + 0.5 * PI * (k as f64) / (arc_segs as f64);
        push_unique(&mut poly, [c_up[0] + r_front * theta.cos(), c_up[1] + r_front * theta.sin()]);
    }

    // Front lower quarter-circle arc (PI -> 3*PI/2)
    let c_lo = [r_front, y_bot + r_front];
    for k in 0..arc_segs {
        let theta = PI + 0.5 * PI * (k as f64) / (arc_segs as f64);
        push_unique(&mut poly, [c_lo[0] + r_front * theta.cos(), c_lo[1] + r_front * theta.sin()]);
    }

    let z0 = -0.5 * width;
    let z1 = 0.5 * width;
    extrude_convex_polygon_z(&poly, z0, z1, tris);

    Ok(())
}

/// Solid rectangular heat-sink base plate (resting on `y = 0`) topped by a uniform
/// `fin_count_x × fin_count_z` array of closed cylindrical pin fins rising along `+Y`.
fn build_pin_fin_array(
    base_length: f64,
    base_width: f64,
    fin_count_x: u32,
    fin_count_z: u32,
    fin_radius: f64,
    fin_height: f64,
    tris: &mut Vec<Tri>,
) -> Result<(), VenturiError> {
    validate_positive("base_length", base_length)?;
    validate_positive("base_width", base_width)?;
    validate_positive("fin_radius", fin_radius)?;
    validate_positive("fin_height", fin_height)?;
    if fin_count_x == 0 || fin_count_x > 64 || fin_count_z == 0 || fin_count_z > 64 {
        return Err(VenturiError::PhysicsBounds(format!(
            "Pin-fin counts must be in 1..=64 (got {}x{})",
            fin_count_x, fin_count_z
        )));
    }

    let base_t = (0.18 * fin_height).min(0.05).max(1e-4);
    let x0 = -0.5 * base_length;
    let x1 = 0.5 * base_length;
    let y0 = 0.0;
    let y1 = base_t;
    let z0 = -0.5 * base_width;
    let z1 = 0.5 * base_width;

    // 1. Watertight base plate box
    push_box(tris, [x0, y0, z0], [x1, y1, z1]);

    // 2. Grid of closed cylindrical pin fins rising in +Y
    let fin_y0 = base_t;
    let fin_y1 = base_t + fin_height;
    let n_seg: usize = 24;

    let centers_1d = |span_len: f64, count: u32| -> Vec<f64> {
        if count == 1 {
            return vec![0.0];
        }
        let margin = (span_len / (2.0 * count as f64))
            .max(fin_radius)
            .min(span_len * 0.48);
        let usable = (span_len - 2.0 * margin).max(0.0);
        (0..count)
            .map(|i| -0.5 * usable + (i as f64 / (count - 1) as f64) * usable)
            .collect()
    };

    let xs = centers_1d(base_length, fin_count_x);
    let zs = centers_1d(base_width, fin_count_z);

    for &cx in &xs {
        for &cz in &zs {
            push_cylinder_y(tris, cx, cz, fin_y0, fin_y1, fin_radius, n_seg);
        }
    }

    Ok(())
}

fn push_box(tris: &mut Vec<Tri>, min: Vec3, max: Vec3) {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;

    let v000 = [x0, y0, z0];
    let v100 = [x1, y0, z0];
    let v110 = [x1, y1, z0];
    let v010 = [x0, y1, z0];
    let v001 = [x0, y0, z1];
    let v101 = [x1, y0, z1];
    let v111 = [x1, y1, z1];
    let v011 = [x0, y1, z1];

    // -Z face
    push_tri(tris, v000, v010, v110);
    push_tri(tris, v000, v110, v100);
    // +Z face
    push_tri(tris, v001, v101, v111);
    push_tri(tris, v001, v111, v011);
    // -Y face
    push_tri(tris, v000, v100, v101);
    push_tri(tris, v000, v101, v001);
    // +Y face
    push_tri(tris, v010, v011, v111);
    push_tri(tris, v010, v111, v110);
    // -X face
    push_tri(tris, v000, v001, v011);
    push_tri(tris, v000, v011, v010);
    // +X face
    push_tri(tris, v100, v110, v111);
    push_tri(tris, v100, v111, v101);
}

fn push_cylinder_y(
    tris: &mut Vec<Tri>,
    cx: f64,
    cz: f64,
    y0: f64,
    y1: f64,
    radius: f64,
    n_seg: usize,
) {
    let ring: Vec<(f64, f64)> = (0..n_seg)
        .map(|j| {
            let phi = 2.0 * PI * (j as f64) / (n_seg as f64);
            (cx + radius * phi.cos(), cz + radius * phi.sin())
        })
        .collect();

    let bot_c = [cx, y0, cz];
    let top_c = [cx, y1, cz];

    for j in 0..n_seg {
        let jn = (j + 1) % n_seg;
        let (x0, z0) = ring[j];
        let (x1, z1) = ring[jn];

        let a = [x0, y0, z0];
        let b = [x1, y0, z1];
        let c = [x1, y1, z1];
        let d = [x0, y1, z0];

        // Side wall (outward radially)
        push_tri(tris, a, c, b);
        push_tri(tris, a, d, c);

        // Top cap (+Y)
        push_tri(tris, top_c, c, d);

        // Bottom cap (-Y)
        push_tri(tris, bot_c, a, b);
    }
}

/// Smooth circular cylinder centered at the origin along the Z axis (`[-span/2, +span/2]`).
fn build_cylinder(radius: f64, span: f64, tris: &mut Vec<Tri>) -> Result<(), VenturiError> {
    validate_positive("radius", radius)?;
    validate_positive("span", span)?;

    let n_seg: usize = 64;
    let poly: Vec<[f64; 2]> = (0..n_seg)
        .map(|j| {
            let phi = 2.0 * PI * (j as f64) / (n_seg as f64);
            [radius * phi.cos(), radius * phi.sin()]
        })
        .collect();

    extrude_convex_polygon_z(&poly, -0.5 * span, 0.5 * span, tris);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Verifies that every directed half-edge in a Binary STL has an exact opposite twin
    /// (proving watertight 2-manifoldness with consistent outward orientation) and positive signed volume.
    fn assert_watertight_binary_stl(bytes: &[u8]) {
        assert!(bytes.len() >= 84);
        let count = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
        assert_eq!(bytes.len(), 84 + count * 50);
        assert!(count >= 12);

        let mut half_edges: HashMap<([u32; 3], [u32; 3]), usize> = HashMap::new();
        let mut signed_vol6 = 0.0_f64;

        let quant = |v: [f32; 3]| -> [u32; 3] {
            // Normalize -0.0 to +0.0 before bit-hashing
            [
                if v[0] == 0.0 { 0.0f32.to_bits() } else { v[0].to_bits() },
                if v[1] == 0.0 { 0.0f32.to_bits() } else { v[1].to_bits() },
                if v[2] == 0.0 { 0.0f32.to_bits() } else { v[2].to_bits() },
            ]
        };

        let mut offset = 84;
        for _ in 0..count {
            let rf = |o: usize| f32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
            let n = [rf(offset), rf(offset + 4), rf(offset + 8)];
            let n_len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((n_len - 1.0).abs() < 1e-3, "non-unit normal: {:?}", n);

            let v0 = [rf(offset + 12), rf(offset + 16), rf(offset + 20)];
            let v1 = [rf(offset + 24), rf(offset + 28), rf(offset + 32)];
            let v2 = [rf(offset + 36), rf(offset + 40), rf(offset + 44)];

            let q = [quant(v0), quant(v1), quant(v2)];
            for e in [(q[0], q[1]), (q[1], q[2]), (q[2], q[0])] {
                *half_edges.entry(e).or_insert(0) += 1;
            }

            let a = [v0[0] as f64, v0[1] as f64, v0[2] as f64];
            let b = [v1[0] as f64, v1[1] as f64, v1[2] as f64];
            let c = [v2[0] as f64, v2[1] as f64, v2[2] as f64];
            let bxc = [
                b[1] * c[2] - b[2] * c[1],
                b[2] * c[0] - b[0] * c[2],
                b[0] * c[1] - b[1] * c[0],
            ];
            signed_vol6 += a[0] * bxc[0] + a[1] * bxc[1] + a[2] * bxc[2];

            offset += 50;
        }

        assert!(
            signed_vol6 > 0.0,
            "Expected strictly positive outward signed volume, got {}",
            signed_vol6 / 6.0
        );

        for (&(u, v), &cnt) in &half_edges {
            let twin = half_edges.get(&(v, u)).copied().unwrap_or(0);
            assert_eq!(
                cnt, twin,
                "Non-watertight edge detected: {:?} -> {:?} count={} twin={}",
                u, v, cnt, twin
            );
        }
    }

    #[test]
    fn generates_watertight_naca_2412_airfoil() {
        let spec = ParametricGeometrySpec::NacaAirfoil {
            chord: 1.0,
            camber_pct: 2.0,
            camber_pos_pct: 40.0,
            thickness_pct: 12.0,
            aoa_deg: 6.0,
            span: 0.8,
        };
        assert_eq!(suggested_filename(&spec), "naca_airfoil.stl");
        assert!((reference_length(&spec) - 1.0).abs() < 1e-9);

        let (payload, bytes) = generate_parametric_stl(&spec).unwrap();
        assert_watertight_binary_stl(&bytes);
        assert!(payload.summary.is_binary);
        assert!(payload.summary.triangle_count >= 500);
        assert!(payload.summary.surface_area > 1.5);
        // NACA 4-digit 12% thickness cross-sectional area ~ 0.082 * c^2 -> vol ~ 0.0656
        assert!(payload.summary.volume_est > 0.055 && payload.summary.volume_est < 0.080);
        assert!((payload.summary.bounds_min[2] + 0.4).abs() < 1e-4);
        assert!((payload.summary.bounds_max[2] - 0.4).abs() < 1e-4);
        // Positive AoA lifts leading edge (+Y) and lowers trailing edge (-Y)
        assert!(payload.summary.bounds_max[1] > 0.06);
        assert!(payload.summary.bounds_min[1] < -0.05);
    }

    #[test]
    fn generates_watertight_venturi_tube() {
        let spec = ParametricGeometrySpec::VenturiTube {
            inlet_radius: 0.50,
            throat_radius: 0.25,
            outlet_radius: 0.50,
            convergent_len: 1.0,
            throat_len: 0.6,
            divergent_len: 2.4,
        };
        assert_eq!(suggested_filename(&spec), "venturi_tube.stl");
        assert!((reference_length(&spec) - 0.50).abs() < 1e-9);

        let (payload, bytes) = generate_parametric_stl(&spec).unwrap();
        assert_watertight_binary_stl(&bytes);
        assert!(payload.summary.is_binary);
        assert!(payload.summary.triangle_count > 2000);
        assert!((payload.summary.bounds_min[0] + 2.0).abs() < 1e-4);
        assert!((payload.summary.bounds_max[0] - 2.0).abs() < 1e-4);
        assert!(payload.summary.bounds_max[1] > 0.50 && payload.summary.bounds_max[1] < 0.56);
        assert!(payload.summary.surface_area > 10.0);
        assert!(payload.summary.volume_est > 0.15);
    }

    #[test]
    fn generates_watertight_ahmed_diffuser() {
        let spec = ParametricGeometrySpec::AhmedDiffuser {
            length: 1.044,
            width: 0.389,
            height: 0.288,
            rear_slant_deg: 25.0,
            ground_clearance: 0.05,
        };
        assert_eq!(suggested_filename(&spec), "ahmed_diffuser.stl");
        assert!((reference_length(&spec) - 1.044).abs() < 1e-9);

        let (payload, bytes) = generate_parametric_stl(&spec).unwrap();
        assert_watertight_binary_stl(&bytes);
        assert!(payload.summary.is_binary);
        assert!(payload.summary.triangle_count >= 60);
        assert!((payload.summary.bounds_min[0] - 0.0).abs() < 1e-4);
        assert!((payload.summary.bounds_max[0] - 1.044).abs() < 1e-4);
        assert!((payload.summary.bounds_min[1] - 0.05).abs() < 1e-4);
        assert!((payload.summary.bounds_max[1] - 0.338).abs() < 1e-4);
        assert!((payload.summary.bounds_min[2] + 0.1945).abs() < 1e-4);
        assert!((payload.summary.bounds_max[2] - 0.1945).abs() < 1e-4);
        // Box volume = 1.044 * 0.389 * 0.288 = 0.1170; minus nose rounding & 25° slant ~ 0.105
        assert!(payload.summary.volume_est > 0.095 && payload.summary.volume_est < 0.116);
        assert!(payload.summary.surface_area > 1.2);
    }

    #[test]
    fn generates_watertight_pin_fin_array() {
        let spec = ParametricGeometrySpec::PinFinArray {
            base_length: 0.20,
            base_width: 0.15,
            fin_count_x: 4,
            fin_count_z: 3,
            fin_radius: 0.008,
            fin_height: 0.05,
        };
        assert_eq!(suggested_filename(&spec), "pin_fin_array.stl");
        assert!((reference_length(&spec) - 0.20).abs() < 1e-9);

        let (payload, bytes) = generate_parametric_stl(&spec).unwrap();
        assert_watertight_binary_stl(&bytes);
        assert!(payload.summary.is_binary);
        // 12 base tris + 12 fins * (24 * 4 = 96 tris/fin) = 1164 tris
        assert_eq!(payload.summary.triangle_count, 12 + 12 * 24 * 4);
        assert!((payload.summary.bounds_min[0] + 0.10).abs() < 1e-4);
        assert!((payload.summary.bounds_max[0] - 0.10).abs() < 1e-4);
        assert!((payload.summary.bounds_min[1] - 0.0).abs() < 1e-4);
        let expected_top = 0.18 * 0.05 + 0.05;
        assert!((payload.summary.bounds_max[1] - expected_top).abs() < 1e-4);

        let expected_vol = 0.20 * 0.15 * (0.18 * 0.05) + 12.0 * PI * 0.008_f64.powi(2) * 0.05;
        assert!(
            (payload.summary.volume_est - expected_vol).abs() / expected_vol < 0.02,
            "vol={} expected={}",
            payload.summary.volume_est,
            expected_vol
        );
    }

    #[test]
    fn generates_watertight_cylinder() {
        let spec = ParametricGeometrySpec::Cylinder {
            radius: 0.25,
            span: 0.60,
        };
        assert_eq!(suggested_filename(&spec), "cylinder.stl");
        assert!((reference_length(&spec) - 0.50).abs() < 1e-9);

        let (payload, bytes) = generate_parametric_stl(&spec).unwrap();
        assert_watertight_binary_stl(&bytes);
        assert!(payload.summary.is_binary);
        assert_eq!(payload.summary.triangle_count, 64 * 4);
        assert!((payload.summary.bounds_min[0] + 0.25).abs() < 1e-4);
        assert!((payload.summary.bounds_max[0] - 0.25).abs() < 1e-4);
        assert!((payload.summary.bounds_min[2] + 0.30).abs() < 1e-4);
        assert!((payload.summary.bounds_max[2] - 0.30).abs() < 1e-4);

        let analytic_vol = PI * 0.25_f64.powi(2) * 0.60;
        let analytic_area = 2.0 * PI * 0.25 * 0.60 + 2.0 * PI * 0.25_f64.powi(2);
        assert!((payload.summary.volume_est - analytic_vol).abs() / analytic_vol < 0.005);
        assert!((payload.summary.surface_area - analytic_area).abs() / analytic_area < 0.005);
    }

    #[test]
    fn rejects_invalid_parametric_bounds() {
        let bad = ParametricGeometrySpec::NacaAirfoil {
            chord: -1.0,
            camber_pct: 2.0,
            camber_pos_pct: 40.0,
            thickness_pct: 12.0,
            aoa_deg: 0.0,
            span: 0.5,
        };
        assert!(generate_parametric_stl(&bad).is_err());

        let bad_fins = ParametricGeometrySpec::PinFinArray {
            base_length: 0.2,
            base_width: 0.2,
            fin_count_x: 0,
            fin_count_z: 3,
            fin_radius: 0.01,
            fin_height: 0.05,
        };
        assert!(generate_parametric_stl(&bad_fins).is_err());
    }

    #[test]
    fn watertight_under_extreme_camber_aoa_and_squareback_ahmed() {
        let high_camber = ParametricGeometrySpec::NacaAirfoil {
            chord: 1.2,
            camber_pct: 6.0,
            camber_pos_pct: 25.0,
            thickness_pct: 22.0,
            aoa_deg: 16.0,
            span: 1.0,
        };
        let (p1, b1) = generate_parametric_stl(&high_camber).unwrap();
        assert_watertight_binary_stl(&b1);
        assert!(p1.summary.volume_est > 0.1);

        let squareback = ParametricGeometrySpec::AhmedDiffuser {
            length: 1.0,
            width: 0.4,
            height: 0.3,
            rear_slant_deg: 0.0,
            ground_clearance: 0.0,
        };
        let (p2, b2) = generate_parametric_stl(&squareback).unwrap();
        assert_watertight_binary_stl(&b2);
        assert!(p2.summary.bounds_min[1].abs() < 1e-5);
    }
}
