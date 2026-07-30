// Character registry. Adding a pet = add one entry here (+ its animation data).
//   lottie:true → single-loop Lottie character animated by LottiePet (lottie-pet.js).
// Only the Lottie blob ships for now; retired characters live in git history.
(function () {
  // Ordered registry the right-click menu cycles through.
  window.PET_CHARACTERS = [
    { id: "blob", name: "Blob", lottie: true },
  ];
  // Lottie characters: def = { anim: <lottie json> }. blob art loaded by blob-anim.js.
  window.PET_LOTTIE = {
    blob: { anim: (window.KAIJU_LOTTIE || {}).blob },
  };
})();
