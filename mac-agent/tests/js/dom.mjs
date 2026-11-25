export class Element extends EventTarget {
  constructor(tag) { super(); this.tag = tag; this.children = []; this.value = ""; }
  appendChild(child) { this.children.push(child); return child; }
  removeChild(child) { this.children.splice(this.children.indexOf(child), 1); }
  replaceChildren(...children) { this.children = children; }
  setAttribute(key, value) { this[key] = ["hidden", "disabled"].includes(key) ? true : value; }
  removeAttribute(key) { delete this[key]; }
  get firstChild() { return this.children[0]; }
  set textContent(text) { this.children = [text]; }
  get textContent() { return this.children.map(child => child.textContent ?? child).join(""); }
  find(pred) {
    if (pred(this)) return this;
    for (const child of this.children) { const match = child.find?.(pred); if (match) return match; }
  }
  all(pred) {
    return [...(pred(this) ? [this] : []), ...this.children.flatMap(child => child.all?.(pred) || [])];
  }
  fire(type, value) { if (value !== undefined) this.value = value; this.dispatchEvent(new Event(type)); }
}

globalThis.window = {};
globalThis.Node = globalThis.HTMLElement = Element;
globalThis.document = {createElement: tag => new Element(tag), createTextNode: text => text};
