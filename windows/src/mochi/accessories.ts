// Things Mochi can wear. Drawn in code, like the rest of the character, in the
// body's own coordinate space (origin = body centre) so they follow its tilt,
// squash and look direction for free.

export type HatKind =
  | "none" | "cap" | "crown" | "bow" | "tophat"
  | "party" | "beanie" | "nightcap" | "santa" | "witch";
export type FaceKind = "none" | "glasses" | "sunglasses";
export type NeckKind = "none" | "scarf" | "bowtie";

export const HATS: { id: HatKind; label: string }[] = [
  { id: "none", label: "None" },
  { id: "cap", label: "Cap" },
  { id: "crown", label: "Crown" },
  { id: "bow", label: "Bow" },
  { id: "tophat", label: "Top hat" },
  { id: "party", label: "Party hat" },
  { id: "beanie", label: "Beanie" },
  { id: "nightcap", label: "Nightcap" },
  { id: "santa", label: "Santa hat" },
  { id: "witch", label: "Witch hat" },
];

export const NECKS: { id: NeckKind; label: string }[] = [
  { id: "none", label: "None" },
  { id: "scarf", label: "Scarf" },
  { id: "bowtie", label: "Bow tie" },
];

export const FACES: { id: FaceKind; label: string }[] = [
  { id: "none", label: "None" },
  { id: "glasses", label: "Glasses" },
  { id: "sunglasses", label: "Sunglasses" },
];

export const isHat = (v: unknown): v is HatKind => HATS.some((h) => h.id === v);
export const isFace = (v: unknown): v is FaceKind => FACES.some((f) => f.id === v);
export const isNeck = (v: unknown): v is NeckKind => NECKS.some((n) => n.id === v);

/** Extras Mochi puts on by itself (see island/outfits.ts). */
export interface Extras {
  neck: NeckKind;
  umbrella: boolean;
  /** Gamer headset: headphones with a boom mic. */
  mic: boolean;
}

export const NO_EXTRAS: Extras = { neck: "none", umbrella: false, mic: false };

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
  /** Beat pulse 0…1 while music plays (headphones on), null otherwise. */
  headphones: number | null = null,
  extras: Extras = NO_EXTRAS,
) {
  if (extras.umbrella) drawUmbrella(x, pose, R, rx, ry);
  if (extras.neck !== "none") drawNeck(x, extras.neck, pose, R, rx, ry);
  if (face !== "none") drawFace(x, face, pose, R, rx, ry);
  if (headphones != null || extras.mic) drawHeadphones(x, headphones ?? 0, pose, R, rx, ry, extras.mic);
  if (hat !== "none") drawHat(x, hat, pose, R, rx, ry);
}

const PHONE_DARK = "#1A1412";
const PHONE_SHELL = "#2A2A30";
const PHONE_ACCENT = "#1ED760";
const GAMER_ACCENT = "#A855F7";

/**
 * Over-ear headphones: a band over the top of the head and a cup on each side.
 * The cups follow the yaw (the far one tucks behind the body) and thump on the beat.
 */
function drawHeadphones(
  x: CanvasRenderingContext2D,
  pulse: number,
  pose: Pose,
  R: number,
  rx: number,
  ry: number,
  mic = false,
) {
  const shift = Math.sin(pose.yaw) * rx * 0.22;
  const cupW = R * 0.3 * (1 + pulse * 0.08);
  const cupH = R * 0.62 * (1 + pulse * 0.08);
  const cupY = ry * 0.02;
  const cups = [-1, 1].map((sd) => ({
    sd,
    cx: sd * rx * 0.98 + shift * 0.6,
    // The cup on the side Mochi turns away from shrinks behind the body.
    k: clamp01(1 - Math.max(0, -sd * Math.sin(pose.yaw)) * 1.4),
  }));

  x.save();
  x.lineCap = "round";
  x.lineJoin = "round";

  // Band: thick dark arc with a lighter inner line.
  const bandTop = -ry * 1.12;
  const left = cups[0].cx;
  const right = cups[1].cx;
  const band = () => {
    x.beginPath();
    x.moveTo(left, cupY - cupH * 0.35);
    x.bezierCurveTo(left - rx * 0.05, bandTop, right + rx * 0.05, bandTop, right, cupY - cupH * 0.35);
  };
  band();
  x.strokeStyle = PHONE_DARK;
  x.lineWidth = R * 0.14;
  x.stroke();
  band();
  x.strokeStyle = PHONE_SHELL;
  x.lineWidth = R * 0.07;
  x.stroke();

  for (const c of cups) {
    if (c.k < 0.05) continue;
    const w = cupW * (0.55 + 0.45 * c.k);
    const hgt = cupH * (0.75 + 0.25 * c.k);
    x.save();
    x.translate(c.cx, cupY);
    x.globalAlpha = 0.35 + 0.65 * c.k;
    // Shell
    x.beginPath();
    x.roundRect(-w / 2, -hgt / 2, w, hgt, w * 0.48);
    x.fillStyle = PHONE_SHELL;
    x.fill();
    x.strokeStyle = PHONE_DARK;
    x.lineWidth = R * 0.05;
    x.stroke();
    // Accent stripe on the outer face
    x.beginPath();
    x.roundRect(c.sd * w * 0.06 - w * 0.17, -hgt * 0.3, w * 0.34, hgt * 0.6, w * 0.17);
    x.fillStyle = mic ? GAMER_ACCENT : PHONE_ACCENT;
    x.globalAlpha *= 0.75 + pulse * 0.25;
    x.fill();
    // Glint
    x.beginPath();
    x.moveTo(-c.sd * w * 0.22, -hgt * 0.32);
    x.lineTo(-c.sd * w * 0.22, -hgt * 0.12);
    x.strokeStyle = "rgba(255,255,255,0.35)";
    x.lineWidth = R * 0.035;
    x.stroke();
    x.restore();
  }

  if (mic) {
    // Boom from the left cup round to the mouth, ending in a little capsule.
    const c = cups[0];
    if (c.k > 0.05) {
      const end = { x: -rx * 0.28 + shift * 0.5, y: ry * 0.5 };
      x.beginPath();
      x.moveTo(c.cx, cupY + cupH * 0.2);
      x.quadraticCurveTo(c.cx + rx * 0.05, ry * 0.62, end.x, end.y);
      x.strokeStyle = PHONE_DARK;
      x.lineWidth = R * 0.06;
      x.stroke();
      x.beginPath();
      x.ellipse(end.x, end.y, R * 0.09, R * 0.065, 0, 0, Math.PI * 2);
      x.fillStyle = PHONE_SHELL;
      x.fill();
      x.strokeStyle = GAMER_ACCENT;
      x.lineWidth = R * 0.025;
      x.stroke();
    }
  }
  x.restore();
}

