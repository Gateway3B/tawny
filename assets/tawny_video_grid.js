(() => {
  const key = Symbol.for("tawny.video-grid-motion");
  if (window[key]) return;
  window[key] = true;

  const grids = new Map();
  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)");
  const timing = { duration: 240, easing: "cubic-bezier(0.2, 0, 0, 1)" };
  const containsCards = (node) =>
    node.nodeType === 1 &&
    (node.matches(".video-card-frame, .video-grid") ||
      node.querySelector(".video-card-frame, .video-grid"));

  function measure() {
    for (const [grid] of grids) {
      if (!grid.isConnected) grids.delete(grid);
    }
    for (const grid of document.querySelectorAll(".video-grid")) {
      const previous = grids.get(grid);
      const visibleHeight = grid.getBoundingClientRect().height;
      for (const animation of grid.getAnimations()) animation.cancel();
      const height = grid.scrollHeight;
      const cards = new Map(
        [...grid.children]
          .filter((node) => node.matches(".video-card-frame"))
          .map((node) => [node, { x: node.offsetLeft, y: node.offsetTop }]),
      );
      const changed =
        previous &&
        (previous.cards.size !== cards.size ||
          [...previous.cards.keys()].some((node) => !cards.has(node)));
      // Offset coordinates ignore scrolling, including the shell restoring
      // its scroll anchor after a removal. A resize gets a fresh baseline.
      if (
        changed &&
        previous.width === grid.clientWidth &&
        !reducedMotion.matches &&
        !document.documentElement.dataset.routeTransition
      ) {
        for (const [node, position] of cards) {
          const before = previous.cards.get(node);
          if (!before) continue;
          const current = new DOMMatrixReadOnly(getComputedStyle(node).transform);
          const x = before.x - position.x + current.m41;
          const y = before.y - position.y + current.m42;
          for (const animation of node.getAnimations()) animation.cancel();
          if (Math.abs(x) > 0.5 || Math.abs(y) > 0.5) {
            node.animate(
              [{ transform: `translate(${x}px, ${y}px)` }, { transform: "translate(0, 0)" }],
              timing,
            );
          }
        }
        if (previous.height !== height) {
          grid.animate([{ height: `${visibleHeight}px` }, { height: `${height}px` }], timing);
        }
      }
      grids.set(grid, { cards, width: grid.clientWidth, height });
    }
  }

  new MutationObserver((records) => {
    if (
      records.some((record) => [...record.addedNodes, ...record.removedNodes].some(containsCards))
    ) {
      measure();
    }
  }).observe(document.body, { childList: true, subtree: true });
  window.addEventListener("resize", measure, { passive: true });
  measure();
})();
