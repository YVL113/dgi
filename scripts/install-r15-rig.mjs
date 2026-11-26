import { createHash } from "node:crypto";
import { readFile, stat } from "node:fs/promises";
import { basename } from "node:path";
import { Box3, Vector3 } from "three";
import { unzipSync } from "three/addons/libs/fflate.module.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { localOutput, readPack, studioRigs, writePack } from "./rig-pack.mjs";

const innerArchive = "source/RobloxR15Rigged.zip";
const objectPath = "RobloxR15Rigged.obj";
const maxInput = 4 * 1024 * 1024;
const maxCorners = 12_000;
const scale = 100;
const uvTolerance = 0.00001;
const { R15: studioR15 } = studioRigs;

function unzipEntry(bytes, name) {
  const data = unzipSync(bytes, {
    filter: entry => entry.name === name && entry.originalSize <= maxInput,
  })[name];
  if (!data?.length) throw new Error(`Missing or oversized archive entry: ${name}.`);
  return data;
}

function rounded(value) {
  return Number(value.toFixed(6));
}

function validateFaceUv(uv, source) {
  for (let offset = 0; offset < uv.length; offset += 2) {
    let matched = false;
    for (let index = 0; index < source.length; index += 2) {
      if (Math.abs(uv[offset] - source[index]) < uvTolerance
          && Math.abs(uv[offset + 1] - source[index + 1]) < uvTolerance) {
        matched = true;
        break;
      }
    }
    if (!matched) throw new Error("The R15 head UV layout does not match the installed R6 texture.");
  }
}

function readParts(obj, r6) {
  const scene = new OBJLoader().parse(new TextDecoder().decode(obj));
  const meshes = {};
  const centers = new Map();
  const color = r6.Torso?.color ?? [115, 119, 126];
  let corners = 0;
  for (const mesh of scene.children) {
    const name = mesh.name;
    if (!mesh.isMesh || !Object.hasOwn(studioR15, name) || Object.hasOwn(meshes, name)) {
      throw new Error(`Unexpected R15 OBJ group: ${name}.`);
    }
    const position = mesh.geometry.getAttribute("position");
    const texture = mesh.geometry.getAttribute("uv");
    corners += position.count;
    if (corners > maxCorners || !position.count || position.count % 3 || (name === "Head" && !texture)) {
      throw new Error(`Invalid vertex or texture data for ${name}.`);
    }
    const bounds = new Box3().setFromBufferAttribute(position);
    const center = bounds.getCenter(new Vector3()).divideScalar(scale);
    const size = bounds.getSize(new Vector3()).divideScalar(scale).toArray();
    if (size.some(value => !Number.isFinite(value) || value < 0.01 || value > 8)
        || Math.abs(size[0] - studioR15[name].size[0]) > 0.01) {
      throw new Error(`Unexpected dimensions for ${name}.`);
    }
    centers.set(name, center);
    const vertices = [];
    const triangles = [];
    const uv = [];
    const indices = new Map();
    const corner = new Vector3();
    for (let i = 0; i < position.count; i++) {
      const xyz = corner.fromBufferAttribute(position, i).divideScalar(scale).sub(center).toArray().map(rounded);
      const texel = name === "Head" ? [texture.getX(i), 1 - texture.getY(i)].map(rounded) : [];
      const key = [...xyz, ...texel].join(",");
      if (!indices.has(key)) {
        indices.set(key, vertices.length / 3);
        vertices.push(...xyz);
        uv.push(...texel);
      }
      triangles.push(indices.get(key));
    }
    if (name === "Head") validateFaceUv(uv, r6.Head.uv);
    meshes[name] = {
      vertices, triangles, color,
      ...(name === "Head" && { uv, texturePng: r6.Head.texturePng }),
    };
  }
  if (Object.keys(meshes).length !== Object.keys(studioR15).length) {
    throw new Error("The OBJ must contain all 15 named R15 parts.");
  }

  const origin = new Vector3();
  const rig = {};
  for (const [name, part] of Object.entries(studioR15)) {
    // Keep the Studio animation axes and joint offsets inside each part.
    const rest = centers.get(name).clone().sub(centers.get(part.parent) ?? origin).toArray();
    rig[name] = {
      ...part,
      c0: [...rest.map((value, axis) => rounded(value + part.c1[axis])), ...part.c0.slice(3)],
    };
  }
  return { meshes, rig };
}

async function install(input, output) {
  if (!input || !output) throw new Error("Usage: npm run rig:install:r15 -- <R15 ZIP> [output JSON]");
  if ((await stat(input)).size > maxInput) throw new Error("The R15 ZIP exceeds the size limit.");
  const archive = await readFile(input);
  const obj = unzipEntry(unzipEntry(archive, innerArchive), objectPath);
  const pack = await readPack(output);
  if (typeof pack.R6?.Head?.texturePng !== "string" || !Array.isArray(pack.R6.Head.uv)) {
    throw new Error("Install the R6 FBX rig first so DGI can copy its face texture.");
  }
  const { meshes, rig } = readParts(obj, pack.R6);
  Object.assign(pack, {
    R15: meshes, r15Rig: rig, r15Source: basename(input),
    r15SourceSha256: createHash("sha256").update(archive).digest("hex"),
    r15MatchHeadToTorso: true,
  });
  await writePack(output, pack, ".before-r15");
  const count = Object.values(meshes).reduce((sum, part) => sum + part.triangles.length / 3, 0);
  console.log(`Installed 15 R15 parts and ${count} triangles from ${pack.r15Source} in ${output}.`);
}

try {
  await install(process.argv[2], process.argv[3] ?? localOutput);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
