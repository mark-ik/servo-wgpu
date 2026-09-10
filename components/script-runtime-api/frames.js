(function () {
  'use strict';
  var serializeRecord = __scSerializeRecord;
  var deserializeRecord = __scDeserializeRecord;
  var Message = MessageEvent;
  var Port = MessagePort;
  var schedule = setTimeout;
  var dispatch = dispatchEvent;
  var trace = __traceProtocol;
  var nextMessage = 0;
  var parseURL = URL.parse;
  var globals = globalThis;
  var define = Object.defineProperty;
  define(globals, '__frameSameReceiver', { value: function(receiver, global) {
    return receiver === undefined || receiver === null ? undefined : receiver === global;
  }});
  // These non-replaceable functions retain trusted codecs in their closures.
  // Authored code never receives a foreign serialization record.
  define(globals, '__frameNormalize', { value: function(options) {
    var origin = '/';
    if (typeof options === 'string') origin = options;
    else if (options && options.targetOrigin !== undefined) origin = String(options.targetOrigin);
    if (origin !== '*' && origin !== '/') {
      var parsed = parseURL(origin);
      if (!parsed) throw new DOMException('Invalid target origin', 'SyntaxError');
      origin = parsed.origin;
    }
    return origin;
  }});
  define(globals, '__frameSerialize', { value: function(message, options, transfer) {
    if (transfer === undefined && options && typeof options === 'object') transfer = options.transfer;
    var ports = [];
    if (transfer) for (var i = 0; i < transfer.length; i++) if (transfer[i] instanceof Port) ports.push(transfer[i]);
    return serializeRecord({data:message, ports:ports}, transfer);
  }});
  define(globals, '__frameDeliver', { value: function(record, source, origin) {
    var envelope = deserializeRecord(record);
    var id = ++nextMessage;
    trace('post_message', 'enqueue', id);
    schedule(function() {
      trace('post_message', 'deliver', id);
      var event = new Message('message', {ports:envelope.ports, source:source, origin:origin});
      define(event, 'data', {value:envelope.data, writable:true, enumerable:true, configurable:true});
      dispatch(event);
    }, 0);
  }});
  var views = new Map();
  var property = __windowProperty;
  var post = __postToWindow;
  var create = Object.create, apply = Reflect.apply;
  var SecurityException = DOMException;
  var getView = Map.prototype.get, setView = Map.prototype.set;
  var symbols = [Symbol.toStringTag, Symbol.hasInstance, Symbol.isConcatSpreadable];
  var windowKeys = ['window', 'self', 'location', 'close', 'closed', 'focus', 'blur', 'frames', 'length', 'top', 'opener', 'parent', 'postMessage'];
  function denied() { throw new SecurityException('Cross-origin window access is denied', 'SecurityError'); }
  function dataDescriptor(value, enumerable) {
    var descriptor = create(null);
    descriptor.value = value;
    descriptor.writable = false;
    descriptor.enumerable = !!enumerable;
    descriptor.configurable = true;
    return descriptor;
  }
  function fallback(key) {
    if (key === 'then' || key === symbols[0] || key === symbols[1] || key === symbols[2]) return dataDescriptor(undefined, false);
    return denied();
  }
  function keysWithFallback(keys) {
    keys[keys.length] = 'then';
    for (var i = 0; i < symbols.length; i++) keys[keys.length] = symbols[i];
    return keys;
  }
  function handlerFor(descriptor, keys) {
    var handler = create(null);
    handler.getOwnPropertyDescriptor = function (_, key) { return descriptor(key); };
    handler.has = function (_, key) { return descriptor(key) !== undefined; };
    handler.get = function (_, key, receiver) {
      var desc = descriptor(key);
      if ('value' in desc) return desc.value;
      if (!desc.get) return denied();
      return apply(desc.get, receiver, []);
    };
    handler.set = function (_, key, value, receiver) {
      var desc = descriptor(key);
      if (!desc.set) return denied();
      apply(desc.set, receiver, [value]);
      return true;
    };
    handler.defineProperty = denied;
    handler.deleteProperty = denied;
    handler.ownKeys = keys;
    handler.getPrototypeOf = function () { return null; };
    handler.setPrototypeOf = function (_, value) { return value === null; };
    handler.isExtensible = function () { return true; };
    handler.preventExtensions = function () { return false; };
    return handler;
  }
  globalThis.__makeCrossOriginWindow = function (id) {
    var cached = apply(getView, views, [id]);
    if (cached) return cached;
    var descriptors = create(null);
    // Navigation remains the host's separate operation. These accessors expose
    // the prescribed shapes but retain the existing refusal when invoked.
    var href = create(null);
    href.get = undefined;
    href.set = function (value) { return denied(); };
    href.enumerable = false;
    href.configurable = true;
    var replace = dataDescriptor(function replace(value) { return denied(); }, false);
    var location = new Proxy(create(null), handlerFor(function (key) {
      if (key === 'href') return href;
      if (key === 'replace') return replace;
      return fallback(key);
    }, function () { return keysWithFallback(['href', 'replace']); }));
    function read(key) {
      if (key === 'location') return location;
      var value = property(String(id), key);
      if (value === '__security_error__') return denied();
      return value;
    }
    function descriptor(key) {
      if (typeof key !== 'string') return fallback(key);
      if (descriptors[key]) return descriptors[key];
      var method = key === 'close' || key === 'focus' || key === 'blur' || key === 'postMessage';
      if (method) {
        var fn = key === 'postMessage' ? function postMessage(message) { return post(id, message, arguments[1], arguments[2]); } : function () {};
        return (descriptors[key] = dataDescriptor(fn, false));
      }
      for (var i = 0; i < windowKeys.length; i++) {
        if (key !== windowKeys[i]) continue;
        var accessor = create(null);
        accessor.get = function () { return read(key); };
        accessor.set = key === 'location' ? function (value) { return denied(); } : undefined;
        accessor.enumerable = false;
        accessor.configurable = true;
        return (descriptors[key] = accessor);
      }
      // Only canonical array indexes enter the indexed-property path.
      var index = Number(key);
      if (index >= 0 && index < 4294967295 && String(index) === key && Math.floor(index) === index) {
        return dataDescriptor(read(key), true);
      }
      return fallback(key);
    }
    var proxy = new Proxy(create(null), handlerFor(descriptor, function () {
      var keys = [], length = read('length');
      for (var i = 0; i < length; i++) keys[keys.length] = String(i);
      for (var j = 0; j < windowKeys.length; j++) keys[keys.length] = windowKeys[j];
      return keysWithFallback(keys);
    }));
    apply(setView, views, [id, proxy]);
    return proxy;
  };
  var relation = __windowRelation;
  define(globals, 'parent', {
    enumerable: true, configurable: true,
    get: function () { return relation('parent'); },
    // WebIDL [Replaceable]: assignment creates an ordinary own data property.
    set: function (value) {
      define(this, 'parent', {
        value: value, writable: true, enumerable: true, configurable: true
      });
    }
  });
  define(globals, 'top', {
    enumerable: true, configurable: false,
    get: function () { return relation('top'); }
  });
  define(globals, 'frameElement', {
    enumerable: true, configurable: true,
    get: function () { return relation('frameElement'); }
  });
  globalThis.opener = null;
  globalThis.postMessage = __realmPostMessage;
})();
