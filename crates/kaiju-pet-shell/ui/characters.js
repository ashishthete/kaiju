// Character registry. Adding a pet = add one entry here (+ a def for engine
// characters). "optimus" (PNG) and "kaiju" (CSS shapes) stay declarative in
// index.html (engine:false); engine characters are data-defined rigs animated
// by PetEngine (anim.js).
//
// A def = { svg, parts[], origins{}, clips{ state:{duration,tracks} },
//           events{ name:{duration,tracks} }, moods{ name:{duration,tracks} } }
// Track keyframe: { t: 0..1, v: number, e?: easeName }. Props: x,y,rot,sx,sy,op.
(function () {
  const kf = (...pairs) => pairs.map(([t, v, e]) => (e ? { t, v, e } : { t, v }));
  const blink = { sy: kf([0, 1], [0.9, 1], [0.94, 0.1, "in"], [0.98, 1, "out"], [1, 1]) };

  // ---- "Titan" — a Pacific Rim / Real Steel style battle-mech -----------------
  // Heavy angular armor, a single glowing reactor accent, and a light edge stroke
  // so the silhouette reads on any desktop. Rig is NESTED (visor ⊂ head ⊂ torso ⊂
  // body) so motion compounds with natural weight and follow-through.
  const titanSvg = `
<svg viewBox="0 0 120 120" width="100%" height="100%" xmlns="http://www.w3.org/2000/svg" style="overflow:visible">
  <defs>
    <linearGradient id="tArmor" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#9fadbd"/><stop offset="1" stop-color="#454f5c"/></linearGradient>
    <linearGradient id="tLimb" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#7c8794"/><stop offset="1" stop-color="#333b45"/></linearGradient>
    <radialGradient id="tCore" cx="50%" cy="42%" r="60%"><stop offset="0" stop-color="#ecffff"/><stop offset="45%" stop-color="#46e6ff"/><stop offset="100%" stop-color="#1782c8"/></radialGradient>
  </defs>
  <g data-part="shadow"><ellipse cx="60" cy="115" rx="30" ry="5" fill="#04121f" opacity="0.5"/></g>
  <g data-part="body">
    <g data-part="legL"><polygon points="46,74 58,74 57,104 45,104" fill="url(#tLimb)" stroke="#cfdae4" stroke-width="1.3"/><polygon points="41,103 60,103 60,111 40,111" fill="#3c444e" stroke="#cfdae4" stroke-width="1.3"/></g>
    <g data-part="legR"><polygon points="62,74 74,74 75,104 63,104" fill="url(#tLimb)" stroke="#cfdae4" stroke-width="1.3"/><polygon points="60,103 79,103 80,111 60,111" fill="#3c444e" stroke="#cfdae4" stroke-width="1.3"/></g>
    <g data-part="torso">
      <polygon points="47,60 73,60 70,75 50,75" fill="url(#tLimb)" stroke="#cfdae4" stroke-width="1.3"/>
      <polygon points="42,28 78,28 74,61 46,61" fill="url(#tArmor)" stroke="#cfdae4" stroke-width="1.5"/>
      <line x1="60" y1="30" x2="60" y2="58" stroke="#2c333c" stroke-width="1.2" opacity="0.7"/>
      <g data-part="shoulderL"><polygon points="20,30 44,27 45,42 24,45" fill="url(#tArmor)" stroke="#cfdae4" stroke-width="1.4"/><rect x="26" y="33" width="12" height="3" fill="#ffb23e" opacity="0.9"/></g>
      <g data-part="shoulderR"><polygon points="100,30 76,27 75,42 96,45" fill="url(#tArmor)" stroke="#cfdae4" stroke-width="1.4"/><rect x="82" y="33" width="12" height="3" fill="#ffb23e" opacity="0.9"/></g>
      <g data-part="armL"><polygon points="22,40 34,40 31,79 20,77" fill="url(#tLimb)" stroke="#cfdae4" stroke-width="1.3"/></g>
      <g data-part="armR"><polygon points="98,40 86,40 89,79 100,77" fill="url(#tLimb)" stroke="#cfdae4" stroke-width="1.3"/></g>
      <g data-part="reactor">
        <circle cx="60" cy="44" r="13" fill="#46e6ff" opacity="0.28"/>
        <circle cx="60" cy="44" r="9" fill="url(#tCore)" stroke="#dff9ff" stroke-width="1.3"/>
        <circle cx="60" cy="44" r="3.4" fill="#f2ffff"/>
      </g>
      <g data-part="head">
        <rect x="56" y="23" width="8" height="5" fill="#39424c"/>
        <polygon points="52,9 68,9 70,25 50,25" fill="url(#tArmor)" stroke="#cfdae4" stroke-width="1.5"/>
        <g data-part="visor"><rect x="50" y="14.5" width="20" height="6" rx="3" fill="#46e6ff" opacity="0.35"/><rect x="52.5" y="15.5" width="15" height="4" rx="2" fill="#c8f9ff"/></g>
      </g>
    </g>
  </g>
</svg>`;

  const titan = {
    svg: titanSvg,
    parts: ["shadow", "body", "legL", "legR", "torso", "shoulderL", "shoulderR", "armL", "armR", "reactor", "head", "visor"],
    origins: {
      body: "center bottom", torso: "center bottom", head: "center bottom",
      armL: "center top", armR: "center top", shoulderL: "center", shoulderR: "center",
      reactor: "center", visor: "center", legL: "center top", legR: "center top",
    },
    clips: {
      idle: {
        duration: 3800,
        tracks: {
          body: { x: kf([0, -1.3], [0.5, 1.3], [1, -1.3]) },
          torso: { sy: kf([0, 1], [0.5, 1.012], [1, 1]), y: kf([0, 0], [0.5, -1], [1, 0]), rot: kf([0, -0.6], [0.55, 0.6], [1, -0.6]) },
          head: { rot: kf([0, -2.5], [0.6, 2.5], [1, -2.5]) },
          visor: { op: kf([0, 0.8], [0.5, 1], [1, 0.8]) },
          reactor: { sx: kf([0, 1], [0.5, 1.1], [1, 1]), sy: kf([0, 1], [0.5, 1.1], [1, 1]), op: kf([0, 0.85], [0.5, 1], [1, 0.85]) },
          armL: { rot: kf([0, 2], [0.58, -1.5], [1, 2]) },
          armR: { rot: kf([0, -2], [0.58, 1.5], [1, -2]) },
          shoulderL: { y: kf([0, 0], [0.5, -0.6], [1, 0]) },
          shoulderR: { y: kf([0, 0], [0.5, -0.6], [1, 0]) },
        },
      },
      working: {
        duration: 1700,
        tracks: {
          torso: { rot: kf([0, -2], [0.5, 2], [1, -2]), y: kf([0, 0], [0.5, -2], [1, 0]) },
          armL: { rot: kf([0, 18], [0.5, -6], [1, 18]) },
          armR: { rot: kf([0, -6], [0.5, 18], [1, -6]) },
          head: { rot: kf([0, -1], [0.55, 1], [1, -1]) },
          reactor: { sx: kf([0, 1.02], [0.5, 1.12], [1, 1.02]), sy: kf([0, 1.02], [0.5, 1.12], [1, 1.02]), op: kf([0, 0.9], [0.25, 1], [0.5, 0.85], [0.75, 1], [1, 0.9]) },
          visor: { op: kf([0, 0.9], [0.5, 1], [1, 0.9]) },
        },
      },
      waiting: {
        duration: 2000,
        tracks: {
          torso: { rot: kf([0, 2], [0.5, 3.5], [1, 2]) },
          head: { rot: kf([0, 5], [0.5, 7], [1, 5]) },
          reactor: { sx: kf([0, 1.05], [0.5, 1.18], [1, 1.05]), sy: kf([0, 1.05], [0.5, 1.18], [1, 1.05]), op: kf([0, 0.95], [0.5, 1], [1, 0.95]) },
          visor: { op: kf([0, 0.95], [0.5, 1], [1, 0.95]) },
          armL: { rot: kf([0, 4], [0.5, 2], [1, 4]) },
        },
      },
      stuck: {
        duration: 3200,
        tracks: {
          torso: { rot: kf([0, 8], [0.5, 10], [1, 8]), y: kf([0, 3], [1, 3]) },
          head: { rot: kf([0, 12], [0.5, 14], [1, 12]) },
          armL: { rot: kf([0, 20], [0.5, 24], [1, 20]) },
          armR: { rot: kf([0, -20], [0.5, -24], [1, -20]) },
          shoulderL: { y: kf([0, 3], [1, 3]) }, shoulderR: { y: kf([0, 3], [1, 3]) },
          reactor: { op: kf([0, 0.5], [0.5, 0.3], [1, 0.5]) },
          visor: { op: kf([0, 0.5], [0.4, 0.7], [1, 0.5]) },
        },
      },
      limited: {
        duration: 3600,
        tracks: {
          reactor: { op: kf([0, 0.8], [0.5, 0.35], [1, 0.8]), sx: kf([0, 1], [0.5, 0.9], [1, 1]), sy: kf([0, 1], [0.5, 0.9], [1, 1]) },
          visor: { op: kf([0, 0.7], [0.5, 0.4], [1, 0.7]) },
          torso: { y: kf([0, 1], [0.5, 0], [1, 1]) },
        },
      },
      error: {
        duration: 660,
        tracks: {
          body: { x: kf([0, -3], [0.2, 3], [0.4, -3], [0.6, 3], [0.8, -3], [1, 2], "linear") },
          head: { rot: kf([0, -4], [0.5, 4], [1, -4]) },
          visor: { op: kf([0, 1], [0.15, 0.2], [0.3, 1], [0.5, 0.2], [0.7, 1], [1, 0.3]) },
          reactor: { op: kf([0, 1], [0.2, 0.3], [0.5, 1], [0.8, 0.4], [1, 1]), sx: kf([0, 1], [0.5, 1.2], [1, 1]), sy: kf([0, 1], [0.5, 1.2], [1, 1]) },
        },
      },
      celebrate: {
        duration: 1000,
        tracks: {
          torso: { y: kf([0, 2], [0.2, 5, "in"], [0.5, -8, "out"], [1, 2, "outBack"]), rot: kf([0, 0], [0.5, -3], [1, 0]) },
          armL: { rot: kf([0, 10], [0.2, 25, "in"], [0.55, -70, "out"], [1, 10]) },
          armR: { rot: kf([0, -10], [0.2, -25, "in"], [0.55, 70, "out"], [1, -10]) },
          head: { rot: kf([0, 2], [0.5, -4], [1, 2]) },
          reactor: { sx: kf([0, 1], [0.5, 1.35], [1, 1]), sy: kf([0, 1], [0.5, 1.35], [1, 1]), op: kf([0, 0.9], [0.5, 1], [1, 0.9]) },
        },
      },
      disconnected: {
        duration: 4600,
        tracks: {
          torso: { rot: kf([0, 10], [1, 10]), y: kf([0, 4], [0.5, 5], [1, 4]) },
          head: { rot: kf([0, 16], [1, 16]) },
          reactor: { op: kf([0, 0], [1, 0]) },
          visor: { op: kf([0, 0.08], [0.5, 0.15], [1, 0.08]) },
          armL: { rot: kf([0, 22], [1, 22]) }, armR: { rot: kf([0, -22], [1, -22]) },
        },
      },
    },
    events: {
      waiting: {
        duration: 700,
        tracks: {
          head: { rot: kf([0, 0], [0.3, 7, "out"], [1, 0, "inOut"]) },
          reactor: { sx: kf([0, 1], [0.3, 1.25, "out"], [1, 1]), sy: kf([0, 1], [0.3, 1.25, "out"], [1, 1]) },
        },
      },
      stuck: {
        duration: 640,
        tracks: {
          body: { x: kf([0, 0], [0.12, -5], [0.3, 5], [0.5, -4], [0.7, 3], [1, 0], "linear") },
          torso: { rot: kf([0, 0], [0.5, 3], [1, 0]) },
        },
      },
      limited: { duration: 1000, tracks: { reactor: { op: kf([0, 1], [0.5, 0.4], [1, 1]) } } },
      error: {
        duration: 560,
        tracks: {
          body: { x: kf([0, 0], [0.2, -6], [0.4, 6], [0.6, -5], [0.8, 5], [1, 0], "linear"), rot: kf([0, 0], [0.3, -3], [0.6, 3], [1, 0]) },
          visor: { op: kf([0, 1], [0.2, 0.2], [0.4, 1], [0.6, 0.3], [1, 1]) },
        },
      },
      celebrate: {
        duration: 900,
        tracks: {
          torso: { y: kf([0, 0], [0.2, 4, "in"], [0.5, -14, "out"], [1, 0, "outBack"]) },
          reactor: { sx: kf([0, 1], [0.45, 1.5], [1, 1]), sy: kf([0, 1], [0.45, 1.5], [1, 1]) },
        },
      },
      reconnect: {
        duration: 820,
        tracks: {
          body: { sx: kf([0, 0.82], [0.6, 1.05, "outBack"], [1, 1]), sy: kf([0, 0.82], [0.6, 1.05, "outBack"], [1, 1]) },
          reactor: { sx: kf([0, 0.4], [0.6, 1.3, "outBack"], [1, 1]), sy: kf([0, 0.4], [0.6, 1.3, "outBack"], [1, 1]) },
        },
      },
      // ---- interaction reactions ----
      hover: {
        duration: 460,
        tracks: {
          head: { rot: kf([0, 0], [0.4, -4, "out"], [1, 0, "inOut"]) },
          reactor: { sx: kf([0, 1], [0.35, 1.3, "out"], [1, 1]), sy: kf([0, 1], [0.35, 1.3, "out"], [1, 1]) },
        },
      },
      poke: {
        duration: 460,
        tracks: {
          torso: { rot: kf([0, 0], [0.2, 8, "out"], [0.5, -3], [1, 0, "outBack"]), y: kf([0, 0], [0.25, 3], [1, 0]) },
          head: { rot: kf([0, 0], [0.2, -11, "out"], [0.6, 4], [1, 0]) },
          reactor: { op: kf([0, 1], [0.15, 0.4], [0.35, 1], [1, 1]) },
        },
      },
      spin: {
        duration: 760,
        tracks: {
          body: { rot: kf([0, 0], [0.13, -24, "in"], [1, 360, "out"]) },
          torso: { y: kf([0, 0], [0.5, -6, "out"], [1, 0]) },
          reactor: { sx: kf([0, 1], [0.5, 1.4], [1, 1]), sy: kf([0, 1], [0.5, 1.4], [1, 1]) },
        },
      },
    },
    moods: {
      angry: {
        duration: 380,
        tracks: {
          body: { x: kf([0, -2.5], [0.25, 2.5], [0.5, -2.5], [0.75, 2.5], [1, -2.5], "linear") },
          torso: { rot: kf([0, -2], [0.5, 2], [1, -2]), y: kf([0, -1], [0.5, -2], [1, -1]) },
          head: { rot: kf([0, 6], [0.5, 8], [1, 6]) },
          armL: { rot: kf([0, -46], [0.5, -53], [1, -46]) },
          armR: { rot: kf([0, 46], [0.5, 53], [1, 46]) },
          reactor: { sx: kf([0, 1.18], [0.5, 1.38], [1, 1.18]), sy: kf([0, 1.18], [0.5, 1.38], [1, 1.18]), op: kf([0, 1], [1, 1]) },
          visor: { op: kf([0, 1], [1, 1]) },
        },
      },
      dragging: {
        duration: 1100,
        tracks: {
          body: { rot: kf([0, -6], [0.5, 6], [1, -6]) },
          torso: { rot: kf([0, 4], [0.5, -4], [1, 4]) },
          legL: { rot: kf([0, -14], [0.5, 10], [1, -14]) },
          legR: { rot: kf([0, 14], [0.5, -10], [1, 14]) },
          armL: { rot: kf([0, -20], [0.5, -12], [1, -20]) },
          armR: { rot: kf([0, 20], [0.5, 12], [1, 20]) },
          head: { rot: kf([0, 10], [0.5, -6], [1, 10]) },
          reactor: { sx: kf([0, 1.1], [0.5, 1.2], [1, 1.1]), sy: kf([0, 1.1], [0.5, 1.2], [1, 1.1]) },
        },
      },
    },
  };

  // Ordered registry the menu cycles through.
  //   engine:true → data-defined rig animated by PetEngine (anim.js).
  //   lottie:true → single-loop Lottie character animated by LottiePet (lottie-pet.js).
  //   otherwise   → legacy declarative CSS (optimus/kaiju) in index.html.
  window.PET_CHARACTERS = [
    { id: "blob", name: "Blob", lottie: true },
    { id: "titan", name: "Titan (mech)", engine: true },
    { id: "optimus", name: "Optimus Prime", engine: false },
    { id: "kaiju", name: "Kaiju (classic)", engine: false },
  ];
  window.PET_DEFS = { titan };
  // Lottie characters: def = { anim: <lottie json> }. blob art loaded by blob-anim.js.
  window.PET_LOTTIE = {
    blob: { anim: (window.KAIJU_LOTTIE || {}).blob },
  };
})();
