<!-- cargo-rdme start -->

Asv is A Separated Value encoder and parser.
Loosely compatible with \[ct\]sv,
while allowing efficient binary encoding and parsing.

- The asv protocol is an ascii byte-level protocol. You are welcome to
  encode utf8 data in binary fields, or to percent encode the data into
  raw or quoted fields.
- Rows are newline delimited
- Fields are comma or whitespace delimited
- "Raw" type fields only allow classic url characters
  (`azAZ09-._~:/?#[]@!$&'()*+;=%`)
- "Quoted" type fields are bracketed by double quotes (`"`)
  and also allow the field delimiters and newline characters
- "Binary" type fields start with a back tick, specify the byte
  count in decimal, have a pipe character to mark the end of the
  byte count, then contain the raw bytes, and are followed by
  a closing back tick

```text
row1 raw-field "quoted field" `12|binary field`
row2 "" "empty fields must be quoted"
row3, "encoder delimits fields with spaces", "but you can use commas too"
```

This particular asv protocol implementation makes use of the [bytes] crate
to allow for zero-copy encoding and parsing in the case of large fields,
but also aggregates smaller field reading and writing for efficiency
gains on that end.

# Example

```rust

// -- ENCODING -- //

// this "field" contains utf8 data
const FIELD: &str = "smiley -\n🙃";

let mut enc = AsvEncoder::default();

// this data is safe to encode in a "raw" type field
enc.field("test");
// this data will be quoted because of the newline
enc.field(r#"multi
line"#);
// this field will be binary, because of the utf8 data
enc.field(FIELD);
// it is more efficient to explicitly use binary,
// so it doesn't have to run the heuristic algorithm.
enc.binary_bytes(FIELD);
// always remember to complete your row
enc.finish_row();

// -- Verify the Encoding -- //

let mut expected = String::new();
expected.push_str("test");
expected.push(' ');
expected.push_str("\"multi\nline\"");
expected.push(' ');
expected.push_str(&format!("`13|{FIELD}`"));
expected.push(' ');
expected.push_str(&format!("`13|{FIELD}`"));
expected.push('\n');

let bytes = enc.drain().into_vec();

assert_eq!(expected, String::from_utf8_lossy(&bytes));

// -- PARSING -- //

let mut parser = AsvParser::default();
let mut result = parser.parse(bytes).unwrap();

// -- Verify the Parsing -- //

assert_eq!(1, result.len());

let mut row = result.remove(0);

assert_eq!(4, row.len());

assert_eq!("test", &row.remove(0).into_string().unwrap());
assert_eq!("multi\nline", &row.remove(0).into_string().unwrap());
assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
```

<!-- cargo-rdme end -->
