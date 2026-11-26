/// <reference types="vite/client" />

import { button } from "./ui";
import { EMPTY_STUDIO_MESHES, meshKey, type StudioMeshes, type StudioPart } from "./rig-mesh";
import { createRigView } from "./rig-view";
import studioRigs from "./studio-rigs.json";
import "./rig.css";

type Vector = readonly [number, number, number];
type Rotation = readonly [number, number, number, number];
type Matrix = readonly [number, number, number, number, number, number, number, number, number];

interface Transform {
  position: Vector;
  rotation: Rotation;
}

interface Key extends Transform {
  time: number;
  easing: number;
  direction: number;
}

interface Clip {
  duration: number;
  loop: boolean;
  tracks: Map<string, Key[]>;
  rig: "R6" | "R15";
}

interface Part {
  name: string;
  parent: string;
  c0: Transform;
  inverseC1: Transform;
  size: Vector;
  material: "body" | "limb" | "head";
}

const identity: Transform = { position: [0, 0, 0], rotation: [0, 0, 0, 1] };
const root = "HumanoidRootPart";

function studioFrame(values: number[]): Transform {
  return {
    position: [values[0], values[1], values[2]],
    rotation: matrixRotation([values[3], values[4], values[5], values[6], values[7],
      values[8], values[9], values[10], values[11]]),
  };
}

function makeRig(source: Record<string, StudioPart>, kind: Clip["rig"]): Part[] {
  const parts = new Map<string, Part>();
  function addPart(name: string) {
    if (name === root || parts.has(name)) return;
    const part = source[name];
    addPart(part.parent);
    const size: Vector = kind === "R6" && name === "Head"
      ? [1.25, 1.25, 1.25] // Visible SpecialMesh scale, separate from the R6 collision box.
      : [part.size[0], part.size[1], part.size[2]];
    parts.set(name, {
      name, parent: part.parent, size,
      c0: studioFrame(part.c0),
      inverseC1: invert(studioFrame(part.c1)),
      material: name === "Head" ? "head" : name.endsWith("Torso") ? "body" : "limb",
    });
  }
  for (const name of Object.keys(source)) addPart(name);
  return [...parts.values()];
}

const r6 = makeRig(studioRigs.R6, "R6");
const r15 = makeRig(studioRigs.R15, "R15");

const rotationFields = ["R00", "R01", "R02", "R10", "R11", "R12", "R20", "R21", "R22"] as const;
const maxXmlBytes = 32 * 1024 * 1024;
const maxKeyframes = 3000;
const maxPoses = 30000;
const maxDuration = 3600;
const playbackRates = [0.25, 0.5, 1, 1.5, 2] as const;
const easingStyle = { constant: 1, elastic: 2, legacyCubic: 3, bounce: 4, cubic: 5 };
const easingDirection = { in: 0, out: 1, inOut: 2 };

function children(element: Element, itemClass: string): Element[] {
  return Array.from(element.children).filter((child) => child.localName === "Item" && child.getAttribute("class") === itemClass);
}

function property(item: Element, name: string): Element | undefined {
  const properties = Array.from(item.children).find((child) => child.localName === "Properties");
  return properties && Array.from(properties.children).find((child) => child.getAttribute("name") === name);
}

function finite(value: string | null | undefined, label: string): number {
  const number = value?.trim() ? Number(value) : NaN;
  if (!Number.isFinite(number)) throw new Error(`Animation contains an invalid ${label}.`);
  return number;
}

function normalized(rotation: Rotation): Rotation {
  const length = Math.hypot(...rotation);
  if (!Number.isFinite(length) || length < 1e-8) throw new Error("Animation contains an invalid rotation.");
  return [rotation[0] / length, rotation[1] / length, rotation[2] / length, rotation[3] / length];
}

function matrixRotation([a, b, c, d, e, f, g, h, i]: Matrix): Rotation {
  const trace = a + e + i;
  if (trace > 0) {
    const s = 2 * Math.sqrt(trace + 1);
    return normalized([(h - f) / s, (c - g) / s, (d - b) / s, s / 4]);
  }
  if (a > e && a > i) {
    const s = 2 * Math.sqrt(1 + a - e - i);
    return normalized([s / 4, (b + d) / s, (c + g) / s, (h - f) / s]);
  }
  if (e > i) {
    const s = 2 * Math.sqrt(1 + e - a - i);
    return normalized([(b + d) / s, s / 4, (f + h) / s, (c - g) / s]);
  }
  const s = 2 * Math.sqrt(1 + i - a - e);
  return normalized([(c + g) / s, (f + h) / s, s / 4, (d - b) / s]);
}

