#!/usr/bin/env python3
# Regenerates crates/isomar-core/src/chart/s57names.rs from GDAL's S-57 tables (needs the gdal-data package).
# Run from the repository root.
import csv, sys
def esc(s): return s.replace('\\','\\\\').replace('"','\\"')
out = ['//! S-57 object class and attribute names, and the meanings of enumerated attribute values, for the click report.',
       '//! Generated from the IHO S-57 edition 3.1 tables (as shipped with GDAL, MIT licence), checked against OpenCPN\'s',
       '//! `s57objectclasses.csv` and `s57attributes.csv`; do not edit by hand.', '']
classes = []
for r in csv.DictReader(open('/usr/share/gdal/s57objectclasses.csv', encoding='latin-1')):
    if 0 < int(r['Code']) < 1000: classes.append((int(r['Code']), r['Acronym'], r['ObjectClass'].strip()))
classes.sort()
out.append('/// `(code, acronym, name)` of every object class, by code.')
out.append('pub static CLASSES: &[(u16, &str, &str)] = &[')
for c, a, n in classes: out.append(f'    ({c}, "{a}", "{esc(n)}"),')
out.append('];\n')
attrs = []
for r in csv.DictReader(open('/usr/share/gdal/s57attributes.csv', encoding='latin-1')):
    if 0 < int(r['Code']) < 1000: attrs.append((int(r['Code']), r['Acronym'], ' '.join(r['Attribute'].split()).replace('navigational System of','navigational - system of'), r['Attributetype']))
attrs.sort()
out.append('/// `(code, acronym, name, type)` of every attribute, by code. The type is S-57\'s: `E` enumerated, `L` list of enumerated,')
out.append('/// `I` integer, `F` float, `A` text, `S` free text, `T` date.')
out.append('pub static ATTRIBUTES: &[(u16, &str, &str, char)] = &[')
for c, a, n, t in attrs: out.append(f'    ({c}, "{a}", "{esc(n)}", \'{t}\'),')
out.append('];\n')
enums = []
for r in csv.DictReader(open('/usr/share/gdal/s57expectedinput.csv', encoding='latin-1')):
    if 0 < int(r['Code']) < 1000: enums.append((int(r['Code']), int(r['ID']), r['Meaning'].strip()))
enums.sort(key=lambda e: (e[0], e[1]))
out.append('/// `(attribute code, value, meaning)` of every enumerated value, by attribute then value.')
out.append('pub static ENUMS: &[(u16, u16, &str)] = &[')
for c, i, m in enums: out.append(f'    ({c}, {i}, "{esc(m)}"),')
out.append('];')
out.append('''
/// The name of an object class, as the standard has it ("Wreck").
pub fn class_name(objl: u16) -> Option<&'static str> {
    CLASSES.binary_search_by_key(&objl, |c| c.0).ok().map(|i| CLASSES[i].2)
}

/// The acronym of an object class ("WRECKS").
pub fn class_acronym(objl: u16) -> Option<&'static str> {
    CLASSES.binary_search_by_key(&objl, |c| c.0).ok().map(|i| CLASSES[i].1)
}

/// The name of an attribute ("Category of wreck").
pub fn attr_name(code: u16) -> Option<&'static str> {
    ATTRIBUTES.binary_search_by_key(&code, |a| a.0).ok().map(|i| ATTRIBUTES[i].2)
}

/// The type of an attribute: `E`, `L`, `I`, `F`, `A`, `S` or `T`.
pub fn attr_type(code: u16) -> Option<char> {
    ATTRIBUTES.binary_search_by_key(&code, |a| a.0).ok().map(|i| ATTRIBUTES[i].3)
}

/// What an enumerated value of an attribute means ("dangerous wreck").
pub fn enum_meaning(code: u16, value: u16) -> Option<&'static str> {
    ENUMS.binary_search_by_key(&(code, value), |e| (e.0, e.1)).ok().map(|i| ENUMS[i].2)
}
''')
open('crates/isomar-core/src/chart/s57names.rs','w', encoding='utf-8').write('\n'.join(out)+'\n')
print(len(classes), len(attrs), len(enums))
