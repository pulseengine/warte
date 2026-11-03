// The airport map (REQ-DASH-015): the fleet as an airport, drawn from
// /api/airport. Every string from the API goes in as text (textContent),
// never as markup. Geometry follows the approved mockup: a hall in the
// middle, one pier per machine (LAN on top, remote beyond the tunnel ring),
// stands for boards, planes for agent sessions, runways for busy runners.
(function () {
  var root = document.getElementById('airport');
  if (!root || root.dataset.started) return;
  root.dataset.started = '1';
  var W = 1440, H = 1000, cx = 720, cy = 395, hubR = 128, ringR = hubR + 105;
  var KIND = { terminal: ['terminal', '#1c7ed6'], workshop: ['workshop', '#e64980'], cargo_hall: ['cargo hall', '#f08c00'],
    server: ['server', '#0c8599'], github_hosted: ['GitHub-hosted', '#495057'] };
  var PLANE = 'M21 16v-2l-8-5V3.5a1.5 1.5 0 0 0-3 0V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z';
  var SIGNS = [['Info desk', 'Ask', 'ask'], ['Departures', 'Queue', 'queue'], ['Doctor', 'Health', 'health'],
    ['Fire station', 'Outages', 'outages'], ['Radar', 'Machines', 'machines'], ['Fuel', 'Costs', 'costs'],
    ['Taxiway', 'Critical Path', 'critical-path'], ['Bags', 'Artifacts', 'artifacts'], ['Agents', 'Agents', 'agents']];

  function el(tag, cls, css, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (css) e.style.cssText = css;
    if (text != null) e.textContent = text;
    return e;
  }
  function svg(path, size, fill, rot) {
    var ns = 'http://www.w3.org/2000/svg', s = document.createElementNS(ns, 'svg'), p = document.createElementNS(ns, 'path');
    s.setAttribute('width', size); s.setAttribute('height', size); s.setAttribute('viewBox', '0 0 24 24');
    s.setAttribute('aria-hidden', 'true'); if (rot) s.style.transform = 'rotate(' + rot + 'deg)';
    p.setAttribute('d', path); p.setAttribute('fill', fill); s.appendChild(p); return s;
  }
  function rad(a) { return a * Math.PI / 180; }
  function at(a, d, off) {
    return [cx + Math.cos(rad(a)) * d + Math.cos(rad(a + 90)) * (off || 0), cy + Math.sin(rad(a)) * d + Math.sin(rad(a + 90)) * (off || 0)];
  }
  function spread(list, from, to) {
    return list.map(function (p, i) { return [p, list.length === 1 ? (from + to) / 2 : from + (to - from) * i / (list.length - 1)]; });
  }
  function dur(s) { return s < 90 ? s + 's' : s < 5400 ? Math.round(s / 60) + ' min' : (s / 3600).toFixed(1) + ' h'; }
  function lenOf(p) { var cap = p.location === 'lan' ? 190 : 150; return Math.min(cap, 100 + Math.min(1, Math.log2(Math.max(p.cores, 2)) / 6) * 110); }
  function widthOf(p) { return p.kind === 'cargo_hall' || p.kind === 'github_hosted' ? 56 : p.kind === 'terminal' ? 46 : 32; }
  function openView(slug) {
    var b = Array.prototype.find.call(document.querySelectorAll('.nav-btn'), function (x) {
      return x.textContent.trim().toLowerCase().replace(/\s+/g, '-') === slug; });
    if (b) b.click();
  }

  function draw(a) {
    var stage = el('div', 'ap-stage', 'width:' + W + 'px;height:' + H + 'px');
    stage.appendChild(el('div', 'ap-island'));
    stage.appendChild(el('div', 'ap-ring', 'left:' + (cx - ringR) + 'px;top:' + (cy - ringR) + 'px;width:' + 2 * ringR + 'px;height:' + 2 * ringR + 'px'));
    stage.appendChild(el('span', 'ap-ringlabel', 'left:' + (cx + ringR - 40) + 'px;top:' + (cy + 30) + 'px', 'tunnel · remote machines beyond'));
    var lan = a.piers.filter(function (p) { return p.location === 'lan'; }), far = a.piers.filter(function (p) { return p.location !== 'lan'; });
    var laid = spread(lan, -165, -15).concat(spread(far, 25, 155));
    laid.forEach(function (pa) {
      var p = pa[0], ang = pa[1], len = lenOf(p), w = widthOf(p);
      stage.appendChild(el('div', 'ap-pier' + (p.reporting ? '' : ' off'), 'left:' + cx + 'px;top:' + (cy - w / 2) + 'px;width:' + len + 'px;height:' + w + 'px;transform:rotate(' + ang + 'deg) translateX(' + (hubR - 12) + 'px);border-radius:' + w / 2 + 'px'));
      p.boards.forEach(function (b, i) {
        var xy = at(ang, hubR + len - 30 - i * 22, (i % 2 ? 1 : -1) * (w / 2 + 9));
        var s = el('span', 'ap-stand', 'left:' + (xy[0] - 8) + 'px;top:' + (xy[1] - 8) + 'px'); s.title = b; stage.appendChild(s);
      });
      for (var i = 0; i < Math.min(p.agents, 6); i++) {
        var side = i % 2 ? 1 : -1, xy = at(ang, hubR + 40 + Math.floor(i / 2) * 36, side * (w / 2 + 16));
        var pl = el('span', 'ap-plane', 'left:' + (xy[0] - 12) + 'px;top:' + (xy[1] - 12) + 'px');
        pl.appendChild(svg(PLANE, 24, '#1c7ed6', ang + 90 * side + 90)); stage.appendChild(pl);
      }
      var lxy = at(ang, hubR + len + 32, 0), kind = KIND[p.kind] || KIND.server;
      var lab = el('div', 'ap-label', 'left:' + lxy[0] + 'px;top:' + lxy[1] + 'px;transform:translate(-50%,' + (p.location === 'lan' ? '-100%' : '0') + ');border-color:' + kind[1]);
      lab.appendChild(el('b', null, null, p.name));
      var sub = [p.reporting ? kind[0] : kind[0] + ' · not reporting'];
      if (p.agents) sub.push(p.agents + ' agents'); if (p.runners) sub.push(p.runners + ' runways'); if (p.note && p.reporting) sub.push(p.note);
      lab.appendChild(el('span', p.reporting && !p.note ? null : 'warn', null, sub.join(' · ')));
      if (p.boards.length) lab.appendChild(el('span', 'ap-boards', null, p.boards.join(' · ')));
      stage.appendChild(lab);
    });
    stage.appendChild(el('div', 'ap-hall', 'left:' + (cx - hubR) + 'px;top:' + (cy - hubR) + 'px;width:' + 2 * hubR + 'px;height:' + 2 * hubR + 'px'));
    var tower = el('div', 'ap-tower', 'left:' + (cx - 30) + 'px;top:' + (cy - 30) + 'px'); tower.appendChild(el('div', 'ap-sweep')); stage.appendChild(tower);
    stage.appendChild(el('span', 'ap-towername', 'left:' + (cx - 70) + 'px;top:' + (cy + 34) + 'px', 'tower · ' + a.tower));
    SIGNS.forEach(function (sg, i) {
      var xy = at(-90 + i * 360 / SIGNS.length, 86, 0);
      var b = el('button', 'ap-sign', 'left:' + xy[0] + 'px;top:' + xy[1] + 'px', sg[0]);
      b.type = 'button'; b.title = sg[1] + ' view'; b.addEventListener('click', function () { openView(sg[2]); });
      stage.appendChild(b);
    });
    var field = el('div', 'ap-field'), tilt = el('div', 'ap-tilt');
    var runs = a.runways.slice(0, 7), laneH = 430 / Math.max(runs.length, 1);
    runs.forEach(function (r, i) {
      var lane = el('div', 'ap-lane', 'top:' + (i * laneH + 6) + 'px;height:' + (laneH - 12) + 'px');
      lane.appendChild(el('span', 'ap-laneid', null, r.id));
      var pos = Math.min(r.elapsed_s / 1800, 1);
      var craft = el('div', 'ap-craft', 'left:calc(70px + (100% - 230px) * ' + pos + ')');
      craft.appendChild(el('span', 'ap-shadow')); var body = el('span', 'ap-body'); body.appendChild(svg(PLANE, 30, '#1c7ed6', 90)); craft.appendChild(body);
      var sign = el('span', 'ap-runsign'); sign.appendChild(el('b', null, null, (r.flight || 'job') + ' · ' + r.job)); sign.appendChild(el('span', null, null, dur(r.elapsed_s) + ' on the runway'));
      craft.appendChild(sign); lane.appendChild(craft); tilt.appendChild(lane);
    });
    field.appendChild(tilt); stage.appendChild(field);
    stage.appendChild(el('span', 'ap-fieldlabel', null, 'RUNWAYS · ' + a.runways.length + ' of ' + a.runner_total + ' busy · ' + a.holding + ' jobs holding'));
    var hold = el('div', 'ap-hold'); for (var k = 0; k < Math.min(a.holding, 4); k++) { var hp = el('span', 'ap-holdplane', 'animation-delay:' + (-k * 3.5) + 's'); hp.appendChild(svg(PLANE, 18, '#f08c00')); hold.appendChild(hp); }
    hold.appendChild(el('span', 'ap-holdlabel', null, a.holding + ' waiting')); stage.appendChild(hold);
    root.replaceChildren(stage);
    fit();
  }
  function fit() { var s = root.firstChild; if (s) { var k = Math.min(1, root.clientWidth / W); s.style.transform = 'scale(' + k + ')'; root.style.height = (H * k) + 'px'; } }
  function load() {
    if (!document.body.contains(root)) { clearInterval(timer); return; }
    fetch('api/airport').then(function (r) { return r.json(); }).then(draw).catch(function () {
      root.replaceChildren(el('p', 'empty', null, 'The airport could not be loaded.'));
    });
  }
  var timer = setInterval(function () { if (!document.hidden) load(); }, 30000);
  window.addEventListener('resize', fit);
  load();
})();
