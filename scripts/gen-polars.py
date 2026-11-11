#!/usr/bin/env python3
"""Regenerates the built-in polars, data/polars/*.csv, by averaging published polars of each kind of boat.

  python3 scripts/gen-polars.py           # downloads the sources into testdata/polars (gitignored), then writes data/polars

Sources (all fetched, none committed):
  - ORC certificates with their VPP, github.com/jieter/orc-data (MIT): the cruisers and the cruiser-racer,
    each model averaged first, then the models;
  - the OpenCPN weather routing plugin's polars, github.com/rgleason/weather_routing_pi (GPL-3.0);
  - Virtual Regatta's polars as kept in github.com/toxcct/VRPolarsChartData (GPL-3.0): best sail, foils on.
The downloads are untrusted data: ORC's files are Python literals and are read with ast.literal_eval.
"""
import ast, json, math, os, re, sys, urllib.request
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
D = os.path.join(ROOT, 'testdata', 'polars')
OUT = os.path.join(ROOT, 'data', 'polars')
ORC_YEARS = range(2019, 2025)
ORC = 'https://raw.githubusercontent.com/jieter/orc-data/master/ALL{y}.json'
OCPN = 'https://raw.githubusercontent.com/rgleason/weather_routing_pi/master/data/polars/{n}.pol'
VR = 'https://raw.githubusercontent.com/toxcct/VRPolarsChartData/master/data/boats/{n}.json'

def fetch(url, path):
    if os.path.exists(path):
        return
    os.makedirs(os.path.dirname(path), exist_ok=True)
    print('fetching', url)
    with urllib.request.urlopen(url) as r, open(path + '.tmp', 'wb') as f:
        f.write(r.read())
    os.rename(path + '.tmp', path)

for y in ORC_YEARS:
    fetch(ORC.format(y=y), f'{D}/orc/ALL{y}.json')
for n in ['Imoca_2015', 'Maxi_Multi_2013']:
    fetch(OCPN.format(n=n), f'{D}/ocpn/{n}.pol')
for n in ['mono/imoca_60_foils', 'multi/maxi_trimaran_v2', 'multi/ultim_macif']:
    fetch(VR.format(n=n), f'{D}/vr/{n.replace("/", "_")}.json')

def load(paths):
    """ORC certificates by sail number, the later files winning."""
    out = {}
    for p in paths:
        for b in ast.literal_eval(open(p).read()):
            out[b['sailnumber']] = b
    return list(out.values())

def interp(xs, ys, x):
    if x <= xs[0]: return ys[0]
    if x >= xs[-1]: return ys[-1]
    for i in range(1, len(xs)):
        if x <= xs[i]:
            t = (x - xs[i-1]) / (xs[i] - xs[i-1]); return ys[i-1] + t * (ys[i] - ys[i-1])

def bilinear(twa, tws, table, a, w):
    col = [interp(tws, row, w) for row in table]
    return interp(twa, col, a)

# ---------- ORC monohulls ----------
MONO_TWA = [0, 30, 35, 40, 45, 52, 60, 75, 90, 110, 120, 135, 150, 165, 180]
ORC_TWS = [6, 8, 10, 12, 14, 16, 20, 24]

def orc_polar(v):
    """One certificate on MONO_TWA x ORC_TWS."""
    rows = {a: [] for a in MONO_TWA}
    for j, w in enumerate(ORC_TWS):
        ba, bv = v['beat_angle'][j], v['beat_vmg'][j]
        bs = bv / math.cos(math.radians(ba))
        ra, rv = v['run_angle'][j], v['run_vmg'][j]
        rs = rv / -math.cos(math.radians(ra))
        pts = [(ba, bs)] + [(a, v[a][j]) for a in [52, 60, 75, 90, 110, 120, 135, 150] if a > ba + 1]
        v180 = rs if ra >= 177 else rv * 0.97
        if ra > 152 and ra < 177: pts.append((ra, rs))
        pts.append((180, v180))
        xs = [p[0] for p in pts]; ys = [p[1] for p in pts]
        for a in MONO_TWA:
            if a == 0: s = 0
            elif a < ba: s = bs * (a / ba) ** 3
            else: s = interp(xs, ys, a)
            rows[a].append(s)
    return [rows[a] for a in MONO_TWA]

