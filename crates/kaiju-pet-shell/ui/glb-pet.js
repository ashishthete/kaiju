// GlbPet — drives a rigged glTF character with PROCEDURAL bone animation, using
// the same interface PetEngine and LottiePet expose so index.html treats all
// three uniformly:
//   mount(host, def) · setReduced(bool) · setState(name) · trigger(name)
//   setMood(name, ms) · clearMood()
//
// Why procedural rather than baked clips: Meshy's auto-rig ships a skeleton but
// no usable motion (its one "clip" is a zero-length bind pose), and a desktop
// pet does not want a walk cycle anyway — it wants calm ambient life plus
// reactions to fleet state. Sine-driven bone offsets give us both, cost no
// asset bytes, and expose an amplitude dial that a canned loop cannot.
//
// def shape: { url: "assets/chibi.glb", faceCamera?: <radians> }
(function () {
  // Fleet-state → motion tuning. `freeze` parks the pose and stops the loop
  // (dead/errored look); `tint` maps to a CSS filter on the canvas.
  const STATES = {
    idle: { speed: 0.7, amp: 1.0 },
    working: { speed: 1.6, amp: 1.25 },
    waiting: { speed: 0.55, amp: 0.85 },
    stuck: { speed: 0.3, amp: 0.5, tint: "dim" },
    limited: { speed: 0.4, amp: 0.6, tint: "dim" },
    error: { speed: 0, amp: 0, tint: "bad", freeze: true },
    celebrate: { speed: 1.9, amp: 1.5 },
    disconnected: { speed: 0, amp: 0, tint: "off", freeze: true },
  };

  // One-shot reactions. Each is a function of progress t (0→1) returning bone
  // offsets layered on top of the ambient motion.
  const REACTIONS = {
    hover: { ms: 460, fn: (t) => ({ bob: Math.sin(t * Math.PI) * 0.09 }) },
    poke: { ms: 420, fn: (t) => ({ squash: Math.sin(t * Math.PI) * 0.16 }) },
    spin: { ms: 760, fn: (t) => ({ turn: t * Math.PI * 2 }) },
    waiting: { ms: 640, fn: (t) => ({ nod: Math.sin(t * Math.PI * 2) * 0.28 }) },
    stuck: { ms: 620, fn: (t) => ({ shake: Math.sin(t * Math.PI * 6) * (1 - t) * 0.2 }) },
    error: { ms: 620, fn: (t) => ({ shake: Math.sin(t * Math.PI * 6) * (1 - t) * 0.24 }) },
    limited: { ms: 900, fn: (t) => ({ slump: Math.sin(t * Math.PI) * 0.18 }) },
    celebrate: { ms: 800, fn: (t) => ({ bob: Math.sin(t * Math.PI) * 0.22, arms: Math.sin(t * Math.PI) * 0.9 }) },
    reconnect: { ms: 780, fn: (t) => ({ pop: Math.sin(t * Math.PI) * 0.2 }) },
  };

  const MOODS = {
    angry: { speed: 2.2, amp: 1.4, hunch: 0.18, tint: "bad" },
    dragging: { speed: 1.1, amp: 0.6, hunch: -0.12, tint: "" },
  };

  const TINTS = {
    dim: "saturate(.6) brightness(.82)",
    off: "grayscale(1) brightness(.7) opacity(.62)",
    bad: "brightness(1.04) drop-shadow(0 0 7px #ff4d4d)",
  };

  const FRAME_MS = 1000 / 30; // 30fps is plenty for ambient motion and halves GPU wakeups
  const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

  class GlbPet {
    constructor() {
      this.host = null;
      this.three = null; // { THREE, renderer, scene, camera, root }
      this.bones = {};
      this.rest = new Map(); // bone → { quat, pos }
      this.reduced = false;
      this.cur = "idle";
      this.mood = null;
      this.moodTimer = null;
      this.react = null; // { def, start }
      this.raf = null;
      this.last = 0;
      this.clock = 0;
      this.look = { x: 0, y: 0 }; // −1..1, pointer relative to the pet
      this.unit = 1; // body height in model units; scales positional offsets
      this.armDrop = 0; // radians to lower the T-pose arms, per-model
      this.loadToken = 0;
      this._onPointer = this._onPointer.bind(this);
      this._onVisibility = this._onVisibility.bind(this);
      this._tick = this._tick.bind(this);
    }

    // Loads three.js and the model on first mount only — Lottie users never pay
    // for the 3D runtime.
    async mount(container, def) {
      const token = ++this.loadToken;
      this._teardown();
      this.host = container;
      container.innerHTML = "";

      let THREE, GLTFLoader;
      try {
        const [core, loader] = await Promise.all([
          import("three"),
          import("three/addons/loaders/GLTFLoader.js"),
        ]);
        THREE = core;
        GLTFLoader = loader.GLTFLoader;
      } catch (e) {
        console.warn("[glb-pet] three.js failed to load", e);
        return;
      }
      if (token !== this.loadToken) return; // a newer mount superseded us

      const w = container.clientWidth || 220;
      const h = container.clientHeight || 220;

      const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true });
      renderer.setClearAlpha(0);
      renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
      renderer.setSize(w, h, false);
      renderer.domElement.style.cssText = "width:100%;height:100%;display:block";
      container.appendChild(renderer.domElement);

      const scene = new THREE.Scene();
      const camera = new THREE.PerspectiveCamera(30, w / h, 0.1, 100);
      scene.add(new THREE.HemisphereLight(0xffffff, 0x334455, 2.2));
      const key = new THREE.DirectionalLight(0xffffff, 1.6);
      key.position.set(2, 4, 3);
      scene.add(key);

      let gltf;
      try {
        gltf = await new GLTFLoader().loadAsync(def.url);
      } catch (e) {
        console.warn("[glb-pet] model failed to load", def.url, e);
        return;
      }
      if (token !== this.loadToken) return;

      const root = gltf.scene;
      scene.add(root);
      this._collectBones(root);
      this._frame(THREE, root, camera);
      this.armDrop = def.armDrop != null ? def.armDrop : 1.15;
      if (def.faceCamera) root.rotation.y += def.faceCamera;

      this.three = { THREE, renderer, scene, camera, root };
      this.cur = "idle";
      this.mood = null;
      this.setState(this.cur);

      window.addEventListener("pointermove", this._onPointer);
      document.addEventListener("visibilitychange", this._onVisibility);
      this._start();
    }

    // Meshy rigs use Mixamo-standard names; miss quietly if one is absent so a
    // differently-named rig degrades to fewer motion layers instead of throwing.
    _collectBones(root) {
      const want = [
        "Hips", "Spine", "Spine01", "Spine02", "neck", "Head",
        "LeftArm", "RightArm", "LeftForeArm", "RightForeArm",
        "LeftShoulder", "RightShoulder",
      ];
      this.bones = {};
      this.rest = new Map();
      root.traverse((o) => {
        if (!o.isBone && !o.isObject3D) return;
        if (want.includes(o.name) && !this.bones[o.name]) {
          this.bones[o.name] = o;
          this.rest.set(o, { quat: o.quaternion.clone(), pos: o.position.clone() });
        }
      });
      // Positional offsets are written in BONE-LOCAL space, which the armature
      // may scale relative to world space (Meshy's is ~28x). The hips' own rest
      // height is the only body-scale reference in that same space.
      const hips = this.bones.Hips && this.rest.get(this.bones.Hips);
      this.unit = hips ? Math.abs(hips.pos.y) || 1 : 1;
    }

    // Auto-frame: the model's scale and origin are whatever the exporter chose.
    // Meshy exports in centimetres (hips ~47 units up), so every positional
    // offset below is expressed as a fraction of `unit` rather than in raw
    // units — otherwise the motion is invisible on one rig and violent on another.
    _frame(THREE, root, camera) {
      const box = new THREE.Box3().setFromObject(root);
      const size = box.getSize(new THREE.Vector3());
      const center = box.getCenter(new THREE.Vector3());
      const height = size.y || 1;
      root.position.sub(center);
      root.position.y += height * 0.5; // sit on the origin
      const dist = height / (2 * Math.tan((camera.fov * Math.PI) / 360));
      camera.position.set(0, height * 0.55, dist * 1.35);
      camera.lookAt(0, height * 0.5, 0);
    }

    setReduced(r) {
      this.reduced = !!r;
      if (this.reduced) this._resetPose();
      this._applyPlayback();
    }

    setState(name) {
      this.cur = STATES[name] ? name : "idle";
      if (this.host) this.host.dataset.state = this.cur;
      this._applyPlayback();
    }

    _params() {
      const s = STATES[this.cur] || STATES.idle;
      const m = this.mood ? MOODS[this.mood] : null;
      return {
        speed: m ? m.speed : s.speed,
        amp: m ? m.amp : s.amp,
        hunch: m ? m.hunch : 0,
        tint: m ? m.tint : s.tint || "",
        freeze: !!s.freeze && !m,
      };
    }

    _applyPlayback() {
      if (!this.three) return;
      const p = this._params();
      if (this.host) this.host.style.filter = TINTS[p.tint] || "";
      if (this.reduced || p.freeze) {
        this._resetPose();
        this._renderOnce();
        this._stop(); // frozen states cost nothing
      } else {
        this._start();
      }
    }

    trigger(name) {
      const r = REACTIONS[name];
      if (!r || !this.three || this.reduced) return;
      this.react = { def: r, start: performance.now() };
      this._start(); // a reaction wakes a frozen pet briefly
    }

    setMood(name, ms) {
      if (!MOODS[name]) return;
      this.clearMood();
      this.mood = name;
      this._applyPlayback();
      clearTimeout(this.moodTimer);
      if (ms) this.moodTimer = setTimeout(() => this.clearMood(), ms);
    }

    clearMood() {
      clearTimeout(this.moodTimer);
      this.mood = null;
      this._applyPlayback();
    }

    _onPointer(e) {
      if (!this.host) return;
      const r = this.host.getBoundingClientRect();
      if (!r.width) return;
      this.look.x = clamp((e.clientX - (r.left + r.width / 2)) / (r.width * 1.5), -1, 1);
      this.look.y = clamp((e.clientY - (r.top + r.height / 2)) / (r.height * 1.5), -1, 1);
    }

    _onVisibility() {
      if (document.hidden) this._stop();
      else this._applyPlayback();
    }

    _start() {
      if (this.raf != null || !this.three) return;
      this.last = performance.now();
      this.raf = requestAnimationFrame(this._tick);
    }

    _stop() {
      if (this.raf != null) cancelAnimationFrame(this.raf);
      this.raf = null;
    }

    _tick(now) {
      this.raf = requestAnimationFrame(this._tick);
      const dt = now - this.last;
      if (dt < FRAME_MS) return; // throttle: ambient motion does not need 60fps
      this.last = now;

      const p = this._params();
      this.clock += (dt / 1000) * p.speed;
      this._pose(this.clock, p, now);
      this._renderOnce();

      // Once a reaction finishes and the state is frozen, go back to sleep.
      if (this.react && now - this.react.start > this.react.def.ms) {
        this.react = null;
        if (p.freeze || this.reduced) this._applyPlayback();
      }
    }

    // Compose ambient layers + the active reaction onto the rest pose. Every
    // bone is written from rest each frame, so layers never accumulate drift.
    _pose(t, p, now) {
      const B = this.bones;
      const a = p.amp;

      let r = {};
      if (this.react) {
        const prog = clamp((now - this.react.start) / this.react.def.ms, 0, 1);
        r = this.react.def.fn(prog) || {};
      }

      const breathe = Math.sin(t * 1.9) * 0.045 * a;
      const sway = Math.sin(t * 0.8) * 0.05 * a;
      // Positional layers are fractions of body height; rotational ones are radians.
      const bob = (Math.sin(t * 1.9) * 0.012 * a + (r.bob || 0) + (r.pop || 0)) * this.unit;

      this._setBone(B.Hips, (b, rest) => {
        b.position.copy(rest.pos);
        b.position.y += bob;
        b.quaternion.copy(rest.quat);
        b.rotateZ(sway * 0.5);
        b.rotateY(sway + (r.turn || 0) + (r.shake || 0));
      });

      const spineBend = breathe + p.hunch + (r.slump || 0) - (r.squash || 0);
      for (const name of ["Spine", "Spine01", "Spine02"]) {
        this._setBone(B[name], (b, rest) => {
          b.quaternion.copy(rest.quat);
          b.rotateX(spineBend * 0.5);
        });
      }

      // Head tracks the pointer, damped, and layers nod/shake on top.
      this._setBone(B.Head, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateY(this.look.x * 0.45 + (r.shake || 0) * 1.4);
        b.rotateX(this.look.y * 0.3 + (r.nod || 0) - breathe);
      });
      this._setBone(B.neck, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateY(this.look.x * 0.2);
        b.rotateX(this.look.y * 0.15);
      });

      // The rig ships in a T-pose, which reads as a mannequin rather than a pet.
      // Drop the arms to rest first, then swing around that. Measured on this
      // rig: local +X is the lower axis for BOTH arms (they mirror via bone
      // orientation, not via sign), and ~75deg puts the elbow straight down.
      const armSwing = Math.sin(t * 1.3) * 0.08 * a + (r.arms || 0);
      this._setBone(B.LeftArm, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(this.armDrop - armSwing);
      });
      this._setBone(B.RightArm, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(this.armDrop + armSwing);
      });
    }

    _setBone(bone, fn) {
      if (!bone) return;
      const rest = this.rest.get(bone);
      if (rest) fn(bone, rest);
    }

    _resetPose() {
      for (const [bone, rest] of this.rest) {
        bone.quaternion.copy(rest.quat);
        bone.position.copy(rest.pos);
      }
    }

    _renderOnce() {
      if (!this.three) return;
      const { renderer, scene, camera } = this.three;
      renderer.render(scene, camera);
    }

    _teardown() {
      this._stop();
      clearTimeout(this.moodTimer);
      window.removeEventListener("pointermove", this._onPointer);
      document.removeEventListener("visibilitychange", this._onVisibility);
      if (this.three) {
        this.three.renderer.dispose();
        this.three.renderer.forceContextLoss();
      }
      this.three = null;
      this.bones = {};
      this.rest = new Map();
      this.react = null;
    }
  }

  window.GlbPet = GlbPet;
})();
