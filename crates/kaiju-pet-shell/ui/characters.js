// Character registry. Adding a pet = add one entry here (+ a def for engine
// characters). The two originals ("optimus" PNG, "kaiju" CSS) stay declarative
// in index.html and are marked engine:false; new characters are fully
// data-defined rigs animated by PetEngine (anim.js).
//
// A def = { svg, parts[], origins{}, clips{ state: {duration, tracks} }, events{} }
// Track keyframe: { t: 0..1, v: number, e?: easeName }. Props: x,y,rot,sx,sy,op.
(function () {
  // ---- "Kai" — a slime-kaiju drawn as an SVG rig ----------------------------
  const kaiSvg = `
<svg viewBox="0 0 120 120" width="100%" height="100%" xmlns="http://www.w3.org/2000/svg" style="overflow:visible">
  <defs>
    <radialGradient id="kaiBody" cx="40%" cy="32%" r="80%">
      <stop offset="0%" stop-color="#6ff0b4"/><stop offset="70%" stop-color="#1f9d74"/><stop offset="100%" stop-color="#128463"/>
    </radialGradient>
  </defs>
  <g data-part="footL"><ellipse cx="47" cy="103" rx="13" ry="7" fill="#0f6e57"/></g>
  <g data-part="footR"><ellipse cx="73" cy="103" rx="13" ry="7" fill="#0f6e57"/></g>
  <g data-part="armL"><ellipse cx="26" cy="76" rx="8" ry="13" fill="#1a8f6b"/></g>
  <g data-part="armR"><ellipse cx="94" cy="76" rx="8" ry="13" fill="#1a8f6b"/></g>
  <g data-part="body">
    <ellipse cx="60" cy="74" rx="36" ry="33" fill="url(#kaiBody)" stroke="#eafff6" stroke-width="2.5"/>
    <ellipse cx="60" cy="82" rx="21" ry="19" fill="#bdf7da" opacity="0.55"/>
  </g>
  <g data-part="hornL"><path d="M46 40 L52 16 L58 40 Z" fill="#dffbec"/></g>
  <g data-part="hornR"><path d="M62 40 L68 16 L74 40 Z" fill="#dffbec"/></g>
  <g data-part="eyeL"><circle cx="49" cy="60" r="8" fill="#0b2a22"/><circle cx="51.5" cy="57.5" r="2.6" fill="#fff"/></g>
  <g data-part="eyeR"><circle cx="71" cy="60" r="8" fill="#0b2a22"/><circle cx="73.5" cy="57.5" r="2.6" fill="#fff"/></g>
  <g data-part="mouth"><path d="M52 72 Q60 79 68 72" fill="none" stroke="#0a5747" stroke-width="2.6" stroke-linecap="round"/></g>
</svg>`;

  // keyframe helpers
  const kf = (...pairs) => pairs.map(([t, v, e]) => (e ? { t, v, e } : { t, v }));
  const blink = { sy: kf([0, 1], [0.88, 1], [0.92, 0.08, "in"], [0.96, 1, "out"], [1, 1]) };

  const kai = {
    svg: kaiSvg,
    parts: ["body", "belly", "armL", "armR", "hornL", "hornR", "eyeL", "eyeR", "mouth", "footL", "footR"],
    origins: {
      body: "center bottom", armL: "center top", armR: "center top",
      hornL: "center bottom", hornR: "center bottom",
    },
    clips: {
      idle: {
        duration: 2800,
        tracks: {
          body: { sy: kf([0, 1], [0.5, 1.06], [1, 1]), y: kf([0, 0], [0.5, -3], [1, 0]) },
          armL: { rot: kf([0, 7], [0.5, -3], [1, 7]) },
          armR: { rot: kf([0, -7], [0.5, 3], [1, -7]) },
          hornL: { rot: kf([0, -2], [0.5, 2], [1, -2]) },
          hornR: { rot: kf([0, 2], [0.5, -2], [1, 2]) },
          eyeL: blink, eyeR: blink,
        },
      },
      working: {
        duration: 1350,
        tracks: {
          body: { y: kf([0, 0], [0.5, -4], [1, 0]), rot: kf([0, -2], [0.5, 2], [1, -2]) },
          armL: { rot: kf([0, 20], [0.5, -14], [1, 20]) },
          armR: { rot: kf([0, -14], [0.5, 20], [1, -14]) },
          eyeL: blink, eyeR: blink,
        },
      },
      waiting: {
        duration: 1700,
        tracks: {
          body: { y: kf([0, -2], [0.5, -6], [1, -2]), sy: kf([0, 1.02], [0.5, 1.05], [1, 1.02]) },
          eyeL: { y: kf([0, -2], [0.5, -3.5], [1, -2]) },
          eyeR: { y: kf([0, -2], [0.5, -3.5], [1, -2]) },
          hornL: { rot: kf([0, -10], [0.5, -13], [1, -10]) },
          hornR: { rot: kf([0, 10], [0.5, 13], [1, 10]) },
        },
      },
      stuck: {
        duration: 2600,
        tracks: {
          body: { sy: kf([0, 0.95], [0.5, 0.92], [1, 0.95]), y: kf([0, 3], [0.5, 5], [1, 3]) },
          armL: { rot: kf([0, 24], [0.5, 28], [1, 24]) },
          armR: { rot: kf([0, -24], [0.5, -28], [1, -24]) },
          hornL: { rot: kf([0, 8], [1, 8]) }, hornR: { rot: kf([0, -8], [1, -8]) },
        },
      },
      limited: {
        duration: 3200,
        tracks: {
          body: { op: kf([0, 1], [0.5, 0.72], [1, 1]), sy: kf([0, 1], [0.5, 0.97], [1, 1]) },
          eyeL: blink, eyeR: blink,
        },
      },
      error: {
        duration: 620,
        tracks: {
          body: { x: kf([0, -2.5], [0.25, 2.5], [0.5, -2.5], [0.75, 2.5], [1, -2.5], "linear") },
          eyeL: { sy: kf([0, 0.6], [1, 0.6]) }, eyeR: { sy: kf([0, 0.6], [1, 0.6]) },
        },
      },
      celebrate: {
        duration: 620,
        tracks: {
          body: { y: kf([0, 2], [0.5, -10, "out"], [1, 2]), rot: kf([0, -5], [0.5, 5], [1, -5]) },
          armL: { rot: kf([0, -30], [0.5, -50], [1, -30]) },
          armR: { rot: kf([0, 30], [0.5, 50], [1, 30]) },
        },
      },
      disconnected: {
        duration: 4200,
        tracks: {
          body: { op: kf([0, 0.55], [0.5, 0.4], [1, 0.55]), y: kf([0, 3], [0.5, 4], [1, 3]) },
          eyeL: { sy: kf([0, 0.18], [1, 0.18]) }, eyeR: { sy: kf([0, 0.18], [1, 0.18]) },
        },
      },
    },
    events: {
      waiting: { duration: 620, tracks: { body: { y: kf([0, 0], [0.35, -9, "out"], [1, 0, "inOut"]) } } },
      stuck: {
        duration: 620,
        tracks: { body: { x: kf([0, 0], [0.15, -6], [0.35, 6], [0.55, -5], [0.75, 4], [1, 0], "linear") } },
      },
      limited: { duration: 1000, tracks: { body: { op: kf([0, 1], [0.5, 0.5], [1, 1]) } } },
      error: {
        duration: 520,
        tracks: {
          body: { x: kf([0, 0], [0.2, -6], [0.4, 6], [0.6, -5], [0.8, 5], [1, 0], "linear"), rot: kf([0, 0], [0.3, -4], [0.6, 4], [1, 0]) },
        },
      },
      celebrate: {
        duration: 780,
        tracks: {
          body: { y: kf([0, 0], [0.4, -20, "out"], [0.62, -18], [1, 0, "outBack"]), rot: kf([0, 0], [0.5, 12], [1, 0]) },
        },
      },
      reconnect: {
        duration: 720,
        tracks: { body: { sx: kf([0, 0.7], [0.6, 1.08, "outBack"], [1, 1]), sy: kf([0, 0.7], [0.6, 1.08, "outBack"], [1, 1]) } },
      },
    },
  };

  // Ordered registry the menu cycles through. engine:true → animated by PetEngine.
  window.PET_CHARACTERS = [
    { id: "optimus", name: "Optimus Prime", engine: false },
    { id: "kaiju", name: "Kaiju (classic)", engine: false },
    { id: "kai", name: "Kai (animated)", engine: true },
  ];
  window.PET_DEFS = { kai };
})();
