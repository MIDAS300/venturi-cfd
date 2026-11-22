import React, { useEffect, useRef, useState } from "react";
import * as THREE from "three";
import {
  Grid3x3,
  Wind,
  Eye,
  ShieldCheck,
  AlertTriangle,
  XCircle,
  RotateCcw,
  Box as BoxIcon,
  Layers,
} from "lucide-react";
import {
  BoundaryKind,
  CfdCase,
  DomainFace,
  ParametricGeometrySpec,
  StlMeshPayload,
  TelemetrySnapshot,
  ValidationReport,
} from "../types/cfd";

interface Viewport3DProps {
  cfdCase: CfdCase;
  stlMesh: StlMeshPayload | null;
  telemetry: TelemetrySnapshot | null;
  validation: ValidationReport | null;
  highlightedFace: DomainFace | null;
  onJumpToAudit: () => void;
}

type CameraPreset = "iso" | "xy" | "xz" | "yz";

function boundaryColorHex(kind: BoundaryKind): number {
  switch (kind) {
    case "velocityInlet":
      return 0x22d3ee; // Cyan
    case "pressureOutlet":
      return 0x10b981; // Emerald
    case "noSlipWall":
      return 0xf43f5e; // Coral
    case "movingWall":
      return 0xf59e0b; // Amber
    case "symmetry":
      return 0x60a5fa; // Slate Blue
    case "empty2D":
      return 0x64748b; // Muted Slate
  }
}

function refinementLevelColorHex(level: number): number {
  if (level >= 4) return 0xfbbf24; // Bright Pitot Amber (L4+)
  if (level >= 3) return 0xf59e0b; // Pitot Amber (L3)
  return 0x22d3ee; // Cyan (L1/L2)
}

/** Authentic NACA 4-digit cambered airfoil rotated by -aoaDeg about quarter-chord, extruded along Z */
function buildNacaAirfoilGeometry(
  chord: number,
  span: number,
  camberPct = 0,
  camberPosPct = 40,
  thicknessPct = 12,
  aoaDeg = 0
): THREE.BufferGeometry {
  const c = Math.max(0.05, chord);
  const s = Math.max(0.02, span);
  const m = Math.max(0, Math.min(0.12, camberPct / 100));
  const p = Math.max(0.1, Math.min(0.9, camberPosPct / 100));
  const t = Math.max(0.03, Math.min(0.4, thicknessPct / 100));
  const aoa = (-aoaDeg * Math.PI) / 180;
  const cosA = Math.cos(aoa);
  const sinA = Math.sin(aoa);

  const upper: THREE.Vector2[] = [];
  const lower: THREE.Vector2[] = [];
  const n = 48;

  for (let i = 0; i <= n; i++) {
    const beta = (i / n) * Math.PI;
    const x = 0.5 * (1 - Math.cos(beta));
    const yt =
      5 *
      t *
      (0.2969 * Math.sqrt(x) -
        0.126 * x -
        0.3516 * x * x +
        0.2843 * x * x * x -
        0.1015 * x * x * x * x);

    let yc = 0;
    let dyc = 0;
    if (m > 1e-6) {
      if (x < p) {
        yc = (m / (p * p)) * (2 * p * x - x * x);
        dyc = ((2 * m) / (p * p)) * (p - x);
      } else {
        yc = (m / ((1 - p) * (1 - p))) * (1 - 2 * p + 2 * p * x - x * x);
        dyc = ((2 * m) / ((1 - p) * (1 - p))) * (p - x);
      }
    }
    const theta = Math.atan(dyc);
    const xu = (x - yt * Math.sin(theta) - 0.25) * c;
    const yu = (yc + yt * Math.cos(theta)) * c;
    const xl = (x + yt * Math.sin(theta) - 0.25) * c;
    const yl = (yc - yt * Math.cos(theta)) * c;

    upper.push(new THREE.Vector2(xu * cosA - yu * sinA, xu * sinA + yu * cosA));
    lower.push(new THREE.Vector2(xl * cosA - yl * sinA, xl * sinA + yl * cosA));
  }

  const pts: THREE.Vector2[] = [...upper, ...lower.slice(1, -1).reverse()];
  const shape = new THREE.Shape(pts);
  const geom = new THREE.ExtrudeGeometry(shape, {
    depth: s,
    bevelEnabled: false,
    curveSegments: 24,
  });
  geom.translate(0, 0, -s / 2);
  geom.computeVertexNormals();
  return geom;
}

/** Parametric Ahmed Diffuser body with exact rear slant angle and ground clearance */
function buildAhmedDiffuserGeometry(
  length = 1.044,
  width = 0.389,
  height = 0.288,
  rearSlantDeg = 25,
  groundClearance = 0.05
): THREE.BufferGeometry {
  const L = Math.max(0.2, length);
  const W = Math.max(0.08, width);
  const H = Math.max(0.08, height);
  const gc = Math.max(0.005, groundClearance);
  const slantRad = (Math.max(2, Math.min(60, rearSlantDeg)) * Math.PI) / 180;
  const slantLen = Math.min(L * 0.36, 0.222 * (L / 1.044));
  const slantDrop = Math.min(H * 0.72, slantLen * Math.tan(slantRad));
  const noseR = Math.min(H * 0.32, L * 0.1);

  const shape = new THREE.Shape();
  shape.moveTo(noseR, gc);
  shape.lineTo(L, gc);
  shape.lineTo(L, gc + H - slantDrop);
  shape.lineTo(L - slantLen, gc + H);
  shape.lineTo(noseR, gc + H);
  shape.quadraticCurveTo(0, gc + H, 0, gc + H - noseR);
  shape.lineTo(0, gc + noseR);
  shape.quadraticCurveTo(0, gc, noseR, gc);

  const bevel = Math.min(0.02, W * 0.05, H * 0.06);
  const geom = new THREE.ExtrudeGeometry(shape, {
    depth: Math.max(0.02, W - 2 * bevel),
    bevelEnabled: true,
    bevelThickness: bevel,
    bevelSize: bevel,
    bevelSegments: 3,
  });
  geom.translate(-0.2 * L, 0, -(W - 2 * bevel) / 2);
  geom.computeVertexNormals();
  return geom;
}

