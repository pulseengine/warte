// warte idle scene (REQ-DASH-018): after a while without input, a
// translucent seasonal layer drifts over the dashboard, which stays readable
// underneath. Any key, click, touch or mouse move takes it away.
//
// Everything is worked out in the browser. warte stores no location: the
// browser's time zone stands in for it (its city's coordinates, or the UTC
// offset), which gives the hemisphere, the sun times and the weather. The
// theme changes every month; calendar moments, the sky (sun, moon, stars),
// the weather and a few facts from the ground station's own API join in.
// Off with `?calm` in the URL or `[idle] enabled = false`.
(function () {
  "use strict";
  var cfg = {};
  try {
    cfg = JSON.parse(document.body.getAttribute("data-idle") || "{}");
  } catch (e) {
    cfg = {};
  }
  if (cfg.enabled === false) return;
  if (/[?&]calm(=|&|$)/.test(location.search)) return;
  var AFTER = Math.max(10, cfg.after_s || 120) * 1000;
  var WEATHER = cfg.weather !== false;
  var still = window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ---------- where: the time zone's city ----------
  // [lat, lon, place]; a zone not listed falls back to its UTC offset.
  var ZONES = {
    "Europe/Berlin": [52.52, 13.4, "Berlin"], "Europe/Vienna": [48.21, 16.37, "Vienna"],
    "Europe/Zurich": [47.37, 8.54, "Zurich"], "Europe/Amsterdam": [52.37, 4.9, "Amsterdam"],
    "Europe/Brussels": [50.85, 4.35, "Brussels"], "Europe/Paris": [48.86, 2.35, "Paris"],
    "Europe/London": [51.51, -0.13, "London"], "Europe/Dublin": [53.35, -6.26, "Dublin"],
    "Europe/Madrid": [40.42, -3.7, "Madrid"], "Europe/Lisbon": [38.72, -9.14, "Lisbon"],
    "Europe/Rome": [41.9, 12.5, "Rome"], "Europe/Copenhagen": [55.68, 12.57, "Copenhagen"],
    "Europe/Stockholm": [59.33, 18.07, "Stockholm"], "Europe/Oslo": [59.91, 10.75, "Oslo"],
    "Europe/Helsinki": [60.17, 24.94, "Helsinki"], "Europe/Warsaw": [52.23, 21.01, "Warsaw"],
    "Europe/Prague": [50.08, 14.44, "Prague"], "Europe/Budapest": [47.5, 19.04, "Budapest"],
    "Europe/Athens": [37.98, 23.73, "Athens"], "Europe/Istanbul": [41.01, 28.98, "Istanbul"],
    "Europe/Kyiv": [50.45, 30.52, "Kyiv"], "Europe/Bucharest": [44.43, 26.1, "Bucharest"],
    "America/New_York": [40.71, -74.01, "New York"], "America/Chicago": [41.88, -87.63, "Chicago"],
    "America/Denver": [39.74, -104.99, "Denver"], "America/Los_Angeles": [34.05, -118.24, "Los Angeles"],
    "America/Toronto": [43.65, -79.38, "Toronto"], "America/Vancouver": [49.28, -123.12, "Vancouver"],
    "America/Mexico_City": [19.43, -99.13, "Mexico City"], "America/Sao_Paulo": [-23.55, -46.63, "São Paulo"],
    "America/Argentina/Buenos_Aires": [-34.6, -58.38, "Buenos Aires"], "America/Santiago": [-33.45, -70.67, "Santiago"],
    "America/Bogota": [4.71, -74.07, "Bogotá"], "America/Lima": [-12.05, -77.04, "Lima"],
    "Asia/Tokyo": [35.68, 139.69, "Tokyo"], "Asia/Seoul": [37.57, 126.98, "Seoul"],
    "Asia/Shanghai": [31.23, 121.47, "Shanghai"], "Asia/Hong_Kong": [22.32, 114.17, "Hong Kong"],
    "Asia/Singapore": [1.35, 103.82, "Singapore"], "Asia/Kolkata": [22.57, 88.36, "Kolkata"],
    "Asia/Dubai": [25.2, 55.27, "Dubai"], "Asia/Jerusalem": [31.77, 35.21, "Jerusalem"],
    "Asia/Bangkok": [13.76, 100.5, "Bangkok"], "Asia/Jakarta": [-6.21, 106.85, "Jakarta"],
    "Australia/Sydney": [-33.87, 151.21, "Sydney"], "Australia/Melbourne": [-37.81, 144.96, "Melbourne"],
    "Australia/Brisbane": [-27.47, 153.03, "Brisbane"], "Australia/Perth": [-31.95, 115.86, "Perth"],
    "Pacific/Auckland": [-36.85, 174.76, "Auckland"], "Africa/Johannesburg": [-26.2, 28.05, "Johannesburg"],
    "Africa/Cairo": [30.04, 31.24, "Cairo"], "Africa/Lagos": [6.52, 3.38, "Lagos"],
    "Africa/Nairobi": [-1.29, 36.82, "Nairobi"], "UTC": [51.48, 0, "Greenwich"]
  };
  var SOUTH = /^(Australia|Antarctica)\/|^Pacific\/(Auckland|Fiji|Chatham)|^America\/(Argentina|Santiago|Sao_Paulo|Montevideo|Asuncion)|^Africa\/(Johannesburg|Maputo|Harare|Windhoek)|^Indian\/(Mauritius|Reunion)/;

  function where() {
    var tz = "UTC";
    try {
      tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
    } catch (e) {}
    var z = ZONES[tz];
    if (z) return { tz: tz, lat: z[0], lon: z[1], place: z[2], south: z[0] < 0 };
    var south = SOUTH.test(tz);
    var lon = Math.max(-180, Math.min(180, (-new Date().getTimezoneOffset() / 60) * 15));
    var place = tz.split("/").pop().replace(/_/g, " ");
    return { tz: tz, lat: south ? -35 : 50, lon: lon, place: place, south: south };
  }

  // ---------- the sky ----------
  var RAD = Math.PI / 180;
  function jd(ms) { return ms / 86400000 + 2440587.5; }
  function ms(j) { return (j - 2440587.5) * 86400000; }

  // Sunrise and sunset (the sunrise equation, to a minute or two).
  function sunTimes(now, lat, lon) {
    var noon = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 12).getTime();
    var n = Math.ceil(jd(noon) - 2451545.0 + 0.0008);
    var js = n - lon / 360;
    var M = (357.5291 + 0.98560028 * js) % 360;
    var C = 1.9148 * Math.sin(M * RAD) + 0.02 * Math.sin(2 * M * RAD) + 0.0003 * Math.sin(3 * M * RAD);
    var L = (M + C + 180 + 102.9372) % 360;
    var transit = 2451545.0 + js + 0.0053 * Math.sin(M * RAD) - 0.0069 * Math.sin(2 * L * RAD);
    var dec = Math.asin(Math.sin(L * RAD) * Math.sin(23.4397 * RAD));
    var cosw = (Math.sin(-0.833 * RAD) - Math.sin(lat * RAD) * Math.sin(dec)) /
      (Math.cos(lat * RAD) * Math.cos(dec));
    if (cosw > 1) return { polar: "night" };
    if (cosw < -1) return { polar: "day" };
    var w = Math.acos(cosw) / RAD;
    return { rise: new Date(ms(transit - w / 360)), set: new Date(ms(transit + w / 360)) };
  }

  var SYNODIC = 29.530588853;
  var NEW_MOON = Date.UTC(2000, 0, 6, 18, 14);
  function moon(now) {
    var age = ((now.getTime() - NEW_MOON) / 86400000) % SYNODIC;
    if (age < 0) age += SYNODIC;
    var p = age / SYNODIC;
    var names = ["new moon", "waxing crescent", "first quarter", "waxing gibbous",
      "full moon", "waning gibbous", "last quarter", "waning crescent"];
    var name = names[Math.floor(p * 8 + 0.5) % 8];
    var toFull = (SYNODIC / 2 - age + SYNODIC) % SYNODIC;
    return { p: p, lit: (1 - Math.cos(2 * Math.PI * p)) / 2, name: name, toFull: toFull };
  }

  // ---------- the calendar ----------
  function easter(y) {
    var a = y % 19, b = Math.floor(y / 100), c = y % 100, d = Math.floor(b / 4), e = b % 4,
      f = Math.floor((b + 8) / 25), g = Math.floor((b - f + 1) / 3),
      h = (19 * a + b - d - g + 15) % 30, i = Math.floor(c / 4), k = c % 4,
      l = (32 + 2 * e + 2 * i - h - k) % 7, m = Math.floor((a + 11 * h + 22 * l) / 451),
      mo = Math.floor((h + l - 7 * m + 114) / 31), da = ((h + l - 7 * m + 114) % 31) + 1;
    return new Date(y, mo - 1, da);
  }
  function addDays(d, n) { return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n); }

  // [date, name, line, effect]; the effect replaces the month's for the day.
  function moments(y, south) {
    var e = easter(y);
    var xmas = new Date(y, 11, 25);
    var advent4 = addDays(xmas, -(xmas.getDay() || 7));
    var longest = south ? "shortest" : "longest";
    var shortest = south ? "longest" : "shortest";
    return [
      [new Date(y, 0, 1), "New Year", "A fresh year of green builds.", "fireworks"],
      [new Date(y, 0, 6), "Epiphany", "Three kings, zero queued jobs, one can dream.", null],
      [addDays(e, -52), "Weiberfastnacht", "Carnival's Thursday: mind your tie.", "confetti"],
      [addDays(e, -48), "Rosenmontag", "Carnival parades. Kamelle!", "confetti"],
      [new Date(y, 1, 14), "Valentine's Day", "Somewhere a runner and a job found each other.", "hearts"],
      [new Date(y, 2, 14), "Pi Day", "3.14159… and the queue still is not irrational.", null],
      [new Date(y, 2, 20), "Equinox", "Day and night are equal today.", null],
      [new Date(y, 3, 1), "April 1st", "Every queue is exactly zero seconds long today. Honest.", null],
      [addDays(e, -2), "Good Friday", "A quiet day for the runners.", null],
      [e, "Easter", "Somebody hid an egg in the build cache.", "petals"],
      [new Date(y, 4, 1), "May Day", "Maypoles, and the first warm evenings.", "petals"],
      [new Date(y, 4, 4), "May the 4th", "May the green checks be with you.", "meteors"],
      [new Date(y, 5, 21), "Solstice", "The " + longest + " day of the year.", null],
      [new Date(y, 7, 12), "Perseids", "Meteor shower peaks tonight: look up after midnight.", "meteors"],
      [new Date(y, 8, 22), "Equinox", "Day and night are equal again.", null],
      [new Date(y, 9, 31), "Halloween", "Spooky: a job that passes only on the second try.", "bats"],
      [new Date(y, 10, 11), "St. Martin", "Lantern walks tonight; at 11:11 carnival season begins.", "lanterns"],
      [addDays(advent4, -21), "First Advent", "One candle.", "candles"],
      [addDays(advent4, -14), "Second Advent", "Two candles.", "candles"],
      [addDays(advent4, -7), "Third Advent", "Three candles.", "candles"],
      [advent4, "Fourth Advent", "Four candles. Almost there.", "candles"],
      [new Date(y, 11, 6), "St. Nicholas", "Boots by the door: hopefully a green build in each.", "snow"],
      [new Date(y, 11, 13), "Geminids", "The year's richest meteor shower peaks tonight.", "meteors"],
      [new Date(y, 11, 21), "Solstice", "The " + shortest + " day of the year.", null],
      [new Date(y, 11, 24), "Christmas Eve", "Silent night. Even the runners rest. Mostly.", "snow"],
      [new Date(y, 11, 25), "Christmas", "Merry Christmas from the control tower.", "snow"],
      [new Date(y, 11, 31), "New Year's Eve", "Fireworks at midnight. Last builds of the year.", "fireworks"]
    ];
  }

  function moment(now, south) {
    var today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
    var all = moments(now.getFullYear(), south).concat(moments(now.getFullYear() + 1, south));
    var next = null;
    for (var i = 0; i < all.length; i++) {
      var days = Math.round((all[i][0].getTime() - today) / 86400000);
      if (days === 0) return { name: all[i][1], line: all[i][2], fx: all[i][3], days: 0 };
      if (days > 0 && days <= 14 && (!next || days < next.days)) {
        next = { name: all[i][1], line: all[i][2], fx: null, days: days };
      }
    }
    return next;
  }

  // ---------- the month ----------
  // By the season's month: in the southern hemisphere the year turns by six.
  var MONTHS = [
    ["Frost", "Deep winter. The runners keep each other warm.", "snow", "#bfe3ff"],
    ["Carnival", "Masks, confetti, and builds in disguise.", "confetti", "#ffcf6b"],
    ["Spring rain", "Rain on the windows; things start to grow.", "rain", "#8fc7ff"],
    ["Blossom", "Cherry trees, and green checks in bloom.", "petals", "#ffc0d9"],
    ["Seeds on the wind", "Dandelion seeds drift past the tower.", "seeds", "#f4f1da"],
    ["Midsummer", "Fireflies over the long evenings.", "fireflies", "#e8ff8a"],
    ["The lake", "High summer. Somebody went swimming.", "bubbles", "#9ef0ff"],
    ["Shooting stars", "August nights: the Perseids fly.", "meteors", "#ffffff"],
    ["Migration", "The swallows leave; the queues stay.", "birds", "#d8e2ee"],
    ["Falling leaves", "Autumn paints the runway.", "leaves", "#ffad5c"],
    ["Lanterns", "Fog, lantern walks, early dark.", "lanterns", "#ffcf7a"],
    ["Advent lights", "Lights in every window; snow, maybe.", "lights", "#fff0b0"]
  ];

  // ---------- weather (Open-Meteo, from the browser, no key) ----------
  var WMO = [[0, "clear"], [1, "mostly clear"], [2, "partly cloudy"], [3, "overcast"],
    [45, "fog"], [48, "rime fog"], [51, "light drizzle"], [53, "drizzle"], [55, "heavy drizzle"],
    [56, "freezing drizzle"], [61, "light rain"], [63, "rain"], [65, "heavy rain"],
    [66, "freezing rain"], [71, "light snow"], [73, "snow"], [75, "heavy snow"], [77, "snow grains"],
    [80, "showers"], [81, "showers"], [82, "violent showers"], [85, "snow showers"], [86, "snow showers"],
    [95, "thunderstorm"], [96, "thunderstorm with hail"], [99, "thunderstorm with hail"]];
  function wmo(code) {
    var t = "";
    for (var i = 0; i < WMO.length; i++) if (WMO[i][0] <= code) t = WMO[i][1];
    var fx = code >= 95 ? "storm" : (code >= 71 && code <= 77) || code === 85 || code === 86 ? "snow"
      : code >= 51 ? "rain" : code === 45 || code === 48 ? "fog" : null;
    return { text: t, fx: fx };
  }
  var weather = null, weatherAt = 0;
  function fetchWeather(w) {
    if (!WEATHER || Date.now() - weatherAt < 30 * 60000) return;
    weatherAt = Date.now();
    var u = "https://api.open-meteo.com/v1/forecast?latitude=" + w.lat.toFixed(2) +
      "&longitude=" + w.lon.toFixed(2) + "&current=temperature_2m,weather_code,wind_speed_10m";
    fetch(u).then(function (r) { return r.ok ? r.json() : null; }).then(function (j) {
      if (!j || !j.current) return;
      var c = wmo(j.current.weather_code | 0);
      weather = { temp: Math.round(j.current.temperature_2m), text: c.text, fx: c.fx,
        wind: Math.round(j.current.wind_speed_10m || 0) };
      if (showing) { caption(); layers(); }
    }).catch(function () {});
  }

  // ---------- facts from the ground station ----------
  var facts = [];
  function fmt(n) { return Math.round(n).toLocaleString("en"); }
  function fetchFacts() {
    var f = [];
    var q = fetch("api/queue").then(function (r) { return r.ok ? r.json() : null; }).then(function (j) {
      var s = j && j.self_hosted;
      if (!s) return;
      if (s.busy === 0 && s.queued_jobs === 0) f.push("All " + s.total + " runners are napping. Nothing waits.");
      else f.push(s.busy + " of " + s.total + " runners are busy; " + fmt(s.queued_jobs) + " jobs wait their turn.");
    }).catch(function () {});
    var r = fetch("api/report").then(function (r) { return r.ok ? r.json() : null; }).then(function (j) {
      if (!j) return;
      var ran = 0, minutes = 0;
      (j.pools || []).forEach(function (p) {
        ran += p.jobs_ran || 0;
        if (/^hosted/.test(p.pool || "")) minutes += p.minutes || 0;
      });
      if (ran) f.push(fmt(ran) + " jobs ran this month. Stacked as punch cards, that is " +
        fmt(ran * 0.18 / 1000) + " m of paper.");
      if (minutes) f.push(fmt(minutes) + " GitHub-hosted minutes this month: " +
        fmt(minutes / 1440) + " days of one runner, back to back.");
      var best = (j.runners || []).slice().sort(function (a, b) { return b.work_h - a.work_h; })[0];
      if (best && best.work_h > 1) f.push("Hardest worker this month: " + best.runner + ", " +
        fmt(best.work_h) + " hours of jobs.");
      var day = (j.days || [])[(j.days || []).length - 1];
      if (day && day.stored) f.push(fmt(day.stored) + " jobs finished on " + day.day + ".");
    }).catch(function () {});
    Promise.all([q, r]).then(function () {
      facts = f;
      if (showing) caption();
    });
  }

  // ---------- the scene ----------
  var canvas, ctx, box, showing = false, raf = 0, last = 0, parts = [], scene = null, factIx = 0, factTimer = 0;

  function build() {
    if (canvas) return;
    canvas = document.createElement("canvas");
    canvas.id = "idle-scene";
    canvas.setAttribute("aria-hidden", "true");
    box = document.createElement("div");
    box.id = "idle-caption";
    box.setAttribute("role", "status");
    document.body.appendChild(canvas);
    document.body.appendChild(box);
    ctx = canvas.getContext("2d");
  }

  function size() {
    var d = window.devicePixelRatio || 1;
    canvas.width = Math.floor(innerWidth * d);
    canvas.height = Math.floor(innerHeight * d);
    canvas.style.width = innerWidth + "px";
    canvas.style.height = innerHeight + "px";
    if (ctx) ctx.setTransform(d, 0, 0, d, 0, 0);
  }

  function hhmm(d) {
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function compose() {
    var now = new Date();
    var w = where();
    var season = (now.getMonth() + (w.south ? 6 : 0)) % 12;
    var m = MONTHS[season];
    var sun = sunTimes(now, w.lat, w.lon);
    var night = sun.polar ? sun.polar === "night" : now < sun.rise || now > sun.set;
    scene = { now: now, w: w, theme: m, sun: sun, night: night, moon: moon(now),
      moment: moment(now, w.south) };
  }

  function caption() {
    var s = scene;
    var title = s.moment && s.moment.days === 0 ? s.moment.name : s.theme[0];
    var lines = [];
    lines.push(s.moment && s.moment.days === 0 ? s.moment.line : s.theme[1]);
    var sky;
    if (s.sun.polar) sky = s.sun.polar === "day" ? "The sun does not set today." : "The sun does not rise today.";
    else if (s.night) {
      var rise = s.now < s.sun.rise ? s.sun.rise : sunTimes(addDays(s.now, 1), s.w.lat, s.w.lon).rise || s.sun.rise;
      sky = "Sun up at " + hhmm(rise) + ".";
    }
    else {
      var left = Math.round((s.sun.set - s.now) / 60000);
      sky = "Sunset at " + hhmm(s.sun.set) + ", " + Math.floor(left / 60) + " h " + (left % 60) + " min of daylight left.";
    }
    var mn = s.moon;
    sky += " Moon: " + mn.name + (mn.name !== "full moon" && mn.toFull < 10 ? ", full in " + Math.round(mn.toFull) + " d" : "") + ".";
    lines.push(sky);
    if (s.moment && s.moment.days > 0) {
      lines.push(s.moment.name + " in " + s.moment.days + (s.moment.days === 1 ? " day." : " days."));
    }
    if (weather) {
      lines.push(s.w.place + ": " + weather.temp + " °C, " + weather.text +
        (weather.wind >= 30 ? ", windy" : "") + ".");
    }
    if (facts.length) lines.push(facts[factIx % facts.length]);
    var html = '<b class="idle-title">' + esc(title) + "</b>";
    for (var i = 0; i < lines.length; i++) html += "<span>" + esc(lines[i]) + "</span>";
    html += '<small>' + esc(s.w.place) + " · " + esc(hhmm(s.now)) + " · move to return</small>";
    box.innerHTML = html;
    box.style.setProperty("--idle-tint", s.theme[3]);
  }

  function esc(t) {
    return String(t).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  // ---------- particles ----------
  function R(a, b) { return a + Math.random() * (b - a); }
  var COUNT = { snow: 90, confetti: 80, rain: 110, petals: 45, seeds: 35, fireflies: 40,
    bubbles: 40, meteors: 4, birds: 3, leaves: 40, lanterns: 14, lights: 60, fireworks: 5,
    hearts: 25, bats: 10, candles: 4, storm: 120, fog: 6 };

  function spawn(kind, p, fresh) {
    var W = innerWidth, H = innerHeight;
    p.kind = kind; p.t = 0;
    p.x = R(0, W); p.y = fresh ? R(0, H) : -20;
    p.r = R(1.5, 4); p.vx = 0; p.vy = R(20, 50); p.a = R(0, 6.28); p.va = R(-1, 1);
    p.hue = R(0, 360);
    switch (kind) {
      case "rain": case "storm": p.vy = R(380, 620); p.vx = -60; p.r = R(8, 16); break;
      case "confetti": p.vy = R(40, 90); p.r = R(3, 6); break;
      case "petals": case "leaves": case "hearts": p.vy = R(25, 60); p.vx = R(10, 40); p.r = R(4, 8); break;
      case "seeds": p.vy = R(-12, 12); p.vx = R(15, 45); p.x = fresh ? R(0, W) : -20; p.y = R(0, H); break;
      case "fireflies": p.y = R(H * 0.3, H); p.vy = R(-8, 8); p.vx = R(-8, 8); break;
      case "bubbles": p.y = fresh ? R(0, H) : H + 20; p.vy = -R(20, 55); p.r = R(3, 10); break;
      case "lanterns": p.y = fresh ? R(0, H) : H + 30; p.vy = -R(8, 18); p.r = R(6, 11); break;
      case "lights": p.y = R(0, H); p.vy = 0; p.r = R(1.5, 3.5); break;
      case "meteors": p.x = R(W * 0.2, W * 1.1); p.y = R(-H * 0.3, H * 0.3); p.vx = -R(500, 800); p.vy = R(250, 400); p.delay = R(0, 6); break;
      case "birds": p.x = -60; p.y = R(H * 0.1, H * 0.5); p.vx = R(40, 70); p.vy = R(-4, 4); p.r = R(5, 8); break;
      case "fireworks": p.delay = R(0, 3); p.x = R(W * 0.15, W * 0.85); p.y = R(H * 0.15, H * 0.5); break;
      case "bats": p.x = R(0, W); p.y = R(0, H * 0.6); p.vx = R(-60, 60); p.vy = R(-20, 20); p.r = R(6, 10); break;
      case "candles": p.x = innerWidth * (0.38 + 0.08 * parts.length); p.y = H - 70; break;
      case "fog": p.y = R(0, H); p.vx = R(6, 14); p.r = R(120, 260); p.x = fresh ? R(0, W) : -300; break;
    }
    return p;
  }

  function layers() {
    var s = scene;
    var main = (s.moment && s.moment.days === 0 && s.moment.fx) || s.theme[2];
    var list = [[main, COUNT[main] || 40]];
    if (weather && weather.fx && weather.fx !== main) list.push([weather.fx, Math.round((COUNT[weather.fx] || 40) * 0.5)]);
    parts = [];
    list.forEach(function (l) {
      var n = Math.round(l[1] * Math.min(1.6, innerWidth / 1400));
      if (l[0] === "candles") n = 4;
      for (var i = 0; i < n; i++) parts.push(spawn(l[0], {}, true));
    });
    // a fixed field of stars at night (seeded per day, so it does not jump)
    s.stars = [];
    if (s.night) {
      var seed = s.now.getDate() * 9301 + 49297;
      for (var k = 0; k < 120; k++) {
        seed = (seed * 9301 + 49297) % 233280;
        var x = seed / 233280;
        seed = (seed * 9301 + 49297) % 233280;
        s.stars.push([x, seed / 233280 * 0.7, ((k * 37) % 10) / 10]);
      }
    }
  }

  function step(p, dt) {
    var W = innerWidth, H = innerHeight;
    p.t += dt; p.a += p.va * dt;
    switch (p.kind) {
      case "fireflies": p.vx += R(-20, 20) * dt; p.vy += R(-20, 20) * dt; p.vx *= 0.98; p.vy *= 0.98; break;
      case "seeds": p.vy += R(-8, 8) * dt; break;
      case "bats": p.vx += R(-120, 120) * dt; p.vy += R(-80, 80) * dt; p.vx = Math.max(-90, Math.min(90, p.vx)); break;
      case "snow": case "petals": case "leaves": case "hearts": case "confetti": p.vx = Math.sin(p.t * 1.3 + p.hue) * 25 + (p.kind === "snow" ? 0 : 15); break;
      case "meteors": case "fireworks": if (p.delay > 0) { p.delay -= dt; return; } break;
    }
    if (p.kind !== "lights" && p.kind !== "candles" && p.kind !== "fireworks") { p.x += p.vx * dt; p.y += p.vy * dt; }
    var gone = p.y > H + 40 || p.x > W + 320 || p.x < -340 || p.y < -60;
    if (p.kind === "meteors" && p.t > 1.6) gone = true;
    if (p.kind === "fireworks" && p.t > 2.6) gone = true;
    if (p.kind === "fireflies" || p.kind === "bats") { if (p.x < 0 || p.x > W) p.vx = -p.vx; if (p.y < 0 || p.y > H) p.vy = -p.vy; gone = false; }
    if (gone) spawn(p.kind, p, false);
  }

  function draw(p, tint) {
    var c = ctx;
    if (p.delay > 0) return;
    switch (p.kind) {
      case "snow":
        c.fillStyle = "rgba(240,248,255,.85)"; c.beginPath(); c.arc(p.x, p.y, p.r, 0, 6.29); c.fill(); break;
      case "rain": case "storm":
        c.strokeStyle = "rgba(160,200,255,.5)"; c.lineWidth = 1.2; c.beginPath(); c.moveTo(p.x, p.y);
        c.lineTo(p.x - p.r * 0.12, p.y - p.r); c.stroke(); break;
      case "confetti":
        c.save(); c.translate(p.x, p.y); c.rotate(p.a); c.fillStyle = "hsla(" + p.hue + ",90%,62%,.9)";
        c.fillRect(-p.r / 2, -p.r / 4, p.r, p.r / 2); c.restore(); break;
      case "petals": case "leaves":
        c.save(); c.translate(p.x, p.y); c.rotate(p.a);
        c.fillStyle = p.kind === "petals" ? "rgba(255,190,215,.85)" : "hsla(" + (15 + p.hue % 35) + ",85%,55%,.85)";
        c.beginPath(); c.ellipse(0, 0, p.r, p.r / 2, 0, 0, 6.29); c.fill(); c.restore(); break;
      case "hearts":
        c.save(); c.translate(p.x, p.y); c.rotate(p.a * 0.3); c.fillStyle = "rgba(255,110,150,.8)";
        c.beginPath(); c.moveTo(0, p.r * 0.3); c.bezierCurveTo(-p.r, -p.r * 0.5, -p.r * 0.4, -p.r * 1.1, 0, -p.r * 0.4);
        c.bezierCurveTo(p.r * 0.4, -p.r * 1.1, p.r, -p.r * 0.5, 0, p.r * 0.3); c.fill(); c.restore(); break;
      case "seeds":
        c.strokeStyle = "rgba(250,248,230,.7)"; c.lineWidth = 0.8;
        for (var i = 0; i < 6; i++) { var an = i * 1.047 + p.a; c.beginPath(); c.moveTo(p.x, p.y); c.lineTo(p.x + Math.cos(an) * 5, p.y + Math.sin(an) * 5); c.stroke(); }
        break;
      case "fireflies": case "lights":
        var on = 0.5 + 0.5 * Math.sin(p.t * (p.kind === "lights" ? 1.5 : 3) + p.hue);
        var col = p.kind === "lights" ? "hsla(" + (40 + p.hue % 20) + ",100%,75%," : "rgba(220,255,120,";
        c.fillStyle = col + (0.15 + 0.75 * on) + ")"; c.shadowColor = tint; c.shadowBlur = 12 * on;
        c.beginPath(); c.arc(p.x, p.y, p.r, 0, 6.29); c.fill(); c.shadowBlur = 0; break;
      case "bubbles":
        c.strokeStyle = "rgba(170,240,255,.6)"; c.lineWidth = 1; c.beginPath(); c.arc(p.x + Math.sin(p.t * 2 + p.hue) * 6, p.y, p.r, 0, 6.29); c.stroke(); break;
      case "lanterns":
        var fl = 0.8 + 0.2 * Math.sin(p.t * 7 + p.hue);
        c.fillStyle = "rgba(255,190,90," + (0.55 * fl) + ")"; c.shadowColor = "#ffb347"; c.shadowBlur = 22 * fl;
        c.beginPath(); c.arc(p.x + Math.sin(p.t + p.hue) * 8, p.y, p.r, 0, 6.29); c.fill(); c.shadowBlur = 0; break;
      case "meteors":
        var g = c.createLinearGradient(p.x, p.y, p.x - p.vx * 0.12, p.y - p.vy * 0.12);
        g.addColorStop(0, "rgba(255,255,255,.95)"); g.addColorStop(1, "rgba(255,255,255,0)");
        c.strokeStyle = g; c.lineWidth = 2; c.beginPath(); c.moveTo(p.x, p.y); c.lineTo(p.x - p.vx * 0.12, p.y - p.vy * 0.12); c.stroke(); break;
      case "birds":
        c.strokeStyle = "rgba(220,230,240,.75)"; c.lineWidth = 1.5;
        for (var b = 0; b < 7; b++) {
          var bx = p.x - Math.abs(b - 3) * 16, by = p.y + (b - 3) * 10, fw = Math.sin(p.t * 8 + b) * p.r * 0.6;
          c.beginPath(); c.moveTo(bx - p.r, by - fw); c.lineTo(bx, by); c.lineTo(bx + p.r, by - fw); c.stroke();
        }
        break;
      case "bats":
        var flap = Math.sin(p.t * 14) * p.r * 0.7;
        c.fillStyle = "rgba(20,10,30,.75)"; c.beginPath(); c.moveTo(p.x, p.y);
        c.quadraticCurveTo(p.x - p.r, p.y - flap, p.x - p.r * 1.8, p.y); c.quadraticCurveTo(p.x - p.r, p.y + 2, p.x, p.y + 3);
        c.quadraticCurveTo(p.x + p.r, p.y + 2, p.x + p.r * 1.8, p.y); c.quadraticCurveTo(p.x + p.r, p.y - flap, p.x, p.y); c.fill(); break;
      case "fireworks":
        var k = p.t / 2.6, rad = 20 + 140 * Math.sqrt(k);
        for (var j = 0; j < 28; j++) {
          var aj = j * 0.2244;
          c.fillStyle = "hsla(" + p.hue + ",100%,65%," + (1 - k) + ")";
          c.beginPath(); c.arc(p.x + Math.cos(aj) * rad, p.y + Math.sin(aj) * rad + 30 * k * k, 2, 0, 6.29); c.fill();
        }
        break;
      case "candles":
        c.fillStyle = "rgba(240,235,220,.6)"; c.fillRect(p.x - 6, p.y, 12, 40);
        var fc = 0.85 + 0.15 * Math.sin(p.t * 9 + p.hue);
        c.fillStyle = "rgba(255,200,90," + fc + ")"; c.shadowColor = "#ffb347"; c.shadowBlur = 25;
        c.beginPath(); c.ellipse(p.x, p.y - 8, 4, 9 * fc, 0, 0, 6.29); c.fill(); c.shadowBlur = 0; break;
      case "fog":
        var fg = c.createRadialGradient(p.x, p.y, 0, p.x, p.y, p.r);
        fg.addColorStop(0, "rgba(200,210,220,.12)"); fg.addColorStop(1, "rgba(200,210,220,0)");
        c.fillStyle = fg; c.beginPath(); c.arc(p.x, p.y, p.r, 0, 6.29); c.fill(); break;
    }
  }

  function sky(c) {
    var s = scene, W = innerWidth, H = innerHeight;
    if (!s.night) {
      var g = c.createRadialGradient(W * 0.85, H * 0.1, 0, W * 0.85, H * 0.1, H * 0.6);
      g.addColorStop(0, "rgba(255,220,150,.12)"); g.addColorStop(1, "rgba(255,220,150,0)");
      c.fillStyle = g; c.fillRect(0, 0, W, H);
      return;
    }
    var tw = performance.now() / 1000;
    s.stars.forEach(function (st) {
      c.fillStyle = "rgba(255,255,255," + (0.25 + 0.35 * (0.5 + 0.5 * Math.sin(tw * 1.7 + st[2] * 20))) + ")";
      c.fillRect(st[0] * W, st[1] * H, 1.4, 1.4);
    });
    // the moon in its phase: the lit half-disc, closed by the terminator,
    // an ellipse whose width follows cos(phase)
    var mx = W * 0.86, my = H * 0.14, mr = Math.min(34, W * 0.04), p = s.moon.p;
    var side = p < 0.5 ? 1 : -1, k = Math.cos(2 * Math.PI * p);
    c.fillStyle = "rgba(245,240,215,.08)";
    c.beginPath(); c.arc(mx, my, mr, 0, 6.29); c.fill();
    c.fillStyle = "rgba(245,240,215,.9)";
    c.shadowColor = "rgba(245,240,215,.6)"; c.shadowBlur = 30 * s.moon.lit;
    c.beginPath();
    c.arc(mx, my, mr, -Math.PI / 2, Math.PI / 2, side < 0);
    c.ellipse(mx, my, mr * Math.abs(k), mr, 0, Math.PI / 2, -Math.PI / 2, side * k > 0 ? true : false);
    c.fill();
    c.shadowBlur = 0;
  }

  function frame(ts) {
    raf = 0;
    if (!showing) return;
    var dt = Math.min(0.05, last ? (ts - last) / 1000 : 0.016);
    last = ts;
    ctx.clearRect(0, 0, innerWidth, innerHeight);
    sky(ctx);
    var tint = scene.theme[3];
    for (var i = 0; i < parts.length; i++) { step(parts[i], dt); draw(parts[i], tint); }
    if (weather && weather.fx === "storm" && Math.random() < 0.004) { ctx.fillStyle = "rgba(255,255,255,.18)"; ctx.fillRect(0, 0, innerWidth, innerHeight); }
    if (!still && !document.hidden) raf = requestAnimationFrame(frame);
  }

  function show() {
    if (showing) return;
    build();
    compose();
    size();
    layers();
    caption();
    fetchFacts();
    if (scene) fetchWeather(scene.w);
    showing = true;
    document.body.classList.add("idle-on");
    last = 0;
    if (still) {
      // one quiet frame, no motion
      ctx.clearRect(0, 0, innerWidth, innerHeight);
      sky(ctx);
      parts.forEach(function (p) { draw(p, scene.theme[3]); });
    } else {
      raf = requestAnimationFrame(frame);
    }
    factTimer = setInterval(function () {
      factIx++;
      var stars = scene.stars;
      compose();
      scene.stars = stars;
      caption();
    }, 20000);
  }

  function hide() {
    if (!showing) return;
    showing = false;
    document.body.classList.remove("idle-on");
    if (raf) cancelAnimationFrame(raf);
    raf = 0;
    clearInterval(factTimer);
  }

  var lastInput = Date.now();
  function input() {
    lastInput = Date.now();
    if (showing) hide();
  }
  ["mousemove", "mousedown", "keydown", "wheel", "touchstart", "pointerdown"].forEach(function (e) {
    window.addEventListener(e, input, { passive: true, capture: true });
  });
  document.addEventListener("visibilitychange", function () {
    if (!document.hidden && showing && !raf && !still) raf = requestAnimationFrame(frame);
  });
  window.addEventListener("resize", function () { if (showing) { size(); layers(); } });
  setInterval(function () {
    if (!showing && Date.now() - lastInput >= AFTER) show();
  }, 1000);

  // for the browser tests: the pure parts
  window.warteIdle = { sunTimes: sunTimes, moon: moon, easter: easter, moment: moment,
    where: where, show: show, hide: hide, months: MONTHS };
})();
