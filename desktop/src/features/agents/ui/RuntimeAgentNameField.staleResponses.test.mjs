import assert from "node:assert/strict";
import test from "node:test";

function installDOMShim() {
  class EventTargetShim {
    constructor() {
      this.listeners = new Map();
    }

    addEventListener(type, listener) {
      this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
    }

    removeEventListener(type, listener) {
      this.listeners.set(
        type,
        (this.listeners.get(type) ?? []).filter(
          (candidate) => candidate !== listener,
        ),
      );
    }

    dispatchEvent(event) {
      for (const listener of this.listeners.get(event.type) ?? []) {
        listener(event);
      }
      return true;
    }
  }

  class NodeShim extends EventTargetShim {
    constructor(tagName, namespaceURI = "http://www.w3.org/1999/xhtml") {
      super();
      this.attributes = {};
      this.childNodes = [];
      this.children = this.childNodes;
      this.namespaceURI = namespaceURI;
      this.nodeName = tagName.toUpperCase();
      this.nodeType = 1;
      this.parentNode = null;
      this.style = {};
      this.tagName = tagName;
      this._nodeValue = null;
    }

    get ownerDocument() {
      return globalThis.document;
    }

    get firstChild() {
      return this.childNodes[0] ?? null;
    }

    get lastChild() {
      return this.childNodes.at(-1) ?? null;
    }

    get nextSibling() {
      if (!this.parentNode) return null;
      const index = this.parentNode.childNodes.indexOf(this);
      return this.parentNode.childNodes[index + 1] ?? null;
    }

    get nodeValue() {
      return this._nodeValue;
    }

    set nodeValue(value) {
      this._nodeValue = value;
    }

    get options() {
      if (this.tagName !== "select") return undefined;
      return this.childNodes.filter((child) => child.tagName === "option");
    }

    get value() {
      return this._value ?? this.attributes.value ?? "";
    }

    set value(value) {
      this._value = String(value);
    }

    get textContent() {
      if (this.nodeType === 3 || this.nodeType === 8) {
        return this._nodeValue ?? "";
      }
      return this.childNodes.map((child) => child.textContent).join("");
    }

    set textContent(value) {
      this.childNodes = [];
      this.children = this.childNodes;
      if (value !== "") {
        this.appendChild(globalThis.document.createTextNode(value));
      }
    }

    setAttribute(name, value) {
      this.attributes[name] = String(value);
    }

    removeAttribute(name) {
      delete this.attributes[name];
    }

    getAttribute(name) {
      return this.attributes[name] ?? null;
    }

    appendChild(child) {
      this.childNodes.push(child);
      child.parentNode = this;
      return child;
    }

    removeChild(child) {
      this.childNodes = this.childNodes.filter(
        (candidate) => candidate !== child,
      );
      this.children = this.childNodes;
      child.parentNode = null;
      return child;
    }

    insertBefore(child, reference) {
      if (!reference) return this.appendChild(child);
      const index = this.childNodes.indexOf(reference);
      if (index < 0) return this.appendChild(child);
      this.childNodes.splice(index, 0, child);
      child.parentNode = this;
      return child;
    }

    contains(node) {
      return (
        this === node ||
        this.childNodes.some((child) => child.contains?.(node) ?? false)
      );
    }
  }

  class DocumentShim extends EventTargetShim {
    constructor() {
      super();
      this.defaultView = globalThis;
      this.nodeType = 9;
    }

    createElement(tagName) {
      return new NodeShim(tagName);
    }

    createElementNS(namespaceURI, tagName) {
      return new NodeShim(tagName, namespaceURI);
    }

    createTextNode(value) {
      const node = new NodeShim("#text");
      node.nodeName = "#text";
      node.nodeType = 3;
      node.nodeValue = String(value);
      return node;
    }

    createComment(value) {
      const node = new NodeShim("#comment");
      node.nodeName = "#comment";
      node.nodeType = 8;
      node.nodeValue = String(value);
      return node;
    }

    get activeElement() {
      return null;
    }

    get body() {
      if (!this._body) this._body = this.createElement("body");
      return this._body;
    }

    contains(node) {
      return node != null;
    }
  }

  globalThis.document = new DocumentShim();
  globalThis.HTMLIFrameElement = NodeShim;
  globalThis.HTMLElement = NodeShim;
  globalThis.IS_REACT_ACT_ENVIRONMENT = true;
  process.env.IS_REACT_ACT_ENVIRONMENT = "true";
  Object.defineProperty(globalThis, "window", {
    configurable: true,
    value: globalThis,
  });
  if (!Object.getOwnPropertyDescriptor(globalThis, "navigator")?.value) {
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: { userAgent: "node" },
    });
  }
  globalThis.MutationObserver = class {
    observe() {}
    disconnect() {}
    takeRecords() {
      return [];
    }
  };
  globalThis.requestAnimationFrame = (callback) => setTimeout(callback, 0);
  globalThis.cancelAnimationFrame = (id) => clearTimeout(id);
}

