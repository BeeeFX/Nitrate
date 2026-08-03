// The one setting the extension has.
//
// Written to `storage.sync` so it follows a signed-in profile between machines;
// the content script watches the same key and redraws its buttons on change,
// which means open tabs update without a reload.

const DEFAULTS = { labelMode: "auto" };

const saved = document.getElementById("saved");
const inputs = Array.from(document.querySelectorAll('input[name="labelMode"]'));

chrome.storage.sync.get(DEFAULTS, (stored) => {
  const mode = stored?.labelMode || DEFAULTS.labelMode;
  const input = inputs.find((i) => i.value === mode) || inputs[0];
  if (input) input.checked = true;
});

for (const input of inputs) {
  input.addEventListener("change", () => {
    if (!input.checked) return;
    chrome.storage.sync.set({ labelMode: input.value }, () => {
      saved.classList.add("show");
      setTimeout(() => saved.classList.remove("show"), 1200);
    });
  });
}