boats = [b for b in load([f'{D}/orc/ALL{y}.json' for y in ORC_YEARS]) if b['boat'].get('type') and b.get('vpp') and b['vpp'].get('beat_angle') and b['vpp'].get('speeds') == ORC_TWS]
def norm(t):
    t = t.upper().replace('-', ' ').replace('.', ' ')
    t = re.sub(r'\b(SD|DK|WK|SK|DS|P|PERF\w*|CRUISER|CR|CLIPP?E?R?|GL|GRAND L\w*|QB|E|MOD\w*|BK|I)\b', ' ', t)
    return ' '.join(t.split()[:3])

CLASSES = {
  'small-cruiser': (7.4, 8.9, 1980, r'^(OCEANIS|SUN ODYSSEY|BAVARIA (?!.*MATCH)|HANSE|DUFOUR|HALLBERG|MOODY|ETAP|COMPIS|MAXI (77|84|87|909)|MAXI77|ALBIN (78|79|VEGA)|GIB.?SEA|SUN 2500|BENETEAU 25|CONTEST|MARIEHOLM)'),
  'medium-cruiser': (10.5, 11.7, 1990, r'^(OCEANIS|SUN ODYSSEY|BAVARIA (?!.*MATCH)|BAVARIA CR|HANSE|DUFOUR|HALLBERG|HALBERG|MOODY|WESTERLY|NAJAD|OYSTER|XC |COMPIS|GIB.?SEA|JEANNEAU ?37|BENETEAU 361|ELAN IMPRESSION|DEGERO|SCANMAR|MAXI 1100|MAXI 108)'),
  'large-cruiser': (14.0, 16.2, 1995, r'^(OCEANIS|SUN ODYSSEY|BAVARIA|HANSE|DUFOUR|HALLBERG|MOODY|NAJAD|OYSTER|AMEL|XC |JEANNEAU|BENETEAU (50|523|SENSE)|LM 46)'),
  'cruiser-racer': (11.5, 12.6, 2005, r'^(FIRST (40|405)|J 12[0-2]|J/12|X ?41|X ?40|X 4 3|X4|GRAND SOLEIL (39|40|42)|ITALIA 11|DEHLER (39|41|42)|SALONA (41|380)|ARCONA (400|410)|ELAN (40|410|E5)|SWAN 40|IMX 40|SUN FAST 40|MORE 40|SOLARIS)'),
}
out = {}
lens = {}
for key, (lo, hi, year, pat) in CLASSES.items():
    rx = re.compile(pat)
    by = defaultdict(list)
    for b in boats:
        loa = b['boat']['sizes'].get('loa') or 0
        t = b['boat']['type'].upper()
        if lo <= loa <= hi and rx.search(t) and (b['boat'].get('year') or 0) >= year:
            by[norm(t)].append(orc_polar(b['vpp'])); lens.setdefault(key, []).append(loa)
    models = {m: [[sum(p[i][j] for p in ps) / len(ps) for j in range(len(ORC_TWS))] for i in range(len(MONO_TWA))] for m, ps in by.items()}
    n = len(models)
    avg = [[sum(m[i][j] for m in models.values()) / n for j in range(len(ORC_TWS))] for i in range(len(MONO_TWA))]
    out[key] = dict(twa=MONO_TWA, tws=ORC_TWS, speeds=avg, models=sorted(models), certs=sum(len(v) for v in by.values()), loa=sum(lens[key])/len(lens[key]))

# ---------- IMOCA and Ultim ----------
FAST_TWA = [0, 30, 35, 40, 45, 50, 55, 60, 70, 80, 90, 100, 110, 120, 130, 140, 150, 160, 170, 180]
FAST_TWS = [4, 6, 8, 10, 12, 14, 16, 20, 25, 30, 35, 40]

def vr(name):
    d = json.load(open(f'{D}/vr/{name}.json'))
    f = d['foil']; hull = d['hull']['speedRatio']
    def ramp(x, lo, hi, m):
        if lo <= x <= hi: return 1
        if lo - m < x < lo: return (x - (lo - m)) / m
        if hi < x < hi + m: return ((hi + m) - x) / m
        return 0
    tab = []
    for i, a in enumerate(d['twa']):
        row = []
        for j, w in enumerate(d['tws']):
            s = max(sl['speed'][i][j] for sl in d['sail'])
            k = 1 + (f['speedRatio'] - 1) * ramp(a, f['twaMin'], f['twaMax'], f['twaMerge']) * ramp(w, f['twsMin'], f['twsMax'], f['twsMerge'])
            row.append(s * k * hull)
        tab.append(row)
    return d['twa'], d['tws'], tab