installDOMShim();

const ipcCalls = [];
globalThis.__TAURI_INTERNALS__ = {
  invoke(command, args) {
    assert.equal(command, "list_intel_agents");
    const pending = deferred();
    ipcCalls.push({ args, ...pending });
    return pending.promise;
  },
  transformCallback() {
    return Math.random();
  },
};

const React = await import("react");
const { createRoot } = await import("react-dom/client");
const { RuntimeAgentNameField } = await import("./RuntimeAgentNameField.tsx");

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, reject, resolve };
}

function roster(name) {
  return {
    agents: [{ id: name, name, description: `${name} description` }],
  };
}

function fieldProps(apiKey) {
  return {
    apiKey,
    disabled: false,
    enabled: true,
    gatewayUrl: "https://intel.test",
    label: "Agent name",
    onValueChange: () => {},
    placeholder: "INTEL_AGENT",
    required: true,
    runtimeId: "intel",
    value: "",
  };
}

async function waitForCallCount(count) {
  const deadline = Date.now() + 5_000;
  while (ipcCalls.length < count && Date.now() < deadline) {
    await React.act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
  }
  assert.equal(
    ipcCalls.length,
    count,
    `expected ${count} roster call(s) before deadline`,
  );
}

async function settle() {
  await React.act(async () => {
    await Promise.resolve();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

function mountedField() {
  const container = document.createElement("div");
  const root = createRoot(container);
  return {
    container,
    render: async (apiKey) => {
      await React.act(async () => {
        root.render(
          React.createElement(RuntimeAgentNameField, fieldProps(apiKey)),
        );
      });
    },
    unmount: async () => {
      await React.act(async () => root.unmount());
    },
  };
}

test("new credential roster wins when the old response resolves last", async () => {
  ipcCalls.length = 0;
  const field = mountedField();
  await field.render("old-key");
  await waitForCallCount(1);
  assert.equal(ipcCalls[0].args.input.apiKey, "old-key");

  await field.render("new-key");
  await waitForCallCount(2);
  assert.equal(ipcCalls[1].args.input.apiKey, "new-key");

  await React.act(async () => ipcCalls[1].resolve(roster("new-agent")));
  await settle();
  assert.match(field.container.textContent, /new-agent/);

  await React.act(async () => ipcCalls[0].resolve(roster("stale-old-agent")));
  await settle();
  assert.match(field.container.textContent, /new-agent/);
  assert.doesNotMatch(field.container.textContent, /stale-old-agent/);

  await field.unmount();
});

test("late failure from the old request cannot replace a successful roster", async () => {
  ipcCalls.length = 0;
  const field = mountedField();
  await field.render("old-key");
  await waitForCallCount(1);

  await field.render("new-key");
  await waitForCallCount(2);
  await React.act(async () => ipcCalls[1].resolve(roster("current-agent")));
  await settle();
  assert.match(field.container.textContent, /current-agent/);

  await React.act(async () =>
    ipcCalls[0].reject(new Error("stale request failed")),
  );
  await settle();
  assert.match(field.container.textContent, /current-agent/);
  assert.doesNotMatch(
    field.container.textContent,
    /Could not reach the Intelligence Platform gateway/,
  );
  assert.equal(
    findByTestId(
      field.container,
      "persona-runtime-agent-roster-connectionError",
    ),
    null,
  );

  await field.unmount();
});

function findByTestId(node, testId) {
  if (node.getAttribute?.("data-testid") === testId) return node;
  for (const child of node.childNodes ?? []) {
    const found = findByTestId(child, testId);
    if (found) return found;
  }
  return null;
}
