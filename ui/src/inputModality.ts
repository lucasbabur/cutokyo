/**
 * Records whether the last interaction was a pointer press or a key press on
 * `<html data-input>`, so CSS can keep focus rings for keyboard users only.
 */
export function trackInputModality(root: HTMLElement): void {
  root.dataset["input"] = "keyboard";
  globalThis.addEventListener(
    "pointerdown",
    () => {
      root.dataset["input"] = "pointer";
    },
    true,
  );
  globalThis.addEventListener(
    "keydown",
    (event) => {
      if (event.metaKey || event.ctrlKey || event.altKey) return;
      root.dataset["input"] = "keyboard";
    },
    true,
  );
}
