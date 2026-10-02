// Things Mochi can wear. Drawn in code, like the rest of the character, in the
// body's own coordinate space (origin = body centre) so they follow its tilt,
// squash and look direction for free.

export type HatKind = "none" | "cap" | "crown" | "bow" | "tophat";
export type FaceKind = "none" | "glasses" | "sunglasses";

export const HATS: { id: HatKind; label: string }[] = [
  { id: "none", label: "None" },
  { id: "cap", label: "Cap" },
  { id: "crown", label: "Crown" },
  { id: "bow", label: "Bow" },
  { id: "tophat", label: "Top hat" },
];

export const FACES: { id: FaceKind; label: string }[] = [
  { id: "none", label: "None" },
  { id: "glasses", label: "Glasses" },
  { id: "sunglasses", label: "Sunglasses" },
];

export const isHat = (v: unknown): v is HatKind => HATS.some((h) => h.id === v);
export const isFace = (v: unknown): v is FaceKind => FACES.some((f) => f.id === v);

export interface Pose {
  yaw: number;
  pitch: number;
  roll: number;
}

// Must match the eye layout in engine.ts.
const EYE_SP = 0.37;
const EYE_P = -0.12;

/** Where each eye sits on the body, plus how squashed it looks from the side. */
function eyeAt(sd: number, pose: Pose, rx: number, ry: number) {
  const eyeYaw = sd * EYE_SP + pose.yaw;
  const eyePitch = EYE_P + pose.pitch + pose.roll;
  const cp = Math.cos(eyePitch);
  return {
    x: Math.sin(eyeYaw) * cp * rx,
    y: -Math.sin(eyePitch) * ry,
    fx: Math.max(0.2, Math.cos(eyeYaw)),
    visible: Math.cos(eyeYaw) * cp > 0.04,
  };
}

export function drawAccessories(
  x: CanvasRenderingContext2D,
  hat: HatKind,
  face: FaceKind,
  pose: Pose,
  R: number,
  rx: number,
  ry: number,
) {
  if (face !== "none") drawFace(x, face, pose, R, rx, ry);
  if (hat !== "none") drawHat(x, hat, pose, R, rx, ry);
}

function drawFace(
  x: CanvasRenderingContext2D,
  face: FaceKind,
  pose: Pose,
  R: number,
  rx: number,
  ry: number,
) {
  const eyes = [-1, 1].map((sd) => eyeAt(sd, pose, rx, ry));
  const r = R * 0.34;
  const frame = R * 0.06;

  x.save();
  x.lineJoin = "round";
  x.lineCap = "round";

  // Bridge first, so the lenses sit on top of it.
  if (eyes[0].visible && eyes[1].visible) {
    x.strokeStyle = "rgb(26,20,18)";
    x.lineWidth = frame;
    x.beginPath();
    x.moveTo(eyes[0].x + r * eyes[0].fx, eyes[0].y);
    x.lineTo(eyes[1].x - r * eyes[1].fx, eyes[1].y);
    x.stroke();
  }

  for (const e of eyes) {
    if (!e.visible) continue;
    x.save();
    x.translate(e.x, e.y);
    x.scale(e.fx, 1);
    x.beginPath();
    if (face === "sunglasses") {
      x.roundRect(-r, -r * 0.78, r * 2, r * 1.56, r * 0.55);
      x.fillStyle = "rgba(12,12,18,0.94)";
      x.fill();
      x.strokeStyle = "rgb(26,20,18)";
      x.lineWidth = frame;
      x.stroke();
      // Glint
      x.beginPath();
      x.moveTo(-r * 0.55, -r * 0.35);
      x.lineTo(-r * 0.1, -r * 0.55);
      x.strokeStyle = "rgba(255,255,255,0.45)";
      x.lineWidth = frame * 0.7;
      x.stroke();
    } else {
      x.arc(0, 0, r, 0, Math.PI * 2);
      x.fillStyle = "rgba(190,225,255,0.18)";
      x.fill();
      x.strokeStyle = "rgb(26,20,18)";
      x.lineWidth = frame;
      x.stroke();
    }
    x.restore();
  }
  x.restore();
}

function drawHat(
  x: CanvasRenderingContext2D,
  hat: HatKind,
  pose: Pose,
  R: number,
  rx: number,
  ry: number,
) {
  // The hat shifts a little with where Mochi is looking.
  const shift = Math.sin(pose.yaw) * rx * 0.25;
  const top = -ry * 0.9;

  x.save();
  x.translate(shift, 0);
  x.lineJoin = "round";
  x.lineCap = "round";

  switch (hat) {
    case "cap": {
      const w = rx * 0.82;
      x.beginPath();
      x.moveTo(-w, top + R * 0.2);
      x.bezierCurveTo(-w, top - R * 0.5, w, top - R * 0.5, w, top + R * 0.2);
      x.closePath();
      x.fillStyle = "#E0453A";
      x.fill();
      // Brim
      x.beginPath();
      x.ellipse(w * 0.55, top + R * 0.22, w * 0.7, R * 0.11, 0, 0, Math.PI * 2);
      x.fillStyle = "#B8322A";
      x.fill();
      // Button
      x.beginPath();
      x.arc(0, top - R * 0.27, R * 0.06, 0, Math.PI * 2);
      x.fillStyle = "#B8322A";
      x.fill();
      break;
    }
    case "crown": {
      const w = rx * 0.6;
      const h = R * 0.5;
      const base = top + R * 0.18;
      x.beginPath();
      x.moveTo(-w, base);
      x.lineTo(-w, base - h * 0.7);
      x.lineTo(-w * 0.5, base - h * 0.35);
      x.lineTo(0, base - h);
      x.lineTo(w * 0.5, base - h * 0.35);
      x.lineTo(w, base - h * 0.7);
      x.lineTo(w, base);
      x.closePath();
      x.fillStyle = "#F5C542";
      x.fill();
      x.strokeStyle = "#C99A1E";
      x.lineWidth = R * 0.04;
      x.stroke();
      for (const px of [-w * 0.5, 0, w * 0.5]) {
        x.beginPath();
        x.arc(px, base - R * 0.1, R * 0.045, 0, Math.PI * 2);
        x.fillStyle = "#F4505E";
        x.fill();
      }
      break;
    }
    case "bow": {
      const cx = rx * 0.5;
      const cy = top + R * 0.05;
      const s = R * 0.3;
      x.fillStyle = "#F472B6";
      for (const sd of [-1, 1]) {
        x.beginPath();
        x.moveTo(cx, cy);
        x.bezierCurveTo(cx + sd * s * 1.3, cy - s * 0.9, cx + sd * s * 1.5, cy + s * 0.9, cx, cy);
        x.fill();
      }
      x.beginPath();
      x.arc(cx, cy, s * 0.26, 0, Math.PI * 2);
      x.fillStyle = "#DB2777";
      x.fill();
      break;
    }
    case "tophat": {
      const w = rx * 0.55;
      const h = R * 0.6;
      const base = top + R * 0.2;
      x.fillStyle = "#16161C";
      x.fillRect(-w, base - h, w * 2, h);
      x.beginPath();
      x.ellipse(0, base, w * 1.45, R * 0.1, 0, 0, Math.PI * 2);
      x.fill();
      x.fillStyle = "#F4505E";
      x.fillRect(-w, base - h * 0.28, w * 2, h * 0.16);
      break;
    }
  }
  x.restore();
}
