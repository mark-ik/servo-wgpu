// Copyright 2026 Mark Alan Boykin
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// SPDX-License-Identifier: MPL-2.0

(function() {
  // Annex B compatibility used by the upstream WebGL helpers. Nova and some
  // Boa builds intentionally omit this legacy method from the base realm;
  // install the small ES-compatible surface at the browser-host boundary so
  // third-party web content sees the same String API on both engines.
  if (typeof String.prototype.substr !== 'function') {
    String.prototype.substr = function(start, length) {
      var value = String(this);
      var size = value.length;
      start = start === undefined ? 0 : Number(start);
      if (start !== start) start = 0;
      start = start < 0 ? Math.max(size + start, 0) : Math.min(start, size);
      start = start < 0 ? Math.ceil(start) : Math.floor(start);
      if (length === undefined) return value.slice(start);
      length = Number(length);
      if (length !== length || length <= 0) return '';
      length = length < 0 ? Math.ceil(length) : Math.floor(length);
      return value.slice(start, start + length);
    };
  }

  // Wrapper cache keyed by the canonical reflector (engine-side `reflector_for`
  // returns the same reflector object per node), so the same node yields the same
  // wrapper: document.getElementById('x') === document.getElementById('x').
  //
  // A WeakMap, not a Map: a strong Map would root every reflector (key) and wrapper
  // (value) for the realm's life, pinning the underlying node forever and defeating
  // the whole weak-reflector GC (G1-G3) — script could never drop a node. Weak-keyed
  // by the reflector, the wrapper dies when script's last reference does (the
  // reflector and wrapper form an ephemeron cycle the engine collects), the native
  // weak reflector cache then reports the death, and the node is reaped at the next
  // GC tick. (Found by the gc-arena soak: a strong Map peaked at ~12k live nodes
  // under churn; weak-keyed, it stays bounded.)
  var wrappers = new WeakMap();

  // Name validation (DOM "validate" / XML Name + QName productions), used by
  // createElement(NS) / setAttribute(NS) to throw the spec exceptions. The ranges
  // are the XML NameStartChar / NameChar sets; a colon is allowed in a plain Name
  // (createElement does not split on it).
  var NAME_START = ":A-Z_a-z\\u00C0-\\u00D6\\u00D8-\\u00F6\\u00F8-\\u02FF\\u0370-\\u037D\\u037F-\\u1FFF\\u200C-\\u200D\\u2070-\\u218F\\u2C00-\\u2FEF\\u3001-\\uD7FF\\uF900-\\uFDCF\\uFDF0-\\uFFFD";
  var NAME_CHAR = NAME_START + "\\-.0-9\\u00B7\\u0300-\\u036F\\u203F-\\u2040";
  var NAME_RE = new RegExp("^[" + NAME_START + "][" + NAME_CHAR + "]*$");
  function validateName(name) {
    if (!NAME_RE.test(name)) {
      throw new DOMException("The string '" + name + "' is not a valid name.", "InvalidCharacterError");
    }
  }
  // QName: a Name with at most one colon, neither side empty (DOM validate-and-extract,
  // throwing InvalidCharacterError for a malformed qualified name).
  function validateQName(qname) {
    validateName(qname);
    var parts = qname.split(':');
    if (parts.length > 2 || (parts.length === 2 && (parts[0] === '' || parts[1] === ''))) {
      throw new DOMException("The qualified name '" + qname + "' is not valid.", "InvalidCharacterError");
    }
  }
  // validate-and-extract namespace constraints (DOM): a prefix requires a namespace;
  // the `xml`/`xmlns` prefixes are bound to their canonical namespaces.
  function validateNS(ns, qname) {
    validateQName(qname);
    var prefix = qname.indexOf(':') !== -1 ? qname.split(':')[0] : null;
    if (prefix !== null && ns === null) {
      throw new DOMException("A prefix requires a namespace.", "NamespaceError");
    }
    if (prefix === 'xml' && ns !== 'http://www.w3.org/XML/1998/namespace') {
      throw new DOMException("The 'xml' prefix is bound to the XML namespace.", "NamespaceError");
    }
    if ((qname === 'xmlns' || prefix === 'xmlns') && ns !== 'http://www.w3.org/2000/xmlns/') {
      throw new DOMException("The 'xmlns' prefix is bound to the xmlns namespace.", "NamespaceError");
    }
    if (ns === 'http://www.w3.org/2000/xmlns/' && qname !== 'xmlns' && prefix !== 'xmlns') {
      throw new DOMException("The xmlns namespace requires the 'xmlns' prefix.", "NamespaceError");
    }
  }

  // wrapNode is hoisted (function declaration), so the prototype methods defined
  // below may reference it before this point — they only run when called. The
  // prototype is chosen by nodeType, giving the Element / Text split (`instanceof
  // Element`, `node.nodeType`). Within Element (nodeType 1), elements with a tag
  // name in the per-tag table get a more specific prototype (HTMLCanvasElement
  // for CANVAS, etc.) — the rest fall back to HTMLElement. The tag lookup is one
  // native call per element wrap.
  var XHTML_NS = "http://www.w3.org/1999/xhtml";

  function wrapNode(ref) {
    if (ref === undefined || ref === null) return null;
    if (wrappers.has(ref)) return wrappers.get(ref);
    var nt = +__nodeType(ref);
    var proto;
    var customDef = null;
    if (nt === 1) {
      if (__namespaceURI(ref) === XHTML_NS) {
        var tag = __tagName(ref);
        customDef = customElementDefinitionForRef(ref, tag);
        // An HTML-namespaced name the table does not list is HTMLUnknownElement,
        // unless it is a valid custom element name, which is HTMLElement.
        proto = (customDef && customDef.ctor.prototype) ||
                (tag && elementSubclassProto[tag]) ||
                unknownElementProto(tag);
      } else {
        proto = Element.prototype;
      }
    } else {
      proto = nt === 9 ? Document.prototype
            : nt === 3 ? Text.prototype
            : nt === 4 ? CDATASection.prototype
            : nt === 8 ? Comment.prototype
            : nt === 7 ? ProcessingInstruction.prototype
            : nt === 10 ? DocumentType.prototype
            : nt === 11 ? DocumentFragment.prototype
            : Node.prototype;
    }
    var node = Object.create(proto);
    node.__ref = ref;
    node.nodeType = nt;
    wrappers.set(ref, node);
    if (customDef) upgradeCustomElement(node, customDef);
    return node;
  }

  // Per-tag prototype table populated below as HTML* subclasses come online.
  // Each entry's key is the uppercased tag name `__tagName` returns.
  var elementSubclassProto = {};
  function unknownElementProto(tag) {
    var HTMLEl = globalThis.HTMLElement;
    var base = HTMLEl ? HTMLEl.prototype : Element.prototype;
    var Unknown = globalThis.HTMLUnknownElement;
    if (!Unknown || !tag) return base;
    // `__tagName` uppercases, so validity is checked on the folded name.
    return isValidCustomElementName(String(tag).toLowerCase()) ? base : Unknown.prototype;
  }
  var htmlInterfaceConstructors = {};
  var htmlInterfaceDefinitions = Object.create(null);
  var customElementDefinitions = Object.create(null);
  var autonomousCustomElementDefinitions = Object.create(null);
  var customizedBuiltInDefinitions = Object.create(null);
  var customElementDefinitionsByCtor = new Map();
  var reservedCustomElementNames = {
    'annotation-xml': true,
    'color-profile': true,
    'font-face': true,
    'font-face-src': true,
    'font-face-uri': true,
    'font-face-format': true,
    'font-face-name': true,
    'missing-glyph': true
  };
  var upgradedCustomElements = new WeakMap();
  var connectedCustomElements = new WeakMap();
  var ownerDocuments = new WeakMap();
  var iframeDocuments = new WeakMap();
  var iframeWindows = new WeakMap();
  var htmlElementConstructionStack = [];
  var customElementReactionQueue = [];
  var customElementReactionScheduled = false;

  // Node: the base every node shares (tree + events + textContent). Methods live
  // on the prototype (shared, instanceof-able), not per-object. `this.__ref` is
  // the node's reflector.
  function Node() {}
  Node.ELEMENT_NODE = Node.prototype.ELEMENT_NODE = 1;
  Node.TEXT_NODE = Node.prototype.TEXT_NODE = 3;
  Node.CDATA_SECTION_NODE = Node.prototype.CDATA_SECTION_NODE = 4;
  Node.PROCESSING_INSTRUCTION_NODE = Node.prototype.PROCESSING_INSTRUCTION_NODE = 7;
  Node.COMMENT_NODE = Node.prototype.COMMENT_NODE = 8;
  Node.DOCUMENT_TYPE_NODE = Node.prototype.DOCUMENT_TYPE_NODE = 10;
  Node.DOCUMENT_FRAGMENT_NODE = Node.prototype.DOCUMENT_FRAGMENT_NODE = 11;
  Node.ATTRIBUTE_NODE = Node.prototype.ATTRIBUTE_NODE = 2;
  Node.DOCUMENT_NODE = Node.prototype.DOCUMENT_NODE = 9;
  // Pre-insertion validity (DOM): the inserted node must not be an inclusive
  // ancestor of the parent (would form a cycle) → HierarchyRequestError.
  function ensureInsertable(parent, node) {
    if (node === parent || (node.contains && node.contains(parent))) {
      throw new DOMException("The new child is an ancestor of the parent.", "HierarchyRequestError");
    }
  }
  function rootDocument(node) {
    var n = node;
    while (n && n.parentNode) n = n.parentNode;
    return (n && n.nodeType === 9) ? n : null;
  }
  function ownerDocumentOf(node) {
    if (!node) return null;
    if (node.nodeType === 9) return null;
    var root = rootDocument(node);
    if (root) return root;
    return ownerDocuments.get(node) || document;
  }
  function documentForInsertionTarget(parent) {
    return parent ? (parent.nodeType === 9 ? parent : ownerDocumentOf(parent)) : null;
  }
  function movedRoots(node) {
    if (!node) return [];
    if (node.nodeType === 11) {
      var kids = node.childNodes;
      var roots = [];
      for (var i = 0; i < kids.length; i++) roots.push(kids[i]);
      return roots;
    }
    return [node];
  }
  function snapshotTree(root, out) {
    out.push(root);
    var kids = root.childNodes;
    for (var i = 0; i < kids.length; i++) snapshotTree(kids[i], out);
    return out;
  }
  function snapshotMovedNodes(node) {
    return snapshotTree(node, []);
  }
  function setOwnerDocumentSnapshot(nodes, doc) {
    if (!doc) return;
    for (var i = 0; i < nodes.length; i++) {
      if (nodes[i] && nodes[i].nodeType !== 9) ownerDocuments.set(nodes[i], doc);
    }
  }
  function enqueueAdoptedTree(root, oldDoc, newDoc) {
    if (!root || oldDoc === newDoc) return;
    if (root.nodeType === 1 && upgradedCustomElements.get(root)) {
      enqueueCustomElementReaction(root, 'adoptedCallback', [oldDoc, newDoc]);
    }
    var kids = root.childNodes;
    for (var i = 0; i < kids.length; i++) enqueueAdoptedTree(kids[i], oldDoc, newDoc);
  }
  function prepareNodeMove(parent, node) {
    return {
      roots: movedRoots(node),
      snapshot: snapshotMovedNodes(node),
      oldDoc: ownerDocumentOf(node),
      newDoc: documentForInsertionTarget(parent),
      oldParent: node.parentNode
    };
  }
  function disconnectMovedRoots(move) {
    if (!move || !move.oldParent) return;
    for (var i = 0; i < move.roots.length; i++) {
      if (move.roots[i].isConnected) disconnectCustomElementTree(move.roots[i]);
    }
  }
  function finalizeNodeMove(move) {
    if (!move) return;
    if (move.oldDoc && move.newDoc && move.oldDoc !== move.newDoc) {
      setOwnerDocumentSnapshot(move.snapshot, move.newDoc);
      for (var i = 0; i < move.roots.length; i++) {
        enqueueAdoptedTree(move.roots[i], move.oldDoc, move.newDoc);
      }
    }
    for (var j = 0; j < move.roots.length; j++) connectCustomElementTree(move.roots[j]);
  }
  // DOM "insert a node into a parent before a child": a DocumentFragment
  // contributes its **children**, not itself. The children leave the fragment
  // first (one childList record naming all of them) and land under the parent as
  // one group (one record there), which is the record shape the spec's insertion
  // steps produce. `ref === undefined` means append.
  function insertNodeInto(parent, node, ref) {
    if (node.nodeType !== 11) {
      if (ref === undefined) moAppendChild(parent.__ref, node.__ref);
      else moInsertBefore(parent.__ref, node.__ref, ref);
      return;
    }
    var kids = rawChildNodes(node);
    if (!kids.length) return; // spec: count 0, return before any record
    moBeginGroup();
    try {
      for (var i = 0; i < kids.length; i++) moRemoveChild(node.__ref, kids[i].__ref);
      for (var j = 0; j < kids.length; j++) {
        if (ref === undefined) moAppendChild(parent.__ref, kids[j].__ref);
        else moInsertBefore(parent.__ref, kids[j].__ref, ref);
      }
    } finally {
      moEndGroup();
    }
  }
  Node.prototype.appendChild = function(child) {
    ensureInsertable(this, child);
    var move = prepareNodeMove(this, child);
    if (move.oldDoc !== move.newDoc) disconnectMovedRoots(move);
    insertNodeInto(this, child, undefined);
    finalizeNodeMove(move);
    return child;
  };
  Object.defineProperty(Node.prototype, 'textContent', {
    configurable: true,
    get: function() { return __getTextContent(this.__ref); },
    set: function(v) { moSetTextContent(this.__ref, String(v)); }
  });
  Object.defineProperty(Node.prototype, 'parentNode', {
    configurable: true,
    get: function() { return wrapNode(__parentNode(this.__ref)); }
  });
  Object.defineProperty(Node.prototype, 'parentElement', {
    configurable: true,
    get: function() { var p = this.parentNode; return (p && p.nodeType === 1) ? p : null; }
  });
  Object.defineProperty(Node.prototype, 'firstChild', {
    configurable: true, get: function() { return wrapNode(__firstChild(this.__ref)); }
  });
  Object.defineProperty(Node.prototype, 'lastChild', {
    configurable: true, get: function() { return wrapNode(__lastChild(this.__ref)); }
  });
  Object.defineProperty(Node.prototype, 'nextSibling', {
    configurable: true, get: function() { return wrapNode(__nextSibling(this.__ref)); }
  });
  Object.defineProperty(Node.prototype, 'previousSibling', {
    configurable: true, get: function() { return wrapNode(__prevSibling(this.__ref)); }
  });
  Object.defineProperty(Node.prototype, 'childNodes', {
    configurable: true,
    get: function() { var self = this; return makeCollection(function() { return rawChildNodes(self); }, false); }
  });
  Object.defineProperty(Node.prototype, 'nodeName', {
    configurable: true, get: function() { return __nodeName(this.__ref); }
  });
  Object.defineProperty(Node.prototype, 'nodeValue', {
    configurable: true, get: function() { return __nodeValue(this.__ref); }
  });
  Node.prototype.hasChildNodes = function() { return +__childNodesCount(this.__ref) > 0; };
  Node.prototype.contains = function(other) {
    var n = other;
    while (n) { if (n === this) return true; n = n.parentNode; }
    return false;
  };

  // Node identity / connectivity. ownerDocument resolves from the current root
  // document when connected, otherwise from the last adopting/creating document.
  Object.defineProperty(Node.prototype, 'ownerDocument', {
    configurable: true, get: function() { return ownerDocumentOf(this); }
  });
  Object.defineProperty(Node.prototype, 'isConnected', {
    configurable: true,
    get: function() {
      // Connected iff the root reached via parentNode is the live document.
      var n = this;
      while (n.parentNode) n = n.parentNode;
      return n.nodeType === 9;
    }
  });
  // `baseURI` is the node document's base URL: the document URL, overridden by
  // the first `<base href>` in the tree (HTML "document base URL"). It is the
  // same value for every node in a document, attributes included.
  Object.defineProperty(Node.prototype, 'baseURI', {
    configurable: true, get: function() { return documentBaseURI(); }
  });
  function documentBaseURI() {
    var url = globalThis.document ? globalThis.document.URL : undefined;
    if (url === undefined || url === null) return '';
    var bases = globalThis.document.getElementsByTagName('base');
    for (var i = 0; i < bases.length; i++) {
      var href = bases[i].getAttribute('href');
      if (href !== null) return String(__resolve_url(href));
    }
    return String(url);
  }
  Node.prototype.isSameNode = function(other) { return other === this; };
  Node.prototype.getRootNode = function() {
    var n = this; while (n.parentNode) n = n.parentNode; return n;
  };
  // DOCUMENT_POSITION_* bit constants (on both constructor and prototype).
  var DP = {
    DOCUMENT_POSITION_DISCONNECTED: 1, DOCUMENT_POSITION_PRECEDING: 2,
    DOCUMENT_POSITION_FOLLOWING: 4, DOCUMENT_POSITION_CONTAINS: 8,
    DOCUMENT_POSITION_CONTAINED_BY: 16, DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC: 32
  };
  for (var dk in DP) { Node[dk] = Node.prototype[dk] = DP[dk]; }
  Node.prototype.compareDocumentPosition = function(other) {
    if (other === this) return 0;
    // Ancestor chains, root -> node.
    function chain(n) { var c = []; while (n) { c.unshift(n); n = n.parentNode; } return c; }
    var a = chain(this), b = chain(other);
    if (a[0] !== b[0]) {
      // Different trees: disconnected (+ stable implementation-specific order).
      return DP.DOCUMENT_POSITION_DISCONNECTED | DP.DOCUMENT_POSITION_IMPLEMENTATION_SPECIFIC |
             DP.DOCUMENT_POSITION_PRECEDING;
    }
    // Containment.
    if (this.contains(other)) return DP.DOCUMENT_POSITION_CONTAINED_BY | DP.DOCUMENT_POSITION_FOLLOWING;
    if (other.contains(this)) return DP.DOCUMENT_POSITION_CONTAINS | DP.DOCUMENT_POSITION_PRECEDING;
    // Find the divergence point; compare child order there.
    var i = 0; while (i < a.length && i < b.length && a[i] === b[i]) i++;
    var parent = a[i - 1];
    var kids = parent.childNodes;
    var ai = -1, bi = -1;
    for (var k = 0; k < kids.length; k++) { if (kids[k] === a[i]) ai = k; if (kids[k] === b[i]) bi = k; }
    return ai < bi ? DP.DOCUMENT_POSITION_FOLLOWING : DP.DOCUMENT_POSITION_PRECEDING;
  };
  Node.prototype.isEqualNode = function(other) {
    if (!other) return false;
    if (this.nodeType !== other.nodeType) return false;
    switch (this.nodeType) {
      case 1: // Element: localName/namespace/prefix + attributes + children
        if (this.localName !== other.localName || this.namespaceURI !== other.namespaceURI ||
            this.prefix !== other.prefix) return false;
        var aAttrs = this.__ref !== undefined ? __attributeNames(this.__ref) : '';
        var bAttrs = other.__ref !== undefined ? __attributeNames(other.__ref) : '';
        var an = aAttrs ? aAttrs.split(' ').sort() : [];
        var bn = bAttrs ? bAttrs.split(' ').sort() : [];
        if (an.length !== bn.length) return false;
        for (var j = 0; j < an.length; j++) {
          if (an[j] !== bn[j] || this.getAttribute(an[j]) !== other.getAttribute(bn[j])) return false;
        }
        break;
      case 3: case 8: // Text / Comment: same data
        if (this.data !== other.data) return false;
        break;
    }
    var ac = this.childNodes, bc = other.childNodes;
    if (ac.length !== bc.length) return false;
    for (var c = 0; c < ac.length; c++) { if (!ac[c].isEqualNode(bc[c])) return false; }
    return true;
  };
  // DOM `normalize()`: drop empty exclusive Text nodes and merge each run of
  // contiguous ones into its first node. The merge goes through `appendData` and
  // the removal through `removeChild`, so the live-range steps at the mutation
  // funnel see a real replace-data and a real removal.
  Node.prototype.normalize = function() {
    // Walked live, not over a snapshot: the merge removes siblings as it goes.
    var node = this.firstChild;
    while (node) {
      var after = node.nextSibling;
      if (node.nodeType !== 3) { node.normalize(); node = after; continue; }
      if (node.data.length === 0) { this.removeChild(node); node = after; continue; }
      while (node.nextSibling && node.nextSibling.nodeType === 3) {
        var next = node.nextSibling;
        if (next.data.length) node.appendData(next.data);
        this.removeChild(next);
      }
      node = node.nextSibling;
    }
  };
  Node.prototype.cloneNode = function(deep) {
    // Shallow copy of this node by type, then (deep) recurse over children. Pure JS
    // over the existing create* / setAttribute primitives.
    var copy;
    var copyDocument = this.nodeType === 9 ? document : ownerDocumentOf(this);
    switch (this.nodeType) {
      case 1: // Element: clone with namespace + every attribute.
        copy = this.namespaceURI
          ? copyDocument.createElementNS(this.namespaceURI, this.prefix ? this.prefix + ':' + this.localName : this.localName)
          : copyDocument.createElement(this.localName);
        var recs = this.__ref !== undefined ? attributeRecords(this) : [];
        for (var i = 0; i < recs.length; i++) {
          copy.setAttributeNS(recs[i].ns, recs[i].qname,
                              __getAttributeNS(this.__ref, recs[i].ns || '', recs[i].local));
        }
        break;
      case 3: copy = copyDocument.createTextNode(this.data); break;
      case 4: copy = wrapNode(__createCDATASection(this.data)); break;
      case 7:
        copy = copyDocument.createProcessingInstruction(this.target, this.data);
        break;
      case 8: copy = copyDocument.createComment(this.data); break;
      case 10:
        copy = wrapNode(__createDoctype(this.name, this.publicId, this.systemId));
        break;
      case 11: copy = copyDocument.createDocumentFragment(); break;
      case 9: copy = document.implementation.createHTMLDocument(); break;
      default: copy = copyDocument.createTextNode('');
    }
    if (deep) {
      var kids = this.childNodes;
      for (var k = 0; k < kids.length; k++) { copy.appendChild(kids[k].cloneNode(true)); }
    }
    return copy;
  };
  Node.prototype.removeChild = function(child) {
    if (!child || child.parentNode !== this) {
      throw new DOMException("The node to be removed is not a child of this node.", "NotFoundError");
    }
    moRemoveChild(this.__ref, child.__ref);
    disconnectCustomElementTree(child);
    return child;
  };
  Node.prototype.insertBefore = function(node, ref) {
    ensureInsertable(this, node);
    if (ref !== null && ref !== undefined && ref.parentNode !== this) {
      throw new DOMException("The reference node is not a child of this node.", "NotFoundError");
    }
    var move = prepareNodeMove(this, node);
    if (move.oldDoc !== move.newDoc) disconnectMovedRoots(move);
    insertNodeInto(this, node, ref ? ref.__ref : undefined);
    finalizeNodeMove(move);
    return node;
  };
  // The tree top a node hangs from: its document when connected, else the root
  // of its detached tree. moveBefore's same-root gate compares these.
  function treeTopOf(node) {
    var t = node;
    while (t.parentNode) t = t.parentNode;
    return t;
  }
  // DOM `Node.moveBefore(node, child)`: an atomic, state-preserving move — the
  // subtree never disconnects, so retained per-node state survives where
  // insertBefore's remove+insert resets it. Stricter than insertBefore by
  // design: a move never adopts, so both nodes must share one root (both under
  // this document, or both inside the same detached tree), and only
  // element/text/comment nodes move. (moveBefore plan S3.)
  Node.prototype.moveBefore = function(node, ref) {
    if (this.nodeType !== 1 && this.nodeType !== 9 && this.nodeType !== 11) {
      throw new DOMException("This node cannot contain children.", "HierarchyRequestError");
    }
    ensureInsertable(this, node);
    if (node.nodeType !== 1 && node.nodeType !== 3 && node.nodeType !== 8) {
      throw new DOMException("Only element and character data nodes can be moved.", "HierarchyRequestError");
    }
    if (node.nodeType === 3 && this.nodeType === 9) {
      throw new DOMException("Documents cannot contain text nodes.", "HierarchyRequestError");
    }
    if (ref !== null && ref !== undefined && ref.parentNode !== this) {
      throw new DOMException("The reference node is not a child of this node.", "NotFoundError");
    }
    var thisTop = treeTopOf(this);
    if (thisTop !== treeTopOf(node)) {
      throw new DOMException("moveBefore does not adopt: both nodes must share a root.", "HierarchyRequestError");
    }
    moMoveBefore(this.__ref, node.__ref, ref ? ref.__ref : undefined);
    // Custom elements: the spec fires connectedMoveCallback when defined, else
    // the disconnected + connected fallback pair. genet's registry does not
    // capture connectedMoveCallback yet (plan S4), so a connected-tree move
    // fires the fallback pair; a detached-tree move fires nothing.
    if (thisTop.nodeType === 9) {
      disconnectCustomElementTree(node);
      connectCustomElementTree(node);
    }
    return node;
  };
  Node.prototype.replaceChild = function(newChild, oldChild) {
    if (!newChild || !newChild.__ref) {
      throw new TypeError("replaceChild: the replacement is not a Node.");
    }
    if (!oldChild || oldChild.parentNode !== this) {
      throw new DOMException("The node to be replaced is not a child of this node.", "NotFoundError");
    }
    ensureInsertable(this, newChild);
    // DOM `replace`: the reference is resolved first, the replaced child is
    // removed and the new one inserted under one mutation record, and the new
    // node's own removal from wherever it was is a record of its own before it.
    var reference = oldChild.nextSibling;
    if (reference === newChild) reference = newChild.nextSibling;
    if (newChild.parentNode) newChild.parentNode.removeChild(newChild);
    moBeginGroup();
    try {
      if (oldChild.parentNode === this) moRemoveChild(this.__ref, oldChild.__ref);
      this.insertBefore(newChild, reference);
    } finally {
      moEndGroup();
    }
    disconnectCustomElementTree(oldChild);
    return oldChild;
  };

  // Node-level EventTarget with real tree propagation (capture → target → bubble)
  // over the parentNode chain. Listeners live on the (cached) wrapper, keyed by
  // phase: 'c:'+type for capture, 'b:'+type for bubble/target.
  // The 3rd arg of add/removeEventListener is either a boolean `capture` or an
  // options object `{ capture, once, passive }` (DOM §dom-eventtarget-addeventlistener).
  function eventOpts(arg) {
    if (arg && typeof arg === 'object') {
      return { capture: !!arg.capture, once: !!arg.once, passive: !!arg.passive };
    }
    return { capture: !!arg, once: false, passive: false };
  }
  // A listener is stored as `{ cb, once, passive }`: `once` so it can be removed
  // after it first fires; `passive` so its `preventDefault()` is ignored (DOM:
  // a passive listener cannot cancel the default action). Listeners are keyed by
  // phase ('c:'/'b:' + type), so a capture and a bubble listener for the same
  // callback are distinct entries (matching the DOM's (type, callback, capture)
  // listener identity).
  Node.prototype.addEventListener = function(type, cb, opts) {
    if (typeof cb !== 'function') return;
    var o = eventOpts(opts);
    if (!this.__listeners) this.__listeners = {};
    var key = (o.capture ? 'c:' : 'b:') + type;
    var l = this.__listeners[key] || (this.__listeners[key] = []);
    // Duplicate (type, callback, capture) listeners are ignored (DOM spec).
    for (var i = 0; i < l.length; i++) { if (l[i].cb === cb) return; }
    l.push({ cb: cb, once: o.once, passive: o.passive });
  };
  Node.prototype.removeEventListener = function(type, cb, opts) {
    if (!this.__listeners) return;
    var o = eventOpts(opts);
    var l = this.__listeners[(o.capture ? 'c:' : 'b:') + type];
    if (!l) return;
    for (var i = 0; i < l.length; i++) {
      if (l[i].cb === cb) { l.splice(i, 1); return; }
    }
  };
  function fire(node, event, key) {
    if (!node.__listeners) return;
    var l = node.__listeners[key];
    if (!l) return;
    event.currentTarget = node;
    // Snapshot: addEventListener during dispatch must not affect this node's
    // current firing (DOM spec). stopImmediatePropagation halts the rest of
    // THIS node's listeners (not just later nodes — that's __stop).
    var copy = l.slice();
    for (var i = 0; i < copy.length && !event.__stopImmediate; i++) {
      var rec = copy[i];
      // `once`: remove from the live list before calling, so a handler that
      // re-dispatches the same event does not re-enter this listener.
      if (rec.once) {
        var j = l.indexOf(rec);
        if (j !== -1) l.splice(j, 1);
      }
      // `passive`: preventDefault() must be a no-op for the duration of this
      // listener (DOM). The flag is read by Event.preventDefault (lib.rs).
      event.__inPassive = rec.passive;
      // A listener's exception is *reported*, not propagated: dispatch continues
      // to the remaining listeners and dispatchEvent returns normally (DOM
      // §dispatch step "if this throws an exception, report the exception").
      // Without this, one throwing onload/handler errors out the whole test.
      try { rec.cb.call(node, event); }
      catch (ex) { globalThis.__reportListenerException(ex); }
      event.__inPassive = false;
    }
  }
  Node.prototype.dispatchEvent = function(event) {
    // DOM §dispatch: an uninitialized event (createEvent without initEvent) or
    // one already mid-dispatch is an InvalidStateError.
    if (event.__initialized === false || event.__dispatch) {
      throw new DOMException("The event is not initialized or is being dispatched.", "InvalidStateError");
    }
    event.__dispatch = true;
    var path = [];
    var n = this;
    while (n) { path.push(n); n = n.parentNode; }
    // window sits above the document in the propagation path (DOM: the event
    // path's root is the Window for a node in a document). window shares the
    // EventTarget listener-record shape, so `fire` handles it like any node.
    // Appended only when the chain reaches the document (a connected node), so
    // detached subtrees don't spuriously route to window.
    if (path.length && path[path.length - 1].nodeType === 9 && globalThis.window) {
      path.push(globalThis.window);
    }
    event.target = this;
    event.srcElement = this; // legacy alias for target
    event.__path = path;     // composedPath() (no shadow DOM: target → root + window)
    // The stop-propagation flags are NOT cleared here. The DOM clears them
    // *after* dispatch (§dispatch), so an event whose `cancelBubble` /
    // `stopPropagation()` was set *before* dispatch arrives already stopped and
    // fires nothing — every phase below is guarded on `__stop`. Clearing them
    // here instead silently un-stopped such an event.
    // eventPhase constants: NONE 0, CAPTURING 1, AT_TARGET 2, BUBBLING 3.
    // Capture: root → just above the target.
    event.eventPhase = 1;
    for (var i = path.length - 1; i >= 1 && !event.__stop; i--) {
      fire(path[i], event, 'c:' + event.type);
    }
    // Target: capture- then bubble-registered listeners on the target itself.
    event.eventPhase = 2;
    if (!event.__stop) { fire(this, event, 'c:' + event.type); }
    if (!event.__stop) { fire(this, event, 'b:' + event.type); }
    // Bubble: just above the target → root, when the event bubbles.
    event.eventPhase = 3;
    if (event.bubbles) {
      for (var j = 1; j < path.length && !event.__stop; j++) {
        fire(path[j], event, 'b:' + event.type);
      }
    }
    // Clear the dispatch flag + transient fields (DOM: after dispatch
    // currentTarget is null, eventPhase is NONE, the stop flags are unset, and
    // the event may be dispatched again).
    event.__dispatch = false;
    event.currentTarget = null;
    event.eventPhase = 0;
    event.__stop = false;
    event.__stopImmediate = false;
    return !event.__canceled;
  };
  // stopPropagation halts further nodes (the current node's other listeners still
  // run). stopImmediatePropagation also halts the rest of the current node's
  // listeners — and implies stopPropagation (sets both flags), per the DOM spec.
  // Extends the shell's Event, installed before this bootstrap.
  if (globalThis.Event && globalThis.Event.prototype) {
    globalThis.Event.prototype.stopPropagation = function() { this.__stop = true; };
    globalThis.Event.prototype.stopImmediatePropagation = function() {
      this.__stop = true; this.__stopImmediate = true;
    };
    // Legacy initEvent (DOM §dom-event-initevent): (re)initialize a createEvent'd
    // event's type/bubbles/cancelable and set the initialized flag. No-op while
    // mid-dispatch, per spec.
    globalThis.Event.prototype.initEvent = function(type, bubbles, cancelable) {
      if (this.__dispatch) { return; }
      this.__initialized = true;
      this.type = String(type);
      this.bubbles = !!bubbles;
      this.cancelable = !!cancelable;
      this.defaultPrevented = false;
      this.__canceled = false;
    };
    // Legacy cancelBubble: an alias for stopPropagation (set), reflecting the
    // stop flag (get). (DOM keeps it for compat.)
    Object.defineProperty(globalThis.Event.prototype, 'cancelBubble', {
      configurable: true,
      get: function() { return !!this.__stop; },
      set: function(v) { if (v) { this.__stop = true; } },
    });
    // composedPath(): the propagation path recorded during dispatch (target →
    // root). No shadow DOM, so no retargeting/composed boundary to honor yet;
    // returns a fresh copy, or [] outside a dispatch (DOM spec).
    globalThis.Event.prototype.composedPath = function() {
      return this.__path ? this.__path.slice() : [];
    };
    // eventPhase constants (DOM). Instances carry a live `eventPhase` number set
    // during dispatch; these are the named values, on the constructor + proto.
    var E = globalThis.Event;
    E.NONE = 0; E.CAPTURING_PHASE = 1; E.AT_TARGET = 2; E.BUBBLING_PHASE = 3;
    E.prototype.NONE = 0; E.prototype.CAPTURING_PHASE = 1;
    E.prototype.AT_TARGET = 2; E.prototype.BUBBLING_PHASE = 3;
  }

  // CharacterData : Node — the shared text-bearing base for Text and Comment.
  // `data` / `nodeValue` read/write the node's character data; the substring
  // mutators use UTF-16 offsets and throw IndexSizeError out of range (DOM
  // "CharacterData" interface). length is the UTF-16 code-unit count.
  function CharacterData() {}
  CharacterData.prototype = Object.create(Node.prototype);
  Object.defineProperty(CharacterData.prototype, 'data', {
    configurable: true,
    get: function() { var v = __getTextContent(this.__ref); return v === null ? '' : v; },
    // Every write is DOM "replace data", so the live-range steps see a real
    // (offset, count, data) rather than a whole new string.
    set: function(v) { cdReplaceData(this, 0, this.data.length, v === null ? '' : String(v)); }
  });
  Object.defineProperty(CharacterData.prototype, 'length', {
    configurable: true, get: function() { return this.data.length; }
  });
  CharacterData.prototype.substringData = function(offset, count) {
    var d = this.data; offset = offset >>> 0;
    if (offset > d.length) throw new DOMException("offset out of range", "IndexSizeError");
    // slice (core ES), not substr (Annex B — not implemented on all backends).
    return d.slice(offset, offset + (count >>> 0));
  };
  CharacterData.prototype.appendData = function(s) {
    cdReplaceData(this, this.data.length, 0, String(s));
  };
  CharacterData.prototype.insertData = function(offset, s) {
    cdReplaceData(this, offset, 0, String(s));
  };
  CharacterData.prototype.deleteData = function(offset, count) {
    cdReplaceData(this, offset, count, '');
  };
  CharacterData.prototype.replaceData = function(offset, count, s) {
    cdReplaceData(this, offset, count, String(s));
  };
  globalThis.CharacterData = CharacterData;

  // Text : CharacterData. `new Text(data)` mints a detached text node;
  // splitText / wholeText round out the interface.
  function Text(data) {
    if (!(this instanceof Text)) return new Text(data);
    return wrapNode(__createTextNode(data === undefined ? '' : String(data)));
  }
  Text.prototype = Object.create(CharacterData.prototype);
  // DOM "split a Text node": the new node is placed and the live ranges moved
  // onto it *before* the original's data is truncated, which is the spec's
  // order and the only order under which a boundary past the split survives.
  Text.prototype.splitText = function(offset) {
    var d = this.data; offset = offset >>> 0;
    if (offset > d.length) throw new DOMException("offset out of range", "IndexSizeError");
    var newNode = this.ownerDocument.createTextNode(d.slice(offset));
    var parent = this.parentNode;
    if (parent) parent.insertBefore(newNode, this.nextSibling);
    rangeDidSplit(this, newNode, offset);
    cdReplaceData(this, offset, d.length - offset, '');
    return newNode;
  };
  Object.defineProperty(Text.prototype, 'wholeText', {
    configurable: true,
    get: function() {
      // Concatenate this node's contiguous Text siblings (both directions).
      var start = this;
      while (start.previousSibling && start.previousSibling.nodeType === 3) start = start.previousSibling;
      var out = ''; var n = start;
      while (n && n.nodeType === 3) { out += n.data; n = n.nextSibling; }
      return out;
    }
  });
  globalThis.Text = Text;

  // Comment : CharacterData. `new Comment(data)` mints a detached comment node.
  function Comment(data) {
    if (!(this instanceof Comment)) return new Comment(data);
    return wrapNode(__createComment(data === undefined ? '' : String(data)));
  }
  Comment.prototype = Object.create(CharacterData.prototype);
  globalThis.Comment = Comment;

  // CDATASection : Text (nodeType 4). XML-only; `createCDATASection` mints it.
  function CDATASection() { throw new TypeError('Illegal constructor'); }
  CDATASection.prototype = Object.create(Text.prototype);
  Object.defineProperty(CDATASection.prototype, 'constructor', {
    configurable: true, writable: true, value: CDATASection
  });
  globalThis.CDATASection = CDATASection;

  // DocumentType : Node (nodeType 10). A real arena node now, so
  // `document.doctype`, `createDocumentType` and cloning all name the same thing.
  // Its three strings live host-side; `nodeValue` / `textContent` are null.
  function DocumentType() { throw new TypeError('Illegal constructor'); }
  DocumentType.prototype = Object.create(Node.prototype);
  Object.defineProperty(DocumentType.prototype, 'constructor', {
    configurable: true, writable: true, value: DocumentType
  });
  Object.defineProperty(DocumentType.prototype, 'name', {
    configurable: true, get: function() { return __doctypeField(this.__ref, 'name') || ''; }
  });
  Object.defineProperty(DocumentType.prototype, 'publicId', {
    configurable: true, get: function() { return __doctypeField(this.__ref, 'publicId') || ''; }
  });
  Object.defineProperty(DocumentType.prototype, 'systemId', {
    configurable: true, get: function() { return __doctypeField(this.__ref, 'systemId') || ''; }
  });
  Object.defineProperty(DocumentType.prototype, 'textContent', {
    configurable: true, get: function() { return null; }, set: function() {}
  });
  globalThis.DocumentType = DocumentType;

  // ProcessingInstruction : CharacterData (nodeType 7). Its `target` is the
  // node name; `data` is the shared character-data slot. Not constructible —
  // `document.createProcessingInstruction` mints it.
  function ProcessingInstruction() { throw new TypeError('Illegal constructor'); }
  ProcessingInstruction.prototype = Object.create(CharacterData.prototype);
  Object.defineProperty(ProcessingInstruction.prototype, 'constructor', {
    configurable: true, writable: true, value: ProcessingInstruction
  });
  Object.defineProperty(ProcessingInstruction.prototype, 'target', {
    configurable: true, get: function() { return __nodeName(this.__ref); }
  });
  globalThis.ProcessingInstruction = ProcessingInstruction;

  // DocumentFragment : Node. `new DocumentFragment()` mints a detached fragment;
  // querySelector(All)/getElementById scope to it (assigned after Element defines
  // the shared query functions, below).
  function DocumentFragment() {
    if (!(this instanceof DocumentFragment)) return new DocumentFragment();
    return wrapNode(__createFragment());
  }
  DocumentFragment.prototype = Object.create(Node.prototype);
  globalThis.DocumentFragment = DocumentFragment;

  // Element : Node — attributes, reflection, selectors.
  function Element() {}
  Element.prototype = Object.create(Node.prototype);
  Element.prototype.setAttribute = function(name, value) {
    name = String(name); validateName(name);
    // In an HTML element, the qualified name is lowercased.
    if (this.namespaceURI === 'http://www.w3.org/1999/xhtml') name = name.toLowerCase();
    var oldValue = __getAttribute(this.__ref, name);
    var newValue = String(value);
    moSetAttribute(this.__ref, name, newValue);
    if (name === 'style') inlineStyleStates.delete(this);
    customElementAttributeChanged(this, name, oldValue, newValue);
  };
  Element.prototype.getAttribute = function(name) { return __getAttribute(this.__ref, String(name)); };
  Object.defineProperty(Element.prototype, 'innerHTML', {
    configurable: true,
    get: function() { return String(__getInnerHtml(this.__ref)); },
    set: function(value) {
      var oldChildren = this.childNodes;
      for (var i = 0; i < oldChildren.length; i++) disconnectCustomElementTree(oldChildren[i]);
      moSetInnerHtml(this.__ref, String(value));
      var newChildren = this.childNodes;
      for (var j = 0; j < newChildren.length; j++) {
        upgradeCustomElementTree(newChildren[j]);
        if (this.isConnected) connectCustomElementTree(newChildren[j]);
      }
      __refreshNamedProperties();
    }
  });
  Object.defineProperty(Element.prototype, 'outerHTML', {
    configurable: true,
    get: function() { return String(__getOuterHtml(this.__ref)); },
    set: function(value) {
      var parent = this.parentNode;
      if (!parent) {
        throw new DOMException("outerHTML has no parent to replace into.", "NoModificationAllowedError");
      }
      if (parent.nodeType === 9) {
        throw new DOMException("outerHTML cannot replace the document element's parent.", "NoModificationAllowedError");
      }
      // Fragment-parse in the parent's context, then swap this element for the
      // result in one group so an observer sees a single childList record.
      var holder = ownerDocumentOf(this).createElement(
        parent.nodeType === 1 ? parent.localName : 'div');
      holder.innerHTML = String(value);
      var kids = rawChildNodes(holder);
      var fragment = ownerDocumentOf(this).createDocumentFragment();
      for (var i = 0; i < kids.length; i++) fragment.appendChild(kids[i]);
      var next = this.nextSibling;
      moBeginGroup();
      try {
        parent.removeChild(this);
        parent.insertBefore(fragment, next);
      } finally { moEndGroup(); }
    }
  });
  // scrollIntoView: record this element as the host's pending scroll-into-view
  // target (the host resolves it to a viewport scroll after the run). Options
  // (alignToTop / { block, inline, behavior }) are ignored for now: block-start.
  Element.prototype.scrollIntoView = function() { __scrollIntoView(this.__ref); };
  // Namespaced attributes are stored with a real (namespace, prefix, local)
  // name, so `setAttributeNS(XLINK, 'xlink:href')` and `getAttribute('xlink:href')`
  // name the same attribute from opposite directions and a MutationRecord's
  // `attributeNamespace` is the namespace rather than null.
  function nsArg(ns) { return (ns === null || ns === undefined || ns === '') ? '' : String(ns); }
  Element.prototype.setAttributeNS = function(ns, qname, value) {
    ns = (ns === null || ns === undefined) ? null : String(ns);
    qname = String(qname); validateNS(ns, qname);
    var local = qname.indexOf(':') !== -1 ? qname.split(':')[1] : qname;
    var oldValue = __getAttributeNS(this.__ref, nsArg(ns), local);
    var newValue = String(value);
    moSetAttributeNS(this.__ref, nsArg(ns), qname, newValue);
    if (qname === 'style') inlineStyleStates.delete(this);
    customElementAttributeChanged(this, local, oldValue, newValue);
  };
  Element.prototype.getAttributeNS = function(ns, local) {
    return __getAttributeNS(this.__ref, nsArg(ns), String(local));
  };
  Element.prototype.hasAttributeNS = function(ns, local) {
    return __getAttributeNS(this.__ref, nsArg(ns), String(local)) !== null;
  };
  Element.prototype.removeAttributeNS = function(ns, local) {
    local = String(local);
    var oldValue = __getAttributeNS(this.__ref, nsArg(ns), local);
    if (oldValue === null) return;
    moRemoveAttributeNS(this.__ref, nsArg(ns), local);
    customElementAttributeChanged(this, local, oldValue, null);
  };
  Element.prototype.hasAttribute = function(name) { return __getAttribute(this.__ref, String(name)) !== null; };
  Element.prototype.hasAttributes = function() { return __attributeRecords(this.__ref) !== ''; };
  Element.prototype.getAttributeNames = function() {
    return attributeRecords(this).map(function(r) { return r.qname; });
  };
  Element.prototype.removeAttribute = function(name) {
    name = String(name);
    if (this.namespaceURI === 'http://www.w3.org/1999/xhtml') name = name.toLowerCase();
    var oldValue = __getAttribute(this.__ref, name);
    moRemoveAttribute(this.__ref, name);
    if (name === 'style') inlineStyleStates.delete(this);
    customElementAttributeChanged(this, name, oldValue, null);
  };
  Element.prototype.toggleAttribute = function(name, force) {
    var has = this.hasAttribute(name);
    if (force === undefined) force = !has;
    if (force) { if (!has) this.setAttribute(name, ''); return true; }
    if (has) this.removeAttribute(name);
    return false;
  };
  Element.prototype.matches = function(sel) { return __matches(this.__ref, String(sel)) === 'true'; };
  Object.defineProperty(Element.prototype, 'tagName', {
    configurable: true, get: function() { return __tagName(this.__ref); }
  });
  Object.defineProperty(Element.prototype, 'id', {
    configurable: true,
    get: function() { return this.getAttribute('id') || ''; },
    set: function(v) { this.setAttribute('id', String(v)); }
  });
  Object.defineProperty(Element.prototype, 'className', {
    configurable: true,
    get: function() { return this.getAttribute('class') || ''; },
    set: function(v) { this.setAttribute('class', String(v)); }
  });
  Object.defineProperty(Element.prototype, 'localName', {
    configurable: true, get: function() { return __localName(this.__ref); }
  });
  Object.defineProperty(Element.prototype, 'namespaceURI', {
    configurable: true, get: function() { return __namespaceURI(this.__ref); }
  });
  Object.defineProperty(Element.prototype, 'prefix', {
    configurable: true, get: function() { return __prefix(this.__ref); }
  });
  Object.defineProperty(Element.prototype, 'classList', {
    configurable: true, get: function() { return makeDOMTokenList(this, 'class'); }
  });
  Object.defineProperty(Element.prototype, 'dataset', {
    configurable: true, get: function() { return makeDataset(this); }
  });

  // element.style: a CSSStyleDeclaration over the inline `style` content
  // attribute. The declaration string is parsed/serialized in JS (no engine CSS
  // parser here); a value containing ';' (url(), quoted strings) is a known gap.
  // Surface: getPropertyValue / setProperty / removeProperty / item / length /
  // cssText, plus camelCase (`style.fontSize`) and numeric-index access via a
  // Proxy. A fresh declaration is returned each access; all of them read/write
  // the same live attribute, so it stays correct (identity `el.style === el.style`
  // is not preserved — a later refinement).
  function cssKebab(s) { return s.replace(/[A-Z]/g, function(m) { return '-' + m.toLowerCase(); }); }
  function cssParse(text) {
    var out = [];
    if (!text) return out;
    var decls = String(text).split(';');
    for (var i = 0; i < decls.length; i++) {
      var ci = decls[i].indexOf(':');
      if (ci < 0) continue;
      var name = decls[i].slice(0, ci).trim().toLowerCase();
      var value = decls[i].slice(ci + 1).trim();
      if (name) out.push([name, value]);
    }
    return out;
  }
  // Native inline-style records are line-oriented, so values are escaped before
  // crossing the boundary. Keep decoding strict: a malformed native expansion
  // must not turn into a partial CSS declaration update.
  function cssDecodeField(field) {
    var out = '';
    for (var i = 0; i < field.length; i++) {
      var ch = field.charAt(i);
      if (ch !== '\\') { out += ch; continue; }
      if (++i >= field.length) return null;
      ch = field.charAt(i);
      if (ch === '\\') out += '\\';
      else if (ch === 'n') out += '\n';
      else if (ch === 'r') out += '\r';
      else if (ch === 't') out += '\t';
      else return null;
    }
    return out;
  }
  function cssEncodeField(field) {
    return String(field).replace(/\\/g, '\\\\').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
  }
  function cssDecodeList(record) {
    if (record === '') return [];
    var raw = record.split('\n');
    var values = [];
    for (var i = 0; i < raw.length; i++) {
      var value = cssDecodeField(raw[i]);
      if (value === null || value === '') return null;
      values.push(value);
    }
    return values;
  }
  function cssDecodePairs(record) {
    if (record === '') return [];
    var raw = record.split('\n');
    var pairs = [];
    for (var i = 0; i < raw.length; i++) {
      var tab = raw[i].indexOf('\t');
      if (tab <= 0 || raw[i].indexOf('\t', tab + 1) >= 0) return null;
      var name = cssDecodeField(raw[i].slice(0, tab));
      var value = cssDecodeField(raw[i].slice(tab + 1));
      if (name === null || name === '' || value === null) return null;
      pairs.push([name, value]);
    }
    return pairs;
  }
  function cssEncodePairs(pairs) {
    var lines = [];
    for (var i = 0; i < pairs.length; i++) {
      lines.push(cssEncodeField(pairs[i][0]) + '\t' + cssEncodeField(pairs[i][1]));
    }
    return lines.join('\n');
  }
  function cssShorthandComponents(name) {
    var components = cssDecodeList(String(__inlineStyleShorthandComponents(name)));
    return components === null ? [] : components;
  }
  function cssShorthands() {
    var names = cssDecodeList(String(__inlineStyleShorthands()));
    return names === null ? [] : names;
  }
  function cssPendingShorthand(map, shorthand, components) {
    var pending = null;
    for (var i = 0; i < components.length; i++) {
      var index = cssIdx(map, components[i]);
      if (index < 0 || !map[index][2]) return null;
      var origin = map[index][2];
      if (origin[0] !== shorthand) return null;
      if (pending === null) pending = origin;
      else if (pending[1] !== origin[1]) return null;
    }
    return pending;
  }
  function cssShorthandSerialization(map, shorthand, components) {
    if (components.length === 0) return null;
    var rows = [];
    var first = map.length;
    var hasPending = false;
    for (var i = 0; i < components.length; i++) {
      var index = cssIdx(map, components[i]);
      if (index < 0) return null;
      first = Math.min(first, index);
      if (map[index][2]) hasPending = true;
      rows.push([components[i], map[index][1]]);
    }
    var pending = cssPendingShorthand(map, shorthand, components);
    if (pending !== null) return { first: first, value: pending[1], components: components };
    if (hasPending) return null;
    var value = cssDecodeField(String(__inlineStyleShorthandValue(shorthand, cssEncodePairs(rows))));
    return value === null || value === '' ? null : { first: first, value: value, components: components };
  }
  function cssSerialize(map) {
    var shorthands = cssShorthands();
    var serializations = [];
    for (var i = 0; i < shorthands.length; i++) {
      var components = cssShorthandComponents(shorthands[i]);
      var serialization = cssShorthandSerialization(map, shorthands[i], components);
      if (serialization !== null) {
        serialization.name = shorthands[i];
        serializations.push(serialization);
      }
    }
    var parts = [];
    var consumed = [];
    for (var j = 0; j < map.length; j++) consumed.push(false);
    for (var index = 0; index < map.length; index++) {
      if (consumed[index]) continue;
      var emitted = false;
      for (var k = 0; k < serializations.length; k++) {
        var candidate = serializations[k];
        if (candidate.first !== index) continue;
        parts.push(candidate.name + ': ' + candidate.value + ';');
        for (var c = 0; c < candidate.components.length; c++) {
          var component = cssIdx(map, candidate.components[c]);
          if (component >= 0) consumed[component] = true;
        }
        emitted = true;
        break;
      }
      if (!emitted && !consumed[index]) {
        // A pending-substitution longhand is unobservable on its own. This
        // matters after a later longhand mutation breaks an authored shorthand.
        parts.push(map[index][0] + ': ' + (map[index][2] ? '' : map[index][1]) + ';');
      }
    }
    return parts.join(' ');
  }
  function cssStyleValue(name, value) {
    var record = String(__inlineStyleValue(name, value));
    var split = record.indexOf('\n');
    var kind = split < 0 ? record : record.slice(0, split);
    if (kind === 'invalid') return null;
    if (kind === 'canonical') {
      var canonical = cssDecodeField(record.slice(split + 1));
      return canonical === null ? null : { kind: kind, entries: [[name, canonical]] };
    }
    if (kind === 'expanded') {
      var entries = cssDecodePairs(record.slice(split + 1));
      return entries === null || entries.length === 0 ? null : { kind: kind, entries: entries };
    }
    return { kind: kind, entries: [[name, value]] };
  }
  function cssIdx(map, name) {
    for (var i = 0; i < map.length; i++) { if (map[i][0] === name) return i; }
    return -1;
  }
  function cssRemoveNames(map, names) {
    var removed = false;
    for (var i = map.length - 1; i >= 0; i--) {
      if (names.indexOf(map[i][0]) >= 0) { map.splice(i, 1); removed = true; }
    }
    return removed;
  }
  function cssPut(map, name, value, pending) {
    var i = cssIdx(map, name);
    if (i >= 0) {
      map[i][1] = value;
      map[i][2] = pending;
    } else {
      map.push(pending ? [name, value, pending] : [name, value]);
    }
  }
  function cssMatchesComponents(entries, components) {
    if (entries.length !== components.length) return false;
    for (var i = 0; i < components.length; i++) {
      if (entries[i][0] !== components[i]) return false;
    }
    return true;
  }
  function cssApplyDeclaration(map, name, value) {
    var normalized = cssStyleValue(name, value);
    if (normalized === null) return false;
    var components = cssShorthandComponents(name);
    if (normalized.kind === 'expanded' &&
        (components.length === 0 || !cssMatchesComponents(normalized.entries, components))) return false;
    if (normalized.kind === 'deferred' && components.length === 0) return false;
    if (components.length > 0) cssRemoveNames(map, [name].concat(components));
    if (normalized.kind === 'expanded') {
      for (var i = 0; i < normalized.entries.length; i++) {
        cssPut(map, normalized.entries[i][0], normalized.entries[i][1]);
      }
    } else if (normalized.kind === 'deferred') {
      for (var j = 0; j < components.length; j++) {
        cssPut(map, components[j], value, [name, value]);
      }
    } else {
      cssPut(map, name, normalized.entries[0][1]);
    }
    return true;
  }
  function cssCanonicalMap(map) {
    var out = [];
    for (var i = 0; i < map.length; i++) {
      cssApplyDeclaration(out, map[i][0], map[i][1]);
    }
    return out;
  }
  function cssPropertyValue(map, name) {
    var direct = cssIdx(map, name);
    if (direct >= 0) return map[direct][2] ? '' : map[direct][1];
    var components = cssShorthandComponents(name);
    if (components.length === 0) return '';
    var pending = cssPendingShorthand(map, name, components);
    if (pending !== null) return pending[1];
    var rows = [];
    for (var i = 0; i < components.length; i++) {
      var component = cssIdx(map, components[i]);
      if (component < 0 || map[component][2]) return '';
      rows.push([components[i], map[component][1]]);
    }
    var shorthand = cssDecodeField(String(__inlineStyleShorthandValue(name, cssEncodePairs(rows))));
    return shorthand === null ? '' : shorthand;
  }
  function cssRemoveProperty(map, name) {
    return cssRemoveNames(map, [name].concat(cssShorthandComponents(name)));
  }
  // The content attribute cannot retain the internal pending-substitution
  // marker. Keep that unobservable state beside the element while its serialized
  // attribute is unchanged, and invalidate it if the attribute is edited by a
  // different DOM path.
  var inlineStyleStates = new WeakMap();
  function cssCloneMap(map) {
    var cloned = [];
    for (var i = 0; i < map.length; i++) {
      cloned.push(map[i][2] ? [map[i][0], map[i][1], [map[i][2][0], map[i][2][1]]] : [map[i][0], map[i][1]]);
    }
    return cloned;
  }
  function makeStyleDecl(el) {
    function read() {
      var text = el.getAttribute('style');
      var state = inlineStyleStates.get(el);
      if (state && state.text === text) return cssCloneMap(state.map);
      return cssCanonicalMap(cssParse(text));
    }
    function write(map) {
      var text = map.length === 0 ? null : cssSerialize(map);
      if (text === null) el.removeAttribute('style'); else el.setAttribute('style', text);
      inlineStyleStates.set(el, { text: text, map: cssCloneMap(map) });
    }
    var api = {
      getPropertyValue: function(name) { return cssPropertyValue(read(), String(name).toLowerCase()); },
      setProperty: function(name, value) {
        name = String(name).toLowerCase();
        value = (value === undefined || value === null) ? '' : String(value);
        var m = read();
        if (value === '') { if (cssRemoveProperty(m, name)) write(m); return; }
        if (cssApplyDeclaration(m, name, value)) write(m);
      },
      removeProperty: function(name) {
        name = String(name).toLowerCase();
        var m = read(); var old = cssPropertyValue(m, name);
        if (cssRemoveProperty(m, name)) write(m);
        return old;
      },
      item: function(i) { var m = read(); i = i >>> 0; return i < m.length ? m[i][0] : ''; },
    };
    Object.defineProperty(api, 'length', { configurable: true, get: function() { return read().length; } });
    Object.defineProperty(api, 'cssText', {
      configurable: true,
      get: function() { return cssSerialize(read()); },
      set: function(v) { write(cssCanonicalMap(cssParse(v))); },
    });
    var reserved = { getPropertyValue: 1, setProperty: 1, removeProperty: 1, item: 1, length: 1, cssText: 1 };
    return new Proxy(api, {
      get: function(target, prop) {
        if (typeof prop !== 'string' || reserved[prop]) { return target[prop]; }
        if (/^[0-9]+$/.test(prop)) { return target.item(Number(prop)); }
        return target.getPropertyValue(cssKebab(prop));
      },
      set: function(target, prop, value) {
        if (typeof prop === 'string' && !reserved[prop] && !/^[0-9]+$/.test(prop)) {
          target.setProperty(cssKebab(prop), value);
        } else {
          // reserved (e.g. `cssText`) / numeric: run the target's own setter.
          target[prop] = value;
        }
        return true;
      },
      has: function(target, prop) {
        if (typeof prop === 'string' && reserved[prop]) { return true; }
        return typeof prop === 'string' && target.getPropertyValue(cssKebab(prop)) !== '';
      },
    });
  }
  Object.defineProperty(Element.prototype, 'style', {
    configurable: true,
    get: function() { return makeStyleDecl(this); },
    // CSSOM's [PutForwards=cssText]: assigning a string to `element.style`
    // replaces the declaration block rather than the declaration object.
    set: function(value) { makeStyleDecl(this).cssText = String(value); },
  });

  // window.getComputedStyle(el): a read-only CSSStyleDeclaration whose property
  // reads go through the host computed-style seam (`__computedStyleValue`, which
  // calls the host's ComputedStyleHandler over its layout). No handler / unstyled
  // / unsupported property -> "". Enumeration (length / item / iteration over all
  // computed longhands) is not supported in this first cut.
  function makeComputedStyle(el, context) {
    var ref = el ? el.__ref : 0;
    function val(name) {
      var v = context
        ? __computedStyleValueInContext(context.__ref, ref, name)
        : __computedStyleValue(ref, name);
      return v === null ? '' : v;
    }
    var api = {
      getPropertyValue: function(name) { return val(String(name).toLowerCase()); },
      setProperty: function() {},          // read-only
      removeProperty: function() { return ''; }, // read-only
      item: function() { return ''; },     // enumeration unsupported (first cut)
    };
    Object.defineProperty(api, 'length', { configurable: true, get: function() { return 0; } });
    Object.defineProperty(api, 'cssText', { configurable: true, get: function() { return ''; }, set: function() {} });
    var reserved = { getPropertyValue: 1, setProperty: 1, removeProperty: 1, item: 1, length: 1, cssText: 1 };
    return new Proxy(api, {
      get: function(target, prop) {
        if (typeof prop !== 'string' || reserved[prop]) { return target[prop]; }
        if (/^[0-9]+$/.test(prop)) { return ''; }
        return val(cssKebab(prop));
      },
      set: function() { return true; }, // read-only: ignore writes
      has: function(target, prop) {
        if (typeof prop === 'string' && reserved[prop]) { return true; }
        return typeof prop === 'string' && !/^[0-9]+$/.test(prop) && val(cssKebab(prop)) !== '';
      },
    });
  }
  globalThis.getComputedStyle = function(el) { return makeComputedStyle(el); };
  if (globalThis.window) { globalThis.window.getComputedStyle = globalThis.getComputedStyle; }

  // The selected style engine classifies two-argument CSS.supports queries.
  // A one-string declaration is split at its first colon; condition grammar is
  // a later surface.
  var cssApi = globalThis.CSS || {};
  cssApi.supports = function(property, value) {
    if (arguments.length < 2) {
      var declaration = String(property);
      var colon = declaration.indexOf(':');
      if (colon < 1) return false;
      value = declaration.slice(colon + 1).trim();
      property = declaration.slice(0, colon).trim();
    }
    return String(__supportsStyleValue(String(property).toLowerCase(), String(value))) === 'true';
  };
  globalThis.CSS = cssApi;
  if (globalThis.window) { globalThis.window.CSS = cssApi; }

  // Retained author stylesheets. The selected CSS engine owns the actual rule
  // objects; these live wrappers ask it for counts and route CSSOM mutation into
  // its parser. Keys come from the host rather than list indices, so a wrapper
  // remains attached to the same `<style>` / `<link>` while the live list changes.
  function cssomMutationResult(record) {
    var text = String(record);
    var cut = text.indexOf('\n');
    var kind = cut < 0 ? text : text.slice(0, cut);
    var detail = cut < 0 ? '' : text.slice(cut + 1);
    if (kind === 'index') throw new DOMException(detail, 'IndexSizeError');
    if (kind === 'syntax') throw new DOMException(detail, 'SyntaxError');
    return Number(detail);
  }
  function CSSRule() {}
  CSSRule.STYLE_RULE = 1; CSSRule.IMPORT_RULE = 3; CSSRule.MEDIA_RULE = 4; CSSRule.FONT_FACE_RULE = 5;
  CSSRule.KEYFRAMES_RULE = 7; CSSRule.KEYFRAME_RULE = 8; CSSRule.CONTAINER_RULE = 17;
  function MediaList(text) { this.__text = text || ''; }
  Object.defineProperty(MediaList.prototype, 'mediaText', {
    configurable: true,
    get: function() { return this.__text; },
  });
  MediaList.prototype.toString = function() { return this.__text; };

  function cssomPath(path) { return path.length ? path.join('/') : ''; }
  function cssomRuleType(kind) {
    if (kind === 'style') return CSSRule.STYLE_RULE;
    if (kind === 'import') return CSSRule.IMPORT_RULE;
    if (kind === 'font-face') return CSSRule.FONT_FACE_RULE;
    if (kind === 'media') return CSSRule.MEDIA_RULE;
    if (kind === 'keyframes') return CSSRule.KEYFRAMES_RULE;
    if (kind === 'keyframe') return CSSRule.KEYFRAME_RULE;
    if (kind === 'container') return CSSRule.CONTAINER_RULE;
    return 0;
  }
  // Author-rule declarations are intentionally read-only: nested-rule mutation
  // needs a parser/resource transaction, while CSSStyleSheet root mutation is
  // already routed through the retained engine. The surface still supplies the
  // standard reads needed for CSSStyleRule and CSSKeyframeRule inspection.
  function makeRuleStyleDecl(text) {
    var api = {
      getPropertyValue: function(name) {
        var m = cssParse(text), target = String(name).toLowerCase();
        for (var i = 0; i < m.length; i++) if (m[i][0] === target) return m[i][1];
        return '';
      },
      setProperty: function() {}, removeProperty: function() { return ''; },
      item: function(i) { var m = cssParse(text); i = i >>> 0; return i < m.length ? m[i][0] : ''; },
    };
    Object.defineProperty(api, 'length', { configurable: true, get: function() { return cssParse(text).length; } });
    Object.defineProperty(api, 'cssText', { configurable: true, get: function() { return text || ''; }, set: function() {} });
    return new Proxy(api, {
      get: function(target, prop) {
        if (typeof prop !== 'string' || prop === 'getPropertyValue' || prop === 'setProperty' || prop === 'removeProperty' || prop === 'item' || prop === 'length' || prop === 'cssText') return target[prop];
        if (/^[0-9]+$/.test(prop)) return target.item(Number(prop));
        return target.getPropertyValue(cssKebab(prop));
      },
      set: function() { return true; },
    });
  }
  function CSSRuleList(sheetKey, path, parentRule) {
    this.__sheetKey = String(sheetKey); this.__path = path || []; this.__parentRule = parentRule || null;
  }
  Object.defineProperty(CSSRuleList.prototype, 'length', {
    configurable: true,
    get: function() { var n = Number(__styleSheetRuleCount(this.__sheetKey, cssomPath(this.__path))); return n < 0 ? 0 : n; }
  });
  CSSRuleList.prototype.item = function(index) {
    index = Number(index) >>> 0;
    return index >= this.length ? null : cssomRule(this.__sheetKey, this.__path.concat([index]), this.__parentRule);
  };
  function makeRuleList(sheetKey, path, parentRule) {
    var list = new CSSRuleList(sheetKey, path, parentRule);
    return new Proxy(list, { get: function(target, prop) {
      if (typeof prop === 'string' && /^[0-9]+$/.test(prop)) return target.item(Number(prop));
      return target[prop];
    }});
  }
  function CSSRuleBase(sheetKey, path, parentRule) {
    this.__sheetKey = String(sheetKey); this.__path = path; this.__parentRule = parentRule || null;
  }
  CSSRuleBase.prototype = Object.create(CSSRule.prototype); CSSRuleBase.prototype.constructor = CSSRuleBase;
  Object.defineProperty(CSSRuleBase.prototype, 'cssText', { configurable: true, get: function() {
    var text = __styleSheetRuleText(this.__sheetKey, cssomPath(this.__path)); return text === null ? '' : String(text);
  }});
  Object.defineProperty(CSSRuleBase.prototype, 'type', { configurable: true, get: function() {
    var kind = __styleSheetRuleKind(this.__sheetKey, cssomPath(this.__path)); return cssomRuleType(kind === null ? '' : String(kind));
  }});
  Object.defineProperty(CSSRuleBase.prototype, 'parentStyleSheet', { configurable: true, get: function() { return cssomSheet(this.__sheetKey); }});
  Object.defineProperty(CSSRuleBase.prototype, 'parentRule', { configurable: true, get: function() { return this.__parentRule; }});
  function CSSStyleRule(sheetKey, path, parentRule) { CSSRuleBase.call(this, sheetKey, path, parentRule); }
  CSSStyleRule.prototype = Object.create(CSSRuleBase.prototype); CSSStyleRule.prototype.constructor = CSSStyleRule;
  Object.defineProperty(CSSStyleRule.prototype, 'selectorText', { configurable: true, get: function() {
    var value = __styleSheetRuleSelectorText(this.__sheetKey, cssomPath(this.__path)); return value === null ? '' : String(value);
  }});
  Object.defineProperty(CSSStyleRule.prototype, 'style', { configurable: true, get: function() {
    var value = __styleSheetRuleStyleText(this.__sheetKey, cssomPath(this.__path)); return makeRuleStyleDecl(value === null ? '' : String(value));
  }});
  function CSSFontFaceRule(sheetKey, path, parentRule) { CSSRuleBase.call(this, sheetKey, path, parentRule); }
  CSSFontFaceRule.prototype = Object.create(CSSRuleBase.prototype); CSSFontFaceRule.prototype.constructor = CSSFontFaceRule;
  Object.defineProperty(CSSFontFaceRule.prototype, 'style', { configurable: true, get: function() {
    var value = __styleSheetRuleStyleText(this.__sheetKey, cssomPath(this.__path)); return makeRuleStyleDecl(value === null ? '' : String(value));
  }});
  function CSSGroupingRule(sheetKey, path, parentRule) {
    CSSRuleBase.call(this, sheetKey, path, parentRule); this.__rules = makeRuleList(sheetKey, path, this);
  }
  CSSGroupingRule.prototype = Object.create(CSSRuleBase.prototype); CSSGroupingRule.prototype.constructor = CSSGroupingRule;
  Object.defineProperty(CSSGroupingRule.prototype, 'cssRules', { configurable: true, get: function() { return this.__rules; }});
  function CSSMediaRule(sheetKey, path, parentRule) { CSSGroupingRule.call(this, sheetKey, path, parentRule); }
  CSSMediaRule.prototype = Object.create(CSSGroupingRule.prototype); CSSMediaRule.prototype.constructor = CSSMediaRule;
  Object.defineProperty(CSSMediaRule.prototype, 'media', { configurable: true, get: function() {
    var value = __styleSheetRuleConditionText(this.__sheetKey, cssomPath(this.__path)); return new MediaList(value === null ? '' : String(value));
  }});
  function CSSContainerRule(sheetKey, path, parentRule) { CSSGroupingRule.call(this, sheetKey, path, parentRule); }
  CSSContainerRule.prototype = Object.create(CSSGroupingRule.prototype); CSSContainerRule.prototype.constructor = CSSContainerRule;
  Object.defineProperty(CSSContainerRule.prototype, 'conditionText', { configurable: true, get: function() {
    var value = __styleSheetRuleConditionText(this.__sheetKey, cssomPath(this.__path)); return value === null ? '' : String(value);
  }});
  Object.defineProperty(CSSContainerRule.prototype, 'containerName', { configurable: true, get: function() {
    var value = __styleSheetRuleName(this.__sheetKey, cssomPath(this.__path)); return value === null ? '' : String(value);
  }});
  function CSSKeyframesRule(sheetKey, path, parentRule) { CSSGroupingRule.call(this, sheetKey, path, parentRule); }
  CSSKeyframesRule.prototype = Object.create(CSSGroupingRule.prototype); CSSKeyframesRule.prototype.constructor = CSSKeyframesRule;
  Object.defineProperty(CSSKeyframesRule.prototype, 'name', { configurable: true, get: function() {
    var value = __styleSheetRuleName(this.__sheetKey, cssomPath(this.__path)); return value === null ? '' : String(value);
  }});
  function CSSKeyframeRule(sheetKey, path, parentRule) { CSSRuleBase.call(this, sheetKey, path, parentRule); }
  CSSKeyframeRule.prototype = Object.create(CSSRuleBase.prototype); CSSKeyframeRule.prototype.constructor = CSSKeyframeRule;
  Object.defineProperty(CSSKeyframeRule.prototype, 'keyText', { configurable: true, get: function() {
    var value = __styleSheetRuleKeyText(this.__sheetKey, cssomPath(this.__path)); return value === null ? '' : String(value);
  }});
  Object.defineProperty(CSSKeyframeRule.prototype, 'style', { configurable: true, get: function() {
    var value = __styleSheetRuleStyleText(this.__sheetKey, cssomPath(this.__path)); return makeRuleStyleDecl(value === null ? '' : String(value));
  }});
  function CSSImportRule(sheetKey, index) { CSSRuleBase.call(this, sheetKey, [index], null); this.__index = index; }
  CSSImportRule.prototype = Object.create(CSSRuleBase.prototype); CSSImportRule.prototype.constructor = CSSImportRule;
  Object.defineProperty(CSSImportRule.prototype, 'href', { configurable: true, get: function() {
    var href = __styleSheetImportHref(this.__sheetKey, String(this.__index)); return href === null ? '' : String(href);
  }});
  Object.defineProperty(CSSImportRule.prototype, 'media', { configurable: true, get: function() {
    var media = __styleSheetImportMedia(this.__sheetKey, String(this.__index)); return new MediaList(media === null ? '' : String(media));
  }});
  Object.defineProperty(CSSImportRule.prototype, 'styleSheet', { configurable: true, get: function() {
    var child = __styleSheetImportChildKey(this.__sheetKey, String(this.__index)); return child === null ? null : cssomSheet(String(child));
  }});
  function CSSStyleSheet(sheetKey) {
    if (!(this instanceof CSSStyleSheet) || sheetKey === undefined) throw new TypeError('Illegal constructor');
    this.__sheetKey = String(sheetKey); this.__rules = makeRuleList(this.__sheetKey, [], null);
  }
  Object.defineProperty(CSSStyleSheet.prototype, 'cssRules', { configurable: true, get: function() { return this.__rules; }});
  Object.defineProperty(CSSStyleSheet.prototype, 'ownerNode', { configurable: true, get: function() { return wrapNode(__styleSheetOwnerNode(this.__sheetKey)); }});
  Object.defineProperty(CSSStyleSheet.prototype, 'ownerRule', { configurable: true, get: function() {
    var parent = __styleSheetOwnerImportParentKey(this.__sheetKey); if (parent === null) return null;
    var index = Number(__styleSheetOwnerImportIndex(this.__sheetKey)); return index < 0 ? null : cssomImportRule(String(parent), index);
  }});
  CSSStyleSheet.prototype.insertRule = function(rule, index) {
    index = index === undefined ? 0 : (Number(index) >>> 0);
    var result = cssomMutationResult(__insertRule(this.__sheetKey, String(rule), String(index)));
    invalidateCssomRuleCache(this.__sheetKey); return result;
  };
  CSSStyleSheet.prototype.deleteRule = function(index) {
    cssomMutationResult(__deleteRule(this.__sheetKey, String(Number(index) >>> 0)));
    invalidateCssomRuleCache(this.__sheetKey);
  };
  var cssomSheetCache = {}; var cssomRuleCache = {}; var cssomImportCache = {};
  function invalidateCssomRuleCache(sheetKey) {
    var prefix = String(sheetKey) + '\u0000';
    for (var key in cssomRuleCache) if (key.indexOf(prefix) === 0) delete cssomRuleCache[key];
  }
  function cssomSheet(sheetKey) { sheetKey = String(sheetKey); if (!cssomSheetCache[sheetKey]) cssomSheetCache[sheetKey] = new CSSStyleSheet(sheetKey); return cssomSheetCache[sheetKey]; }
  function cssomImportRule(sheetKey, index) {
    var key = String(sheetKey) + '\u0000' + String(index); if (!cssomImportCache[key]) cssomImportCache[key] = new CSSImportRule(String(sheetKey), index); return cssomImportCache[key];
  }
  function cssomRule(sheetKey, path, parentRule) {
    var encoded = cssomPath(path), kind = __styleSheetRuleKind(String(sheetKey), encoded);
    if (kind === null) return null; kind = String(kind);
    if (kind === 'import') return path.length === 1 ? cssomImportRule(sheetKey, path[0]) : null;
    var cacheKey = String(sheetKey) + '\u0000' + encoded, rule = cssomRuleCache[cacheKey];
    if (rule) return rule;
    if (kind === 'style') rule = new CSSStyleRule(sheetKey, path, parentRule);
    else if (kind === 'font-face') rule = new CSSFontFaceRule(sheetKey, path, parentRule);
    else if (kind === 'media') rule = new CSSMediaRule(sheetKey, path, parentRule);
    else if (kind === 'container') rule = new CSSContainerRule(sheetKey, path, parentRule);
    else if (kind === 'keyframes') rule = new CSSKeyframesRule(sheetKey, path, parentRule);
    else if (kind === 'keyframe') rule = new CSSKeyframeRule(sheetKey, path, parentRule);
    else return null;
    cssomRuleCache[cacheKey] = rule; return rule;
  }
  function StyleSheetList() {}
  Object.defineProperty(StyleSheetList.prototype, 'length', {
    configurable: true, get: function() { return Number(__styleSheetCount()); }
  });
  StyleSheetList.prototype.item = function(index) {
    index = Number(index) >>> 0;
    if (index >= this.length) return null;
    var key = String(__styleSheetKey(String(index)));
    if (key === '-1') return null;
    return cssomSheet(key);
  };
  var documentStyleSheets = new Proxy(new StyleSheetList(), {
    get: function(target, prop) {
      if (typeof prop === 'string' && /^[0-9]+$/.test(prop)) return target.item(Number(prop));
      return target[prop];
    }
  });
  globalThis.CSSRuleList = CSSRuleList;
  globalThis.CSSRule = CSSRule;
  globalThis.CSSImportRule = CSSImportRule;
  globalThis.CSSFontFaceRule = CSSFontFaceRule;
  globalThis.CSSStyleRule = CSSStyleRule;
  globalThis.CSSGroupingRule = CSSGroupingRule;
  globalThis.CSSMediaRule = CSSMediaRule;
  globalThis.CSSContainerRule = CSSContainerRule;
  globalThis.CSSKeyframesRule = CSSKeyframesRule;
  globalThis.CSSKeyframeRule = CSSKeyframeRule;
  globalThis.CSSStyleSheet = CSSStyleSheet;
  globalThis.MediaList = MediaList;
  globalThis.StyleSheetList = StyleSheetList;

  // querySelector / querySelectorAll, shared by Element and Document (scope is the
  // receiver). querySelectorAll returns an array (NodeList-approximate).
  function querySelector(sel) { return wrapNode(__querySelector(this.__ref, String(sel))); }
  function querySelectorAll(sel) {
    // querySelectorAll returns a *static* NodeList: snapshot now, captured by the
    // collection's getItems closure.
    var n = +__querySelectorAllCount(this.__ref, String(sel));
    var out = [];
    for (var i = 0; i < n; i++) { out.push(wrapNode(__querySelectorAllItem(this.__ref, String(sel), String(i)))); }
    return makeCollection(function() { return out; }, false);
  }
  Element.prototype.querySelector = querySelector;
  Element.prototype.querySelectorAll = querySelectorAll;

  // getElementsByTagName / getElementsByClassName, shared by Element and Document
  // (scope is the receiver), returning live HTMLCollections.
  function getElementsByTagName(tag) {
    var ref = this.__ref; tag = String(tag);
    return makeCollection(function() {
      var n = +__elementsByTagNameCount(ref, tag);
      var out = [];
      for (var i = 0; i < n; i++) { out.push(wrapNode(__elementsByTagNameItem(ref, tag, String(i)))); }
      return out;
    }, true);
  }
  function getElementsByClassName(cls) {
    var ref = this.__ref;
    var want = String(cls).trim().split(/\s+/).filter(function(s) { return s.length; });
    return makeCollection(function() {
      var n = +__elementsByTagNameCount(ref, '*');
      var out = [];
      for (var i = 0; i < n; i++) {
        var el = wrapNode(__elementsByTagNameItem(ref, '*', String(i)));
        var have = (el.getAttribute('class') || '').trim().split(/\s+/);
        var ok = true;
        for (var j = 0; j < want.length; j++) { if (have.indexOf(want[j]) === -1) { ok = false; break; } }
        if (ok && want.length) out.push(el);
      }
      return out;
    }, true);
  }
  Element.prototype.getElementsByTagName = getElementsByTagName;
  Element.prototype.getElementsByClassName = getElementsByClassName;

  // Element-only tree views: children (a live HTMLCollection of element children),
  // the element siblings, count.
  Object.defineProperty(Element.prototype, 'children', {
    configurable: true,
    get: function() {
      var self = this;
      return makeCollection(function() {
        return rawChildNodes(self).filter(function(n) { return n.nodeType === 1; });
      }, true);
    }
  });
  Object.defineProperty(Element.prototype, 'firstElementChild', {
    configurable: true, get: function() { var c = this.children; return c.length ? c[0] : null; }
  });
  Object.defineProperty(Element.prototype, 'lastElementChild', {
    configurable: true, get: function() { var c = this.children; return c.length ? c[c.length - 1] : null; }
  });
  Object.defineProperty(Element.prototype, 'childElementCount', {
    configurable: true, get: function() { return this.children.length; }
  });
  Object.defineProperty(Element.prototype, 'nextElementSibling', {
    configurable: true,
    get: function() { var n = this.nextSibling; while (n) { if (n.nodeType === 1) return n; n = n.nextSibling; } return null; }
  });
  Object.defineProperty(Element.prototype, 'previousElementSibling', {
    configurable: true,
    get: function() { var n = this.previousSibling; while (n) { if (n.nodeType === 1) return n; n = n.previousSibling; } return null; }
  });

  // ChildNode / ParentNode mixins (DOM §4.2.6-7). Every entry point runs the
  // spec's "convert nodes into a node": a lone node stays itself, a string
  // becomes a text node, and two or more become one DocumentFragment — which is
  // why these can be one insert each rather than a loop, now that inserting a
  // fragment moves its children.
  function toNode(arg, doc) {
    return (typeof arg === 'string') ? (doc || document).createTextNode(arg) : arg;
  }
  function convertNodes(args, doc) {
    doc = doc || document;
    if (args.length === 1) return toNode(args[0], doc);
    var fragment = doc.createDocumentFragment();
    for (var i = 0; i < args.length; i++) fragment.appendChild(toNode(args[i], doc));
    return fragment;
  }
  // The spec's "viable previous/next sibling": the nearest sibling that is not
  // itself one of the nodes being inserted, so `after(this)` and friends behave.
  function viableSibling(node, args, dir) {
    var s = dir < 0 ? node.previousSibling : node.nextSibling;
    while (s) {
      var found = false;
      for (var i = 0; i < args.length; i++) { if (args[i] === s) { found = true; break; } }
      if (!found) return s;
      s = dir < 0 ? s.previousSibling : s.nextSibling;
    }
    return null;
  }
  var childNodeMixin = {
    remove: function() { var p = this.parentNode; if (p) p.removeChild(this); },
    before: function() {
      var p = this.parentNode; if (!p || !arguments.length) return;
      var prev = viableSibling(this, arguments, -1);
      var node = convertNodes(arguments, ownerDocumentOf(this));
      p.insertBefore(node, prev ? prev.nextSibling : p.firstChild);
    },
    after: function() {
      var p = this.parentNode; if (!p || !arguments.length) return;
      var next = viableSibling(this, arguments, 1);
      p.insertBefore(convertNodes(arguments, ownerDocumentOf(this)), next);
    },
    replaceWith: function() {
      var p = this.parentNode; if (!p) return;
      var next = viableSibling(this, arguments, 1);
      var node = arguments.length ? convertNodes(arguments, ownerDocumentOf(this)) : null;
      if (this.parentNode === p) {
        if (node) p.replaceChild(node, this); else p.removeChild(this);
      } else if (node) {
        p.insertBefore(node, next);
      }
    }
  };
  var parentNodeMixin = {
    append: function() {
      if (!arguments.length) return;
      this.appendChild(convertNodes(arguments, this.nodeType === 9 ? this : ownerDocumentOf(this)));
    },
    prepend: function() {
      if (!arguments.length) return;
      var doc = this.nodeType === 9 ? this : ownerDocumentOf(this);
      this.insertBefore(convertNodes(arguments, doc), this.firstChild);
    },
    replaceChildren: function() {
      var doc = this.nodeType === 9 ? this : ownerDocumentOf(this);
      var node = arguments.length ? convertNodes(arguments, doc) : null;
      moBeginGroup();
      try {
        var kids = rawChildNodes(this);
        for (var i = 0; i < kids.length; i++) this.removeChild(kids[i]);
        if (node) this.appendChild(node);
      } finally { moEndGroup(); }
    }
  };
  function installMixin(proto, mixin) {
    for (var k in mixin) {
      if (Object.prototype.hasOwnProperty.call(mixin, k)) {
        Object.defineProperty(proto, k, {
          configurable: true, writable: true, enumerable: false, value: mixin[k]
        });
      }
    }
  }
  // -- Attr and NamedNodeMap -------------------------------------------------
  //
  // An `Attr` is a Node in the DOM, but attributes are stored as a column on
  // their element rather than as arena nodes, so an `Attr` here is a JS view
  // bound to (ownerElement, namespace, localName) that reads and writes through
  // the element -- which is what "live" means for every test that reads
  // `attr.value` after a `setAttribute`. A detached `Attr` (from
  // `createAttribute`) owns its own value until `setAttributeNode` binds it.
  // Views are cached per element so `getAttributeNode('x')` has identity.
  var attrViews = new WeakMap();
  var RS = '\u001e', US = '\u001f';

  function attributeRecords(el) {
    var raw = __attributeRecords(el.__ref);
    if (!raw) return [];
    var parts = raw.split(RS);
    var out = [];
    for (var i = 0; i < parts.length; i++) {
      var f = parts[i].split(US);
      var prefix = f[1] || null;
      out.push({
        ns: f[0] || null,
        prefix: prefix,
        local: f[2],
        qname: prefix ? prefix + ':' + f[2] : f[2]
      });
    }
    return out;
  }
  function attrKey(ns, local) { return (ns || '') + US + local; }

  function Attr() { throw new TypeError('Illegal constructor'); }
  Attr.prototype = Object.create(Node.prototype);
  Object.defineProperty(Attr.prototype, 'constructor', {
    configurable: true, writable: true, value: Attr
  });
  function makeAttr(owner, ns, prefix, local, doc) {
    var a = Object.create(Attr.prototype);
    a.nodeType = 2;
    a.__owner = owner || null;
    a.__ns = ns || null;
    a.__prefix = prefix || null;
    a.__local = String(local);
    a.__value = '';
    a.__doc = doc || document;
    a.__isAttr = true; // the Range boundary guard rejects an Attr container
    return a;
  }
  function attrFor(el, ns, prefix, local) {
    var byKey = attrViews.get(el);
    if (!byKey) { byKey = Object.create(null); attrViews.set(el, byKey); }
    var key = attrKey(ns, local);
    var existing = byKey[key];
    if (existing) { existing.__prefix = prefix || null; return existing; }
    var a = makeAttr(el, ns, prefix, local, ownerDocumentOf(el));
    byKey[key] = a;
    return a;
  }
  Object.defineProperty(Attr.prototype, 'namespaceURI', {
    configurable: true, get: function() { return this.__ns; }
  });
  Object.defineProperty(Attr.prototype, 'prefix', {
    configurable: true, get: function() { return this.__prefix; }
  });
  Object.defineProperty(Attr.prototype, 'localName', {
    configurable: true, get: function() { return this.__local; }
  });
  Object.defineProperty(Attr.prototype, 'name', {
    configurable: true,
    get: function() { return this.__prefix ? this.__prefix + ':' + this.__local : this.__local; }
  });
  Object.defineProperty(Attr.prototype, 'nodeName', {
    configurable: true, get: function() { return this.name; }
  });
  Object.defineProperty(Attr.prototype, 'ownerElement', {
    configurable: true, get: function() { return this.__owner; }
  });
  Object.defineProperty(Attr.prototype, 'ownerDocument', {
    configurable: true,
    get: function() { return this.__owner ? ownerDocumentOf(this.__owner) : this.__doc; }
  });
  Object.defineProperty(Attr.prototype, 'specified', {
    configurable: true, get: function() { return true; }
  });
  function attrValue(a) {
    if (!a.__owner) return a.__value;
    var v = __getAttributeNS(a.__owner.__ref, a.__ns || '', a.__local);
    return v === null ? a.__value : v;
  }
  function setAttrValue(a, v) {
    v = String(v);
    a.__value = v;
    if (a.__owner) a.__owner.setAttributeNS(a.__ns, a.name, v);
  }
  var attrValueDescriptor = {
    configurable: true,
    get: function() { return attrValue(this); },
    set: function(v) { setAttrValue(this, v); }
  };
  Object.defineProperty(Attr.prototype, 'value', attrValueDescriptor);
  Object.defineProperty(Attr.prototype, 'nodeValue', attrValueDescriptor);
  Object.defineProperty(Attr.prototype, 'textContent', attrValueDescriptor);
  Object.defineProperty(Attr.prototype, 'childNodes', {
    configurable: true, get: function() { return makeCollection(function() { return []; }, false); }
  });
  Object.defineProperty(Attr.prototype, 'parentNode', {
    configurable: true, get: function() { return null; }
  });
  Attr.prototype.hasChildNodes = function() { return false; };
  Attr.prototype.cloneNode = function() {
    var copy = makeAttr(null, this.__ns, this.__prefix, this.__local, this.ownerDocument);
    copy.__value = this.value;
    return copy;
  };
  Attr.prototype.isEqualNode = function(other) {
    return !!other && other.nodeType === 2 && other.__ns === this.__ns &&
           other.__local === this.__local && other.value === this.value;
  };
  globalThis.Attr = Attr;

  // NamedNodeMap: a live, indexed view of one element's attributes. The proxy
  // adds the indexed and named property access the IDL's getters describe.
  function NamedNodeMap() { throw new TypeError('Illegal constructor'); }
  NamedNodeMap.prototype.item = function(i) {
    i = Number(i) >>> 0;
    var recs = attributeRecords(this.__el);
    if (i >= recs.length) return null;
    var r = recs[i];
    return attrFor(this.__el, r.ns, r.prefix, r.local);
  };
  NamedNodeMap.prototype.getNamedItem = function(qname) {
    return this.__el.getAttributeNode(qname);
  };
  NamedNodeMap.prototype.getNamedItemNS = function(ns, local) {
    return this.__el.getAttributeNodeNS(ns, local);
  };
  NamedNodeMap.prototype.setNamedItem = function(attr) { return this.__el.setAttributeNode(attr); };
  NamedNodeMap.prototype.setNamedItemNS = function(attr) { return this.__el.setAttributeNodeNS(attr); };
  NamedNodeMap.prototype.removeNamedItem = function(qname) {
    var a = this.__el.getAttributeNode(qname);
    if (!a) throw new DOMException("No attribute named '" + qname + "'.", "NotFoundError");
    this.__el.removeAttribute(qname);
    return a;
  };
  NamedNodeMap.prototype.removeNamedItemNS = function(ns, local) {
    var a = this.__el.getAttributeNodeNS(ns, local);
    if (!a) throw new DOMException("No such attribute.", "NotFoundError");
    this.__el.removeAttributeNS(ns, local);
    return a;
  };
  Object.defineProperty(NamedNodeMap.prototype, 'length', {
    configurable: true, get: function() { return attributeRecords(this.__el).length; }
  });
  globalThis.NamedNodeMap = NamedNodeMap;
  var namedNodeMaps = new WeakMap();
  function namedNodeMapFor(el) {
    var existing = namedNodeMaps.get(el);
    if (existing) return existing;
    var target = Object.create(NamedNodeMap.prototype);
    target.__el = el;
    var proxy = new Proxy(target, {
      get: function(t, prop) {
        if (typeof prop === 'string' && /^[0-9]+$/.test(prop)) return t.item(Number(prop));
        if (typeof prop === 'string' && !(prop in t)) return t.getNamedItem(prop) || undefined;
        return t[prop];
      },
      has: function(t, prop) {
        if (typeof prop === 'string' && /^[0-9]+$/.test(prop)) return Number(prop) < t.length;
        return (prop in t) || (typeof prop === 'string' && !!t.getNamedItem(prop));
      }
    });
    namedNodeMaps.set(el, proxy);
    return proxy;
  }
  Object.defineProperty(Element.prototype, 'attributes', {
    configurable: true, get: function() { return namedNodeMapFor(this); }
  });
  Element.prototype.getAttributeNode = function(qname) {
    qname = String(qname);
    if (this.namespaceURI === 'http://www.w3.org/1999/xhtml') qname = qname.toLowerCase();
    var recs = attributeRecords(this);
    for (var i = 0; i < recs.length; i++) {
      if (recs[i].qname === qname) return attrFor(this, recs[i].ns, recs[i].prefix, recs[i].local);
    }
    return null;
  };
  Element.prototype.getAttributeNodeNS = function(ns, local) {
    ns = nsArg(ns); local = String(local);
    var recs = attributeRecords(this);
    for (var i = 0; i < recs.length; i++) {
      if ((recs[i].ns || '') === ns && recs[i].local === local) {
        return attrFor(this, recs[i].ns, recs[i].prefix, recs[i].local);
      }
    }
    return null;
  };
  function setAttrNode(el, attr) {
    if (!attr || attr.nodeType !== 2) throw new TypeError('not an Attr');
    if (attr.__owner && attr.__owner !== el) {
      throw new DOMException("The attribute is in use by another element.", "InUseAttributeError");
    }
    var old = el.getAttributeNodeNS(attr.__ns, attr.__local);
    attr.__owner = el;
    el.setAttributeNS(attr.__ns, attr.name, attr.__value);
    var byKey = attrViews.get(el);
    if (!byKey) { byKey = Object.create(null); attrViews.set(el, byKey); }
    byKey[attrKey(attr.__ns, attr.__local)] = attr;
    return (old && old !== attr) ? old : null;
  }
  Element.prototype.setAttributeNode = function(attr) { return setAttrNode(this, attr); };
  Element.prototype.setAttributeNodeNS = function(attr) { return setAttrNode(this, attr); };
  Element.prototype.removeAttributeNode = function(attr) {
    if (!attr || attr.__owner !== this) {
      throw new DOMException("The attribute is not on this element.", "NotFoundError");
    }
    attr.__value = attr.value;
    this.removeAttributeNS(attr.__ns, attr.__local);
    attr.__owner = null;
    var byKey = attrViews.get(this);
    if (byKey) delete byKey[attrKey(attr.__ns, attr.__local)];
    return attr;
  };

  installMixin(Element.prototype, childNodeMixin);
  installMixin(CharacterData.prototype, childNodeMixin);
  installMixin(DocumentType.prototype, childNodeMixin);
  installMixin(Element.prototype, parentNodeMixin);
  installMixin(DocumentFragment.prototype, parentNodeMixin);

  function customElementKey(tag, isValue) {
    return String(tag).toUpperCase() + '\n' + String(isValue);
  }

  function customElementSyntaxError(name) {
    return new (globalThis.DOMException || TypeError)('invalid custom element name', 'SyntaxError');
  }

  function isValidCustomElementName(name) {
    name = String(name);
    if (reservedCustomElementNames[name]) return false;
    if (name.indexOf('-') === -1) return false;
    if (name !== name.toLowerCase()) return false;
    try {
      validateName(name);
    } catch (_) {
      return false;
    }
    return true;
  }

  function customElementCallback(holder, name) {
    var value = holder[name];
    if (value !== undefined && value !== null && typeof value !== 'function') {
      throw new TypeError(name + ' must be a function');
    }
    return value || null;
  }

  function toDomStringSequence(value) {
    if (value === undefined || value === null) return [];
    var iter = value[Symbol.iterator];
    if (typeof iter !== 'function') throw new TypeError('value is not iterable');
    var iterator = iter.call(value);
    var out = [];
    while (true) {
      var step = iterator.next();
      if (step.done) return out;
      out.push(String(step.value));
    }
  }

  function setElementPrototype(el, proto) {
    if (Object.setPrototypeOf) Object.setPrototypeOf(el, proto);
    else el.__proto__ = proto;
  }

  function isClassConstructor(ctor) {
    try { return /^class\b/.test(Function.prototype.toString.call(ctor)); }
    catch (_) { return false; }
  }

  function customElementDefinitionForRef(ref, tag) {
    var isValue = __getAttribute(ref, 'is');
    if (isValue !== null) {
      return customizedBuiltInDefinitions[customElementKey(tag, isValue)] || null;
    }
    return autonomousCustomElementDefinitions[String(tag).toUpperCase()] || null;
  }

  function customElementDefinitionForElement(el) {
    if (!el || el.nodeType !== 1 || el.namespaceURI !== XHTML_NS) return null;
    var isValue = el.getAttribute('is');
    if (isValue !== null) {
      return customizedBuiltInDefinitions[customElementKey(el.tagName, isValue)] || null;
    }
    return autonomousCustomElementDefinitions[el.tagName] || null;
  }

  function scheduleCustomElementReactions() {
    if (customElementReactionScheduled) return;
    customElementReactionScheduled = true;
    Promise.resolve().then(flushCustomElementReactions);
  }

  function enqueueCustomElementReaction(el, name, args) {
    customElementReactionQueue.push({ el: el, name: name, args: args || [] });
    scheduleCustomElementReactions();
  }

  function flushCustomElementReactions() {
    customElementReactionScheduled = false;
    while (customElementReactionQueue.length) {
      var reaction = customElementReactionQueue.shift();
      var cb = reaction.el && reaction.el[reaction.name];
      if (typeof cb === 'function') cb.apply(reaction.el, reaction.args);
    }
    if (customElementReactionQueue.length) scheduleCustomElementReactions();
  }

  function observesAttribute(def, attr) {
    var observed = def && def.observedAttributes;
    if (!observed) return false;
    for (var i = 0; i < observed.length; i++) {
      if (observed[i] === attr) return true;
    }
    return false;
  }

  function customElementAttributeChanged(el, attr, oldValue, newValue) {
    var def = upgradedCustomElements.get(el);
    if (!def || oldValue === newValue || !observesAttribute(def, attr)) return;
    enqueueCustomElementReaction(el, 'attributeChangedCallback', [attr, oldValue, newValue]);
  }

  function enqueueInitialAttributeReactions(el, def) {
    var observed = def && def.observedAttributes;
    if (!observed) return;
    for (var i = 0; i < observed.length; i++) {
      var attr = observed[i];
      var value = el.getAttribute(attr);
      if (value !== null) {
        enqueueCustomElementReaction(el, 'attributeChangedCallback', [attr, null, value]);
      }
    }
  }

  function constructCustomElementWithStack(el, def) {
    htmlElementConstructionStack.push(el);
    try {
      Reflect.construct(def.ctor, [], def.ctor);
    } finally {
      if (htmlElementConstructionStack[htmlElementConstructionStack.length - 1] === el) {
        htmlElementConstructionStack.pop();
      }
    }
  }

  function constructCustomElement(el, def) {
    if (isClassConstructor(def.ctor)) {
      constructCustomElementWithStack(el, def);
    } else {
      try {
        def.ctor.call(el);
      } catch (err) {
        var msg = String((err && err.message) || err).toLowerCase();
        if (typeof Reflect === 'object' && Reflect.construct && msg.indexOf('class constructor') !== -1) {
          constructCustomElementWithStack(el, def);
          return;
        }
        throw err;
      }
    }
  }

  function connectCustomElement(el) {
    var def = upgradedCustomElements.get(el);
    if (!def || !el.isConnected || connectedCustomElements.get(el)) return;
    connectedCustomElements.set(el, true);
    enqueueCustomElementReaction(el, 'connectedCallback', []);
  }

  function disconnectCustomElement(el) {
    var def = upgradedCustomElements.get(el);
    if (!def || !connectedCustomElements.get(el)) return;
    connectedCustomElements.delete(el);
    enqueueCustomElementReaction(el, 'disconnectedCallback', []);
  }

  function upgradeCustomElement(el, def) {
    if (!el || upgradedCustomElements.get(el) === def) return el;
    setElementPrototype(el, def.ctor.prototype);
    upgradedCustomElements.set(el, def);
    constructCustomElement(el, def);
    enqueueInitialAttributeReactions(el, def);
    connectCustomElement(el);
    return el;
  }

  function customElementConstructibleWithHtmlInterface(htmlName, htmlDef, def) {
    if (!def) return false;
    if (htmlName === 'HTMLElement') return !def.customizedBuiltIn;
    if (!def.customizedBuiltIn) return false;
    var tags = (htmlDef && htmlDef.tags) || [];
    for (var i = 0; i < tags.length; i++) {
      if (tags[i] === def.localName) return true;
    }
    return false;
  }

  function createElementForHtmlConstructor(Ctor, htmlName, newTarget) {
    var def = customElementDefinitionsByCtor.get(newTarget);
    var htmlDef = htmlInterfaceDefinitions[htmlName];
    if (!customElementConstructibleWithHtmlInterface(htmlName, htmlDef, def)) {
      throw new TypeError('Invalid custom element constructor');
    }
    var proto = newTarget.prototype;
    if ((!proto || typeof proto !== 'object') && typeof proto !== 'function') {
      proto = Ctor.prototype;
    }
    var el = wrapNode(__createElement(def.localName));
    ownerDocuments.set(el, document);
    if (def.customizedBuiltIn) el.setAttribute('is', def.name);
    setElementPrototype(el, proto);
    upgradedCustomElements.set(el, def);
    return el;
  }

  function upgradeCustomElementTree(root) {
    if (!root) return;
    if (root.nodeType === 1) {
      var def = customElementDefinitionForElement(root);
      if (def) upgradeCustomElement(root, def);
    }
    var kids = root.childNodes;
    for (var i = 0; i < kids.length; i++) {
      upgradeCustomElementTree(kids[i]);
    }
  }

  function connectCustomElementTree(root) {
    if (!root) return;
    if (root.nodeType === 1) connectCustomElement(root);
    var kids = root.childNodes;
    for (var i = 0; i < kids.length; i++) {
      connectCustomElementTree(kids[i]);
    }
  }

  function disconnectCustomElementTree(root) {
    if (!root) return;
    if (root.nodeType === 1) disconnectCustomElement(root);
    var kids = root.childNodes;
    for (var i = 0; i < kids.length; i++) {
      disconnectCustomElementTree(kids[i]);
    }
  }

  function customElementIsOption(options) {
    if (options === undefined || options === null) return null;
    if (typeof options === 'string') return String(options);
    if (typeof options === 'object' && options.is !== undefined) return String(options.is);
    return null;
  }

  // Document : Node, with the construction/lookup methods.
  // `new Document()` mints an XML document (DOM: the constructor's document is
  // "XML", which is what makes `createCDATASection` legal on it).
  function Document() {
    if (!(this instanceof Document)) {
      throw new TypeError("Constructor Document requires 'new'");
    }
    var doc = wrapNode(__createDocument());
    doc.__isHtml = false;
    return doc;
  }
  Document.prototype = Object.create(Node.prototype);
  Object.defineProperty(Document.prototype, 'styleSheets', {
    configurable: true, get: function() { return documentStyleSheets; }
  });
  // document.cookie reads/writes the host's cookie store (the session jar). The get
  // returns the document's script-visible cookies ("n1=v1; n2=v2"); the set records
  // one Set-Cookie-style assignment. No store -> "" / no-op.
  Object.defineProperty(Document.prototype, 'cookie', {
    get: function() { return __cookieGet(); },
    set: function(v) { __cookieSet(String(v)); },
    configurable: true,
    enumerable: true
  });
  Document.prototype.createElement = function(tag, options) {
    tag = String(tag); validateName(tag);
    // HTML document: the local name is lowercased.
    tag = tag.toLowerCase();
    var el = wrapNode(__createElement(tag));
    ownerDocuments.set(el, this);
    var isValue = customElementIsOption(options);
    if (isValue !== null) {
      el.setAttribute('is', isValue);
    }
    var def = customElementDefinitionForElement(el);
    if (def) upgradeCustomElement(el, def);
    return el;
  };
  Document.prototype.createElementNS = function(ns, qname) {
    ns = (ns === null || ns === undefined) ? null : String(ns);
    qname = String(qname);
    validateNS(ns, qname);
    var el = wrapNode(__createElementNS(ns === null ? '' : ns, qname));
    ownerDocuments.set(el, this);
    return el;
  };
  Document.prototype.createTextNode = function(data) {
    var node = wrapNode(__createTextNode(String(data)));
    ownerDocuments.set(node, this);
    return node;
  };
  Document.prototype.createComment = function(data) {
    var node = wrapNode(__createComment(String(data)));
    ownerDocuments.set(node, this);
    return node;
  };
  Document.prototype.createProcessingInstruction = function(target, data) {
    target = String(target);
    validateName(target);
    data = String(data);
    if (data.indexOf('?>') >= 0) {
      throw new DOMException("data contains '?>'", "InvalidCharacterError");
    }
    var node = wrapNode(__createProcessingInstruction(target, data));
    ownerDocuments.set(node, this);
    return node;
  };
  // XML only: the DOM throws NotSupportedError for an HTML document, and
  // InvalidCharacterError when the data would close the section.
  Document.prototype.createCDATASection = function(data) {
    if (this.__isHtml !== false) {
      throw new DOMException("createCDATASection is not available on an HTML document.", "NotSupportedError");
    }
    data = String(data);
    if (data.indexOf(']]>') >= 0) {
      throw new DOMException("data contains ']]>'", "InvalidCharacterError");
    }
    var node = wrapNode(__createCDATASection(data));
    ownerDocuments.set(node, this);
    return node;
  };
  Document.prototype.createAttribute = function(local) {
    local = String(local); validateName(local);
    if (this.__isHtml !== false) local = local.toLowerCase();
    return makeAttr(null, null, null, local, this);
  };
  Document.prototype.createAttributeNS = function(ns, qname) {
    ns = (ns === null || ns === undefined) ? null : String(ns);
    qname = String(qname); validateNS(ns, qname);
    var cut = qname.indexOf(':');
    var prefix = cut === -1 ? null : qname.slice(0, cut);
    var local = cut === -1 ? qname : qname.slice(cut + 1);
    return makeAttr(null, ns, prefix, local, this);
  };
  Document.prototype.createDocumentFragment = function() {
    var node = wrapNode(__createFragment());
    ownerDocuments.set(node, this);
    return node;
  };
  // Range and Selection entry points. `createRange` mints a live range already
  // collapsed at (document, 0); `getSelection` hands back the one Selection.
  Document.prototype.createRange = function() {
    var range = new Range();
    setBothBP(range, this, 0, this, 0);
    return range;
  };
  Document.prototype.getSelection = function() { return getSelection(); };
  Document.prototype.adoptNode = function(node) {
    if (!node) return node;
    if (node.nodeType === 9) {
      throw new DOMException("Cannot adopt a document node.", "NotSupportedError");
    }
    var move = prepareNodeMove(this, node);
    move.newDoc = this;
    if (move.oldParent) {
      disconnectMovedRoots(move);
      moRemoveChild(move.oldParent.__ref, node.__ref);
    }
    if (move.oldDoc !== move.newDoc) {
      setOwnerDocumentSnapshot(move.snapshot, move.newDoc);
      for (var i = 0; i < move.roots.length; i++) {
        enqueueAdoptedTree(move.roots[i], move.oldDoc, move.newDoc);
      }
    }
    return node;
  };
  // Legacy event construction (DOM §dom-document-createevent). Every accepted
  // interface alias ("Event"/"Events"/"HTMLEvents"/"UIEvent"/"MouseEvent"/…)
  // yields a base Event with the **initialized flag unset** — dispatchEvent
  // throws InvalidStateError until initEvent() runs. An unrecognized interface
  // is a NotSupportedError. (We don't model per-interface event subclasses yet;
  // the base Event satisfies the harness's createEvent+initEvent pattern.)
  Document.prototype.createEvent = function(iface) {
    var name = String(iface).toLowerCase();
    var known = {
      'event': 1, 'events': 1, 'htmlevents': 1, 'svgevents': 1,
      'uievent': 1, 'uievents': 1, 'mouseevent': 1, 'mouseevents': 1,
      'keyboardevent': 1, 'customevent': 1, 'messageevent': 1, 'focusevent': 1,
      'compositionevent': 1, 'textevent': 1, 'dragevent': 1, 'hashchangeevent': 1,
      'storageevent': 1, 'beforeunloadevent': 1, 'devicemotionevent': 1,
      'deviceorientationevent': 1,
    };
    if (!known[name]) {
      throw new DOMException("createEvent: unsupported interface '" + iface + "'.", "NotSupportedError");
    }
    var e = new Event('');
    e.__initialized = false; // must call initEvent() before dispatch
    return e;
  };
  Document.prototype.getElementById = function(id) { return wrapNode(__getElementById(this.__ref, String(id))); };
  Document.prototype.getElementsByTagName = getElementsByTagName;
  Document.prototype.getElementsByClassName = getElementsByClassName;
  Document.prototype.querySelector = querySelector;
  Document.prototype.querySelectorAll = querySelectorAll;
  function XPathResult(resultType, value) {
    this.resultType = resultType;
    this.booleanValue = false;
    this.numberValue = 0;
    this.stringValue = '';
    this.singleNodeValue = null;
    this.invalidIteratorState = false;
    this._nodes = [];
    this._cursor = 0;
    this.snapshotLength = 0;

    if (value.kind === 'nodes') {
      this._nodes = xpathNodes(value);
      this.singleNodeValue = firstXPathNode(this._nodes);
      this.snapshotLength = this._nodes.length;
    }
    if (resultType === XPathResult.BOOLEAN_TYPE) {
      this.booleanValue = xpathBooleanValue(value);
    } else if (resultType === XPathResult.NUMBER_TYPE) {
      this.numberValue = xpathNumberValue(value);
    } else if (resultType === XPathResult.STRING_TYPE) {
      this.stringValue = xpathStringValue(value);
    }
  }
  XPathResult.ANY_TYPE = 0;
  XPathResult.NUMBER_TYPE = 1;
  XPathResult.STRING_TYPE = 2;
  XPathResult.BOOLEAN_TYPE = 3;
  XPathResult.UNORDERED_NODE_ITERATOR_TYPE = 4;
  XPathResult.ORDERED_NODE_ITERATOR_TYPE = 5;
  XPathResult.UNORDERED_NODE_SNAPSHOT_TYPE = 6;
  XPathResult.ORDERED_NODE_SNAPSHOT_TYPE = 7;
  XPathResult.ANY_UNORDERED_NODE_TYPE = 8;
  XPathResult.FIRST_ORDERED_NODE_TYPE = 9;
  for (var xk in XPathResult) {
    if (Object.prototype.hasOwnProperty.call(XPathResult, xk)) {
      XPathResult.prototype[xk] = XPathResult[xk];
    }
  }
  XPathResult.prototype.iterateNext = function() {
    if (this._cursor >= this._nodes.length) return null;
    return wrapNode(__reflectNode(this._nodes[this._cursor++]));
  };
  XPathResult.prototype.snapshotItem = function(i) {
    i = i >>> 0;
    return i < this._nodes.length ? wrapNode(__reflectNode(this._nodes[i])) : null;
  };
  globalThis.XPathResult = XPathResult;
  function xpathNodes(value) {
    return value.kind === 'nodes' && value.value ? value.value.split(',') : [];
  }
  function firstXPathNode(nodes) {
    return nodes.length ? wrapNode(__reflectNode(nodes[0])) : null;
  }
  function xpathStringValue(value) {
    if (value.kind === 'nodes') {
      var node = firstXPathNode(xpathNodes(value));
      return node ? node.textContent : '';
    }
    return String(value.value);
  }
  function xpathNumberValue(value) {
    if (value.kind === 'boolean') return value.value === 'true' ? 1 : 0;
    return Number(xpathStringValue(value));
  }
  function xpathBooleanValue(value) {
    if (value.kind === 'nodes') return xpathNodes(value).length > 0;
    if (value.kind === 'number') {
      var n = Number(value.value);
      return n !== 0 && n === n;
    }
    if (value.kind === 'string') return value.value.length > 0;
    return value.value === 'true';
  }
  function parseXPathRecord(record) {
    var s = String(record);
    var cut = s.indexOf('\n');
    var kind = cut < 0 ? s : s.slice(0, cut);
    var value = cut < 0 ? '' : s.slice(cut + 1);
    if (kind === 'error') {
      throw new DOMException(value, 'SyntaxError');
    }
    return { kind: kind, value: value };
  }
  Document.prototype.evaluate = function(expression, contextNode, namespaceResolver, resultType, result) {
    var context = contextNode || this;
    var requested = resultType == null ? XPathResult.ANY_TYPE : (resultType >>> 0);
    if (requested > XPathResult.FIRST_ORDERED_NODE_TYPE) {
      throw new DOMException('Unsupported XPathResult type', 'NotSupportedError');
    }
    var parsed = parseXPathRecord(__xpathEvaluate(String(expression), context.__ref));
    var actual = requested;
    if (requested === XPathResult.ANY_TYPE) {
      actual = parsed.kind === 'number' ? XPathResult.NUMBER_TYPE
        : parsed.kind === 'string' ? XPathResult.STRING_TYPE
        : parsed.kind === 'boolean' ? XPathResult.BOOLEAN_TYPE
        : XPathResult.ORDERED_NODE_ITERATOR_TYPE;
    }
    if (actual >= XPathResult.UNORDERED_NODE_ITERATOR_TYPE && parsed.kind !== 'nodes') {
      throw new DOMException('XPath expression did not return a node set', 'TypeError');
    }
    return new XPathResult(actual, parsed);
  };
  // Document takes the ParentNode mixin here, once its prototype exists.
  installMixin(Document.prototype, parentNodeMixin);
  // DocumentFragment is a query scope too (ParentNode mixin).
  DocumentFragment.prototype.querySelector = querySelector;
  DocumentFragment.prototype.querySelectorAll = querySelectorAll;
  DocumentFragment.prototype.getElementById = function(id) { return wrapNode(__getElementById(this.__ref, String(id))); };
  Object.defineProperty(Document.prototype, 'doctype', {
    configurable: true, get: function() { return wrapNode(__documentDoctype(this.__ref)); }
  });
  Object.defineProperty(Document.prototype, 'documentElement', {
    configurable: true, get: function() { return wrapNode(__documentElement(this.__ref)); }
  });
  Object.defineProperty(Document.prototype, 'body', {
    configurable: true,
    get: function() { return wrapNode(__documentBody(this.__ref)); },
    set: function(v) {
      var old = this.body;
      if (old) { old.parentNode.replaceChild(v, old); }
      else { var root = this.documentElement; if (root) root.appendChild(v); }
    }
  });
  Object.defineProperty(Document.prototype, 'head', {
    configurable: true, get: function() { return wrapNode(__documentHead(this.__ref)); }
  });
  // DOMImplementation: hasFeature (always true, per spec), plus createDocument /
  // createHTMLDocument / createDocumentType building fresh detached documents.
  Object.defineProperty(Document.prototype, 'implementation', {
    configurable: true,
    get: function() {
      var self = this;
      return {
        hasFeature: function() { return true; },
        createDocumentType: function(name, pub, sys) {
          name = String(name);
          validateQName(name);
          var d = wrapNode(__createDoctype(name, pub === undefined ? '' : String(pub),
                                           sys === undefined ? '' : String(sys)));
          ownerDocuments.set(d, self);
          return d;
        },
        createHTMLDocument: function(title) {
          var doc = wrapNode(__createDocument());
          var html = doc.createElement('html'); doc.appendChild(html);
          var head = doc.createElement('head'); html.appendChild(head);
          if (title !== undefined) { var t = doc.createElement('title'); t.textContent = String(title); head.appendChild(t); }
          html.appendChild(doc.createElement('body'));
          return doc;
        },
        createDocument: function(ns, qname, doctype) {
          var doc = wrapNode(__createDocument());
          doc.__isHtml = false; // an XML document: createCDATASection is allowed
          if (doctype) doc.appendChild(doctype);
          if (qname) { doc.appendChild(doc.createElementNS(ns === null ? '' : String(ns), String(qname))); }
          return doc;
        }
      };
    }
  });
  // Document IDL accessors (Lever 10): title walks to <title> (whitespace-collapsed);
  // dir reflects documentElement's dir; compatMode/readyState are constants.
  Object.defineProperty(Document.prototype, 'title', {
    configurable: true,
    get: function() {
      var titles = this.getElementsByTagName('title');
      if (!titles.length) return '';
      return (titles[0].textContent || '').replace(/[ \t\n\f\r]+/g, ' ').replace(/^ | $/g, '');
    },
    set: function(v) {
      var titles = this.getElementsByTagName('title');
      var t = titles.length ? titles[0] : null;
      if (!t) {
        var head = this.head; if (!head) return;
        t = this.createElement('title'); head.appendChild(t);
      }
      t.textContent = String(v);
    }
  });
  Object.defineProperty(Document.prototype, 'dir', {
    configurable: true,
    get: function() { var r = this.documentElement; return r ? r.dir : ''; },
    set: function(v) { var r = this.documentElement; if (r) r.dir = v; }
  });
  Object.defineProperty(Document.prototype, 'compatMode', {
    configurable: true, get: function() { return 'CSS1Compat'; }
  });
  Object.defineProperty(Document.prototype, 'readyState', {
    configurable: true, get: function() { return 'complete'; }
  });

  globalThis.Node = Node;
  globalThis.Element = Element;
  globalThis.Document = Document;
  // `htmlConstructor` is the IDL's [HTMLConstructor]. Without it (e.g.
  // HTMLUnknownElement) the interface object is never constructible.
  function makeHtmlInterfaceConstructor(name, htmlConstructor) {
    var Ctor = function() {
      if (htmlElementConstructionStack.length) {
        return htmlElementConstructionStack.pop();
      }
      var newTarget = new.target || Ctor;
      if (newTarget === Ctor || !htmlConstructor) throw new TypeError('Illegal constructor');
      return createElementForHtmlConstructor(Ctor, name, newTarget);
    };
    try { Object.defineProperty(Ctor, 'name', { configurable: true, value: name }); } catch (_) {}
    return Ctor;
  }

  function installHtmlInterfaceMembers(name, proto) {
    if (name === 'HTMLIFrameElement') {
      Object.defineProperty(proto, 'contentDocument', {
        configurable: true,
        get: function() {
          var child = iframeDocuments.get(this);
          if (!child) {
            child = document.implementation.createHTMLDocument('');
            iframeDocuments.set(this, child);
          }
          return child;
        }
      });
      Object.defineProperty(proto, 'contentWindow', {
        configurable: true,
        get: function() {
          var childWindow = iframeWindows.get(this);
          if (!childWindow) {
            var frame = this;
            childWindow = Object.create(globalThis.EventTarget && globalThis.EventTarget.prototype || Object.prototype);
            childWindow.document = frame.contentDocument;
            childWindow.getComputedStyle = function(el) { return makeComputedStyle(el, frame); };
            Object.defineProperty(childWindow, 'innerWidth', {
              configurable: true,
              get: function() { return parseFloat(makeComputedStyle(frame).width) || 300; }
            });
            Object.defineProperty(childWindow, 'innerHeight', {
              configurable: true,
              get: function() { return parseFloat(makeComputedStyle(frame).height) || 150; }
            });
            childWindow.window = childWindow;
            childWindow.self = childWindow;
            iframeWindows.set(frame, childWindow);
          }
          return childWindow;
        }
      });
      return;
    }
    if (name !== 'HTMLCanvasElement') return;
    proto.getContext = function(contextType) {
      var t = String(contextType || '');
      // `experimental-webgl` is the legacy alias retained by most browsers for
      // the WebGL 1.0 context; the conformance tests use both spellings.
      if (t !== 'webgl' && t !== 'experimental-webgl') return null;
      if (this.__webglContext) return this.__webglContext;
      var Ctor = globalThis.WebGLRenderingContext;
      if (typeof Ctor !== 'function') return null;
      var w = parseInt(this.getAttribute('width'), 10); if (!(w > 0)) w = 300;
      var h = parseInt(this.getAttribute('height'), 10); if (!(h > 0)) h = 150;
      this.__webglContext = new Ctor(w, h);
      // WebGL helpers use the standard back-reference for drawing-buffer
      // dimensions and context classification. Keep it on the context rather
      // than making the runtime API know about DOM wrapper identity.
      this.__webglContext.canvas = this;
      if (this.__webglContext._externalTextureKey) {
        this.setAttribute('data-genet-external-texture-key', this.__webglContext._externalTextureKey);
      }
      return this.__webglContext;
    };
  }

  function exposedOnWindow(def) {
    var e = def.exposed || [];
    // An interface with no [Exposed] in the source IDL is treated as exposed;
    // the generator only emits Window-side definitions.
    if (!e.length) return true;
    return e.indexOf('Window') !== -1;
  }

  function setClassString(proto, name) {
    try {
      Object.defineProperty(proto, Symbol.toStringTag, {
        configurable: true, value: name
      });
    } catch (_) {}
  }

  // [LegacyFactoryFunction]: `new Image(w, h)` and kin build the element the
  // interface's tag names select, then apply the documented argument mapping.
  function installNamedConstructor(alias, Ctor, tag) {
    var F = function(a, b, c, d) {
      var el = document.createElement(tag);
      if (alias === 'Image') {
        if (a !== undefined) el.setAttribute('width', String(a));
        if (b !== undefined) el.setAttribute('height', String(b));
      } else if (alias === 'Audio') {
        el.setAttribute('preload', 'auto');
        if (a !== undefined) { el.setAttribute('src', String(a)); }
      } else if (alias === 'Option') {
        if (a !== undefined && a !== '') el.textContent = String(a);
        if (b !== undefined) el.setAttribute('value', String(b));
        if (c) el.setAttribute('selected', '');
        el.selected = !!d;
      }
      return el;
    };
    F.prototype = Ctor.prototype;
    try { Object.defineProperty(F, 'name', { configurable: true, value: alias }); } catch (_) {}
    globalThis[alias] = F;
  }

  function installHtmlInterfaceTable() {
    var table = globalThis.__genetHtmlInterfaceTable || [];
    for (var i = 0; i < table.length; i++) {
      var def = table[i];
      if (!exposedOnWindow(def)) continue;
      htmlInterfaceDefinitions[def.name] = def;
      var parent = htmlInterfaceConstructors[def.parent] || globalThis[def.parent];
      if (typeof parent !== 'function') {
        throw new Error('Unknown HTML interface parent: ' + def.parent);
      }
      var Ctor = makeHtmlInterfaceConstructor(def.name, def.htmlConstructor !== false);
      Ctor.prototype = Object.create(parent.prototype);
      Object.defineProperty(Ctor.prototype, 'constructor', {
        configurable: true,
        writable: true,
        value: Ctor
      });
      setClassString(Ctor.prototype, def.name);
      globalThis[def.name] = Ctor;
      htmlInterfaceConstructors[def.name] = Ctor;
      installReflectedAttributes(Ctor.prototype, def.reflected || []);
      installHtmlInterfaceMembers(def.name, Ctor.prototype);

      var tags = def.tags || [];
      for (var j = 0; j < tags.length; j++) {
        elementSubclassProto[String(tags[j]).toUpperCase()] = Ctor.prototype;
      }
      var named = def.namedConstructors || [];
      for (var n = 0; n < named.length; n++) {
        if (tags.length) installNamedConstructor(named[n], Ctor, tags[0]);
      }
    }
    try { delete globalThis.__genetHtmlInterfaceTable; } catch (_) {}
  }

  // DOM / CSSOM interfaces the generator read from dom.idl and cssom.idl. Only
  // the shape is installed, and only where the bootstrap has not already
  // defined the name: an interface object with the right prototype chain and
  // class string, no members. Never replaces a working implementation.
  function installShapeInterfaces() {
    var table = globalThis.__genetShapeInterfaceTable || [];
    for (var i = 0; i < table.length; i++) {
      var def = table[i];
      if (!exposedOnWindow(def)) continue;
      var existing = globalThis[def.name];
      if (typeof existing === 'function') {
        if (existing.prototype) setClassString(existing.prototype, def.name);
        continue;
      }
      var parent = def.parent ? globalThis[def.parent] : null;
      var Ctor = (function(name, constructible) {
        return function() {
          if (!constructible) throw new TypeError('Illegal constructor');
          throw new TypeError(name + ' is not implemented');
        };
      })(def.name, def.constructible);
      Ctor.prototype = typeof parent === 'function'
        ? Object.create(parent.prototype)
        : Object.create(Object.prototype);
      Object.defineProperty(Ctor.prototype, 'constructor', {
        configurable: true, writable: true, value: Ctor
      });
      setClassString(Ctor.prototype, def.name);
      try { Object.defineProperty(Ctor, 'name', { configurable: true, value: def.name }); } catch (_) {}
      globalThis[def.name] = Ctor;
    }
    try { delete globalThis.__genetShapeInterfaceTable; } catch (_) {}
  }
  // (Text / Comment / CharacterData exposed above, with their prototype chain.)


  // ── MutationObserver ───────────────────────────────────────────────────────
  //
  // The second consumer of the arena's mutation point. The registry, the option
  // validation, the interested-observer walk and the "notify mutation
  // observers" microtask live here; the arena records only what JS cannot
  // re-derive after the fact — the siblings either side of a removal, an
  // attribute's or a character-data node's old value, and the target's ancestor
  // chain at mutation time — and `__moTake` drains that record. Livery's
  // `DomMutation` stream is never touched by any of this.
  var moRegisteredIds = Object.create(null);   // raw node id -> registration count
  var moRegistrationCount = 0;
  var moActive = false;
  var moNotifySet = [];
  var moNotifyQueued = false;

  function moSyncActive() {
    var want = moRegistrationCount > 0;
    if (want === moActive) return;
    moActive = want;
    __moObserving(want ? '1' : '0');
  }

  function moKey(node) { return __nodeRawId(node.__ref); }

  function moAddRegistration(node, reg) {
    if (!node.__moRegs) node.__moRegs = [];
    node.__moRegs.push(reg);
    var key = moKey(node);
    moRegisteredIds[key] = (moRegisteredIds[key] || 0) + 1;
    moRegistrationCount++;
    moSyncActive();
    if (reg.observer.__nodes.indexOf(node) < 0) reg.observer.__nodes.push(node);
  }

  // Drop this node's registrations matching `pred`, keeping the id census and
  // the active switch in step.
  function moRemoveRegs(node, pred) {
    var regs = node.__moRegs;
    if (!regs || !regs.length) return;
    var kept = [];
    for (var i = 0; i < regs.length; i++) {
      if (pred(regs[i])) {
        var key = moKey(node);
        moRegisteredIds[key]--;
        if (!moRegisteredIds[key]) delete moRegisteredIds[key];
        moRegistrationCount--;
      } else {
        kept.push(regs[i]);
      }
    }
    node.__moRegs = kept;
    moSyncActive();
  }

  function moUnescape(v) {
    if (!v || v.indexOf('\\') < 0) return v;
    var out = '';
    for (var i = 0; i < v.length; i++) {
      var c = v.charAt(i);
      if (c !== '\\') { out += c; continue; }
      var n = v.charAt(++i);
      out += n === 'n' ? '\n' : n === 'r' ? '\r' : n === 't' ? '\t' : n === '\\' ? '\\' : n;
    }
    return out;
  }

  function moNodeById(id) { return id ? wrapNode(__reflectNode(id)) : null; }
  function moNodesById(csv) {
    var out = [];
    if (!csv) return out;
    var parts = csv.split(',');
    for (var i = 0; i < parts.length; i++) {
      var n = moNodeById(parts[i]);
      if (n) out.push(n);
    }
    return out;
  }

  function MutationRecord() { throw new TypeError('Illegal constructor'); }
  MutationRecord.prototype = Object.create(Object.prototype);
  Object.defineProperty(MutationRecord.prototype, 'constructor', {
    configurable: true, writable: true, value: MutationRecord
  });
  globalThis.MutationRecord = MutationRecord;

  function moMakeRecord(type, target, fields) {
    var r = Object.create(MutationRecord.prototype);
    var added = fields.added || [];
    var removed = fields.removed || [];
    r.type = type;
    r.target = target;
    r.addedNodes = makeCollection(function() { return added; }, false);
    r.removedNodes = makeCollection(function() { return removed; }, false);
    r.previousSibling = fields.previousSibling || null;
    r.nextSibling = fields.nextSibling || null;
    r.attributeName = fields.attributeName === undefined ? null : fields.attributeName;
    r.attributeNamespace = fields.attributeNamespace === undefined ? null : fields.attributeNamespace;
    r.oldValue = fields.oldValue === undefined ? null : fields.oldValue;
    return r;
  }

  // DOM "queue a mutation record", walking the target's inclusive ancestors as
  // they stood when the mutation happened.
  function moQueueRecord(type, targetId, chainCsv, fields, oldValue) {
    var chain = chainCsv ? chainCsv.split(',') : [];
    var interested = [];
    for (var i = 0; i < chain.length; i++) {
      var id = chain[i];
      if (!moRegisteredIds[id]) continue;
      var node = moNodeById(id);
      if (!node || !node.__moRegs) continue;
      var regs = node.__moRegs.slice();
      for (var j = 0; j < regs.length; j++) {
        var o = regs[j].options;
        if (id !== targetId && !o.subtree) continue;
        if (type === 'attributes' && !o.attributes) continue;
        if (type === 'attributes' && o.attributeFilter &&
            (fields.attributeNamespace !== null ||
             o.attributeFilter.indexOf(fields.attributeName) < 0)) continue;
        if (type === 'characterData' && !o.characterData) continue;
        if (type === 'childList' && !o.childList) continue;
        var wants = (type === 'attributes' && o.attributeOldValue) ||
                    (type === 'characterData' && o.characterDataOldValue);
        var slot = null;
        for (var k = 0; k < interested.length; k++) {
          if (interested[k][0] === regs[j].observer) { slot = interested[k]; break; }
        }
        if (!slot) { slot = [regs[j].observer, false]; interested.push(slot); }
        if (wants) slot[1] = true;
      }
    }
    if (!interested.length) return;
    var target = moNodeById(targetId);
    if (!target) return;
    for (var m = 0; m < interested.length; m++) {
      fields.oldValue = interested[m][1] ? oldValue : null;
      moEnqueue(interested[m][0], moMakeRecord(type, target, fields));
    }
  }

  // A removed subtree keeps its ancestors' subtree observers until the next
  // delivery ("transient registered observer").
  function moAddTransient(removed, chainCsv) {
    if (!removed.length) return;
    var chain = chainCsv ? chainCsv.split(',') : [];
    var sources = [];
    for (var i = 0; i < chain.length; i++) {
      if (!moRegisteredIds[chain[i]]) continue;
      var n = moNodeById(chain[i]);
      if (!n || !n.__moRegs) continue;
      for (var j = 0; j < n.__moRegs.length; j++) {
        if (n.__moRegs[j].options.subtree) sources.push(n.__moRegs[j]);
      }
    }
    if (!sources.length) return;
    for (var r = 0; r < removed.length; r++) {
      var node = removed[r];
      for (var s = 0; s < sources.length; s++) {
        var dup = false;
        var have = node.__moRegs || [];
        for (var h = 0; h < have.length; h++) {
          if (have[h].source === sources[s]) { dup = true; break; }
        }
        if (dup) continue;
        moAddRegistration(node, {
          observer: sources[s].observer,
          options: sources[s].options,
          transient: true,
          source: sources[s]
        });
      }
    }
  }

  // Drain the arena's record and attribute it to the observers registered now.
  // Called at the head of each microtask checkpoint and at every entry point
  // that can change the registry (`observe`, `takeRecords`, `disconnect`), so
  // "now" is always the registry as it stood when the mutation happened.
  function moFlush() {
    if (!moActive) return;
    var blob = __moTake();
    if (!blob) return;
    var lines = blob.split('\n');
    for (var i = 0; i < lines.length; i++) {
      var f = lines[i].split('\t');
      if (f[0] === 'C') {
        var removed = moNodesById(f[5]);
        moQueueRecord('childList', f[1], f[6], {
          added: moNodesById(f[4]),
          removed: removed,
          previousSibling: moNodeById(f[2]),
          nextSibling: moNodeById(f[3])
        }, null);
        moAddTransient(removed, f[6]);
      } else if (f[0] === 'A') {
        moQueueRecord('attributes', f[1], f[6], {
          attributeName: moUnescape(f[2]),
          attributeNamespace: f[3] ? moUnescape(f[3]) : null
        }, f[4] === '1' ? moUnescape(f[5]) : null);
      } else if (f[0] === 'D') {
        moQueueRecord('characterData', f[1], f[4], {},
          f[2] === '1' ? moUnescape(f[3]) : null);
      }
    }
  }

  function moEnqueue(observer, record) {
    observer.__queue.push(record);
    if (moNotifySet.indexOf(observer) < 0) moNotifySet.push(observer);
    if (moNotifyQueued) return;
    moNotifyQueued = true;
    Promise.resolve().then(moNotify);
  }

  // HTML "notify mutation observers": one compound microtask, observers in the
  // order their first record was queued.
  function moNotify() {
    moNotifyQueued = false;
    moFlush();
    var list = moNotifySet;
    moNotifySet = [];
    for (var i = 0; i < list.length; i++) {
      var ob = list[i];
      var records = ob.__queue;
      ob.__queue = [];
      moDropTransients(ob, null);
      if (!records.length) continue;
      try {
        ob.__cb.call(ob, records, ob);
      } catch (e) {
        if (typeof globalThis.reportError === 'function') globalThis.reportError(e);
      }
    }
    // A callback that mutated the DOM queues its own delivery, still inside
    // this checkpoint's microtask drain.
    moFlush();
  }

  // Drop this observer's transient registrations — all of them, or only those
  // sourced from one registration when `source` is given.
  function moDropTransients(observer, source) {
    var nodes = observer.__nodes.slice();
    for (var i = 0; i < nodes.length; i++) {
      moRemoveRegs(nodes[i], function(reg) {
        return reg.transient && reg.observer === observer &&
               (source === null || reg.source === source);
      });
    }
    moPruneNodes(observer);
  }

  function moPruneNodes(observer) {
    var kept = [];
    for (var i = 0; i < observer.__nodes.length; i++) {
      var node = observer.__nodes[i];
      var regs = node.__moRegs || [];
      for (var j = 0; j < regs.length; j++) {
        if (regs[j].observer === observer) { kept.push(node); break; }
      }
    }
    observer.__nodes = kept;
  }

  function MutationObserver(callback) {
    if (!(this instanceof MutationObserver)) {
      throw new TypeError("Constructor MutationObserver requires 'new'");
    }
    if (typeof callback !== 'function') {
      throw new TypeError('MutationObserver: callback is not a function');
    }
    this.__cb = callback;
    this.__queue = [];
    this.__nodes = [];
  }

  MutationObserver.prototype.observe = function(target, options) {
    if (!target || !target.__ref) {
      throw new TypeError('MutationObserver.observe: target is not a Node');
    }
    var src = options === undefined || options === null ? {} : options;
    var o = {
      childList: !!src.childList,
      subtree: !!src.subtree,
      attributes: src.attributes,
      characterData: src.characterData,
      attributeOldValue: !!src.attributeOldValue,
      characterDataOldValue: !!src.characterDataOldValue,
      attributeFilter: src.attributeFilter
    };
    // Spec defaulting: an attribute/character-data qualifier implies its flag
    // unless the caller stated the flag itself.
    if (('attributeOldValue' in src || 'attributeFilter' in src) && !('attributes' in src)) {
      o.attributes = true;
    }
    if ('characterDataOldValue' in src && !('characterData' in src)) o.characterData = true;
    o.attributes = !!o.attributes;
    o.characterData = !!o.characterData;
    if (o.attributeFilter === undefined || o.attributeFilter === null) {
      o.attributeFilter = null;
    } else {
      var filter = [];
      for (var i = 0; i < o.attributeFilter.length; i++) filter.push(String(o.attributeFilter[i]));
      o.attributeFilter = filter;
    }
    if (o.attributeOldValue && !o.attributes) {
      throw new TypeError('MutationObserver.observe: attributeOldValue without attributes');
    }
    if (o.attributeFilter && !o.attributes) {
      throw new TypeError('MutationObserver.observe: attributeFilter without attributes');
    }
    if (o.characterDataOldValue && !o.characterData) {
      throw new TypeError('MutationObserver.observe: characterDataOldValue without characterData');
    }
    if (!o.childList && !o.attributes && !o.characterData) {
      throw new TypeError('MutationObserver.observe: childList, attributes or characterData is required');
    }
    // Mutations from before this registration are not ours.
    moFlush();
    var regs = target.__moRegs || [];
    var existing = null;
    for (var r = 0; r < regs.length; r++) {
      if (regs[r].observer === this && !regs[r].transient) { existing = regs[r]; break; }
    }
    if (existing) {
      moDropTransients(this, existing);
      existing.options = o;
      if (this.__nodes.indexOf(target) < 0) this.__nodes.push(target);
    } else {
      moAddRegistration(target, { observer: this, options: o, transient: false, source: null });
    }
  };

  MutationObserver.prototype.disconnect = function() {
    moFlush();
    var self = this;
    var nodes = this.__nodes.slice();
    for (var i = 0; i < nodes.length; i++) {
      moRemoveRegs(nodes[i], function(reg) { return reg.observer === self; });
    }
    this.__nodes = [];
    this.__queue = [];
  };

  MutationObserver.prototype.takeRecords = function() {
    moFlush();
    var records = this.__queue;
    this.__queue = [];
    return records;
  };

  setClassString(MutationObserver.prototype, 'MutationObserver');
  setClassString(MutationRecord.prototype, 'MutationRecord');
  globalThis.MutationObserver = MutationObserver;

  // Every DOM mutation the bootstrap performs goes through one of these, so
  // this is where the arena's record is attributed: at mutation time, which is
  // when the spec queues a mutation record and schedules the notify microtask.
  // Neither backend allows interposing on the natives themselves (Nova's
  // globals are non-writable), so the funnel is these twelve call sites.
  var moGroupDepth = 0;
  function moAfterMutation() { if (moActive && !moGroupDepth) moFlush(); }
  function moBeginGroup() { if (moGroupDepth++ === 0) __moGroup('1'); }
  function moEndGroup() {
    if (--moGroupDepth === 0) { __moGroup('0'); moAfterMutation(); }
  }
  // The same call sites carry the DOM's live-range steps: `rangeWillRemove`
  // before the tree moves (it needs the child index and the pre-mutation
  // boundaries), `rangeDidInsert` after (the index the node lands at is the
  // reference child's old index).
  function moAppendChild(p, c) {
    rangeWillRemove(wrapNode(c)); __appendChild(p, c);
    rangeDidInsert(wrapNode(c)); moAfterMutation();
  }
  function moInsertBefore(p, n, r) {
    rangeWillRemove(wrapNode(n)); __insertBefore(p, n, r);
    rangeDidInsert(wrapNode(n)); moAfterMutation();
  }
  function moMoveBefore(p, n, r) {
    rangeWillRemove(wrapNode(n)); __moveBefore(p, n, r);
    rangeDidInsert(wrapNode(n)); moAfterMutation();
  }
  function moRemoveChild(p, c) {
    rangeWillRemove(wrapNode(c)); __removeChild(p, c); moAfterMutation();
  }
  function moSetAttribute(e, n, v) { __setAttribute(e, n, v); moAfterMutation(); }
  function moRemoveAttribute(e, n) { __removeAttribute(e, n); moAfterMutation(); }
  function moSetAttributeNS(e, ns, q, v) { __setAttributeNS(e, ns, q, v); moAfterMutation(); }
  function moRemoveAttributeNS(e, ns, l) { __removeAttributeNS(e, ns, l); moAfterMutation(); }
  function moSetTextContent(n, t) {
    rangeWillReplaceAll(wrapNode(n)); __setTextContent(n, t); moAfterMutation();
  }
  function moSetInnerHtml(n, h) {
    rangeWillReplaceAll(wrapNode(n)); __setInnerHtml(n, h); moAfterMutation();
  }

  // The runtime calls this at the head of every microtask checkpoint while any
  // observer is registered, which is what turns the arena's pending record into
  // queued records and schedules the notify microtask the checkpoint then runs.
  globalThis.__moPump = function() { moFlush(); };

  // ── Range, StaticRange and Selection ───────────────────────────────────────
  //
  // DOM §5 boundary points over the scripted arena, plus the Selection API's
  // single live range. Every live range registers here, and the bootstrap's own
  // mutation funnel (the `mo*` wrappers above) runs the spec's removing,
  // inserting, replace-data and split steps against the registry. The arena's
  // observer record is deliberately not the driver: those steps need the child
  // index and the boundary offsets as they stood *before* the mutation, which
  // only the call site still holds.
  //
  // A registration is a WeakRef where the backend has one, so a test that mints
  // thousands of ranges does not make every later mutation linear in all of
  // them for the realm's life; `eachLiveRange` compacts as it walks.
  // Boundaries are indexed by the node they sit in, not held in one flat list:
  // an editing test mints thousands of ranges, and a per-mutation walk over all
  // of them is quadratic (the first measurement of this lane hung eight
  // `editing/run/*` files that had merely failed before). Each boundary owns one
  // index entry `[ref, which, resolved]` — `which` 0 for the start, 1 for the
  // end — and the range keeps a handle to its own entries, so retiring one is
  // O(1). A removal still walks the removed subtree, but that cost is the
  // tree's, not the range population's.
  var rangeIndex = Object.create(null);
  var liveRangeCount = 0;
  var haveWeakRef = typeof WeakRef === 'function';

  function indexAdd(node, range, which) {
    var key = __nodeRawId(node.__ref);
    var bucket = rangeIndex[key] || (rangeIndex[key] = []);
    var entry = [haveWeakRef ? new WeakRef(range) : range, which, null];
    bucket.push(entry);
    if (which === 0) range.__se = entry; else range.__ee = entry;
  }

  function setStartBP(range, node, offset) {
    if (range.__sc !== node) {
      if (range.__se) range.__se[0] = null;
      range.__sc = node;
      indexAdd(node, range, 0);
    }
    range.__so = offset;
  }
  function setEndBP(range, node, offset) {
    if (range.__ec !== node) {
      if (range.__ee) range.__ee[0] = null;
      range.__ec = node;
      indexAdd(node, range, 1);
    }
    range.__eo = offset;
  }
  function setBothBP(range, sc, so, ec, eo) {
    setStartBP(range, sc, so);
    setEndBP(range, ec, eo);
  }

  // The live boundaries in `node`, with dead entries swept out. Entry slot 2
  // carries the resolved range so the caller does not deref twice.
  function boundariesAt(node) {
    if (!liveRangeCount) return null;
    var key = __nodeRawId(node.__ref);
    var bucket = rangeIndex[key];
    if (!bucket) return null;
    var live = [];
    for (var i = 0; i < bucket.length; i++) {
      var entry = bucket[i];
      if (!entry[0]) continue;
      var range = haveWeakRef ? entry[0].deref() : entry[0];
      if (!range) { entry[0] = null; continue; }
      entry[2] = range;
      live.push(entry);
    }
    if (!live.length) { delete rangeIndex[key]; return null; }
    rangeIndex[key] = live.slice();
    return live;
  }

  // DOM "length of a node".
  function nodeLengthOf(node) {
    var t = node.nodeType;
    if (t === 10) return 0;
    if (t === 3 || t === 8 || t === 7) return node.data.length;
    return +__childNodesCount(node.__ref);
  }

  function childAt(node, index) {
    var ref = __childNodesItem(node.__ref, index);
    return (ref === null || ref === undefined) ? null : wrapNode(ref);
  }

  function nodeIndex(node) {
    var i = 0, n = node.previousSibling;
    while (n) { i++; n = n.previousSibling; }
    return i;
  }

  function rootOf(node) { return node.getRootNode(); }

  // DOM "position of boundary point A relative to boundary point B":
  // -1 before, 0 equal, 1 after. Callers compare roots first; a disconnected
  // pair reports 0 rather than recursing.
  function bpPosition(an, ao, bn, bo) {
    if (an === bn) return ao < bo ? -1 : ao > bo ? 1 : 0;
    var pos = an.compareDocumentPosition(bn);
    if (pos & 1) return 0;
    if (pos & 2) return -bpPosition(bn, bo, an, ao);
    if (pos & 16) {
      var child = bn;
      while (child.parentNode !== an) child = child.parentNode;
      return nodeIndex(child) < ao ? 1 : -1;
    }
    return -1;
  }

  // ---- live range updating, run from the bootstrap's mutation funnel --------

  // Move every boundary inside `root`'s subtree onto (parent, index).
  function moveBoundariesUnder(root, parent, index) {
    var stack = [root];
    while (stack.length) {
      var node = stack.pop();
      var live = boundariesAt(node);
      if (live) {
        for (var i = 0; i < live.length; i++) {
          var range = live[i][2];
          if (live[i][1] === 0) { if (range.__sc === node) setStartBP(range, parent, index); }
          else if (range.__ec === node) setEndBP(range, parent, index);
        }
      }
      var count = +__childNodesCount(node.__ref);
      for (var k = 0; k < count; k++) stack.push(childAt(node, k));
    }
  }

  // DOM "removing steps", run before `node` leaves its parent.
  function rangeWillRemove(node) {
    if (!liveRangeCount || !node) return;
    var parent = node.parentNode;
    if (!parent) return;
    var index = nodeIndex(node);
    moveBoundariesUnder(node, parent, index);
    var live = boundariesAt(parent);
    if (!live) return;
    for (var i = 0; i < live.length; i++) {
      var range = live[i][2];
      if (live[i][1] === 0) { if (range.__sc === parent && range.__so > index) range.__so--; }
      else if (range.__ec === parent && range.__eo > index) range.__eo--;
    }
  }

  // DOM "insertion steps", run after `node` has taken its place. The index the
  // node now occupies is the reference child's index before the insertion,
  // which is what the spec compares offsets against.
  function rangeDidInsert(node) {
    if (!liveRangeCount || !node) return;
    var parent = node.parentNode;
    if (!parent) return;
    var live = boundariesAt(parent);
    if (!live) return;
    var index = nodeIndex(node);
    for (var i = 0; i < live.length; i++) {
      var range = live[i][2];
      if (live[i][1] === 0) { if (range.__sc === parent && range.__so > index) range.__so++; }
      else if (range.__ec === parent && range.__eo > index) range.__eo++;
    }
  }

  // DOM "replace all", which removes every child in tree order. Sequentially
  // that leaves any boundary inside the parent at offset 0, and any boundary
  // inside a removed subtree at (parent, 0).
  function rangeWillReplaceAll(parent) {
    if (!liveRangeCount || !parent) return;
    var count = +__childNodesCount(parent.__ref);
    if (!count) return;
    for (var i = 0; i < count; i++) moveBoundariesUnder(childAt(parent, i), parent, 0);
    var live = boundariesAt(parent);
    if (!live) return;
    for (var j = 0; j < live.length; j++) {
      var range = live[j][2];
      if (live[j][1] === 0) { if (range.__sc === parent) range.__so = 0; }
      else if (range.__ec === parent) range.__eo = 0;
    }
  }

  // CharacterData "replace data" (DOM §4.10) with its live-range steps. Every
  // character-data mutator funnels here so the range steps see a real
  // (offset, count, data) rather than a whole new string.
  function cdReplaceData(node, offset, count, data) {
    var current = node.data;
    var length = current.length;
    offset = offset >>> 0;
    if (offset > length) {
      throw new DOMException("offset out of range", "IndexSizeError");
    }
    count = count >>> 0;
    if (offset + count > length) count = length - offset;
    moSetTextContent(node.__ref, current.slice(0, offset) + data + current.slice(offset + count));
    var live = boundariesAt(node);
    if (!live) return;
    var added = data.length;
    for (var i = 0; i < live.length; i++) {
      var range = live[i][2];
      if (live[i][1] === 0) {
        if (range.__sc !== node) continue;
        if (range.__so > offset && range.__so <= offset + count) range.__so = offset;
        else if (range.__so > offset + count) range.__so += added - count;
      } else {
        if (range.__ec !== node) continue;
        if (range.__eo > offset && range.__eo <= offset + count) range.__eo = offset;
        else if (range.__eo > offset + count) range.__eo += added - count;
      }
    }
  }

  // DOM "split a Text node" range steps, run after the new node is in place and
  // before the original's data is truncated.
  function rangeDidSplit(node, newNode, offset) {
    if (!liveRangeCount) return;
    var live = boundariesAt(node);
    var i, range;
    if (live) {
      for (i = 0; i < live.length; i++) {
        range = live[i][2];
        if (live[i][1] === 0) {
          if (range.__sc === node && range.__so > offset) setStartBP(range, newNode, range.__so - offset);
        } else if (range.__ec === node && range.__eo > offset) {
          setEndBP(range, newNode, range.__eo - offset);
        }
      }
    }
    var parent = node.parentNode;
    if (!parent) return;
    var after = nodeIndex(node) + 1;
    live = boundariesAt(parent);
    if (!live) return;
    for (i = 0; i < live.length; i++) {
      range = live[i][2];
      if (live[i][1] === 0) {
        if (range.__sc === parent && range.__so === after) setStartBP(range, newNode, 0);
      } else if (range.__ec === parent && range.__eo === after) {
        setEndBP(range, newNode, 0);
      }
    }
  }

  // ---- AbstractRange / StaticRange ------------------------------------------

  function AbstractRange() { throw new TypeError('Illegal constructor'); }
  AbstractRange.prototype = Object.create(Object.prototype);
  Object.defineProperty(AbstractRange.prototype, 'constructor', {
    configurable: true, writable: true, value: AbstractRange
  });
  function defineBoundaryAccessors(proto) {
    Object.defineProperty(proto, 'startContainer', {
      configurable: true, get: function() { return this.__sc; }
    });
    Object.defineProperty(proto, 'startOffset', {
      configurable: true, get: function() { return this.__so; }
    });
    Object.defineProperty(proto, 'endContainer', {
      configurable: true, get: function() { return this.__ec; }
    });
    Object.defineProperty(proto, 'endOffset', {
      configurable: true, get: function() { return this.__eo; }
    });
    Object.defineProperty(proto, 'collapsed', {
      configurable: true,
      get: function() { return this.__sc === this.__ec && this.__so === this.__eo; }
    });
  }
  defineBoundaryAccessors(AbstractRange.prototype);
  setClassString(AbstractRange.prototype, 'AbstractRange');
  globalThis.AbstractRange = AbstractRange;

  function StaticRange(init) {
    if (!(this instanceof StaticRange)) {
      throw new TypeError("Constructor StaticRange requires 'new'");
    }
    if (init === null || typeof init !== 'object') {
      throw new TypeError('StaticRange: init is not a StaticRangeInit');
    }
    var sc = init.startContainer, ec = init.endContainer;
    if (!sc || !sc.__ref || !ec || !ec.__ref) {
      throw new TypeError('StaticRange: containers must be Nodes');
    }
    if (sc.nodeType === 10 || ec.nodeType === 10 || sc.__isAttr || ec.__isAttr) {
      throw new DOMException('DocumentType or Attr boundary', 'InvalidNodeTypeError');
    }
    this.__sc = sc;
    this.__so = init.startOffset >>> 0;
    this.__ec = ec;
    this.__eo = init.endOffset >>> 0;
  }
  StaticRange.prototype = Object.create(AbstractRange.prototype);
  Object.defineProperty(StaticRange.prototype, 'constructor', {
    configurable: true, writable: true, value: StaticRange
  });
  defineBoundaryAccessors(StaticRange.prototype);
  setClassString(StaticRange.prototype, 'StaticRange');
  globalThis.StaticRange = StaticRange;

  // ---- Range ----------------------------------------------------------------

  function Range() {
    if (!(this instanceof Range)) {
      throw new TypeError("Constructor Range requires 'new'");
    }
    this.__sc = null; this.__so = 0; this.__ec = null; this.__eo = 0;
    this.__se = null; this.__ee = null;
    liveRangeCount++;
    setBothBP(this, document, 0, document, 0);
  }
  Range.prototype = Object.create(AbstractRange.prototype);
  Object.defineProperty(Range.prototype, 'constructor', {
    configurable: true, writable: true, value: Range
  });
  defineBoundaryAccessors(Range.prototype);
  Range.START_TO_START = Range.prototype.START_TO_START = 0;
  Range.START_TO_END = Range.prototype.START_TO_END = 1;
  Range.END_TO_END = Range.prototype.END_TO_END = 2;
  Range.END_TO_START = Range.prototype.END_TO_START = 3;

  function requireNode(node, what) {
    if (!node || !node.__ref) {
      throw new TypeError(what + ' is not a Node');
    }
    return node;
  }

  function checkBoundary(node, offset) {
    if (node.nodeType === 10) {
      throw new DOMException('boundary node is a DocumentType', 'InvalidNodeTypeError');
    }
    if (offset > nodeLengthOf(node)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
  }

  // DOM "set the start" / "set the end": the other boundary collapses onto the
  // new one when they end up in different trees or out of order.
  function rangeSetStart(range, node, offset) {
    checkBoundary(node, offset);
    if (rootOf(node) !== rootOf(range.__ec) || bpPosition(node, offset, range.__ec, range.__eo) === 1) {
      setEndBP(range, node, offset);
    }
    setStartBP(range, node, offset);
  }
  function rangeSetEnd(range, node, offset) {
    checkBoundary(node, offset);
    if (rootOf(node) !== rootOf(range.__sc) || bpPosition(node, offset, range.__sc, range.__so) === -1) {
      setStartBP(range, node, offset);
    }
    setEndBP(range, node, offset);
  }

  Range.prototype.setStart = function(node, offset) {
    rangeSetStart(this, requireNode(node, 'setStart: node'), offset >>> 0);
    selectionMayHaveChanged(this);
  };
  Range.prototype.setEnd = function(node, offset) {
    rangeSetEnd(this, requireNode(node, 'setEnd: node'), offset >>> 0);
    selectionMayHaveChanged(this);
  };
  function siblingBoundary(node, what) {
    requireNode(node, what + ': node');
    var parent = node.parentNode;
    if (!parent) throw new DOMException('node has no parent', 'InvalidNodeTypeError');
    return parent;
  }
  Range.prototype.setStartBefore = function(node) {
    var parent = siblingBoundary(node, 'setStartBefore');
    rangeSetStart(this, parent, nodeIndex(node));
    selectionMayHaveChanged(this);
  };
  Range.prototype.setStartAfter = function(node) {
    var parent = siblingBoundary(node, 'setStartAfter');
    rangeSetStart(this, parent, nodeIndex(node) + 1);
    selectionMayHaveChanged(this);
  };
  Range.prototype.setEndBefore = function(node) {
    var parent = siblingBoundary(node, 'setEndBefore');
    rangeSetEnd(this, parent, nodeIndex(node));
    selectionMayHaveChanged(this);
  };
  Range.prototype.setEndAfter = function(node) {
    var parent = siblingBoundary(node, 'setEndAfter');
    rangeSetEnd(this, parent, nodeIndex(node) + 1);
    selectionMayHaveChanged(this);
  };
  Range.prototype.collapse = function(toStart) {
    if (toStart) setEndBP(this, this.__sc, this.__so);
    else setStartBP(this, this.__ec, this.__eo);
    selectionMayHaveChanged(this);
  };
  Range.prototype.selectNode = function(node) {
    var parent = siblingBoundary(node, 'selectNode');
    var index = nodeIndex(node);
    setBothBP(this, parent, index, parent, index + 1);
    selectionMayHaveChanged(this);
  };
  Range.prototype.selectNodeContents = function(node) {
    requireNode(node, 'selectNodeContents: node');
    if (node.nodeType === 10) {
      throw new DOMException('node is a DocumentType', 'InvalidNodeTypeError');
    }
    setBothBP(this, node, 0, node, nodeLengthOf(node));
    selectionMayHaveChanged(this);
  };
  Range.prototype.compareBoundaryPoints = function(how, sourceRange) {
    how = how >>> 0;
    if (how > 3) {
      throw new DOMException('invalid comparison method', 'NotSupportedError');
    }
    if (!sourceRange || sourceRange.__sc === undefined) {
      throw new TypeError('compareBoundaryPoints: sourceRange is not a Range');
    }
    if (rootOf(this.__sc) !== rootOf(sourceRange.__sc)) {
      throw new DOMException('ranges are in different trees', 'WrongDocumentError');
    }
    var thisNode, thisOffset, otherNode, otherOffset;
    if (how === 0) { thisNode = this.__sc; thisOffset = this.__so; otherNode = sourceRange.__sc; otherOffset = sourceRange.__so; }
    else if (how === 1) { thisNode = this.__ec; thisOffset = this.__eo; otherNode = sourceRange.__sc; otherOffset = sourceRange.__so; }
    else if (how === 2) { thisNode = this.__ec; thisOffset = this.__eo; otherNode = sourceRange.__ec; otherOffset = sourceRange.__eo; }
    else { thisNode = this.__sc; thisOffset = this.__so; otherNode = sourceRange.__ec; otherOffset = sourceRange.__eo; }
    return bpPosition(thisNode, thisOffset, otherNode, otherOffset);
  };
  Range.prototype.comparePoint = function(node, offset) {
    requireNode(node, 'comparePoint: node');
    offset = offset >>> 0;
    if (rootOf(node) !== rootOf(this.__sc)) {
      throw new DOMException('node is in a different tree', 'WrongDocumentError');
    }
    if (node.nodeType === 10) {
      throw new DOMException('node is a DocumentType', 'InvalidNodeTypeError');
    }
    if (offset > nodeLengthOf(node)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
    if (bpPosition(node, offset, this.__sc, this.__so) === -1) return -1;
    if (bpPosition(node, offset, this.__ec, this.__eo) === 1) return 1;
    return 0;
  };
  Range.prototype.isPointInRange = function(node, offset) {
    requireNode(node, 'isPointInRange: node');
    offset = offset >>> 0;
    if (rootOf(node) !== rootOf(this.__sc)) return false;
    if (node.nodeType === 10) {
      throw new DOMException('node is a DocumentType', 'InvalidNodeTypeError');
    }
    if (offset > nodeLengthOf(node)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
    return bpPosition(node, offset, this.__sc, this.__so) !== -1 &&
           bpPosition(node, offset, this.__ec, this.__eo) !== 1;
  };
  Range.prototype.intersectsNode = function(node) {
    requireNode(node, 'intersectsNode: node');
    if (rootOf(node) !== rootOf(this.__sc)) return false;
    var parent = node.parentNode;
    if (!parent) return true;
    var offset = nodeIndex(node);
    return bpPosition(parent, offset, this.__ec, this.__eo) === -1 &&
           bpPosition(parent, offset + 1, this.__sc, this.__so) === 1;
  };
  Object.defineProperty(Range.prototype, 'commonAncestorContainer', {
    configurable: true,
    get: function() {
      var container = this.__sc;
      while (container && !container.contains(this.__ec)) container = container.parentNode;
      return container || this.__sc;
    }
  });
  Range.prototype.cloneRange = function() {
    var copy = new Range();
    setBothBP(copy, this.__sc, this.__so, this.__ec, this.__eo);
    return copy;
  };
  Range.prototype.detach = function() {};

  // A node is *contained* in a range when both its boundaries lie strictly
  // inside it; *partially contained* when it is an inclusive ancestor of
  // exactly one of the range's two nodes.
  function rangeContains(range, node) {
    if (rootOf(node) !== rootOf(range.__sc)) return false;
    return bpPosition(node, 0, range.__sc, range.__so) === 1 &&
           bpPosition(node, nodeLengthOf(node), range.__ec, range.__eo) === -1;
  }
  function rangePartiallyContains(range, node) {
    var a = node.contains(range.__sc);
    var b = node.contains(range.__ec);
    return (a && !b) || (b && !a);
  }

  // Appending a fragment now moves its children (the DOM's insertion steps), so
  // the Range algorithms that "append a fragment" are one `appendChild`.
  function appendFragmentChildren(target, fragment) {
    target.appendChild(fragment);
  }

  function containedChildrenOf(range, common) {
    var out = [];
    var count = +__childNodesCount(common.__ref);
    for (var i = 0; i < count; i++) {
      var child = childAt(common, i);
      if (rangeContains(range, child)) out.push(child);
    }
    return out;
  }

  function subRange(sc, so, ec, eo) {
    var r = new Range();
    setBothBP(r, sc, so, ec, eo);
    return r;
  }

  Range.prototype.deleteContents = function() {
    if (this.__sc === this.__ec && this.__so === this.__eo) return;
    var sn = this.__sc, so = this.__so, en = this.__ec, eo = this.__eo;
    if (sn === en && (sn.nodeType === 3 || sn.nodeType === 8 || sn.nodeType === 7)) {
      cdReplaceData(sn, so, eo - so, '');
      return;
    }
    var common = this.commonAncestorContainer;
    var toRemove = containedChildrenOf(this, common);
    var newNode, newOffset;
    if (sn.contains(en)) { newNode = sn; newOffset = so; }
    else {
      var reference = sn;
      while (reference.parentNode && !reference.parentNode.contains(en)) reference = reference.parentNode;
      newNode = reference.parentNode;
      newOffset = nodeIndex(reference) + 1;
    }
    if (sn.nodeType === 3 || sn.nodeType === 8 || sn.nodeType === 7) {
      cdReplaceData(sn, so, nodeLengthOf(sn) - so, '');
    }
    for (var i = 0; i < toRemove.length; i++) {
      if (toRemove[i].parentNode) toRemove[i].parentNode.removeChild(toRemove[i]);
    }
    if (en.nodeType === 3 || en.nodeType === 8 || en.nodeType === 7) {
      cdReplaceData(en, 0, eo, '');
    }
    setBothBP(this, newNode, newOffset, newNode, newOffset);
    selectionMayHaveChanged(this);
  };

  // DOM "extract" (extractContents) and "clone the contents" share their shape;
  // `extract` additionally removes what it took.
  function rangeExtractOrClone(range, extract) {
    var fragment = document.createDocumentFragment();
    var sn = range.__sc, so = range.__so, en = range.__ec, eo = range.__eo;
    if (sn === en && so === eo) return fragment;
    var isCharacterData = function(n) {
      return n.nodeType === 3 || n.nodeType === 8 || n.nodeType === 7;
    };
    if (sn === en && isCharacterData(sn)) {
      var only = sn.cloneNode(false);
      only.data = sn.substringData(so, eo - so);
      fragment.appendChild(only);
      if (extract) cdReplaceData(sn, so, eo - so, '');
      return fragment;
    }
    var common = range.commonAncestorContainer;
    var first = null, last = null;
    var count = +__childNodesCount(common.__ref);
    var i, child;
    if (!sn.contains(en)) {
      for (i = 0; i < count; i++) {
        child = childAt(common, i);
        if (rangePartiallyContains(range, child)) { first = child; break; }
      }
    }
    if (!en.contains(sn)) {
      for (i = count - 1; i >= 0; i--) {
        child = childAt(common, i);
        if (rangePartiallyContains(range, child)) { last = child; break; }
      }
    }
    var contained = containedChildrenOf(range, common);
    for (i = 0; i < contained.length; i++) {
      if (contained[i].nodeType === 10) {
        throw new DOMException('range contains a DocumentType', 'HierarchyRequestError');
      }
    }
    var newNode, newOffset;
    if (sn.contains(en)) { newNode = sn; newOffset = so; }
    else {
      var reference = sn;
      while (reference.parentNode && !reference.parentNode.contains(en)) reference = reference.parentNode;
      newNode = reference.parentNode;
      newOffset = nodeIndex(reference) + 1;
    }
    var clone;
    if (first && isCharacterData(first)) {
      clone = sn.cloneNode(false);
      clone.data = sn.substringData(so, nodeLengthOf(sn) - so);
      fragment.appendChild(clone);
      if (extract) cdReplaceData(sn, so, nodeLengthOf(sn) - so, '');
    } else if (first) {
      clone = first.cloneNode(false);
      fragment.appendChild(clone);
      var head = subRange(sn, so, first, nodeLengthOf(first));
      appendFragmentChildren(clone, rangeExtractOrClone(head, extract));
    }
    for (i = 0; i < contained.length; i++) {
      fragment.appendChild(extract ? contained[i] : contained[i].cloneNode(true));
    }
    if (last && isCharacterData(last)) {
      clone = en.cloneNode(false);
      clone.data = en.substringData(0, eo);
      fragment.appendChild(clone);
      if (extract) cdReplaceData(en, 0, eo, '');
    } else if (last) {
      clone = last.cloneNode(false);
      fragment.appendChild(clone);
      var tail = subRange(last, 0, en, eo);
      appendFragmentChildren(clone, rangeExtractOrClone(tail, extract));
    }
    if (extract) {
      setBothBP(range, newNode, newOffset, newNode, newOffset);
      selectionMayHaveChanged(range);
    }
    return fragment;
  }

  Range.prototype.extractContents = function() { return rangeExtractOrClone(this, true); };
  Range.prototype.cloneContents = function() { return rangeExtractOrClone(this, false); };

  Range.prototype.insertNode = function(node) {
    requireNode(node, 'insertNode: node');
    var sn = this.__sc, so = this.__so;
    if (sn.nodeType === 7 || sn.nodeType === 8 ||
        (sn.nodeType === 3 && !sn.parentNode) || node === sn) {
      throw new DOMException('cannot insert here', 'HierarchyRequestError');
    }
    var reference = sn.nodeType === 3 ? sn : childAt(sn, so);
    var parent = reference === null ? sn : reference.parentNode;
    if (node.contains && node.contains(parent)) {
      throw new DOMException('node is an ancestor of the insertion parent', 'HierarchyRequestError');
    }
    if (sn.nodeType === 3) reference = sn.splitText(so);
    if (node === reference) reference = node.nextSibling;
    if (node.parentNode) node.parentNode.removeChild(node);
    var newOffset = reference === null ? nodeLengthOf(parent) : nodeIndex(reference);
    newOffset += node.nodeType === 11 ? nodeLengthOf(node) : 1;
    parent.insertBefore(node, reference);
    if (this.__sc === this.__ec && this.__so === this.__eo) {
      setEndBP(this, parent, newOffset);
    }
    selectionMayHaveChanged(this);
  };

  Range.prototype.surroundContents = function(newParent) {
    requireNode(newParent, 'surroundContents: newParent');
    var common = this.commonAncestorContainer;
    var node = this.__sc;
    while (node) {
      if (node.nodeType !== 3 && node.nodeType !== 8 && node.nodeType !== 7 &&
          rangePartiallyContains(this, node)) {
        throw new DOMException('range partially contains a non-Text node', 'InvalidStateError');
      }
      if (node === common) break;
      node = node.parentNode;
    }
    node = this.__ec;
    while (node) {
      if (node.nodeType !== 3 && node.nodeType !== 8 && node.nodeType !== 7 &&
          rangePartiallyContains(this, node)) {
        throw new DOMException('range partially contains a non-Text node', 'InvalidStateError');
      }
      if (node === common) break;
      node = node.parentNode;
    }
    if (newParent.nodeType === 9 || newParent.nodeType === 10 || newParent.nodeType === 11) {
      throw new DOMException('invalid surround parent', 'InvalidNodeTypeError');
    }
    var fragment = this.extractContents();
    while (newParent.firstChild) newParent.removeChild(newParent.firstChild);
    this.insertNode(newParent);
    appendFragmentChildren(newParent, fragment);
    this.selectNode(newParent);
  };

  // Every Text node the range covers, in tree order.
  function rangeText(range) {
    var sn = range.__sc, so = range.__so, en = range.__ec, eo = range.__eo;
    if (sn === en && sn.nodeType === 3) return sn.substringData(so, eo - so);
    var out = '';
    if (sn.nodeType === 3) out += sn.substringData(so, nodeLengthOf(sn) - so);
    var common = range.commonAncestorContainer;
    var stack = [];
    var count = +__childNodesCount(common.__ref);
    for (var i = count - 1; i >= 0; i--) stack.push(childAt(common, i));
    while (stack.length) {
      var node = stack.pop();
      if (node.nodeType === 3 && rangeContains(range, node)) out += node.data;
      var kids = +__childNodesCount(node.__ref);
      for (var k = kids - 1; k >= 0; k--) stack.push(childAt(node, k));
    }
    if (en.nodeType === 3) out += en.substringData(0, eo);
    return out;
  }
  Range.prototype.toString = function() { return rangeText(this); };

  // The fragment parser, borrowed from the element the range starts in: the
  // context element's `innerHTML` runs the same parser, and the children move
  // into a fragment afterwards.
  Range.prototype.createContextualFragment = function(html) {
    var context = this.__sc;
    if (context.nodeType !== 1) context = context.parentNode || document.body || document.documentElement;
    var host = (context && context.nodeType === 1)
      ? context.cloneNode(false)
      : document.createElement('body');
    host.innerHTML = String(html);
    var fragment = document.createDocumentFragment();
    while (host.firstChild) fragment.appendChild(host.firstChild);
    return fragment;
  };

  // ---- geometry --------------------------------------------------------------
  //
  // `__rangeRects` asks the host's selection seam for viewport rectangles over
  // the laid-out document. No layout bound (or a range that resolves to no
  // shaped text) yields an empty list, which is what the spec asks of a range
  // in a document with no box tree.
  function rangeRectList(range) {
    var out = [];
    if (typeof __rangeRects !== 'function') return out;
    var blob = __rangeRects(range.__sc.__ref, range.__so >>> 0, range.__ec.__ref, range.__eo >>> 0);
    if (!blob) return out;
    var lines = String(blob).split('\n');
    for (var i = 0; i < lines.length; i++) {
      if (!lines[i]) continue;
      var f = lines[i].split(',');
      out.push(new DOMRect(+f[0], +f[1], +f[2], +f[3]));
    }
    return out;
  }
  Range.prototype.getClientRects = function() {
    var rects = rangeRectList(this);
    rects.item = function(i) { return this[i] === undefined ? null : this[i]; };
    return rects;
  };
  Range.prototype.getBoundingClientRect = function() {
    var rects = rangeRectList(this);
    if (!rects.length) return new DOMRect(0, 0, 0, 0);
    var left = rects[0].x, top = rects[0].y;
    var right = left + rects[0].width, bottom = top + rects[0].height;
    for (var i = 1; i < rects.length; i++) {
      var r = rects[i];
      if (r.width === 0 && r.height === 0) continue;
      if (r.x < left) left = r.x;
      if (r.y < top) top = r.y;
      if (r.x + r.width > right) right = r.x + r.width;
      if (r.y + r.height > bottom) bottom = r.y + r.height;
    }
    return new DOMRect(left, top, right - left, bottom - top);
  };

  setClassString(Range.prototype, 'Range');
  globalThis.Range = Range;

  // ---- Selection -------------------------------------------------------------
  //
  // One source of truth: the Selection's single live `Range` is it. The visual
  // selection Livery paints is a *projection* pushed through `__selectionVisual`
  // whenever that range changes, never a second authority; a host-side pointer
  // gesture reaches script through the same seam in the other direction.
  var selectionRange = null;
  var selectionDirection = 'none';
  var selectionChangeQueued = false;
  var theSelection = null;

  function selectionProject() {
    if (typeof __selectionVisual !== 'function') return;
    if (!selectionRange || (selectionRange.__sc === selectionRange.__ec &&
                            selectionRange.__so === selectionRange.__eo)) {
      __selectionVisual();
      return;
    }
    __selectionVisual(selectionRange.__sc.__ref, selectionRange.__so >>> 0,
                      selectionRange.__ec.__ref, selectionRange.__eo >>> 0);
  }

  // Called by every boundary mutator. A range that is not the selection's own
  // is not a selection change; the check is one identity comparison.
  function selectionMayHaveChanged(range) {
    if (range !== selectionRange) return;
    selectionProject();
    queueSelectionChange();
  }

  function queueSelectionChange() {
    if (selectionChangeQueued) return;
    selectionChangeQueued = true;
    Promise.resolve().then(function() {
      selectionChangeQueued = false;
      try {
        document.dispatchEvent(new Event('selectionchange', { bubbles: false, cancelable: false }));
      } catch (_) {}
    });
  }

  function selectionSet(range, direction) {
    selectionRange = range;
    selectionDirection = range ? (direction || 'forward') : 'none';
    selectionProject();
    queueSelectionChange();
  }

  function Selection() { throw new TypeError('Illegal constructor'); }
  Selection.prototype = Object.create(Object.prototype);
  Object.defineProperty(Selection.prototype, 'constructor', {
    configurable: true, writable: true, value: Selection
  });
  function selectionAnchorIsEnd() { return selectionDirection === 'backward'; }
  Object.defineProperty(Selection.prototype, 'anchorNode', {
    configurable: true,
    get: function() {
      if (!selectionRange) return null;
      return selectionAnchorIsEnd() ? selectionRange.__ec : selectionRange.__sc;
    }
  });
  Object.defineProperty(Selection.prototype, 'anchorOffset', {
    configurable: true,
    get: function() {
      if (!selectionRange) return 0;
      return selectionAnchorIsEnd() ? selectionRange.__eo : selectionRange.__so;
    }
  });
  Object.defineProperty(Selection.prototype, 'focusNode', {
    configurable: true,
    get: function() {
      if (!selectionRange) return null;
      return selectionAnchorIsEnd() ? selectionRange.__sc : selectionRange.__ec;
    }
  });
  Object.defineProperty(Selection.prototype, 'focusOffset', {
    configurable: true,
    get: function() {
      if (!selectionRange) return 0;
      return selectionAnchorIsEnd() ? selectionRange.__so : selectionRange.__eo;
    }
  });
  Object.defineProperty(Selection.prototype, 'isCollapsed', {
    configurable: true,
    get: function() {
      return !selectionRange ||
             (selectionRange.__sc === selectionRange.__ec && selectionRange.__so === selectionRange.__eo);
    }
  });
  Object.defineProperty(Selection.prototype, 'rangeCount', {
    configurable: true, get: function() { return selectionRange ? 1 : 0; }
  });
  Object.defineProperty(Selection.prototype, 'type', {
    configurable: true,
    get: function() {
      if (!selectionRange) return 'None';
      return (selectionRange.__sc === selectionRange.__ec && selectionRange.__so === selectionRange.__eo)
        ? 'Caret' : 'Range';
    }
  });
  Object.defineProperty(Selection.prototype, 'direction', {
    configurable: true,
    get: function() { return selectionRange ? selectionDirection : 'none'; }
  });
  Selection.prototype.getRangeAt = function(index) {
    index = index >>> 0;
    if (!selectionRange || index !== 0) {
      throw new DOMException('no range at that index', 'IndexSizeError');
    }
    return selectionRange;
  };
  Selection.prototype.addRange = function(range) {
    if (!range || range.__sc === undefined) {
      throw new TypeError('addRange: argument is not a Range');
    }
    if (rootOf(range.__sc) !== document) return;
    if (selectionRange) return;
    selectionSet(range, 'forward');
  };
  Selection.prototype.removeRange = function(range) {
    if (range !== selectionRange) {
      throw new DOMException('range is not in the selection', 'NotFoundError');
    }
    selectionSet(null, 'none');
  };
  Selection.prototype.removeAllRanges = function() { selectionSet(null, 'none'); };
  Selection.prototype.empty = function() { selectionSet(null, 'none'); };
  function selectionCollapse(node, offset) {
    if (node === null || node === undefined) { selectionSet(null, 'none'); return; }
    requireNode(node, 'collapse: node');
    if (node.nodeType === 10) {
      throw new DOMException('node is a DocumentType', 'InvalidNodeTypeError');
    }
    offset = offset === undefined ? 0 : offset >>> 0;
    if (offset > nodeLengthOf(node)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
    if (rootOf(node) !== document) return;
    var range = new Range();
    setBothBP(range, node, offset, node, offset);
    selectionSet(range, 'forward');
  }
  Selection.prototype.collapse = function(node, offset) { selectionCollapse(node, offset); };
  Selection.prototype.setPosition = function(node, offset) { selectionCollapse(node, offset); };
  Selection.prototype.collapseToStart = function() {
    if (!selectionRange) {
      throw new DOMException('no selection', 'InvalidStateError');
    }
    selectionCollapse(selectionRange.__sc, selectionRange.__so);
  };
  Selection.prototype.collapseToEnd = function() {
    if (!selectionRange) {
      throw new DOMException('no selection', 'InvalidStateError');
    }
    selectionCollapse(selectionRange.__ec, selectionRange.__eo);
  };
  Selection.prototype.extend = function(node, offset) {
    requireNode(node, 'extend: node');
    if (rootOf(node) !== document) {
      throw new DOMException('node is not in the document', 'InvalidNodeTypeError');
    }
    if (!selectionRange) {
      throw new DOMException('no selection', 'InvalidStateError');
    }
    offset = offset === undefined ? 0 : offset >>> 0;
    if (offset > nodeLengthOf(node)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
    var anchorNode = selectionAnchorIsEnd() ? selectionRange.__ec : selectionRange.__sc;
    var anchorOffset = selectionAnchorIsEnd() ? selectionRange.__eo : selectionRange.__so;
    var range = new Range();
    if (bpPosition(node, offset, anchorNode, anchorOffset) === -1) {
      setBothBP(range, node, offset, anchorNode, anchorOffset);
      selectionSet(range, 'backward');
    } else {
      setBothBP(range, anchorNode, anchorOffset, node, offset);
      selectionSet(range, 'forward');
    }
  };
  Selection.prototype.setBaseAndExtent = function(anchorNode, anchorOffset, focusNode, focusOffset) {
    requireNode(anchorNode, 'setBaseAndExtent: anchorNode');
    requireNode(focusNode, 'setBaseAndExtent: focusNode');
    anchorOffset = anchorOffset >>> 0;
    focusOffset = focusOffset >>> 0;
    if (anchorOffset > nodeLengthOf(anchorNode) || focusOffset > nodeLengthOf(focusNode)) {
      throw new DOMException('offset is past the end of the node', 'IndexSizeError');
    }
    if (rootOf(anchorNode) !== document || rootOf(focusNode) !== document) return;
    var range = new Range();
    if (bpPosition(anchorNode, anchorOffset, focusNode, focusOffset) === 1) {
      setBothBP(range, focusNode, focusOffset, anchorNode, anchorOffset);
      selectionSet(range, 'backward');
    } else {
      setBothBP(range, anchorNode, anchorOffset, focusNode, focusOffset);
      selectionSet(range, 'forward');
    }
  };
  Selection.prototype.selectAllChildren = function(node) {
    requireNode(node, 'selectAllChildren: node');
    if (node.nodeType === 10) {
      throw new DOMException('node is a DocumentType', 'InvalidNodeTypeError');
    }
    if (rootOf(node) !== document) return;
    var range = new Range();
    setBothBP(range, node, 0, node, nodeLengthOf(node));
    selectionSet(range, 'forward');
  };
  Selection.prototype.deleteFromDocument = function() {
    if (selectionRange) selectionRange.deleteContents();
  };
  Selection.prototype.containsNode = function(node, allowPartial) {
    requireNode(node, 'containsNode: node');
    if (!selectionRange || rootOf(node) !== document) return false;
    if (allowPartial) return selectionRange.intersectsNode(node);
    var parent = node.parentNode;
    if (!parent) return false;
    var index = nodeIndex(node);
    return bpPosition(parent, index, selectionRange.__sc, selectionRange.__so) !== -1 &&
           bpPosition(parent, index + 1, selectionRange.__ec, selectionRange.__eo) !== 1;
  };
  Selection.prototype.getComposedRanges = function() {
    if (!selectionRange) return [];
    return [new StaticRange({
      startContainer: selectionRange.__sc, startOffset: selectionRange.__so,
      endContainer: selectionRange.__ec, endOffset: selectionRange.__eo
    })];
  };
  // `modify` needs shaped-text granularity the script tier does not own; it is
  // a no-op rather than a throw so a caller's feature test still succeeds.
  Selection.prototype.modify = function() {};
  Selection.prototype.toString = function() {
    return selectionRange ? rangeText(selectionRange) : '';
  };
  setClassString(Selection.prototype, 'Selection');
  globalThis.Selection = Selection;

  function getSelection() {
    if (!theSelection) theSelection = Object.create(Selection.prototype);
    return theSelection;
  }
  globalThis.getSelection = getSelection;
  if (globalThis.window) { globalThis.window.getSelection = getSelection; }

  // The host's pointer gesture, arriving as the same boundary points the script
  // side would have set. This is the projection running the other way, and it
  // is the only writer of the selection that is not script.
  globalThis.__selectionFromHost = function(scRef, so, ecRef, eo) {
    if (scRef === undefined || scRef === null) { selectionSet(null, 'none'); return; }
    var range = new Range();
    setBothBP(range, wrapNode(scRef), so >>> 0, wrapNode(ecRef), eo >>> 0);
    selectionRange = range;
    selectionDirection = 'forward';
    queueSelectionChange();
  };

  installHtmlInterfaceTable();
  installShapeInterfaces();
  installTraversal();

  // Live HTMLCollection / NodeList as legacy-platform exotic objects, modeled with
  // a JS Proxy (both backends support the get/has/ownKeys/getOwnPropertyDescriptor
  // traps — verified by `proxy_capability`). `getItems()` returns the current
  // element/node array, re-read per access for liveness. `isHtml` selects
  // HTMLCollection (named access + namedItem, no forEach/values/entries/keys) vs
  // NodeList (forEach/entries/keys/values, no named access).
  function isArrayIndex(k) {
    return typeof k === 'string' && /^(0|[1-9][0-9]*)$/.test(k) && k <= 4294967294;
  }
  function collectionIterator(getItems) {
    var i = 0;
    var it = { next: function() {
      var a = getItems();
      return i < a.length ? { value: a[i++], done: false } : { value: undefined, done: true };
    } };
    it[Symbol.iterator] = function() { return it; };
    return it;
  }
  function supportedNames(getItems) {
    var a = getItems(); var seen = {}; var out = [];
    for (var i = 0; i < a.length; i++) {
      var id = a[i].getAttribute && a[i].getAttribute('id');
      if (id && !seen['$' + id]) { seen['$' + id] = 1; out.push(id); }
      var nm = a[i].getAttribute && a[i].getAttribute('name');
      if (nm && !seen['$' + nm]) { seen['$' + nm] = 1; out.push(nm); }
    }
    return out;
  }
  function namedMatch(getItems, name) {
    var a = getItems();
    for (var i = 0; i < a.length; i++) {
      if (a[i].getAttribute && (a[i].getAttribute('id') === name || a[i].getAttribute('name') === name)) return a[i];
    }
    return null;
  }
  function makeCollection(getItems, isHtml) {
    var handler = {
      get: function(t, k) {
        if (k === 'length') return getItems().length;
        if (k === 'item') return function(i) { var a = getItems(); i = i >>> 0; return i < a.length ? a[i] : null; };
        if (k === Symbol.iterator) return function() { return collectionIterator(getItems); };
        if (isHtml) {
          if (k === 'namedItem') return function(name) {
            name = String(name); return name === '' ? null : namedMatch(getItems, name);
          };
        } else {
          if (k === 'forEach') return function(cb, thisArg) { var a = getItems(); for (var i = 0; i < a.length; i++) cb.call(thisArg, a[i], i, this); };
          if (k === 'values') return function() { return collectionIterator(getItems); };
          if (k === 'keys') return function() { var i = 0; var a = getItems(); var o = { next: function() { return i < a.length ? { value: i++, done: false } : { value: undefined, done: true }; } }; o[Symbol.iterator] = function() { return o; }; return o; };
          if (k === 'entries') return function() { var i = 0; var a = getItems(); var o = { next: function() { return i < a.length ? { value: [i, a[i++]], done: false } : { value: undefined, done: true }; } }; o[Symbol.iterator] = function() { return o; }; return o; };
        }
        if (isArrayIndex(k)) { var a = getItems(); var idx = +k; return idx < a.length ? a[idx] : undefined; }
        if (isHtml && typeof k === 'string') { var m = namedMatch(getItems, k); if (m) return m; }
        return t[k];
      },
      has: function(t, k) {
        if (k === 'length' || k === 'item' || k === Symbol.iterator) return true;
        if (isHtml && k === 'namedItem') return true;
        if (!isHtml && (k === 'forEach' || k === 'values' || k === 'keys' || k === 'entries')) return true;
        if (isArrayIndex(k)) return (+k) < getItems().length;
        if (isHtml && typeof k === 'string' && namedMatch(getItems, k)) return true;
        return k in t;
      },
      ownKeys: function() {
        var a = getItems(); var keys = [];
        for (var i = 0; i < a.length; i++) keys.push(String(i));
        if (isHtml) { var names = supportedNames(getItems); for (var j = 0; j < names.length; j++) keys.push(names[j]); }
        return keys;
      },
      getOwnPropertyDescriptor: function(t, k) {
        if (isArrayIndex(k)) { var a = getItems(); var idx = +k; if (idx < a.length) return { value: a[idx], writable: false, enumerable: true, configurable: true }; return undefined; }
        if (isHtml && typeof k === 'string') { var m = namedMatch(getItems, k); if (m) return { value: m, writable: false, enumerable: true, configurable: true }; }
        return Object.getOwnPropertyDescriptor(t, k);
      }
    };
    return new Proxy({}, handler);
  }
  // Raw (real Array) child nodes — internal, for collection backing and filtering.
  function rawChildNodes(node) {
    var n = +__childNodesCount(node.__ref);
    var out = [];
    for (var i = 0; i < n; i++) { out.push(wrapNode(__childNodesItem(node.__ref, String(i)))); }
    return out;
  }

  // DOMTokenList: a real iterable, branded object over a whitespace-separated
  // attribute (class, rel). Prototype carries the methods; a Proxy adds indexed
  // access + ownKeys (the exotic index machinery, same Proxy route as collections).
  function DOMTokenList() {}
  DOMTokenList.prototype._toks = function() {
    var c = this.__el.getAttribute(this.__attr);
    return c ? c.trim().split(/\s+/).filter(function(s) { return s.length; }) : [];
  };
  DOMTokenList.prototype._write = function(a) { this.__el.setAttribute(this.__attr, a.join(' ')); };
  Object.defineProperty(DOMTokenList.prototype, 'length', {
    configurable: true, get: function() { return this._toks().length; }
  });
  Object.defineProperty(DOMTokenList.prototype, 'value', {
    configurable: true,
    get: function() { return this.__el.getAttribute(this.__attr) || ''; },
    set: function(v) { this.__el.setAttribute(this.__attr, String(v)); }
  });
  DOMTokenList.prototype.item = function(i) { var t = this._toks(); i = i >>> 0; return i < t.length ? t[i] : null; };
  DOMTokenList.prototype.contains = function(tok) { return this._toks().indexOf(String(tok)) !== -1; };
  // Every token is validated before anything is written, so a bad token in a
  // multi-token call leaves the attribute — and the mutation record — untouched.
  var TOKEN_WHITESPACE = String.fromCharCode(32, 9, 10, 12, 13);
  function tokenHasWhitespace(tok) {
    for (var i = 0; i < tok.length; i++) {
      if (TOKEN_WHITESPACE.indexOf(tok.charAt(i)) !== -1) return true;
    }
    return false;
  }
  function tokenListValidate(args) {
    var out = [];
    for (var i = 0; i < args.length; i++) {
      var tok = String(args[i]);
      if (tok === '') throw new DOMException('The token provided must not be empty.', 'SyntaxError');
      if (tokenHasWhitespace(tok)) {
        throw new DOMException("The token '" + tok + "' contains whitespace.", 'InvalidCharacterError');
      }
      out.push(tok);
    }
    return out;
  }
  DOMTokenList.prototype.add = function() {
    var toks = tokenListValidate(arguments); var t = this._toks();
    for (var i = 0; i < toks.length; i++) { if (t.indexOf(toks[i]) === -1) t.push(toks[i]); }
    this._write(t);
  };
  DOMTokenList.prototype.remove = function() {
    var toks = tokenListValidate(arguments); var t = this._toks();
    for (var i = 0; i < toks.length; i++) { var x = t.indexOf(toks[i]); if (x !== -1) t.splice(x, 1); }
    this._write(t);
  };
  DOMTokenList.prototype.toggle = function(tok, force) {
    tok = tokenListValidate([tok])[0]; var t = this._toks(); var has = t.indexOf(tok) !== -1;
    if (force === true || (force === undefined && !has)) { if (!has) { t.push(tok); this._write(t); } return true; }
    if (has) { t.splice(t.indexOf(tok), 1); this._write(t); }
    return false;
  };
  DOMTokenList.prototype.replace = function(oldT, newT) {
    var toks = tokenListValidate([oldT, newT]);
    oldT = toks[0]; newT = toks[1]; var t = this._toks(); var i = t.indexOf(oldT);
    if (i === -1) return false;
    if (t.indexOf(newT) !== -1 && newT !== oldT) { t.splice(i, 1); } else { t[i] = newT; }
    this._write(t); return true;
  };
  DOMTokenList.prototype.supports = function() { return true; };
  DOMTokenList.prototype.forEach = function(cb, thisArg) { var t = this._toks(); for (var i = 0; i < t.length; i++) cb.call(thisArg, t[i], i, this); };
  DOMTokenList.prototype.values = function() { var t = this._toks(); var i = 0; var o = { next: function() { return i < t.length ? { value: t[i++], done: false } : { value: undefined, done: true }; } }; o[Symbol.iterator] = function() { return o; }; return o; };
  DOMTokenList.prototype.keys = function() { var t = this._toks(); var i = 0; var o = { next: function() { return i < t.length ? { value: i++, done: false } : { value: undefined, done: true }; } }; o[Symbol.iterator] = function() { return o; }; return o; };
  DOMTokenList.prototype.entries = function() { var t = this._toks(); var i = 0; var o = { next: function() { return i < t.length ? { value: [i, t[i++]], done: false } : { value: undefined, done: true }; } }; o[Symbol.iterator] = function() { return o; }; return o; };
  DOMTokenList.prototype[Symbol.iterator] = DOMTokenList.prototype.values;
  DOMTokenList.prototype[Symbol.toStringTag] = 'DOMTokenList';
  DOMTokenList.prototype.toString = function() { return this.value; };
  globalThis.DOMTokenList = DOMTokenList;
  function makeDOMTokenList(el, attr) {
    var inst = Object.create(DOMTokenList.prototype);
    inst.__el = el; inst.__attr = attr;
    return new Proxy(inst, {
      get: function(t, k) {
        if (isArrayIndex(k)) { var toks = t._toks(); var i = +k; return i < toks.length ? toks[i] : undefined; }
        return t[k];
      },
      has: function(t, k) { if (isArrayIndex(k)) return (+k) < t._toks().length; return k in t; },
      ownKeys: function(t) { var toks = t._toks(); var keys = []; for (var i = 0; i < toks.length; i++) keys.push(String(i)); return keys; },
      getOwnPropertyDescriptor: function(t, k) {
        if (isArrayIndex(k)) { var toks = t._toks(); var i = +k; if (i < toks.length) return { value: toks[i], writable: false, enumerable: true, configurable: true }; return undefined; }
        return Object.getOwnPropertyDescriptor(t, k);
      }
    });
  }

  // dataset: a DOMStringMap named-property exotic. IDL key `fooBar` maps to the
  // content attribute `data-foo-bar` and back. A Proxy intercepts get/set/has/
  // delete/ownKeys over the element's data-* attributes.
  function datasetToContent(k) {
    // camelCase -> data-kebab; an uppercase becomes -lowercase.
    return 'data-' + k.replace(/[A-Z]/g, function(c) { return '-' + c.toLowerCase(); });
  }
  function datasetToIdl(name) {
    // data-foo-bar -> fooBar; -x becomes X.
    return name.slice(5).replace(/-([a-z])/g, function(_, c) { return c.toUpperCase(); });
  }
  function makeDataset(el) {
    return new Proxy({ __el: el }, {
      get: function(t, k) {
        if (typeof k !== 'string') return t[k];
        var v = t.__el.getAttribute(datasetToContent(k));
        return v === null ? undefined : v;
      },
      set: function(t, k, v) {
        if (typeof k === 'string') t.__el.setAttribute(datasetToContent(k), String(v));
        return true;
      },
      has: function(t, k) {
        if (typeof k !== 'string') return k in t;
        return t.__el.getAttribute(datasetToContent(k)) !== null;
      },
      deleteProperty: function(t, k) {
        if (typeof k === 'string') t.__el.removeAttribute(datasetToContent(k));
        return true;
      },
      ownKeys: function(t) {
        var names = __attributeNames(t.__el.__ref);
        var out = [];
        if (names) {
          var parts = names.split(' ');
          for (var i = 0; i < parts.length; i++) { if (parts[i].indexOf('data-') === 0) out.push(datasetToIdl(parts[i])); }
        }
        return out;
      },
      getOwnPropertyDescriptor: function(t, k) {
        if (typeof k === 'string') {
          var v = t.__el.getAttribute(datasetToContent(k));
          if (v !== null) return { value: v, writable: true, enumerable: true, configurable: true };
        }
        return undefined;
      }
    });
  }

  // Reflected IDL attribute accessors on Element.prototype (Lever 1). Driven by a
  // table of [idlName, attr, kind]; kinds: s=DOMString, tc=textContent, b=boolean, e=enumerated
  // (approximate: lowercased pass-through, '' default — keyword canonicalization
  // deferred), l=long, t=tokenlist (a DOMTokenList over the attribute), u=url
  // (resolved against the document base URL via `__resolve_url`). Only `double` is
  // deferred. All over the existing get/set/has/toggle/removeAttribute.
  function installReflectedAttributes(proto, attrs) {
    function parseHtmlLong(s) {
      if (s === null || s === undefined) return null;
      var m = /^[ \t\n\f\r]*([+-]?[0-9]+)/.exec(String(s));
      return m ? parseInt(m[1], 10) : null;
    }
    function parseHtmlUnsignedLong(s) {
      if (s === null || s === undefined) return null;
      var m = /^[ \t\n\f\r]*([0-9]+)/.exec(String(s));
      return m ? parseInt(m[1], 10) : null;
    }
    function toLong(v) {
      v = Number(v);
      if (!isFinite(v)) return 0;
      return (v < 0 ? Math.ceil(v) : Math.floor(v)) | 0;
    }
    function toUnsignedLong(v) {
      v = Number(v);
      if (!isFinite(v) || v < 0) return 0;
      return Math.floor(v) >>> 0;
    }
    function def(idl, kind, attr, keywords, miss, readonly) {
      // Reflected HTML content-attribute names are lowercase (tabIndex ->
      // tabindex); this also keeps get/set consistent with HTML setAttribute,
      // which lowercases. Explicit names passed in are already lowercase.
      attr = (attr || idl).toLowerCase();
      var desc = { configurable: true, enumerable: true };
      if (kind === 's') {
        desc.get = function() { var v = this.getAttribute(attr); return v === null ? '' : v; };
        desc.set = function(v) { this.setAttribute(attr, String(v)); };
      } else if (kind === 'tc') {
        desc.get = function() { return this.textContent || ''; };
        desc.set = function(v) { this.textContent = String(v); };
      } else if (kind === 'b') {
        desc.get = function() { return this.hasAttribute(attr); };
        desc.set = function(v) { this.toggleAttribute(attr, !!v); };
      } else if (kind === 'e') {
        // Enumerated: canonicalize the stored token (ASCII case-insensitive) against
        // the allowed keyword set; unknown/absent returns the missing-value default
        // (`miss`, default ""). This is the limited-enum-with-"" case, which covers
        // most reflected enums; per-attribute invalid-value defaults are later work.
        var kw = keywords || [];
        var missing = miss || '';
        desc.get = function() {
          var v = this.getAttribute(attr);
          if (v === null) return missing;
          v = String(v).toLowerCase();
          return kw.indexOf(v) !== -1 ? v : missing;
        };
        desc.set = function(v) { this.setAttribute(attr, String(v)); };
      } else if (kind === 'l') {
        var longMissing = miss === null || miss === undefined ? -1 : toLong(miss);
        desc.get = function() { var n = parseHtmlLong(this.getAttribute(attr)); return n === null ? longMissing : n; };
        desc.set = function(v) { this.setAttribute(attr, String(toLong(v))); };
      } else if (kind === 'ul') {
        var unsignedMissing = miss === null || miss === undefined ? 0 : toUnsignedLong(miss);
        desc.get = function() { var n = parseHtmlUnsignedLong(this.getAttribute(attr)); return n === null ? unsignedMissing : n; };
        desc.set = function(v) { this.setAttribute(attr, String(toUnsignedLong(v))); };
      } else if (kind === 't') {
        // tokenlist: a DOMTokenList over the content attribute (e.g. relList -> rel).
        // Every one of these is [SameObject, PutForwards=value] in IDL, so
        // assigning to the IDL attribute writes the content attribute.
        desc.get = function() { return makeDOMTokenList(this, attr); };
        desc.set = function(v) { this.setAttribute(attr, String(v)); };
      } else if (kind === 'u') {
        // url: reflect the content attribute resolved against the document base URL
        // (absent -> ""); the raw string is stored on set. `__resolve_url` returns the
        // input unchanged when it is already absolute or no base URL is set.
        desc.get = function() { var v = this.getAttribute(attr); return v === null ? '' : __resolve_url(v); };
        desc.set = function(v) { this.setAttribute(attr, String(v)); };
      }
      // A readonly IDL attribute keeps only its getter, except the
      // PutForwards token lists above, whose setter is the forwarded write.
      if (readonly && kind !== 't') delete desc.set;
      if (desc.get || desc.set) Object.defineProperty(proto, idl, desc);
    }
    for (var i = 0; i < attrs.length; i++) {
      var a = attrs[i];
      def(a.idl, a.kind, a.attr, a.keywords || [], a.missing, !!a.readonly);
    }
  }

  // NodeFilter + createTreeWalker / createNodeIterator (Lever 3), pure JS over the
  // wrapNode tree (firstChild/nextSibling/parentNode). Implements the DOM filter
  // semantics (whatToShow bitmask + ACCEPT/REJECT/SKIP) and the spec traversal.
  function installTraversal() {
    var NodeFilter = {
      FILTER_ACCEPT: 1, FILTER_REJECT: 2, FILTER_SKIP: 3,
      SHOW_ALL: 0xFFFFFFFF, SHOW_ELEMENT: 0x1, SHOW_ATTRIBUTE: 0x2, SHOW_TEXT: 0x4,
      SHOW_CDATA_SECTION: 0x8, SHOW_PROCESSING_INSTRUCTION: 0x40, SHOW_COMMENT: 0x80,
      SHOW_DOCUMENT: 0x100, SHOW_DOCUMENT_TYPE: 0x200, SHOW_DOCUMENT_FRAGMENT: 0x400
    };
    globalThis.NodeFilter = NodeFilter;

    function filterNode(node, whatToShow, filter) {
      if (!((whatToShow >>> 0) & (1 << (node.nodeType - 1)))) return NodeFilter.FILTER_SKIP;
      if (!filter) return NodeFilter.FILTER_ACCEPT;
      return (typeof filter === 'function') ? filter(node) : filter.acceptNode(node);
    }

    function TreeWalker(root, whatToShow, filter) {
      this.root = root;
      this.whatToShow = whatToShow >>> 0;
      this.filter = filter || null;
      this.currentNode = root;
    }
    TreeWalker.prototype._f = function(n) { return filterNode(n, this.whatToShow, this.filter); };
    TreeWalker.prototype.parentNode = function() {
      var node = this.currentNode;
      while (node !== null && node !== this.root) {
        node = node.parentNode;
        if (node !== null && this._f(node) === 1) { this.currentNode = node; return node; }
      }
      return null;
    };
    TreeWalker.prototype._traverseChildren = function(first) {
      var node = first ? this.currentNode.firstChild : this.currentNode.lastChild;
      while (node !== null) {
        var result = this._f(node);
        if (result === 1) { this.currentNode = node; return node; }
        if (result === 3) {
          var child = first ? node.firstChild : node.lastChild;
          if (child !== null) { node = child; continue; }
        }
        while (node !== null) {
          var sibling = first ? node.nextSibling : node.previousSibling;
          if (sibling !== null) { node = sibling; break; }
          var parent = node.parentNode;
          if (parent === null || parent === this.root || parent === this.currentNode) return null;
          node = parent;
        }
      }
      return null;
    };
    TreeWalker.prototype.firstChild = function() { return this._traverseChildren(true); };
    TreeWalker.prototype.lastChild = function() { return this._traverseChildren(false); };
    TreeWalker.prototype._traverseSiblings = function(next) {
      var node = this.currentNode;
      if (node === this.root) return null;
      while (true) {
        var sibling = next ? node.nextSibling : node.previousSibling;
        while (sibling !== null) {
          node = sibling;
          var result = this._f(node);
          if (result === 1) { this.currentNode = node; return node; }
          sibling = next ? node.firstChild : node.lastChild;
          if (result === 2 || sibling === null) { sibling = next ? node.nextSibling : node.previousSibling; }
        }
        node = node.parentNode;
        if (node === null || node === this.root) return null;
        if (this._f(node) === 1) return null;
      }
    };
    TreeWalker.prototype.nextSibling = function() { return this._traverseSiblings(true); };
    TreeWalker.prototype.previousSibling = function() { return this._traverseSiblings(false); };
    TreeWalker.prototype.nextNode = function() {
      var node = this.currentNode;
      var result = 1;
      while (true) {
        while (result !== 2 && node.firstChild !== null) {
          node = node.firstChild;
          result = this._f(node);
          if (result === 1) { this.currentNode = node; return node; }
        }
        var temporary = node;
        var sibling = null;
        while (temporary !== null) {
          if (temporary === this.root) return null;
          sibling = temporary.nextSibling;
          if (sibling !== null) break;
          temporary = temporary.parentNode;
        }
        if (sibling === null) return null;
        node = sibling;
        result = this._f(node);
        if (result === 1) { this.currentNode = node; return node; }
      }
    };
    TreeWalker.prototype.previousNode = function() {
      var node = this.currentNode;
      while (node !== this.root) {
        var sibling = node.previousSibling;
        while (sibling !== null) {
          node = sibling;
          var result = this._f(node);
          while (result !== 2 && node.lastChild !== null) {
            node = node.lastChild;
            result = this._f(node);
          }
          if (result === 1) { this.currentNode = node; return node; }
          sibling = node.previousSibling;
        }
        if (node === this.root) return null;
        var parent = node.parentNode;
        if (parent === null) return null;
        node = parent;
        if (this._f(node) === 1) { this.currentNode = node; return node; }
      }
      return null;
    };
    globalThis.TreeWalker = TreeWalker;
    Document.prototype.createTreeWalker = function(root, whatToShow, filter) {
      return new TreeWalker(root, whatToShow === undefined ? 0xFFFFFFFF : whatToShow, filter);
    };

    // NodeIterator over document order within root's subtree.
    function following(node, root) {
      if (node.firstChild) return node.firstChild;
      var n = node;
      while (n) {
        if (n === root) return null;
        if (n.nextSibling) return n.nextSibling;
        n = n.parentNode;
      }
      return null;
    }
    function preceding(node, root) {
      if (node === root) return null;
      if (node.previousSibling) {
        var n = node.previousSibling;
        while (n.lastChild) n = n.lastChild;
        return n;
      }
      return node.parentNode === root ? null : node.parentNode;
    }
    function NodeIterator(root, whatToShow, filter) {
      this.root = root;
      this.whatToShow = whatToShow >>> 0;
      this.filter = filter || null;
      this.referenceNode = root;
      this.pointerBeforeReferenceNode = true;
    }
    NodeIterator.prototype._traverse = function(next) {
      var node = this.referenceNode;
      var beforeNode = this.pointerBeforeReferenceNode;
      while (true) {
        if (next) {
          if (!beforeNode) { node = following(node, this.root); if (node === null) return null; }
          else { beforeNode = false; }
        } else {
          if (beforeNode) { node = preceding(node, this.root); if (node === null) return null; }
          else { beforeNode = true; }
        }
        if (filterNode(node, this.whatToShow, this.filter) === 1) {
          this.referenceNode = node;
          this.pointerBeforeReferenceNode = beforeNode;
          return node;
        }
      }
    };
    NodeIterator.prototype.nextNode = function() { return this._traverse(true); };
    NodeIterator.prototype.previousNode = function() { return this._traverse(false); };
    NodeIterator.prototype.detach = function() {};
    globalThis.NodeIterator = NodeIterator;
    Document.prototype.createNodeIterator = function(root, whatToShow, filter) {
      return new NodeIterator(root, whatToShow === undefined ? 0xFFFFFFFF : whatToShow, filter);
    };
  }

  // The document is a Document instance over the root reflector, registered in the
  // wrapper cache so wrapNode(rootRef) returns this same object.
  var docRef = __documentRoot();
  var document = Object.create(Document.prototype);
  document.__ref = docRef;
  document.nodeType = 9;
  wrappers.set(docRef, document);
  globalThis.document = document;

  // A snapshot-cloned runtime keeps this JS heap but swaps the Rust host, so
  // the retained `document` still points at the donor document's root -- any
  // DOM access through it would hand a foreign NodeId to the fresh host. The
  // host calls this after a swap; reflectors are canonical per host, so on an
  // unswapped runtime the root compares identical and this is a no-op.
  globalThis.__rebindDocument = function() {
    var freshRef = __documentRoot();
    if (freshRef === docRef || freshRef === undefined || freshRef === null) {
      return;
    }
    wrappers.delete(docRef);
    docRef = freshRef;
    document.__ref = docRef;
    wrappers.set(docRef, document);
  };

  // Refresh the parse-time named-element properties after the host clones a
  // source document into this live tree. The getter keeps later replacement
  // of an element with the same id observable without pinning its wrapper.
  // HTML's complete WindowNamedProperties rules are broader; this retained
  // lane intentionally covers non-colliding element ids.
  var installedNamedProperties = [];
  globalThis.__refreshNamedProperties = function() {
    for (var oldIndex = 0; oldIndex < installedNamedProperties.length; oldIndex++) {
      delete globalThis[installedNamedProperties[oldIndex]];
    }
    installedNamedProperties = [];
    var elements = document.querySelectorAll('*');
    for (var index = 0; index < elements.length; index++) {
      var name = elements[index].getAttribute('id');
      if (!name || Object.prototype.hasOwnProperty.call(globalThis, name)) {
        continue;
      }
      (function(namedId) {
        Object.defineProperty(globalThis, namedId, {
          configurable: true,
          enumerable: true,
          get: function() { return document.getElementById(namedId); },
          // HTML puts named properties on Window's prototype chain, so a
          // script assigning the same name shadows them. Here they are own
          // accessors, and a getter-only accessor would make that assignment
          // fail silently -- an `id="test"` element would then shadow
          // testharness's own `test()`. Replacing self with a data property
          // reproduces the shadowing.
          set: function(value) {
            Object.defineProperty(globalThis, namedId, {
              configurable: true,
              enumerable: true,
              writable: true,
              value: value
            });
          }
        });
      })(name);
      installedNamedProperties.push(name);
    }
  };

  // Host-facing synthetic-event entry (the input -> event bridge). `wrapNode` is
  // IIFE-local, so a host eval can't reach it directly; this exposes a minimal
  // global the host calls with a raw NodeId (e.g. from a hit-test) and an event
  // type. Returns dispatchEvent's value: false iff preventDefault was called, so
  // the host knows whether to run the default action (follow the link, etc.).
  globalThis.__dispatchSynthetic = function(rawId, type, opts) {
    var node = wrapNode(__reflectNode(String(rawId)));
    if (!node) { return false; }
    var ev = new Event(String(type), opts || { bubbles: true, cancelable: true });
    return node.dispatchEvent(ev);
  };

  // TransitionEvent (css-transitions): an Event carrying `propertyName`,
  // `elapsedTime`, and `pseudoElement`. Bubbles, not cancelable. Minimal:
  // backed by the shell Event with the extra fields attached and the instance
  // reparented onto TransitionEvent.prototype, so `instanceof TransitionEvent`
  // (what the WPT event tests assert first) and `instanceof Event` both hold.
  globalThis.TransitionEvent = function(type, init) {
    init = init || {};
    var ev = new Event(String(type), {
      bubbles: init.bubbles !== undefined ? !!init.bubbles : true,
      cancelable: !!init.cancelable,
    });
    ev.propertyName = init.propertyName !== undefined ? String(init.propertyName) : '';
    ev.elapsedTime = init.elapsedTime !== undefined ? Number(init.elapsedTime) : 0;
    ev.pseudoElement = init.pseudoElement !== undefined ? String(init.pseudoElement) : '';
    Object.setPrototypeOf(ev, TransitionEvent.prototype);
    return ev;
  };
  globalThis.TransitionEvent.prototype = Object.create(Event.prototype, {
    constructor: { value: globalThis.TransitionEvent, writable: true, configurable: true },
  });

  // Host bridge: dispatch a transition* event at a node (from the layout tick's
  // harvested lifecycle events). `type` is one of transitionrun /
  // transitionstart / transitionend / transitioncancel.
  globalThis.__dispatchTransition = function(rawId, type, propertyName, elapsedTime) {
    var node = wrapNode(__reflectNode(String(rawId)));
    if (!node) { return false; }
    var ev = new globalThis.TransitionEvent(String(type), {
      propertyName: propertyName,
      elapsedTime: elapsedTime,
    });
    return node.dispatchEvent(ev);
  };

  // ---- UA-generated input events (touch / wheel) ----
  //
  // The passive-listener optimization, and the rule WPT's
  // `dom/events/non-cancelable-when-passive` pins: a UA-generated touch or wheel
  // event is cancelable **only if some non-passive listener for its type exists
  // on the propagation path**. If every listener is passive, nothing can call
  // preventDefault, so the UA marks the event non-cancelable (and is free to
  // scroll without consulting script). Only the DOM knows the listener set, so
  // the decision lives here rather than in the host.
  //
  // This applies to the UA input path only. A *script*-dispatched event keeps
  // whatever `cancelable` its constructor was given, passive listeners or not
  // (`generic-events-stay-cancelable`).
  function hasNonPassiveListener(node, type) {
    var path = [];
    var n = node;
    while (n) { path.push(n); n = n.parentNode; }
    if (path.length && path[path.length - 1].nodeType === 9 && globalThis.window) {
      path.push(globalThis.window);
    }
    var keys = ['c:' + type, 'b:' + type];
    for (var i = 0; i < path.length; i++) {
      var listeners = path[i].__listeners;
      if (!listeners) continue;
      for (var k = 0; k < keys.length; k++) {
        var l = listeners[keys[k]];
        if (!l) continue;
        for (var j = 0; j < l.length; j++) {
          if (!l[j].passive) return true;
        }
      }
    }
    return false;
  }

  // Host bridge: dispatch a touch event carrying one touch point. Per Touch
  // Events, `touches`/`targetTouches` list the points currently on the surface
  // (empty once the finger lifts), while `changedTouches` always carries the
  // point this event is about.
  globalThis.__dispatchTouch = function(rawId, type, x, y, identifier) {
    var node = wrapNode(__reflectNode(String(rawId)));
    if (!node) { return false; }
    type = String(type);
    var touch = new Touch({
      identifier: Number(identifier), target: node,
      clientX: Number(x), clientY: Number(y),
      pageX: Number(x), pageY: Number(y),
      screenX: Number(x), screenY: Number(y),
      force: (type === 'touchend' || type === 'touchcancel') ? 0 : 1,
    });
    var lifted = (type === 'touchend' || type === 'touchcancel');
    var active = lifted ? [] : [touch];
    var ev = new TouchEvent(type, {
      bubbles: true,
      cancelable: hasNonPassiveListener(node, type),
      touches: active,
      targetTouches: active,
      changedTouches: [touch],
    });
    return node.dispatchEvent(ev);
  };

  // Host bridge: dispatch a wheel input as both the standard `wheel` event and
  // the legacy `mousewheel` (WPT covers both). Each gets its own cancelable
  // decision, since the rule is per event type.
  globalThis.__dispatchWheel = function(rawId, x, y, deltaX, deltaY, deltaMode) {
    var node = wrapNode(__reflectNode(String(rawId)));
    if (!node) { return false; }
    var proceed = true;
    for (var i = 0; i < 2; i++) {
      var type = (i === 0) ? 'wheel' : 'mousewheel';
      var ev = new WheelEvent(type, {
        bubbles: true,
        cancelable: hasNonPassiveListener(node, type),
        clientX: Number(x), clientY: Number(y),
        screenX: Number(x), screenY: Number(y),
        deltaX: Number(deltaX), deltaY: Number(deltaY), deltaZ: 0,
        deltaMode: Number(deltaMode),
      });
      if (type === 'mousewheel') {
        // Legacy: wheelDelta is the inverse of deltaY, scaled by 120 per notch.
        ev.wheelDelta = -Number(deltaY) * 120;
        ev.wheelDeltaX = -Number(deltaX) * 120;
        ev.wheelDeltaY = -Number(deltaY) * 120;
      }
      if (!node.dispatchEvent(ev)) { proceed = false; }
    }
    return proceed;
  };

  // AnimationEvent (css-animations): the `@keyframes` twin of TransitionEvent.
  // Carries `animationName` (the @keyframes rule's name) rather than a property
  // name, plus `elapsedTime` and `pseudoElement`. Bubbles, not cancelable.
  // Prototype-chained exactly like TransitionEvent, for `instanceof`.
  globalThis.AnimationEvent = function(type, init) {
    init = init || {};
    var ev = new Event(String(type), {
      bubbles: init.bubbles !== undefined ? !!init.bubbles : true,
      cancelable: !!init.cancelable,
    });
    ev.animationName = init.animationName !== undefined ? String(init.animationName) : '';
    ev.elapsedTime = init.elapsedTime !== undefined ? Number(init.elapsedTime) : 0;
    ev.pseudoElement = init.pseudoElement !== undefined ? String(init.pseudoElement) : '';
    Object.setPrototypeOf(ev, AnimationEvent.prototype);
    return ev;
  };
  globalThis.AnimationEvent.prototype = Object.create(Event.prototype, {
    constructor: { value: globalThis.AnimationEvent, writable: true, configurable: true },
  });

  // Host bridge: dispatch an animation* event at a node (from the layout tick's
  // harvested lifecycle events). `type` is one of animationstart /
  // animationiteration / animationend / animationcancel.
  globalThis.__dispatchAnimation = function(rawId, type, animationName, elapsedTime) {
    var node = wrapNode(__reflectNode(String(rawId)));
    if (!node) { return false; }
    var ev = new globalThis.AnimationEvent(String(type), {
      animationName: animationName,
      elapsedTime: elapsedTime,
    });
    return node.dispatchEvent(ev);
  };

  // window.matchMedia (css-mediaqueries): a MediaQueryList over the host's
  // media-query evaluation. `.matches` / `.media` are LIVE (re-evaluated against
  // the current device on each access). `change` fires (addEventListener /
  // addListener / onchange) when the host calls
  // `Runtime::notify_media_features_changed` and the query's result flipped.
  // Note: a MQL with a change listener is retained for re-evaluation (a small
  // leak vs a real weak-ref registry); MQLs never listened to are not retained.
  (function() {
    var live = []; // `fire` closures for MQLs with a change listener / onchange
    function evalq(q) { return __matchMedia(q); }
    globalThis.matchMedia = function(query) {
      var q = String(query);
      var listeners = [];
      var last = evalq(q).charAt(0) === '1';
      var registered = false;
      var onchange = null;
      var mql = {
        get matches() { return evalq(q).charAt(0) === '1'; },
        get media() { var r = evalq(q); var nl = r.indexOf('\n'); return nl >= 0 ? r.slice(nl + 1) : ''; },
        addEventListener: function(type, cb) {
          if (type === 'change' && typeof cb === 'function') { listeners.push(cb); register(); }
        },
        removeEventListener: function(type, cb) {
          if (type === 'change') { var i = listeners.indexOf(cb); if (i >= 0) listeners.splice(i, 1); }
        },
        addListener: function(cb) { this.addEventListener('change', cb); },
        removeListener: function(cb) { this.removeEventListener('change', cb); },
        dispatchEvent: function() { return false; },
      };
      Object.defineProperty(mql, 'onchange', {
        configurable: true, enumerable: true,
        get: function() { return onchange; },
        set: function(v) { onchange = v; if (typeof v === 'function') register(); },
      });
      function register() { if (!registered) { registered = true; live.push(fire); } }
      function fire() {
        var now = evalq(q).charAt(0) === '1';
        if (now === last) { return; }
        last = now;
        var ev = { type: 'change', matches: now, media: mql.media, target: mql, currentTarget: mql };
        if (typeof onchange === 'function') { try { onchange.call(mql, ev); } catch (e) {} }
        var snap = listeners.slice();
        for (var i = 0; i < snap.length; i++) { try { snap[i].call(mql, ev); } catch (e) {} }
      }
      return mql;
    };
    // Re-evaluate all listened MediaQueryLists; fire `change` on those that
    // flipped. The host calls this after any device / preference change.
    globalThis.__reevaluateMediaQueries = function() {
      var snap = live.slice();
      for (var i = 0; i < snap.length; i++) { snap[i](); }
    };
  })();

  // window.frames is the window itself when there are no child browsing
  // contexts (the static-DOM harness has none).
  globalThis.frames = globalThis.window || globalThis;

  // Minimal CustomElementRegistry: define/get/getName/whenDefined/upgrade plus
  // a first customized-built-ins slice over the HTML interface table.
  (function() {
    function CustomElementRegistry() {}
    var pending = Object.create(null);
    var definitionRunning = false;
    CustomElementRegistry.prototype.define = function(name, ctor, options) {
      if (typeof ctor !== 'function') {
        throw new TypeError('constructor must be a function');
      }
      name = String(name);
      if (!isValidCustomElementName(name)) {
        throw customElementSyntaxError(name);
      }
      if (name in customElementDefinitions || customElementDefinitionsByCtor.has(ctor)) {
        throw new (globalThis.DOMException || TypeError)('name already defined', 'NotSupportedError');
      }
      if (definitionRunning) {
        throw new (globalThis.DOMException || TypeError)('definition already running', 'NotSupportedError');
      }
      definitionRunning = true;
      var def;
      var localName;
      var isCustomizedBuiltIn = false;
      try {
        var prototype = ctor.prototype;
        if (!prototype || (typeof prototype !== 'object' && typeof prototype !== 'function')) {
          throw new TypeError('constructor prototype must be an object');
        }
        var callbacks = {
          connectedCallback: customElementCallback(prototype, 'connectedCallback'),
          disconnectedCallback: customElementCallback(prototype, 'disconnectedCallback'),
          adoptedCallback: customElementCallback(prototype, 'adoptedCallback'),
          attributeChangedCallback: customElementCallback(prototype, 'attributeChangedCallback')
        };
        var observedAttributes = [];
        if (callbacks.attributeChangedCallback) {
          observedAttributes = toDomStringSequence(ctor.observedAttributes);
          for (var oi = 0; oi < observedAttributes.length; oi++) {
            observedAttributes[oi] = String(observedAttributes[oi]).toLowerCase();
          }
        }
        toDomStringSequence(ctor.disabledFeatures);
        var formAssociated = !!ctor.formAssociated;
        if (formAssociated) {
          callbacks.formAssociatedCallback = customElementCallback(prototype, 'formAssociatedCallback');
          callbacks.formResetCallback = customElementCallback(prototype, 'formResetCallback');
          callbacks.formDisabledCallback = customElementCallback(prototype, 'formDisabledCallback');
          callbacks.formStateRestoreCallback = customElementCallback(prototype, 'formStateRestoreCallback');
        }
        localName = name;
        if (options && options.extends !== undefined) {
          localName = String(options.extends).toLowerCase();
          validateName(localName);
          if (localName.indexOf('-') !== -1 || !elementSubclassProto[localName.toUpperCase()]) {
            throw new (globalThis.DOMException || TypeError)('unknown built-in extension target', 'NotSupportedError');
          }
          isCustomizedBuiltIn = true;
        }
        def = {
          name: name,
          localName: localName,
          ctor: ctor,
          customizedBuiltIn: isCustomizedBuiltIn,
          observedAttributes: observedAttributes,
          callbacks: callbacks,
          formAssociated: formAssociated
        };
      } finally {
        definitionRunning = false;
      }
      customElementDefinitions[name] = def;
      if (isCustomizedBuiltIn) {
        customizedBuiltInDefinitions[customElementKey(localName, name)] = def;
      } else {
        autonomousCustomElementDefinitions[localName.toUpperCase()] = def;
      }
      customElementDefinitionsByCtor.set(ctor, def);
      if (pending[name]) { pending[name].resolve(ctor); delete pending[name]; }
      upgradeCustomElementTree(document);
    };
    CustomElementRegistry.prototype.get = function(name) {
      var def = customElementDefinitions[name];
      return def ? def.ctor : undefined;
    };
    CustomElementRegistry.prototype.getName = function(ctor) {
      if (typeof ctor !== 'function') throw new TypeError('constructor must be a function');
      var def = customElementDefinitionsByCtor.get(ctor);
      return def ? def.name : null;
    };
    CustomElementRegistry.prototype.whenDefined = function(name) {
      name = String(name);
      if (!isValidCustomElementName(name)) {
        return Promise.reject(customElementSyntaxError(name));
      }
      if (name in customElementDefinitions) return Promise.resolve(customElementDefinitions[name].ctor);
      if (pending[name]) return pending[name].promise;
      var slot = {};
      slot.promise = new Promise(function(resolve) { slot.resolve = resolve; });
      pending[name] = slot;
      return slot.promise;
    };
    CustomElementRegistry.prototype.upgrade = function(root) { upgradeCustomElementTree(root); };
    globalThis.CustomElementRegistry = CustomElementRegistry;
    globalThis.customElements = new CustomElementRegistry();
  })();

  // ---- Event-handler IDL attributes (HTML §event-handler-idl-attributes) ----
  //
  // `el.onclick = fn`, `window.onload = fn`, `document.body.onload = fn`: a
  // getter/setter pair per handler name managing a single event listener. The
  // listener is a stable wrapper registered once (on the first non-null
  // assignment) that calls the *current* handler value, so reassigning the
  // handler keeps its listener registration order (per spec, an event handler
  // interleaves with addEventListener listeners by first-set position) and
  // setting it to null makes the wrapper a no-op. Only the IDL attribute is
  // implemented; the content-attribute form (`<body onload="...">` parsed as a
  // function body) is a separate compile step, deferred.
  (function() {
    function defineHandler(proto, type, resolveTarget) {
      Object.defineProperty(proto, 'on' + type, {
        configurable: true,
        get: function() {
          var t = resolveTarget ? resolveTarget(this) : this;
          return (t && t.__handlers && t.__handlers[type]) || null;
        },
        set: function(v) {
          var t = resolveTarget ? resolveTarget(this) : this;
          if (!t) return;
          if (!t.__handlers) t.__handlers = {};
          t.__handlers[type] = (typeof v === 'function') ? v : null;
          if (t.__handlers[type] && !(t.__handlerWrappers && t.__handlerWrappers[type])) {
            if (!t.__handlerWrappers) t.__handlerWrappers = {};
            var wrapper = function(event) {
              var h = t.__handlers[type];
              if (typeof h === 'function') { return h.call(this, event); }
            };
            t.__handlerWrappers[type] = wrapper;
            t.addEventListener(type, wrapper);
          }
        },
      });
    }
    // WindowEventHandlers reflect from <body>/<frameset> onto the Window; on any
    // other element (or on the Window itself) they are ordinary node handlers.
    function bodyReflectsToWindow(node) {
      var nm = node.nodeName;
      if (nm === 'BODY' || nm === 'FRAMESET') { return globalThis.window || globalThis; }
      return node;
    }
    var WINDOW_REFLECTING = [
      'load', 'unload', 'resize', 'scroll', 'blur', 'focus', 'error',
      'hashchange', 'popstate', 'beforeunload', 'pagehide', 'pageshow',
      'message', 'messageerror', 'offline', 'online', 'storage', 'languagechange',
    ];
    var ELEMENT_HANDLERS = [
      'click', 'dblclick', 'auxclick', 'contextmenu',
      'mousedown', 'mouseup', 'mousemove', 'mouseover', 'mouseout', 'mouseenter', 'mouseleave',
      'pointerdown', 'pointerup', 'pointermove', 'pointerover', 'pointerout',
      'pointerenter', 'pointerleave', 'pointercancel', 'gotpointercapture', 'lostpointercapture',
      'keydown', 'keyup', 'keypress',
      // NB: on* touch handlers (ontouchstart, …) are deliberately omitted. Per
      // Touch Events, those IDL attributes exist only when "expose legacy touch
      // event APIs" is true (a touch-capable device); genet is not one, and
      // `'ontouchstart' in document` gates real WPT branches
      // (Document-createEvent-touchevent). Touch listeners still work via
      // addEventListener; only the on* reflection is gated. The TouchEvent
      // *interface* object stays defined (H8b).
      'wheel',
      'input', 'change', 'beforeinput', 'submit', 'reset', 'select',
      'focusin', 'focusout',
      'drag', 'dragstart', 'dragend', 'dragenter', 'dragover', 'dragleave', 'drop',
      'copy', 'cut', 'paste',
      'selectionchange', 'selectstart',
      'animationstart', 'animationiteration', 'animationend', 'animationcancel',
      'transitionstart', 'transitionrun', 'transitionend', 'transitioncancel',
      'toggle',
    ];
    for (var i = 0; i < ELEMENT_HANDLERS.length; i++) {
      defineHandler(Node.prototype, ELEMENT_HANDLERS[i], null);
    }
    for (var j = 0; j < WINDOW_REFLECTING.length; j++) {
      defineHandler(Node.prototype, WINDOW_REFLECTING[j], bodyReflectsToWindow);
    }
    // The Window: every handler registers on the global EventTarget seam
    // (globalThis.addEventListener delegates to it).
    var all = ELEMENT_HANDLERS.concat(WINDOW_REFLECTING);
    for (var k = 0; k < all.length; k++) {
      defineHandler(globalThis, all[k], null);
    }
  })();

  // -- DOMParser / XMLSerializer ---------------------------------------------
  //
  // `parseFromString` builds a whole new Document in the same arena through the
  // static tier's parsers: html5ever for `text/html`, xml5ever for the four XML
  // types. A failed XML parse yields the spec's `parsererror` document rather
  // than throwing, which is what callers (and `responseXML`) branch on.
  var XML_TYPES = {
    'text/xml': 1, 'application/xml': 1,
    'application/xhtml+xml': 1, 'image/svg+xml': 1
  };
  function DOMParser() {
    if (!(this instanceof DOMParser)) return new DOMParser();
  }
  DOMParser.prototype.parseFromString = function(source, type) {
    type = String(type);
    var isHtml = type === 'text/html';
    if (!isHtml && !XML_TYPES[type]) {
      throw new TypeError("DOMParser.parseFromString: unsupported type '" + type + "'");
    }
    var ref = __parseDocument(String(source), isHtml ? 'html' : 'xml');
    if (ref === undefined || ref === null) {
      // Parse failure (XML only; the HTML parser never fails).
      var bad = wrapNode(__createDocument());
      bad.__isHtml = false;
      var err = bad.createElementNS('http://www.mozilla.org/newlayout/xml/parseerror.xml', 'parsererror');
      err.textContent = 'XML parse error';
      bad.appendChild(err);
      return bad;
    }
    var doc = wrapNode(ref);
    doc.__isHtml = isHtml;
    return doc;
  };
  globalThis.DOMParser = DOMParser;

  function XMLSerializer() {
    if (!(this instanceof XMLSerializer)) return new XMLSerializer();
  }
  function xmlEscape(text, inAttribute) {
    var out = String(text).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    return inAttribute ? out.replace(/"/g, '&quot;') : out;
  }
  // XML serialization (DOM Parsing "fragment serializing algorithm"), without
  // namespace-prefix invention: an element is emitted under the prefix it
  // already carries, plus the `xmlns` declaration its namespace needs when the
  // parent did not already supply it.
  function serializeXml(node, parentNs) {
    switch (node.nodeType) {
      case 1: {
        var ns = node.namespaceURI;
        var name = node.prefix ? node.prefix + ':' + node.localName : node.localName;
        var out = '<' + name;
        if (ns && ns !== parentNs && !node.prefix) out += ' xmlns="' + xmlEscape(ns, true) + '"';
        var attrs = node.attributes;
        for (var i = 0; i < attrs.length; i++) {
          var a = attrs.item(i);
          out += ' ' + a.name + '="' + xmlEscape(a.value, true) + '"';
        }
        var kids = rawChildNodes(node);
        if (!kids.length) return out + '/>';
        out += '>';
        for (var k = 0; k < kids.length; k++) out += serializeXml(kids[k], ns);
        return out + '</' + name + '>';
      }
      case 3: return xmlEscape(node.data, false);
      case 4: return '<![CDATA[' + node.data + ']]>';
      case 7: return '<?' + node.target + ' ' + node.data + '?>';
      case 8: return '<!--' + node.data + '-->';
      case 10: return '<!DOCTYPE ' + node.name + '>';
      case 9: case 11: {
        var body = '';
        var children = rawChildNodes(node);
        for (var j = 0; j < children.length; j++) body += serializeXml(children[j], null);
        return body;
      }
      default: return '';
    }
  }
  XMLSerializer.prototype.serializeToString = function(node) {
    if (!node) throw new TypeError('XMLSerializer.serializeToString: not a Node');
    return serializeXml(node, null);
  };
  globalThis.XMLSerializer = XMLSerializer;
})();
