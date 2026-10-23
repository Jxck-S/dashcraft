//! "Parse people field" transform for text columns holding Python `repr`
//! lists of people.
//!
//! Exports often write multi-person fields as Python lists, either of names,
//! like the account managers in the example's `customers.csv`
//! (`['Ana Ruiz', "O'Neil Park"]`), or of person records
//! (`[{'id': '1', 'firstName': 'Ana', 'lastName': 'Ruiz'}]`). Python switches to
//! double quotes for strings containing an apostrophe (`"O'Neil Park"`), so
//! both quote styles are accepted. The transform returns one name per person
//! (`First Last` for records) joined by `"; "`, like the example's M function
//! `fnParseNames`.
//!
//! [`people_array`] applies the transform to a whole column; it backs
//! [`DerivedColumn::People`](crate::DerivedColumn::People).

use std::sync::Arc;

use arrow::array::{Array, ArrayRef, AsArray, StringBuilder};
use arrow::datatypes::DataType as ArrowType;

/// Result of parsing one cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum People {
    /// Blank cell or an empty list.
    Empty,
    /// One or more people (`First Last` for person records).
    Names(Vec<String>),
    /// The cell is not a people list; the trimmed original text is kept.
    Unparsed(String),
}

impl People {
    /// Display form used in the grid: names joined with `"; "`, `None` when empty.
    pub fn display(&self) -> Option<String> {
        match self {
            People::Empty => None,
            People::Names(names) => Some(names.join("; ")),
            People::Unparsed(text) => Some(text.clone()),
        }
    }
}

/// Parses a single people-field cell. Never panics; unrecognised input is
/// returned as [`People::Unparsed`].
pub fn parse_people(raw: &str) -> People {
    let text = raw.trim();
    if text.is_empty() {
        return People::Empty;
    }
    let Some(items) = parse_list(text) else {
        return People::Unparsed(text.to_string());
    };
    let names: Vec<String> = items
        .iter()
        .filter_map(|item| {
            let name = match item {
                Item::Name(name) => name.trim().to_string(),
                Item::Person(fields) => {
                    let first = field(fields, "firstName").unwrap_or("");
                    let last = field(fields, "lastName").unwrap_or("");
                    format!("{first} {last}").trim().to_string()
                }
            };
            (!name.is_empty()).then_some(name)
        })
        .collect();
    if !names.is_empty() {
        People::Names(names)
    } else if items.iter().all(|item| matches!(item, Item::Name(_))) {
        // `[]` or a list of blank names.
        People::Empty
    } else {
        People::Unparsed(text.to_string())
    }
}

/// Convenience wrapper returning the grid display value.
pub fn parse_people_display(raw: &str) -> Option<String> {
    parse_people(raw).display()
}

/// `array` as `Utf8` when it is a text column (`Utf8`, `LargeUtf8`,
/// `Utf8View` or a dictionary of those), else `None`.
#[must_use]
pub fn text_array(array: &ArrayRef) -> Option<ArrayRef> {
    let is_text = |t: &ArrowType| {
        matches!(
            t,
            ArrowType::Utf8 | ArrowType::LargeUtf8 | ArrowType::Utf8View
        )
    };
    let ok = match array.data_type() {
        ArrowType::Dictionary(_, values) => is_text(values),
        t => is_text(t),
    };
    if !ok {
        return None;
    }
    arrow::compute::cast(array, &ArrowType::Utf8).ok()
}

/// The parsed people (`First Last; …`) of every cell of a text column, as
/// `Utf8`: blanks and empty lists become BLANK, text that is not a people
/// list is kept (trimmed). `None` if `array` is not a text column.
#[must_use]
pub fn people_array(array: &ArrayRef) -> Option<ArrayRef> {
    let text = text_array(array)?;
    let strings = text.as_string_opt::<i32>()?;
    let mut out = StringBuilder::with_capacity(strings.len(), strings.value_data().len());
    for value in strings.iter() {
        match value.and_then(parse_people_display) {
            Some(parsed) => out.append_value(parsed),
            None => out.append_null(),
        }
    }
    Some(Arc::new(out.finish()))
}

