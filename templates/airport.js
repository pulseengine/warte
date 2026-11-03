// The airport map (REQ-DASH-015): the fleet as an airport, drawn from
// /api/airport. Every string from the API goes in as text (textContent),
// never as markup. Geometry follows the approved mockup (the "Island" board):
// a hall in the middle with the tower, one pier per machine (LAN on top,
// remote beyond the tunnel ring), stands for boards, planes for agent
// sessions, a 3D runway field for busy runners, guard posts for security.
// Tap a pier, a sign or a guard and the card on the left tells its story.
(function () {
  var root = document.getElementById('airport');
  if (!root || root.dataset.started) return;
  root.dataset.started = '1';
  var W = 1440, H = 1000, cx = 720, cy = 395, hubR = 128, ringR = hubR + 105, LANES = 7;
  var KIND = { terminal: ['terminal', '#1c7ed6'], workshop: ['workshop', '#e64980'], cargo_hall: ['cargo hall', '#f08c00'],
    server: ['server', '#0c8599'], github_hosted: ['GitHub-hosted', '#495057'] };
  var ICON = { terminal: 'M4 5h16v11H4zM2 19h20', workshop: 'M5 8h14v11H5zM9 4v4M15 4v4M9 12h6M9 15h6',
    cargo_hall: 'M3 8l9-4 9 4v10l-9 4-9-4zM3 8l9 4 9-4M12 12v10', server: 'M4 4h16v6H4zM4 14h16v6H4zM8 7h.01M8 17h.01',
    github_hosted: 'M7 18a4 4 0 0 1 0-8 6 6 0 0 1 11 2 3 3 0 0 1 0 6z' };
  var PLANE = 'M21 16v-2l-8-5V3.5a1.5 1.5 0 0 0-3 0V9l-8 5v2l8-2.5V19l-2 1.5V22l3.5-1 3.5 1v-1.5L13 19v-5.5z';
  var C = { blue: '#1c7ed6', orange: '#e8590c', green: '#2f9e44', red: '#e03131', teal: '#0c8599', purple: '#7048e8' };
  // [sign, view, icon, colour, story, for grown-ups]
  var SIGNS = [
    ['Info desk', 'Ask', 'M12 8h.01M11 12h1v5h1M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18', 'blue', 'Ask when your plane leaves. The helper checks how long others waited today.', 'ETA from measured waits.'],
    ['Departures', 'Queue', 'M4 5h16v12H4zM7 9h6M7 13h10', 'blue', 'The board lists every plane waiting to fly.', 'Pools, waits, every queued run.'],
    ['Fuel', 'Costs', 'M4 21V5a2 2 0 0 1 2-2h6a2 2 0 0 1 2 2v16M3 21h12M14 10h2a2 2 0 0 1 2 2v4a1 1 0 0 0 2 0V8l-3-3', 'green', 'Planes need fuel; agents need tokens to think.', 'Notional cost of agent chats.'],
    ['Fire station', 'Outages', 'M12 3c1 4 5 5 5 10a5 5 0 0 1-10 0c0-3 2-4 2-6 2 1 3 3 3 3s1-3 0-7z', 'red', 'When something breaks, the bell rings here first.', 'Problems now, worst first.'],
    ['Doctor', 'Health', 'M12 20s-7-4.5-7-10a4 4 0 0 1 7-2.5A4 4 0 0 1 19 10c0 5.5-7 10-7 10z', 'red', 'The doctor checks the cargo halls.', 'CI host health and link.'],
    ['Radar', 'Machines', 'M12 12l6-6M12 3a9 9 0 1 0 9 9M12 7a5 5 0 1 0 5 5', 'teal', 'The radar knows every pier and what is parked there.', 'Every machine, its disks, tools and agents.'],
    ['Taxiway', 'Critical Path', 'M4 20L12 4l8 16M8 13h8', 'teal', 'Some planes must wait for others to land first.', 'What blocks what, the longest chain.'],
    ['Bags', 'Artifacts', 'M6 8h12v12H6zM9 8V5h6v3', 'orange', 'Finished things arrive here, labelled.', 'Build outputs by hash.'],
    ['Pilots', 'Agents', 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0', 'purple', 'Every pilot has a name and a badge.', 'Agent identities and rights.']
  ];
  // security: the watchtower's other job. [title, x, y, icon, story, for grown-ups]
  var GUARDS = [
    ['Passport control', cx + hubR + 45, cy - 8, 'M6 3h12v18H6zM12 13a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM9 17h6', 'Every computer that wants to land here shows its passport first: a secret key only it has, checked by the tower.', 'One-time-token enrollment, a key made on the machine, signed reports (DD-PROBE-002, REQ-PROBE-006).'],
    ['Boarding pass', cx - hubR - 45, cy - 8, 'M3 8a2 2 0 0 0 0 4v4h18v-4a2 2 0 0 0 0-4V4H3zM14 4v16', 'An agent plane needs a boarding pass before it may change anything.', 'Every write needs a token.'],
    ['Express lane', cx + ringR + 75, cy - 8, 'M5 12h10M11 6l6 6-6 6M19 5v14', 'Fire engines never wait: a fix for a security hole or a broken main road goes to the front. It still goes through every check!', 'Security and red-main PRs are always admitted (/api/admission). They skip the waiting, never the checks.'],
    ['Customs', cx + 210, cy + 250, 'M12 3l8 4v5c0 5-3.5 8-8 9-4.5-1-8-4-8-9V7zM9 12l2 2 4-4', 'Everything that leaves is checked: is the seal real, is it what the label says?', 'Releases carry signatures and checksums.'],
    ['VIP lane', cx - ringR - 75, cy - 8, 'M12 3l2.8 5.8 6.2.9-4.5 4.4 1 6.2L12 17.4 6.5 20.3l1-6.2L3 9.7l6.2-.9z', 'Planes from people you care about most are shown first. They still show their passport like everyone else.', 'Planned: watched authors and repos first in Reviews (REQ-REVIEWS-001).']
  ];

  var sel = null, last = null, WAITING = 'waiting for you';
  function el(tag, cls, css, text) {
    var e = document.createElement(tag);
    if (cls) e.className = cls;
    if (css) e.style.cssText = css;
    if (text != null) e.textContent = text;
    return e;
  }
  function svg(path, size, fill, rot, stroke) {
    var ns = 'http://www.w3.org/2000/svg', s = document.createElementNS(ns, 'svg'), p = document.createElementNS(ns, 'path');
    s.setAttribute('width', size); s.setAttribute('height', size); s.setAttribute('viewBox', '0 0 24 24');
    s.setAttribute('aria-hidden', 'true'); if (rot) s.style.transform = 'rotate(' + rot + 'deg)';
    p.setAttribute('d', path);
    if (stroke) { p.setAttribute('fill', 'none'); p.setAttribute('stroke', stroke); p.setAttribute('stroke-width', '2.4'); p.setAttribute('stroke-linecap', 'round'); p.setAttribute('stroke-linejoin', 'round'); }
    else p.setAttribute('fill', fill);
    s.appendChild(p); return s;
  }
  function person(shirt) {
    var ns = 'http://www.w3.org/2000/svg', s = document.createElementNS(ns, 'svg');
    s.setAttribute('width', 16); s.setAttribute('height', 22); s.setAttribute('viewBox', '0 0 16 22'); s.setAttribute('aria-hidden', 'true');
    var head = document.createElementNS(ns, 'circle'); head.setAttribute('cx', 8); head.setAttribute('cy', 4); head.setAttribute('r', 3.4); head.setAttribute('fill', '#f1c8a5');
    var body = document.createElementNS(ns, 'path'); body.setAttribute('d', 'M3 21v-7a5 5 0 0 1 10 0v7z'); body.setAttribute('fill', shirt);
    s.appendChild(head); s.appendChild(body); return s;
  }
  function rad(a) { return a * Math.PI / 180; }
  function at(a, d, off) {
    return [cx + Math.cos(rad(a)) * d + Math.cos(rad(a + 90)) * (off || 0), cy + Math.sin(rad(a)) * d + Math.sin(rad(a + 90)) * (off || 0)];
  }
  function spread(list, from, to) {
    return list.map(function (p, i) { return [p, list.length === 1 ? (from + to) / 2 : from + (to - from) * i / (list.length - 1)]; });
  }
  function dur(s) { return s < 90 ? s + ' s' : s < 5400 ? Math.round(s / 60) + ' min' : (s / 3600).toFixed(1) + ' h'; }
  function lenOf(p) { var cap = p.location === 'lan' ? 190 : 150; return Math.min(cap, 100 + Math.min(1, Math.log2(Math.max(p.cores, 2)) / 6) * 110); }
  function widthOf(p) { return p.kind === 'cargo_hall' || p.kind === 'github_hosted' ? 56 : p.kind === 'terminal' ? 46 : 32; }
  function openView(name) {
    var slug = name.toLowerCase().replace(/\s+/g, '-');
    var b = Array.prototype.find.call(document.querySelectorAll('.nav-btn'), function (x) {
      return x.textContent.trim().toLowerCase().replace(/\s+/g, '-') === slug; });
    if (document.fullscreenElement) document.exitFullscreen();
    if (b) b.click();
  }
  function pick(card) { sel = card; if (last) draw(last); }
  function story(p) {
    var k = (KIND[p.kind] || KIND.server)[0];
    var s = p.kind === 'terminal' ? 'A passenger terminal. ' + (p.agents ? p.agents + ' agent planes are parked here.' : 'No planes parked right now.')
      : p.kind === 'workshop' ? 'A little workshop' + (p.boards.length ? ' with ' + p.boards.length + ' boards on its bench. Only one plane may use a board at a time.' : '.')
      : p.kind === 'cargo_hall' ? 'A big cargo hall that loads work onto ' + p.runners + ' runways at once.'
      : p.kind === 'github_hosted' ? 'Runways we borrow from GitHub. We cannot see inside, but we can see the planes.'
      : 'A server.';
    if (!p.reporting) s += ' Its radio is quiet: it does not report yet.';
    return { title: p.name, view: 'Machines', story: s,
      adult: (p.location === 'lan' ? 'Home LAN' : 'Remote, over the tunnel') + ', ' + k + (p.cores ? ', ' + p.cores + ' cores' : '') + (p.boards.length ? ', boards: ' + p.boards.join(', ') : '') + (p.note ? ', ' + p.note : '') + '.' };
  }

  function draw(a) {
    last = a;
    var stage = el('div', 'ap-stage', 'width:' + W + 'px;height:' + H + 'px');
    stage.appendChild(el('div', 'ap-island'));
    var busy = a.runways.slice(0, LANES), lanes = Math.min(Math.max(a.runner_total, busy.length), LANES);

    // the runway field, in 3D: always the same lane height; busy lanes first
    if (lanes > 0) {
      var laneH = 430 / LANES, field = el('div', 'ap-field'), tilt = el('div', 'ap-tilt', 'height:' + Math.round(lanes * laneH + 10) + 'px');
      var per = {}; a.runways.forEach(function (r) { per[r.flight] = (per[r.flight] || 0) + 1; });
      for (var i = 0; i < lanes; i++) {
        var r = busy[i], lane = el('div', 'ap-lane' + (r ? '' : ' idle'), 'top:' + (i * laneH + 5) + 'px;height:' + (laneH - 10) + 'px');
        lane.appendChild(el('span', 'ap-centre'));
        lane.appendChild(el('span', 'ap-laneid', null, r ? r.id : 'free'));
        if (r) {
          var n = per[r.flight] || 1, size = n >= 7 ? 'L' : n >= 3 ? 'M' : 'S', px = size === 'L' ? 46 : size === 'M' ? 36 : 26;
          var pos = Math.min(r.elapsed_s / 1800, 1), long = r.elapsed_s > 3600, tone = long ? C.orange : C.blue;
          // the plane rolls along the left part of the lane; its sign stands at the far end
          var travel = '(100% - 520px) * ' + pos;
          lane.appendChild(el('span', 'ap-trail', 'width:calc(' + travel + ');background:' + tone));
          var craft = el('div', 'ap-craft', 'left:calc(70px + ' + travel + ');width:' + px + 'px;height:' + px + 'px;margin:-' + px / 2 + 'px 0 0 -' + px / 2 + 'px');
          craft.appendChild(el('span', 'ap-shadow'));
          var body = el('span', 'ap-body'); body.appendChild(svg(PLANE, px, tone, 90)); craft.appendChild(body);
          lane.appendChild(craft);
          var what = (r.flight || 'job') + ' · ' + r.job, jobs = n + (n === 1 ? ' job' : ' jobs');
          var sign = el('button', 'ap-runsign', 'border-color:' + tone); sign.type = 'button';
          sign.setAttribute('aria-label', r.id + ': ' + what + ', ' + dur(r.elapsed_s));
          sign.appendChild(el('b', null, null, what));
          sign.appendChild(el('span', long ? 'warn' : null, null, size + ' · ' + dur(r.elapsed_s)));
          (function (r, what, jobs, size, long) {
            sign.addEventListener('click', function () {
              pick({ title: r.id + ' · ' + (r.flight || 'a job'), view: 'Queue',
                story: 'A plane on runway ' + r.id + ' is taking off: ' + r.job + '. It has been rolling for ' + dur(r.elapsed_s) + (long ? ', which is a long time.' : '.') ,
                adult: what + ': ' + jobs + ' of this PR running now (plane size ' + size + '), ' + dur(r.elapsed_s) + ' on the runner.', open: true });
            });
          })(r, what, jobs, size, long);
          lane.appendChild(sign);
        }
        tilt.appendChild(lane);
      }
      field.appendChild(tilt); stage.appendChild(field);
      stage.appendChild(el('span', 'ap-fieldlabel', null, 'RUNWAY FIELD · ' + a.runways.length + ' OF ' + a.runner_total + ' RUNNERS BUSY · PLANE SIZE = JOBS OF THAT PR FLYING NOW'));
      var hold = el('div', 'ap-hold'); hold.appendChild(el('span', 'ap-holdring'));
      for (var k = 0; k < Math.min(a.holding, 4); k++) { var hp = el('span', 'ap-holdplane', 'animation-delay:' + (-k * 3.5) + 's'); hp.appendChild(svg(PLANE, 18, '#f08c00')); hold.appendChild(hp); }
      hold.appendChild(el('span', 'ap-holdlabel', null, a.holding + ' waiting')); stage.appendChild(hold);
    }

    stage.appendChild(el('div', 'ap-ring', 'left:' + (cx - ringR) + 'px;top:' + (cy - ringR) + 'px;width:' + 2 * ringR + 'px;height:' + 2 * ringR + 'px'));
    stage.appendChild(el('span', 'ap-ringlabel', 'left:' + (cx + ringR - 40) + 'px;top:' + (cy + 30) + 'px', 'tunnel · remote machines beyond'));
    var lan = a.piers.filter(function (p) { return p.location === 'lan'; }), far = a.piers.filter(function (p) { return p.location !== 'lan'; });
    var laid = spread(lan, -165, -15).concat(spread(far, 25, 155)), dense = laid.length > 9;
    laid.forEach(function (pa) {
      var p = pa[0], ang = pa[1], len = lenOf(p), w = widthOf(p) * (dense ? 0.8 : 1);
      stage.appendChild(el('div', 'ap-pier' + (p.reporting ? '' : ' off'), 'left:' + cx + 'px;top:' + (cy - w / 2) + 'px;width:' + len + 'px;height:' + w + 'px;transform:rotate(' + ang + 'deg) translateX(' + (hubR - 12) + 'px);border-radius:' + w / 2 + 'px'));
    });
    laid.forEach(function (pa) {
      var p = pa[0], ang = pa[1], len = lenOf(p), w = widthOf(p);
      p.boards.forEach(function (b, i) {
        var xy = at(ang, hubR + len - 30 - i * 22, (i % 2 ? 1 : -1) * (w / 2 + 9));
        var s = el('span', 'ap-stand', 'left:' + (xy[0] - 8) + 'px;top:' + (xy[1] - 8) + 'px'); s.title = b; stage.appendChild(s);
      });
      for (var i = 0; i < Math.min(p.agents, dense ? 4 : 6); i++) {
        var side = i % 2 ? 1 : -1, xy = at(ang, hubR + 40 + Math.floor(i / 2) * 36, side * (w / 2 + 16));
        var waits = p.sessions && p.sessions[i] && p.sessions[i].activity === WAITING;
        var pl = el('span', 'ap-plane', 'left:' + (xy[0] - 12) + 'px;top:' + (xy[1] - 12) + 'px');
        pl.appendChild(svg(PLANE, 24, waits ? '#f08c00' : C.blue, ang + 90 * side + 90));
        if (waits) pl.appendChild(el('span', 'ap-beacon'));
        stage.appendChild(pl);
      }
    });

    stage.appendChild(el('div', 'ap-hall', 'left:' + (cx - hubR) + 'px;top:' + (cy - hubR) + 'px;width:' + 2 * hubR + 'px;height:' + 2 * hubR + 'px'));
    var tower = el('button', 'ap-tower', 'left:' + (cx - 30) + 'px;top:' + (cy - 30) + 'px'); tower.type = 'button'; tower.setAttribute('aria-label', 'Control tower');
    tower.appendChild(el('span', 'ap-sweep')); tower.addEventListener('click', function () { pick(null); }); stage.appendChild(tower);
    stage.appendChild(el('span', 'ap-towername', 'left:' + (cx - 70) + 'px;top:' + (cy + 34) + 'px', 'tower · ' + a.tower));
    SIGNS.forEach(function (sg, i) {
      var xy = at(-90 + i * 360 / SIGNS.length, 86, 0), on = sel && sel.title === sg[0];
      var b = el('button', 'ap-sign', 'left:' + xy[0] + 'px;top:' + xy[1] + 'px');
      b.type = 'button'; b.title = sg[1] + ' view'; b.setAttribute('aria-label', sg[0] + ': ' + sg[1]);
      var dot = el('span', 'ap-dot', 'background:' + C[sg[3]] + (on ? ';box-shadow:0 0 0 3px #fff,0 0 0 6px ' + C[sg[3]] : ''));
      dot.appendChild(svg(sg[2], 14, null, 0, '#fff')); b.appendChild(dot); b.appendChild(el('span', null, null, sg[0]));
      b.addEventListener('click', function () { if (on) openView(sg[1]); else pick({ title: sg[0], view: sg[1], story: sg[4], adult: sg[5], open: true }); });
      stage.appendChild(b);
    });

    laid.forEach(function (pa) {
      var p = pa[0], ang = pa[1], kind = KIND[p.kind] || KIND.server;
      var xy = at(ang, hubR + lenOf(p) + (p.location === 'lan' ? 34 : 30), 0), on = sel && sel.title === p.name;
      var b = el('button', 'ap-label', 'left:' + xy[0] + 'px;top:' + xy[1] + 'px;transform:translate(-50%,' + (p.location === 'lan' ? '-100%' : '0') + ');max-width:' + (dense ? 150 : 210) + 'px' + (on ? ';box-shadow:0 3px 10px rgba(0,0,0,.12),0 0 0 3px ' + kind[1] : ''));
      b.type = 'button';
      var head = el('span', 'ap-head'), badge = el('span', 'ap-badge', 'background:' + kind[1]);
      badge.appendChild(svg(ICON[p.kind] || ICON.server, 13, null, 0, '#fff')); head.appendChild(badge); head.appendChild(el('b', null, null, p.name)); b.appendChild(head);
      var sub = [p.reporting ? kind[0] : kind[0] + ' · not reporting'];
      if (p.agents) sub.push(p.agents + ' agents'); if (p.runners) sub.push(p.runners + ' runways'); if (p.note && p.reporting) sub.push(p.note);
      var subEl = el('span', p.reporting && !p.note ? 'ap-sub' : 'ap-sub warn', null, sub.join(' · ')); subEl.title = sub.join(' · '); b.appendChild(subEl);
      if (p.boards.length) {
        var devs = el('span', 'ap-devs');
        (dense ? p.boards.slice(0, 2) : p.boards).forEach(function (d) { devs.appendChild(el('span', null, null, d)); });
        b.appendChild(devs);
      }
      b.addEventListener('click', function () { pick(story(p)); });
      stage.appendChild(b);
    });

    // pilots: agents walking between their pier and the hall, saying what they do
    var nw = 0;
    laid.forEach(function (pa) {
      var p = pa[0], ang = pa[1], len = lenOf(p), w = widthOf(p), list = p.sessions || [];
      var busy = list.filter(function (s) { return s.activity; });
      var show = (busy.length ? busy : list.slice(0, 2)).slice(0, dense ? 2 : 4);
      show.forEach(function (s, j) {
        var waiting = s.activity === WAITING, say = (s.repo ? s.repo + ' · ' : '') + (s.activity || dur(s.minutes * 60));
        var walker = el('button', 'ap-walker' + (waiting ? ' waiting' : ''));
        walker.type = 'button'; walker.setAttribute('aria-label', s.agent + ' on ' + p.name + ': ' + say);
        if (waiting) {
          var xy = at(ang, hubR + len + 4, (j % 2 ? 1 : -1) * (w / 2 + 14));
          walker.style.cssText = 'left:' + (xy[0] - 8) + 'px;top:' + (xy[1] - 22) + 'px';
        } else {
          var off = (j % 2 ? 1 : -1) * (w / 2 + 6 + Math.floor(j / 2) * 10);
          var a1 = at(ang, hubR + len - 24, off), a2 = at(ang, hubR - 2, off), hall = at(ang + (j % 2 ? 24 : -24), hubR - 26, 0);
          var path = 'M' + a1.join(' ') + ' L' + a2.join(' ') + ' L' + hall.join(' ') + ' L' + a2.join(' ') + ' Z';
          walker.style.cssText = 'left:0;top:0;offset-path:path(\'' + path + '\');offset-rotate:0deg;animation:apwalk ' + (22 + (nw % 3) * 5) + 's linear infinite;animation-delay:' + (-nw * 6) + 's';
        }
        var fig = el('span', 'ap-fig'); fig.appendChild(person(waiting ? '#f08c00' : nw % 2 ? C.purple : C.blue)); walker.appendChild(fig);
        walker.appendChild(el('span', 'ap-say', null, say));
        walker.addEventListener('click', function () {
          pick({ title: s.agent + ' on ' + p.name, view: 'Agents',
            story: waiting ? 'This pilot has finished and is waiting for you' + (s.repo ? ' in ' + s.repo : '') + '.'
              : 'This pilot is ' + (s.activity || 'busy') + (s.repo ? ' in ' + s.repo : '') + '. They started ' + dur(s.minutes * 60) + ' ago.',
            adult: s.agent + ' session, ' + dur(s.minutes * 60) + ' old' + (s.repo ? ', in ' + s.repo : '') + (s.activity ? ', last hook: ' + s.activity : ', no hook installed') + '.' });
        });
        stage.appendChild(walker); nw++;
      });
    });

    stage.appendChild(el('div', 'ap-fence'));
    stage.appendChild(el('span', 'ap-fencelabel', null, 'PERIMETER FENCE · private network only'));
    GUARDS.forEach(function (g) {
      var on = sel && sel.title === g[0];
      var b = el('button', 'ap-guard', 'left:' + g[1] + 'px;top:' + g[2] + 'px');
      b.type = 'button'; b.setAttribute('aria-label', g[0]);
      var dot = el('span', 'ap-gdot', on ? 'box-shadow:0 3px 0 #7f1d1d,0 0 0 3px #fff,0 0 0 6px #c92a2a' : null);
      dot.appendChild(svg(g[3], 16, null, 0, '#fff')); b.appendChild(dot); b.appendChild(el('span', null, null, g[0]));
      b.addEventListener('click', function () { pick({ title: g[0], view: 'Security', story: g[4], adult: g[5] }); });
      stage.appendChild(b);
    });

    var c = sel || { title: 'Control tower · ' + a.tower, view: 'Overview', story: 'The tower sits in the middle of the main hall and watches every pier, every runway and every plane.', adult: 'The ground station; every sign around it is one view.' };
    var card = el('div', 'ap-card');
    card.appendChild(el('span', 'ap-kicker', null, 'TAP A PIER, A SIGN OR A GUARD'));
    card.appendChild(el('span', 'ap-title', null, c.title));
    card.appendChild(el('span', 'ap-story', null, c.story));
    var adult = el('span', 'ap-adult', null, 'For grown-ups: '); adult.appendChild(el('b', null, null, c.view)); adult.appendChild(document.createTextNode('. ' + c.adult));
    card.appendChild(adult);
    if (c.open) { var go = el('button', 'ap-go', null, 'Open ' + c.view + ' →'); go.type = 'button'; go.addEventListener('click', function () { openView(c.view); }); card.appendChild(go); }
    stage.appendChild(card);

    var head = el('div', 'ap-brand');
    head.appendChild(el('span', 'ap-name', null, 'warte airport'));
    var up = a.piers.filter(function (p) { return p.reporting; }).length;
    head.appendChild(el('span', 'ap-caption', null, 'live · ' + up + ' of ' + a.piers.length + ' machines reporting · ' + new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })));
    var fs = el('button', 'ap-fs', null, document.fullscreenElement ? 'Exit full screen' : 'Full screen'); fs.type = 'button';
    fs.addEventListener('click', toggle); head.appendChild(fs);
    stage.appendChild(head);

    root.replaceChildren(stage);
    fit();
    placeFieldLabel(stage);
  }
  // The 3D field's projected top edge, and the run signs standing above it,
  // depend on how many lanes there are: measure them and put the label above.
  function placeFieldLabel(stage) {
    var label = stage.querySelector('.ap-fieldlabel'), sr = stage.getBoundingClientRect();
    if (!label || !sr.height) return;
    var k = sr.height / H, top = H;
    stage.querySelectorAll('.ap-tilt, .ap-runsign').forEach(function (e) {
      var r = e.getBoundingClientRect(); if (r.height) top = Math.min(top, (r.top - sr.top) / k);
    });
    label.style.top = Math.round(top - 24) + 'px';
  }
  function toggle() {
    if (document.fullscreenElement) document.exitFullscreen();
    else if (root.requestFullscreen) root.requestFullscreen();
  }
  function fit() {
    var s = root.firstChild; if (!s || !s.style) return;
    if (document.fullscreenElement === root) {
      var k = Math.min(window.innerWidth / W, window.innerHeight / H);
      s.style.transform = 'translate(' + (window.innerWidth - W * k) / 2 + 'px,' + (window.innerHeight - H * k) / 2 + 'px) scale(' + k + ')';
      root.style.height = '';
    } else {
      var k2 = Math.min(1, root.clientWidth / W);
      s.style.transform = 'scale(' + k2 + ')'; root.style.height = (H * k2) + 'px';
    }
  }
  function load() {
    if (!document.body.contains(root)) { clearInterval(timer); return; }
    fetch('api/airport').then(function (r) { return r.json(); }).then(draw).catch(function () {
      root.replaceChildren(el('p', 'empty', null, 'The airport could not be loaded.'));
    });
  }
  var timer = setInterval(function () { if (!document.hidden) load(); }, 30000);
  // page-wide listeners once; they act on whichever airport is on the page now
  window.__warteAirport = { fit: fit, toggle: toggle, redraw: function () { if (last) draw(last); }, root: root };
  if (!window.__warteAirportKeys) {
    window.__warteAirportKeys = true;
    var cur = function () { var a = window.__warteAirport; return a && document.body.contains(a.root) ? a : null; };
    window.addEventListener('resize', function () { var a = cur(); if (a) a.fit(); });
    document.addEventListener('fullscreenchange', function () { var a = cur(); if (a) a.redraw(); });
    document.addEventListener('keydown', function (e) {
      var a = cur();
      if (a && e.key === 'f' && !e.ctrlKey && !e.metaKey && !/INPUT|TEXTAREA|SELECT/.test((e.target && e.target.tagName) || '')) a.toggle();
    });
  }
  load();
})();
