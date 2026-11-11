const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { test } = require("node:test");
const vm = require("node:vm");

test("scroll memory measures only card-tree changes, not player and refresh text", () => {
  const script = fs.readFileSync(path.join(__dirname, "../assets/tawny_scroll_memory.js"), "utf8");
  const listeners = new Map();
  let mutated;
  let reads = 0;
  class Element {
    nodeType = 1;
    scrollTop = 100;
    clientTop = 0;
    offsetHeight = 200;
    layoutY = 0;
    dataset = {};
    get offsetTop() {
      reads++;
      return this.layoutY;
    }
    toggleAttribute() {}
    contains() {
      return true;
    }
    matches(selector) {
      return selector === ".g3-content-scroll";
    }
    querySelector() {
      return null;
    }
    getBoundingClientRect() {
      reads++;
      return { top: 0, bottom: 200 };
    }
  }
  const card = new Element();
  card.dataset.scrollAnchor = "video";
  card.matches = (selector) => selector === "[data-scroll-anchor]";
  const scroller = new Element();
  scroller.querySelectorAll = () => [card];
  scroller.querySelector = () => card;
  const document = {
    body: {},
    documentElement: { dataset: {} },
    querySelector: () => scroller,
    addEventListener: (name, listener) => listeners.set(name, listener),
  };
  vm.runInNewContext(script.replace("__PATH__", '"/"'), {
    window: {},
    document,
    Element,
    CSS: { escape: (value) => value },
    MutationObserver: class {
      constructor(callback) {
        mutated = callback;
      }
      observe() {}
    },
  });
  listeners.get("scroll")({ target: scroller });
  reads = 0;
  const text = { nodeType: 3 };
  const icon = new Element();
  const label = new Element();
  mutated([{ target: label, addedNodes: [text, icon], removedNodes: [text] }]);
  assert.equal(reads, 0, "unrelated updates must not force anchor layout");
  card.layoutY = 300;
  mutated([{ target: scroller, addedNodes: [card], removedNodes: [] }]);
  assert.ok(reads > 0, "a changed card tree still restores the anchor");
  assert.equal(scroller.scrollTop, 400);
  listeners.get("scroll")({ target: scroller });
  card.layoutY = 600;
  mutated([{ target: scroller, addedNodes: [card], removedNodes: [] }]);
  assert.equal(scroller.scrollTop, 700, "our own correction must not replace the saved anchor");
  reads = 0;
  const page = new Element();
  page.querySelector = () => card;
  mutated([{ target: scroller, addedNodes: [], removedNodes: [page] }]);
  assert.ok(reads > 0, "removing an ancestor of cards also restores the anchor");
});