const clamp01 = (v: number) => Math.max(0, Math.min(1, v));

/** Scarf wrapped round the lower body, or a bow tie under the face. */
function drawNeck(
  x: CanvasRenderingContext2D,
  neck: NeckKind,
  pose: Pose,
  R: number,
  rx: number,
  ry: number,
) {
  const shift = Math.sin(pose.yaw) * rx * 0.3;
  x.save();
  if (neck === "scarf") {
    const y = ry * 0.62;
    x.beginPath();
    x.ellipse(0, y, rx * 0.98, R * 0.15, 0, 0, Math.PI);
    x.ellipse(0, y - R * 0.12, rx * 0.98, R * 0.1, 0, Math.PI, 0, true);
    x.closePath();
    x.fillStyle = "#E0453A";
    x.fill();
    // Stripes and the hanging end.
    x.fillStyle = "#F5F6F8";
    for (const sx of [-0.55, 0, 0.55]) {
      x.fillRect(sx * rx + shift * 0.4 - R * 0.035, y - R * 0.12, R * 0.07, R * 0.22);
    }
    x.beginPath();
    x.roundRect(rx * 0.32 + shift * 0.5, y, R * 0.24, R * 0.48, R * 0.06);
    x.fillStyle = "#C0352B";
    x.fill();
  } else if (neck === "bowtie") {
    const cx = shift;
    const cy = ry * 0.72;
    const s = R * 0.2;
    x.fillStyle = "#DC2626";
    for (const sd of [-1, 1]) {
      x.beginPath();
      x.moveTo(cx, cy);
      x.lineTo(cx + sd * s * 1.3, cy - s * 0.7);
      x.lineTo(cx + sd * s * 1.3, cy + s * 0.7);
      x.closePath();
      x.fill();
    }
    x.beginPath();
    x.arc(cx, cy, s * 0.3, 0, Math.PI * 2);
    x.fillStyle = "#991B1B";
    x.fill();
  }
  x.restore();
}

