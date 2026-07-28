// Test for _SWT script increment/decrement lvalue issue

#[test]
fn swt_quad_increment() {
    // Line 74 from _SWT: quadruple prefix increment
    let source = r#"
func Test() {
    var i;
    while (Local(i)) ++++i;
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn swt_postfix_in_local_assignment() {
    // Line 75 from _SWT: Local(i++) as lvalue
    let source = r#"
func Test(pTarget, iDir) {
    var i;
    Local(i++) = pTarget;
    Local(i) = iDir;
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn swt_full_add_target() {
    // Full AddTarget function from _SWT
    let source = r#"
public func AddTarget(object pTarget, int iDir)
  {
//  if (!iDir) iDir=1;
  var i; while (Local(i)) ++++i;
  Local(i++) = pTarget; Local(i) = iDir;
  }
    "#;
    crate::support::assert_compiles(source);
}