def pol(name):
    lines = [l.split() for l in open(f'{D}/ocpn/{name}.pol') if l.strip()]
    tws = [float(x) for x in lines[0][1:]]
    twa = [float(l[0]) for l in lines[1:]]
    tab = [[float(x) for x in l[1:]] for l in lines[1:]]
    return twa, tws, tab

def avg_of(sources):
    res = []
    for a in FAST_TWA:
        row = []
        for w in FAST_TWS:
            row.append(0 if a == 0 else sum(bilinear(*s, a, w) for s in sources) / len(sources))
        res.append(row)
    return res

imoca_src = {'VR IMOCA 60 (foils)': vr('mono_imoca_60_foils'), 'OpenCPN Imoca_2015': pol('Imoca_2015')}
ultim_src = {'VR Ultimate (V2)': vr('multi_maxi_trimaran_v2'), 'VR Ultimate MACIF': vr('multi_ultim_macif'), 'OpenCPN Maxi_Multi_2013': pol('Maxi_Multi_2013')}
for key, src in [('imoca-60', imoca_src), ('ultim', ultim_src)]:
    for n, s in src.items():
        print(key, n, 'at 90/20:', round(bilinear(*s, 90, 20), 1), '120/16:', round(bilinear(*s, 120, 16), 1), '45/12:', round(bilinear(*s, 45, 12), 1), '150/25:', round(bilinear(*s, 150, 25), 1))
    out[key] = dict(twa=FAST_TWA, tws=FAST_TWS, speeds=avg_of(list(src.values())), models=list(src))


NOTES = {
    'small-cruiser': ('Small cruiser, about 8 m', 'ORC VPP of %(certs)d certificates (2019-2024), %(n)d production cruisers of 7.4 to 8.9 m, mean %(loa).1f m, each model averaged first'),
    'medium-cruiser': ('Medium cruiser, about 11 m', 'ORC VPP of %(certs)d certificates (2019-2024), %(n)d production cruisers of 10.5 to 11.7 m, mean %(loa).1f m, each model averaged first'),
    'large-cruiser': ('Large cruiser, about 15 m', 'ORC VPP of %(certs)d certificates (2019-2024), %(n)d production cruisers of 14 to 16.2 m, mean %(loa).1f m, each model averaged first'),
    'cruiser-racer': ('Cruiser-racer, about 12 m', 'ORC VPP of %(certs)d certificates (2019-2024), %(n)d cruiser-racers built since 2005, 11.5 to 12.6 m, mean %(loa).1f m, each model averaged first'),
    'imoca-60': ('IMOCA 60', "the mean of the OpenCPN weather routing plugin's Imoca_2015.pol and Virtual Regatta's IMOCA 60 (foils), best sail, foils on"),
    'ultim': ('Ultim trimaran', "the mean of Virtual Regatta's Ultimate (V2) and Ultimate MACIF, best sail, foils on, and the OpenCPN plugin's Maxi_Multi_2013.pol"),
}
num = lambda v: (f'{v:.2f}').rstrip('0').rstrip('.')
os.makedirs(OUT, exist_ok=True)
for k, v in out.items():
    name, how = NOTES[k]
    how = how % dict(certs=v.get('certs', 0), n=len(v['models']), loa=v.get('loa', 0))
    lines = [f'# {name}', f'# Isomar built-in polar: {how}.']
    if 'certs' in v:
        lines.append('# ORC data: github.com/jieter/orc-data (MIT). Racing trim, full crew, spinnaker: a cruiser loaded for a passage is slower.')
        lines.append("# Rows 30 to 45 and 165, 180 come from the certificates' best beat and run angles; past 24 kn the last column holds.")
    else:
        lines.append('# Sources: github.com/rgleason/weather_routing_pi data/polars (GPL-3.0), github.com/toxcct/VRPolarsChartData (GPL-3.0).')
    lines.append('TWA/TWS;' + ';'.join(num(w) for w in v['tws']))
    for a, r in zip(v['twa'], v['speeds']):
        lines.append(num(a) + ';' + ';'.join(num(s) for s in r))
    open(f'{OUT}/{k}.csv', 'w').write('\n'.join(lines) + '\n')
    print(f'{k}: {len(v["models"])} models' + (f' ({v["certs"]} certificates)' if 'certs' in v else '') + ': ' + ', '.join(v['models']))