function readTransform(pose: Element): Transform {
  const cframe = property(pose, "CFrame");
  if (!cframe) return identity;

  const fields = new Map(Array.from(cframe.children, (field) => [field.localName, field.textContent]));
  const component = (key: string) => finite(fields.get(key), `CFrame ${key}`);
  const position: Vector = [component("X"), component("Y"), component("Z")];
  const matrix = rotationFields.map(component) as unknown as Matrix;
  if (Math.hypot(...position) > 10000 || matrix.some((component) => Math.abs(component) > 1.01)) {
    throw new Error("Animation contains a pose outside the preview limits.");
  }
  return { position, rotation: matrixRotation(matrix) };
}

function readClip(bytes: Uint8Array): Clip {
  if (bytes.byteLength > maxXmlBytes) throw new Error("This animation exceeds the 32 MiB preview limit.");
  const signature = new TextDecoder().decode(bytes.subarray(0, 8));
  if (signature === "<roblox!") throw new Error("This preview requires a Roblox XML KeyframeSequence (.rbxmx).");

  const xml = new TextDecoder().decode(bytes);
  if (/<!\s*(DOCTYPE|ENTITY)\b/i.test(xml)) throw new Error("Animation XML cannot contain a document type or entity declaration.");
  const document = new DOMParser().parseFromString(xml, "application/xml");
  if (document.documentElement.localName !== "roblox" || document.querySelector("parsererror")) {
    throw new Error("This file is not a supported Roblox animation document.");
  }

  const sequences = document.querySelectorAll('Item[class="KeyframeSequence"]');
  const sequence = sequences.item(0);
  if (sequences.length !== 1 || !sequence) throw new Error("The preview requires a file containing one KeyframeSequence.");
  const keyframes = children(sequence, "Keyframe");
  if (!keyframes.length || keyframes.length > maxKeyframes) throw new Error("The animation has no keyframes or exceeds the preview limit.");

  const trackByTime = new Map<string, Map<number, Key>>();
  const known = new Set([...r6, ...r15].map((part) => part.name));
  known.add(root);
  let duration = 0;
  let poseCount = 0;

  for (const frame of keyframes) {
    const time = finite(property(frame, "Time")?.textContent ?? "0", "keyframe time");
    if (time < 0 || time > maxDuration) throw new Error("Keyframe time is outside the preview limits.");
    duration = Math.max(duration, time);

    const stack = children(frame, "Pose");
    while (stack.length) {
      const pose = stack.pop()!;
      poseCount += 1;
      if (poseCount > maxPoses) throw new Error("The animation exceeds the pose preview limit.");
      for (const child of children(pose, "Pose")) stack.push(child);
      const name = property(pose, "Name")?.textContent?.trim();
      if (!name || !known.has(name)) continue;
      const key = {
        time,
        ...readTransform(pose),
        easing: Number(property(pose, "EasingStyle")?.textContent ?? "0"),
        direction: Number(property(pose, "EasingDirection")?.textContent ?? "0"),
      };
      let track = trackByTime.get(name);
      if (!track) {
        track = new Map();
        trackByTime.set(name, track);
      }
      track.set(time, key);
    }
  }

  const r15Names = r15.filter(({ name }) => name !== "Head").map(({ name }) => name);
  const rig = r15Names.some((name) => trackByTime.has(name)) ? "R15" : "R6";
  if (!trackByTime.has(root) && !(rig === "R15" ? r15 : r6).some(({ name }) => trackByTime.has(name))) {
    throw new Error("This sequence has no poses for the R6 or R15 preview rig.");
  }
  const tracks = new Map(Array.from(trackByTime, ([name, keys]) => [name, [...keys.values()].sort((a, b) => a.time - b.time)]));
  return { duration, tracks, rig, loop: property(sequence, "Loop")?.textContent?.trim() === "true" };
}

