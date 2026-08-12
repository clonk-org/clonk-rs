// Test for _SWT script increment/decrement lvalue issue

// Line 74 from _SWT: quadruple prefix increment
crate::support::compile_case!(
    swt_quad_increment,
    r#"
func Test() {
    var i;
    while (Local(i)) ++++i;
}
    "#,
);

// Line 75 from _SWT: Local(i++) as lvalue
crate::support::compile_case!(
    swt_postfix_in_local_assignment,
    r#"
func Test(pTarget, iDir) {
    var i;
    Local(i++) = pTarget;
    Local(i) = iDir;
}
    "#,
);

// Full AddTarget function from _SWT
crate::support::compile_case!(
    swt_full_add_target,
    r#"
public func AddTarget(object pTarget, int iDir)
  {
//  if (!iDir) iDir=1;
  var i; while (Local(i)) ++++i;
  Local(i++) = pTarget; Local(i) = iDir;
  }
    "#,
);
