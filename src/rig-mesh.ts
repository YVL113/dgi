import { invoke } from "@tauri-apps/api/core";
import studioRigs from "./studio-rigs.json";

type Vector = readonly [number, number, number];
type RigType = "R6" | "R15";

interface MeshTexture {
  image: HTMLImageElement | HTMLCanvasElement;
  uv: readonly number[];
}

export interface Geometry {
  vertices: readonly Vector[];
  faces: ReadonlyArray<{ vertices: readonly number[]; normal: Vector }>;
  color?: Vector;
  frontDecal?: MeshTexture;
  texture?: MeshTexture;
}

export interface StudioPart {
  parent: string;
  size: number[];
  c0: number[];
  c1: number[];
}

export interface StudioMeshes {
  parts: ReadonlyMap<string, Geometry>;
  r6Rig?: Record<string, StudioPart>;
  r15Rig?: Record<string, StudioPart>;
}

export const EMPTY_STUDIO_MESHES: StudioMeshes = { parts: new Map() };
const MAX_VERTICES = 12_000;
const MAX_TRIANGLES = 16_000;
const MAX_PARTS = 21;
const MAX_COORDINATE = 64;
const MAX_TEXTURE_BASE64 = 32_768;
const MAX_TEXTURE_SIDE = 1024;
const PNG_HEADER = [137, 80, 78, 71, 13, 10, 26, 10];
let loaded: Promise<StudioMeshes> | undefined;

