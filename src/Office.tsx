import { useEffect, useMemo, useRef, useState } from "react";
import * as THREE from "three";
import { OrbitControls } from "three/examples/jsm/controls/OrbitControls.js";

type Agent = {
  id: string;
  name: string;
  role?: string;
  color?: string;
  supervisorId?: string | null;
};
type Props = {
  agents: Agent[];
  selectedId: string;
  onSelect: (id: string) => void;
  runningIds?: string[];
  quality?: string;
};
type Label = { id: string; x: number; y: number };

/** A bounded, low-cost office. Its people and activity always come from saved agents/tasks. */
export default function Office({
  agents,
  selectedId,
  onSelect,
  runningIds = [],
  quality = "efficient",
}: Props) {
  const host = useRef<HTMLDivElement>(null);
  const selectRef = useRef(onSelect);
  const selectedRef = useRef(selectedId);
  const runningRef = useRef(runningIds);
  const resetRef = useRef<() => void>(() => {});
  const [labels, setLabels] = useState<Label[]>([]);
  const [failed, setFailed] = useState(false);
  selectRef.current = onSelect;
  selectedRef.current = selectedId;
  runningRef.current = runningIds;
  const agentKey = JSON.stringify(
    agents.map((a) => [a.id, a.name, a.color, a.supervisorId]),
  );
  const visibleAgents = useMemo(() => agents.slice(0, 12), [agentKey]);

  useEffect(() => {
    const element = host.current;
    if (!element) return;
    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({
        antialias: quality === "balanced",
        alpha: false,
        powerPreference: "low-power",
      });
    } catch {
      setFailed(true);
      return;
    }
    setFailed(false);
    const reduced = window.matchMedia(
      "(prefers-reduced-motion: reduce)",
    ).matches;
    const scene = new THREE.Scene();
    scene.background = new THREE.Color("#121418");
    scene.fog = new THREE.Fog("#121418", 28, 68);
    renderer.setPixelRatio(
      Math.min(window.devicePixelRatio || 1, quality === "balanced" ? 1.5 : 1),
    );
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.25;
    renderer.domElement.style.cssText =
      "display:block;width:100%;height:100%;touch-action:none";
    renderer.domElement.setAttribute(
      "aria-label",
      "Interactive office. Select an agent using its name button. Drag to orbit; scroll to zoom.",
    );
    renderer.domElement.setAttribute("role", "img");
    renderer.domElement.setAttribute("data-testid", "office-canvas");
    element.appendChild(renderer.domElement);
    const camera = new THREE.OrthographicCamera(-10, 10, 8, -8, 0.1, 100);
    camera.position.set(14, 14, 17);
    const controls = new OrbitControls(camera, renderer.domElement);
    controls.target.set(0, 0.3, 0);
    controls.enableDamping = !reduced;
    controls.dampingFactor = 0.12;
    controls.enablePan = false;
    controls.minPolarAngle = Math.PI / 5;
    controls.maxPolarAngle = Math.PI / 2.8;
    controls.minAzimuthAngle = -0.1;
    controls.maxAzimuthAngle = Math.PI / 2;
    controls.minZoom = 0.7;
    controls.maxZoom = 1.8;
    controls.update();
    controls.saveState();
    resetRef.current = () => controls.reset();
    const materials = new Map<string, THREE.MeshStandardMaterial>();
    const material = (color: string, metalness = 0.05, roughness = 0.6) => {
      const key = `${color}:${metalness}:${roughness}`;
      if (!materials.has(key))
        materials.set(
          key,
          new THREE.MeshStandardMaterial({ color, metalness, roughness }),
        );
      return materials.get(key)!;
    };
    const boxGeometry = new THREE.BoxGeometry(1, 1, 1);
    const sphereGeometry = new THREE.SphereGeometry(1, 12, 10);
    const cylinderGeometry = new THREE.CylinderGeometry(1, 1, 1, 12);
    const capsuleGeometry = new THREE.CapsuleGeometry(0.15, 0.3, 3, 8);
    const ringGeometry = new THREE.TorusGeometry(0.44, 0.025, 6, 28);
    const meshes: THREE.Mesh[] = [];
    function box(
      parent: THREE.Object3D,
      x: number,
      y: number,
      z: number,
      sx: number,
      sy: number,
      sz: number,
      color: string,
    ) {
      const mesh = new THREE.Mesh(boxGeometry, material(color));
      mesh.position.set(x, y, z);
      mesh.scale.set(sx, sy, sz);
      parent.add(mesh);
      meshes.push(mesh);
      return mesh;
    }
    function cylinder(
      parent: THREE.Object3D,
      x: number,
      y: number,
      z: number,
      radius: number,
      height: number,
      color: string,
    ) {
      const mesh = new THREE.Mesh(cylinderGeometry, material(color));
      mesh.position.set(x, y, z);
      mesh.scale.set(radius, height, radius);
      parent.add(mesh);
      meshes.push(mesh);
      return mesh;
    }
    function ball(
      parent: THREE.Object3D,
      x: number,
      y: number,
      z: number,
      radius: number,
      color: string,
    ) {
      const mesh = new THREE.Mesh(sphereGeometry, material(color));
      mesh.position.set(x, y, z);
      mesh.scale.setScalar(radius);
      parent.add(mesh);
      meshes.push(mesh);
      return mesh;
    }
    scene.add(new THREE.HemisphereLight("#f4f7ff", "#383b45", 2.2));
    const light = new THREE.DirectionalLight("#ffffff", 3.2);
    light.position.set(-8, 16, 5);
    scene.add(light);
    const fill = new THREE.DirectionalLight("#b8c9e7", 1.1);
    fill.position.set(10, 8, -8);
    scene.add(fill);
    // The room is deliberately open at the front so every interaction target is visible.
    const room = new THREE.Group();
    scene.add(room);
    const rows = Math.max(2, Math.ceil(Math.min(visibleAgents.length, 12) / 3));
    const depth = Math.max(9.2, rows * 2.6 + 3.3);
    box(room, 0, -0.28, 0, 11.8, 0.45, depth, "#262a31");
    box(room, 0, -0.025, 0, 11.5, 0.06, depth - 0.25, "#b5b7bb");
    // Subtle floor grid, a dark border, and bright cut edges establish depth without shadows.
    for (let x = -5; x <= 5; x += 1)
      box(room, x, 0.01, 0, 0.012, 0.012, depth - 0.25, "#9da0a6");
    for (let z = -depth / 2 + 0.5; z < depth / 2; z++)
      box(room, 0, 0.01, z, 11.5, 0.012, 0.012, "#9da0a6");
    box(room, 0, 0.025, depth / 2 - 0.12, 11.6, 0.035, 0.05, "#f1f4f8");
    box(room, -5.8, 0.025, 0, 0.05, 0.035, depth, "#e8ebef");
    const back = -depth / 2 + 0.1;
    box(room, 0, 1.35, back, 11.8, 2.7, 0.15, "#292d34");
    // Window recess and frame. Static highlights approximate a glass studio wall cheaply.
    box(room, -0.8, 1.65, back + 0.11, 7.6, 1.5, 0.06, "#53606a");
    box(room, -0.8, 1.65, back + 0.16, 7.4, 1.35, 0.025, "#87959d");
    for (let x = -4.5; x <= 3; x += 1.85)
      box(room, x, 1.65, back + 0.2, 0.045, 1.5, 0.04, "#23282f");
    box(room, -0.8, 1, back + 0.24, 7.6, 0.07, 0.32, "#d3d7db");
    // Storage wall, slim lights, and planted corners make the office an intentional room.
    for (let i = 0; i < 3; i++) {
      const x = 3.7 + i * 0.58;
      box(
        room,
        x,
        1.05,
        back + 0.36,
        0.55,
        2.1,
        0.45,
        i === 1 ? "#3b4048" : "#353941",
      );
      box(room, x + 0.17, 1.05, back + 0.6, 0.018, 0.32, 0.03, "#aeb6c0");
    }
    for (let i = 0; i < 3; i++)
      box(room, -3.6 + i * 3.4, 2.53, back + 0.18, 2.4, 0.045, 0.06, "#f8fafc");
    function plant(x: number, z: number) {
      cylinder(room, x, 0.24, z, 0.26, 0.48, "#41474e");
      cylinder(room, x, 0.64, z, 0.035, 0.8, "#51555b");
      for (let i = 0; i < 6; i++) {
        const angle = (i / 6) * Math.PI * 2;
        const leaf = ball(
          room,
          x + Math.sin(angle) * 0.18,
          0.9 + (i % 2) * 0.16,
          z + Math.cos(angle) * 0.18,
          0.25,
          "#6a6d72",
        );
        leaf.scale.set(0.16, 0.32, 0.11);
        leaf.rotation.z = Math.sin(angle) * 0.6;
      }
    }
    plant(-5, back + 1.05);
    plant(5, depth / 2 - 0.85);
    // Glass meeting zone at the side: thin panes and bright edge catches, no expensive transmission.
    const glassMaterial = new THREE.MeshPhysicalMaterial({
      color: "#d4e4ed",
      transparent: true,
      opacity: 0.16,
      roughness: 0.08,
      metalness: 0.1,
      side: THREE.DoubleSide,
      depthWrite: false,
    });
    const glass = new THREE.Mesh(boxGeometry, glassMaterial);
    glass.position.set(-4.65, 1.05, -0.25);
    glass.scale.set(0.035, 2.1, Math.min(5.6, depth - 3));
    room.add(glass);
    for (const z of [-2.95, 2.45]) {
      box(room, -4.65, 1.05, z, 0.035, 2.1, 0.035, "#dce4eb");
      box(room, -4.65, 0.05, z, 0.18, 0.05, 0.18, "#666e78");
    }
    box(room, -4.65, 2.1, -0.25, 0.04, 0.04, 5.45, "#c2ccd6");
    // Each station belongs to one saved agent; unused positions remain visibly vacant.
    const anchorPoints: { id: string; point: THREE.Vector3 }[] = [];
    const selectionRings: {
      id: string;
      mesh: THREE.Mesh;
      indicator: THREE.Mesh;
    }[] = [];
    const stations = Math.max(6, visibleAgents.length);
    for (let i = 0; i < stations; i++) {
      const agent = visibleAgents[i];
      const row = Math.floor(i / 3),
        column = i % 3;
      const x = -2.75 + column * 3.05;
      const z = (row - (rows - 1) / 2) * 2.6 + 0.35;
      const station = new THREE.Group();
      station.position.set(x, 0, z);
      if (agent) station.userData.agentId = agent.id;
      room.add(station);
      // Inset carpet pad, suspended white desk, modest monitor and keyboard.
      box(station, 0, 0.035, 0.15, 2.55, 0.025, 2.18, "#a1a4aa");
      box(station, 0.12, 0.85, -0.28, 2.22, 0.11, 1.02, "#e3e5e9");
      for (const legX of [-0.78, 1.0])
        box(station, legX, 0.42, -0.28, 0.07, 0.8, 0.8, "#3c424a");
      box(station, 0.26, 1.26, -0.54, 0.94, 0.55, 0.055, "#252a31");
      box(station, 0.26, 1.26, -0.505, 0.85, 0.45, 0.012, "#46535d");
      box(station, 0.26, 1.26, -0.495, 0.69, 0.014, 0.006, "#758c9a");
      box(station, 0.26, 1.36, -0.493, 0.69, 0.012, 0.006, "#94a9b5");
      box(station, 0.26, 1.15, -0.493, 0.45, 0.012, 0.006, "#758c9a");
      box(station, 0.26, 0.99, -0.54, 0.05, 0.2, 0.05, "#4c525a");
      box(station, 0.26, 0.92, -0.54, 0.32, 0.025, 0.21, "#6e7681");
      box(station, 0.28, 0.92, -0.05, 0.64, 0.025, 0.2, "#838b96");
      box(station, 0.82, 0.92, -0.05, 0.09, 0.025, 0.15, "#616b77");
      cylinder(station, -0.66, 0.99, -0.53, 0.07, 0.15, "#818b94");
      // Chair is present even at an empty desk.
      box(station, 0.24, 0.52, 0.66, 0.58, 0.12, 0.54, "#3d454f");
      box(station, 0.24, 0.85, 0.94, 0.58, 0.55, 0.09, "#414a55");
      cylinder(station, 0.24, 0.28, 0.66, 0.035, 0.46, "#65717e");
      box(station, 0.24, 0.06, 0.66, 0.72, 0.04, 0.06, "#54606d");
      box(station, 0.24, 0.06, 0.66, 0.06, 0.04, 0.72, "#54606d");
      if (agent) {
        const color = /^#[0-9a-f]{6}$/i.test(agent.color || "")
          ? agent.color!
          : "#a4b8dc";
        // Figure to the left of its desk; silhouettes remain distinct at this small scale.
        const person = new THREE.Group();
        person.position.set(-0.88, 0, 0.7);
        station.add(person);
        box(person, -0.105, 0.24, 0, 0.13, 0.4, 0.17, "#333a44");
        box(person, 0.105, 0.24, 0, 0.13, 0.4, 0.17, "#333a44");
        box(person, -0.105, 0.055, 0.065, 0.15, 0.09, 0.28, "#1b2027");
        box(person, 0.105, 0.055, 0.065, 0.15, 0.09, 0.28, "#1b2027");
        const torso = new THREE.Mesh(
          capsuleGeometry,
          material(i % 2 ? "#eceef2" : "#737e8b"),
        );
        torso.position.y = 0.68;
        torso.scale.set(1.3, 1, 0.82);
        person.add(torso);
        ball(person, 0, 1.1, 0, 0.19, "#ccd0d5");
        const hair = ball(
          person,
          0,
          1.2,
          -0.025,
          0.18,
          i % 2 ? "#363e49" : "#6b737e",
        );
        hair.scale.y = 0.65;
        box(person, -0.068, 1.105, 0.175, 0.035, 0.028, 0.014, "#333b46");
        box(person, 0.068, 1.105, 0.175, 0.035, 0.028, 0.014, "#333b46");
        for (const armX of [-0.25, 0.25]) {
          const arm = new THREE.Mesh(capsuleGeometry, material("#b2bac4"));
          arm.position.set(armX, 0.69, 0.03);
          arm.scale.set(0.4, 0.7, 0.5);
          arm.rotation.z = -armX * 0.7;
          person.add(arm);
        }
        box(person, 0, 0.76, 0.145, 0.22, 0.17, 0.035, color);
        const ring = new THREE.Mesh(
          ringGeometry,
          new THREE.MeshBasicMaterial({
            color,
            transparent: true,
            opacity: 0.35,
          }),
        );
        ring.rotation.x = -Math.PI / 2;
        ring.position.set(-0.88, 0.065, 0.7);
        station.add(ring);
        const indicator = new THREE.Mesh(
          sphereGeometry,
          new THREE.MeshBasicMaterial({ color }),
        );
        indicator.position.set(1.04, 0.98, -0.54);
        indicator.scale.setScalar(0.055);
        station.add(indicator);
        selectionRings.push({ id: agent.id, mesh: ring, indicator });
        anchorPoints.push({
          id: agent.id,
          point: new THREE.Vector3(x - 0.7, 1.75, z + 0.85),
        });
      }
    }
    const raycaster = new THREE.Raycaster();
    const mouse = new THREE.Vector2();
    let down: { x: number; y: number } | null = null;
    const pointerDown = (event: PointerEvent) => {
      down = { x: event.clientX, y: event.clientY };
    };
    const pointerUp = (event: PointerEvent) => {
      if (
        !down ||
        Math.hypot(event.clientX - down.x, event.clientY - down.y) > 6
      )
        return;
      const bounds = renderer.domElement.getBoundingClientRect();
      mouse.set(
        ((event.clientX - bounds.left) / bounds.width) * 2 - 1,
        (-(event.clientY - bounds.top) / bounds.height) * 2 + 1,
      );
      raycaster.setFromCamera(mouse, camera);
      for (const hit of raycaster.intersectObjects(scene.children, true)) {
        let object: THREE.Object3D | null = hit.object;
        while (object) {
          if (object.userData.agentId) {
            selectRef.current(object.userData.agentId);
            return;
          }
          object = object.parent;
        }
      }
    };
    renderer.domElement.addEventListener("pointerdown", pointerDown);
    renderer.domElement.addEventListener("pointerup", pointerUp);
    const resize = () => {
      const width = element.clientWidth || 800,
        height = element.clientHeight || 560;
      const halfHeight = Math.max(6.4, depth * 0.49, 8.2 / (width / height));
      camera.left = (-halfHeight * width) / height;
      camera.right = (halfHeight * width) / height;
      camera.top = halfHeight;
      camera.bottom = -halfHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(width, height, false);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    resize();
    let frame = 0,
      last = 0,
      lastLabels = "";
    let disposed = false;
    const render = (now: number) => {
      if (disposed) return;
      frame = requestAnimationFrame(render);
      if (document.hidden || now - last < 1000 / 30) return;
      last = now;
      controls.update();
      for (const ring of selectionRings) {
        const selected = ring.id === selectedRef.current;
        (ring.mesh.material as THREE.MeshBasicMaterial).opacity = selected
          ? 1
          : 0.25;
        ring.mesh.scale.setScalar(selected ? 1.2 : 1);
        const busy = runningRef.current.includes(ring.id);
        ring.indicator.visible = busy || selected;
        ring.indicator.scale.setScalar(
          busy && !reduced ? 0.055 + Math.sin(now / 600) * 0.008 : 0.055,
        );
      }
      renderer.render(scene, camera);
      const width = element.clientWidth,
        height = element.clientHeight;
      const next = anchorPoints.map((a) => {
        const projected = a.point.clone().project(camera);
        return {
          id: a.id,
          x: Math.round(((projected.x + 1) * width) / 2),
          y: Math.round(((-projected.y + 1) * height) / 2),
        };
      });
      const key = JSON.stringify(next);
      if (key !== lastLabels) {
        lastLabels = key;
        setLabels(next);
      }
    };
    frame = requestAnimationFrame(render);
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      controls.dispose();
      renderer.domElement.removeEventListener("pointerdown", pointerDown);
      renderer.domElement.removeEventListener("pointerup", pointerUp);
      scene.traverse((object) => {
        const mesh = object as THREE.Mesh;
        if (mesh.isMesh && mesh.material) {
          if (Array.isArray(mesh.material))
            mesh.material.forEach((m) => m.dispose());
          else mesh.material.dispose();
        }
      });
      boxGeometry.dispose();
      sphereGeometry.dispose();
      cylinderGeometry.dispose();
      capsuleGeometry.dispose();
      ringGeometry.dispose();
      glassMaterial.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
      if (renderer.domElement.parentNode === element)
        element.removeChild(renderer.domElement);
    };
  }, [agentKey, quality]);

  return (
    <div
      style={{
        position: "relative",
        width: "100%",
        height: "100%",
        minHeight: 420,
        overflow: "hidden",
        background: "#121418",
      }}
    >
      <div ref={host} style={{ position: "absolute", inset: 0 }} />
      {!failed && (
        <>
          <div
            style={{
              position: "absolute",
              left: 24,
              top: 22,
              pointerEvents: "none",
              color: "#a4aab4",
              fontSize: 10,
              letterSpacing: "1.8px",
              fontWeight: 600,
            }}
          >
            WORKSPACE 01{" "}
            <span style={{ color: "#4e5662", margin: "0 8px" }}>/</span> TEAM
            FLOOR
          </div>
          <div
            style={{
              position: "absolute",
              right: 18,
              bottom: 18,
              display: "flex",
              gap: 8,
              alignItems: "center",
            }}
          >
            <span
              style={{
                fontSize: 10,
                color: "#8e97a5",
                background: "#15191ee8",
                padding: "8px 12px",
                borderRadius: 6,
                border: "1px solid #303641",
              }}
            >
              Drag to orbit · Scroll to zoom
            </span>
            <button
              type="button"
              onClick={() => resetRef.current()}
              title="Reset office view"
              aria-label="Reset office view"
              style={{
                width: 44,
                height: 44,
                background: "#1f252de8",
                border: "1px solid #3d4653",
                color: "#e6eaf0",
                borderRadius: 9,
                cursor: "pointer",
                fontSize: 20,
              }}
            >
              ↺
            </button>
          </div>
          {labels.map((label) => {
            const agent = visibleAgents.find((a) => a.id === label.id);
            if (!agent) return null;
            const selected = selectedId === agent.id;
            const running = runningIds.includes(agent.id);
            return (
              <button
                key={agent.id}
                type="button"
                onClick={() => onSelect(agent.id)}
                aria-pressed={selected}
                aria-label={`Message ${agent.name}${running ? ", working" : ""}`}
                style={{
                  position: "absolute",
                  left: label.x,
                  top: label.y,
                  transform: "translate(-50%, -100%)",
                  display: "flex",
                  gap: 7,
                  alignItems: "center",
                  minHeight: 44,
                  maxWidth: 160,
                  padding: "8px 12px",
                  border: `1px solid ${selected ? "#aab5c7" : "#414954"}`,
                  borderRadius: 8,
                  color: selected ? "#ffffff" : "#d5dbe4",
                  background: selected ? "#202630f5" : "#1c222be8",
                  boxShadow: "0 3px 12px #0004",
                  fontSize: 11,
                  fontWeight: selected ? 650 : 500,
                  cursor: "pointer",
                  whiteSpace: "nowrap",
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    width: 6,
                    height: 6,
                    flexShrink: 0,
                    borderRadius: "50%",
                    background: agent.color || "#b8c2d5",
                    boxShadow: running
                      ? `0 0 8px ${agent.color || "#b8c2d5"}`
                      : "none",
                  }}
                />
                <span style={{ overflow: "hidden", textOverflow: "ellipsis" }}>
                  {agent.name}
                </span>
                {running && (
                  <span style={{ color: "#b8c2d1", fontSize: 9 }}>Working</span>
                )}
              </button>
            );
          })}
          {agents.length === 0 && (
            <div
              style={{
                position: "absolute",
                left: "50%",
                top: "48%",
                transform: "translate(-50%, -50%)",
                background: "#12161ce8",
                border: "1px solid #464d59",
                borderRadius: 12,
                padding: "20px 26px",
                textAlign: "center",
                color: "#f1f3f7",
                pointerEvents: "none",
                boxShadow: "0 20px 50px #0004",
              }}
            >
              <strong style={{ fontSize: 15 }}>
                A place for your next idea.
              </strong>
              <div style={{ fontSize: 12, marginTop: 8, color: "#b4bcc9" }}>
                Create your first agent to bring the office to life.
              </div>
            </div>
          )}
          {agents.length > 12 && (
            <div
              style={{
                position: "absolute",
                left: 20,
                bottom: 20,
                color: "#bbc4d0",
                fontSize: 11,
              }}
            >
              Showing 12 of {agents.length} agents · All agents are available in
              the team list.
            </div>
          )}
        </>
      )}
      {failed && (
        <div
          style={{
            position: "absolute",
            inset: 0,
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            flexDirection: "column",
            padding: 36,
            gap: 12,
            color: "#e4e9f0",
            textAlign: "center",
          }}
        >
          <strong>3D is unavailable on this device.</strong>
          <span style={{ color: "#a8b1bf", fontSize: 13 }}>
            Use the team list to select and message any agent. All task controls
            remain available.
          </span>
          <div
            style={{
              display: "flex",
              flexWrap: "wrap",
              gap: 8,
              justifyContent: "center",
            }}
          >
            {agents.map((a) => (
              <button
                key={a.id}
                type="button"
                onClick={() => onSelect(a.id)}
                style={{
                  minHeight: 44,
                  padding: "8px 14px",
                  color: "#fff",
                  background: "#282e38",
                  border: "1px solid #495567",
                  borderRadius: 8,
                  cursor: "pointer",
                }}
              >
                {a.name}
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