function add(a: Vector, b: Vector): Vector {
  return [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
}

function scale([x, y, z]: Vector, factor: number): Vector {
  return [x * factor, y * factor, z * factor];
}

function cross(a: Vector, b: Vector): Vector {
  return [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
}

function rotate([x, y, z, w]: Rotation, v: Vector): Vector {
  const axis: Vector = [x, y, z];
  const t = scale(cross(axis, v), 2);
  return add(v, add(scale(t, w), cross(axis, t)));
}

function multiply([x, y, z, w]: Rotation, [i, j, k, l]: Rotation): Rotation {
  return [w * i + x * l + y * k - z * j, w * j - x * k + y * l + z * i,
    w * k + x * j - y * i + z * l, w * l - x * i - y * j - z * k];
}

function blendRotation(a: Rotation, b: Rotation, t: number): Rotation {
  let cosine = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
  if (cosine < 0) {
    cosine = -cosine;
    b = [-b[0], -b[1], -b[2], -b[3]];
  }
  if (cosine > 0.9995) {
    return normalized([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]),
      a[2] + t * (b[2] - a[2]), a[3] + t * (b[3] - a[3])]);
  }
  const angle = Math.acos(Math.min(1, cosine));
  const divisor = Math.sin(angle);
  const start = Math.sin((1 - t) * angle) / divisor;
  const end = Math.sin(t * angle) / divisor;
  return [a[0] * start + b[0] * end, a[1] * start + b[1] * end,
    a[2] * start + b[2] * end, a[3] * start + b[3] * end];
}

function bounce(t: number): number {
  const velocity = 7.5625;
  const period = 2.75;
  if (t < 1 / period) return velocity * t * t;
  if (t < 2 / period) return velocity * (t - 1.5 / period) ** 2 + 0.75;
  if (t < 2.5 / period) return velocity * (t - 2.25 / period) ** 2 + 0.9375;
  return velocity * (t - 2.625 / period) ** 2 + 0.984375;
}

function eased(t: number, style: number, direction: number): number {
  if (t <= 0 || t >= 1) return t;
  if (style === easingStyle.constant) return direction === easingDirection.out ? 1 : 0;
  const easeIn = (x: number) => {
    if (style === easingStyle.elastic && x > 0 && x < 1) return -(2 ** (10 * x - 10)) * Math.sin((10 * x - 10.75) * (2 * Math.PI / 3));
    if (style === easingStyle.legacyCubic || style === easingStyle.cubic) return x ** 3;
    if (style === easingStyle.bounce) return 1 - bounce(1 - x);
    return x;
  };
  if (direction === easingDirection.inOut) return t < 0.5 ? easeIn(2 * t) / 2 : 1 - easeIn(2 - 2 * t) / 2;
  // The legacy Cubic enum has its In and Out directions reversed in Roblox runtime.
  const out = style === easingStyle.legacyCubic ? direction === easingDirection.in : direction === easingDirection.out;
  if (out) return 1 - easeIn(1 - t);
  return easeIn(t);
}

function interpolatePose(from: Key, to: Key, progress: number): Transform {
  const t = eased(progress, from.easing, from.direction);
  return { position: add(scale(from.position, 1 - t), scale(to.position, t)), rotation: blendRotation(from.rotation, to.rotation, t) };
}

function poseAt(track: Key[] | undefined, time: number, repeatDuration: number): Transform {
  if (!track?.length) return identity;
  const first = track[0];
  const last = track[track.length - 1];
  if (first === last || time === first.time) return first;
  if (time === last.time) return last;
  if (time < first.time || time > last.time) {
    if (repeatDuration === 0) return time < first.time ? first : last;
    // A sparse looping track can span the end of the clip between authored keys.
    const wrappedTime = time < first.time ? time + repeatDuration : time;
    return interpolatePose(last, first, (wrappedTime - last.time) / (first.time + repeatDuration - last.time));
  }

  let low = 0;
  let high = track.length - 1;
  while (high - low > 1) {
    const middle = Math.floor((high + low) / 2);
    if (track[middle].time <= time) low = middle;
    else high = middle;
  }
  const from = track[low];
  const to = track[high];
  if (time === from.time) return from;
  return interpolatePose(from, to, (time - from.time) / (to.time - from.time));
}

function compose(a: Transform, b: Transform): Transform {
  return {
    position: add(a.position, rotate(a.rotation, b.position)),
    rotation: multiply(a.rotation, b.rotation),
  };
}

function invert(transform: Transform): Transform {
  const [x, y, z, w] = transform.rotation;
  const rotation: Rotation = [-x, -y, -z, w];
  return { position: rotate(rotation, scale(transform.position, -1)), rotation };
}

function partTransform(part: Part, parent: Transform, pose: Transform): Transform {
  return compose(compose(compose(parent, part.c0), pose), part.inverseC1);
}