export function loadStudioMeshes(): Promise<StudioMeshes> {
  return loaded ??= invoke<ArrayBuffer>("rig_meshes")
    .then(decodeStudioMeshes)
    .catch(() => EMPTY_STUDIO_MESHES);
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export async function decodeStudioMeshes(bytes: ArrayBuffer): Promise<StudioMeshes> {
  const parts = new Map<string, Geometry>();
  if (!bytes.byteLength) return EMPTY_STUDIO_MESHES;
  const pack: unknown = JSON.parse(new TextDecoder().decode(bytes));
  if (!record(pack) || pack.version !== 1) return EMPTY_STUDIO_MESHES;
  let vertexCount = 0;
  let faceCount = 0;
  scan: for (const rig of ["R6", "R15"] as const) {
    const inputParts = pack[rig];
    if (!record(inputParts)) continue;
    for (const name of Object.keys(studioRigs[rig])) {
      if (parts.size >= MAX_PARTS) break scan;
      const input = inputParts[name];
      const geometry = decodePart(input);
      if (!geometry) continue;
      vertexCount += geometry.vertices.length;
      faceCount += geometry.faces.length;
      if (vertexCount > MAX_VERTICES || faceCount > MAX_TRIANGLES) break scan;
      if (name === "Head" && record(input)) {
        geometry.texture = await decodeTexture(input.texturePng, input.uv, geometry.vertices.length);
        if (!geometry.texture) {
          geometry.frontDecal = await decodeTexture(input.frontDecalPng, input.uv, geometry.vertices.length);
        }
      }
      parts.set(meshKey(rig, name), geometry);
    }
  }
  for (const [rig, torso, match] of [
    ["R6", "Torso", pack.r6MatchHeadToTorso], ["R15", "UpperTorso", pack.r15MatchHeadToTorso],
  ] as const) {
    const head = parts.get(meshKey(rig, "Head"));
    const color = parts.get(meshKey(rig, torso))?.color;
    if (match === true && head?.texture && color) {
      head.texture = { ...head.texture, image: recolorHead(head.texture.image, color) };
      head.color = color;
    }
  }
  return { parts, r6Rig: decodeRig(pack.r6Rig, "R6"), r15Rig: decodeRig(pack.r15Rig, "R15") };
}

export function meshKey(rig: RigType, part: string) {
  return rig + ":" + part;
}

function decodePart(input: unknown): Geometry | undefined {
  if (!record(input)) return;
  const { vertices, triangles } = input;
  if (!Array.isArray(vertices) || vertices.length % 3 || vertices.length > MAX_VERTICES * 3
      || !Array.isArray(triangles) || triangles.length % 3 || triangles.length > MAX_TRIANGLES * 3) return;
  if (!vertices.every(value => typeof value === "number" && Math.abs(value) <= MAX_COORDINATE)
      || !triangles.every(value => Number.isInteger(value) && value >= 0 && value < vertices.length / 3)) return;
  const points: Vector[] = [];
  for (let i = 0; i < vertices.length; i += 3) points.push([vertices[i], vertices[i + 1], vertices[i + 2]]);
  const faces: { vertices: number[]; normal: Vector }[] = [];
  for (let i = 0; i < triangles.length; i += 3) {
    const [a, b, c] = [triangles[i], triangles[i + 1], triangles[i + 2]];
    const normal = normalOf(points[a], points[b], points[c]);
    if (normal) faces.push({ vertices: [a, b, c], normal });
  }
  if (!faces.length) return;
  const color = numericVector(input.color, 3, 0, 255) && input.color.every(Number.isInteger)
    ? [input.color[0], input.color[1], input.color[2]] as const : undefined;
  return { vertices: points, faces, color };
}

function numericVector(value: unknown, length: number, min: number, max: number): value is number[] {
  return Array.isArray(value) && value.length === length
    && value.every(entry => typeof entry === "number" && entry >= min && entry <= max);
}

function decodeRig(value: unknown, rig: RigType): Record<string, StudioPart> | undefined {
  if (!record(value)) return;
  const parts: Record<string, StudioPart> = {};
  for (const [name, expected] of Object.entries(studioRigs[rig])) {
    const part = value[name];
    if (!record(part) || part.parent !== expected.parent
        || !numericVector(part.size, 3, 0.001, MAX_COORDINATE)
        || !numericVector(part.c0, 12, -MAX_COORDINATE, MAX_COORDINATE)
        || !numericVector(part.c1, 12, -MAX_COORDINATE, MAX_COORDINATE)) return;
    parts[name] = { parent: part.parent, size: part.size, c0: part.c0, c1: part.c1 };
  }
  return parts;
}

async function decodeTexture(encoded: unknown, uv: unknown, vertexCount: number): Promise<MeshTexture | undefined> {
  if (typeof encoded !== "string" || encoded.length > MAX_TEXTURE_BASE64
      || !numericVector(uv, vertexCount * 2, 0, 1)) return;
  try {
    const png = Uint8Array.from(atob(encoded), character => character.charCodeAt(0));
    if (png.length < 24 || !PNG_HEADER.every((byte, i) => png[i] === byte)) return;
    const header = new DataView(png.buffer);
    const width = header.getUint32(16);
    const height = header.getUint32(20);
    if (!width || !height || width > MAX_TEXTURE_SIDE || height > MAX_TEXTURE_SIDE) return;
    const url = URL.createObjectURL(new Blob([png], { type: "image/png" }));
    try {
      const image = new Image();
      image.src = url;
      await image.decode();
      return { image, uv };
    } finally {
      URL.revokeObjectURL(url);
    }
  } catch {
    return undefined;
  }
}

function recolorHead(image: MeshTexture["image"], color: Vector): MeshTexture["image"] {
  const canvas = document.createElement("canvas");
  canvas.width = image.width;
  canvas.height = image.height;
  const context = canvas.getContext("2d");
  if (!context) return image;
  context.drawImage(image, 0, 0);
  const pixels = context.getImageData(0, 0, canvas.width, canvas.height);
  const { data } = pixels;
  const histogram = new Uint32Array(256);
  const gray = (offset: number) => Math.round((data[offset] + data[offset + 1] + data[offset + 2]) / 3);
  for (let offset = 0; offset < data.length; offset += 4) {
    if (data[offset + 3] === 255) histogram[gray(offset)]++;
  }
  let base = 1;
  for (let level = 2; level < histogram.length; level++) {
    if (histogram[level] > histogram[base]) base = level;
  }
  if (!histogram[base]) return image;

  // Match the atlas's flat color to the torso while retaining its face ink.
  for (let offset = 0; offset < data.length; offset += 4) {
    const opacity = data[offset + 3] / 255;
    const tone = 1 - opacity * (1 - Math.min(1, gray(offset) / base));
    data[offset] = color[0] * tone;
    data[offset + 1] = color[1] * tone;
    data[offset + 2] = color[2] * tone;
    data[offset + 3] = 255;
  }
  context.putImageData(pixels, 0, 0);
  return canvas;
}

function normalOf(a: Vector, b: Vector, c: Vector): Vector | undefined {
  const [ux, uy, uz] = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
  const [vx, vy, vz] = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
  const [x, y, z] = [uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx];
  const length = Math.hypot(x, y, z);
  if (length > 1e-10) return [x / length, y / length, z / length];
}
