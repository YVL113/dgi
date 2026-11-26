import {
  BoxGeometry, BufferGeometry, CircleGeometry, Color, DirectionalLight, Float32BufferAttribute, Group,
  HemisphereLight, LatheGeometry, Mesh, MeshBasicMaterial, MeshStandardMaterial, OrthographicCamera,
  Scene, SRGBColorSpace, Texture, Vector2, Vector3, WebGLRenderer,
} from "three";
import { mergeVertices } from "three/addons/utils/BufferGeometryUtils.js";
import type { Geometry } from "./rig-mesh";

type Vector = readonly [number, number, number];
type Rotation = readonly [number, number, number, number];

interface RenderPart {
  name: string;
  parent: string;
  size: Vector;
  material: "body" | "limb" | "head";
  geometry?: Geometry;
}

const orbitDistance = 18;
const fallbackColors = { body: 0x73777e, limb: 0xa3a7ae, head: 0xc4c7cc };

function studioGeometry(source: Geometry, decal = false) {
  const geometry = new BufferGeometry();
  const faces = decal ? source.faces.filter(face => face.normal[2] < -0.1) : source.faces;
  geometry.setAttribute("position", new Float32BufferAttribute(source.vertices.flat(), 3));
  geometry.setIndex(faces.flatMap(face => face.vertices));
  return geometry;
}

function smoothAcrossUvSeams(geometry: BufferGeometry) {
  // Weld positions for the normal calculation, then use the original UV splits for rendering.
  const welded = mergeVertices(geometry);
  welded.computeVertexNormals();
  const normals = welded.getAttribute("normal");
  const weldedIndex = welded.getIndex()!;
  const index = geometry.getIndex()!;
  const output = new Float32BufferAttribute(new Float32Array(geometry.getAttribute("position").count * 3), 3);
  for (let corner = 0; corner < index.count; corner++) {
    const vertex = weldedIndex.getX(corner);
    output.setXYZ(index.getX(corner), normals.getX(vertex), normals.getY(vertex), normals.getZ(vertex));
  }
  geometry.setAttribute("normal", output);
  welded.dispose();
}

function fallbackGeometry(part: RenderPart) {
  if (part.material !== "head") return new BoxGeometry(...part.size);
  const profile = [[0, -0.5], [0.36, -0.5], [0.5, -0.375], [0.5, 0.375], [0.36, 0.5], [0, 0.5]];
  return new LatheGeometry(profile.map(([radius, height]) => new Vector2(radius, height)), 32).scale(...part.size);
}

export function createRigView(canvas: HTMLCanvasElement, parts: RenderPart[], rootName: string, centerHeight: number) {
  const context = canvas.getContext("webgl2", { alpha: true, antialias: true });
  if (!context) throw new Error("WebGL 2 is unavailable. Animation previews require graphics acceleration.");
  const renderer = new WebGLRenderer({ canvas, context, alpha: true, antialias: true });
  const scene = new Scene();
  const camera = new OrthographicCamera(-1, 1, 1, -1, 0.1, 256);
  const focus = new Vector3();
  const root = new Group();
  const joints = new Map([[rootName, root]]);
  scene.add(root);

  scene.add(new HemisphereLight(0xffffff, 0xb6bac2, 1.5));
  const key = new DirectionalLight(0xffffff, 2.1);
  key.position.set(-3, 6, -4);
  scene.add(key);
  const fill = new DirectionalLight(0xffffff, 0.7);
  fill.position.set(4, 2, 4);
  scene.add(fill);

  const resources: { dispose(): void }[] = [];
  function keep<T extends { dispose(): void }>(resource: T): T {
    resources.push(resource);
    return resource;
  }

  function textureOf(source: NonNullable<Geometry["texture"]>) {
    const texture = keep(new Texture(source.image));
    // Rig-pack UV coordinates use the top-left of the decoded PNG as their origin.
    texture.flipY = false;
    texture.colorSpace = SRGBColorSpace;
    texture.anisotropy = Math.min(4, renderer.capabilities.getMaxAnisotropy());
    texture.needsUpdate = true;
    return texture;
  }

  const shadow = new Mesh(keep(new CircleGeometry(1, 48)),
    keep(new MeshBasicMaterial({ color: 0x0b0c0e, opacity: 0.10, transparent: true, depthWrite: false })));
  shadow.rotation.x = -Math.PI / 2;
  shadow.scale.set(1.35, 0.58, 1);
  scene.add(shadow);

  for (const part of parts) {
    const joint = new Group();
    joints.get(part.parent)!.add(joint);
    joints.set(part.name, joint);
    const source = part.geometry;
    const geometry = keep(source ? studioGeometry(source) : fallbackGeometry(part));
    const atlas = source?.texture;
    if (source && part.material === "head") smoothAcrossUvSeams(geometry);
    if (atlas) geometry.setAttribute("uv", new Float32BufferAttribute([...atlas.uv], 2));
    const color = new Color(fallbackColors[part.material]);
    if (atlas) color.setHex(0xffffff);
    else if (source?.color) {
      const [red, green, blue] = source.color;
      color.setRGB(red / 255, green / 255, blue / 255, SRGBColorSpace);
    }
    const material = keep(new MeshStandardMaterial({
      color, map: atlas ? textureOf(atlas) : null, roughness: 0.9, flatShading: part.material !== "head",
    }));
    joint.add(new Mesh(geometry, material));

    if (source?.frontDecal && !atlas) {
      const decal = keep(studioGeometry(source, true));
      decal.setAttribute("uv", new Float32BufferAttribute([...source.frontDecal.uv], 2));
      const ink = keep(new MeshBasicMaterial({
        map: textureOf(source.frontDecal), transparent: true, depthWrite: false,
        polygonOffset: true, polygonOffsetFactor: -1, polygonOffsetUnits: -1,
      }));
      joint.add(new Mesh(decal, ink));
    }
  }

  let viewportWidth = 0;
  let viewportHeight = 0;
  let viewportDensity = 0;
  return {
    setJoint(name: string, position: Vector, rotation: Rotation) {
      const joint = joints.get(name)!;
      joint.position.set(...position);
      joint.quaternion.set(...rotation);
    },
    draw(width: number, height: number, yaw: number, pitch: number) {
      const density = Math.min(devicePixelRatio || 1, 2);
      if (width !== viewportWidth || height !== viewportHeight || density !== viewportDensity) {
        viewportWidth = width;
        viewportHeight = height;
        if (density !== viewportDensity) {
          viewportDensity = density;
          renderer.setPixelRatio(density);
        }
        renderer.setSize(width, height, false);
        const zoom = Math.min(width / 6.1, height / (centerHeight * 2 + 1.2));
        camera.left = -width / (2 * zoom);
        camera.right = width / (2 * zoom);
        camera.top = height / (2 * zoom);
        camera.bottom = -camera.top;
        camera.updateProjectionMatrix();
      }
      focus.set(root.position.x, centerHeight, root.position.z);
      shadow.position.set(focus.x, 0, focus.z);
      const horizontal = Math.cos(pitch) * orbitDistance;
      camera.position.set(focus.x - Math.sin(yaw) * horizontal, focus.y + Math.sin(pitch) * orbitDistance,
        focus.z - Math.cos(yaw) * horizontal);
      camera.lookAt(focus);
      renderer.render(scene, camera);
    },
    dispose() {
      for (const resource of resources) resource.dispose();
      renderer.dispose();
      renderer.forceContextLoss();
    },
  };
}
