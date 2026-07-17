// Tiny declarative character-animation engine.
//
// A character is a rig (named SVG parts) + named "clips". A clip is a set of
// eased keyframe tracks per part (x, y, rot, sx, sy, op). The runtime samples
// the active clip every frame, cross-fades smoothly when the action changes,
// and layers additive one-shot "event" bursts on top. This is real interpolated
// motion — not translating a static image around.
//
// Exposes: window.PetEngine, window.PET_EASE.
(function () {
  const EASE = {
    linear: (t) => t,
    inOut: (t) => (t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2),
    out: (t) => 1 - Math.pow(1 - t, 3),
    in: (t) => t * t,
    outBack: (t) => {
      const c1 = 1.70158, c3 = c1 + 1;
      return 1 + c3 * Math.pow(t - 1, 3) + c1 * Math.pow(t - 1, 2);
    },
  };
  const PROPS = ["x", "y", "rot", "sx", "sy", "op"];
  const IDENT = { x: 0, y: 0, rot: 0, sx: 1, sy: 1, op: 1 };

  // Sample one keyframe track [{t, v, e?}, ...] at phase p in [0,1].
  function sampleTrack(track, p) {
    if (!track || !track.length) return null;
    if (p <= track[0].t) return track[0].v;
    for (let i = 0; i < track.length - 1; i++) {
      const a = track[i], b = track[i + 1];
      if (p >= a.t && p <= b.t) {
        const span = b.t - a.t || 1;
        const k = (EASE[b.e] || EASE.inOut)((p - a.t) / span);
        return a.v + (b.v - a.v) * k;
      }
    }
    return track[track.length - 1].v;
  }

  function samplePose(clip, phase) {
    const pose = {};
    for (const part in clip.tracks) {
      const tr = clip.tracks[part];
      const o = Object.assign({}, IDENT);
      for (const prop of PROPS) {
        const s = sampleTrack(tr[prop], phase);
        if (s !== null && s !== undefined) o[prop] = s;
      }
      pose[part] = o;
    }
    return pose;
  }

  const g = (p, k) => (p && p[k] != null ? p[k] : IDENT[k]);

  // Linear blend between two full poses (for cross-fading actions).
  function blendPose(a, b, k) {
    const out = {};
    const parts = new Set([...Object.keys(a || {}), ...Object.keys(b || {})]);
    for (const part of parts) {
      const pa = a[part], pb = b[part], o = {};
      for (const prop of PROPS) o[prop] = g(pa, prop) + (g(pb, prop) - g(pa, prop)) * k;
      out[part] = o;
    }
    return out;
  }

  // Additive overlay: x/y/rot add, scale/opacity multiply their deviation.
  function addPose(base, ov, w) {
    if (!ov) return base;
    const out = Object.assign({}, base);
    for (const part in ov) {
      const bp = base[part] || IDENT, op = ov[part];
      out[part] = {
        x: g(bp, "x") + (op.x || 0) * w,
        y: g(bp, "y") + (op.y || 0) * w,
        rot: g(bp, "rot") + (op.rot || 0) * w,
        sx: g(bp, "sx") * (1 + (g(op, "sx") - 1) * w),
        sy: g(bp, "sy") * (1 + (g(op, "sy") - 1) * w),
        op: g(bp, "op") * (1 + (g(op, "op") - 1) * w),
      };
    }
    return out;
  }

  class PetEngine {
    constructor() {
      this.reduced = false;
      this.parts = {};
      this.clips = {};
      this.events = {};
      this.cur = null;
      this.curClip = null;
      this.prevPose = null;
      this.blendStart = 0;
      this.blendDur = 300;
      this.event = null;
      this.eventStart = 0;
      this.start0 = performance.now();
      this.lastPose = null;
      this.raf = null;
    }

    // Render a character definition into `container` and start its loop.
    mount(container, def) {
      container.innerHTML = def.svg;
      this.root = container.firstElementChild;
      this.parts = {};
      for (const name of def.parts) {
        const el = this.root.querySelector(`[data-part="${name}"]`);
        if (!el) continue;
        el.style.transformBox = "fill-box";
        el.style.transformOrigin = (def.origins && def.origins[name]) || "center";
        this.parts[name] = el;
      }
      this.clips = def.clips || {};
      this.events = def.events || {};
      this.cur = "idle";
      this.curClip = this.clips.idle || Object.values(this.clips)[0] || { duration: 2000, tracks: {} };
      this.prevPose = null;
      this.event = null;
      if (!this.raf) this.loop();
    }

    setReduced(r) { this.reduced = !!r; }

    // Switch the looping action, cross-fading from the current pose.
    setState(name) {
      if (name === this.cur) return;
      const clip = this.clips[name] || this.clips.idle;
      if (!clip) return;
      this.prevPose = this.lastPose || samplePose(this.curClip, 0);
      this.curClip = clip;
      this.cur = name;
      this.blendStart = performance.now();
    }

    // Fire a one-shot additive burst (waiting/stuck/error/celebrate/reconnect).
    trigger(name) {
      const ev = this.events[name];
      if (!ev) return;
      this.event = ev;
      this.eventStart = performance.now();
    }

    loop() {
      this.raf = requestAnimationFrame((now) => { this.frame(now); this.loop(); });
    }

    frame(now) {
      if (!this.curClip || !this.root) return;
      const clip = this.curClip;
      const phase = this.reduced ? 0 : (((now - this.start0) / (clip.duration || 2000)) % 1);
      let pose = samplePose(clip, phase);

      // Cross-fade from the frozen previous pose into the new clip.
      if (this.prevPose) {
        const k = Math.min(1, (now - this.blendStart) / this.blendDur);
        if (k < 1) pose = blendPose(this.prevPose, pose, EASE.inOut(k));
        else this.prevPose = null;
      }

      // Additive one-shot event burst.
      if (this.event) {
        const et = (now - this.eventStart) / (this.event.duration || 700);
        if (et >= 1) this.event = null;
        else pose = addPose(pose, samplePose(this.event, this.reduced ? 0 : et), 1);
      }

      this.lastPose = pose;
      for (const part in this.parts) {
        const p = pose[part] || IDENT;
        const el = this.parts[part];
        el.style.transform =
          `translate(${p.x.toFixed(2)}px, ${p.y.toFixed(2)}px) rotate(${p.rot.toFixed(2)}deg) scale(${p.sx.toFixed(3)}, ${p.sy.toFixed(3)})`;
        el.style.opacity = (p.op == null ? 1 : p.op).toFixed(3);
      }
    }
  }

  window.PetEngine = PetEngine;
  window.PET_EASE = EASE;
})();