/** Convergent-Divergent Venturi Tube shell along X axis */
function buildVenturiNozzleGeometry(xMin: number, xMax: number, outerR: number): THREE.BufferGeometry {
  const len = xMax - xMin;
  const pts: THREE.Vector2[] = [];
  const samples = 44;
  for (let i = 0; i <= samples; i++) {
    const u = i / samples; // 0..1 along axis
    const x = xMin + u * len;
    // Smooth Venturi throat profile centered at u = 0.36
    let r = outerR * 0.88;
    if (u > 0.14 && u < 0.82) {
      const throatFactor =
        u < 0.36
          ? Math.pow(Math.sin(((u - 0.14) / 0.22) * (Math.PI / 2)), 2)
          : Math.pow(Math.cos(((u - 0.36) / 0.46) * (Math.PI / 2)), 2);
      r = outerR * (0.88 - 0.46 * throatFactor);
    }
    pts.push(new THREE.Vector2(Math.max(0.05, r), x));
  }
  const geom = new THREE.LatheGeometry(pts, 40);
  // Rotate so Lathe Y-axis maps onto X-axis
  geom.rotateZ(-Math.PI / 2);
  geom.computeVertexNormals();
  return geom;
}

/** Parametric Venturi Tube revolved along X axis centered at X=0 */
function buildParametricVenturiGeometry(
  inletRadius: number,
  throatRadius: number,
  outletRadius: number,
  convergentLen: number,
  throatLen: number,
  divergentLen: number
): THREE.BufferGeometry {
  const lConv = Math.max(0.05, convergentLen);
  const lThroat = Math.max(0.05, throatLen);
  const lDiv = Math.max(0.05, divergentLen);
  const total = lConv + lThroat + lDiv;
  const xStart = -total * 0.5;
  const pts: THREE.Vector2[] = [];
  const samples = 48;

  for (let i = 0; i <= samples; i++) {
    const s = (i / samples) * total;
    let r = throatRadius;
    if (s < lConv) {
      const u = s / lConv;
      const smooth = 0.5 * (1 - Math.cos(Math.PI * u));
      r = inletRadius + (throatRadius - inletRadius) * smooth;
    } else if (s > lConv + lThroat) {
      const u = (s - lConv - lThroat) / lDiv;
      const smooth = 0.5 * (1 - Math.cos(Math.PI * u));
      r = throatRadius + (outletRadius - throatRadius) * smooth;
    }
    pts.push(new THREE.Vector2(Math.max(0.02, r), xStart + s));
  }
  const geom = new THREE.LatheGeometry(pts, 40);
  geom.rotateZ(-Math.PI / 2);
  geom.computeVertexNormals();
  return geom;
}

/** Procedural fallback group for Pin-Fin Array (base plate + cylinder grid) */
function buildPinFinArrayGroup(
  spec: Extract<ParametricGeometrySpec, { family: "pinFinArray" }>,
  material: THREE.Material,
  wireframe: boolean,
  obstacleHighlighted: boolean
): THREE.Group {
  const grp = new THREE.Group();
  const bl = Math.max(0.02, spec.baseLength);
  const bw = Math.max(0.02, spec.baseWidth);
  const finH = Math.max(0.005, spec.finHeight);
  const finR = Math.max(0.001, spec.finRadius);
  const baseThick = Math.max(0.004, finH * 0.18);

  const edgeMat = !wireframe
    ? new THREE.LineBasicMaterial({
        color: obstacleHighlighted ? 0xf59e0b : 0x38bdf8,
        transparent: true,
        opacity: obstacleHighlighted ? 0.95 : 0.28,
      })
    : null;

  const baseGeom = new THREE.BoxGeometry(bl, baseThick, bw);
  const baseMesh = new THREE.Mesh(baseGeom, material);
  baseMesh.position.set(0, baseThick * 0.5, 0);
  grp.add(baseMesh);
  if (edgeMat) {
    const baseWire = new THREE.LineSegments(new THREE.EdgesGeometry(baseGeom, 24), edgeMat);
    baseWire.position.copy(baseMesh.position);
    grp.add(baseWire);
  }

  const nx = Math.max(1, Math.min(18, Math.round(spec.finCountX)));
  const nz = Math.max(1, Math.min(18, Math.round(spec.finCountZ)));
  const pinGeom = new THREE.CylinderGeometry(finR, finR, finH, 20);
  const pinEdges = edgeMat ? new THREE.EdgesGeometry(pinGeom, 24) : null;

  for (let ix = 0; ix < nx; ix++) {
    const fx = nx === 1 ? 0 : -bl * 0.38 + (ix / (nx - 1)) * (bl * 0.76);
    for (let iz = 0; iz < nz; iz++) {
      const fz = nz === 1 ? 0 : -bw * 0.38 + (iz / (nz - 1)) * (bw * 0.76);
      const pinMesh = new THREE.Mesh(pinGeom, material);
      pinMesh.position.set(fx, baseThick + finH * 0.5, fz);
      grp.add(pinMesh);
      if (pinEdges && edgeMat) {
        const pinWire = new THREE.LineSegments(pinEdges, edgeMat);
        pinWire.position.copy(pinMesh.position);
        grp.add(pinWire);
      }
    }
  }

  return grp;
}