type Fields = Vec<(String, Option<String>)>;

/// One list element: a plain name or a person record.
enum Item {
    Name(String),
    Person(Fields),
}

fn field<'a>(fields: &'a Fields, key: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_deref())
        .map(str::trim)
}

/// Parses `[ item, ... ]` where an item is a Python string or a
/// `{k: v, ...}` dict whose keys/values are Python string, number,
/// `True`/`False`/`None` literals. Nested containers are rejected.
fn parse_list(text: &str) -> Option<Vec<Item>> {
    let mut p = Parser {
        chars: text.chars().collect(),
        pos: 0,
    };
    p.skip_ws();
    p.expect('[')?;
    let mut items = Vec::new();
    loop {
        p.skip_ws();
        if p.eat(']') {
            break;
        }
        items.push(match p.peek()? {
            '{' => Item::Person(p.dict()?),
            _ => Item::Name(p.string()?),
        });
        p.skip_ws();
        if p.eat(',') {
            continue;
        }
        p.expect(']')?;
        break;
    }
    p.skip_ws();
    (p.pos == p.chars.len()).then_some(items)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: char) -> Option<()> {
        self.eat(c).then_some(())
    }

    fn dict(&mut self) -> Option<Fields> {
        self.expect('{')?;
        let mut fields = Vec::new();
        loop {
            self.skip_ws();
            if self.eat('}') {
                return Some(fields);
            }
            let key = self.string()?;
            self.skip_ws();
            self.expect(':')?;
            self.skip_ws();
            let value = self.scalar()?;
            fields.push((key, value));
            self.skip_ws();
            if self.eat(',') {
                continue;
            }
            self.expect('}')?;
            return Some(fields);
        }
    }

    /// A scalar value: string, number, or `True`/`False`/`None` (`None` → absent).
    fn scalar(&mut self) -> Option<Option<String>> {
        match self.peek()? {
            '\'' | '"' => self.string().map(Some),
            c if is_word_char(c) => {
                let start = self.pos;
                while self.peek().is_some_and(is_word_char) {
                    self.pos += 1;
                }
                let word: String = self.chars[start..self.pos].iter().collect();
                Some((word != "None").then_some(word))
            }
            _ => None,
        }
    }

    /// Python string literal with `'` or `"` quotes and backslash escapes.
    fn string(&mut self) -> Option<String> {
        let quote = self.peek().filter(|c| *c == '\'' || *c == '"')?;
        self.pos += 1;
        let mut out = String::new();
        loop {
            let c = self.peek()?;
            self.pos += 1;
            match c {
                c if c == quote => return Some(out),
                '\\' => {
                    let escaped = self.peek()?;
                    self.pos += 1;
                    out.push(match escaped {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        other => other,
                    });
                }
                other => out.push(other),
            }
        }
    }
}

fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '+')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_name() {
        assert_eq!(
            parse_people_display("['Lena Ford']").as_deref(),
            Some("Lena Ford")
        );
    }

    #[test]
    fn apostrophe_name_uses_double_quotes() {
        assert_eq!(
            parse_people_display(r#"['Ana Ruiz', "O'Neil Park"]"#).as_deref(),
            Some("Ana Ruiz; O'Neil Park")
        );
    }

    #[test]
    fn multiple_names_are_joined() {
        assert_eq!(
            parse_people(" ['Ana Ruiz', 'Jo Kim'] "),
            People::Names(vec!["Ana Ruiz".into(), "Jo Kim".into()])
        );
        assert_eq!(
            parse_people_display("['Ana Ruiz', 'Jo Kim']").as_deref(),
            Some("Ana Ruiz; Jo Kim")
        );
    }

    #[test]
    fn blank_names_are_dropped() {
        assert_eq!(
            parse_people_display("[' Lena Ford ', '', 'Jo Kim']").as_deref(),
            Some("Lena Ford; Jo Kim")
        );
        assert_eq!(parse_people("['', ' ']"), People::Empty);
    }

    #[test]
    fn person_records() {
        let raw = "[{'id': '1', 'firstName': 'Ana', 'middleName': '42', 'lastName': 'Ruiz', 'value': '42'}]";
        assert_eq!(parse_people_display(raw).as_deref(), Some("Ana Ruiz"));
        let raw = r#"[{'id': '7', 'firstName': 'Ana', 'lastName': 'Ruiz'}, {'firstName': 'Neil', 'lastName': "O'Park"}]"#;
        assert_eq!(
            parse_people(raw),
            People::Names(vec!["Ana Ruiz".into(), "Neil O'Park".into()])
        );
    }

    #[test]
    fn escaped_quote_and_non_string_values() {
        let raw = r"[{'firstName': 'O\'Neil', 'middleName': None, 'lastName': 'Park', 'id': 12, 'active': True}]";
        assert_eq!(parse_people_display(raw).as_deref(), Some("O'Neil Park"));
    }

    #[test]
    fn missing_last_name_keeps_first() {
        assert_eq!(
            parse_people_display("[{'firstName': 'Lena'}]").as_deref(),
            Some("Lena")
        );
    }

    #[test]
    fn trailing_comma_is_tolerated() {
        assert_eq!(
            parse_people_display("[{'firstName': 'A', 'lastName': 'B',},]").as_deref(),
            Some("A B")
        );
        assert_eq!(
            parse_people_display("['Jo Kim',]").as_deref(),
            Some("Jo Kim")
        );
    }

    #[test]
    fn empty_inputs() {
        assert_eq!(parse_people(""), People::Empty);
        assert_eq!(parse_people("  "), People::Empty);
        assert_eq!(parse_people("[]"), People::Empty);
        assert_eq!(parse_people_display("[]"), None);
    }

    #[test]
    fn non_people_text_is_kept() {
        assert_eq!(parse_people(" Jo Kim "), People::Unparsed("Jo Kim".into()));
        assert_eq!(
            parse_people("[{'id': '1'}]"),
            People::Unparsed("[{'id': '1'}]".into())
        );
    }

    #[test]
    fn whole_columns_are_parsed() {
        use arrow::array::{Int64Array, StringArray};
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("['Lena Ford']"),
            Some(r#"['Ana Ruiz', "O'Neil Park"]"#),
            Some(" Jo Kim "),
            Some("[]"),
            None,
        ]));
        let parsed = people_array(&array).unwrap_or_else(|| Arc::new(StringArray::from(vec![""])));
        let parsed = parsed.as_string::<i32>();
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed.value(0), "Lena Ford");
        assert_eq!(parsed.value(1), "Ana Ruiz; O'Neil Park");
        assert_eq!(parsed.value(2), "Jo Kim");
        assert!(parsed.is_null(3) && parsed.is_null(4));
        let numbers: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
        assert!(people_array(&numbers).is_none());
    }

    #[test]
    fn malformed_input_is_unparsed_not_panicking() {
        for raw in [
            "[",
            "[{",
            "['Ana Ruiz'",
            "[{'firstName'",
            "[{'firstName': 'A'",
            "[{'a': [1]}]",
            "[{'a': 'b'}] trailing",
            "['a'] trailing",
            "[{'a':\\",
            "{'firstName': 'A'}",
            "[,]",
            "[1, 2]",
            "[['Ana Ruiz']]",
            "[{'firstName': 'unterminated}]",
        ] {
            assert!(
                matches!(parse_people(raw), People::Unparsed(_)),
                "expected Unparsed for {raw:?}"
            );
        }
    }
}
