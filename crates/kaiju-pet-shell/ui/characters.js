// Character registry. Adding a pet = add one entry here (+ its animation data).
//   lottie:true → single-loop Lottie character animated by LottiePet (lottie-pet.js).
//   otherwise   → legacy declarative CSS (optimus/kaiju) in index.html.
// The Lottie blob is the default; the CSS pets stay as alternates.
(function () {
  // Ordered registry the right-click menu cycles through.
  window.PET_CHARACTERS = [
    { id: "blob", name: "Blob", lottie: true },
    { id: "chibi", name: "Chibi (3D)", glb: true },
    { id: "optimus", name: "Optimus Prime", engine: false },
    { id: "kaiju", name: "Kaiju (classic)", engine: false },
  ];
  // glb characters: def = { url, faceCamera? }. Animated procedurally by GlbPet
  // (glb-pet.js) — the model carries a rig but no usable clips.
  window.PET_GLB = {
    chibi: { url: "assets/chibi.glb" },
  };
  window.PET_DEFS = {}; // PetEngine (anim.js) rigs — none currently.
  // Lottie characters: def = { anim: <lottie json> }. blob art loaded by blob-anim.js.
  window.PET_LOTTIE = {
    blob: { anim: (window.KAIJU_LOTTIE || {}).blob },
  };
})();
