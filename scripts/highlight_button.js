(locator) => {
  if (!locator || !locator.selector) return false;

  let root = document;
  for (const hostSelector of locator.shadowPath || []) {
    const host = root.querySelector(hostSelector);
    if (!host || !host.shadowRoot) return false;
    root = host.shadowRoot;
  }

  const element = root.querySelector(locator.selector);
  if (!element) return false;

  element.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
  const oldOutline = element.style.outline;
  const oldOffset = element.style.outlineOffset;
  element.style.outline = "4px solid #ff4d4f";
  element.style.outlineOffset = "3px";
  setTimeout(() => {
    element.style.outline = oldOutline;
    element.style.outlineOffset = oldOffset;
  }, 1800);
  return true;
}