function rigBounds(parts: Part[], rig: Clip["rig"], meshes: StudioMeshes) {
  const frames = new Map([[root, identity]]);
  let bottom = Infinity;
  let top = -Infinity;
  for (const part of parts) {
    const bind = partTransform(part, frames.get(part.parent)!, identity);
    frames.set(part.name, bind);
    const geometry = meshes.parts.get(meshKey(rig, part.name));
    if (geometry) {
      for (const vertex of geometry.vertices) {
        const y = bind.position[1] + rotate(bind.rotation, vertex)[1];
        bottom = Math.min(bottom, y);
        top = Math.max(top, y);
      }
    } else {
      bottom = Math.min(bottom, bind.position[1] - part.size[1] / 2);
      top = Math.max(top, bind.position[1] + part.size[1] / 2);
    }
  }
  return { rootHeight: -bottom, centerHeight: (top - bottom) / 2 };
}

const initialYaw = 0.45;
const initialPitch = 0.24;
const playbackFrameInterval = 1000 / 30;

export function mountRig(container: HTMLElement, bytes: Uint8Array, meshes = EMPTY_STUDIO_MESHES): () => void {
  const clip = readClip(bytes);
  const panel = document.createElement("section");
  panel.className = "rig-preview";
  panel.setAttribute("aria-label", "Animation preview");
  const stage = document.createElement("div");
  stage.className = "rig-stage";
  const canvas = document.createElement("canvas");
  canvas.tabIndex = 0;
  canvas.setAttribute("aria-label", `${clip.rig} animation rig. Use arrow keys or drag to rotate the view.`);
  canvas.textContent = `${clip.rig} animation preview requires WebGL 2.`;

  const importedRig = clip.rig === "R15" ? meshes.r15Rig : meshes.r6Rig;
  const parts = importedRig ? makeRig(importedRig, clip.rig) : clip.rig === "R15" ? r15 : r6;
  const bounds = rigBounds(parts, clip.rig, meshes);
  const view = createRigView(canvas, parts.map(part => ({
    ...part, geometry: meshes.parts.get(meshKey(clip.rig, part.name)),
  })), root, bounds.centerHeight);
  stage.append(canvas);

  const controls = document.createElement("div");
  controls.className = "rig-controls";
  const scrubber = document.createElement("div");
  scrubber.className = "rig-scrubber";
  const play = button("Play", { variant: "secondary", size: "small" });
  const playLabel = play.querySelector(".button-label")!;
  const playable = clip.duration > 0;
  play.disabled = !playable;
  const timeline = document.createElement("input");
  timeline.type = "range";
  timeline.min = "0";
  timeline.max = String(clip.duration);
  timeline.step = "any";
  timeline.value = "0";
  timeline.disabled = !playable;
  timeline.setAttribute("aria-label", "Animation position");
  const clock = document.createElement("span");
  clock.className = "rig-clock";
  clock.setAttribute("aria-hidden", "true");
  scrubber.append(play, timeline, clock);

  const modes = document.createElement("div");
  modes.className = "rig-modes";
  const rateField = document.createElement("label");
  rateField.className = "field";
  const rate = document.createElement("select");
  rate.setAttribute("aria-label", "Playback speed");
  rate.disabled = !playable;
  for (const speed of playbackRates) {
    rate.add(new Option(`${speed}×`, String(speed), speed === 1, speed === 1));
  }
  rateField.append(rate);
  const loop = button("Loop", { variant: "secondary", size: "small" });
  loop.classList.add("rig-loop");
  loop.disabled = !playable;
  loop.setAttribute("aria-pressed", String(clip.loop));
  modes.append(rateField, loop);
  controls.append(scrubber, modes);
  panel.append(stage, controls);
  container.replaceChildren(panel);

  const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
  const life = new AbortController();
  const options = { signal: life.signal };
  const totalTime = clip.duration.toFixed(2);
  let time = 0;
  let yaw = initialYaw;
  let pitch = initialPitch;
  let playback = false;
  let repeat = clip.loop;
  let speed = 1;
  let lastTick: number | undefined;
  let raf: number | undefined;
  let viewport: DOMRectReadOnly | undefined;
  let inViewport = false;
  let drag: { pointer: number; x: number; y: number } | undefined;

  function render() {
    if (!viewport?.width || !viewport.height) return;
    const repeatDuration = repeat ? clip.duration : 0;
    const rootPose = poseAt(clip.tracks.get(root), time, repeatDuration);
    view.setJoint(root, add([0, bounds.rootHeight, 0], rootPose.position), rootPose.rotation);
    for (const part of parts) {
      const pose = poseAt(clip.tracks.get(part.name), time, repeatDuration);
      const joint = partTransform(part, identity, pose);
      view.setJoint(part.name, joint.position, joint.rotation);
    }
    view.draw(viewport.width, viewport.height, yaw, pitch);
  }

  function updateTime() {
    timeline.value = String(time);
    const elapsed = time.toFixed(2);
    const display = `${elapsed} / ${totalTime} s`;
    if (clock.textContent === display) return;
    timeline.setAttribute("aria-valuetext", `${elapsed} of ${totalTime} seconds`);
    clock.textContent = display;
  }

  function requestFrame() {
    if (raf === undefined && inViewport && !document.hidden) raf = requestAnimationFrame(step);
  }

  function cancelFrame() {
    if (raf !== undefined) cancelAnimationFrame(raf);
    raf = undefined;
  }

  function syncVisibility() {
    lastTick = undefined;
    if (document.hidden || !inViewport) cancelFrame();
    else requestFrame();
  }

  function step(now: number) {
    raf = undefined;
    if (playback) {
      if (lastTick !== undefined) {
        const elapsed = now - lastTick;
        // Allow for sub-millisecond variation in display refresh timestamps.
        if (elapsed < playbackFrameInterval - 1) {
          requestFrame();
          return;
        }
        time += elapsed / 1000 * speed;
      }
      lastTick = now;
      if (time >= clip.duration) {
        time = repeat ? time % clip.duration : clip.duration;
        if (!repeat) setPlayback(false);
      }
      updateTime();
    }
    render();
    if (playback) requestFrame();
  }

  function setPlayback(active: boolean) {
    playback = active && playable;
    lastTick = undefined;
    playLabel.textContent = playback ? "Pause" : "Play";
    if (playback) requestFrame();
  }

  function moveCamera(dx: number, dy: number) {
    yaw += dx;
    pitch = Math.min(0.7, Math.max(-0.2, pitch + dy));
    requestFrame();
  }

  play.addEventListener("click", () => {
    if (!playback && time >= clip.duration) time = 0;
    setPlayback(!playback);
  }, options);
  timeline.addEventListener("input", () => {
    setPlayback(false);
    time = timeline.valueAsNumber;
    updateTime();
    requestFrame();
  }, options);
  rate.addEventListener("change", () => {
    speed = Number(rate.value);
  }, options);
  loop.addEventListener("click", () => {
    repeat = !repeat;
    loop.setAttribute("aria-pressed", String(repeat));
    requestFrame();
  }, options);
  canvas.addEventListener("pointerdown", (event) => {
    if (!event.isPrimary || event.button !== 0) return;
    drag = { pointer: event.pointerId, x: event.clientX, y: event.clientY };
    canvas.setPointerCapture(event.pointerId);
    canvas.classList.add("is-dragging");
  }, options);
  canvas.addEventListener("pointermove", (event) => {
    if (drag?.pointer !== event.pointerId) return;
    const { x, y } = drag;
    drag.x = event.clientX;
    drag.y = event.clientY;
    moveCamera((x - drag.x) * 0.01, (y - drag.y) * 0.01);
  }, options);
  canvas.addEventListener("lostpointercapture", () => {
    drag = undefined;
    canvas.classList.remove("is-dragging");
  }, options);
  canvas.addEventListener("keydown", (event) => {
    const movement = { ArrowLeft: [-0.12, 0], ArrowRight: [0.12, 0], ArrowUp: [0, 0.1], ArrowDown: [0, -0.1] }[event.key];
    if (!movement) return;
    event.preventDefault();
    moveCamera(movement[0], movement[1]);
  }, options);
  reducedMotion.addEventListener("change", (event) => {
    if (event.matches) setPlayback(false);
  }, options);
  document.addEventListener("visibilitychange", syncVisibility, options);
  canvas.addEventListener("webglcontextrestored", requestFrame, options);

  const observer = new ResizeObserver(([entry]) => {
    viewport = entry.contentRect;
    requestFrame();
  });
  observer.observe(canvas);
  const visibility = new IntersectionObserver(([entry]) => {
    inViewport = entry.isIntersecting;
    syncVisibility();
  });
  visibility.observe(canvas);
  updateTime();

  return () => {
    cancelFrame();
    life.abort();
    observer.disconnect();
    visibility.disconnect();
    view.dispose();
    panel.remove();
  };
}