function formatEngineering(n: number): string {
  if (!Number.isFinite(n)) return "—";
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}k`;
  return n.toFixed(0);
}

export const Viewport3D: React.FC<Viewport3DProps> = ({
  cfdCase,
  stlMesh,
  telemetry,
  validation,
  highlightedFace,
  onJumpToAudit,
}) => {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const [showMeshGrid, setShowMeshGrid] = useState(true);
  const [showFlow, setShowFlow] = useState(true);
  const [wireframe, setWireframe] = useState(false);
  const [xrayDomain, setXrayDomain] = useState(true);
  const [showRefinementBoxes, setShowRefinementBoxes] = useState(true);

  // Orbit camera state stored in ref for smooth 60fps animation without re-renders
  const camStateRef = useRef({
    theta: 0.72,
    phi: 1.08,
    radius: 10.5,
    target: new THREE.Vector3(1.5, 0.0, 0.0),
    dragging: false,
    panning: false,
    lastX: 0,
    lastY: 0,
  });

  const sceneGroupRef = useRef<THREE.Group | null>(null);
  const particlesRef = useRef<{
    points: THREE.Points;
    velocities: Float32Array;
    bounds: { xMin: number; xMax: number; yMin: number; yMax: number; zMin: number; zMax: number };
    speed: number;
    dir: [number, number, number];
  } | null>(null);
  const spriteTexRef = useRef<THREE.CanvasTexture | null>(null);
  const showFlowRef = useRef(showFlow);
  showFlowRef.current = showFlow;

  // Initialize Three.js renderer + animation loop once
  useEffect(() => {
    const container = mountRef.current;
    if (!container) return;

    const scene = new THREE.Scene();
    const camera = new THREE.PerspectiveCamera(
      40,
      Math.max(1, container.clientWidth) / Math.max(1, container.clientHeight),
      0.05,
      500
    );

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setSize(container.clientWidth, container.clientHeight);
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.15;
    container.appendChild(renderer.domElement);

    // Studio aerospace lighting rig
    scene.add(new THREE.AmbientLight(0xdbeafe, 0.65));
    const keyLight = new THREE.DirectionalLight(0xfffbeb, 1.35);
    keyLight.position.set(12, 16, 14);
    scene.add(keyLight);

    const rimLight = new THREE.DirectionalLight(0x38bdf8, 0.75);
    rimLight.position.set(-14, 6, -12);
    scene.add(rimLight);

    const fillLight = new THREE.DirectionalLight(0xf59e0b, 0.35);
    fillLight.position.set(4, -10, 6);
    scene.add(fillLight);

    const group = new THREE.Group();
    scene.add(group);
    sceneGroupRef.current = group;

    // Subtle global reference floor axes at origin
    const axesHelper = new THREE.AxesHelper(1.2);
    scene.add(axesHelper);

    const updateCamera = () => {
      const s = camStateRef.current;
      const sinPhi = Math.sin(s.phi);
      camera.position.set(
        s.target.x + s.radius * sinPhi * Math.sin(s.theta),
        s.target.y + s.radius * Math.cos(s.phi),
        s.target.z + s.radius * sinPhi * Math.cos(s.theta)
      );
      camera.lookAt(s.target);
    };
    updateCamera();

    // Mouse interaction handlers
    const dom = renderer.domElement;
    const onPointerDown = (e: PointerEvent) => {
      dom.setPointerCapture(e.pointerId);
      camStateRef.current.lastX = e.clientX;
      camStateRef.current.lastY = e.clientY;
      if (e.button === 2 || e.shiftKey) {
        camStateRef.current.panning = true;
      } else {
        camStateRef.current.dragging = true;
      }
    };

    const onPointerMove = (e: PointerEvent) => {
      const s = camStateRef.current;
      if (!s.dragging && !s.panning) return;
      const dx = e.clientX - s.lastX;
      const dy = e.clientY - s.lastY;
      s.lastX = e.clientX;
      s.lastY = e.clientY;

      if (s.dragging) {
        s.theta -= dx * 0.007;
        s.phi = Math.max(0.08, Math.min(Math.PI - 0.08, s.phi - dy * 0.007));
      } else if (s.panning) {
        const panScale = s.radius * 0.0014;
        const right = new THREE.Vector3();
        const up = new THREE.Vector3();
        camera.matrix.extractBasis(right, up, new THREE.Vector3());
        s.target.addScaledVector(right, -dx * panScale);
        s.target.addScaledVector(up, dy * panScale);
      }
    };

    const onPointerUp = (e: PointerEvent) => {
      camStateRef.current.dragging = false;
      camStateRef.current.panning = false;
      if (dom.hasPointerCapture(e.pointerId)) {
        dom.releasePointerCapture(e.pointerId);
      }
    };

    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const s = camStateRef.current;
      const factor = e.deltaY > 0 ? 1.09 : 0.91;
      s.radius = Math.max(0.3, Math.min(120, s.radius * factor));
    };

    const onContext = (e: MouseEvent) => e.preventDefault();

    dom.addEventListener("pointerdown", onPointerDown);
    dom.addEventListener("pointermove", onPointerMove);
    dom.addEventListener("pointerup", onPointerUp);
    dom.addEventListener("wheel", onWheel, { passive: false });
    dom.addEventListener("contextmenu", onContext);

    const resizeObserver = new ResizeObserver(() => {
      const w = Math.max(1, container.clientWidth);
      const h = Math.max(1, container.clientHeight);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    });
    resizeObserver.observe(container);

    let rafId = 0;
    let lastTime = performance.now();
    const animate = (now: number) => {
      rafId = requestAnimationFrame(animate);
      const dt = Math.min(0.05, (now - lastTime) / 1000);
      lastTime = now;

      // Advect streamline flow particles (only when velocity particles are enabled)
      const pState = particlesRef.current;
      if (pState) {
        pState.points.visible = showFlowRef.current;
        if (showFlowRef.current) {
          const posAttr = pState.points.geometry.getAttribute("position") as THREE.BufferAttribute;
          const arr = posAttr.array as Float32Array;
          const b = pState.bounds;
          const lx = Math.max(0.1, b.xMax - b.xMin);
          const ly = Math.max(0.1, b.yMax - b.yMin);
          const lz = Math.max(0.1, b.zMax - b.zMin);
          const [dxDir, dyDir, dzDir] = pState.dir;
          const stepBase = Math.min(lx * 0.45, Math.max(0.6, Math.log10(pState.speed + 1) * 2.2));
          for (let i = 0; i < arr.length; i += 3) {
            const px = arr[i];
            const py = arr[i + 1];
            // Potential-flow style subtle streamline deflection near origin
            const r2 = px * px + py * py;
            const deflectY = r2 < 2.2 && r2 > 0.05 ? (py / r2) * 0.14 * dt : 0;
            const step = stepBase * pState.velocities[i / 3] * dt;
            arr[i] += step * dxDir;
            arr[i + 1] += step * dyDir + deflectY;
            arr[i + 2] += step * dzDir;

            if (
              arr[i] > b.xMax ||
              arr[i] < b.xMin ||
              arr[i + 1] > b.yMax ||
              arr[i + 1] < b.yMin ||
              arr[i + 2] > b.zMax ||
              arr[i + 2] < b.zMin
            ) {
              arr[i] = dxDir >= 0 ? b.xMin + Math.random() * 0.08 * lx : b.xMax - Math.random() * 0.08 * lx;
              arr[i + 1] = b.yMin + 0.08 * ly + Math.random() * 0.84 * ly;
              arr[i + 2] = b.zMin + 0.1 * lz + Math.random() * 0.8 * lz;
            }
          }
          posAttr.needsUpdate = true;
        }
      }

      updateCamera();
      renderer.render(scene, camera);
    };
    rafId = requestAnimationFrame(animate);

    return () => {
      cancelAnimationFrame(rafId);
      resizeObserver.disconnect();
      dom.removeEventListener("pointerdown", onPointerDown);
      dom.removeEventListener("pointermove", onPointerMove);
      dom.removeEventListener("pointerup", onPointerUp);
      dom.removeEventListener("wheel", onWheel);
      dom.removeEventListener("contextmenu", onContext);
      if (spriteTexRef.current) {
        spriteTexRef.current.dispose();
        spriteTexRef.current = null;
      }
      renderer.dispose();
      if (dom.parentNode === container) container.removeChild(dom);
    };
  }, []);

  const lastCaseKeyRef = useRef<string>("");

  // Auto-frame camera cleanly whenever the case preset, parametric family, custom STL, or CAD scale changes
  useEffect(() => {
    const key = `${cfdCase.name}::${cfdCase.geometryKind}::${cfdCase.parametric?.family ?? ""}::${cfdCase.stl?.filename ?? ""}::${cfdCase.stl?.scaleFactor ?? 1}`;
    if (lastCaseKeyRef.current === key) return;
    lastCaseKeyRef.current = key;
    const d = cfdCase.domain;
    const cx = 0.5 * (d.xMin + d.xMax);
    const cy = 0.5 * (d.yMin + d.yMax);
    const cz = 0.5 * (d.zMin + d.zMax);
    const diag = Math.hypot(d.xMax - d.xMin, d.yMax - d.yMin, d.zMax - d.zMin);
    const s = camStateRef.current;
    s.target.set(cx, cy, cz);
    s.radius = Math.max(0.5, diag * 1.02);
    s.theta = 0.65;
    s.phi = 1.08;
  }, [cfdCase.name, cfdCase.geometryKind, cfdCase.parametric?.family, cfdCase.stl, cfdCase.domain]);

  // Rebuild 3D CFD scene whenever case, STL, parametric spec, refinement boxes, display toggles, or highlighted face changes
  useEffect(() => {
    const group = sceneGroupRef.current;
    if (!group) return;

    // Clear & dispose previous scene objects to avoid GPU memory leaks
    const disposedGeoms = new Set<THREE.BufferGeometry>();
    const disposedMats = new Set<THREE.Material>();
    while (group.children.length > 0) {
      const child = group.children[0];
      group.remove(child);
      child.traverse((obj) => {
        const mesh = obj as THREE.Mesh;
        if (mesh.geometry && !disposedGeoms.has(mesh.geometry)) {
          disposedGeoms.add(mesh.geometry);
          mesh.geometry.dispose();
        }
        if (mesh.material) {
          const mats = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
          mats.forEach((m) => {
            if (!disposedMats.has(m)) {
              disposedMats.add(m);
              m.dispose();
            }
          });
        }
      });
    }

    const d = cfdCase.domain;
    const lx = Math.max(0.05, d.xMax - d.xMin);
    const ly = Math.max(0.05, d.yMax - d.yMin);
    const lz = Math.max(0.05, d.zMax - d.zMin);
    const cx = 0.5 * (d.xMin + d.xMax);
    const cy = 0.5 * (d.yMin + d.yMax);
    const cz = 0.5 * (d.zMin + d.zMax);

    // 1. Domain Bounding Wireframe
    const boxGeom = new THREE.BoxGeometry(lx, ly, lz);
    const edges = new THREE.EdgesGeometry(boxGeom);
    boxGeom.dispose();
    const edgeMat = new THREE.LineBasicMaterial({ color: 0x3b4d6b, transparent: true, opacity: 0.9 });
    const wireBox = new THREE.LineSegments(edges, edgeMat);
    wireBox.position.set(cx, cy, cz);
    group.add(wireBox);

    // 2. Live blockMesh subdivision grid preview (bottom & back planes)
    if (showMeshGrid) {
      const gridVerts: number[] = [];
      const nx = Math.min(80, Math.max(2, cfdCase.mesh.cellsX));
      const ny = Math.min(60, Math.max(2, cfdCase.mesh.cellsY));
      const nz = Math.min(40, Math.max(1, cfdCase.mesh.cellsZ));

      // Bottom plane (y = yMin)
      for (let i = 0; i <= nx; i++) {
        const x = d.xMin + (i / nx) * lx;
        gridVerts.push(x, d.yMin, d.zMin, x, d.yMin, d.zMax);
      }
      for (let k = 0; k <= nz; k++) {
        const z = d.zMin + (k / nz) * lz;
        gridVerts.push(d.xMin, d.yMin, z, d.xMax, d.yMin, z);
      }
      // Back plane (z = zMin)
      for (let i = 0; i <= nx; i++) {
        const x = d.xMin + (i / nx) * lx;
        gridVerts.push(x, d.yMin, d.zMin, x, d.yMax, d.zMin);
      }
      for (let j = 0; j <= ny; j++) {
        const y = d.yMin + (j / ny) * ly;
        gridVerts.push(d.xMin, y, d.zMin, d.xMax, y, d.zMin);
      }

      const gridGeom = new THREE.BufferGeometry();
      gridGeom.setAttribute("position", new THREE.Float32BufferAttribute(gridVerts, 3));
      const gridMat = new THREE.LineBasicMaterial({
        color: 0x26354d,
        transparent: true,
        opacity: 0.52,
      });
      group.add(new THREE.LineSegments(gridGeom, gridMat));
    }

    // 2b. 3D Refinement Regions (snappyHexMeshDict refinementBoxes)
    if (showRefinementBoxes && cfdCase.mesh.refinementBoxes.length > 0) {
      for (const box of cfdCase.mesh.refinementBoxes) {
        const bx = Math.max(0.01, Math.abs(box.max[0] - box.min[0]));
        const by = Math.max(0.01, Math.abs(box.max[1] - box.min[1]));
        const bz = Math.max(0.01, Math.abs(box.max[2] - box.min[2]));
        const rcx = 0.5 * (box.min[0] + box.max[0]);
        const rcy = 0.5 * (box.min[1] + box.max[1]);
        const rcz = 0.5 * (box.min[2] + box.max[2]);
        const col = refinementLevelColorHex(box.level);

        const rbGeom = new THREE.BoxGeometry(bx, by, bz);
        const fillMat = new THREE.MeshBasicMaterial({
          color: col,
          transparent: true,
          opacity: 0.12,
          side: THREE.DoubleSide,
          depthWrite: false,
        });
        const fillMesh = new THREE.Mesh(rbGeom, fillMat);
        fillMesh.position.set(rcx, rcy, rcz);
        group.add(fillMesh);

        const rbEdges = new THREE.EdgesGeometry(rbGeom);
        const rbEdgeMat = new THREE.LineBasicMaterial({
          color: col,
          transparent: true,
          opacity: box.level >= 3 ? 0.95 : 0.85,
        });
        const rbWire = new THREE.LineSegments(rbEdges, rbEdgeMat);
        rbWire.position.set(rcx, rcy, rcz);
        group.add(rbWire);
      }
    }

    // 3. Boundary Patches (semantic tinted translucent faces + velocity arrows)
    for (const patch of cfdCase.boundaries) {
      if (patch.face === "obstacle") continue;
      const isHighlighted = highlightedFace === patch.face;
      const color = boundaryColorHex(patch.kind);
      const baseOpacity =
        patch.kind === "velocityInlet" || patch.kind === "pressureOutlet"
          ? 0.14
          : patch.kind === "movingWall"
            ? 0.12
            : xrayDomain
              ? 0.03
              : 0.09;

      const mat = new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: isHighlighted ? 0.36 : baseOpacity,
        side: THREE.DoubleSide,
        depthWrite: false,
      });

      let plane: THREE.Mesh | null = null;
      if (patch.face === "xMin" || patch.face === "xMax") {
        plane = new THREE.Mesh(new THREE.PlaneGeometry(lz, ly), mat);
        plane.rotation.y = Math.PI / 2;
        plane.position.set(patch.face === "xMin" ? d.xMin : d.xMax, cy, cz);
      } else if (patch.face === "yMin" || patch.face === "yMax") {
        plane = new THREE.Mesh(new THREE.PlaneGeometry(lx, lz), mat);
        plane.rotation.x = Math.PI / 2;
        plane.position.set(cx, patch.face === "yMin" ? d.yMin : d.yMax, cz);
      } else if (patch.face === "zMin" || patch.face === "zMax") {
        plane = new THREE.Mesh(new THREE.PlaneGeometry(lx, ly), mat);
        plane.position.set(cx, cy, patch.face === "zMin" ? d.zMin : d.zMax);
      }

      if (plane) {
        group.add(plane);
      }

      // Draw 3D Velocity Inlet Arrows
      if (patch.kind === "velocityInlet" && patch.face === "xMin") {
        const dir = new THREE.Vector3(patch.velocity[0], patch.velocity[1], patch.velocity[2]);
        if (dir.lengthSq() > 1e-6) {
          dir.normalize();
          const arrowLen = Math.min(lx * 0.12, 0.75);
          for (const fy of [-0.28, 0, 0.28]) {
            for (const fz of [-0.22, 0.22]) {
              const origin = new THREE.Vector3(
                d.xMin - arrowLen * 0.35,
                cy + fy * ly,
                cz + fz * lz
              );
              const arrow = new THREE.ArrowHelper(dir, origin, arrowLen, 0x22d3ee, arrowLen * 0.28, arrowLen * 0.14);
              group.add(arrow);
            }
          }
        }
      }
    }

    // 4. Obstacle / Internal CAD Geometry
    const obstacleHighlighted = highlightedFace === "obstacle";
    const isInternalTube =
      cfdCase.geometryKind === "venturiNozzle" ||
      (cfdCase.geometryKind === "parametric" && cfdCase.parametric?.family === "venturiTube");

    const obsMat = new THREE.MeshStandardMaterial({
      color: obstacleHighlighted ? 0xfbbf24 : 0xe2e8f0,
      metalness: 0.42,
      roughness: 0.32,
      wireframe,
      transparent: isInternalTube,
      opacity: isInternalTube ? 0.58 : 1.0,
      side: THREE.DoubleSide,
    });

    let geom: THREE.BufferGeometry | null = null;
    let customGroup: THREE.Group | null = null;
    let renderedFromStl = false;

    const hasStlBuffers = stlMesh !== null && stlMesh.positions.length > 0;

    if (hasStlBuffers && (cfdCase.geometryKind === "customStl" || cfdCase.geometryKind === "parametric")) {
      geom = new THREE.BufferGeometry();
      geom.setAttribute("position", new THREE.Float32BufferAttribute(stlMesh.positions, 3));
      if (stlMesh.normals.length === stlMesh.positions.length) {
        geom.setAttribute("normal", new THREE.Float32BufferAttribute(stlMesh.normals, 3));
      } else {
        geom.computeVertexNormals();
      }
      renderedFromStl = true;
    } else if (cfdCase.geometryKind === "parametric" && cfdCase.parametric) {
      const spec = cfdCase.parametric;
      if (spec.family === "nacaAirfoil") {
        geom = buildNacaAirfoilGeometry(
          spec.chord,
          spec.span,
          spec.camberPct,
          spec.camberPosPct,
          spec.thicknessPct,
          spec.aoaDeg
        );
      } else if (spec.family === "venturiTube") {
        geom = buildParametricVenturiGeometry(
          spec.inletRadius,
          spec.throatRadius,
          spec.outletRadius,
          spec.convergentLen,
          spec.throatLen,
          spec.divergentLen
        );
      } else if (spec.family === "ahmedDiffuser") {
        geom = buildAhmedDiffuserGeometry(
          spec.length,
          spec.width,
          spec.height,
          spec.rearSlantDeg,
          spec.groundClearance
        );
      } else if (spec.family === "pinFinArray") {
        customGroup = buildPinFinArrayGroup(spec, obsMat, wireframe, obstacleHighlighted);
      } else if (spec.family === "cylinder") {
        geom = new THREE.CylinderGeometry(
          Math.max(0.01, spec.radius),
          Math.max(0.01, spec.radius),
          Math.max(0.02, spec.span),
          40
        );
        geom.rotateX(Math.PI / 2);
      }
    } else if (cfdCase.geometryKind === "airfoilNaca0012") {
      geom = buildNacaAirfoilGeometry(cfdCase.physics.referenceLength, lz * 0.96, 0, 40, 12, 0);
    } else if (cfdCase.geometryKind === "ahmedBody") {
      geom = buildAhmedDiffuserGeometry(1.044, 0.389, 0.288, 25, 0.05);
    } else if (cfdCase.geometryKind === "venturiNozzle") {
      geom = buildVenturiNozzleGeometry(d.xMin, d.xMax, Math.min(ly, lz) * 0.5);
    } else if (cfdCase.geometryKind === "cylinderCrossflow") {
      geom = new THREE.CylinderGeometry(
        cfdCase.physics.referenceLength * 0.5,
        cfdCase.physics.referenceLength * 0.5,
        lz * 0.96,
        36
      );
      geom.rotateX(Math.PI / 2);
    }

    if (customGroup) {
      group.add(customGroup);
    } else if (geom) {
      const mesh = new THREE.Mesh(geom, obsMat);
      group.add(mesh);

      // Subtle wireframe accent on procedural CAD geometry for visual clarity
      if (!wireframe && !renderedFromStl) {
        const accentWire = new THREE.LineSegments(
          new THREE.EdgesGeometry(geom, 24),
          new THREE.LineBasicMaterial({
            color: obstacleHighlighted ? 0xf59e0b : 0x38bdf8,
            transparent: true,
            opacity: obstacleHighlighted ? 0.95 : 0.28,
          })
        );
        group.add(accentWire);
      }
    } else {
      obsMat.dispose();
    }

    // 5. Streamline Flow Tracer Particles (soft radial aerodynamic tracer sprites)
    const count = 340;
    const pPos = new Float32Array(count * 3);
    const pColors = new Float32Array(count * 3);
    const pVels = new Float32Array(count);
    for (let i = 0; i < count; i++) {
      const px = d.xMin + Math.random() * lx;
      const py = d.yMin + 0.08 * ly + Math.random() * 0.84 * ly;
      const pz = d.zMin + 0.1 * lz + Math.random() * 0.8 * lz;
      pPos[i * 3] = px;
      pPos[i * 3 + 1] = py;
      pPos[i * 3 + 2] = pz;
      pVels[i] = 0.78 + Math.random() * 0.44;

      // Velocity-tinted cyan-to-amber tracer gradient
      const c = new THREE.Color().setHSL(0.53 - Math.random() * 0.09, 0.92, 0.64);
      pColors[i * 3] = c.r;
      pColors[i * 3 + 1] = c.g;
      pColors[i * 3 + 2] = c.b;
    }
    const spriteCanvas = document.createElement("canvas");
    spriteCanvas.width = 32;
    spriteCanvas.height = 32;
    const sctx = spriteCanvas.getContext("2d");
    if (sctx) {
      const grad = sctx.createRadialGradient(16, 16, 1, 16, 16, 15);
      grad.addColorStop(0, "rgba(255,255,255,1)");
      grad.addColorStop(0.35, "rgba(180,245,255,0.85)");
      grad.addColorStop(1, "rgba(34,211,238,0)");
      sctx.fillStyle = grad;
      sctx.fillRect(0, 0, 32, 32);
    }
    if (spriteTexRef.current) {
      spriteTexRef.current.dispose();
    }
    const spriteTex = new THREE.CanvasTexture(spriteCanvas);
    spriteTexRef.current = spriteTex;

    const pGeom = new THREE.BufferGeometry();
    pGeom.setAttribute("position", new THREE.BufferAttribute(pPos, 3));
    pGeom.setAttribute("color", new THREE.BufferAttribute(pColors, 3));
    const pMat = new THREE.PointsMaterial({
      size: Math.max(0.03, Math.min(lx, ly) * 0.032),
      map: spriteTex,
      vertexColors: true,
      transparent: true,
      opacity: 0.85,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
    });
    const points = new THREE.Points(pGeom, pMat);
    points.visible = showFlowRef.current;
    group.add(points);

    const inletPatch = cfdCase.boundaries.find((b) => b.kind === "velocityInlet");
    let dir: [number, number, number] = [1, 0, 0];
    let speed = Math.max(0.2, cfdCase.physics.referenceVelocity);
    if (inletPatch) {
      const mag = Math.hypot(inletPatch.velocity[0], inletPatch.velocity[1], inletPatch.velocity[2]);
      if (mag > 1e-6) {
        dir = [inletPatch.velocity[0] / mag, inletPatch.velocity[1] / mag, inletPatch.velocity[2] / mag];
        speed = Math.max(0.2, mag);
      }
    }

    particlesRef.current = {
      points,
      velocities: pVels,
      bounds: { ...d },
      speed,
      dir,
    };
  }, [
    cfdCase.domain,
    cfdCase.mesh.cellsX,
    cfdCase.mesh.cellsY,
    cfdCase.mesh.cellsZ,
    cfdCase.mesh.refinementBoxes,
    cfdCase.geometryKind,
    cfdCase.parametric,
    cfdCase.boundaries,
    cfdCase.physics.referenceLength,
    cfdCase.physics.referenceVelocity,
    stlMesh,
    showMeshGrid,
    showRefinementBoxes,
    wireframe,
    xrayDomain,
    highlightedFace,
  ]);

  const setCameraPreset = (preset: CameraPreset) => {
    const d = cfdCase.domain;
    const cx = 0.5 * (d.xMin + d.xMax);
    const cy = 0.5 * (d.yMin + d.yMax);
    const cz = 0.5 * (d.zMin + d.zMax);
    const diag = Math.hypot(d.xMax - d.xMin, d.yMax - d.yMin, d.zMax - d.zMin);
    const s = camStateRef.current;
    s.target.set(cx, cy, cz);
    s.radius = Math.max(0.5, diag * 1.05);

    if (preset === "iso") {
      s.theta = 0.65;
      s.phi = 1.08;
    } else if (preset === "xy") {
      s.theta = 0.001;
      s.phi = Math.PI / 2;
    } else if (preset === "xz") {
      s.theta = 0.001;
      s.phi = 0.08;
    } else if (preset === "yz") {
      s.theta = Math.PI / 2;
      s.phi = Math.PI / 2;
    }
  };

  const zoneCount = cfdCase.mesh.refinementBoxes.length;

  return (
    <div className="viewport-container">
      {/* Live Instrument Telemetry Strip */}
      <div className="telemetry-strip">
        <div className="telemetry-group">
          <div className="telemetry-cell">
            <span className="telemetry-label">Reynolds Re</span>
            <span className="telemetry-val" style={{ color: "var(--cfd-inlet)" }}>
              {telemetry ? telemetry.reynolds.toExponential(2) : "—"}
            </span>
          </div>
          <div className="telemetry-cell">
            <span className="telemetry-label">Mach Ma</span>
            <span
              className="telemetry-val"
              style={{
                color: telemetry?.compressibleWarning ? "var(--accent-amber-bright)" : "var(--text-primary)",
              }}
            >
              {telemetry ? telemetry.mach.toFixed(3) : "—"}
            </span>
          </div>
          <div className="telemetry-cell">
            <span className="telemetry-label">Est. Cells</span>
            <span className="telemetry-val">
              {telemetry ? formatEngineering(telemetry.estimatedCells) : "—"}
            </span>
          </div>
          <div className="telemetry-cell">
            <span className="telemetry-label">Est. RAM</span>
            <span className="telemetry-val">
              {telemetry
                ? telemetry.estimatedRamMb >= 1024
                  ? `${(telemetry.estimatedRamMb / 1024).toFixed(2)} GB`
                  : `${telemetry.estimatedRamMb.toFixed(0)} MB`
                : "—"}
            </span>
          </div>
          <div className="telemetry-cell">
            <span className="telemetry-label">Target y+ / Δy₁</span>
            <span className="telemetry-val" style={{ color: "var(--accent-amber-bright)" }}>
              {cfdCase.mesh.targetYPlus.toFixed(1)} ·{" "}
              {telemetry ? `${telemetry.firstCellHeightMm.toFixed(3)} mm` : "—"}
            </span>
          </div>
        </div>

        {/* Clickable Pre-Flight Audit Score */}
        <button
          type="button"
          className={
            !validation
              ? "pill"
              : validation.errorCount > 0
                ? "pill pill-error"
                : validation.warningCount > 0
                  ? "pill pill-warn"
                  : "pill pill-pass"
          }
          style={{ pointerEvents: "auto", cursor: "pointer", padding: "5px 11px", fontSize: 11 }}
          onClick={onJumpToAudit}
          title="Click to inspect full Pre-Flight Engineering Audit"
        >
          {!validation || (validation.errorCount === 0 && validation.warningCount === 0) ? (
            <ShieldCheck size={13} />
          ) : validation.errorCount > 0 ? (
            <XCircle size={13} />
          ) : (
            <AlertTriangle size={13} />
          )}
          AUDIT {validation ? `${validation.score}/100` : "—"}
          {validation && validation.errorCount > 0 && ` · ${validation.errorCount} ERR`}
          {validation && validation.warningCount > 0 && ` · ${validation.warningCount} WARN`}
        </button>
      </div>

      {/* Three.js Canvas Mount */}
      <div ref={mountRef} style={{ width: "100%", height: "100%" }} />

      {/* Bottom Floating Viewport Controls */}
      <div className="viewport-controls">
        <div className="vp-pill-group">
          <button type="button" className="vp-btn" onClick={() => setCameraPreset("iso")}>
            <RotateCcw size={11} /> ISO
          </button>
          <button type="button" className="vp-btn" onClick={() => setCameraPreset("xy")}>
            XY
          </button>
          <button type="button" className="vp-btn" onClick={() => setCameraPreset("xz")}>
            XZ
          </button>
          <button type="button" className="vp-btn" onClick={() => setCameraPreset("yz")}>
            YZ
          </button>
        </div>

        <div className="vp-pill-group">
          <button
            type="button"
            className={`vp-btn ${showMeshGrid ? "active" : ""}`}
            onClick={() => setShowMeshGrid((v) => !v)}
          >
            <Grid3x3 size={11} /> Mesh
          </button>
          <button
            type="button"
            className={`vp-btn ${showFlow ? "active" : ""}`}
            onClick={() => setShowFlow((v) => !v)}
            aria-pressed={showFlow}
            title="Attiva o disattiva la visualizzazione dei particellari del flusso di velocità"
          >
            <Wind size={11} />
            <span>Particellari</span>
            <span
              style={{
                fontSize: 9,
                padding: "0 4px",
                borderRadius: 3,
                background: showFlow ? "rgba(34, 211, 238, 0.22)" : "rgba(100, 116, 139, 0.22)",
                color: showFlow ? "var(--cfd-inlet)" : "var(--text-muted)",
                fontWeight: 700,
              }}
            >
              {showFlow ? "ON" : "OFF"}
            </span>
          </button>
          <button
            type="button"
            className={`vp-btn ${wireframe ? "active" : ""}`}
            onClick={() => setWireframe((v) => !v)}
          >
            <BoxIcon size={11} /> Wire
          </button>
          <button
            type="button"
            className={`vp-btn ${xrayDomain ? "active" : ""}`}
            onClick={() => setXrayDomain((v) => !v)}
          >
            <Eye size={11} /> X-Ray
          </button>
          <button
            type="button"
            className={`vp-btn ${showRefinementBoxes ? "active" : ""}`}
            onClick={() => setShowRefinementBoxes((v) => !v)}
            title="Toggle snappyHexMesh 3D refinement zones"
          >
            <Layers size={11} /> Zones{zoneCount > 0 ? ` (${zoneCount})` : ""}
          </button>
        </div>

        <div
          className="vp-pill-group mono"
          style={{ padding: "4px 9px", fontSize: 10, color: "var(--text-muted)", gap: 8 }}
        >
          <span style={{ color: "#f43f5e" }}>X</span>
          <span style={{ color: "#10b981" }}>Y</span>
          <span style={{ color: "#60a5fa" }}>Z</span>
          <span>· Drag: Orbit · Shift/Right: Pan · Scroll: Zoom</span>
        </div>
      </div>
    </div>
  );
};
