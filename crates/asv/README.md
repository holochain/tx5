<!-- cargo-rdme start -->

A separated value encoder and parser.

# Example

```rust
const FIELD: &str = "smiley -\n🙃";

let mut enc = AsvEncoder::default();

enc.field("test");
enc.field(r#"multi
line"#);
enc.field(FIELD);
enc.binary_bytes(FIELD);
enc.finish_row();

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

let mut parser = AsvParser::default();
let mut result =  parser.parse(bytes).unwrap();

assert_eq!(1, result.len());

for mut row in result {
    assert_eq!(4, row.len());
    assert_eq!("test", &row.remove(0).into_string().unwrap());
    assert_eq!("multi\nline", &row.remove(0).into_string().unwrap());
    assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
    assert_eq!(FIELD, &row.remove(0).into_string().unwrap());
}
```

<!-- cargo-rdme end -->
