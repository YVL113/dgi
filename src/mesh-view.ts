import {
  BufferAttribute, BufferGeometry, DirectionalLight, DoubleSide, HemisphereLight, Mesh,
  MeshStandardMaterial, type Object3D, PerspectiveCamera, Scene, Spherical, WebGLRenderer,
} from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';
import { button } from './ui';
import './rig.css';

function readGeometry(bytes: ArrayBuffer) {
  if (bytes.byteLength < 12) throw new Error('The decoded mesh is incomplete.');
  const header = new DataView(bytes);
  const vertices = header.getUint32(0, true);
  const faces = header.getUint32(4, true);
  const normals = header.getUint32(8, true);
  const indexOffset = 12 + vertices * 12 * (normals + 1);
  if (!vertices || !faces || normals > 1 || bytes.byteLength !== indexOffset + faces * 12) {
    throw new Error('The decoded mesh has invalid geometry.');
  }
  const geometry = new BufferGeometry();
  geometry.setAttribute('position', new BufferAttribute(new Float32Array(bytes, 12, vertices * 3), 3));
  geometry.setIndex(new BufferAttribute(new Uint32Array(bytes, indexOffset, faces * 3), 1));
  if (normals) {
    geometry.setAttribute('normal', new BufferAttribute(new Float32Array(bytes, 12 + vertices * 12, vertices * 3), 3));
  } else geometry.computeVertexNormals();
  geometry.computeBoundingSphere();
  const bounds = geometry.boundingSphere!;
  if (!bounds.radius || !Number.isFinite(bounds.radius)) {
    geometry.dispose();
    throw new Error('This mesh has no visible size.');
  }
  geometry.translate(-bounds.center.x, -bounds.center.y, -bounds.center.z);
  geometry.scale(1 / bounds.radius, 1 / bounds.radius, 1 / bounds.radius);
  return { geometry, vertices, faces };
}

export function mountMesh(container: HTMLElement, bytes: ArrayBuffer): () => void {
  const { geometry, vertices, faces } = readGeometry(bytes);
  const material = new MeshStandardMaterial({ color: 0xa0a0a0, roughness: 0.85, side: DoubleSide });
  return mountOrbitPreview(container, new Mesh(geometry, material), 'Mesh',
    `${vertices.toLocaleString()} vertices · ${faces.toLocaleString()} triangles`,
    () => { geometry.dispose(); material.dispose(); });
}

export function mountOrbitPreview(
  container: HTMLElement, object: Object3D, type: string, description: string, disposeObject: () => void,
): () => void {
  const canvas = document.createElement('canvas');
  canvas.tabIndex = 0;
  canvas.setAttribute('aria-label', type + ' preview. Drag or use arrow keys to rotate. Scroll to zoom.');
  const context = canvas.getContext('webgl2', { alpha: true, antialias: true });
  if (!context) {
    disposeObject();
    throw new Error('WebGL 2 is unavailable. 3D previews require graphics acceleration.');
  }
  const renderer = new WebGLRenderer({ canvas, context, alpha: true, antialias: true });
  const scene = new Scene();
  scene.add(object, new HemisphereLight(0xffffff, 0x969696, 1.55));
  const key = new DirectionalLight(0xffffff, 2);
  key.position.set(-2, 4, 3);
  scene.add(key);
  const fill = new DirectionalLight(0xffffff, 0.8);
  fill.position.set(3, 1, -4);
  scene.add(fill);
  const camera = new PerspectiveCamera(35, 1, 0.02, 80);
  camera.position.set(2.4, 1.35, -3.4);
  const controls = new OrbitControls(camera, canvas);
  controls.enablePan = false;
  controls.minDistance = 1.6;
  controls.maxDistance = 14;
  controls.update();
  controls.saveState();

  const view = document.createElement('div');
  view.className = 'mesh-preview';
  const stage = document.createElement('div');
  stage.className = 'rig-stage';
  stage.append(canvas);
  const toolbar = document.createElement('div');
  toolbar.className = 'mesh-controls';
  const reset = button('Reset view', { variant: 'secondary', size: 'small' });
  const summary = document.createElement('span');
  summary.className = 'muted mesh-summary';
  summary.textContent = description;
  toolbar.append(reset, summary);
  view.append(stage, toolbar);
  container.replaceChildren(view);

  const life = new AbortController();
  const options = { signal: life.signal };
  let frame: number | undefined;
  function requestFrame() {
    if (frame !== undefined) return;
    frame = requestAnimationFrame(() => {
      frame = undefined;
      renderer.render(scene, camera);
    });
  }

  const observer = new ResizeObserver(([entry]) => {
    const { width, height } = entry.contentRect;
    if (!width || !height) return;
    renderer.setPixelRatio(Math.min(devicePixelRatio || 1, 2));
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    camera.updateProjectionMatrix();
    requestFrame();
  });
  observer.observe(canvas);
  controls.addEventListener('change', requestFrame);
  canvas.addEventListener('webglcontextrestored', requestFrame, options);
  reset.addEventListener('click', () => controls.reset(), options);
  canvas.addEventListener('keydown', event => {
    const movement = { ArrowLeft: [-0.14, 0], ArrowRight: [0.14, 0], ArrowUp: [0, -0.14], ArrowDown: [0, 0.14] }[event.key];
    if (!movement) return;
    event.preventDefault();
    const orbit = new Spherical().setFromVector3(camera.position);
    orbit.theta += movement[0];
    orbit.phi += movement[1];
    camera.position.setFromSpherical(orbit.makeSafe());
    controls.update();
  }, options);

  return () => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    life.abort();
    observer.disconnect();
    controls.dispose();
    disposeObject();
    renderer.dispose();
    renderer.forceContextLoss();
    view.remove();
  };
}
