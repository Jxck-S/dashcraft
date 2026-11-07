//! Deterministic generated DAX input: a small xorshift generator, random
//! expressions and queries that parse, and token soup that mostly doesn't.

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

pub const ATOMS: &[&str] = &[
    "1",
    "0",
    "2.5",
    ".5",
    "1e3",
    "1.",
    "\"a\"",
    "\"q\"\"q\"",
    "\"\"",
    "TRUE",
    "FALSE()",
    "[m]",
    "[a]]b]",
    "T[c]",
    "'Web Sessions'[Traffic Source]",
    "'It''s'[x]",
    "Orders",
    "'Web Sessions'",
    "'Table'",
    "'v0'",
    "BLANK()",
    "DESC",
    "Year",
    "@p",
    "dt\"2020-01-01\"",
    "v0",
    "v1",
    "_x",
];

pub const BINOPS: &[&str] = &[
    "+", "-", "*", "/", "^", "&", "=", "==", "<>", "<", ">", "<=", ">=", "&&", "||", "IN", "in",
];

pub const FUNCS: &[&str] = &[
    "CALCULATE",
    "IF",
    "SumX",
    "PERCENTILE.INC",
    "NOT",
    "f",
    "TRUE",
];

pub fn gen_expr(rng: &mut Rng, depth: usize) -> String {
    if depth == 0 {
        return rng.pick(ATOMS).to_string();
    }
    let d = depth - 1;
    match rng.below(12) {
        0 | 1 => rng.pick(ATOMS).to_string(),
        2..=4 => format!(
            "{} {} {}",
            gen_expr(rng, d),
            rng.pick(BINOPS),
            gen_expr(rng, d)
        ),
        5 => format!("({})", gen_expr(rng, d)),
        6 => format!(
            "{} {}",
            rng.pick(&["-", "+", "NOT", "not"]),
            gen_expr(rng, d)
        ),
        7 => {
            let name = rng.pick(FUNCS);
            if name == "TRUE" {
                return "TRUE()".to_string();
            }
            let args: Vec<String> = (0..rng.below(4))
                .map(|_| {
                    if rng.below(8) == 0 {
                        String::new()
                    } else {
                        gen_expr(rng, d)
                    }
                })
                .collect();
            format!("{name}({})", args.join(", "))
        }
        8 => {
            let mut s = String::new();
            for i in 0..=rng.below(3) {
                s.push_str(&format!("VAR v{i} = {} ", gen_expr(rng, d)));
            }
            format!("{s}RETURN {}", gen_expr(rng, d))
        }
        9 => gen_table(rng, d),
        10 => format!("{} IN {}", gen_expr(rng, d), gen_table(rng, d)),
        _ => format!("({}, {})", gen_expr(rng, d), gen_expr(rng, d)),
    }
}

pub fn gen_table(rng: &mut Rng, depth: usize) -> String {
    let rows: Vec<String> = (0..rng.below(4))
        .map(|_| {
            if rng.below(2) == 0 {
                gen_expr(rng, depth)
            } else {
                format!("({}, {})", gen_expr(rng, depth), gen_expr(rng, depth))
            }
        })
        .collect();
    format!("{{{}}}", rows.join(", "))
}

pub fn gen_query(rng: &mut Rng) -> String {
    let mut s = String::new();
    if rng.below(2) == 0 {
        s.push_str("DEFINE\n");
        for _ in 0..=rng.below(4) {
            let def = match rng.below(5) {
                0 => format!("MEASURE 'T'[m] = {}", gen_expr(rng, 3)),
                1 => format!("VAR v0 = {}", gen_expr(rng, 3)),
                2 => format!("TABLE 'My Table' = {}", gen_expr(rng, 3)),
                3 => format!("COLUMN T[c] = {}", gen_expr(rng, 3)),
                _ => format!(
                    "FUNCTION F.G = (v1 : SCALAR VAL, x) => {}",
                    gen_expr(rng, 3)
                ),
            };
            s.push_str(&def);
            s.push('\n');
        }
    }
    for _ in 0..=rng.below(2) {
        s.push_str(&format!("EVALUATE {}\n", gen_expr(rng, 3)));
        if rng.below(2) == 0 {
            let keys: Vec<String> = (0..=rng.below(2))
                .map(|_| format!("{} {}", gen_expr(rng, 2), rng.pick(&["", "ASC", "DESC"])))
                .collect();
            s.push_str(&format!("ORDER BY {}\n", keys.join(", ")));
            if rng.below(2) == 0 {
                s.push_str(&format!(
                    "START AT {}, {}\n",
                    gen_expr(rng, 1),
                    gen_expr(rng, 1)
                ));
            }
        }
    }
    s
}

/// Token fragments for random "soup" inputs (mostly invalid).
pub const SOUP: &[&str] = &[
    "1", "2.5", "1e", ".", "\"s\"", "\"", "'T'", "'", "T", "[c]", "[", "]", "(", ")", "{", "}",
    ",", "+", "-", "*", "/", "^", "&", "&&", "|", "||", "=", "==", "<>", "<", ">", "<=", ">=",
    "IN", "NOT", "TRUE", "FALSE", "VAR", "x", "RETURN", "SUM", "DESC", "@p", "@", "dt\"d\"", "--",
    "//", "/*", "*/", "\n", "BLANK", "Year", "EVALUATE", "DEFINE", "MEASURE", "ORDER", "BY",
    "START", "AT", "TABLE", "COLUMN", "FUNCTION", "=>", ":", "é", "$", "\u{feff}",
];
