<!-- cargo-rdme start -->

A separated value encoder and parser.

# Example

```rust
const FIELD: &str = "smiley -\n🙃";

let mut enc = Vec::new();

// write a field in the "raw" format:
enc.extend_from_slice(&write_raw_field(FIELD.as_bytes()));

// field delimiter
enc.extend_from_slice(FIELD_DELIM);

// write a field in the "quoted" format:
enc.extend_from_slice(&write_quote_field(FIELD.as_bytes()));

// field delimiter
enc.extend_from_slice(FIELD_DELIM);

// write a field in the "binary" format:
enc.extend_from_slice(&write_bin_header(FIELD.as_bytes().len()));
enc.extend_from_slice(FIELD.as_bytes());
enc.extend_from_slice(BIN_FOOTER);

// row delimiter
enc.extend_from_slice(ROW_DELIM);

let mut expected = String::new();
expected.push_str("smiley%20-%0A%F0%9F%99%83");
expected.push(' ');
expected.push_str(r#""smiley -
%F0%9F%99%83""#);
expected.push(' ');
expected.push_str(r#"`13|smiley -
🙃`"#);
expected.push('\n');

assert_eq!(expected, String::from_utf8_lossy(&enc));

let mut parser = AsvParse::default();
let result = parser.parse(&enc).unwrap();
assert_eq!(1, result.len());
for row in result {
    assert_eq!(3, row.len());
    for field in row {
        assert_eq!("smiley -\n🙃", String::from_utf8_lossy(&field));
    }
}
```

<!-- cargo-rdme end -->
