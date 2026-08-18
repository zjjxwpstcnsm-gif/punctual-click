(locator) => {
  const observedSelectors = Array.isArray(locator?.observedSelectors)
    ? locator.observedSelectors
    : [];
  const captureSnapshot = () => {
    const presentSelectors = observedSelectors.filter((selector) => {
      try { return document.querySelector(selector) !== null; }
      catch (_) { return false; }
    });
    return {
      pageUrl: window.location.href,
      visibleText: document.body?.innerText || "",
      presentSelectors
    };
  };

  const resolveElement = () => {
    if (!locator || !locator.selector) return null;
    let root = document;
    for (const hostSelector of locator.shadowPath || []) {
      const host = root.querySelector(hostSelector);
      if (!host || !host.shadowRoot) return null;
      root = host.shadowRoot;
    }
    return root.querySelector(locator.selector);
  };

  const element = resolveElement();
  const shouldScroll = locator?.scrollIntoView !== false;
  if (!element) {
    return {
      found: false,
      clickable: false,
      reason: "target_not_found",
      ...captureSnapshot(),
      x: 0,
      y: 0,
      width: 0,
      height: 0
    };
  }

  if (shouldScroll) {
    element.scrollIntoView({ behavior: "auto", block: "center", inline: "center" });
  }

  const style = getComputedStyle(element);
  const rect = element.getBoundingClientRect();
  const visible = rect.width > 0 &&
    rect.height > 0 &&
    style.display !== "none" &&
    style.visibility !== "hidden" &&
    Number.parseFloat(style.opacity || "1") > 0;
  const enabled = !element.matches(":disabled") &&
    String(element.getAttribute("aria-disabled")).toLowerCase() !== "true";
  const pointerEvents = style.pointerEvents !== "none";
  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  const inViewport = x >= 0 && y >= 0 && x < window.innerWidth && y < window.innerHeight;

  let covered = true;
  if (visible && inViewport) {
    const ownerRoot = element.getRootNode();
    const top = typeof ownerRoot.elementFromPoint === "function"
      ? ownerRoot.elementFromPoint(x, y)
      : document.elementFromPoint(x, y);
    covered = Boolean(top && top !== element && !element.contains(top));
  }

  const clickable = visible && enabled && pointerEvents && inViewport && !covered;
  let reason = null;
  if (!visible) reason = "target_not_visible";
  else if (!enabled) reason = "target_disabled";
  else if (!pointerEvents) reason = "pointer_events_none";
  else if (!inViewport) reason = "target_outside_viewport";
  else if (covered) reason = "target_covered";

  // Scheduled execution pre-scrolls the target during the Armed phase and
  // calls this probe with scrollIntoView=false at the exact deadline. Manual
  // inspection/tests may keep the default true behavior.
  return {
    found: true,
    clickable,
    reason,
    ...captureSnapshot(),
    x,
    y,
    width: rect.width,
    height: rect.height
  };
}
