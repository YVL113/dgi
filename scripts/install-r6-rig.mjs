import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import { Box3, LoadingManager, Texture, Vector3 } from "three";
import { unzipSync } from "three/addons/libs/fflate.module.js";
import { FBXLoader } from "three/addons/loaders/FBXLoader.js";
import { localOutput, readPack, studioRigs, writePack } from "./rig-pack.mjs";

const meshPath = "source/r6_rigged.fbx";
const texturePath = "textures/Head1_diff.png";
const maxInput = 4 * 1024 * 1024;
const maxTexture = 24_576;
const maxVertices = 12_000;
const unitsPerStud = 100;
const { R6: studioR6 } = studioRigs;

function projectToRoblox(vector) {
  // This FBX faces +Z. Turn it 180 degrees about Y to match the Roblox bind pose.
  return vector.multiply(new Vector3(-1, 1, -1)).divideScalar(unitsPerStud);
}

function rounded(values, decimals) {
  return values.map(value => Number(value.toFixed(decimals)));
}

function readModel(bytes) {
  const manager = new LoadingManager();
  const loader = {
    setPath(path) { this.path = path; },
    // Texture bytes come exclusively from the ZIP. Do not load FBX file references.
    load() { return new Texture(); },
  };
  manager.addHandler(/.*/, loader);
  const scene = new FBXLoader(manager).parse(Uint8Array.from(bytes).buffer, "");
  const meshes = [];
  scene.traverse(object => { if (object.isSkinnedMesh) meshes.push(object); });
  if (meshes.length !== 1) throw new Error("Expected one skinned mesh in the R6 FBX.");
  scene.updateMatrixWorld(true);
  return meshes[0];
}

function readParts(mesh, atlas) {
  const geometry = mesh.geometry;
  const position = geometry.getAttribute("position");
  const joints = geometry.getAttribute("skinIndex");
  const weights = geometry.getAttribute("skinWeight");
  const uv = geometry.getAttribute("uv");
  const materials = Array.isArray(mesh.material) ? mesh.material : [mesh.material];
  if (!position || !joints || !weights || !uv || position.count > maxVertices
      || !geometry.groups.length || mesh.skeleton.bones.length !== Object.keys(studioR6).length) {
    throw new Error("The FBX is missing the R6 bone weights, UVs, or material groups.");
  }
  const parts = mesh.skeleton.bones.map(bone => {
    const name = bone.name.replaceAll("_", " ");
    const studio = studioR6[name];
    if (!Object.hasOwn(studioR6, name)
        || (name !== "Torso" && bone.parent?.name.replaceAll("_", " ") !== studio.parent)) {
      throw new Error(`Unexpected FBX bone or parent: ${bone.name}.`);
    }
    const pivot = projectToRoblox(bone.getWorldPosition(new Vector3()));
    return { name, studio, pivot, bounds: new Box3(), indices: new Map(), vertices: [], triangles: [], uv: [] };
  });
  if (new Set(parts.map(part => part.name)).size !== parts.length) throw new Error("Duplicate R6 bones.");

  for (const { start, count, materialIndex } of geometry.groups) {
    const material = materials[materialIndex];
    if (!material?.color || count % 3) throw new Error("Invalid R6 mesh material group.");
    const color = material.color.getHex();
    for (let offset = start; offset < start + count; offset += 3) {
      const corners = [0, 1, 2].map(i => geometry.index?.getX(offset + i) ?? offset + i);
      const joint = joints.getX(corners[0]);
      const part = parts[joint];
      if (!part || corners.some(i => joints.getX(i) !== joint || weights.getX(i) !== 1
          || weights.getY(i) !== 0 || weights.getZ(i) !== 0 || weights.getW(i) !== 0)) {
        throw new Error("The R6 archive must have one rigid bone per triangle.");
      }
      if (Boolean(material.map) !== (part.name === "Head") || (part.color !== undefined && color !== part.color)) {
        throw new Error(`Unexpected material on ${part.name}.`);
      }
      part.color = color;
      for (const index of corners) {
        const world = mesh.getVertexPosition(index, new Vector3()).applyMatrix4(mesh.matrixWorld);
        const local = projectToRoblox(world).sub(part.pivot);
        part.bounds.expandByPoint(local);
        const xyz = rounded(local.toArray(), 6);
        const texel = material.map ? rounded([uv.getX(index), 1 - uv.getY(index)], 7) : [];
        const key = [...xyz, ...texel].join(",");
        if (!part.indices.has(key)) {
          part.indices.set(key, part.vertices.length / 3);
          part.vertices.push(...xyz);
          part.uv.push(...texel);
        }
        part.triangles.push(part.indices.get(key));
      }
    }
  }

  const pivots = new Map(parts.map(part => [part.name, part.pivot]));
  const meshes = {};
  const rig = {};
  for (const { name, studio, pivot, bounds, vertices, triangles, uv, color } of parts) {
    const size = rounded(bounds.getSize(new Vector3()).toArray(), 6);
    if (!triangles.length || size.some(value => !Number.isFinite(value) || value < 0.01 || value > 8)) {
      throw new Error(`Invalid mesh dimensions for ${name}.`);
    }
    if (name === "Torso" && Math.abs(size[0] - studio.size[0]) > 0.01) {
      throw new Error("Unexpected FBX unit scale.");
    }
    meshes[name] = {
      vertices, triangles, color: [color >> 16, color >> 8 & 255, color & 255],
      ...(name === "Head" && { uv, texturePng: Buffer.from(atlas).toString("base64") }),
    };
    const parent = pivots.get(studio.parent) ?? pivot;
    // Retain Roblox's pose axes, using the FBX joint centers as the rigid pivots.
    rig[name] = {
      parent: studio.parent,
      size,
      c0: [...rounded(pivot.clone().sub(parent).toArray(), 6), ...studio.c0.slice(3)],
      c1: [0, 0, 0, ...studio.c1.slice(3)],
    };
  }
  return { meshes, rig };
}

async function install(input, output) {
  if (!input || !output) throw new Error("Usage: npm run rig:install -- <R6 ZIP> [output JSON]");
  if ((await stat(input)).size > maxInput) throw new Error("The R6 ZIP exceeds the size limit.");
  const archive = await readFile(input);
  const files = unzipSync(archive, {
    filter: entry => (entry.name === meshPath && entry.originalSize <= maxInput)
      || (entry.name === texturePath && entry.originalSize <= maxTexture),
  });
  if (!files[meshPath] || !files[texturePath]) {
    throw new Error(`Expected ${meshPath} and ${texturePath} in the ZIP.`);
  }
  const atlas = Buffer.from(files[texturePath]);
  if (atlas.length < 24 || atlas.subarray(0, 8).toString("hex") !== "89504e470d0a1a0a"
      || atlas.readUInt32BE(16) !== 1024 || atlas.readUInt32BE(20) !== 1024) {
    throw new Error("The FBX texture atlas must be a 1024 × 1024 PNG.");
  }
  const { meshes, rig } = readParts(readModel(files[meshPath]), atlas);
  const pack = await readPack(output);
  Object.assign(pack, {
    R6: meshes, r6Rig: rig, r6Source: basename(input),
    r6SourceSha256: createHash("sha256").update(archive).digest("hex"),
    r6MatchHeadToTorso: true,
  });
  await writePack(output, pack, ".before-fbx");
  const count = Object.values(meshes).reduce((sum, part) => sum + part.triangles.length / 3, 0);
  console.log(`Installed six R6 parts and ${count} triangles from ${pack.r6Source} in ${output}.`);
}

try {
  await install(process.argv[2], process.argv[3] ?? localOutput);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
