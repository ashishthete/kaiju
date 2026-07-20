// Character registry. Adding a pet = add one entry here (+ its animation data).
//   rive:true   → Rive state-machine character animated by RivePet (rive-pet.js).
//   lottie:true → single-loop Lottie character animated by LottiePet (lottie-pet.js).
//   otherwise   → legacy declarative CSS (optimus/kaiju) in index.html.
// The Rive "robocat" is the default (distinct faces per fleet state); the Lottie
// "blob" and CSS "optimus"/"kaiju" stay as alternates.
(function () {
  // Ordered registry the right-click menu cycles through.
  window.PET_CHARACTERS = [
    { id: "robocat", name: "Robocat", rive: true },
    { id: "blob", name: "Blob", lottie: true },
    { id: "optimus", name: "Optimus Prime", engine: false },
    { id: "kaiju", name: "Kaiju (classic)", engine: false },
  ];
  window.PET_DEFS = {}; // PetEngine (anim.js) rigs — none currently.
  // Lottie characters: def = { anim: <lottie json> }. blob art loaded by blob-anim.js.
  window.PET_LOTTIE = {
    blob: { anim: (window.KAIJU_LOTTIE || {}).blob },
  };
  // Rive characters: def = { src, artboard, stateMachine }. .riv bundled in assets/.
  window.PET_RIVE = {
    robocat: { src: "assets/robocat.riv", artboard: "Catbot", stateMachine: "State Machine" },
  };
})();
