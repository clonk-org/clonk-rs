// Test for ELEV issue - if with !variable followed by return with method call

// Minimal reproduction of ELEV lines 56-57
crate::support::compile_cases! {
    elev_pattern_minimal:
    r#"
        func Test() {
            var pCase;
            if (!pCase) return(0);
            return (pCase->IsInPermanentMode());
        }
    "#;

// Exact pattern from ELEV
    elev_pattern_exact:
    r#"
public func IsInPermanentMode()
{
  if (!pCase) return(0);
  return (pCase->IsInPermanentMode());
}
    "#;

// Braceless if with ! operator
    braceless_if_with_not: r#"func Test() { var x; if (!x) return 1; return 0; }"#;
}
