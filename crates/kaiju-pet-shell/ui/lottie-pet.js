// LottiePet — drives a single-loop Lottie character (e.g. the blue blob) with the
// same interface PetEngine exposes, so index.html can treat both uniformly:
//   mount(host, def) · setReduced(bool) · setState(name) · trigger(name)
//   setMood(name, ms) · clearMood()
//
// A single looping animation can't have wildly different art per fleet state, so
// state is expressed by PLAYBACK SPEED + a light persistent tint, and interactions
// are short one-shot CSS reactions on the container. def shape:
//   { anim: <lottie json>, states: { name: {speed, tint?, freeze?} } }
(function () {
  if (!window.lottie) { console.warn("[lottie-pet] lottie runtime missing"); return; }

  // Fleet-state → playback tuning. `freeze` pauses on a frame (dead/errored look);
  // `tint` sets a data attribute the CSS maps to a filter.
  const STATES = {
    idle:         { speed: 0.7 },
    working:      { speed: 1.5 },
    waiting:      { speed: 0.55 },
    stuck:        { speed: 0.3, tint: "dim" },
    limited:      { speed: 0.4, tint: "dim" },
    error:        { speed: 0.0, tint: "bad", freeze: true },
    celebrate:    { speed: 1.9 },
    disconnected: { speed: 0.0, tint: "off", freeze: true },
  };

  // One-shot reactions → CSS class + how long to keep it. Retrigger-safe.
  const REACTIONS = {
    hover: ["lp-hop", 460],
    poke: ["lp-squash", 420],
    spin: ["lp-spin", 760],
    waiting: ["lp-nod", 640],
    stuck: ["lp-shake", 620],
    error: ["lp-shake", 620],
    limited: ["lp-dim", 900],
    celebrate: ["lp-jump", 800],
    reconnect: ["lp-pop", 780],
  };

  const MOODS = { angry: "lp-angry", dragging: "lp-drag" };

  class LottiePet {
    constructor() {
      this.anim = null;
      this.host = null;
      this.reduced = false;
      this.cur = "idle";
      this.mood = null;
      this.moodTimer = null;
      this.reactTimer = null;
      this.enterTimer = null;
    }

    mount(container, def) {
      if (this.anim) { this.anim.destroy(); this.anim = null; }
      container.innerHTML = "";
      this.host = container;
      this.anim = window.lottie.loadAnimation({
        container,
        renderer: "svg",
        loop: true,
        autoplay: true,
        animationData: def.anim,
        rendererSettings: { preserveAspectRatio: "xMidYMax meet" },
      });
      this.cur = "idle";
      this.mood = null;
      this.setState(this.cur);
      // Entrance pop when the blob first appears (skipped under reduced motion).
      if (!this.reduced) {
        container.classList.remove("lp-enter");
        void container.offsetWidth; // reflow so the animation restarts on re-mount
        container.classList.add("lp-enter");
        clearTimeout(this.enterTimer);
        this.enterTimer = setTimeout(() => container.classList.remove("lp-enter"), 720);
      }
    }

    setReduced(r) {
      this.reduced = !!r;
      this._applyPlayback();
    }

    // Persistent fleet state: speed + tint. A mood, if active, overrides tint/speed.
    setState(name) {
      this.cur = STATES[name] ? name : "idle";
      if (this.host) this.host.dataset.state = this.cur;
      this._applyPlayback();
    }

    _applyPlayback() {
      if (!this.anim) return;
      const s = STATES[this.cur] || STATES.idle;
      if (this.host) this.host.dataset.tint = this.mood ? "" : (s.tint || "");
      if (this.reduced || (s.freeze && !this.mood)) {
        this.anim.goToAndStop(this.reduced ? 0 : this.anim.totalFrames * 0.5, true);
        return;
      }
      this.anim.setSpeed(this.mood === "angry" ? 2.2 : s.speed || 1);
      this.anim.play();
    }

    // One-shot body reaction; also nudges playback alive briefly for frozen states.
    trigger(name) {
      const r = REACTIONS[name];
      if (!r || !this.host || this.reduced) return;
      const [cls, ms] = r;
      this.host.classList.remove(cls);
      void this.host.offsetWidth; // reflow so the animation restarts
      this.host.classList.add(cls);
      clearTimeout(this.reactTimer);
      this.reactTimer = setTimeout(() => this.host && this.host.classList.remove(cls), ms);
    }

    // Temporary mood (angry/dragging) overriding state until ms elapses (0 = manual).
    setMood(name, ms) {
      const cls = MOODS[name];
      if (!cls || !this.host) return;
      this.clearMood();
      this.mood = name;
      this.host.classList.add(cls);
      this._applyPlayback();
      clearTimeout(this.moodTimer);
      if (ms) this.moodTimer = setTimeout(() => this.clearMood(), ms);
    }

    clearMood() {
      clearTimeout(this.moodTimer);
      if (this.mood && this.host) this.host.classList.remove(MOODS[this.mood]);
      this.mood = null;
      this._applyPlayback();
    }
  }

  window.LottiePet = LottiePet;
})();
