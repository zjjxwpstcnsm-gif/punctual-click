() => {
  const trimText = (value) => (value || "").replace(/\s+/g, " ").trim();

  const cssEscape = (value) => {
    if (globalThis.CSS && typeof globalThis.CSS.escape === "function") {
      return globalThis.CSS.escape(value);
    }
    return String(value).replace(/[^a-zA-Z0-9_-]/g, "\\$&");
  };

  const attributeEscape = (value) => String(value)
    .replace(/\\/g, "\\\\")
    .replace(/"/g, '\\"');

  const selectorHint = (element) => {
    // A selector hint is later used to re-probe the exact candidate immediately
    // before the native mouse event. Attribute selectors are only safe when
    // they identify this element uniquely inside its own Document/ShadowRoot.
    const queryRoot = element.getRootNode();
    const isUnique = (selector) => {
      if (!selector || typeof queryRoot.querySelectorAll !== "function") return false;
      try {
        const matches = queryRoot.querySelectorAll(selector);
        return matches.length === 1 && matches[0] === element;
      } catch (_) {
        return false;
      }
    };

    const testId = element.getAttribute("data-testid") || element.getAttribute("data-test");
    if (testId) {
      const attribute = element.hasAttribute("data-testid") ? "data-testid" : "data-test";
      const selector = `[${attribute}="${attributeEscape(testId)}"]`;
      if (isUnique(selector)) return selector;
    }

    if (element.id) {
      const selector = `#${cssEscape(element.id)}`;
      if (isUnique(selector)) return selector;
    }

    if (element.getAttribute("name")) {
      const selector = `${element.tagName.toLowerCase()}[name="${attributeEscape(element.getAttribute("name"))}"]`;
      if (isUnique(selector)) return selector;
    }

    // Fall back to a structural path. Grow the path until it is unique in the
    // current root instead of truncating it at an arbitrary depth.
    const parts = [];
    let current = element;
    while (current && current.nodeType === Node.ELEMENT_NODE) {
      const tag = current.tagName.toLowerCase();
      const parent = current.parentElement;
      let suffix = "";
      if (parent) {
        const siblings = Array.from(parent.children)
          .filter((child) => child.tagName === current.tagName);
        if (siblings.length > 1) {
          suffix = `:nth-of-type(${siblings.indexOf(current) + 1})`;
        }
      }

      parts.unshift(`${tag}${suffix}`);
      const selector = parts.join(" > ");
      if (isUnique(selector)) return selector;
      current = parent;
    }

    // The full path should be unique for a concrete DOM element. Returning it
    // is still preferable to silently falling back to a duplicate attribute.
    return parts.join(" > ");
  };

  const labelledByText = (element) => {
    const ids = trimText(element.getAttribute("aria-labelledby")).split(" ").filter(Boolean);
    if (ids.length === 0) return "";
    const root = element.getRootNode();
    return trimText(ids.map((id) => {
      const inRoot = typeof root.getElementById === "function" ? root.getElementById(id) : null;
      const label = inRoot || document.getElementById(id);
      return label ? label.textContent : "";
    }).join(" "));
  };

  const accessibleName = (element) => trimText(
    element.getAttribute("aria-label") ||
    labelledByText(element) ||
    element.getAttribute("title") ||
    (element instanceof HTMLInputElement ? element.value : "") ||
    element.innerText ||
    element.textContent ||
    ""
  );

  const contextText = (element) => {
    const container = element.closest(
      "form, main, article, section, [data-product], [data-item], " +
      "[class*='product'], [class*='order'], [class*='cart'], li"
    );
    const text = trimText(container ? container.innerText : element.parentElement?.innerText);
    return text ? text.slice(0, 240) : null;
  };

  const stableAttributes = (element) => {
    const result = {};
    for (const name of ["id", "name", "type", "data-testid", "data-test", "aria-label"]) {
      const value = element.getAttribute(name);
      if (value) result[name] = value;
    }
    return result;
  };

  const semanticClickable = (element) => {
    const tag = element.tagName.toLowerCase();
    const role = (element.getAttribute("role") || "").toLowerCase();
    const type = (element.getAttribute("type") || "").toLowerCase();
    return tag === "button" ||
      (tag === "a" && element.hasAttribute("href")) ||
      role === "button" ||
      (tag === "input" && ["button", "submit", "image"].includes(type)) ||
      element.hasAttribute("onclick");
  };

  const roots = [{ root: document, shadowPath: [] }];
  const discoveredRoots = new Set([document]);
  for (let index = 0; index < roots.length; index += 1) {
    const entry = roots[index];
    for (const element of entry.root.querySelectorAll("*")) {
      if (element.shadowRoot && !discoveredRoots.has(element.shadowRoot)) {
        discoveredRoots.add(element.shadowRoot);
        roots.push({
          root: element.shadowRoot,
          shadowPath: [...entry.shadowPath, selectorHint(element)]
        });
      }
    }
  }

  const selectors = [
    "button",
    "input[type='submit']",
    "input[type='button']",
    "input[type='image']",
    "[role='button']",
    "a[href]",
    "[onclick]"
  ].join(",");

  const unique = new Set();
  const candidates = [];
  let sequence = 0;

  for (const entry of roots) {
    for (const element of entry.root.querySelectorAll(selectors)) {
      if (unique.has(element)) continue;
      unique.add(element);

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

      let covered = false;
      const intersectsViewport = rect.right > 0 &&
        rect.bottom > 0 &&
        rect.left < window.innerWidth &&
        rect.top < window.innerHeight;
      if (visible && intersectsViewport) {
        const x = Math.min(Math.max(rect.left + rect.width / 2, 0), window.innerWidth - 1);
        const y = Math.min(Math.max(rect.top + rect.height / 2, 0), window.innerHeight - 1);
        const ownerRoot = element.getRootNode();
        const top = typeof ownerRoot.elementFromPoint === "function"
          ? ownerRoot.elementFromPoint(x, y)
          : document.elementFromPoint(x, y);
        covered = Boolean(top && top !== element && !element.contains(top));
      }

      const tagName = element.tagName.toLowerCase();
      const inputType = element.getAttribute("type");
      const role = element.getAttribute("role") || (
        tagName === "button" || (tagName === "input" && ["button", "submit", "image"]
          .includes(String(inputType).toLowerCase()))
          ? "button"
          : tagName === "a" ? "link" : ""
      );
      const hint = selectorHint(element);
      const pathKey = entry.shadowPath.length > 0
        ? `${entry.shadowPath.join(" >>> ")} >>> ${hint}`
        : hint;

      candidates.push({
        candidate_id: `${pathKey}#${sequence++}`,
        tag_name: tagName,
        role,
        input_type: inputType,
        accessible_name: accessibleName(element),
        visible_text: trimText(element.innerText || element.textContent || ""),
        context_text: contextText(element),
        selector_hint: hint,
        shadow_path: entry.shadowPath,
        stable_attributes: stableAttributes(element),
        rect: {
          x: rect.x,
          y: rect.y,
          width: rect.width,
          height: rect.height
        },
        visible,
        enabled,
        pointer_events: pointerEvents,
        covered,
        semantic_clickable: semanticClickable(element),
        score: 0,
        confidence: 0,
        score_reasons: []
      });
    }
  }

  return candidates;
}
