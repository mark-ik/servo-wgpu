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
  // Whether `receiver` is this context's WindowProxy or its Window - the two
  // answers a `postMessage` receiver may legitimately be.
  define(globals, '__frameIsWindowView', { value: function (receiver, global, view) {
    return receiver === global || (view !== undefined && receiver === view);
  }});

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
  // [LegacyUnforgeable], so defined on the Window rather than through the
  // browsing context's WindowProxy. See the note in SELF_WINDOW_BOOTSTRAP.
  var unforgeable = typeof __windowProxyGlobal === 'object' && __windowProxyGlobal
    ? __windowProxyGlobal : globals;
  define(unforgeable, 'top', {
    enumerable: true, configurable: false,
    get: function () { return relation('top'); }
  });
  define(globals, 'frameElement', {
    enumerable: true, configurable: true,
    get: function () { return relation('frameElement'); }
  });
  globalThis.opener = null;
  // A live browsing context; `__discardBrowsingContext` flips this when the
  // context is destroyed and a parent is still holding this global.
  define(globals, 'closed', { value: false, writable: false, enumerable: true, configurable: true });
  // Browsing-context teardown, run in the realm being destroyed and only from
  // the host. `closed` is the whole of what a discarded context reports
  // differently: its `document` deliberately keeps answering (see the getter in
  // bootstrap.js). Its timers and animation callbacks are cancelled separately,
  // so nothing already scheduled in it runs afterwards either.
  define(globals, '__discardBrowsingContext', { value: function () {
    define(globals, 'closed', { value: true, writable: false, enumerable: true, configurable: true });
    delete globals.__discardBrowsingContext;
  }});
  globalThis.postMessage = __realmPostMessage;
})();
