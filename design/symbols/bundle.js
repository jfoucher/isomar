/* Chart design system: SVG drawing helpers shared by the previews. Not a runtime library. */
window.Chart = (function () {
  var uid = 0;
  function v(n) { return 'var(--' + n + ')'; }
  function st(o) {
    var s = [];
    for (var k in o) if (o[k] != null) s.push(k + ':' + o[k]);
    return ' style="' + s.join(';') + '"';
  }
  function svg(w, h, inner, bg) {
    return '<svg viewBox="0 0 ' + w + ' ' + h + '" width="100%" style="display:block" role="img">' +
      (bg ? '<rect width="' + w + '" height="' + h + '"' + st({ fill: v(bg) }) + '/>' : '') + inner + '</svg>';
  }
  /* A stroke Chart draws over the chart: halo underneath, ink on top. */
  function line(d, ink, w, dash, opts) {
    opts = opts || {};
    var halo = '<path d="' + d + '"' + st({ fill: 'none', stroke: v('overlay-halo'), 'stroke-width': (w + 3) + 'px', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'stroke-dasharray': dash || null }) + '/>';
    var ink_ = '<path d="' + d + '"' + st({ fill: 'none', stroke: v(ink), 'stroke-width': w + 'px', 'stroke-linecap': opts.cap || 'round', 'stroke-linejoin': 'round', 'stroke-dasharray': dash || null, opacity: opts.opacity || null }) + '/>';
    return halo + ink_;
  }
  function label(x, y, text, cls, anchor, ink) {
    var a = ' x="' + x + '" y="' + y + '" text-anchor="' + (anchor || 'start') + '"';
    var base = 'font-family:var(--font-sans);font-size:12px;font-weight:' + (cls === 'strong' ? 700 : 500) + (cls === 'italic' ? ';font-style:italic;font-weight:400' : '');
    return '<text' + a + ' style="' + base + ';fill:none;stroke:var(--overlay-halo);stroke-width:3px;stroke-linejoin:round">' + text + '</text>' +
      '<text' + a + ' style="' + base + ';fill:' + v(ink || 'text') + '">' + text + '</text>';
  }
  function chartText(x, y, text, size, italic, ink, anchor, weight) {
    return '<text x="' + x + '" y="' + y + '" text-anchor="' + (anchor || 'start') + '" style="font-family:var(--font-sans);font-size:' + (size || 11) + 'px;' +
      (italic ? 'font-style:italic;' : '') + 'font-weight:' + (weight || 400) + ';fill:' + v(ink || 'chart-ink') + '">' + text + '</text>';
  }
  /* Light flare (tear): tip at the light's position, pointing along `bearing` (degrees true). */
  function flare(x, y, colour, bearing, len) {
    var L = len || 30, r = L * 0.27;
    var d = 'M0,0 C' + (r * 0.35) + ',' + (-L * 0.25) + ' ' + r + ',' + (-L * 0.55) + ' ' + r + ',' + (-(L - r)) +
      ' A' + r + ',' + r + ' 0 0 0 ' + (-r) + ',' + (-(L - r)) + ' C' + (-r) + ',' + (-L * 0.55) + ' ' + (-r * 0.35) + ',' + (-L * 0.25) + ' 0,0 Z';
    return '<path transform="translate(' + x + ',' + y + ') rotate(' + (bearing == null ? 135 : bearing) + ')" d="' + d + '"' +
      st({ fill: v(colour), stroke: v('chart-ink'), 'stroke-width': '0.6px' }) + '/>';
  }
  function lightDot(x, y) { return '<circle cx="' + x + '" cy="' + y + '" r="2.4"' + st({ fill: v('chart-ink') }) + '/>'; }
  function polar(cx, cy, r, brg) { var a = (brg - 90) * Math.PI / 180; return [cx + r * Math.cos(a), cy + r * Math.sin(a)]; }
  function arcPath(cx, cy, r, a1, a2) {
    var p1 = polar(cx, cy, r, a1), p2 = polar(cx, cy, r, a2), large = ((a2 - a1 + 360) % 360) > 180 ? 1 : 0;
    return 'M' + p1[0].toFixed(1) + ',' + p1[1].toFixed(1) + ' A' + r + ',' + r + ' 0 ' + large + ' 1 ' + p2[0].toFixed(1) + ',' + p2[1].toFixed(1);
  }
  /* Sector band: a1..a2 are directions FROM the light (clockwise). */
  function sector(cx, cy, r, a1, a2, colour, w) {
    w = w || 6;
    var d = arcPath(cx, cy, r, a1, a2);
    return '<path d="' + d + '"' + st({ fill: 'none', stroke: v('chart-ink'), 'stroke-width': (w + 1.4) + 'px' }) + '/>' +
      '<path d="' + d + '"' + st({ fill: 'none', stroke: v(colour), 'stroke-width': w + 'px' }) + '/>';
  }
  function limit(cx, cy, r, brg) {
    var p = polar(cx, cy, r, brg);
    return '<path d="M' + cx + ',' + cy + ' L' + p[0].toFixed(1) + ',' + p[1].toFixed(1) + '"' +
      st({ fill: 'none', stroke: v('chart-ink'), 'stroke-width': '0.8px', 'stroke-dasharray': '5 3' }) + '/>';
  }
  /* Buoy and beacon bodies, drawn upright with the position at (0,0). */
  var BODY = {
    can: 'M-8,0 L8,0 L8,-24 L-8,-24 Z',
    cone: 'M-10,0 L10,0 L0,-27 Z',
    sphere: 'M-11,-11 A11,11 0 1 1 11,-11 A11,11 0 1 1 -11,-11 Z',
    pillar: 'M-11,0 L11,0 L7,-9 L3.5,-9 L3.5,-33 L-3.5,-33 L-3.5,-9 L-7,-9 Z',
    spar: 'M-2.6,0 L2.6,0 L2.6,-34 L-2.6,-34 Z',
    tower: 'M-8,0 L8,0 L3.5,-34 L-3.5,-34 Z',
    stake: 'M-1.2,0 L1.2,0 L1.2,-34 L-1.2,-34 Z'
  };
  var TOP = { can: -24, cone: -27, sphere: -22, pillar: -33, spar: -34, tower: -34, stake: -34 };
  function cone(y, up, ink) {
    return '<path d="' + (up ? 'M-5,' + y + ' L5,' + y + ' L0,' + (y - 8) + ' Z' : 'M-5,' + (y - 8) + ' L5,' + (y - 8) + ' L0,' + y + ' Z') + '"' + st({ fill: v(ink || 'mark-black'), stroke: v('chart-ink'), 'stroke-width': '0.8px' }) + '/>';
  }
  function topmark(kind, y) {
    var m = '<path d="M0,' + y + ' L0,' + (y - 4) + '"' + st({ stroke: v('chart-ink'), 'stroke-width': '1.2px' }) + '/>';
    y -= 4;
    switch (kind) {
      case 'N': return m + cone(y, true) + cone(y - 9, true);
      case 'S': return m + cone(y, false) + cone(y - 9, false);
      case 'E': return m + cone(y, false) + cone(y - 8, true);
      case 'W': return m + cone(y, true) + cone(y - 9, false);
      case 'can': return m + '<rect x="-4.5" y="' + (y - 8) + '" width="9" height="8"' + st({ fill: v('mark-red'), stroke: v('chart-ink'), 'stroke-width': '0.8px' }) + '/>';
      case 'conetop': return m + cone(y, true, 'mark-green');
      case 'balls': return m + '<circle cx="0" cy="' + (y - 3.6) + '" r="3.6"' + st({ fill: v('mark-black'), stroke: v('chart-ink'), 'stroke-width': '0.8px' }) + '/>' +
        '<circle cx="0" cy="' + (y - 11.4) + '" r="3.6"' + st({ fill: v('mark-black'), stroke: v('chart-ink'), 'stroke-width': '0.8px' }) + '/>';
      case 'ball': return m + '<circle cx="0" cy="' + (y - 4) + '" r="4"' + st({ fill: v('mark-red'), stroke: v('chart-ink'), 'stroke-width': '0.8px' }) + '/>';
      case 'x': return m + '<path d="M-4.5,' + (y - 9) + ' L4.5,' + y + ' M4.5,' + (y - 9) + ' L-4.5,' + y + '"' + st({ stroke: v('mark-yellow'), 'stroke-width': '2.6px' }) + '/>' +
        '<path d="M-4.5,' + (y - 9) + ' L4.5,' + y + ' M4.5,' + (y - 9) + ' L-4.5,' + y + '"' + st({ stroke: v('chart-ink'), 'stroke-width': '0.6px' }) + '/>';
      case 'cross': return m + '<path d="M0,' + (y - 10) + ' L0,' + y + ' M-4.5,' + (y - 5) + ' L4.5,' + (y - 5) + '"' + st({ stroke: v('mark-yellow'), 'stroke-width': '2.6px' }) + '/>';
    }
    return '';
  }
  /* bands: [{c:'mark-black', from:0, to:0.5}] as fractions of height from top; stripes: vertical. */
  function mark(x, y, o) {
    var id = 'cb' + (uid++), shape = BODY[o.shape], top = TOP[o.shape], h = -top;
    var lean = o.beacon ? 0 : 15, fill = '';
    (o.bands || [{ c: o.colour, from: 0, to: 1 }]).forEach(function (b) {
      fill += '<rect x="-14" y="' + (top + b.from * h) + '" width="28" height="' + ((b.to - b.from) * h + 0.4) + '"' + st({ fill: v(b.c) }) + '/>';
    });
    if (o.stripes) o.stripes.forEach(function (c, i) {
      fill += '<rect x="' + (-14 + i * 5) + '" y="' + top + '" width="5.2" height="' + h + '"' + st({ fill: v(c) }) + '/>';
    });
    var g = '<defs><clipPath id="' + id + '"><path d="' + shape + '"/></clipPath></defs>' +
      '<g clip-path="url(#' + id + ')">' + fill + '</g>' +
      '<path d="' + shape + '"' + st({ fill: 'none', stroke: v('chart-ink'), 'stroke-width': '1px', 'stroke-linejoin': 'round' }) + '/>' +
      (o.top ? topmark(o.top, top) : '');
    var pos = o.beacon
      ? '<path d="M-9,0 L9,0"' + st({ stroke: v('chart-ink'), 'stroke-width': '1.2px' }) + '/>'
      : '<path d="M-13,1 L-4,1 M4,1 L13,1"' + st({ stroke: v('chart-ink'), 'stroke-width': '1px' }) + '/><circle cx="0" cy="1" r="2.6"' + st({ fill: v('chart-deep'), stroke: v('chart-ink'), 'stroke-width': '1px' }) + '/>';
    return '<g transform="translate(' + x + ',' + y + ') scale(' + (o.scale || 1) + ')">' + (o.flare ? flare(0, 0, o.flare, 135, 26) : '') +
      '<g transform="rotate(' + lean + ')">' + g + '</g>' + pos + '</g>';
  }
  function ownShip(x, y, hdg, opts) {
    opts = opts || {};
    var hull = 'M0,-15 C5,-8 6,0 5,10 L-5,10 C-6,0 -5,-8 0,-15 Z';
    var stale = opts.stale;
    return '<g transform="translate(' + x + ',' + y + ') rotate(' + hdg + ')">' +
      '<path d="' + hull + '"' + st({ fill: 'none', stroke: v('overlay-halo'), 'stroke-width': '4px' }) + '/>' +
      '<path d="' + hull + '"' + st({ fill: stale ? 'none' : v('own-ship'), stroke: v(stale ? 'stale' : 'own-ship'), 'stroke-width': '1.5px', 'stroke-dasharray': stale ? '3 2' : null }) + '/>' +
      '<circle cx="0" cy="0" r="1.6"' + st({ fill: v(stale ? 'stale' : 'overlay-halo') }) + '/></g>';
  }
  function aisTri(x, y, cog, kind) {
    var d = 'M0,-11 L7,8 L-7,8 Z', ink = kind === 'danger' || kind === 'ack' ? 'ais-danger' : kind === 'lost' ? 'ais-lost' : 'ais';
    var filled = kind === 'danger' || kind === 'ack';
    var s = '<g transform="translate(' + x + ',' + y + ') rotate(' + cog + ')"' + (kind === 'danger' ? ' class="blink"' : '') + '>' +
      '<path d="' + d + '"' + st({ fill: 'none', stroke: v('overlay-halo'), 'stroke-width': '4.5px', 'stroke-linejoin': 'round' }) + '/>' +
      '<path d="' + d + '"' + st({ fill: filled ? v(ink) : 'none', stroke: v(ink), 'stroke-width': '1.8px', 'stroke-linejoin': 'round', 'stroke-dasharray': kind === 'lost' ? '3 2' : null }) + '/>';
    if (kind === 'lost') s += '<path d="M-5,-2 L5,6 M5,-2 L-5,6"' + st({ stroke: v(ink), 'stroke-width': '1.5px' }) + '/>';
    return s + '</g>';
  }
  function chip(x, y, text, bg, ink) {
    var w = text.length * 6.6 + 12;
    return '<rect x="' + (x - w / 2) + '" y="' + (y - 9) + '" width="' + w + '" height="18" rx="9"' + st({ fill: v(bg), stroke: v('overlay-halo'), 'stroke-width': '1.5px' }) + '/>' +
      '<text x="' + x + '" y="' + (y + 4) + '" text-anchor="middle" style="font-family:var(--font-sans);font-size:11px;font-weight:600;font-variant-numeric:tabular-nums;fill:' + v(ink) + '">' + text + '</text>';
  }
  return { v: v, st: st, svg: svg, line: line, label: label, chartText: chartText, flare: flare, lightDot: lightDot, sector: sector, limit: limit, polar: polar, arcPath: arcPath, mark: mark, ownShip: ownShip, aisTri: aisTri, chip: chip };
})();
