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
    // Double-click used to spin the character a full 360. On a 3D model that
    // just shows you the unlit back of the mesh, so it waves instead.
    wave: {
      ms: 1100,
      fn: (t) => ({
        // Peak must clear armDrop (1.15) by enough to lift the hand ABOVE the
        // shoulder: 2.4 lands the arm around 70deg up from horizontal.
        waveArm: Math.sin(Math.min(t * 1.6, 1) * Math.PI) * 2.4,
        waveHand: Math.sin(t * Math.PI * 6) * 0.5 * Math.sin(Math.min(t * 1.6, 1) * Math.PI),
        nod: Math.sin(t * Math.PI) * 0.12,
      }),
    },
    waiting: { ms: 640, fn: (t) => ({ nod: Math.sin(t * Math.PI * 2) * 0.28 }) },
    stuck: { ms: 620, fn: (t) => ({ shake: Math.sin(t * Math.PI * 6) * (1 - t) * 0.2 }) },
    error: { ms: 620, fn: (t) => ({ shake: Math.sin(t * Math.PI * 6) * (1 - t) * 0.24 }) },
    limited: { ms: 900, fn: (t) => ({ slump: Math.sin(t * Math.PI) * 0.18 }) },
    celebrate: { ms: 800, fn: (t) => ({ bob: Math.sin(t * Math.PI) * 0.22, arms: Math.sin(t * Math.PI) * 0.9 }) },
    reconnect: { ms: 780, fn: (t) => ({ pop: Math.sin(t * Math.PI) * 0.2 }) },

    // Escalation for repeated clicking. `quick` is a startled bounce for the
    // second and third click; `angry` is the payoff once you keep going —
    // a stomp, a torso shake, and arms thrown out.
    quick: {
      ms: 360,
      fn: (t) => ({
        bob: Math.sin(t * Math.PI) * 0.16,
        squash: Math.sin(t * Math.PI * 2) * 0.12,
      }),
    },
    angry: {
      ms: 900,
      fn: (t) => ({
        // Leg lifts fast, slams down, then the whole body judders.
        // 0.85 lifts the thigh ~48deg. Higher reads as a kick, not a stomp,
        // on a chibi with short legs.
        stomp: Math.sin(Math.min(t * 2.4, 1) * Math.PI) * 0.85,
        bob: -Math.sin(Math.min(t * 2.4, 1) * Math.PI) * 0.07,
        shake: Math.sin(t * Math.PI * 9) * (1 - t) * 0.26,
        arms: Math.sin(t * Math.PI) * 0.8,
        hunchPulse: Math.sin(t * Math.PI) * 0.22,
      }),
    },
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

  // The panel has no reachable inspector in a release build, so failures here
  // are invisible unless we push them to the Rust log.
  function report(level, message) {
    console.warn("[glb-pet]", message);
    try {
      const t = window.__TAURI__;
      if (t && t.core) t.core.invoke("log_js", { level, message: `[glb-pet] ${message}` });
    } catch (_) { /* browser fallback: console only */ }
  }

  // Motion amplitudes are tuned for the 188pt collapsed panel, where the whole
  // character is ~150px tall. Values that look right on a large canvas are
  // invisible here: 2% of body height is about 3px. Do not lower these without
  // checking them at actual panel size.
  const BOB = 0.05;      // fraction of hip height
  const BREATHE = 0.11;  // radians
  const SWAY = 0.14;     // radians
  const ARM_SWING = 0.22; // radians
  const LOOK_YAW = 0.7;   // radians at full deflection
  const LOOK_PITCH = 0.4;
  const SHIFT = 0.09;     // weight-shift: hip drop + knee bend, radians
  const NECK_LAG = 0.12;  // seconds the head trails the neck by

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
      this.lookLag = { x: 0, y: 0 }; // head trails the neck; see NECK_LAG
      this.unit = 1; // body height in model units; scales positional offsets
      this.armDrop = 0; // radians to lower the T-pose arms, per-model
      this.driven = false; // true once Rust is pacing us; rAF then stands down
      this.parked = false; // rest pose already rendered for a frozen state
      this.unlistenTick = null;
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
        report("error", `three.js failed to load: ${e && e.message ? e.message : e}`);
        return false;
      }
      if (token !== this.loadToken) return false; // a newer mount superseded us

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
        report("error", `model ${def.url} failed to load: ${e && e.message ? e.message : e}`);
        return false;
      }
      if (token !== this.loadToken) return false;

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

      // Fallback only: fires just while the cursor is over the panel. The real
      // tracking arrives from Rust via kaiju:tick (see _listenForTicks).
      window.addEventListener("pointermove", this._onPointer);
      document.addEventListener("visibilitychange", this._onVisibility);
      this._listenForTicks();
      this._start();
      return true;
    }

    // Meshy rigs use Mixamo-standard names; miss quietly if one is absent so a
    // differently-named rig degrades to fewer motion layers instead of throwing.
    _collectBones(root) {
      const want = [
        "Hips", "Spine", "Spine01", "Spine02", "neck", "Head",
        "LeftArm", "RightArm", "LeftForeArm", "RightForeArm",
        "LeftShoulder", "RightShoulder",
        "LeftUpLeg", "RightUpLeg", "LeftLeg", "RightLeg",
        "LeftFoot", "RightFoot", "LeftToeBase", "RightToeBase",
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
      this.parked = false; // the pose must be re-rendered for the new state
      if (this.reduced || p.freeze) {
        this._resetPose();
        this._renderOnce();
        this.parked = true;
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

    // Rust drives the clock at ~30Hz. WebKit throttles requestAnimationFrame in
    // a non-key NSPanel, so rAF alone leaves the pet frozen whenever it is not
    // focused — which is almost always. Once a tick arrives we stop rAF and let
    // Rust pace us; in a plain browser no tick ever comes and rAF stays.
    _listenForTicks() {
      const tauri = window.__TAURI__;
      if (!tauri || !tauri.event || this.unlistenTick) return;
      tauri.event
        .listen("kaiju:tick", ({ payload }) => {
          if (!this.three) return;
          if (payload) { this.look.x = payload.dx; this.look.y = payload.dy; }
          this.driven = true;
          this._stop(); // rAF is redundant now, and unreliable here
          this._frameFrom(performance.now());
        })
        .then((un) => { this.unlistenTick = un; })
        .catch(() => {});
    }

    _start() {
      if (this.raf != null || !this.three || this.driven) return;
      this.last = performance.now();
      this.raf = requestAnimationFrame(this._tick);
    }

    _stop() {
      if (this.raf != null) cancelAnimationFrame(this.raf);
      this.raf = null;
    }

    _tick(now) {
      this.raf = requestAnimationFrame(this._tick);
      if (now - this.last < FRAME_MS) return; // ambient motion does not need 60fps
      this._frameFrom(now);
    }

    // One animation step. Shared by the rAF loop and the Rust-driven tick so
    // both paths behave identically.
    _frameFrom(now) {
      const p = this._params();
      // Clamp dt so a throttled or backgrounded stretch resumes smoothly
      // instead of teleporting the pose forward.
      const dt = Math.min(now - this.last, 100);
      this.last = now;

      // Frozen/reduced: park on the rest pose and render exactly once, even
      // though ticks keep arriving.
      if (this.reduced || (p.freeze && !this.react)) {
        if (!this.parked) {
          this._resetPose();
          this._renderOnce();
          this.parked = true;
        }
        return;
      }
      this.parked = false;

      // Exponential chase so the head arrives after the neck.
      const k = clamp(dt / 1000 / NECK_LAG, 0, 1);
      this.lookLag.x += (this.look.x - this.lookLag.x) * k;
      this.lookLag.y += (this.look.y - this.lookLag.y) * k;

      this.clock += (dt / 1000) * (p.speed || 1);
      this._pose(this.clock, p, now);
      this._renderOnce();

      if (this.react && now - this.react.start > this.react.def.ms) this.react = null;
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

      const breathe = Math.sin(t * 1.9) * BREATHE * a;
      const sway = Math.sin(t * 0.8) * SWAY * a;
      // Positional layers are fractions of body height; rotational ones are radians.
      const bob = (Math.sin(t * 1.9) * BOB * a + (r.bob || 0) + (r.pop || 0)) * this.unit;

      this._setBone(B.Hips, (b, rest) => {
        b.position.copy(rest.pos);
        b.position.y += bob;
        b.quaternion.copy(rest.quat);
        b.rotateZ(sway * 0.5);
        b.rotateY(sway + (r.turn || 0) + (r.shake || 0));
      });

      const spineBend =
        breathe + p.hunch + (r.hunchPulse || 0) + (r.slump || 0) - (r.squash || 0);
      for (const name of ["Spine", "Spine01", "Spine02"]) {
        this._setBone(B[name], (b, rest) => {
          b.quaternion.copy(rest.quat);
          b.rotateX(spineBend * 0.5);
        });
      }

      // The neck leads and the head trails it by a beat. That lag is what reads
      // as "alive" — far more than raising the amplitude does.
      this._setBone(B.neck, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateY(this.look.x * LOOK_YAW * 0.45);
        b.rotateX(this.look.y * LOOK_PITCH * 0.45);
      });
      const lag = this.lookLag;
      this._setBone(B.Head, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateY(lag.x * LOOK_YAW + (r.shake || 0) * 1.4);
        b.rotateX(lag.y * LOOK_PITCH + (r.nod || 0) - breathe);
      });

      // Weight shift: the hips drop toward the standing leg while the opposite
      // knee softens. A slower cycle than the breath so they don't beat together.
      const shift = Math.sin(t * 0.53) * SHIFT * a;
      this._setBone(B.LeftUpLeg, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateZ(shift);
      });
      // The stomp rides on the right leg, on top of the weight shift.
      this._setBone(B.RightUpLeg, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateZ(shift);
        b.rotateX(-(r.stomp || 0));
      });
      this._setBone(B.LeftLeg, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(Math.max(0, shift) * 1.4);
      });
      this._setBone(B.RightLeg, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(Math.max(0, -shift) * 1.4 + (r.stomp || 0) * 1.3);
      });

      // The rig ships in a T-pose, which reads as a mannequin rather than a pet.
      // Drop the arms to rest first, then swing around that. Measured on this
      // rig: local +X is the lower axis for BOTH arms (they mirror via bone
      // orientation, not via sign), and ~75deg puts the elbow straight down.
      const armSwing = Math.sin(t * 1.3) * ARM_SWING * a + (r.arms || 0);
      this._setBone(B.LeftArm, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(this.armDrop - armSwing);
      });
      // +X lowers, so subtracting waveArm raises the right arm to wave.
      this._setBone(B.RightArm, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateX(this.armDrop + armSwing - (r.waveArm || 0));
      });
      this._setBone(B.RightForeArm, (b, rest) => {
        b.quaternion.copy(rest.quat);
        b.rotateZ(r.waveHand || 0);
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
      if (this.unlistenTick) { this.unlistenTick(); this.unlistenTick = null; }
      this.driven = false;
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