/** A small umbrella held over the head when it rains. */
function drawUmbrella(x: CanvasRenderingContext2D, pose: Pose, R: number, rx: number, ry: number) {
  const tilt = -0.18 + Math.sin(pose.yaw) * 0.1;
  const cx = rx * 0.15;
  const cy = -ry * 1.45;
  const w = rx * 1.35;
  x.save();
  x.translate(cx, cy);
  x.rotate(tilt);
  // Handle down to the side of the body.
  x.beginPath();
  x.moveTo(0, -R * 0.05);
  x.lineTo(0, R * 1.25);
  x.arc(R * 0.1, R * 1.25, R * 0.1, Math.PI, 0, true);
  x.strokeStyle = "#3F3F46";
  x.lineWidth = R * 0.06;
  x.lineCap = "round";
  x.stroke();
  // Canopy: scalloped dome in alternating colours.
  const segs = 4;
  for (let i = 0; i < segs; i++) {
    const a0 = -w + (2 * w * i) / segs;
    const a1 = a0 + (2 * w) / segs;
    x.beginPath();
    x.moveTo(0, -R * 0.75);
    x.quadraticCurveTo(a0 * 0.9, -R * 0.6, a0, 0);
    x.quadraticCurveTo((a0 + a1) / 2, -R * 0.18, a1, 0);
    x.quadraticCurveTo(a1 * 0.9, -R * 0.6, 0, -R * 0.75);
    x.fillStyle = i % 2 ? "#38BDF8" : "#0EA5E9";
    x.fill();
  }
  x.beginPath();
  x.arc(0, -R * 0.78, R * 0.05, 0, Math.PI * 2);
  x.fillStyle = "#3F3F46";
  x.fill();
  x.restore();
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
    case "party": {
      // Striped cone, slightly off-centre, with a pompom.
      const base = top + R * 0.16;
      const w = rx * 0.4;
      const apex = { x: rx * 0.12, y: base - R * 0.85 };
      x.save();
      x.beginPath();
      x.moveTo(-w, base);
      x.lineTo(apex.x, apex.y);
      x.lineTo(w, base);
      x.closePath();
      x.fillStyle = "#7C5CFF";
      x.fill();
      x.clip();
      x.strokeStyle = "#FBBF24";
      x.lineWidth = R * 0.09;
      for (let i = 1; i <= 3; i++) {
        const yy = base - (R * 0.85 * i) / 4;
        x.beginPath();
        x.moveTo(-w, yy + R * 0.06);
        x.lineTo(w, yy - R * 0.06);
        x.stroke();
      }
      x.restore();
      x.beginPath();
      x.arc(apex.x, apex.y, R * 0.09, 0, Math.PI * 2);
      x.fillStyle = "#F472B6";
      x.fill();
      break;
    }
    case "beanie": {
      const w = rx * 0.86;
      const base = top + R * 0.3;
      x.beginPath();
      x.moveTo(-w, base);
      x.bezierCurveTo(-w, top - R * 0.45, w, top - R * 0.45, w, base);
      x.closePath();
      x.fillStyle = "#2563EB";
      x.fill();
      // Ribbing
      x.strokeStyle = "rgba(255,255,255,0.18)";
      x.lineWidth = R * 0.03;
      for (let i = -2; i <= 2; i++) {
        x.beginPath();
        x.moveTo(i * w * 0.3, base - R * 0.05);
        x.lineTo(i * w * 0.22, top - R * 0.2);
        x.stroke();
      }
      // Folded band
      x.beginPath();
      x.roundRect(-w * 1.02, base - R * 0.16, w * 2.04, R * 0.2, R * 0.08);
      x.fillStyle = "#1D4ED8";
      x.fill();
      x.beginPath();
      x.arc(0, top - R * 0.36, R * 0.13, 0, Math.PI * 2);
      x.fillStyle = "#F5F6F8";
      x.fill();
      break;
    }
    case "nightcap":
    case "santa": {
      // A floppy cone that droops to one side, with a trim and a pompom.
      const santa = hat === "santa";
      const w = rx * 0.75;
      const base = top + R * 0.22;
      const tip = { x: rx * 0.95, y: top - R * 0.05 };
      x.beginPath();
      x.moveTo(-w, base);
      x.quadraticCurveTo(-w * 0.4, top - R * 0.75, rx * 0.25, top - R * 0.55);
      x.quadraticCurveTo(rx * 0.7, top - R * 0.45, tip.x, tip.y);
      x.quadraticCurveTo(rx * 0.55, top - R * 0.2, w, base);
      x.closePath();
      x.fillStyle = santa ? "#DC2626" : "#6366F1";
      x.fill();
      if (!santa) {
        // Little stars on the nightcap.
        x.fillStyle = "rgba(255,255,255,0.75)";
        for (const [sx, sy] of [[-w * 0.35, top - R * 0.15], [rx * 0.15, top - R * 0.35]]) {
          x.beginPath();
          x.arc(sx, sy, R * 0.035, 0, Math.PI * 2);
          x.fill();
        }
      }
      x.beginPath();
      x.roundRect(-w * 1.05, base - R * 0.13, w * 2.1, R * 0.2, R * 0.1);
      x.fillStyle = "#F5F6F8";
      x.fill();
      x.beginPath();
      x.arc(tip.x, tip.y, R * 0.12, 0, Math.PI * 2);
      x.fill();
      break;
    }
    case "witch": {
      const base = top + R * 0.2;
      x.beginPath();
      x.ellipse(0, base, rx * 1.05, R * 0.13, 0, 0, Math.PI * 2);
      x.fillStyle = "#2E1065";
      x.fill();
      x.beginPath();
      x.moveTo(-rx * 0.5, base);
      x.quadraticCurveTo(-rx * 0.1, top - R * 0.4, rx * 0.35, top - R * 0.95);
      x.quadraticCurveTo(rx * 0.15, top - R * 0.35, rx * 0.5, base);
      x.closePath();
      x.fill();
      x.fillStyle = "#F59E0B";
      x.fillRect(-rx * 0.48, base - R * 0.16, rx * 0.97, R * 0.1);
      break;
    }
  }
  x.restore();
}
