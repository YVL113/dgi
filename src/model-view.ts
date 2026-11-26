import {
  Box3, BoxGeometry, type BufferGeometry, Color, CylinderGeometry, Group, Matrix4, Mesh,
  MeshStandardMaterial, Sphere, SphereGeometry, SRGBColorSpace, Vector3,
} from 'three';
import { ConvexGeometry } from 'three/addons/geometries/ConvexGeometry.js';
import { mountOrbitPreview } from './mesh-view';

type Vector = [number, number, number];
type Shape = 'ball' | 'block' | 'cylinder' | 'wedge' | 'bounds';

interface Part {
  shape: Shape;
  size: Vector;
  position: Vector;
  rotation: [Vector, Vector, Vector];
  color: Vector;
  opacity: number;
}

interface ModelPreview {
  parts: Part[];
  objects: { name: string; class: string; depth: number; references: string[] }[];
  objectCount: number;
  omittedParts: number;
}

function geometryFor(shape: Shape): BufferGeometry {
  if (shape === 'ball') return new SphereGeometry(.5, 24, 16);
  if (shape === 'cylinder') return new CylinderGeometry(.5, .5, 1, 24).rotateZ(Math.PI / 2);
  if (shape === 'wedge') {
    const vertices = [
      [-.5, -.5, -.5], [.5, -.5, -.5], [-.5, -.5, .5],
      [.5, -.5, .5], [-.5, .5, .5], [.5, .5, .5],
    ];
    return new ConvexGeometry(vertices.map(([x, y, z]) => new Vector3(x, y, z)));
  }
  return new BoxGeometry(1, 1, 1);
}

export function mountModel(container: HTMLElement, bytes: ArrayBuffer, playbackError?: string): () => void {
  const model: ModelPreview = JSON.parse(new TextDecoder().decode(bytes));
  const view = document.createElement('div');
  view.className = 'model-preview';
  const details = document.createElement('details');
  const heading = document.createElement('summary');
  heading.textContent = model.objectCount.toLocaleString() + (model.objectCount === 1 ? ' object' : ' objects');
  const list = document.createElement('ul');
  list.className = 'model-object-list';
  for (const object of model.objects) {
    const row = document.createElement('li');
    row.style.paddingInlineStart = Math.min(object.depth, 6) * 10 + 'px';
    const name = document.createElement('strong');
    name.textContent = object.name || object.class;
    const type = document.createElement('span');
    type.className = 'muted';
    type.textContent = [object.class, ...object.references].join(' · ');
    row.append(name, type);
    list.append(row);
  }
  if (model.objects.length < model.objectCount) {
    const more = document.createElement('li');
    more.className = 'muted';
    more.textContent = (model.objectCount - model.objects.length).toLocaleString() + ' more objects';
    list.append(more);
  }
  details.append(heading, list);
  view.append(details);
  const note = document.createElement('p');
  note.className = 'model-preview-note muted';
  let disposeOrbit: (() => void) | undefined;
  if (model.parts.length) {
    const viewport = document.createElement('div');
    view.prepend(viewport);
    try {
      disposeOrbit = mountParts(viewport, model);
    } catch {
      viewport.remove();
    }
  }
  if (!disposeOrbit) {
    details.open = true;
    note.textContent = playbackError || (model.parts.length ? '3D preview unavailable. Showing saved objects.' :
      'No renderable geometry in this file. Showing saved objects and asset references.');
    view.prepend(note);
  }
  container.replaceChildren(view);
  return () => disposeOrbit?.();
}

function mountParts(viewport: HTMLElement, model: ModelPreview) {
  const modelGroup = new Group();
  const geometries = new Map<Shape, BufferGeometry>();
  const materials: MeshStandardMaterial[] = [];
  const rotation = new Matrix4();
  for (const part of model.parts) {
    let geometry = geometries.get(part.shape);
    if (!geometry) {
      geometry = geometryFor(part.shape);
      geometries.set(part.shape, geometry);
    }
    const material = new MeshStandardMaterial({
      color: new Color().setRGB(...part.color, SRGBColorSpace),
      roughness: .8,
      wireframe: part.shape === 'bounds',
      opacity: part.opacity,
      transparent: part.opacity < 1,
    });
    materials.push(material);
    const mesh = new Mesh(geometry, material);
    const [x, y, z] = part.rotation;
    rotation.set(...x, 0, ...y, 0, ...z, 0, 0, 0, 0, 1);
    mesh.setRotationFromMatrix(rotation);
    mesh.position.set(...part.position);
    mesh.scale.set(...part.size);
    modelGroup.add(mesh);
  }
  const dispose = () => {
    for (const geometry of geometries.values()) geometry.dispose();
    for (const material of materials) material.dispose();
  };
  const bounds = new Box3().setFromObject(modelGroup).getBoundingSphere(new Sphere());
  if (!Number.isFinite(bounds.radius) || bounds.radius <= 0) {
    dispose();
    throw new Error('The model has no visible size.');
  }
  modelGroup.position.copy(bounds.center).negate();
  const normalized = new Group();
  normalized.scale.setScalar(1 / bounds.radius);
  normalized.add(modelGroup);
  const description = [
    model.parts.length.toLocaleString() + (model.parts.length === 1 ? ' part' : ' parts'),
    model.parts.some(part => part.shape === 'bounds') ? 'Wireframes show part bounds' : '',
    model.omittedParts ? model.omittedParts.toLocaleString() + ' more parts' : '',
  ].filter(Boolean).join(' · ');
  return mountOrbitPreview(viewport, normalized, 'Model', description, dispose);
}
