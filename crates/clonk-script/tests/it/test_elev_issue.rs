// Test for ELEV issue - if with !variable followed by return with method call

#[test]
fn elev_pattern_minimal() {
    // Minimal reproduction of ELEV lines 56-57
    let source = r#"
        func Test() {
            var pCase;
            if (!pCase) return(0);
            return (pCase->IsInPermanentMode());
        }
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn elev_pattern_exact() {
    // Exact pattern from ELEV
    let source = r#"
public func IsInPermanentMode()
{
  if (!pCase) return(0);
  return (pCase->IsInPermanentMode());
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn braceless_if_with_not() {
    // Braceless if with ! operator
    let source = r#"func Test() { var x; if (!x) return 1; return 0; }"#;
    crate::support::assert_compiles(source);
}
