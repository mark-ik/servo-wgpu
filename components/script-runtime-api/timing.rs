/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

//! `performance`: High Resolution Time, User Timing and the Performance Timeline.
//!
//! A pure JS bootstrap with no native sink. The clock is the **timers' virtual
//! clock** (`__virtualNow`, from the event-loop bootstrap), not wall time, so a
//! harness run under the drive loop's virtual clock stays deterministic; a
//! per-read sub-tick keeps `now()` strictly increasing and positive, which
//! hr-time requires. `timeOrigin` is the engine's `Date.now()` at install, which
//! is what the hr-time tests compare against.
//!
//! `PerformanceObserver` records are delivered as a task on the drive loop (a
//! zero-delay timer), so a callback never runs inside `mark()`.
//!
//! Absent: resource, navigation, paint, longtask and element entry types — those
//! need a network and a rendering timeline the scripted tier does not report.
//! `performance.timing` / `performance.navigation` are present as the legacy
//! zero-valued shapes `performance.toJSON()` is specified to carry.

use script_engine_api::ScriptEngine;

pub(crate) fn install_timing_surface<E: ScriptEngine>(engine: &mut E) -> Result<(), E::Error> {
    engine.eval(TIMING_BOOTSTRAP)?;
    Ok(())
}

const TIMING_BOOTSTRAP: &str = r#"
(function() {
  var timeOrigin = Date.now();
  // A strictly increasing sub-tick on top of the timers' clock: hr-time requires
  // now() > 0 and a non-negative difference between consecutive reads, which a
  // clock that only moves when a timer fires cannot give.
  var subTick = 0;
  function now() {
    subTick += 0.001;
    return globalThis.__virtualNow() + subTick;
  }

  function defineTag(ctor, name) {
    if (typeof Symbol !== 'undefined' && Symbol.toStringTag) {
      ctor.prototype[Symbol.toStringTag] = name;
    }
  }

  // ---- PerformanceEntry ----
  function PerformanceEntry(name, entryType, startTime, duration) {
    this.name = String(name);
    this.entryType = entryType;
    this.startTime = startTime;
    this.duration = duration;
  }
  PerformanceEntry.prototype.toJSON = function() {
    return { name: this.name, entryType: this.entryType,
             startTime: this.startTime, duration: this.duration };
  };
  defineTag(PerformanceEntry, 'PerformanceEntry');
  globalThis.PerformanceEntry = PerformanceEntry;

  function PerformanceMark(name, options) {
    var startTime = now();
    var detail = null;
    if (options !== undefined && options !== null) {
      if (options.startTime !== undefined) {
        startTime = Number(options.startTime);
        if (!(startTime >= 0)) throw new TypeError("mark startTime must be non-negative");
      }
      if (options.detail !== undefined && options.detail !== null) {
        detail = globalThis.__structuredClone(options.detail);
      }
    }
    PerformanceEntry.call(this, name, 'mark', startTime, 0);
    this.detail = detail;
  }
  PerformanceMark.prototype = Object.create(PerformanceEntry.prototype);
  PerformanceMark.prototype.constructor = PerformanceMark;
  PerformanceMark.prototype.toJSON = function() {
    var j = PerformanceEntry.prototype.toJSON.call(this);
    j.detail = this.detail;
    return j;
  };
  defineTag(PerformanceMark, 'PerformanceMark');
  globalThis.PerformanceMark = PerformanceMark;

  function PerformanceMeasure(name, startTime, duration, detail) {
    PerformanceEntry.call(this, name, 'measure', startTime, duration);
    this.detail = detail === undefined ? null : detail;
  }
  PerformanceMeasure.prototype = Object.create(PerformanceEntry.prototype);
  PerformanceMeasure.prototype.constructor = PerformanceMeasure;
  PerformanceMeasure.prototype.toJSON = function() {
    var j = PerformanceEntry.prototype.toJSON.call(this);
    j.detail = this.detail;
    return j;
  };
  defineTag(PerformanceMeasure, 'PerformanceMeasure');
  globalThis.PerformanceMeasure = PerformanceMeasure;

  // ---- the timeline ----
  var entries = [];               // in insertion order; getEntries sorts by startTime
  var observers = [];             // registered PerformanceObservers
  var deliveryScheduled = false;

  function sortedCopy(list) {
    var out = list.slice();
    // Stable sort by startTime; insertion order breaks ties.
    out.sort(function(a, b) { return a.startTime - b.startTime; });
    return out;
  }

  function scheduleDelivery() {
    if (deliveryScheduled) return;
    deliveryScheduled = true;
    // The performance timeline task source: never run a callback synchronously
    // inside mark()/measure().
    setTimeout(function() {
      deliveryScheduled = false;
      var snapshot = observers.slice();
      for (var i = 0; i < snapshot.length; i++) {
        var o = snapshot[i];
        if (o._buffer.length === 0) continue;
        var records = o._buffer;
        o._buffer = [];
        var dropped = o._dropped; o._dropped = 0;
        try { o._callback.call(o, new PerformanceObserverEntryList(records), o, { droppedEntriesCount: dropped }); }
        catch (ex) { globalThis.__reportListenerException(ex); }
      }
    }, 0);
  }

  function addEntry(entry) {
    entries.push(entry);
    for (var i = 0; i < observers.length; i++) {
      if (observers[i]._types[entry.entryType]) observers[i]._buffer.push(entry);
    }
    scheduleDelivery();
    return entry;
  }

  function filterEntries(list, name, type) {
    var out = [];
    for (var i = 0; i < list.length; i++) {
      var e = list[i];
      if (name !== undefined && e.name !== name) continue;
      if (type !== undefined && type !== null && e.entryType !== type) continue;
      out.push(e);
    }
    return out;
  }

  // ---- PerformanceObserverEntryList ----
  function PerformanceObserverEntryList(list) { this._l = sortedCopy(list); }
  PerformanceObserverEntryList.prototype.getEntries = function() { return this._l.slice(); };
  PerformanceObserverEntryList.prototype.getEntriesByType = function(type) {
    return filterEntries(this._l, undefined, String(type));
  };
  PerformanceObserverEntryList.prototype.getEntriesByName = function(name, type) {
    return filterEntries(this._l, String(name), type === undefined ? undefined : String(type));
  };
  defineTag(PerformanceObserverEntryList, 'PerformanceObserverEntryList');
  globalThis.PerformanceObserverEntryList = PerformanceObserverEntryList;

  // ---- PerformanceObserver ----
  var SUPPORTED = ['mark', 'measure'];
  function PerformanceObserver(callback) {
    if (typeof callback !== 'function') {
      throw new TypeError("PerformanceObserver requires a callback");
    }
    this._callback = callback;
    this._buffer = [];
    this._types = {};
    this._dropped = 0;
    this._mode = null;             // 'entryTypes' | 'type', once chosen it is fixed
  }
  PerformanceObserver.prototype.observe = function(options) {
    options = options || {};
    var hasList = options.entryTypes !== undefined;
    var hasType = options.type !== undefined;
    if (hasList && hasType) {
      throw new TypeError("observe(): entryTypes and type are mutually exclusive");
    }
    if (!hasList && !hasType) {
      throw new TypeError("observe(): either entryTypes or type is required");
    }
    if (hasList && this._mode === 'type') {
      throw new DOMException("This observer has performed observe({type}).", "InvalidModificationError");
    }
    if (hasType && this._mode === 'entryTypes') {
      throw new DOMException("This observer has performed observe({entryTypes}).", "InvalidModificationError");
    }
    if (hasList) {
      // A fresh entryTypes list replaces the previous one wholesale.
      this._mode = 'entryTypes';
      this._types = {};
      var list = options.entryTypes;
      if (typeof list !== 'object' || typeof list.length !== 'number') {
        throw new TypeError("observe(): entryTypes is not a sequence");
      }
      var any = false;
      for (var i = 0; i < list.length; i++) {
        if (SUPPORTED.indexOf(String(list[i])) !== -1) { this._types[String(list[i])] = true; any = true; }
      }
      if (!any) { unregister(this); return; }   // nothing supported: no-op, no throw
    } else {
      this._mode = 'type';
      var type = String(options.type);
      if (SUPPORTED.indexOf(type) === -1) return; // unsupported type: no-op, no throw
      this._types[type] = true;
      if (options.buffered) {
        var buffered = filterEntries(entries, undefined, type);
        for (var j = 0; j < buffered.length; j++) this._buffer.push(buffered[j]);
        if (buffered.length) scheduleDelivery();
      }
    }
    if (observers.indexOf(this) === -1) observers.push(this);
  };
  function unregister(o) {
    var i = observers.indexOf(o);
    if (i !== -1) observers.splice(i, 1);
  }
  PerformanceObserver.prototype.disconnect = function() {
    unregister(this);
    this._buffer = [];
    this._types = {};
    this._mode = null;
  };
  PerformanceObserver.prototype.takeRecords = function() {
    var out = sortedCopy(this._buffer);
    this._buffer = [];
    return out;
  };
  Object.defineProperty(PerformanceObserver, 'supportedEntryTypes', {
    configurable: true,
    get: function() { return Object.freeze(SUPPORTED.slice()); }
  });
  defineTag(PerformanceObserver, 'PerformanceObserver');
  globalThis.PerformanceObserver = PerformanceObserver;

  // ---- legacy PerformanceTiming / PerformanceNavigation ----
  var TIMING_KEYS = ['navigationStart', 'unloadEventStart', 'unloadEventEnd', 'redirectStart',
    'redirectEnd', 'fetchStart', 'domainLookupStart', 'domainLookupEnd', 'connectStart',
    'connectEnd', 'secureConnectionStart', 'requestStart', 'responseStart', 'responseEnd',
    'domLoading', 'domInteractive', 'domContentLoadedEventStart', 'domContentLoadedEventEnd',
    'domComplete', 'loadEventStart', 'loadEventEnd'];
  function PerformanceTiming() {
    for (var i = 0; i < TIMING_KEYS.length; i++) { this[TIMING_KEYS[i]] = 0; }
    this.navigationStart = Math.floor(timeOrigin);
  }
  PerformanceTiming.prototype.toJSON = function() {
    var j = {};
    for (var i = 0; i < TIMING_KEYS.length; i++) { j[TIMING_KEYS[i]] = this[TIMING_KEYS[i]]; }
    return j;
  };
  defineTag(PerformanceTiming, 'PerformanceTiming');
  globalThis.PerformanceTiming = PerformanceTiming;

  function PerformanceNavigation() { this.type = 0; this.redirectCount = 0; }
  PerformanceNavigation.prototype.TYPE_NAVIGATE = PerformanceNavigation.TYPE_NAVIGATE = 0;
  PerformanceNavigation.prototype.TYPE_RELOAD = PerformanceNavigation.TYPE_RELOAD = 1;
  PerformanceNavigation.prototype.TYPE_BACK_FORWARD = PerformanceNavigation.TYPE_BACK_FORWARD = 2;
  PerformanceNavigation.prototype.TYPE_RESERVED = PerformanceNavigation.TYPE_RESERVED = 255;
  PerformanceNavigation.prototype.toJSON = function() {
    return { type: this.type, redirectCount: this.redirectCount };
  };
  defineTag(PerformanceNavigation, 'PerformanceNavigation');
  globalThis.PerformanceNavigation = PerformanceNavigation;

  // ---- Performance ----
  var timing = new PerformanceTiming();
  var navigation = new PerformanceNavigation();

  function markNameIsReserved(name) {
    return TIMING_KEYS.indexOf(name) !== -1;
  }
  function convertMarkToTimestamp(mark) {
    if (typeof mark === 'string') {
      if (markNameIsReserved(mark)) {
        // A navigation-timing attribute name resolves to its (zero) value; a
        // zero one is not a valid measure endpoint.
        throw new DOMException("'" + mark + "' has a value of zero.", "SyntaxError");
      }
      var found = filterEntries(entries, mark, 'mark');
      if (found.length === 0) {
        throw new DOMException("The mark '" + mark + "' does not exist.", "SyntaxError");
      }
      return found[found.length - 1].startTime;
    }
    var n = Number(mark);
    if (!(n >= 0)) throw new TypeError("A timestamp must be non-negative");
    return n;
  }

  function Performance() { EventTarget.call(this); }
  Performance.prototype = Object.create(EventTarget.prototype);
  Performance.prototype.constructor = Performance;
  Performance.prototype.now = function() { return now(); };
  Object.defineProperty(Performance.prototype, 'timeOrigin', {
    configurable: true, enumerable: true, get: function() { return timeOrigin; }
  });
  Object.defineProperty(Performance.prototype, 'timing', {
    configurable: true, enumerable: true, get: function() { return timing; }
  });
  Object.defineProperty(Performance.prototype, 'navigation', {
    configurable: true, enumerable: true, get: function() { return navigation; }
  });
  Performance.prototype.toJSON = function() {
    return { timeOrigin: timeOrigin, timing: timing.toJSON(), navigation: navigation.toJSON() };
  };
  Performance.prototype.getEntries = function() { return sortedCopy(entries); };
  Performance.prototype.getEntriesByType = function(type) {
    return filterEntries(sortedCopy(entries), undefined, String(type));
  };
  Performance.prototype.getEntriesByName = function(name, type) {
    return filterEntries(sortedCopy(entries), String(name), type === undefined || type === null ? undefined : String(type));
  };
  Performance.prototype.mark = function(name, options) {
    if (arguments.length === 0) throw new TypeError("mark requires a name");
    name = String(name);
    if (markNameIsReserved(name)) {
      throw new DOMException("'" + name + "' is a navigation timing attribute.", "SyntaxError");
    }
    return addEntry(new PerformanceMark(name, options));
  };
  Performance.prototype.measure = function(name, startOrOptions, endMark) {
    if (arguments.length === 0) throw new TypeError("measure requires a name");
    name = String(name);
    var start, end, detail = null;
    var isDict = startOrOptions !== null && typeof startOrOptions === 'object';
    if (isDict) {
      if (endMark !== undefined) {
        throw new TypeError("measure(): endMark cannot accompany a measure-options dictionary");
      }
      var hasStart = startOrOptions.start !== undefined;
      var hasEnd = startOrOptions.end !== undefined;
      var hasDur = startOrOptions.duration !== undefined;
      if (hasDur && !(hasStart || hasEnd)) {
        throw new TypeError("measure(): duration needs a start or an end");
      }
      if (hasStart && hasEnd && hasDur) {
        throw new TypeError("measure(): start, end and duration are over-determined");
      }
      if (startOrOptions.detail !== undefined && startOrOptions.detail !== null) {
        detail = globalThis.__structuredClone(startOrOptions.detail);
      }
      if (hasStart) start = convertMarkToTimestamp(startOrOptions.start);
      if (hasEnd) end = convertMarkToTimestamp(startOrOptions.end);
      if (hasDur) {
        var d = convertMarkToTimestamp(startOrOptions.duration);
        if (start === undefined) start = end - d; else end = start + d;
      }
      if (start === undefined) start = 0;
      if (end === undefined) end = now();
    } else {
      start = (startOrOptions === undefined) ? 0 : convertMarkToTimestamp(startOrOptions);
      end = (endMark === undefined) ? now() : convertMarkToTimestamp(endMark);
    }
    return addEntry(new PerformanceMeasure(name, start, end - start, detail));
  };
  Performance.prototype.clearMarks = function(name) {
    entries = filterOut(entries, 'mark', name);
  };
  Performance.prototype.clearMeasures = function(name) {
    entries = filterOut(entries, 'measure', name);
  };
  function filterOut(list, type, name) {
    var out = [];
    for (var i = 0; i < list.length; i++) {
      var e = list[i];
      if (e.entryType === type && (name === undefined || e.name === String(name))) continue;
      out.push(e);
    }
    return out;
  }
  // No resource timeline yet; the buffer controls are inert but present so a
  // feature-detecting test does not fault.
  Performance.prototype.clearResourceTimings = function() {};
  Performance.prototype.setResourceTimingBufferSize = function() {};
  defineTag(Performance, 'Performance');
  globalThis.Performance = Performance;
  globalThis.performance = new Performance();
})();
"#;
