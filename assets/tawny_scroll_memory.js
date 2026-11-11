const path = __PATH__;
const key = Symbol.for("tawny.scroll-memory");
if (!window[key]) {
  // Per path: how far it was scrolled, and the card at the top of the view.
  const saved = new Map();
  const corrected = new WeakMap();
  let current = null;
  let token = 0;
  let restoring = false;
  // The card the view is held to, and how far below the top of the scroll
  // area it sat: `{ id, offset }`. A list that changes above it - the feed
  // refetched with new uploads on top, a playlist another device added to -
  // would otherwise push the reader's place down the page. Native scroll
  // anchoring cannot do this: a refetch moves cards from one page's grid to
  // the next, so the card is a new element, and WebKit has none anyway.
  let anchor = null;
  // Off while something other than the reader takes the page to the top,
  // so holding the view does not fight it. See `release`.
  let following = true;
  // Not the page kept hidden beneath a sheet.
  const scroller = () => document.querySelector(".g3-content-scroll:not(.tawny-covered-page *)");
  // Bounding boxes include the route, pull-refresher and grid's animated
  // transforms. Scroll correction must follow layout, not chase motion.
  const layoutTop = (node) => {
    let top = 0;
    for (let parent = node; parent; parent = parent.offsetParent) top += parent.offsetTop;
    return top;
  };
  const topWithin = (node, element) => layoutTop(node) - layoutTop(element) - element.clientTop;
  const mark = (element) => {
    element.toggleAttribute("data-scroll-preserving", following && !!anchor);
  };
  const moveTo = (element, top) => {
    if (Math.abs(element.scrollTop - top) <= 1) return;
    element.scrollTop = top;
    corrected.set(element, element.scrollTop);
  };
  // The first card still showing at the top. Cards are in reading order, so
  // their bottoms never decrease, and a binary search reads a few boxes
  // rather than every card above the fold on every scroll event.
  const anchorOf = (element) => {
    // At the top, new uploads are what the reader should see.
    if (element.scrollTop <= 0) return null;
    const cards = element.querySelectorAll("[data-scroll-anchor]");
    let low = 0;
    let high = cards.length;
    while (low < high) {
      const middle = (low + high) >> 1;
      if (topWithin(cards[middle], element) + cards[middle].offsetHeight > element.scrollTop + 1)
        high = middle;
      else low = middle + 1;
    }
    const card = cards[low];
    if (!card) return null;
    return { id: card.dataset.scrollAnchor, offset: topWithin(card, element) - element.scrollTop };
  };
  // How far the view must move to put the anchor back where it sat. Null
  // while the anchor is not drawn: a refetch can draw the page a card moved
  // to a moment after the page it left.
  const anchorShift = (element) => {
    if (!anchor) return null;
    const card = element.querySelector(`[data-scroll-anchor="${CSS.escape(anchor.id)}"]`);
    if (!card) return null;
    return topWithin(card, element) - element.scrollTop - anchor.offset;
  };
  const remember = (element) => {
    if (following) anchor = anchorOf(element);
    if (element.scrollTop <= 0) following = true;
    mark(element);
    saved.set(current, { top: element.scrollTop, height: element.scrollHeight, anchor });
  };
  document.addEventListener(
    "scroll",
    (event) => {
      const element = event.target;
      if (restoring || !current || !(element instanceof Element)) return;
      if (element !== scroller()) return;
      const correction = corrected.get(element);
      corrected.delete(element);
      if (correction !== undefined && Math.abs(element.scrollTop - correction) <= 1) return;
      if (document.documentElement.dataset.routeTransition) return;
      remember(element);
    },
    true,
  );
  // Whoever scrolls for themselves while a position is being put back wins.
  let clearFloor = () => {};
  const yield_to_user = () => {
    token += 1;
    restoring = false;
    following = true;
    clearFloor();
    const element = scroller();
    if (element) corrected.delete(element);
  };
  for (const name of ["wheel", "touchstart", "pointerdown", "keydown"]) {
    document.addEventListener(name, yield_to_user, { capture: true, passive: true });
  }
  // Text replacements also arrive as childList records. Only a changed card
  // tree can move the anchor; player ticks and refresh labels must not force
  // a grid layout while the reader is dragging or pressing a button.
  const containsAnchor = (node) =>
    node.nodeType === 1 &&
    (node.matches("[data-scroll-anchor]") || node.querySelector("[data-scroll-anchor]"));
  new MutationObserver((records) => {
    if (restoring || !anchor || document.documentElement.dataset.routeTransition) return;
    const element = scroller();
    if (
      !element ||
      !records.some(
        (record) =>
          element.contains(record.target) &&
          ([...record.addedNodes, ...record.removedNodes].some(containsAnchor) ||
            record.target.querySelector?.("[data-scroll-anchor]")),
      )
    )
      return;
    const shift = anchorShift(element);
    if (shift !== null) {
      moveTo(element, element.scrollTop + shift);
      saved.set(current, { top: element.scrollTop, height: element.scrollHeight, anchor });
    }
  }).observe(document.body, { childList: true, subtree: true });
  window[key] = {
    arrive(next, restore) {
      clearFloor();
      current = next;
      const mine = ++token;
      restoring = false;
      following = true;
      const entry = restore ? saved.get(next) : null;
      if (!restore) saved.delete(next);
      anchor = entry?.anchor ?? null;
      const target = entry?.top || 0;
      if (!target) {
        const element = scroller();
        if (element) mark(element);
        return;
      }
      restoring = true;
      let floor = null;
      clearFloor = () => {
        if (floor) floor.element.style.minHeight = floor.original;
        floor = null;
      };
      const started = Date.now();
      const step = () => {
        if (mine !== token) return;
        const element = scroller();
        const elapsed = Date.now() - started;
        if (element) {
          mark(element);
          // A deep cached feed is mounted in stages. Reserve the old
          // height until its anchor arrives so scrolling isn't clamped
          // repeatedly against a short placeholder page.
          if (!floor) {
            const content = element.querySelector(":scope > .g3-refresher");
            if (content) {
              floor = { element: content, original: content.style.minHeight };
              content.style.minHeight = `${entry.height}px`;
            }
          }
          // Where the card left at the top is now, if the list has
          // changed since; the old offset until it is drawn.
          const shift = anchorShift(element);
          const goal = shift === null ? target : element.scrollTop + shift;
          moveTo(element, goal);
          // Lists arrive in stages, so a page too short to hold the
          // offset yet is given a moment to grow.
          if (shift !== null && Math.abs(element.scrollTop - goal) <= 1 && elapsed > 250) {
            const reservation = floor;
            clearFloor();
            const finalShift = anchorShift(element);
            const finalGoal = element.scrollTop + (finalShift ?? 0);
            moveTo(element, finalGoal);
            if (Math.abs(element.scrollTop - finalGoal) <= 1) {
              restoring = false;
              return;
            }
            // Finding the card isn't enough: the remaining cached cards below
            // it must also fit the old viewport before the height can be released.
            floor = reservation;
            if (floor) floor.element.style.minHeight = `${entry.height}px`;
            moveTo(element, finalGoal);
          }
        }
        if (elapsed > 2500) {
          clearFloor();
          restoring = false;
          if (element) remember(element);
          return;
        }
        setTimeout(step, 50);
      };
      step();
    },
    // The page is being taken to the top on purpose (the Feed tab tapped
    // again): stop holding the view until it gets there or the reader
    // scrolls.
    release() {
      token += 1;
      restoring = false;
      clearFloor();
      anchor = null;
      following = false;
      const element = scroller();
      if (element) mark(element);
    },
  };
}
const roots = ["/", "/subscriptions", "/playlists", "/explore"];
const transition = document.documentElement.dataset.routeTransition;
const returning = transition === "backward" || transition === "dismiss-sheet";
const restore = !path.startsWith("/watch/") && (returning || roots.includes(path));
window[key].arrive(path, restore);
