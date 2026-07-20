// RivePet — drives a Rive state-machine character (the "Catbot" robocat) with the
// same interface PetEngine/LottiePet expose, so index.html treats all pets alike:
//   mount(host, def) · setReduced(bool) · setState(name) · trigger(name)
//   setMood(name, ms) · clearMood()
//
// Unlike the single-loop Lottie blob, this character has DISTINCT faces per state,
// driven by state-machine inputs. def shape:
//   { src, artboard, stateMachine, wasm }
// Robocat inputs: No Internet/Error/Chat (triggers), Download (number 0..100),
// Reset (bool → back to idle). Interaction reactions reuse the shared lp-* CSS.
(function () {
  if (!window.rive) { console.warn("[rive-pet] rive runtime missing"); return; }

  // Fleet state → which face to show. Number faces (Download) hold on their own;
  // trigger faces animate in and hold until Reset or another trigger.
  //   trigger: fire this input · download: set the Download number · reset: return to idle
  const STATE = {
    idle:         { reset: true, download: 0 },
    working:      { download: 55 },
    celebrate:    { download: 100 },
    waiting:      { trigger: "Chat" },
    error:        { trigger: "Error" },
    stuck:        { trigger: "Error" },
    disconnected: { trigger: "No Internet" },
    limited:      { trigger: "No Internet" },
  };

  // One-shot interaction reactions → shared CSS class + duration (see index.html).
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

  class RivePet {
    constructor() {
      this.r = null;
      this.host = null;
      this.sm = "State Machine";
      this.inputs = {};
      this.ready = false;
      this.reduced = false;
      this.cur = "idle";
      this.pending = null;
      this.mood = null;
      this.moodTimer = null;
      this.reactTimer = null;
      this._onResize = () => { if (this.r) this.r.resizeDrawingSurfaceToCanvas(); };
    }

    mount(container, def) {
      if (this.r) { try { this.r.cleanup(); } catch (e) { /* ignore */ } this.r = null; }
      container.innerHTML = "";
      const canvas = document.createElement("canvas");
      canvas.style.width = "100%";
      canvas.style.height = "100%";
      container.appendChild(canvas);
      this.host = container;
      this.sm = def.stateMachine || "State Machine";
      this.inputs = {};
      this.ready = false;
      window.rive.RuntimeLoader.setWasmUrl(def.wasm || "vendor/rive.wasm");
      this.r = new window.rive.Rive({
        src: def.src,
        canvas,
        artboard: def.artboard || undefined,
        stateMachines: this.sm,
        autoplay: true,
        onLoad: () => {
          this.r.resizeDrawingSurfaceToCanvas();
          for (const i of this.r.stateMachineInputs(this.sm) || []) this.inputs[i.name] = i;
          this.ready = true;
          this.setState(this.pending || this.cur);
        },
        onLoadError: (e) => {
          window.__riveError = String(e);
          console.error("[rive-pet] load failed:", e);
        },
      });
      window.addEventListener("resize", this._onResize);
    }

    setReduced(r) {
      this.reduced = !!r;
      if (this.r) { this.reduced ? this.r.pause() : this.r.play(); }
    }

    _fire(name) { const i = this.inputs[name]; if (i && typeof i.fire === "function") i.fire(); }
    _num(name, v) { const i = this.inputs[name]; if (i) i.value = v; }
    _bool(name, v) { const i = this.inputs[name]; if (i) i.value = v; }

    // Show the face for a fleet state. A mood, if active, overrides it.
    setState(name) {
      this.cur = STATE[name] ? name : "idle";
      if (!this.ready) { this.pending = this.cur; return; }
      if (this.mood) return;
      const s = STATE[this.cur];
      // Download number holds a face on its own; keep it in sync every state change.
      this._num("Download", s.download || 0);
      // Reset is a bool the SM reads to return to idle — hold it true for a frame
      // (setting true+false in one tick would never be observed).
      if (s.reset) {
        this._bool("Reset", true);
        clearTimeout(this._resetTimer);
        this._resetTimer = setTimeout(() => this._bool("Reset", false), 90);
      }
      if (s.trigger) this._fire(s.trigger);
    }

    // One-shot body reaction (CSS class on the canvas host). Retrigger-safe.
    trigger(name) {
      const r = REACTIONS[name];
      if (!r || !this.host || this.reduced) return;
      const [cls, ms] = r;
      this.host.classList.remove(cls);
      void this.host.offsetWidth;
      this.host.classList.add(cls);
      clearTimeout(this.reactTimer);
      this.reactTimer = setTimeout(() => this.host && this.host.classList.remove(cls), ms);
    }

    // Temporary mood overriding state until ms elapses (0 = manual clearMood()).
    setMood(name, ms) {
      const cls = MOODS[name];
      if (!cls || !this.host) return;
      this.clearMood();
      this.mood = name;
      this.host.classList.add(cls);
      if (name === "angry") this._fire("Error"); // scowl to match the shake
      clearTimeout(this.moodTimer);
      if (ms) this.moodTimer = setTimeout(() => this.clearMood(), ms);
    }

    clearMood() {
      clearTimeout(this.moodTimer);
      if (this.mood && this.host) this.host.classList.remove(MOODS[this.mood]);
      this.mood = null;
      this.setState(this.cur); // restore the fleet-state face
    }
  }

  window.RivePet = RivePet;
})();
