//! Generates `tokens.rs` from the design tokens (`data/design/tokens.json`): a `Token` for
//! every colour, with its value in each theme, and the numbers of the stroke, size, chart and
//! timing tokens. The file is the one source of the design's values; code never has its own hex.

use serde_json::Value;
use std::fmt::Write;

const TOKENS: &str = "../../data/design/tokens.json";

fn camel(name: &str) -> String {
    name.split('-')
        .map(|p| {
            let mut c = p.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect()
}

fn screaming(name: &str) -> String {
    name.replace('-', "_").to_uppercase()
}

include!("../../data/design/parse.rs");

fn main() {
    println!("cargo:rerun-if-changed={TOKENS}");
    println!("cargo:rerun-if-changed=../../data/design/parse.rs");
    let json: Value = serde_json::from_str(&std::fs::read_to_string(TOKENS).expect("tokens.json")).expect("tokens.json is JSON");
    let mut out = String::new();

    let colours = json["color"]["tokens"].as_array().expect("colour tokens");
    out.push_str("/// A colour of the design, with a value in each theme.\n#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum Token {\n");
    for c in colours {
        writeln!(out, "    {},", camel(c["name"].as_str().unwrap())).unwrap();
    }
    out.push_str("}\n\n");
    writeln!(out, "pub const TOKEN_COUNT: usize = {};\n", colours.len()).unwrap();
    out.push_str("impl Token {\n    pub const ALL: [Token; TOKEN_COUNT] = [\n");
    for c in colours {
        writeln!(out, "        Token::{},", camel(c["name"].as_str().unwrap())).unwrap();
    }
    out.push_str("    ];\n\n    /// The token's name in `tokens.json`.\n    pub fn name(self) -> &'static str {\n        match self {\n");
    for c in colours {
        let n = c["name"].as_str().unwrap();
        writeln!(out, "            Token::{} => \"{n}\",", camel(n)).unwrap();
    }
    out.push_str("        }\n    }\n}\n\n");
    out.push_str("/// Values as (red, green, blue, alpha), for day, dusk and night.\nstatic COLOURS: [[(f64, f64, f64, f64); 3]; TOKEN_COUNT] = [\n");
    for c in colours {
        let v = &c["value"];
        let t = |k: &str| colour(v[k].as_str().unwrap_or_else(|| panic!("{} has no {k}", c["name"])));
        let f = |(r, g, b, a): (f64, f64, f64, f64)| format!("({r:?}, {g:?}, {b:?}, {a:?})");
        writeln!(out, "    [{}, {}, {}],", f(t("day")), f(t("dusk")), f(t("night"))).unwrap();
    }
    out.push_str("];\n\n");

    for (group, unit, ty) in [("size", "px", "f64"), ("stroke", "px", "f64"), ("chart", "mm", "f64"), ("timing", "ms", "f64")] {
        writeln!(out, "// {group} tokens, in {unit}.").unwrap();
        for t in json[group]["tokens"].as_array().expect("tokens") {
            let n = t["name"].as_str().unwrap();
            writeln!(out, "pub const {}: {ty} = {:?};", screaming(n), number_ms(t["value"].as_str().unwrap())).unwrap();
        }
        out.push('\n');
    }
    let dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(format!("{dir}/tokens.rs"), out).unwrap();
}
