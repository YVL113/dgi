import { randomUUID } from "node:crypto";
import { constants } from "node:fs";
import { copyFile, mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

const maxPack = 2 * 1024 * 1024;
export const localOutput = process.env.LOCALAPPDATA
  && join(process.env.LOCALAPPDATA, "com.dgi.desktop", "studio-rig-meshes.json");
export const studioRigs = JSON.parse(await readFile(new URL("../src/studio-rigs.json", import.meta.url), "utf8"));

export async function readPack(path) {
  try {
    if ((await stat(path)).size > maxPack) throw new Error("The existing rig pack exceeds the size limit.");
    const pack = JSON.parse(await readFile(path, "utf8"));
    if (pack?.version !== 1) throw new Error("The existing rig pack has an unsupported version.");
    return pack;
  } catch (error) {
    if (error.code === "ENOENT") return { version: 1 };
    throw error;
  }
}

export async function writePack(output, pack, backupSuffix) {
  const payload = JSON.stringify(pack);
  if (Buffer.byteLength(payload) > maxPack) throw new Error("The combined rig pack exceeds the app's size limit.");
  await mkdir(dirname(output), { recursive: true });
  try {
    await copyFile(output, output + backupSuffix, constants.COPYFILE_EXCL);
  } catch (error) {
    if (error.code !== "ENOENT" && error.code !== "EEXIST") throw error;
  }
  const temp = output + "." + randomUUID() + ".tmp";
  try {
    await writeFile(temp, payload, { flag: "wx" });
    await rename(temp, output);
  } finally {
    await rm(temp, { force: true });
  }
}
